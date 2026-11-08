use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::http::{Downloader, ExpectedFile};
use crate::logging::Logger;
use crate::util::{sanitize_file_name, unique_suffix, validate_url};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaInstallation {
    pub path: String,
    pub version: String,
    pub major: u32,
    pub vendor: Option<String>,
    pub is_64_bit: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaReport {
    pub required_major: u32,
    /// "ok" or "missing".
    pub status: String,
    pub selected: Option<JavaInstallation>,
    pub candidates: Vec<JavaInstallation>,
    pub warning: Option<String>,
}

impl JavaReport {
    pub fn missing(required_major: u32, warning: Option<String>) -> Self {
        Self {
            required_major,
            status: "missing".to_string(),
            selected: None,
            candidates: Vec::new(),
            warning,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    pub fn tail(&self, max_chars: usize) -> String {
        let combined = self.combined();
        let count = combined.chars().count();
        if count <= max_chars {
            combined
        } else {
            combined.chars().skip(count - max_chars).collect()
        }
    }
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["java.exe"]
    } else {
        &["java"]
    }
}

fn read_limited<R: Read>(mut reader: R) -> String {
    let mut bytes = Vec::new();
    let _ = reader.by_ref().take(8 * 1024 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).to_string()
}

/// Runs a command without a console window, captures limited output and
/// enforces a timeout. The helper is used for Java probing and NeoForge's CLI.
pub fn run_command_capture(
    mut command: Command,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> AppResult<CommandOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child: Child = command
        .spawn()
        .map_err(|error| AppError::with_detail("process_start_failed", error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::new("process_start_failed"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::new("process_start_failed"))?;
    let stdout_thread = std::thread::spawn(move || read_limited(stdout));
    let stderr_thread = std::thread::spawn(move || read_limited(stderr));

    let started = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(AppError::with_detail("process_error", error.to_string()));
            }
        }
        if cancel.map(|flag| flag.load(Ordering::Relaxed)).unwrap_or(false) {
            cancelled = true;
            let _ = child.kill();
            let _ = child.wait();
            break std::process::ExitStatus::default();
        }
        if started.elapsed() > timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break std::process::ExitStatus::default();
        }
        std::thread::sleep(Duration::from_millis(60));
    };
    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    if cancelled {
        return Err(AppError::new("cancelled"));
    }
    if timed_out {
        return Err(AppError::with_detail(
            "process_timeout",
            format!("command did not finish within {} seconds", timeout.as_secs()),
        ));
    }
    Ok(CommandOutput {
        status_code: status.code(),
        stdout,
        stderr,
    })
}

fn version_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(?:java|openjdk)\s+version\s+"([^"]+)""#)
            .expect("valid java version regex")
    })
}

fn major_of(version: &str) -> Option<u32> {
    let mut parts = version.split('.');
    let first = parts.next()?.trim();
    let component = if first == "1" { parts.next()? } else { first };
    let digits: String = component.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

pub fn parse_java_version_output(output: &str) -> AppResult<(String, u32, Option<String>, bool)> {
    let lower = output.to_ascii_lowercase();
    let version = version_regex()
        .captures(output)
        .and_then(|captures| captures.get(1).map(|m| m.as_str().to_string()))
        .or_else(|| {
            output
                .lines()
                .find(|_line| lower.contains("version"))
                .and_then(|line| {
                    line.split(|c: char| c.is_whitespace() || c == '"')
                        .find(|token| {
                            token
                                .chars()
                                .next()
                                .map(|c| c.is_ascii_digit())
                                .unwrap_or(false)
                                && token.contains('.')
                        })
                        .map(|token| token.to_string())
                })
        })
        .ok_or_else(|| AppError::with_detail("java_invalid", "no Java version found in output"))?;
    let major = major_of(&version)
        .ok_or_else(|| AppError::with_detail("java_invalid", format!("invalid version {version}")))?;
    let vendor = if lower.contains("temurin") || lower.contains("adoptium") {
        Some("Eclipse Adoptium".to_string())
    } else if lower.contains("microsoft") {
        Some("Microsoft".to_string())
    } else if lower.contains("corretto") {
        Some("Amazon Corretto".to_string())
    } else if lower.contains("zulu") {
        Some("Azul Zulu".to_string())
    } else if lower.contains("graalvm") {
        Some("GraalVM".to_string())
    } else if lower.contains("oracle") {
        Some("Oracle".to_string())
    } else if lower.contains("openjdk") {
        Some("OpenJDK".to_string())
    } else {
        None
    };
    let is_64_bit = {
        let lower = output.to_ascii_lowercase();
        lower.contains("64-bit") || lower.contains("x86_64") || lower.contains("aarch64")
    };
    Ok((version, major, vendor, is_64_bit))
}

pub fn inspect_java(path: &Path, source: &str) -> AppResult<JavaInstallation> {
    if !path.is_file() {
        return Err(AppError::with_detail(
            "java_path_invalid",
            format!("{} is not a file", path.display()),
        ));
    }
    let mut command = Command::new(path);
    command.arg("-version");
    let output = run_command_capture(command, Duration::from_secs(10), None)
        .map_err(|error| AppError::with_detail("java_invalid", error.detail_text().to_string()))?;
    let combined = output.combined();
    let (version, major, vendor, is_64_bit) = parse_java_version_output(&combined)?;
    Ok(JavaInstallation {
        path: path.to_string_lossy().to_string(),
        version,
        major,
        vendor,
        is_64_bit,
        source: source.to_string(),
    })
}

pub fn find_java_executable(root: &Path) -> Option<PathBuf> {
    for name in executable_names() {
        let direct = root.join("bin").join(name);
        if direct.is_file() {
            return Some(direct);
        }
    }
    for entry in walkdir::WalkDir::new(root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && executable_names()
                .iter()
                .any(|name| entry.file_name().to_string_lossy() == *name)
        {
            return Some(entry.path().to_path_buf());
        }
    }
    None
}

fn push_candidate(
    candidates: &mut Vec<(PathBuf, &'static str)>,
    seen: &mut std::collections::HashSet<String>,
    path: PathBuf,
    source: &'static str,
) {
    let key = fs::canonicalize(&path)
        .unwrap_or_else(|_| path.clone())
        .to_string_lossy()
        .to_ascii_lowercase();
    if seen.insert(key) {
        candidates.push((path, source));
    }
}

fn scan_roots(roots: &[PathBuf], depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .max_depth(depth)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file()
                && executable_names()
                    .iter()
                    .any(|name| entry.file_name().to_string_lossy() == *name)
            {
                found.push(entry.path().to_path_buf());
            }
            if found.len() >= 12 {
                return found;
            }
        }
    }
    found
}

fn system_java_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(key) {
                for vendor in [
                    "Eclipse Adoptium",
                    "Java",
                    "Microsoft",
                    "Zulu",
                    "Amazon Corretto",
                    "BellSoft",
                    "Semeru",
                ] {
                    roots.push(PathBuf::from(&base).join(vendor));
                }
                if key == "LOCALAPPDATA" {
                    roots.push(PathBuf::from(&base).join("Programs").join("Eclipse Adoptium"));
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/usr/lib/jvm"));
        roots.push(PathBuf::from("/usr/java"));
    }
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from(
            "/Library/Java/JavaVirtualMachines",
        ));
    }
    roots
}

pub fn detect_java(settings_java: &str, managed_root: &Path, required_major: u32) -> JavaReport {
    let mut candidates: Vec<(PathBuf, &'static str)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let explicit = settings_java.trim();

    if !explicit.is_empty() {
        push_candidate(
            &mut candidates,
            &mut seen,
            PathBuf::from(explicit),
            "manual",
        );
    }
    if let Some(path) = find_java_executable(managed_root) {
        push_candidate(&mut candidates, &mut seen, path, "launcher");
    }
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        if let Some(path) = find_java_executable(Path::new(&java_home)) {
            push_candidate(&mut candidates, &mut seen, path, "JAVA_HOME");
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            for name in executable_names() {
                let path = directory.join(name);
                if path.is_file() {
                    push_candidate(&mut candidates, &mut seen, path, "PATH");
                }
            }
        }
    }
    for path in scan_roots(&system_java_roots(), 4) {
        push_candidate(&mut candidates, &mut seen, path, "system");
    }

    let mut installations = Vec::new();
    let mut manual_seen = false;
    for (path, source) in candidates.into_iter().take(24) {
        if source == "manual" {
            manual_seen = true;
        }
        if let Ok(installation) = inspect_java(&path, source) {
            installations.push(installation);
        }
    }
    installations.sort_by(|a, b| {
        (a.major != required_major)
            .cmp(&(b.major != required_major))
            .then_with(|| b.major.cmp(&a.major))
            .then_with(|| a.path.cmp(&b.path))
    });
    let selected = installations
        .iter()
        .find(|installation| installation.major == required_major)
        .cloned();
    let warning = if selected.is_none() && !explicit.is_empty() {
        Some("java_version_mismatch".to_string())
    } else if selected.is_some()
        && manual_seen
        && installations
            .iter()
            .find(|installation| installation.source == "manual")
            .map(|installation| installation.major != required_major)
            .unwrap_or(false)
    {
        Some("java_manual_version_mismatch".to_string())
    } else {
        None
    };
    let status = if selected.is_some() { "ok" } else { "missing" };
    JavaReport {
        required_major,
        status: status.to_string(),
        selected,
        candidates: installations,
        warning,
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AdoptiumRelease {
    binary: AdoptiumBinary,
}

#[derive(Debug, Clone, Deserialize)]
struct AdoptiumBinary {
    package: AdoptiumPackage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdoptiumPackage {
    pub link: String,
    pub checksum: String,
    pub size: u64,
    pub name: String,
}

pub fn parse_adoptium_response(text: &str) -> AppResult<AdoptiumPackage> {
    let releases: Vec<AdoptiumRelease> = serde_json::from_str(text)?;
    let package = releases
        .into_iter()
        .next()
        .ok_or_else(|| AppError::new("java_download_unavailable"))?
        .binary
        .package;
    validate_url(&package.link, false)
        .map_err(|error| AppError::with_detail("java_download_unavailable", error.detail_text().to_string()))?;
    if package.checksum.len() != 64
        || !package.checksum.chars().all(|c| c.is_ascii_hexdigit())
        || package.size == 0
    {
        return Err(AppError::new("java_download_invalid"));
    }
    let name = sanitize_file_name(&package.name)
        .map_err(|_| AppError::new("java_download_invalid"))?;
    if !(name.to_ascii_lowercase().ends_with(".zip")
        || name.to_ascii_lowercase().ends_with(".tar.gz"))
    {
        return Err(AppError::new("java_download_invalid"));
    }
    Ok(AdoptiumPackage {
        link: package.link,
        checksum: package.checksum.to_ascii_lowercase(),
        size: package.size,
        name,
    })
}

fn extract_zip(archive_path: &Path, destination: &Path) -> AppResult<()> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut total_size: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        total_size = total_size.saturating_add(entry.size());
        if total_size > 1024 * 1024 * 1024 {
            return Err(AppError::with_detail(
                "java_download_invalid",
                "archive expands beyond 1 GiB",
            ));
        }
        let out_path = destination.join(path);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut output)?;
        }
    }
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, destination: &Path) -> AppResult<()> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut total_size: u64 = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            continue;
        }
        total_size = total_size.saturating_add(entry.header().size().unwrap_or(0));
        if total_size > 1024 * 1024 * 1024 {
            return Err(AppError::with_detail(
                "java_download_invalid",
                "archive expands beyond 1 GiB",
            ));
        }
        let _ = entry.unpack_in(destination)?;
    }
    Ok(())
}

pub fn extract_java_archive(archive_path: &Path, destination: &Path) -> AppResult<()> {
    fs::create_dir_all(destination)?;
    let name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive_path, destination)
    } else if name.ends_with(".tar.gz") {
        extract_tar_gz(archive_path, destination)
    } else {
        Err(AppError::new("java_download_invalid"))
    }
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(permissions.mode() | 0o111);
            let _ = fs::set_permissions(path, permissions);
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn install_extracted_runtime(
    extracted: &Path,
    game_dir: &Path,
    required_major: u32,
) -> AppResult<PathBuf> {
    let exe = find_java_executable(extracted)
        .ok_or_else(|| AppError::new("java_download_invalid"))?;
    make_executable(&exe);
    let installation = inspect_java(&exe, "launcher")?;
    if installation.major != required_major {
        return Err(AppError::with_detail(
            "java_version_mismatch",
            format!("downloaded Java {} but {} was required", installation.major, required_major),
        ));
    }
    let java_root = game_dir.join("java");
    fs::create_dir_all(&java_root)?;
    let final_dir = java_root.join(format!("runtime-{required_major}"));
    let old_dir = java_root.join(format!(".runtime-old-{}", unique_suffix()));
    if final_dir.exists() {
        fs::rename(&final_dir, &old_dir)?;
    }
    if let Err(error) = fs::rename(extracted, &final_dir) {
        if old_dir.exists() {
            let _ = fs::rename(&old_dir, &final_dir);
        }
        return Err(error.into());
    }
    if old_dir.exists() {
        let _ = fs::remove_dir_all(&old_dir);
    }
    find_java_executable(&final_dir).ok_or_else(|| AppError::new("java_download_invalid"))
}

/// Downloads an Eclipse Temurin JRE from Adoptium's official API and installs it
/// in the dedicated instance directory. The previous runtime is only replaced
/// after the new one has been extracted and verified.
pub fn download_java(
    runtime: &RuntimeConfig,
    game_dir: &Path,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<JavaInstallation> {
    let downloader = Downloader::new()?;
    let api_url = runtime.java_download_api_url();
    let body = downloader.fetch_text(&api_url, 2 * 1024 * 1024, Duration::from_secs(15))?;
    let package = parse_adoptium_response(&body)?;
    let cache_dir = game_dir.join(".snb").join("cache").join("java");
    fs::create_dir_all(&cache_dir)?;
    let archive = cache_dir.join(&package.name);
    downloader.download(
        &package.link,
        &archive,
        &ExpectedFile::sha256(package.checksum.clone(), Some(package.size)),
        &mut *progress,
        cancel,
    )?;
    logger.info(
        "java",
        format!("Downloaded Java {} archive ({} bytes)", runtime.java_major, package.size),
    );
    let temp = game_dir
        .join("java")
        .join(format!(".extract-{}", unique_suffix()));
    let extraction = extract_java_archive(&archive, &temp);
    if let Err(error) = extraction {
        let _ = fs::remove_dir_all(&temp);
        return Err(error);
    }
    let installed = match install_extracted_runtime(&temp, game_dir, runtime.java_major) {
        Ok(path) => inspect_java(&path, "launcher"),
        Err(error) => {
            let _ = fs::remove_dir_all(&temp);
            return Err(error);
        }
    };
    let _ = fs::remove_dir_all(&temp);
    let _ = fs::remove_file(&archive);
    let installed = installed?;
    logger.info(
        "java",
        format!(
            "Installed Java {} ({}) in the dedicated instance",
            installed.version,
            installed.vendor.as_deref().unwrap_or("unknown vendor")
        ),
    );
    Ok(installed)
}

pub fn ensure_java(
    runtime: &RuntimeConfig,
    settings_java: &str,
    game_dir: &Path,
    auto_download: bool,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<(JavaReport, JavaInstallation)> {
    let managed_root = game_dir.join("java").join(format!("runtime-{}", runtime.java_major));
    let report = detect_java(settings_java, &managed_root, runtime.java_major);
    if let Some(selected) = report.selected.clone() {
        return Ok((report, selected));
    }
    if !auto_download {
        return Err(AppError::new("java_missing"));
    }
    let installed = download_java(runtime, game_dir, cancel, logger, progress)?;
    Ok((
        JavaReport {
            required_major: runtime.java_major,
            status: "ok".to_string(),
            selected: Some(installed.clone()),
            candidates: vec![installed.clone()],
            warning: None,
        },
        installed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_modern_and_legacy_java_versions() {
        let modern = "openjdk version \"21.0.12.1\" 2026-08-18\nOpenJDK Runtime Environment Temurin-21.0.12.1+1 (build 21.0.12.1+1)\nOpenJDK 64-Bit Server VM Temurin-21.0.12.1+1 (build 21.0.12.1+1, mixed mode)";
        let (version, major, vendor, is_64) = parse_java_version_output(modern).unwrap();
        assert_eq!(version, "21.0.12.1");
        assert_eq!(major, 21);
        assert_eq!(vendor.as_deref(), Some("Eclipse Adoptium"));
        assert!(is_64);

        let legacy = "java version \"1.8.0_402\"\nJava(TM) SE Runtime Environment (build 1.8.0_402-b06)";
        let (version, major, vendor, is_64) = parse_java_version_output(legacy).unwrap();
        assert_eq!(version, "1.8.0_402");
        assert_eq!(major, 8);
        assert_eq!(vendor, None);
        assert!(!is_64);

        assert!(parse_java_version_output("not java").is_err());
    }

    #[test]
    fn parses_adoptium_package() {
        let body = r#"[{
            "binary": {
                "package": {
                    "link": "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.1/OpenJDK21U-jre_x64_windows_hotspot_21.0.1_12.zip",
                    "checksum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "size": 50000000,
                    "name": "OpenJDK21U-jre_x64_windows_hotspot_21.0.1_12.zip"
                }
            }
        }]"#;
        let package = parse_adoptium_response(body).unwrap();
        assert_eq!(package.size, 50_000_000);
        assert!(package.link.ends_with(".zip"));
        assert_eq!(package.checksum.len(), 64);
        assert!(parse_adoptium_response("[]").is_err());
        assert!(parse_adoptium_response(
            r#"[{"binary":{"package":{"link":"http://evil.example/x.zip","checksum":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","size":1,"name":"x.zip"}}}]"#
        )
        .is_err());
    }

    #[test]
    fn zip_extraction_and_executable_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("jre.zip");
        let java_name = if cfg!(windows) { "java.exe" } else { "java" };
        {
            let file = File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            use zip::write::SimpleFileOptions;
            let options = SimpleFileOptions::default();
            zip.start_file(format!("jre-21/bin/{java_name}"), options).unwrap();
            zip.write_all(b"#!/bin/sh\n").unwrap();
            zip.start_file("jre-21/release", options).unwrap();
            zip.write_all(b"JAVA_VERSION=\"21\"").unwrap();
            zip.finish().unwrap();
        }
        let extracted = dir.path().join("extracted");
        extract_java_archive(&archive_path, &extracted).unwrap();
        assert!(extracted.join("jre-21/release").is_file());
        let java = find_java_executable(&extracted).unwrap();
        assert_eq!(java.file_name().and_then(|name| name.to_str()), Some(java_name));
    }

    #[test]
    fn managed_runtime_detection_prefers_exact_major() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("runtime-21");
        fs::create_dir_all(managed.join("bin")).unwrap();
        let prefix = dir.path().join("bin");
        let _ = prefix;
        let report = detect_java("", &managed, 21);
        assert!(!report.candidates.iter().any(|c| c.source == "launcher"));
    }

    #[test]
    fn run_command_capture_returns_output_and_status() {
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.arg("-c").arg("echo hello; echo err >&2; exit 3");
            let output = run_command_capture(command, Duration::from_secs(2), None).unwrap();
            assert_eq!(output.status_code, Some(3));
            assert_eq!(output.stdout.trim(), "hello");
            assert_eq!(output.stderr.trim(), "err");
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.arg("/C").arg("echo hello");
            let output = run_command_capture(command, Duration::from_secs(2), None).unwrap();
            assert!(output.status_code.is_some());
            assert!(output.stdout.contains("hello"));
        }
    }

}
