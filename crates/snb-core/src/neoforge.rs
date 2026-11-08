use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::http::{Downloader, ExpectedFile};
use crate::instance::{ensure_compatibility_profile, GamePaths};
use crate::java::run_command_capture;
use crate::logging::Logger;
use crate::minecraft::{load_version_json, VersionJson};
use crate::util::{human_file_name_from_url, sanitize_file_name, validate_url};

#[derive(Debug, Clone)]
pub struct NeoForgeOutcome {
    pub version_id: String,
    pub profile_path: PathBuf,
}

pub fn profile_id(runtime: &RuntimeConfig) -> String {
    format!("neoforge-{}", runtime.neoforge.version)
}

pub fn profile_path(paths: &GamePaths, version_id: &str) -> PathBuf {
    paths.version_dir(version_id).join(format!("{version_id}.json"))
}

pub fn validate_profile(path: &Path, expected_id: &str) -> AppResult<VersionJson> {
    let version = load_version_json(path)?;
    if version.id != expected_id {
        return Err(AppError::new("neoforge_profile_invalid"));
    }
    if version.main_class.trim().is_empty() {
        return Err(AppError::new("neoforge_profile_invalid"));
    }
    Ok(version)
}

async fn noop() {}

fn installer_sha1(downloader: &Downloader, url: &str) -> AppResult<String> {
    let text = downloader.fetch_text(
        &format!("{url}.sha1"),
        4096,
        Duration::from_secs(20),
    )?;
    let digest = text.trim().to_ascii_lowercase();
    if digest.len() != 40 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::new("neoforge_checksum_unavailable"));
    }
    Ok(digest)
}

/// Installs NeoForge through its official installer after creating the minimal
/// launcher profile it expects. The installer is checksum-verified and its
/// generated profile is parsed before success is reported.
pub fn install_neoforge(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    java_path: &Path,
    force: bool,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<NeoForgeOutcome> {
    let version_id = profile_id(runtime);
    let profile = profile_path(paths, &version_id);
    if !force && validate_profile(&profile, &version_id).is_ok() {
        progress(1, 1);
        return Ok(NeoForgeOutcome {
            version_id,
            profile_path: profile,
        });
    }
    if !java_path.is_file() {
        return Err(AppError::new("java_missing"));
    }
    ensure_compatibility_profile(paths, logger)?;
    let url = runtime.neoforge_installer_url();
    validate_url(&url, false)?;
    let downloader = Downloader::new()?;
    let sha1 = installer_sha1(&downloader, &url)?;
    let file_name = human_file_name_from_url(&url)
        .and_then(|name| sanitize_file_name(&name).ok())
        .unwrap_or_else(|| format!("neoforge-{}-installer.jar", runtime.neoforge.version));
    let cache = paths.managed.join("cache").join("neoforge");
    let installer = cache.join(file_name);
    downloader.download(
        &url,
        &installer,
        &ExpectedFile::sha1(sha1, None),
        |current, total| {
            if total > 0 {
                progress(current.min(total), total);
            }
        },
        cancel,
    )?;
    logger.info(
        "neoforge",
        format!("Running the official NeoForge {} installer", runtime.neoforge.version),
    );
    let mut command = Command::new(java_path);
    command
        .arg("-jar")
        .arg(&installer)
        .arg("--installClient")
        .arg(&paths.root)
        .current_dir(&paths.root);
    let output = run_command_capture(command, Duration::from_secs(15 * 60), Some(cancel))?;
    if output.status_code != Some(0) {
        let tail = output.tail(3000);
        logger.error(
            "neoforge",
            format!("NeoForge installer failed: {}", output.tail(1200)),
        );
        return Err(AppError::with_detail("neoforge_install_failed", tail));
    }
    validate_profile(&profile, &version_id).map_err(|error| {
        AppError::with_detail(
            "neoforge_install_failed",
            format!(
                "the installer exited but its profile is unusable: {}",
                error.detail_text()
            ),
        )
    })?;
    progress(1, 1);
    logger.info("neoforge", "NeoForge profile verified");
    Ok(NeoForgeOutcome {
        version_id,
        profile_path: profile,
    })
}

#[allow(dead_code)]
async fn unused_async_marker() {
    noop().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::GamePaths;

    #[test]
    fn builds_expected_profile_path_and_installer_url() {
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(Path::new("/tmp/snb-test"), &runtime);
        assert_eq!(profile_id(&runtime), "neoforge-21.1.248");
        assert!(profile_path(&paths, "neoforge-21.1.248")
            .ends_with("versions/neoforge-21.1.248/neoforge-21.1.248.json"));
        assert!(runtime
            .neoforge_installer_url()
            .ends_with("neoforge-21.1.248-installer.jar"));
    }

    #[test]
    fn validates_generated_profile_shape() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("neoforge-21.1.248.json");
        std::fs::write(
            &profile,
            r#"{"id":"neoforge-21.1.248","inheritsFrom":"1.21.1","mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher"}"#,
        )
        .unwrap();
        assert!(validate_profile(&profile, "neoforge-21.1.248").is_ok());
        assert!(validate_profile(&profile, "different").is_err());
        std::fs::write(&profile, b"{bad").unwrap();
        assert!(validate_profile(&profile, "neoforge-21.1.248").is_err());
    }
}
