use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;

use crate::automodpack::ensure_automodpack;
use crate::config::{RuntimeConfig, ServerEndpoint};
use crate::error::{AppError, UserError};
use crate::instance::{ensure_instance, GamePaths};
use crate::java::{ensure_java, JavaReport};
use crate::logging::Logger;
use crate::minecraft::{
    ensure_assets, ensure_client_jar, ensure_version_metadata, install_libraries,
};
use crate::neoforge::install_neoforge;
use crate::readiness::check_readiness;
use crate::server::update_server_entry;
use crate::settings::Settings;
use crate::util::{available_space, format_compact_utc, is_vanilla_minecraft_dir, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMode {
    Install,
    Repair,
}

impl InstallMode {
    fn as_str(self) -> &'static str {
        match self {
            InstallMode::Install => "install",
            InstallMode::Repair => "repair",
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StepProgress {
    pub current: u64,
    pub total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStep {
    pub id: String,
    /// "pending", "running", "done", "warning" or "failed".
    pub status: String,
    pub progress: Option<StepProgress>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub mode: String,
    pub running: bool,
    pub ready: bool,
    pub steps: Vec<InstallStep>,
    pub current_step: Option<String>,
    pub error: Option<UserError>,
    pub warnings: Vec<UserError>,
    pub java: Option<JavaReport>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

const STEP_IDS: &[&str] = &[
    "prepare",
    "java",
    "minecraft",
    "libraries",
    "assets",
    "neoforge",
    "automodpack",
    "instance",
    "server",
    "verify",
];

fn new_report(mode: InstallMode) -> InstallReport {
    InstallReport {
        mode: mode.as_str().to_string(),
        running: true,
        ready: false,
        steps: STEP_IDS
            .iter()
            .map(|id| InstallStep {
                id: id.to_string(),
                status: "pending".to_string(),
                progress: None,
                warning: None,
            })
            .collect(),
        current_step: None,
        error: None,
        warnings: Vec::new(),
        java: None,
        started_at_ms: now_ms(),
        finished_at_ms: None,
    }
}

fn step_mut<'a>(report: &'a mut InstallReport, id: &str) -> &'a mut InstallStep {
    report
        .steps
        .iter_mut()
        .find(|step| step.id == id)
        .expect("known install step")
}

fn begin(report: &mut InstallReport, id: &str, on_update: &mut dyn FnMut(&InstallReport)) {
    report.current_step = Some(id.to_string());
    let step = step_mut(report, id);
    step.status = "running".to_string();
    step.progress = None;
    on_update(report);
}

fn finish(
    report: &mut InstallReport,
    id: &str,
    warning: Option<String>,
    on_update: &mut dyn FnMut(&InstallReport),
) {
    let step = step_mut(report, id);
    step.status = if warning.is_some() { "warning" } else { "done" }.to_string();
    step.warning = warning;
    on_update(report);
}

fn fail(
    mut report: InstallReport,
    id: &str,
    error: AppError,
    paths: &GamePaths,
    logger: &Arc<Logger>,
    on_update: &mut dyn FnMut(&InstallReport),
) -> InstallReport {
    let step = step_mut(&mut report, id);
    step.status = "failed".to_string();
    logger.error("install", format!("{id} failed: {error}"));
    let detail = error.user.detail.clone();
    report.error = Some(error.into_user());
    report.running = false;
    report.finished_at_ms = Some(now_ms());
    let _ = detail;
    persist_state(paths, &report);
    on_update(&report);
    report
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

fn persist_state(paths: &GamePaths, report: &InstallReport) {
    let state = serde_json::json!({
        "schemaVersion": 1,
        "mode": report.mode,
        "ready": report.ready,
        "error": report.error,
        "completedSteps": report.steps.iter().filter(|step| step.status == "done" || step.status == "warning").map(|step| &step.id).collect::<Vec<_>>(),
        "updatedAt": format_compact_utc(now_ms()),
    });
    let _ = fs::create_dir_all(&paths.managed);
    let _ = crate::util::write_json_atomic(&paths.managed.join("install-state.json"), &state);
}

fn update_progress(
    report: &mut InstallReport,
    id: &str,
    current: u64,
    total: u64,
    files_done: u64,
    files_total: u64,
    unit: &str,
    on_update: &mut dyn FnMut(&InstallReport),
) {
    step_mut(report, id).progress = Some(StepProgress {
        current,
        total,
        files_done,
        files_total,
        unit: unit.to_string(),
    });
    on_update(report);
}

/// Runs a complete, idempotent installation. Every step is safe to rerun and
/// all downloads resume or verify existing files. Progress is emitted only from
/// real work; the function returns a failed report (never a fake success).
pub fn install(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    settings: &mut Settings,
    endpoint: Option<&ServerEndpoint>,
    mode: InstallMode,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    on_update: &mut dyn FnMut(&InstallReport),
) -> InstallReport {
    let mut report = new_report(mode);
    on_update(&report);

    // 1. Prepare and validate the dedicated directory.
    begin(&mut report, "prepare", on_update);
    if is_vanilla_minecraft_dir(&paths.root) {
        return fail(
            report,
            "prepare",
            AppError::new("game_dir_is_vanilla"),
            paths,
            logger,
            on_update,
        );
    }
    if let Err(error) = fs::create_dir_all(&paths.managed) {
        return fail(report, "prepare", error.into(), paths, logger, on_update);
    }
    match available_space(&paths.root) {
        Ok(free) if free < 1_800 * 1024 * 1024 => {
            return fail(
                report,
                "prepare",
                AppError::with_detail(
                    "disk_full",
                    format!("installation requires at least 1.8 GiB free, found {free} bytes"),
                ),
                paths,
                logger,
                on_update,
            )
        }
        Ok(_) => {}
        Err(error) => return fail(report, "prepare", error, paths, logger, on_update),
    }
    finish(&mut report, "prepare", None, on_update);
    if cancelled(cancel) {
        return fail(report, "prepare", AppError::new("cancelled"), paths, logger, on_update);
    }

    // 2. Java.
    begin(&mut report, "java", on_update);
    let mut java_progress = |current: u64, total: u64| {
        update_progress(&mut report, "java", current, total, 0, 0, "bytes", on_update);
    };
    let java_result = ensure_java(
        runtime,
        &settings.java_path,
        &paths.root,
        settings.auto_install_java,
        cancel,
        logger,
        &mut java_progress,
    );
    drop(java_progress);
    let (java_report, selected_java) = match java_result {
        Ok(value) => value,
        Err(error) => return fail(report, "java", error, paths, logger, on_update),
    };
    settings.java_path = selected_java.path.clone();
    report.java = Some(java_report.clone());
    finish(&mut report, "java", java_report.warning.clone(), on_update);
    if cancelled(cancel) {
        return fail(report, "java", AppError::new("cancelled"), paths, logger, on_update);
    }

    // 3. Official Minecraft metadata and client JAR.
    begin(&mut report, "minecraft", on_update);
    let vanilla = match ensure_version_metadata(runtime, paths, mode == InstallMode::Repair, cancel, logger) {
        Ok(version) => version,
        Err(error) => return fail(report, "minecraft", error, paths, logger, on_update),
    };
    let mut client_progress = |current: u64, total: u64| {
        update_progress(&mut report, "minecraft", current, total, 0, 0, "bytes", on_update);
    };
    let client_result = ensure_client_jar(&vanilla, paths, cancel, logger, &mut client_progress);
    drop(client_progress);
    if let Err(error) = client_result {
        return fail(report, "minecraft", error, paths, logger, on_update);
    }
    finish(&mut report, "minecraft", None, on_update);

    // 4. Vanilla libraries.
    begin(&mut report, "libraries", on_update);
    let mut library_progress = |done: u64, total: u64| {
        update_progress(&mut report, "libraries", done, total, done, total, "files", on_update);
    };
    let libraries_result = install_libraries(&vanilla, paths, cancel, logger, &mut library_progress);
    drop(library_progress);
    if let Err(error) = libraries_result {
        return fail(report, "libraries", error, paths, logger, on_update);
    }
    finish(&mut report, "libraries", None, on_update);

    // 5. Assets and logging configuration.
    begin(&mut report, "assets", on_update);
    let mut asset_progress = |progress: &crate::minecraft::AssetProgress| {
        update_progress(
            &mut report,
            "assets",
            progress.bytes,
            progress.total_bytes,
            progress.files,
            progress.total_files,
            "bytes",
            on_update,
        );
    };
    let assets_result = ensure_assets(
        &vanilla,
        paths,
        mode == InstallMode::Repair,
        cancel,
        logger,
        &mut asset_progress,
    );
    drop(asset_progress);
    if let Err(error) = assets_result {
        return fail(report, "assets", error, paths, logger, on_update);
    }
    finish(&mut report, "assets", None, on_update);

    // 6. NeoForge.
    begin(&mut report, "neoforge", on_update);
    let mut neoforge_progress = |current: u64, total: u64| {
        update_progress(&mut report, "neoforge", current, total, 0, 0, "bytes", on_update);
    };
    let neoforge_result = install_neoforge(
        runtime,
        paths,
        std::path::Path::new(&settings.java_path),
        mode == InstallMode::Repair,
        cancel,
        logger,
        &mut neoforge_progress,
    );
    drop(neoforge_progress);
    if let Err(error) = neoforge_result {
        return fail(report, "neoforge", error, paths, logger, on_update);
    }
    finish(&mut report, "neoforge", None, on_update);

    // 7. AutoModpack.
    begin(&mut report, "automodpack", on_update);
    let mut automodpack_progress = |current: u64, total: u64| {
        update_progress(&mut report, "automodpack", current, total, 0, 0, "bytes", on_update);
    };
    let automodpack_result = ensure_automodpack(
        runtime,
        paths,
        cancel,
        logger,
        &mut automodpack_progress,
    );
    drop(automodpack_progress);
    match automodpack_result {
        Ok(status) => finish(&mut report, "automodpack", status.message_code, on_update),
        Err(error) => return fail(report, "automodpack", error, paths, logger, on_update),
    }

    // 8. Instance metadata/options.
    begin(&mut report, "instance", on_update);
    if let Err(error) = ensure_instance(paths, runtime, &settings.language, logger) {
        return fail(report, "instance", error, paths, logger, on_update);
    }
    finish(&mut report, "instance", None, on_update);

    // 9. Server entry. Endpoint absence is a warning, not a blocker.
    begin(&mut report, "server", on_update);
    let server_warning = match endpoint {
        Some(endpoint) => match update_server_entry(&paths.root, &runtime.server_entry_name, endpoint, logger) {
            Ok(_) => None,
            Err(error) => {
                logger.warn("server", format!("Could not update servers.dat: {error}"));
                Some("server_entry_failed".to_string())
            }
        },
        None => Some("server_endpoint_missing".to_string()),
    };
    finish(&mut report, "server", server_warning, on_update);
    if cancelled(cancel) {
        return fail(report, "server", AppError::new("cancelled"), paths, logger, on_update);
    }

    // 10. Independent verification gates readiness.
    begin(&mut report, "verify", on_update);
    let readiness = check_readiness(runtime, paths, settings, endpoint);
    report.java = Some(readiness.java.clone());
    if !readiness.ready {
        let error = AppError::with_detail(
            "verification_failed",
            format!("installation issues: {}", readiness.issues.join(", ")),
        );
        return fail(report, "verify", error, paths, logger, on_update);
    }
    finish(&mut report, "verify", None, on_update);
    report.running = false;
    report.ready = true;
    report.current_step = None;
    report.finished_at_ms = Some(now_ms());
    for code in readiness.issues.iter() {
        if !report.warnings.iter().any(|warning| &warning.code == code) {
            report.warnings.push(UserError::new(code.clone()));
        }
    }
    persist_state(paths, &report);
    logger.info("install", "Installation verified and ready");
    on_update(&report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_starts_with_all_steps_pending() {
        let report = new_report(InstallMode::Repair);
        assert!(report.running);
        assert!(!report.ready);
        assert_eq!(report.mode, "repair");
        assert_eq!(report.steps.len(), STEP_IDS.len());
        assert!(report.steps.iter().all(|step| step.status == "pending"));
    }
}
