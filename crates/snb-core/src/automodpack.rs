use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;

use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::http::{Downloader, ExpectedFile};
use crate::instance::GamePaths;
use crate::logging::Logger;
use crate::util::{sanitize_file_name, validate_url};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomodpackStatus {
    pub installed: bool,
    pub valid: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub pinned_version: String,
    pub message_code: Option<String>,
}

fn version_from_name(name: &str) -> Option<String> {
    let re = Regex::new(r"(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.]+)?)\.jar$").ok()?;
    re.captures(name)
        .and_then(|captures| captures.get(1).map(|m| m.as_str().to_string()))
}

pub fn jar_is_valid(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name().to_ascii_lowercase();
        if name.ends_with("meta-inf/neoforge.mods.toml") || name.ends_with("meta-inf/mods.toml") {
            return true;
        }
    }
    false
}

fn candidates(paths: &GamePaths) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(&paths.mods) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.starts_with("automodpack-") && lower.ends_with(".jar")
                })
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

pub fn detect(paths: &GamePaths, pinned_version: &str) -> AutomodpackStatus {
    let files = candidates(paths);
    let installed = !files.is_empty();
    for path in &files {
        if jar_is_valid(path) {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let version = version_from_name(&name);
            let pinned = version
                .as_deref()
                .map(|version| version == pinned_version)
                .unwrap_or(false);
            return AutomodpackStatus {
                installed,
                valid: true,
                path: Some(path.to_string_lossy().to_string()),
                version,
                pinned_version: pinned_version.to_string(),
                message_code: if pinned {
                    None
                } else {
                    Some("automodpack_version_differs".to_string())
                },
            };
        }
    }
    AutomodpackStatus {
        installed,
        valid: false,
        path: files.first().map(|path| path.to_string_lossy().to_string()),
        version: files
            .first()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .and_then(version_from_name),
        pinned_version: pinned_version.to_string(),
        message_code: if installed {
            Some("automodpack_corrupt".to_string())
        } else {
            Some("automodpack_missing".to_string())
        },
    }
}

/// Installs the pinned AutoModpack JAR only when it is absent or corrupt. A
/// valid JAR managed by AutoModpack's own updater is deliberately preserved,
/// even if its version differs, because server-side AutoModpack may have
/// synchronized it.
pub fn ensure_automodpack(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<AutomodpackStatus> {
    let status = detect(paths, &runtime.automodpack.version);
    if status.valid {
        progress(1, 1);
        if let Some(code) = &status.message_code {
            logger.warn(
                "automodpack",
                format!("{code}; keeping the AutoModpack-managed JAR"),
            );
        }
        return Ok(status);
    }
    validate_url(&runtime.automodpack.url, true)?;
    let file_name = sanitize_file_name(&runtime.automodpack.file_name)?;
    if !file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(AppError::new("automodpack_invalid"));
    }
    fs::create_dir_all(&paths.mods)?;
    let destination = paths.mods.join(&file_name);
    let downloader = Downloader::new()?;
    downloader.download(
        &runtime.automodpack.url,
        &destination,
        &ExpectedFile::sha256(&runtime.automodpack.sha256, None),
        |current, total| progress(current, total),
        cancel,
    )?;
    if !jar_is_valid(&destination) {
        let _ = fs::remove_file(&destination);
        return Err(AppError::new("automodpack_invalid"));
    }
    // Stale launcher-managed copies are removed only after the new JAR passed
    // verification. AutoModpack's configuration and synced packs are untouched.
    for path in candidates(paths) {
        if path != destination {
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                logger.info(
                    "automodpack",
                    format!("Removing stale AutoModpack JAR {name}"),
                );
            }
            let _ = fs::remove_file(&path);
        }
    }
    let installed = detect(paths, &runtime.automodpack.version);
    if !installed.valid {
        return Err(AppError::new("automodpack_invalid"));
    }
    logger.info(
        "automodpack",
        format!("AutoModpack {} installed and verified", runtime.automodpack.version),
    );
    Ok(installed)
}

pub fn automodpack_config_dir(paths: &GamePaths) -> PathBuf {
    paths.root.join("automodpack")
}

pub fn active_jar(paths: &GamePaths) -> Option<PathBuf> {
    candidates(paths).into_iter().find(|path| jar_is_valid(path))
}

#[allow(dead_code)]
pub fn download_timeout_marker() -> Duration {
    Duration::from_secs(30)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_jar(path: &Path, version: &str) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        use zip::write::SimpleFileOptions;
        zip.start_file(
            "META-INF/neoforge.mods.toml",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(format!("version=\"{version}\"").as_bytes())
            .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn detects_valid_corrupt_and_absent_automodpack() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        assert!(!detect(&paths, "4.0.7").valid);
        fs::create_dir_all(&paths.mods).unwrap();
        let file = paths
            .mods
            .join("automodpack-mc1.21.1-neoforge-4.0.7.jar");
        fs::write(&file, b"not a zip").unwrap();
        let status = detect(&paths, "4.0.7");
        assert!(status.installed);
        assert!(!status.valid);
        assert_eq!(status.message_code.as_deref(), Some("automodpack_corrupt"));
        write_jar(&file, "4.0.7");
        let status = detect(&paths, "4.0.7");
        assert!(status.valid);
        assert_eq!(status.version.as_deref(), Some("4.0.7"));
        assert!(status.message_code.is_none());
    }

    #[test]
    fn preserves_valid_different_automodpack_version() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        fs::create_dir_all(&paths.mods).unwrap();
        write_jar(
            &paths
                .mods
                .join("automodpack-mc1.21.1-neoforge-4.0.8.jar"),
            "4.0.8",
        );
        let status = detect(&paths, "4.0.7");
        assert!(status.valid);
        assert_eq!(status.message_code.as_deref(), Some("automodpack_version_differs"));
        assert_eq!(automodpack_config_dir(&paths), dir.path().join("automodpack"));
    }

    #[test]
    fn version_parser_reads_last_semver() {
        assert_eq!(
            version_from_name("automodpack-mc1.21.1-neoforge-4.0.7.jar").as_deref(),
            Some("4.0.7")
        );
        assert_eq!(version_from_name("not-a-jar.txt"), None);
    }
}
