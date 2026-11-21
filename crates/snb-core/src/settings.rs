use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::util::{default_game_dir, is_vanilla_minecraft_dir, username_is_valid};

pub const SETTINGS_SCHEMA_VERSION: u32 = 3;

/// The launcher ships a 4 GiB minimum / 8 GiB maximum allocation. The same
/// range is enforced on every load and save, so a value stored by an older
/// build can never pull the allocation back down after the player saves.
pub const MIN_MEMORY_MB: u32 = 4096;
pub const MAX_MEMORY_MB: u32 = 8192;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub schema_version: u32,
    pub username: String,
    /// Empty means the recommended default directory.
    pub game_dir: String,
    /// Empty means automatic Java 21 detection.
    pub java_path: String,
    /// "en", "fr" or empty for the system language.
    pub language: String,
    /// "dark", "light" or "system".
    pub theme: String,
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub jvm_args: String,
    pub game_args: String,
    pub auto_install_java: bool,
    pub auto_backup: bool,
    pub backup_interval_hours: u32,
    pub backup_retention: u32,
    pub last_backup_ms: u64,
    pub onboarding_complete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            username: String::new(),
            game_dir: String::new(),
            java_path: String::new(),
            language: String::new(),
            theme: "system".to_string(),
            min_memory_mb: 4096,
            max_memory_mb: 8192,
            width: 1280,
            height: 720,
            fullscreen: false,
            jvm_args: String::new(),
            game_args: String::new(),
            auto_install_java: true,
            auto_backup: true,
            backup_interval_hours: 24,
            backup_retention: 5,
            last_backup_ms: 0,
            onboarding_complete: false,
        }
    }
}

impl Settings {
    /// Normalises values loaded from disk without discarding a player's explicit
    /// choice. Used before the settings are handed to the UI.
    pub fn normalize_loaded(&mut self, total_memory_mb: u64) {
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        self.username = self.username.trim().to_string();
        self.game_dir = self.game_dir.trim().to_string();
        self.java_path = self.java_path.trim().to_string();
        self.language = self.language.trim().to_ascii_lowercase();
        self.theme = self.theme.trim().to_ascii_lowercase();
        if self.theme.is_empty() {
            self.theme = "system".to_string();
        }
        if self.game_dir.is_empty() {
            self.game_dir = default_game_dir().to_string_lossy().to_string();
        }
        self.clamp_memory(total_memory_mb);
        if self.width < 640 {
            self.width = 1280;
        }
        if self.height < 360 {
            self.height = 720;
        }
        if self.backup_interval_hours == 0 {
            self.backup_interval_hours = 24;
        }
        if self.backup_retention == 0 {
            self.backup_retention = 5;
        }
        if !matches!(self.language.as_str(), "" | "en" | "fr") {
            self.language = String::new();
        }
        if !matches!(self.theme.as_str(), "dark" | "light" | "system") {
            self.theme = "system".to_string();
        }
    }

    /// Keeps the allocation inside the shipped 4 GiB / 8 GiB range and inside
    /// the machine's memory. A pair stored below the floor is replaced by the
    /// shipped baseline instead of being kept, so an old 1 GiB default cannot
    /// come back and swallow the next save.
    fn clamp_memory(&mut self, total_memory_mb: u64) {
        let ceiling = if total_memory_mb > 0 {
            total_memory_mb.min(MAX_MEMORY_MB as u64).max(1024) as u32
        } else {
            MAX_MEMORY_MB
        };
        let floor = MIN_MEMORY_MB.min(ceiling);

        if self.min_memory_mb < floor || self.max_memory_mb < floor {
            self.min_memory_mb = floor;
            self.max_memory_mb = ceiling;
            return;
        }
        if self.max_memory_mb > ceiling {
            self.max_memory_mb = ceiling;
        }
        if self.min_memory_mb > self.max_memory_mb {
            self.min_memory_mb = self.max_memory_mb;
        }
    }

    pub fn effective_game_dir(&self) -> PathBuf {
        if self.game_dir.trim().is_empty() {
            default_game_dir()
        } else {
            PathBuf::from(self.game_dir.trim())
        }
    }

    pub fn is_first_run(&self) -> bool {
        !self.onboarding_complete || !username_is_valid(&self.username)
    }

    pub fn validate(&self, total_memory_mb: u64) -> AppResult<()> {
        if !self.username.is_empty() && !username_is_valid(&self.username) {
            return Err(AppError::new("username_invalid"));
        }
        if self.username.len() > 64 {
            return Err(AppError::new("username_invalid"));
        }
        if !self.game_dir.trim().is_empty() {
            let path = Path::new(self.game_dir.trim());
            if is_vanilla_minecraft_dir(path) {
                return Err(AppError::with_detail(
                    "game_dir_is_vanilla",
                    "The dedicated instance must not use the vanilla .minecraft directory",
                ));
            }
            if !path.is_absolute() {
                return Err(AppError::new("game_dir_not_absolute"));
            }
        }
        if !self.java_path.trim().is_empty() {
            let path = Path::new(self.java_path.trim());
            if !path.is_absolute() {
                return Err(AppError::new("java_path_not_absolute"));
            }
        }
        if !matches!(self.language.as_str(), "" | "en" | "fr") {
            return Err(AppError::new("language_unsupported"));
        }
        if !matches!(self.theme.as_str(), "dark" | "light" | "system") {
            return Err(AppError::new("theme_unsupported"));
        }
        // A machine smaller than the shipped range cannot provide the floor, so
        // the floor is the smaller of the shipped minimum and the machine's RAM.
        let system_cap = if total_memory_mb > 0 {
            total_memory_mb.min(u32::MAX as u64) as u32
        } else {
            u32::MAX
        };
        let min_allowed = MIN_MEMORY_MB.min(system_cap);
        if self.min_memory_mb < min_allowed {
            return Err(AppError::new("memory_min_too_low").param("minMb", min_allowed.to_string()));
        }
        if self.max_memory_mb < self.min_memory_mb {
            return Err(AppError::new("memory_max_below_min"));
        }
        if self.max_memory_mb > MAX_MEMORY_MB {
            return Err(AppError::new("memory_max_too_high").param("maxMb", MAX_MEMORY_MB.to_string()));
        }
        if total_memory_mb > 0 && self.max_memory_mb as u64 > total_memory_mb.max(1024) {
            return Err(AppError::new("memory_exceeds_system").param(
                "availableMb",
                total_memory_mb.to_string(),
            ));
        }
        if !(640..=7680).contains(&self.width) {
            return Err(AppError::new("resolution_width_invalid"));
        }
        if !(360..=4320).contains(&self.height) {
            return Err(AppError::new("resolution_height_invalid"));
        }
        if self.jvm_args.len() > 4000 || self.game_args.len() > 4000 {
            return Err(AppError::new("arguments_too_long"));
        }
        if self.jvm_args.contains('\0') || self.game_args.contains('\0') {
            return Err(AppError::new("arguments_invalid"));
        }
        if !(1..=720).contains(&self.backup_interval_hours) {
            return Err(AppError::new("backup_interval_invalid"));
        }
        if !(1..=50).contains(&self.backup_retention) {
            return Err(AppError::new("backup_retention_invalid"));
        }
        Ok(())
    }

    pub fn mark_backup_done(&mut self, timestamp_ms: u64) {
        self.last_backup_ms = timestamp_ms;
    }

    pub fn backup_is_due(&self, timestamp_ms: u64) -> bool {
        if !self.auto_backup {
            return false;
        }
        if self.last_backup_ms == 0 {
            return true;
        }
        let interval_ms = self.backup_interval_hours as u64 * 3_600_000;
        timestamp_ms.saturating_sub(self.last_backup_ms) >= interval_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_game_dir() -> String {
        if cfg!(windows) {
            r"C:\Users\player\.skyandbeyond".to_string()
        } else {
            "/home/player/.skyandbeyond".to_string()
        }
    }

    fn valid() -> Settings {
        Settings {
            username: "Steve".to_string(),
            game_dir: test_game_dir(),
            ..Settings::default()
        }
    }

    #[test]
    fn defaults_are_sane_and_not_first_run_when_configured() {
        let mut settings = Settings::default();
        assert!(settings.is_first_run());
        settings.username = "Alex".to_string();
        settings.onboarding_complete = true;
        assert!(!settings.is_first_run());
        assert_eq!(settings.min_memory_mb, 4096);
        assert_eq!(settings.max_memory_mb, 8192);
        assert_eq!(settings.backup_retention, 5);
    }

    #[test]
    fn normalisation_clamps_memory_to_system() {
        let mut settings = Settings::default();
        settings.max_memory_mb = 16_384;
        settings.normalize_loaded(8_192);
        assert_eq!(settings.max_memory_mb, 8_192);
        assert!(settings.game_dir.ends_with(".skyandbeyond"));
    }

    #[test]
    fn stored_memory_outside_the_shipped_range_is_lifted() {
        let mut legacy = Settings {
            schema_version: 1,
            min_memory_mb: 1024,
            max_memory_mb: 4096,
            ..Settings::default()
        };
        legacy.normalize_loaded(16_384);
        assert_eq!(legacy.min_memory_mb, MIN_MEMORY_MB);
        assert_eq!(legacy.max_memory_mb, MAX_MEMORY_MB);
        assert_eq!(legacy.schema_version, SETTINGS_SCHEMA_VERSION);

        // A 1 GiB pair written by an earlier build cannot come back after a save.
        legacy.min_memory_mb = 1024;
        legacy.max_memory_mb = 1024;
        legacy.normalize_loaded(16_384);
        assert_eq!(legacy.min_memory_mb, MIN_MEMORY_MB);
        assert_eq!(legacy.max_memory_mb, MAX_MEMORY_MB);

        // Anything above the shipped maximum is reduced to it.
        legacy.min_memory_mb = 32_768;
        legacy.max_memory_mb = 32_768;
        legacy.normalize_loaded(16_384);
        assert_eq!(legacy.min_memory_mb, MAX_MEMORY_MB);
        assert_eq!(legacy.max_memory_mb, MAX_MEMORY_MB);

        // Choices inside the shipped range are kept.
        legacy.min_memory_mb = MIN_MEMORY_MB;
        legacy.max_memory_mb = 6_144;
        legacy.normalize_loaded(16_384);
        assert_eq!(legacy.min_memory_mb, MIN_MEMORY_MB);
        assert_eq!(legacy.max_memory_mb, 6_144);

        // A machine smaller than the shipped range still gets a usable pair.
        legacy.normalize_loaded(2_048);
        assert_eq!(legacy.min_memory_mb, 2_048);
        assert_eq!(legacy.max_memory_mb, 2_048);
    }

    #[test]
    fn a_machine_smaller_than_the_shipped_range_still_saves() {
        let mut settings = Settings::default();
        settings.normalize_loaded(3_900);
        assert_eq!(settings.min_memory_mb, 3_900);
        assert_eq!(settings.max_memory_mb, 3_900);
        assert!(settings.validate(3_900).is_ok());
    }

    #[test]
    fn validation_rejects_vanilla_and_relative_directories() {
        let mut settings = valid();
        settings.game_dir = "C:\\Users\\me\\.minecraft".to_string();
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "game_dir_is_vanilla");
        settings.game_dir = "relative/path".to_string();
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "game_dir_not_absolute");
        settings.game_dir = test_game_dir();
        assert!(settings.validate(16_384).is_ok());
    }

    #[test]
    fn validation_checks_memory_and_resolution() {
        let mut settings = valid();
        settings.min_memory_mb = 8192;
        settings.max_memory_mb = 4096;
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "memory_max_below_min");
        settings.min_memory_mb = 1024;
        settings.max_memory_mb = 8192;
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "memory_min_too_low");
        settings.min_memory_mb = MIN_MEMORY_MB;
        settings.max_memory_mb = 32_768;
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "memory_max_too_high");
        settings.max_memory_mb = MAX_MEMORY_MB;
        assert_eq!(settings.validate(4_096).unwrap_err().code(), "memory_exceeds_system");
        settings.width = 100;
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "resolution_width_invalid");
    }

    #[test]
    fn backup_due_policy() {
        let mut settings = valid();
        settings.auto_backup = true;
        settings.backup_interval_hours = 24;
        assert!(settings.backup_is_due(1));
        settings.last_backup_ms = 1_000_000;
        assert!(!settings.backup_is_due(1_000_000 + 23 * 3_600_000));
        assert!(settings.backup_is_due(1_000_000 + 24 * 3_600_000));
        settings.auto_backup = false;
        assert!(!settings.backup_is_due(u64::MAX));
    }

    #[test]
    fn usernames_are_rejected_when_invalid() {
        let mut settings = valid();
        settings.username = "no way".to_string();
        assert_eq!(settings.validate(16_384).unwrap_err().code(), "username_invalid");
        settings.username = String::new();
        settings.onboarding_complete = false;
        assert!(settings.validate(16_384).is_ok());
    }
}
