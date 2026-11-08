use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::instance::GamePaths;
use crate::logging::Logger;
use crate::util::{format_compact_utc, now_ms, sanitize_file_name, unique_suffix};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub file_name: String,
    pub created_at_ms: u64,
    pub size: u64,
    pub reason: String,
}

fn backup_name(reason: &str) -> String {
    let reason = sanitize_file_name(reason).unwrap_or_else(|_| "manual".to_string());
    format!(
        "backup-{}-{}.zip",
        format_compact_utc(now_ms()),
        reason.trim_end_matches(".zip")
    )
}

fn validate_zip(path: &Path) -> AppResult<u64> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() == 0 {
        return Err(AppError::new("backup_invalid"));
    }
    let mut size = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        size = size.saturating_add(entry.size());
        std::io::copy(&mut entry, &mut std::io::sink())?;
    }
    Ok(size)
}

fn write_snapshot(saves: &Path, destination: &Path, reason: &str) -> AppResult<u64> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = destination.with_extension("zip.part");
    let _ = fs::remove_file(&part);
    let file = File::create(&part)?;
    let mut zip = zip::ZipWriter::new(file);
    use zip::write::SimpleFileOptions;
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "createdAtMs": now_ms(),
        "reason": reason,
        "launcher": "Sky & Beyond Launcher",
    });
    zip.start_file("snb-backup.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    let mut count = 0u64;
    if saves.is_dir() {
        for entry in walkdir::WalkDir::new(saves)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = path.strip_prefix(saves).map_err(|_| AppError::new("backup_failed"))?;
            let name = format!("saves/{}", relative.to_string_lossy().replace('\\', "/"));
            if name.contains("..") {
                continue;
            }
            zip.start_file(name, options)?;
            let mut source = File::open(path)?;
            std::io::copy(&mut source, &mut zip)?;
            count += 1;
        }
    }
    let file = zip.finish()?;
    file.sync_all()?;
    if count == 0 {
        // An empty saves folder is still a valid snapshot and can restore an
        // intentionally empty state, but never delete older backups for it.
    }
    validate_zip(&part).map_err(|error| {
        let _ = fs::remove_file(&part);
        error
    })?;
    fs::rename(&part, destination)?;
    Ok(fs::metadata(destination)?.len())
}

pub fn list_backups(paths: &GamePaths) -> Vec<BackupEntry> {
    let Ok(entries) = fs::read_dir(&paths.backups) else {
        return Vec::new();
    };
    let mut backups: Vec<BackupEntry> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.eq_ignore_ascii_case("zip"))
                .unwrap_or(false)
        })
        .filter_map(|path| {
            let metadata = fs::metadata(&path).ok()?;
            Some(BackupEntry {
                file_name: path.file_name()?.to_string_lossy().to_string(),
                created_at_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
                size: metadata.len(),
                reason: path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.rsplit('-').next())
                    .unwrap_or("manual")
                    .to_string(),
            })
        })
        .collect();
    backups.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    backups
}

fn apply_retention(paths: &GamePaths, retention: u32, logger: &Arc<Logger>) {
    let keep = retention.max(1) as usize;
    let backups = list_backups(paths);
    for entry in backups.into_iter().skip(keep) {
        let path = paths.backups.join(&entry.file_name);
        if fs::remove_file(&path).is_ok() {
            logger.info("backup", format!("Retention removed {}", entry.file_name));
        }
    }
}

/// Creates a complete, validated snapshot. A failed or interrupted snapshot
/// never replaces an existing backup and is never counted for retention.
pub fn create_backup(
    paths: &GamePaths,
    reason: &str,
    game_running: bool,
    retention: u32,
    logger: &Arc<Logger>,
) -> AppResult<BackupEntry> {
    if game_running {
        return Err(AppError::new("game_running"));
    }
    fs::create_dir_all(&paths.backups)?;
    for entry in fs::read_dir(&paths.backups).into_iter().flatten().flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .ends_with(".zip.part")
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    let file_name = backup_name(reason);
    let destination = paths.backups.join(&file_name);
    write_snapshot(&paths.saves, &destination, reason)?;
    let metadata = fs::metadata(&destination)?;
    let entry = BackupEntry {
        file_name,
        created_at_ms: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(now_ms()),
        size: metadata.len(),
        reason: reason.to_string(),
    };
    apply_retention(paths, retention, logger);
    logger.info(
        "backup",
        format!("Backup verified ({} bytes); retention applied", entry.size),
    );
    Ok(entry)
}

fn safe_extract(archive_path: &Path, staging: &Path) -> AppResult<()> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(AppError::new("backup_invalid"));
        };
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(AppError::new("backup_invalid"));
        }
        let destination = staging.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&destination)?;
            std::io::copy(&mut entry, &mut output)?;
        }
    }
    Ok(())
}

/// Restores a snapshot only after explicit confirmation. The current saves
/// folder is snapshotted first; extraction happens in staging and replacement
/// is rolled back if the final rename fails.
pub fn restore_backup(
    paths: &GamePaths,
    file_name: &str,
    confirmed: bool,
    game_running: bool,
    logger: &Arc<Logger>,
) -> AppResult<BackupEntry> {
    if !confirmed {
        return Err(AppError::new("confirmation_required"));
    }
    if game_running {
        return Err(AppError::new("game_running"));
    }
    let clean = sanitize_file_name(file_name)?;
    if !clean.to_ascii_lowercase().ends_with(".zip") {
        return Err(AppError::new("backup_invalid"));
    }
    let archive = paths.backups.join(&clean);
    if !archive.is_file() {
        return Err(AppError::new("backup_not_found"));
    }
    validate_zip(&archive)?;
    let safety = if paths.saves.exists() {
        Some(write_snapshot(
            &paths.saves,
            &paths
                .backups
                .join(format!("pre-restore-{}.zip", unique_suffix())),
            "pre-restore",
        )?)
    } else {
        None
    };
    let staging = paths
        .managed
        .join(format!(".restore-{}", unique_suffix()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    if let Err(error) = safe_extract(&archive, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged_saves = staging.join("saves");
    if !staged_saves.is_dir() {
        let _ = fs::remove_dir_all(&staging);
        return Err(AppError::new("backup_invalid"));
    }
    let old = paths
        .root
        .join(format!(".saves-old-{}", unique_suffix()));
    let had_saves = paths.saves.exists();
    if had_saves {
        fs::rename(&paths.saves, &old)?;
    }
    if let Err(error) = fs::rename(&staged_saves, &paths.saves) {
        if old.exists() {
            let _ = fs::rename(&old, &paths.saves);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    let _ = fs::remove_dir_all(&old);
    let _ = fs::remove_dir_all(&staging);
    let _ = safety;
    logger.info("backup", "Worlds restored from a verified backup");
    Ok(BackupEntry {
        file_name: clean,
        created_at_ms: now_ms(),
        size: fs::metadata(&archive).map(|m| m.len()).unwrap_or(0),
        reason: "restored".to_string(),
    })
}

pub fn latest_backup_age_ms(paths: &GamePaths) -> Option<u64> {
    list_backups(paths)
        .first()
        .map(|backup| now_ms().saturating_sub(backup.created_at_ms))
}

pub fn saves_zip_path(paths: &GamePaths) -> PathBuf {
    paths.saves.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;

    fn logger() -> Arc<Logger> {
        Logger::new(std::env::temp_dir().join(format!("snb-backup-logs-{}", unique_suffix()))).unwrap()
    }

    #[test]
    fn backup_and_restore_round_trip_requires_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        fs::create_dir_all(paths.saves.join("world")).unwrap();
        fs::write(paths.saves.join("world/level.dat"), b"original").unwrap();
        let entry = create_backup(&paths, "manual", false, 2, &logger()).unwrap();
        fs::write(paths.saves.join("world/level.dat"), b"changed").unwrap();
        assert!(restore_backup(&paths, &entry.file_name, false, false, &logger()).is_err());
        restore_backup(&paths, &entry.file_name, true, false, &logger()).unwrap();
        assert_eq!(fs::read(paths.saves.join("world/level.dat")).unwrap(), b"original");
    }

    #[test]
    fn running_game_blocks_backup() {
        let dir = tempfile::tempdir().unwrap();
        let paths = GamePaths::new(dir.path(), &RuntimeConfig::default());
        assert!(create_backup(&paths, "auto", true, 2, &logger()).is_err());
    }
}
