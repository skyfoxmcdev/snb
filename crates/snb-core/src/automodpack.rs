use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use regex::Regex;
use serde::Serialize;

use crate::config::{AutomodpackOverride, RuntimeConfig};
use crate::error::{AppError, AppResult};
use crate::http::{Downloader, ExpectedFile};
use crate::instance::GamePaths;
use crate::logging::Logger;
use crate::util::{now_ms, sanitize_file_name, sha256_bytes, sha256_file, validate_url};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomodpackStatus {
    pub installed: bool,
    pub valid: bool,
    /// True only when the installed JAR is byte-identical to the expected
    /// build (bundled or published).
    pub up_to_date: bool,
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

fn jar_candidates(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
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

fn candidates(paths: &GamePaths) -> Vec<PathBuf> {
    jar_candidates(&paths.mods)
}

/// Every place a launcher-managed AutoModpack JAR can linger, enabled or not.
fn all_candidates(paths: &GamePaths) -> Vec<PathBuf> {
    let mut files = jar_candidates(&paths.mods);
    files.extend(jar_candidates(&paths.mods_disabled));
    files
}

/// Reports the AutoModpack JAR the game would load and whether it is exactly
/// the expected build. `expected_sha256` identifies the build, so a renamed
/// JAR with the same bytes is still current.
pub fn detect(paths: &GamePaths, expected_sha256: &str, pinned_version: &str) -> AutomodpackStatus {
    let files = candidates(paths);
    let installed = !files.is_empty();
    let mut first_valid: Option<(PathBuf, String)> = None;
    for path in &files {
        if !jar_is_valid(path) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let up_to_date = sha256_file(path)
            .map(|digest| digest.eq_ignore_ascii_case(expected_sha256.trim()))
            .unwrap_or(false);
        // An up-to-date JAR wins even when an older AutoModpack copy sorts
        // before it, otherwise the expected build would look outdated.
        if up_to_date {
            return AutomodpackStatus {
                installed,
                valid: true,
                up_to_date: true,
                path: Some(path.to_string_lossy().to_string()),
                version: version_from_name(&name),
                pinned_version: pinned_version.to_string(),
                message_code: None,
            };
        }
        if first_valid.is_none() {
            first_valid = Some((path.clone(), name));
        }
    }
    if let Some((path, name)) = first_valid {
        return AutomodpackStatus {
            installed,
            valid: true,
            up_to_date: false,
            path: Some(path.to_string_lossy().to_string()),
            version: version_from_name(&name),
            pinned_version: pinned_version.to_string(),
            message_code: Some("automodpack_outdated".to_string()),
        };
    }
    AutomodpackStatus {
        installed,
        valid: false,
        up_to_date: false,
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

fn temp_path(directory: &Path, file_name: &str) -> PathBuf {
    directory.join(format!(".snb-automodpack-{}.{}.tmp", now_ms(), file_name))
}

/// Installs a verified temporary JAR, then removes every other AutoModpack JAR
/// (including disabled copies) so exactly one build can load.
fn commit_jar(
    temp: &Path,
    destination: &Path,
    expected_sha256: &str,
    paths: &GamePaths,
    logger: &Arc<Logger>,
) -> AppResult<()> {
    let digest = sha256_file(temp)?;
    if !digest.eq_ignore_ascii_case(expected_sha256.trim()) || !jar_is_valid(temp) {
        let _ = fs::remove_file(temp);
        return Err(AppError::new("automodpack_invalid"));
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temp, destination)?;
    // Stale launcher-managed copies are removed only after the new JAR is in
    // place. AutoModpack's configuration and synced packs are untouched.
    remove_other_jars(paths, destination, logger);
    Ok(())
}

/// Deletes every AutoModpack JAR except the one that must load. AutoModpack's
/// own updater can leave another JAR next to the pinned build, and two
/// AutoModpack mods in `mods/` cannot load together.
fn remove_other_jars(paths: &GamePaths, keep: &Path, logger: &Arc<Logger>) {
    for path in all_candidates(paths) {
        if path == keep {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            logger.info(
                "automodpack",
                format!("Removing stale AutoModpack JAR {name}"),
            );
        }
        let _ = fs::remove_file(&path);
    }
}

fn install_bundled(
    runtime: &RuntimeConfig,
    bundled: &[u8],
    paths: &GamePaths,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<AutomodpackStatus> {
    let file_name = sanitize_file_name(&runtime.automodpack.file_name)?;
    if !file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(AppError::new("automodpack_invalid"));
    }
    let expected = runtime.automodpack.sha256.trim().to_ascii_lowercase();
    let status = detect(paths, &expected, &runtime.automodpack.version);
    if status.up_to_date {
        if let Some(current) = status.path.as_deref() {
            remove_other_jars(paths, Path::new(current), logger);
        }
        progress(1, 1);
        return Ok(status);
    }
    if sha256_bytes(bundled) != expected {
        return Err(AppError::new("automodpack_invalid"));
    }
    fs::create_dir_all(&paths.mods)?;
    let destination = paths.mods.join(&file_name);
    let temp = temp_path(&paths.mods, &file_name);
    let result = fs::write(&temp, bundled)
        .map_err(AppError::from)
        .and_then(|_| commit_jar(&temp, &destination, &expected, paths, logger));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    let installed = detect(paths, &expected, &runtime.automodpack.version);
    if !installed.up_to_date {
        return Err(AppError::new("automodpack_invalid"));
    }
    logger.info(
        "automodpack",
        format!(
            "Bundled AutoModpack {} installed and verified",
            runtime.automodpack.version
        ),
    );
    progress(1, 1);
    Ok(installed)
}

/// Downloads a JAR published in `launcher.json`, so the owner can ship a new
/// AutoModpack build without a launcher release.
fn install_published(
    published: &AutomodpackOverride,
    paths: &GamePaths,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<AutomodpackStatus> {
    let file_name = sanitize_file_name(&published.file_name)?;
    if !file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(AppError::new("automodpack_invalid"));
    }
    let expected = published.sha256.trim().to_ascii_lowercase();
    let status = detect(paths, &expected, &published.version);
    if status.up_to_date {
        if let Some(current) = status.path.as_deref() {
            remove_other_jars(paths, Path::new(current), logger);
        }
        progress(1, 1);
        return Ok(status);
    }
    validate_url(&published.url, false)?;
    fs::create_dir_all(&paths.mods)?;
    let destination = paths.mods.join(&file_name);
    let temp = temp_path(&paths.mods, &file_name);
    let downloader = Downloader::new()?;
    let downloaded = downloader.download(
        &published.url,
        &temp,
        &ExpectedFile::sha256(expected.clone(), None),
        |current, total| progress(current, total),
        cancel,
    );
    let result = downloaded
        .map(|_| ())
        .and_then(|_| commit_jar(&temp, &destination, &expected, paths, logger));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    let installed = detect(paths, &expected, &published.version);
    if !installed.up_to_date {
        return Err(AppError::new("automodpack_invalid"));
    }
    logger.info(
        "automodpack",
        format!(
            "Published AutoModpack {} downloaded and verified",
            published.version
        ),
    );
    Ok(installed)
}

/// Ensures the expected AutoModpack build is installed and is the only
/// AutoModpack JAR present. A published build wins over the bundled one; if it
/// cannot be fetched or verified the bundled build is used instead, so an
/// installation never fails for lack of network access.
pub fn ensure_automodpack(
    runtime: &RuntimeConfig,
    published: Option<&AutomodpackOverride>,
    bundled: &[u8],
    paths: &GamePaths,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<AutomodpackStatus> {
    if let Some(published) = published {
        match install_published(published, paths, cancel, logger, progress) {
            Ok(status) => return Ok(status),
            Err(error) => logger.warn(
                "automodpack",
                format!(
                    "Published AutoModpack {} could not be installed ({error}); using the bundled build",
                    published.version
                ),
            ),
        }
    }
    install_bundled(runtime, bundled, paths, logger, progress)
}

pub fn automodpack_config_dir(paths: &GamePaths) -> PathBuf {
    paths.root.join("automodpack")
}

pub fn active_jar(paths: &GamePaths) -> Option<PathBuf> {
    candidates(paths).into_iter().find(|path| jar_is_valid(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AutoModpackRuntime;
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
        zip.write_all(format!("version=\"{version}\"\nmodId=\"automodpack\"\n").as_bytes())
            .unwrap();
        zip.finish().unwrap();
    }

    fn jar_bytes(version: &str) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            use zip::write::SimpleFileOptions;
            zip.start_file(
                "META-INF/neoforge.mods.toml",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(format!("version=\"{version}\"\nmodId=\"automodpack\"\n").as_bytes())
                .unwrap();
            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    fn runtime_for(bundled: &[u8]) -> RuntimeConfig {
        RuntimeConfig {
            automodpack: AutoModpackRuntime {
                version: "custom-1.21.1".to_string(),
                sha256: sha256_bytes(bundled),
                file_name: "AutoModpack-Custom-1.21.1-neoforge.jar".to_string(),
            },
            ..RuntimeConfig::default()
        }
    }

    #[test]
    fn detects_valid_corrupt_and_absent_automodpack() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        let expected = runtime.automodpack.sha256.clone();
        assert!(!detect(&paths, &expected, &runtime.automodpack.version).valid);
        fs::create_dir_all(&paths.mods).unwrap();
        let file = paths.mods.join("automodpack-mc1.21.1-neoforge-4.0.7.jar");
        fs::write(&file, b"not a zip").unwrap();
        let status = detect(&paths, &expected, &runtime.automodpack.version);
        assert!(status.installed);
        assert!(!status.valid);
        assert_eq!(status.message_code.as_deref(), Some("automodpack_corrupt"));
        write_jar(&file, "4.0.7");
        let status = detect(&paths, &expected, &runtime.automodpack.version);
        assert!(status.valid);
        assert!(!status.up_to_date);
        assert_eq!(
            status.message_code.as_deref(),
            Some("automodpack_outdated")
        );
        let digest = sha256_file(&file).unwrap();
        let status = detect(&paths, &digest, &runtime.automodpack.version);
        assert!(status.valid);
        assert!(status.up_to_date);
        assert!(status.message_code.is_none());
    }

    #[test]
    fn detects_a_different_automodpack_build() {
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
        let status = detect(&paths, "00", "custom-1.21.1");
        assert!(status.valid);
        assert!(!status.up_to_date);
        assert_eq!(automodpack_config_dir(&paths), dir.path().join("automodpack"));
    }

    #[test]
    fn bundled_jar_replaces_older_automodpack_jars() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = jar_bytes("custom-1.21.1");
        let runtime = runtime_for(&bundled);
        let paths = GamePaths::new(dir.path(), &runtime);
        fs::create_dir_all(&paths.mods).unwrap();
        fs::create_dir_all(&paths.mods_disabled).unwrap();
        write_jar(
            &paths
                .mods
                .join("automodpack-mc1.21.1-neoforge-4.0.7.jar"),
            "4.0.7",
        );
        write_jar(
            &paths
                .mods_disabled
                .join("automodpack-mc1.21.1-neoforge-4.0.6.jar"),
            "4.0.6",
        );
        let logger = Logger::new(dir.path().join("logs")).unwrap();
        let cancel = AtomicBool::new(false);
        let status = ensure_automodpack(
            &runtime,
            None,
            &bundled,
            &paths,
            &cancel,
            &logger,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(status.valid);
        assert!(status.up_to_date);
        assert!(paths
            .mods
            .join("AutoModpack-Custom-1.21.1-neoforge.jar")
            .is_file());
        assert!(!paths
            .mods
            .join("automodpack-mc1.21.1-neoforge-4.0.7.jar")
            .exists());
        assert!(!paths
            .mods_disabled
            .join("automodpack-mc1.21.1-neoforge-4.0.6.jar")
            .exists());
        // A second run is a no-op.
        let again = ensure_automodpack(
            &runtime,
            None,
            &bundled,
            &paths,
            &cancel,
            &logger,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(again.up_to_date);
    }

    #[test]
    fn removes_an_extra_automodpack_jar_next_to_the_expected_build() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = jar_bytes("custom-1.21.1");
        let runtime = runtime_for(&bundled);
        let paths = GamePaths::new(dir.path(), &runtime);
        fs::create_dir_all(&paths.mods).unwrap();
        fs::write(
            paths
                .mods
                .join("AutoModpack-Custom-1.21.1-neoforge.jar"),
            &bundled,
        )
        .unwrap();
        write_jar(
            &paths
                .mods
                .join("automodpack-mc1.21.1-neoforge-4.0.7.jar"),
            "4.0.7",
        );
        let logger = Logger::new(dir.path().join("logs")).unwrap();
        let cancel = AtomicBool::new(false);
        let status = ensure_automodpack(
            &runtime,
            None,
            &bundled,
            &paths,
            &cancel,
            &logger,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(status.up_to_date);
        assert!(!paths
            .mods
            .join("automodpack-mc1.21.1-neoforge-4.0.7.jar")
            .exists());
        assert!(paths
            .mods
            .join("AutoModpack-Custom-1.21.1-neoforge.jar")
            .is_file());
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
