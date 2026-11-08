use std::io::{BufReader, Cursor, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use serde::Serialize;
use serde_json::Value;

use crate::config::ServerEndpoint;
use crate::error::{AppError, AppResult};
use crate::logging::Logger;
use crate::util::{format_compact_utc, now_ms, write_bytes_atomic};

const SERVER_FILE_LIMIT: u64 = 16 * 1024 * 1024;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_COLLECTION: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    fn compound(&self) -> Option<&Vec<(String, Nbt)>> {
        match self {
            Nbt::Compound(entries) => Some(entries),
            _ => None,
        }
    }

    fn compound_mut(&mut self) -> Option<&mut Vec<(String, Nbt)>> {
        match self {
            Nbt::Compound(entries) => Some(entries),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&Nbt> {
        self.compound()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    fn string_value(&self) -> Option<&str> {
        match self {
            Nbt::String(value) => Some(value),
            _ => None,
        }
    }
}

fn nbt_error(message: impl Into<String>) -> AppError {
    AppError::with_detail("invalid_nbt", message)
}

fn read_u8<R: Read>(reader: &mut R) -> AppResult<u8> {
    let mut byte = [0u8; 1];
    reader
        .read_exact(&mut byte)
        .map_err(|e| nbt_error(e.to_string()))?;
    Ok(byte[0])
}

fn read_i16<R: Read>(reader: &mut R) -> AppResult<i16> {
    let mut bytes = [0u8; 2];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| nbt_error(e.to_string()))?;
    Ok(i16::from_be_bytes(bytes))
}

fn read_i32<R: Read>(reader: &mut R) -> AppResult<i32> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| nbt_error(e.to_string()))?;
    Ok(i32::from_be_bytes(bytes))
}

fn read_i64<R: Read>(reader: &mut R) -> AppResult<i64> {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| nbt_error(e.to_string()))?;
    Ok(i64::from_be_bytes(bytes))
}

fn read_string<R: Read>(reader: &mut R) -> AppResult<String> {
    let length = read_i16(reader)?;
    if length < 0 {
        return Err(nbt_error("negative NBT string length"));
    }
    let length = length as usize;
    if length > 1024 * 1024 {
        return Err(nbt_error("NBT string too long"));
    }
    let mut bytes = vec![0u8; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| nbt_error(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| nbt_error(e.to_string()))
}

fn read_collection_length<R: Read>(reader: &mut R) -> AppResult<usize> {
    let length = read_i32(reader)?;
    if length < 0 || length as usize > MAX_NBT_COLLECTION {
        return Err(nbt_error("invalid NBT collection length"));
    }
    Ok(length as usize)
}

fn read_payload<R: Read>(reader: &mut R, tag_type: u8, depth: usize) -> AppResult<Nbt> {
    if depth > MAX_NBT_DEPTH {
        return Err(nbt_error("NBT nesting too deep"));
    }
    match tag_type {
        1 => Ok(Nbt::Byte(read_u8(reader)? as i8)),
        2 => Ok(Nbt::Short(read_i16(reader)?)),
        3 => Ok(Nbt::Int(read_i32(reader)?)),
        4 => Ok(Nbt::Long(read_i64(reader)?)),
        5 => Ok(Nbt::Float(f32::from_bits(read_i32(reader)? as u32))),
        6 => Ok(Nbt::Double(f64::from_bits(read_i64(reader)? as u64))),
        7 => {
            let length = read_collection_length(reader)?;
            let mut bytes = vec![0u8; length];
            reader
                .read_exact(&mut bytes)
                .map_err(|e| nbt_error(e.to_string()))?;
            Ok(Nbt::ByteArray(bytes))
        }
        8 => Ok(Nbt::String(read_string(reader)?)),
        9 => {
            let element_type = read_u8(reader)?;
            let length = read_collection_length(reader)?;
            if element_type == 0 && length > 0 {
                return Err(nbt_error("list of TAG_End with elements"));
            }
            let mut values = Vec::with_capacity(length.min(1024));
            for _ in 0..length {
                values.push(read_payload(reader, element_type, depth + 1)?);
            }
            Ok(Nbt::List(values))
        }
        10 => {
            let mut entries = Vec::new();
            loop {
                let child_type = read_u8(reader)?;
                if child_type == 0 {
                    break;
                }
                let name = read_string(reader)?;
                let value = read_payload(reader, child_type, depth + 1)?;
                entries.push((name, value));
                if entries.len() > MAX_NBT_COLLECTION {
                    return Err(nbt_error("compound too large"));
                }
            }
            Ok(Nbt::Compound(entries))
        }
        11 => {
            let length = read_collection_length(reader)?;
            let mut values = Vec::with_capacity(length);
            for _ in 0..length {
                values.push(read_i32(reader)?);
            }
            Ok(Nbt::IntArray(values))
        }
        12 => {
            let length = read_collection_length(reader)?;
            let mut values = Vec::with_capacity(length.min(10_000));
            for _ in 0..length {
                values.push(read_i64(reader)?);
            }
            Ok(Nbt::LongArray(values))
        }
        _ => Err(nbt_error(format!("unknown NBT tag type {tag_type}"))),
    }
}

pub fn parse_nbt(bytes: &[u8]) -> AppResult<Nbt> {
    let mut cursor = Cursor::new(bytes);
    let tag_type = read_u8(&mut cursor)?;
    if tag_type != 10 {
        return Err(nbt_error("root tag is not a compound"));
    }
    let _name = read_string(&mut cursor)?;
    let value = read_payload(&mut cursor, tag_type, 0)?;
    Ok(value)
}

fn tag_type(value: &Nbt) -> u8 {
    match value {
        Nbt::Byte(_) => 1,
        Nbt::Short(_) => 2,
        Nbt::Int(_) => 3,
        Nbt::Long(_) => 4,
        Nbt::Float(_) => 5,
        Nbt::Double(_) => 6,
        Nbt::ByteArray(_) => 7,
        Nbt::String(_) => 8,
        Nbt::List(_) => 9,
        Nbt::Compound(_) => 10,
        Nbt::IntArray(_) => 11,
        Nbt::LongArray(_) => 12,
    }
}

fn write_string<W: Write>(writer: &mut W, value: &str) -> AppResult<()> {
    let bytes = value.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(nbt_error("NBT string too long"));
    }
    writer.write_all(&(bytes.len() as u16).to_be_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn write_payload<W: Write>(writer: &mut W, value: &Nbt) -> AppResult<()> {
    match value {
        Nbt::Byte(v) => writer.write_all(&[*v as u8])?,
        Nbt::Short(v) => writer.write_all(&v.to_be_bytes())?,
        Nbt::Int(v) => writer.write_all(&v.to_be_bytes())?,
        Nbt::Long(v) => writer.write_all(&v.to_be_bytes())?,
        Nbt::Float(v) => writer.write_all(&v.to_bits().to_be_bytes())?,
        Nbt::Double(v) => writer.write_all(&v.to_bits().to_be_bytes())?,
        Nbt::ByteArray(values) => {
            if values.len() > i32::MAX as usize {
                return Err(nbt_error("byte array too long"));
            }
            writer.write_all(&(values.len() as i32).to_be_bytes())?;
            writer.write_all(values)?;
        }
        Nbt::String(value) => write_string(writer, value)?,
        Nbt::List(values) => {
            let element_type = values.first().map(tag_type).unwrap_or(0);
            writer.write_all(&[element_type])?;
            writer.write_all(&(values.len() as i32).to_be_bytes())?;
            for value in values {
                if tag_type(value) != element_type {
                    return Err(nbt_error("heterogeneous NBT list"));
                }
                write_payload(writer, value)?;
            }
        }
        Nbt::Compound(entries) => {
            for (name, value) in entries {
                writer.write_all(&[tag_type(value)])?;
                write_string(writer, name)?;
                write_payload(writer, value)?;
            }
            writer.write_all(&[0])?;
        }
        Nbt::IntArray(values) => {
            writer.write_all(&(values.len() as i32).to_be_bytes())?;
            for value in values {
                writer.write_all(&value.to_be_bytes())?;
            }
        }
        Nbt::LongArray(values) => {
            writer.write_all(&(values.len() as i32).to_be_bytes())?;
            for value in values {
                writer.write_all(&value.to_be_bytes())?;
            }
        }
    }
    Ok(())
}

pub fn encode_nbt(root: &Nbt) -> AppResult<Vec<u8>> {
    if !matches!(root, Nbt::Compound(_)) {
        return Err(nbt_error("root must be a compound"));
    }
    let mut output = Vec::new();
    output.push(10u8);
    write_string(&mut output, "")?;
    write_payload(&mut output, root)?;
    Ok(output)
}

fn read_servers_dat(path: &Path) -> AppResult<Option<Nbt>> {
    if !path.exists() {
        return Ok(None);
    }
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > SERVER_FILE_LIMIT {
        return Err(nbt_error("servers.dat is unexpectedly large"));
    }
    let bytes = std::fs::read(path)?;
    let decoded = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(BufReader::new(Cursor::new(bytes)));
        let mut limited = Vec::new();
        decoder
            .by_ref()
            .take(SERVER_FILE_LIMIT + 1)
            .read_to_end(&mut limited)
            .map_err(|e| nbt_error(e.to_string()))?;
        if limited.len() as u64 > SERVER_FILE_LIMIT {
            return Err(nbt_error("gzip NBT too large"));
        }
        limited
    } else {
        bytes
    };
    Ok(Some(parse_nbt(&decoded)?))
}

/// Updates or creates the named multiplayer entry while preserving all other
/// entries and unknown fields in servers.dat. `servers.dat` is uncompressed NBT,
/// matching the vanilla client's format.
pub fn update_server_entry(
    game_dir: &Path,
    entry_name: &str,
    endpoint: &ServerEndpoint,
    logger: &Arc<Logger>,
) -> AppResult<PathBuf> {
    if !game_dir.is_dir() {
        return Err(AppError::new("game_dir_missing"));
    }
    let path = game_dir.join("servers.dat");
    let mut root = match read_servers_dat(&path) {
        Ok(Some(root)) if matches!(root, Nbt::Compound(_)) => root,
        Ok(Some(_)) => {
            backup_corrupt(&path, logger);
            Nbt::Compound(Vec::new())
        }
        Ok(None) => Nbt::Compound(Vec::new()),
        Err(error) => {
            logger.warn(
                "server",
                format!(
                    "servers.dat could not be read ({}); creating a safe backup and a new entry",
                    error.code()
                ),
            );
            backup_corrupt(&path, logger);
            Nbt::Compound(Vec::new())
        }
    };

    let entries = root
        .compound_mut()
        .ok_or_else(|| nbt_error("servers.dat root is not a compound"))?;
    if !entries.iter().any(|(name, _)| name == "servers") {
        entries.push(("servers".to_string(), Nbt::List(Vec::new())));
    }
    let servers_index = entries
        .iter()
        .position(|(name, _)| name == "servers")
        .unwrap_or(0);
    if !matches!(entries[servers_index].1, Nbt::List(_)) {
        entries[servers_index].1 = Nbt::List(Vec::new());
    }
    let Nbt::List(servers) = &mut entries[servers_index].1 else {
        unreachable!();
    };

    let mut updated = false;
    for server in servers.iter_mut() {
        let Nbt::Compound(fields) = server else {
            continue;
        };
        let matches_name = fields
            .iter()
            .find(|(name, _)| name == "name")
            .and_then(|(_, value)| value.string_value())
            .map(|value| value == entry_name)
            .unwrap_or(false);
        if matches_name {
            set_string_field(fields, "ip", endpoint.address());
            if !fields.iter().any(|(name, _)| name == "acceptTextures") {
                fields.push(("acceptTextures".to_string(), Nbt::Byte(0)));
            }
            updated = true;
            break;
        }
    }

    if !updated {
        servers.push(Nbt::Compound(vec![
            ("name".to_string(), Nbt::String(entry_name.to_string())),
            ("ip".to_string(), Nbt::String(endpoint.address())),
            ("acceptTextures".to_string(), Nbt::Byte(0)),
        ]));
    }

    let encoded = encode_nbt(&root)?;
    if path.exists() {
        let _ = std::fs::copy(&path, path.with_extension("dat.bak"));
    }
    write_bytes_atomic(&path, &encoded)?;
    logger.info(
        "server",
        format!("Multiplayer entry \"{entry_name}\" updated"),
    );
    Ok(path)
}

fn set_string_field(fields: &mut Vec<(String, Nbt)>, key: &str, value: String) {
    if let Some((_, existing)) = fields.iter_mut().find(|(name, _)| name == key) {
        *existing = Nbt::String(value);
    } else {
        fields.push((key.to_string(), Nbt::String(value)));
    }
}

fn backup_corrupt(path: &Path, logger: &Arc<Logger>) {
    let name = format!(
        "servers.dat.corrupt-{}",
        format_compact_utc(now_ms())
    );
    let backup = path.with_file_name(name);
    if std::fs::copy(path, &backup).is_ok() {
        logger.warn(
            "server",
            format!(
                "Previous servers.dat preserved as {}",
                backup.file_name().and_then(|n| n.to_str()).unwrap_or("backup")
            ),
        );
    }
}

/// Reads the configured named entries. Used by diagnostics and tests.
pub fn read_server_entries(game_dir: &Path) -> AppResult<Vec<(String, String)>> {
    let path = game_dir.join("servers.dat");
    let Some(root) = read_servers_dat(&path)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Some(Nbt::List(servers)) = root.get("servers") {
        for server in servers {
            if let Nbt::Compound(fields) = server {
                let name = fields
                    .iter()
                    .find(|(name, _)| name == "name")
                    .and_then(|(_, value)| value.string_value())
                    .unwrap_or("")
                    .to_string();
                let ip = fields
                    .iter()
                    .find(|(name, _)| name == "ip")
                    .and_then(|(_, value)| value.string_value())
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    out.push((name, ip));
                }
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    /// "online", "offline" or "unknown".
    pub state: String,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub motd: Option<String>,
    pub version_name: Option<String>,
    pub latency_ms: Option<u64>,
    pub checked_at_ms: u64,
    pub error_code: Option<String>,
}

impl ServerStatus {
    pub fn unknown() -> Self {
        Self {
            state: "unknown".to_string(),
            players_online: None,
            players_max: None,
            motd: None,
            version_name: None,
            latency_ms: None,
            checked_at_ms: now_ms(),
            error_code: Some("server_not_configured".to_string()),
        }
    }

    fn offline() -> Self {
        Self {
            state: "offline".to_string(),
            players_online: None,
            players_max: None,
            motd: None,
            version_name: None,
            latency_ms: None,
            checked_at_ms: now_ms(),
            error_code: Some("server_offline".to_string()),
        }
    }
}

fn write_varint(value: i32, out: &mut Vec<u8>) {
    let mut value = value as u32;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

pub(crate) fn read_varint<R: Read>(reader: &mut R) -> AppResult<i32> {
    let mut result: u32 = 0;
    for shift in (0..35).step_by(7) {
        let byte = read_u8(reader)?;
        result |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return Ok(result as i32);
        }
        if shift == 28 {
            return Err(AppError::new("invalid_packet"));
        }
    }
    Err(AppError::new("invalid_packet"))
}

fn write_string_packet(value: &str, out: &mut Vec<u8>) {
    write_varint(value.len() as i32, out);
    out.extend_from_slice(value.as_bytes());
}

pub fn ping(endpoint: &ServerEndpoint, timeout: Duration) -> ServerStatus {
    match ping_inner(endpoint, timeout) {
        Ok(status) => status,
        Err(_) => ServerStatus::offline(),
    }
}

fn ping_inner(endpoint: &ServerEndpoint, timeout: Duration) -> AppResult<ServerStatus> {
    let started = Instant::now();
    let addresses = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| AppError::with_detail("network_error", error.to_string()))?;
    let per_attempt = (timeout / 3).max(Duration::from_millis(500));
    let mut stream: Option<TcpStream> = None;
    for address in addresses {
        if let Ok(candidate) = TcpStream::connect_timeout(&address, per_attempt) {
            stream = Some(candidate);
            break;
        }
    }
    let mut stream = stream.ok_or_else(|| AppError::new("server_offline"))?;
    stream.set_read_timeout(Some(timeout.max(Duration::from_millis(500))))?;
    stream.set_write_timeout(Some(timeout.max(Duration::from_millis(500))))?;

    let mut handshake = Vec::new();
    write_varint(0, &mut handshake);
    write_varint(767, &mut handshake);
    write_string_packet(&endpoint.host, &mut handshake);
    handshake.extend_from_slice(&endpoint.port.to_be_bytes());
    write_varint(1, &mut handshake);
    let mut packet = Vec::new();
    write_varint(handshake.len() as i32, &mut packet);
    packet.extend_from_slice(&handshake);
    stream.write_all(&packet)?;

    let mut request = Vec::new();
    write_varint(0, &mut request);
    let mut framed = Vec::new();
    write_varint(request.len() as i32, &mut framed);
    framed.extend_from_slice(&request);
    stream.write_all(&framed)?;

    let length = read_varint(&mut stream)?;
    if !(1..=2 * 1024 * 1024).contains(&length) {
        return Err(AppError::new("invalid_packet"));
    }
    let mut body = vec![0u8; length as usize];
    stream.read_exact(&mut body)?;
    let mut cursor = Cursor::new(body);
    let packet_id = read_varint(&mut cursor)?;
    if packet_id != 0 {
        return Err(AppError::new("invalid_packet"));
    }
    let json_length = read_varint(&mut cursor)?;
    if !(0..=2 * 1024 * 1024).contains(&json_length) {
        return Err(AppError::new("invalid_packet"));
    }
    let mut json_bytes = vec![0u8; json_length as usize];
    cursor.read_exact(&mut json_bytes)?;
    let value: Value = serde_json::from_slice(&json_bytes)?;

    let players_online = value
        .get("players")
        .and_then(|p| p.get("online"))
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());
    let players_max = value
        .get("players")
        .and_then(|p| p.get("max"))
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());
    let version_name = value
        .get("version")
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .map(|s| s.chars().take(64).collect());
    let motd = value
        .get("description")
        .map(|description| flatten_chat(description, 0))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(300).collect());

    Ok(ServerStatus {
        state: "online".to_string(),
        players_online,
        players_max,
        motd,
        version_name,
        latency_ms: Some(started.elapsed().as_millis() as u64),
        checked_at_ms: now_ms(),
        error_code: None,
    })
}

fn flatten_chat(value: &Value, depth: usize) -> String {
    if depth > 8 {
        return String::new();
    }
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| flatten_chat(item, depth + 1))
            .collect::<Vec<_>>()
            .join(""),
        Value::Object(map) => {
            let mut text = map
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(extra) = map.get("extra") {
                text.push_str(&flatten_chat(extra, depth + 1));
            }
            text
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn logger(dir: &Path) -> Arc<Logger> {
        Logger::new(dir.join("logs")).unwrap()
    }

    #[test]
    fn server_entry_is_created_and_updated() {
        let dir = tempfile::tempdir().unwrap();
        let log = logger(dir.path());
        let endpoint = ServerEndpoint::parse("skyandbeyond.playit.gg", 25565).unwrap();
        update_server_entry(dir.path(), "Sky & Beyond", &endpoint, &log).unwrap();
        let entries = read_server_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "Sky & Beyond");
        assert_eq!(entries[0].1, "skyandbeyond.playit.gg:25565");

        let endpoint = ServerEndpoint::parse("new.playit.gg", 25566).unwrap();
        update_server_entry(dir.path(), "Sky & Beyond", &endpoint, &log).unwrap();
        let entries = read_server_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "new.playit.gg:25566");
    }

    #[test]
    fn preserves_other_entries_and_custom_fields() {
        let dir = tempfile::tempdir().unwrap();
        let log = logger(dir.path());
        let root = Nbt::Compound(vec![(
            "servers".to_string(),
            Nbt::List(vec![
                Nbt::Compound(vec![
                    ("name".to_string(), Nbt::String("Other".to_string())),
                    ("ip".to_string(), Nbt::String("other.example.org:25565".to_string())),
                    ("custom".to_string(), Nbt::Int(42)),
                ]),
                Nbt::Compound(vec![
                    ("name".to_string(), Nbt::String("Sky & Beyond".to_string())),
                    ("ip".to_string(), Nbt::String("old.playit.gg:1".to_string())),
                    ("icon".to_string(), Nbt::String("data:image/png;base64,AA==".to_string())),
                ]),
            ]),
        )]);
        std::fs::write(dir.path().join("servers.dat"), encode_nbt(&root).unwrap()).unwrap();
        let endpoint = ServerEndpoint::parse("new.playit.gg", 25567).unwrap();
        update_server_entry(dir.path(), "Sky & Beyond", &endpoint, &log).unwrap();
        let bytes = std::fs::read(dir.path().join("servers.dat")).unwrap();
        let root = parse_nbt(&bytes).unwrap();
        let Nbt::List(servers) = root.get("servers").unwrap() else {
            panic!("servers list");
        };
        assert_eq!(servers.len(), 2);
        let other = servers[0].compound().unwrap();
        assert_eq!(other.iter().find(|(k, _)| k == "custom").unwrap().1, Nbt::Int(42));
        let target = servers[1].compound().unwrap();
        assert!(target.iter().any(|(k, _)| k == "icon"));
        assert_eq!(
            target.iter().find(|(k, _)| k == "ip").unwrap().1,
            Nbt::String("new.playit.gg:25567".to_string())
        );
    }

    #[test]
    fn corrupt_servers_dat_is_backed_up_before_replacement() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("servers.dat"), b"not nbt at all").unwrap();
        let log = logger(dir.path());
        let endpoint = ServerEndpoint::parse("skyandbeyond.playit.gg", 25565).unwrap();
        update_server_entry(dir.path(), "Sky & Beyond", &endpoint, &log).unwrap();
        let entries = read_server_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains("corrupt")));
    }

    #[test]
    fn nbt_round_trip_covers_types() {
        let root = Nbt::Compound(vec![
            ("byte".to_string(), Nbt::Byte(-4)),
            ("short".to_string(), Nbt::Short(300)),
            ("int".to_string(), Nbt::Int(-99_999)),
            ("long".to_string(), Nbt::Long(123_456_789_012)),
            ("float".to_string(), Nbt::Float(1.5)),
            ("double".to_string(), Nbt::Double(2.25)),
            ("string".to_string(), Nbt::String("héllo".to_string())),
            (
                "list".to_string(),
                Nbt::List(vec![Nbt::Int(1), Nbt::Int(2)]),
            ),
            (
                "compound".to_string(),
                Nbt::Compound(vec![("nested".to_string(), Nbt::Byte(1))]),
            ),
            ("ints".to_string(), Nbt::IntArray(vec![1, 2, 3])),
            ("longs".to_string(), Nbt::LongArray(vec![4, 5])),
            ("bytes".to_string(), Nbt::ByteArray(vec![9, 8, 7])),
        ]);
        let encoded = encode_nbt(&root).unwrap();
        assert_eq!(parse_nbt(&encoded).unwrap(), root);
    }

    #[test]
    fn varints_round_trip() {
        for value in [0, 1, 127, 128, 255, 16_777_215, 767] {
            let mut encoded = Vec::new();
            write_varint(value, &mut encoded);
            let decoded = read_varint(&mut Cursor::new(encoded)).unwrap();
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn chat_components_are_flattened() {
        let value = serde_json::json!({
            "text": "Sky ",
            "extra": ["& ", {"text": "Beyond", "extra": [{"text": "!"}]}]
        });
        assert_eq!(flatten_chat(&value, 0), "Sky & Beyond!");
    }

    fn serve_status_once(players: u32) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let length = read_varint(&mut stream).unwrap();
            let mut handshake = vec![0u8; length as usize];
            stream.read_exact(&mut handshake).unwrap();
            let length = read_varint(&mut stream).unwrap();
            let mut request = vec![0u8; length as usize];
            stream.read_exact(&mut request).unwrap();

            let payload = serde_json::json!({
                "version": {"name": "1.21.1", "protocol": 767},
                "players": {"online": players, "max": 100},
                "description": {"text": "Bienvenue ", "extra": [{"text": "sur Sky & Beyond"}]}
            })
            .to_string();
            let mut body = Vec::new();
            write_varint(0, &mut body);
            write_string_packet(&payload, &mut body);
            let mut packet = Vec::new();
            write_varint(body.len() as i32, &mut packet);
            packet.extend_from_slice(&body);
            stream.write_all(&packet).unwrap();
            let _ = stream.flush();
        });
        (format!("127.0.0.1:{port}"), handle)
    }

    #[test]
    fn pings_a_mock_minecraft_server() {
        let (address, handle) = serve_status_once(42);
        let (host, port) = address.split_once(':').unwrap();
        let endpoint = ServerEndpoint {
            host: host.to_string(),
            port: port.parse().unwrap(),
        };
        let status = ping(&endpoint, Duration::from_secs(2));
        handle.join().unwrap();
        assert_eq!(status.state, "online");
        assert_eq!(status.players_online, Some(42));
        assert_eq!(status.players_max, Some(100));
        assert_eq!(status.version_name.as_deref(), Some("1.21.1"));
        assert_eq!(status.motd.as_deref(), Some("Bienvenue sur Sky & Beyond"));
        assert!(status.latency_ms.is_some());
    }

    #[test]
    fn offline_status_never_contains_the_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let endpoint = ServerEndpoint {
            host: "127.0.0.1".to_string(),
            port,
        };
        let status = ping(&endpoint, Duration::from_millis(400));
        assert_eq!(status.state, "offline");
        assert_eq!(status.error_code.as_deref(), Some("server_offline"));
        assert!(status.motd.is_none());
    }
}
