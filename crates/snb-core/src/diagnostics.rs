use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::automodpack::detect;
use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::instance::{read_instance, GamePaths};
use crate::java::detect_java;
use crate::logging::Logger;
use crate::readiness::check_readiness;
use crate::settings::Settings;
use crate::util::{now_ms, redact_text};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsInfo {
    pub path: String,
    pub size: u64,
    pub created_at_ms: u64,
}

fn redact_home(text: &str) -> String {
    let mut value = redact_text(text);
    if let Ok(re) = regex::Regex::new(r"(?i)\b(token|secret|password|authorization)\b\s*[:=]\s*\S+") {
        value = re.replace_all(&value, "$1=[redacted]").to_string();
    }
    if let Some(home) = dirs::home_dir() {
        value = value.replace(&home.to_string_lossy().to_string(), "~");
    }
    value
}

fn add_text(zip: &mut zip::ZipWriter<File>, name: &str, text: &str) -> AppResult<()> {
    use zip::write::SimpleFileOptions;
    zip.start_file(name, SimpleFileOptions::default())?;
    zip.write_all(redact_home(text).as_bytes())?;
    Ok(())
}

fn mod_summary(paths: &GamePaths) -> Vec<serde_json::Value> {
    let mut mods = Vec::new();
    for (directory, enabled) in [(&paths.mods, true), (&paths.mods_disabled, false)] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten().filter(|entry| entry.path().is_file()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with(".jar") {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            mods.push(serde_json::json!({
                "fileName": redact_home(&name),
                "size": size,
                "enabled": enabled,
                "autoModpackManaged": name.to_ascii_lowercase().starts_with("automodpack-"),
            }));
        }
    }
    mods
}

fn collect_logs(logger: &Logger, paths: &GamePaths) -> Vec<PathBuf> {
    let mut logs = logger.log_files();
    for name in ["latest.log", "debug.log"] {
        let path = paths.logs.join(name);
        if path.is_file() {
            logs.push(path);
        }
    }
    logs.truncate(8);
    logs
}

/// Builds a local diagnostic archive. Nothing is uploaded. Logs and text
/// entries are redacted for IP literals, tokens and the user's home path.
pub fn create_diagnostics_archive(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    settings: &Settings,
    launcher_version: &str,
    output: &Path,
    logger: &Arc<Logger>,
) -> AppResult<DiagnosticsInfo> {
    if !output
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        return Err(AppError::new("invalid_file_name"));
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = output.with_extension("zip.part");
    let _ = fs::remove_file(&part);
    let file = File::create(&part)?;
    let mut zip = zip::ZipWriter::new(file);
    let java = detect_java(
        &settings.java_path,
        &paths.root.join("java").join(format!("runtime-{}", runtime.java_major)),
        runtime.java_major,
    );
    let instance = read_instance(&paths.root);
    let automodpack = detect(paths, &runtime.automodpack.version);
    let readiness = check_readiness(runtime, paths, settings, None);
    let report = serde_json::json!({
        "schemaVersion": 1,
        "launcherVersion": launcher_version,
        "generatedAtMs": now_ms(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "java": java,
        "minecraftVersion": runtime.minecraft.version,
        "neoforgeVersion": runtime.neoforge.version,
        "automodpackVersion": runtime.automodpack.version,
        "automodpack": automodpack,
        "instance": instance,
        "readiness": readiness,
        "memory": { "minMb": settings.min_memory_mb, "maxMb": settings.max_memory_mb },
        "mods": mod_summary(paths),
        "privacy": "No telemetry. This archive was created locally and must be shared manually."
    });
    add_text(
        &mut zip,
        "diagnostics.json",
        &serde_json::to_string_pretty(&report)?,
    )?;
    for path in collect_logs(logger, paths) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("log.txt");
        let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if bytes > 8 * 1024 * 1024 {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            add_text(&mut zip, &format!("logs/{name}"), &text)?;
        }
    }
    let crash = paths.logs.parent().map(|_| paths.root.join("crash-reports"));
    if let Some(crash) = crash {
        if let Ok(entries) = fs::read_dir(crash) {
            for entry in entries.flatten().take(5) {
                if let Ok(text) = fs::read_to_string(entry.path()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    add_text(&mut zip, &format!("crash-reports/{name}"), &text)?;
                }
            }
        }
    }
    let file = zip.finish()?;
    file.sync_all()?;
    fs::rename(&part, output)?;
    let info = DiagnosticsInfo {
        path: output.to_string_lossy().to_string(),
        size: fs::metadata(output)?.len(),
        created_at_ms: now_ms(),
    };
    logger.info("diagnostics", "Diagnostic archive exported locally");
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_archive_redacts_ip_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        fs::create_dir_all(&paths.logs).unwrap();
        fs::write(
            paths.logs.join("latest.log"),
            "server=203.0.113.50 token=abcdefghijklmnopqrstuvwxyz\n",
        )
        .unwrap();
        let logger = Logger::new(dir.path().join("launcher-logs")).unwrap();
        let output = dir.path().join("diagnostics.zip");
        create_diagnostics_archive(
            &runtime,
            &paths,
            &Settings::default(),
            "0.1.0",
            &output,
            &logger,
        )
        .unwrap();
        let file = File::open(&output).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut text = String::new();
        {
            let mut entry = archive.by_name("logs/latest.log").unwrap();
            std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
        }
        assert!(!text.contains("203.0.113.50"));
        assert!(!text.contains("abcdefghijklmnopqrstuvwxyz"));
    }
}
