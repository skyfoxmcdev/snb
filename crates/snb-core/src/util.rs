use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::net::Ipv6Addr;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use md5::Md5;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{AppError, AppResult};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn file_modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

/// Compact, sortable UTC timestamp used for backup and log file names.
pub fn format_compact_utc(ms: u64) -> String {
    let seconds = ms / 1_000;
    let days = (seconds / 86_400) as i64;
    let secs_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

pub fn format_readable_utc(ms: u64) -> String {
    let seconds = ms / 1_000;
    let days = (seconds / 86_400) as i64;
    let secs_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

pub fn unique_suffix() -> String {
    format!("{}-{}", std::process::id(), now_ms())
}

/// Writes bytes to `path` through a sibling temporary file and an atomic rename
/// where the platform supports it. A failed write never truncates the original.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::new("invalid_file_name"))?;
    let tmp = parent.join(format!(".{file_name}.{}.tmp", unique_suffix()));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(error.into());
    }
    // std::fs::rename replaces the target on Unix but not on Windows.
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error.into());
    }
    Ok(())
}

pub fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    path.with_file_name(format!("{name}.bak"))
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if path.is_file() {
        let _ = fs::copy(path, backup_path(path));
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes_atomic(path, &bytes)
}

pub fn write_json_without_backup<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes_atomic(path, &bytes)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> AppResult<T> {
    let bytes = fs::read(path)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(AppError::new("file_too_large"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Reads `path`, transparently falling back to the `.bak` copy written by the
/// previous atomic save. Returns whether the backup was used.
pub fn read_json_with_backup<T: DeserializeOwned>(path: &Path) -> Option<(T, bool)> {
    if let Ok(value) = read_json::<T>(path) {
        return Some((value, false));
    }
    if let Ok(value) = read_json::<T>(&backup_path(path)) {
        return Some((value, true));
    }
    None
}

fn hash_file_with<D: Digest>(path: &Path) -> AppResult<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = D::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn sha1_file(path: &Path) -> AppResult<String> {
    hash_file_with::<Sha1>(path)
}

pub fn sha256_file(path: &Path) -> AppResult<String> {
    hash_file_with::<Sha256>(path)
}

pub fn sha1_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn md5_hex(bytes: &[u8]) -> String {
    format!("{:x}", Md5::digest(bytes))
}

/// Java's `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`.
pub fn offline_uuid(username: &str) -> String {
    let digest = Md5::digest(format!("OfflinePlayer:{username}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest);
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h = |range: std::ops::Range<usize>| {
        bytes[range]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    format!(
        "{}-{}-{}-{}-{}",
        h(0..4),
        h(4..6),
        h(6..8),
        h(8..10),
        h(10..16)
    )
}

pub fn username_is_valid(username: &str) -> bool {
    let len = username.chars().count();
    (3..=16).contains(&len)
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn sanitize_file_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 180
        || name.contains(['/', '\\', '\0'])
        || name.contains(':')
    {
        return Err(AppError::new("invalid_file_name"));
    }
    let upper = name.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem) {
        return Err(AppError::new("invalid_file_name"));
    }
    Ok(name.to_string())
}

/// Joins only normal path components, refusing absolute paths, `..` and root
/// prefixes. Used whenever a name could come from a file or an archive.
pub fn safe_join(base: &Path, relative: &Path) -> AppResult<PathBuf> {
    let mut out = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return Err(AppError::new("unsafe_path")),
        }
    }
    Ok(out)
}

pub fn is_valid_url(value: &str, allow_loopback_http: bool) -> bool {
    validate_url(value, allow_loopback_http).is_ok()
}

pub fn validate_url(value: &str, allow_loopback_http: bool) -> AppResult<Url> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::new("invalid_url"));
    }
    let parsed = Url::parse(value)?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::with_detail(
            "invalid_url",
            "URLs with embedded credentials are not allowed",
        ));
    }
    match parsed.scheme() {
        "https" => {
            if parsed.host_str().is_none() {
                return Err(AppError::new("invalid_url"));
            }
        }
        "http" if allow_loopback_http && url_is_loopback(&parsed) => {}
        _ => return Err(AppError::with_detail("invalid_url", "Only HTTPS URLs are allowed")),
    }
    Ok(parsed)
}

pub fn url_is_loopback(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false),
        None => false,
    }
}

pub fn available_space(path: &Path) -> AppResult<u64> {
    let mut current = path;
    loop {
        if current.exists() {
            return fs2::available_space(current).map_err(|e| {
                AppError::with_detail("disk_space_unavailable", format!("{current:?}: {e}"))
            });
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Err(AppError::new("disk_space_unavailable")),
        }
    }
}

/// Splits a user-entered argument string, honouring single/double quotes and
/// backslashes inside Windows paths. It intentionally does not invoke a shell.
pub fn split_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_double && ch == '\\' {
            match chars.peek() {
                Some('"') | Some('\\') => {
                    current.push(chars.next().unwrap_or('\\'));
                    has_token = true;
                }
                _ => {
                    current.push('\\');
                    has_token = true;
                }
            }
            continue;
        }
        match ch {
            '"' if !in_single => {
                in_double = !in_double;
                has_token = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                has_token = true;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(current);
    }
    out
}

pub fn copy_dir_all(src: &Path, dst: &Path, overwrite: bool) -> AppResult<()> {
    if !src.is_dir() {
        return Err(AppError::with_detail(
            "file_not_found",
            format!("{src:?} is not a directory"),
        ));
    }
    fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap_or(Path::new(""));
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            fs::create_dir_all(&target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if target.exists() && !overwrite {
                continue;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn ipv4_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])(?:\.(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])){3}\b")
            .expect("valid IPv4 regex")
    })
}

fn ipv6_candidate_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\[[0-9a-f:.]{2,60}\]|[0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7}")
            .expect("valid IPv6 regex")
    })
}

fn secret_line_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?im)^([^\r\n]{0,140}?\b(?:access[_-]?token|authorization|password|passwd|secret|api[_-]?key|access[_-]?key|bearer)\b[^\r\n:=]{0,50}[:=]\s*)[^\r\n]*$",
        )
        .expect("valid secret regex")
    })
}

fn bearer_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}").expect("valid bearer regex")
    })
}

fn query_secret_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)([?&](?:token|access_token|key|secret|password)=)[^&\s#]+")
            .expect("valid query secret regex")
    })
}

fn is_ipv6_candidate(token: &str) -> bool {
    let trimmed = token.trim_matches(|c| c == '[' || c == ']');
    trimmed.parse::<Ipv6Addr>().is_ok()
}

pub fn contains_ip_literal(text: &str) -> bool {
    if ipv4_regex().is_match(text) {
        return true;
    }
    ipv6_candidate_regex()
        .find_iter(text)
        .any(|m| is_ipv6_candidate(m.as_str()))
}

/// Removes IP literals and obvious credentials. Applied to every log line and
/// to diagnostic archives as defence in depth.
pub fn redact_text(text: &str) -> String {
    let step1 = bearer_regex().replace_all(text, "Bearer [masque]");
    let step2 = query_secret_regex().replace_all(&step1, "${1}[masque]");
    let step3 = secret_line_regex().replace_all(&step2, "${1}[masque]");
    let step4 = ipv6_candidate_regex().replace_all(&step3, |caps: &regex::Captures<'_>| {
        let token = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        if is_ipv6_candidate(token) {
            "[ip masquee]".to_string()
        } else {
            token.to_string()
        }
    });
    ipv4_regex()
        .replace_all(&step4, "[ip masquee]")
        .into_owned()
}

pub fn redact_paths(text: &str, replacements: &[(PathBuf, &str)]) -> String {
    let mut out = text.to_string();
    let mut items: Vec<&(PathBuf, &str)> = replacements.iter().collect();
    items.sort_by_key(|(path, _)| std::cmp::Reverse(path.as_os_str().len()));
    for (path, replacement) in items {
        if let Some(path_str) = path.to_str() {
            if !path_str.is_empty() {
                out = out.replace(path_str, replacement);
                out = out.replace(&path_str.replace('\\', "/"), replacement);
            }
        }
    }
    out
}

pub fn is_vanilla_minecraft_dir(path: &Path) -> bool {
    // Split on both separators so validation also works with a path written on
    // another platform (for example a Windows path in a test or a migrated config).
    let value = path.to_string_lossy();
    let name = value
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("");
    name.eq_ignore_ascii_case(".minecraft")
}

pub fn default_game_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".skyandbeyond")
}

pub fn human_file_name_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let name = parsed.path_segments()?.next_back()?.trim();
    if name.is_empty() {
        None
    } else {
        sanitize_file_name(name).ok()
    }
}

/// Returns true when the given process id no longer exists.
#[cfg(unix)]
pub fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // Signal 0 performs existence checking without sending a signal.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
pub fn pid_is_alive(pid: u32) -> bool {
    // Without platform bindings in this crate, use a conservative check. The
    // updater helper has a precise Windows implementation.
    pid != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_format_is_stable() {
        assert_eq!(format_compact_utc(0), "19700101-000000");
        assert_eq!(format_compact_utc(1_735_689_600_000), "20250101-000000");
        assert_eq!(format_readable_utc(1_735_689_600_000), "2025-01-01 00:00:00 UTC");
    }

    #[test]
    fn atomic_json_round_trip_and_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_json_atomic(&path, &serde_json::json!({"value": 1})).unwrap();
        write_json_atomic(&path, &serde_json::json!({"value": 2})).unwrap();
        let (value, used_backup): (serde_json::Value, bool) =
            read_json_with_backup(&path).unwrap();
        assert_eq!(value["value"], 2);
        assert!(!used_backup);
        fs::write(&path, b"{broken").unwrap();
        let (value, used_backup): (serde_json::Value, bool) =
            read_json_with_backup(&path).unwrap();
        assert_eq!(value["value"], 1);
        assert!(used_backup);
    }

    #[test]
    fn hashes_match_known_vectors() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha1_bytes(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn offline_uuid_is_version_3_and_deterministic() {
        let a = offline_uuid("Steve");
        let b = offline_uuid("Steve");
        assert_eq!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "3");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(offline_uuid("Steve"), offline_uuid("Alex"));
    }

    #[test]
    fn usernames_are_validated() {
        assert!(username_is_valid("Steve_123"));
        assert!(!username_is_valid("ab"));
        assert!(!username_is_valid("waytoolongusername123"));
        assert!(!username_is_valid("bad name"));
        assert!(!username_is_valid("émile"));
    }

    #[test]
    fn redacts_ipv4_ipv6_and_secrets() {
        let text = "connect 192.0.2.55 [2001:db8::1]:25565\naccessToken=super-secret\nBearer abcdefghijkl\n?token=abc123&x=1";
        let redacted = redact_text(text);
        assert!(!redacted.contains("192.0.2.55"));
        assert!(!redacted.contains("2001:db8::1"));
        assert!(!redacted.contains("super-secret"));
        assert!(!redacted.contains("abcdefghijkl"));
        assert!(!redacted.contains("abc123"));
        assert!(redacted.contains("25565"));
        // Timestamps must not be mistaken for IPv6 addresses.
        assert_eq!(redact_text("21:37:23"), "21:37:23");
    }

    #[test]
    fn detects_ip_literals() {
        assert!(contains_ip_literal("host 192.0.2.2"));
        assert!(contains_ip_literal("host 2001:db8::5"));
        assert!(contains_ip_literal("[2001:db8::5]:25565"));
        assert!(!contains_ip_literal("skyandbeyond.playit.gg:25565"));
        assert!(!contains_ip_literal("version 1.21.1 at 21:37:23"));
    }

    #[test]
    fn split_args_handles_quotes_and_windows_paths() {
        assert_eq!(
            split_args(r#"-Xmx4G "-Dfoo=a b" 'x y' C:\Java\bin\java.exe"#),
            vec!["-Xmx4G", "-Dfoo=a b", "x y", r"C:\Java\bin\java.exe"]
        );
        assert_eq!(split_args(r#""""#), vec![""]);
        assert_eq!(split_args("   "), Vec::<String>::new());
    }

    #[test]
    fn file_names_and_safe_joins_reject_traversal() {
        assert!(sanitize_file_name("mod.jar").is_ok());
        assert!(sanitize_file_name("../evil.jar").is_err());
        assert!(sanitize_file_name("CON.txt").is_err());
        let base = Path::new("/tmp/base");
        assert!(safe_join(base, Path::new("a/b.txt")).is_ok());
        assert!(safe_join(base, Path::new("../b.txt")).is_err());
        assert!(safe_join(base, Path::new("/abs")).is_err());
    }

    #[test]
    fn urls_require_https_except_loopback() {
        assert!(validate_url("https://example.com/a", false).is_ok());
        assert!(validate_url("http://example.com/a", false).is_err());
        assert!(validate_url("http://127.0.0.1:8080/config.json", true).is_ok());
        assert!(validate_url("http://127.0.0.1:8080/config.json", false).is_err());
        assert!(validate_url("https://user:pass@example.com/", false).is_err());
        assert!(validate_url("javascript:alert(1)", false).is_err());
    }

    #[test]
    fn vanilla_dir_detection() {
        assert!(is_vanilla_minecraft_dir(Path::new("/home/me/.minecraft")));
        assert!(is_vanilla_minecraft_dir(Path::new("C:\\Users\\me\\.MINECRAFT")));
        assert!(!is_vanilla_minecraft_dir(Path::new("/home/me/.skyandbeyond")));
    }
}
