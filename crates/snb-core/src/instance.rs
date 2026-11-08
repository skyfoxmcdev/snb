use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::logging::Logger;
use crate::util::{format_compact_utc, now_ms, read_json_with_backup, write_bytes_atomic, write_json_atomic};

pub const INSTANCE_SCHEMA_VERSION: u32 = 1;
pub const INSTANCE_FILE: &str = "instance.json";

#[derive(Debug, Clone)]
pub struct GamePaths {
    pub root: PathBuf,
    pub versions: PathBuf,
    pub libraries: PathBuf,
    pub assets: PathBuf,
    pub asset_indexes: PathBuf,
    pub asset_objects: PathBuf,
    pub natives: PathBuf,
    pub mods: PathBuf,
    pub mods_disabled: PathBuf,
    pub config: PathBuf,
    pub saves: PathBuf,
    pub resourcepacks: PathBuf,
    pub shaderpacks: PathBuf,
    pub backups: PathBuf,
    pub logs: PathBuf,
    pub managed: PathBuf,
}

impl GamePaths {
    pub fn new(root: &Path, runtime: &RuntimeConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            versions: root.join("versions"),
            libraries: root.join("libraries"),
            assets: root.join("assets"),
            asset_indexes: root.join("assets").join("indexes"),
            asset_objects: root.join("assets").join("objects"),
            natives: root.join("natives"),
            mods: root.join("mods"),
            mods_disabled: root.join("mods-disabled"),
            config: root.join("config"),
            saves: root.join("saves"),
            resourcepacks: root.join("resourcepacks"),
            shaderpacks: root.join("shaderpacks"),
            backups: root.join(&runtime.backup_folder_name),
            logs: root.join("logs"),
            managed: root.join(".snb"),
        }
    }

    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.versions.join(version)
    }

    pub fn version_json(&self, version: &str) -> PathBuf {
        self.version_dir(version).join(format!("{version}.json"))
    }

    pub fn client_jar(&self, version: &str) -> PathBuf {
        self.version_dir(version).join(format!("{version}.jar"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InstanceInfo {
    pub schema_version: u32,
    pub managed_by: String,
    pub minecraft_version: String,
    pub neoforge_version: String,
    pub automodpack_version: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl Default for InstanceInfo {
    fn default() -> Self {
        Self {
            schema_version: INSTANCE_SCHEMA_VERSION,
            managed_by: "Sky & Beyond Launcher".to_string(),
            minecraft_version: String::new(),
            neoforge_version: String::new(),
            automodpack_version: String::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }
}

pub fn read_instance(game_dir: &Path) -> Option<InstanceInfo> {
    read_json_with_backup::<InstanceInfo>(&game_dir.join(INSTANCE_FILE)).map(|(info, _)| info)
}

pub fn is_managed_instance(game_dir: &Path) -> bool {
    read_instance(game_dir)
        .map(|info| info.managed_by == "Sky & Beyond Launcher" && info.schema_version == INSTANCE_SCHEMA_VERSION)
        .unwrap_or(false)
}

pub fn ensure_launcher_profiles(game_dir: &Path, logger: &Arc<Logger>) -> AppResult<()> {
    let path = game_dir.join("launcher_profiles.json");
    if path.exists() {
        if let Ok(bytes) = fs::read(&path) {
            if serde_json::from_slice::<serde_json::Value>(&bytes)
                .map(|value| value.is_object())
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
        let backup = game_dir.join(format!(
            "launcher_profiles.json.invalid-{}",
            format_compact_utc(now_ms())
        ));
        let _ = fs::copy(&path, &backup);
        logger.warn(
            "instance",
            "launcher_profiles.json was invalid; preserved a backup and recreated the local installer profile",
        );
    }
    let profile = serde_json::json!({
        "profiles": {},
        "settings": {
            "enableSnapshots": false,
            "keepLauncherOpen": true,
            "showMenu": false
        },
        "version": 3
    });
    let mut bytes = serde_json::to_vec_pretty(&profile)?;
    bytes.push(b'\n');
    write_bytes_atomic(&path, &bytes)?;
    Ok(())
}

fn default_options(language: &str) -> String {
    let lang = if language == "fr" { "fr_fr" } else { "en_us" };
    format!(
        "version:3955\nfullscreen:false\nlang:{lang}\nmusic:0.65\nsound:0.8\npauseOnLostFocus:true\n"
    )
}

pub fn ensure_instance(
    paths: &GamePaths,
    runtime: &RuntimeConfig,
    language: &str,
    logger: &Arc<Logger>,
) -> AppResult<InstanceInfo> {
    for directory in [
        &paths.root,
        &paths.versions,
        &paths.libraries,
        &paths.assets,
        &paths.asset_indexes,
        &paths.asset_objects,
        &paths.natives,
        &paths.mods,
        &paths.mods_disabled,
        &paths.config,
        &paths.saves,
        &paths.resourcepacks,
        &paths.shaderpacks,
        &paths.backups,
        &paths.logs,
        &paths.managed,
    ] {
        fs::create_dir_all(directory)?;
    }
    ensure_launcher_profiles(&paths.root, logger)?;

    let options_path = paths.root.join("options.txt");
    if !options_path.exists() {
        write_bytes_atomic(&options_path, default_options(language).as_bytes())?;
    }

    let now = now_ms();
    let path = paths.root.join(INSTANCE_FILE);
    let existing = read_json_with_backup::<InstanceInfo>(&path);
    let created_at = existing
        .as_ref()
        .map(|(info, _)| if info.created_at_ms == 0 { now } else { info.created_at_ms })
        .unwrap_or(now);
    let info = InstanceInfo {
        schema_version: INSTANCE_SCHEMA_VERSION,
        managed_by: "Sky & Beyond Launcher".to_string(),
        minecraft_version: runtime.minecraft.version.clone(),
        neoforge_version: runtime.neoforge.version.clone(),
        automodpack_version: runtime.automodpack.version.clone(),
        created_at_ms: created_at,
        updated_at_ms: now,
    };
    write_json_atomic(&path, &info)?;
    logger.info(
        "instance",
        format!(
            "Dedicated instance ready (Minecraft {}, NeoForge {})",
            runtime.minecraft.version, runtime.neoforge.version
        ),
    );
    Ok(info)
}

/// Sets the fullscreen key in options.txt while preserving every other line.
pub fn apply_fullscreen(game_dir: &Path, fullscreen: bool) -> AppResult<()> {
    let path = game_dir.join("options.txt");
    let desired = format!("fullscreen:{}", if fullscreen { "true" } else { "false" });
    if !path.exists() {
        fs::create_dir_all(game_dir)?;
        write_bytes_atomic(&path, desired.as_bytes())?;
        return Ok(());
    }
    let content = fs::read_to_string(&path)?;
    let mut found = false;
    let mut lines: Vec<String> = content
        .lines()
        .map(|line| {
            if line.starts_with("fullscreen:") {
                found = true;
                desired.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(desired);
    }
    let mut output = lines.join("\n");
    output.push('\n');
    write_bytes_atomic(&path, output.as_bytes())?;
    Ok(())
}

/// Writes the vanilla/NeoForge compatibility profile file used to make the
/// official NeoForge installer accept the dedicated directory. It contains no
/// credentials and is never used by the launcher for authentication.
pub fn ensure_compatibility_profile(paths: &GamePaths, logger: &Arc<Logger>) -> AppResult<()> {
    ensure_launcher_profiles(&paths.root, logger)
}

pub fn instance_summary(paths: &GamePaths) -> String {
    read_instance(&paths.root)
        .map(|info| {
            format!(
                "Minecraft {} / NeoForge {} / AutoModpack {}",
                info.minecraft_version, info.neoforge_version, info.automodpack_version
            )
        })
        .unwrap_or_else(|| "not installed".to_string())
}

pub fn require_game_dir(game_dir: &Path) -> AppResult<()> {
    if !game_dir.is_dir() {
        return Err(AppError::new("game_dir_missing"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::username_is_valid;

    fn logger(dir: &Path) -> Arc<Logger> {
        Logger::new(dir.join("logs")).unwrap()
    }

    fn runtime() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    #[test]
    fn creates_dedicated_instance_and_never_touches_player_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".skyandbeyond");
        let runtime = runtime();
        let paths = GamePaths::new(&root, &runtime);
        let log = logger(dir.path());
        let info = ensure_instance(&paths, &runtime, "fr", &log).unwrap();
        assert_eq!(info.minecraft_version, "1.21.1");
        assert_eq!(info.neoforge_version, "21.1.248");
        assert!(paths.mods.is_dir());
        assert!(paths.saves.is_dir());
        assert!(paths.backups.ends_with("snb-backups"));
        assert!(root.join("launcher_profiles.json").is_file());
        let options = fs::read_to_string(root.join("options.txt")).unwrap();
        assert!(options.contains("lang:fr_fr"));

        fs::create_dir_all(paths.saves.join("world")).unwrap();
        fs::write(paths.saves.join("world/level.dat"), b"player data").unwrap();
        ensure_instance(&paths, &runtime, "en", &log).unwrap();
        assert_eq!(fs::read(paths.saves.join("world/level.dat")).unwrap(), b"player data");
        let options = fs::read_to_string(root.join("options.txt")).unwrap();
        assert!(options.contains("lang:fr_fr"), "existing options must be preserved");
    }

    #[test]
    fn fullscreen_update_preserves_other_options() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("options.txt"),
            "fov:0.5\nfullscreen:false\nlang:en_us\n",
        )
        .unwrap();
        apply_fullscreen(dir.path(), true).unwrap();
        let content = fs::read_to_string(dir.path().join("options.txt")).unwrap();
        assert!(content.contains("fov:0.5"));
        assert!(content.contains("fullscreen:true"));
        assert!(!content.contains("fullscreen:false"));
        apply_fullscreen(dir.path(), false).unwrap();
        let content = fs::read_to_string(dir.path().join("options.txt")).unwrap();
        assert!(content.contains("fullscreen:false"));
    }

    #[test]
    fn invalid_launcher_profiles_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("launcher_profiles.json"), b"{broken").unwrap();
        let log = logger(dir.path());
        ensure_launcher_profiles(dir.path(), &log).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("launcher_profiles.json")).unwrap())
                .unwrap();
        assert!(value.get("profiles").is_some());
        assert!(fs::read_dir(dir.path()).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("launcher_profiles.json.invalid")
        }));
    }

    #[test]
    fn validates_usernames_for_fixture_sanity() {
        assert!(username_is_valid("SkyPlayer"));
    }
}
