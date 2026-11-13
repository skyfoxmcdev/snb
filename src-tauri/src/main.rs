#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use snb_core::backups::{create_backup, list_backups, restore_backup};
use snb_core::config::RuntimeConfig;
use snb_core::diagnostics::create_diagnostics_archive;
use snb_core::error::{AppError, UserError};
use snb_core::instance::GamePaths;
use snb_core::installer::{install, InstallMode, InstallReport};
use snb_core::launch::{build_launch_plan, spawn_game};
use snb_core::logging::Logger;
use snb_core::readiness::{check_readiness, ReadinessReport};
use snb_core::remote::{RemoteLoader, RemoteSnapshot};
use snb_core::server::{ping, update_server_entry, ServerStatus};
use snb_core::settings::Settings;
use snb_core::updater::{check as check_update, download_update, write_health_marker, UpdateCheck};
use snb_core::util::{format_compact_utc, now_ms, write_json_atomic};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    data_dir: PathBuf,
    runtime: Arc<RuntimeConfig>,
    settings: Arc<Mutex<Settings>>,
    logger: Arc<Logger>,
    remote: Arc<Mutex<RemoteSnapshot>>,
    loader: Arc<RemoteLoader>,
    game_running: Arc<AtomicBool>,
    installing: Arc<AtomicBool>,
    install_cancel: Arc<AtomicBool>,
    update_run_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PathsSummary {
    data_dir: String,
    game_dir: String,
    logs_dir: String,
    backups_dir: String,
    mods_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapPayload {
    launcher_version: String,
    settings: Settings,
    runtime: RuntimeConfig,
    remote: RemoteSnapshot,
    readiness: ReadinessReport,
    server_status: ServerStatus,
    game_running: bool,
    paths: PathsSummary,
    update: UpdateCheck,
    update_run_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModView {
    file_name: String,
    enabled: bool,
    size: u64,
    managed: bool,
    version: Option<String>,
    name: Option<String>,
    mod_id: Option<String>,
    authors: Option<String>,
    description: Option<String>,
    icon: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentView {
    name: String,
    size: u64,
    directory: bool,
}

fn user(error: AppError) -> UserError {
    error.into_user()
}

fn io_error(error: std::io::Error) -> UserError {
    UserError::with_detail("file_error", error.to_string())
}

fn settings_path(state: &AppState) -> PathBuf {
    state.data_dir.join("settings.json")
}

fn current_settings(state: &AppState) -> Settings {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default()
}

fn save_settings_file(state: &AppState, settings: &Settings) -> Result<(), UserError> {
    write_json_atomic(&settings_path(state), settings).map_err(user)
}

fn game_paths(state: &AppState, settings: &Settings) -> GamePaths {
    GamePaths::new(&settings.effective_game_dir(), &state.runtime)
}

fn endpoint(state: &AppState) -> Option<snb_core::ServerEndpoint> {
    state
        .remote
        .lock()
        .ok()
        .and_then(|remote| remote.server.endpoint.clone())
}

#[tauri::command]
fn launcher_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapPayload, UserError> {
    let settings = current_settings(&state);
    let runtime = state.runtime.clone();
    let paths = GamePaths::new(&settings.effective_game_dir(), &runtime);
    let snapshot = state.loader.load();
    if let Some(endpoint) = &snapshot.server.endpoint {
        if paths.root.is_dir() {
            let _ = update_server_entry(&paths.root, &runtime.server_entry_name, endpoint, &state.logger);
        }
    }
    let server_status = snapshot
        .server
        .endpoint
        .as_ref()
        .map(|endpoint| ping(endpoint, Duration::from_secs(4)))
        .unwrap_or_else(ServerStatus::unknown);
    let readiness = check_readiness(&runtime, &paths, &settings, snapshot.server.endpoint.as_ref());
    let update = check_update(env!("CARGO_PKG_VERSION"), &runtime);
    if let Ok(mut remote) = state.remote.lock() {
        *remote = snapshot.clone();
    }
    Ok(BootstrapPayload {
        launcher_version: env!("CARGO_PKG_VERSION").to_string(),
        settings,
        runtime: (*runtime).clone(),
        remote: snapshot,
        readiness,
        server_status,
        game_running: state.game_running.load(Ordering::Relaxed),
        paths: PathsSummary {
            data_dir: state.data_dir.to_string_lossy().to_string(),
            game_dir: paths.root.to_string_lossy().to_string(),
            logs_dir: state.logger.logs_dir().to_string_lossy().to_string(),
            backups_dir: paths.backups.to_string_lossy().to_string(),
            mods_dir: paths.mods.to_string_lossy().to_string(),
        },
        update,
        update_run_id: state.update_run_id.clone(),
    })
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<'_, AppState>, mut settings: Settings) -> Result<Settings, UserError> {
    let total = memory_stats::memory_stats()
        .map(|stats| (stats.physical_mem / 1024 / 1024) as u64)
        .unwrap_or(0);
    settings.normalize_loaded(total);
    settings.validate(total).map_err(user)?;
    save_settings_file(&state, &settings)?;
    if let Ok(mut current) = state.settings.lock() {
        *current = settings.clone();
    }
    state.logger.info("settings", "Preferences saved");
    let _ = app.emit("settings-changed", &settings);
    Ok(settings)
}

#[tauri::command]
async fn pick_directory() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Choose the Sky & Beyond instance directory")
            .pick_folder()
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
async fn pick_java_path() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Choose java.exe")
            .add_filter("Java executable", &["exe"])
            .pick_file()
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
fn refresh_remote(state: State<'_, AppState>) -> Result<RemoteSnapshot, UserError> {
    let snapshot = state.loader.load();
    if let Ok(mut remote) = state.remote.lock() {
        *remote = snapshot.clone();
    }
    Ok(snapshot)
}

#[tauri::command]
async fn install_game(app: AppHandle, state: State<'_, AppState>, repair: bool) -> Result<InstallReport, UserError> {
    if state.game_running.load(Ordering::Relaxed) {
        return Err(UserError::new("game_running"));
    }
    if state
        .installing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(UserError::new("installation_in_progress"));
    }
    state.install_cancel.store(false, Ordering::Relaxed);
    let settings = current_settings(&state);
    let mut working = settings.clone();
    let runtime = state.runtime.clone();
    let logger = state.logger.clone();
    let paths = game_paths(&state, &settings);
    let cancel = state.install_cancel.clone();
    let endpoint = endpoint(&state);
    let emit_handle = app.clone();
    let (result, working) = tauri::async_runtime::spawn_blocking(move || {
        let report = install(
            &runtime,
            &paths,
            &mut working,
            endpoint.as_ref(),
            if repair { InstallMode::Repair } else { InstallMode::Install },
            &cancel,
            &logger,
            &mut |report| {
                let _ = emit_handle.emit("install-progress", report);
            },
        );
        (report, working)
    })
    .await
    .map_err(|error| UserError::with_detail("installation_failed", error.to_string()))?;
    // The install only owns the detected Java path: writing its whole start-up
    // snapshot back would undo preferences changed while it was running.
    let merged = {
        let mut current = current_settings(&state);
        current.java_path = working.java_path.clone();
        current
    };
    if let Ok(mut current) = state.settings.lock() {
        *current = merged.clone();
    }
    let _ = save_settings_file(&state, &merged);
    let _ = app.emit("settings-changed", &merged);
    state.installing.store(false, Ordering::SeqCst);
    let _ = app.emit("install-finished", &result);
    Ok(result)
}

#[tauri::command]
fn cancel_install(state: State<'_, AppState>) {
    state.install_cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn repair_game(app: AppHandle, state: State<'_, AppState>) -> Result<(), UserError> {
    if state.installing.load(Ordering::Relaxed) {
        return Err(UserError::new("installation_in_progress"));
    }
    if state.game_running.load(Ordering::Relaxed) {
        return Err(UserError::new("game_running"));
    }
    let _ = app.emit("open-repair", ());
    Ok(())
}

#[tauri::command]
async fn launch_game(app: AppHandle, state: State<'_, AppState>) -> Result<(), UserError> {
    if state.game_running.load(Ordering::Relaxed) {
        return Err(UserError::new("game_running"));
    }
    if state.installing.load(Ordering::Relaxed) {
        return Err(UserError::new("installation_in_progress"));
    }
    let settings = current_settings(&state);
    let runtime = state.runtime.clone();
    let logger = state.logger.clone();
    let paths = game_paths(&state, &settings);
    let endpoint = endpoint(&state);
    let readiness = check_readiness(&runtime, &paths, &settings, endpoint.as_ref());
    if !readiness.ready {
        return Err(UserError::with_detail(
            "installation_not_ready",
            readiness.issues.join(", "),
        ));
    }
    if let Some(endpoint) = &endpoint {
        update_server_entry(&paths.root, &runtime.server_entry_name, endpoint, &logger).map_err(user)?;
    }
    if settings.backup_is_due(now_ms()) {
        match create_backup(&paths, "auto", false, settings.backup_retention, &logger) {
            Ok(_) => {
                if let Ok(mut current) = state.settings.lock() {
                    current.mark_backup_done(now_ms());
                    let _ = save_settings_file(&state, &current);
                }
            }
            Err(error) => logger.warn("backup", format!("Automatic backup failed: {error}")),
        }
    }
    let java_path = PathBuf::from(settings.java_path.clone());
    let plan = build_launch_plan(&runtime, &paths, &settings, &java_path, env!("CARGO_PKG_VERSION"), &logger)
        .map_err(user)?;
    let child = spawn_game(&plan).map_err(user)?;
    state.game_running.store(true, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
    let _ = app.emit("game-started", &plan);
    let running = state.game_running.clone();
    let handle = app.clone();
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
        running.store(false, Ordering::SeqCst);
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        let _ = handle.emit("game-exited", ());
    });
    Ok(())
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), UserError> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| UserError::new("settings_window_unavailable"))?;
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    // The settings window stays warm while hidden, so ask it to reload the
    // current preferences instead of saving a stale start-up snapshot back.
    let _ = app.emit("settings-refresh", ());
    Ok(())
}

#[tauri::command]
fn mark_update_ready(state: State<'_, AppState>) -> Result<(), UserError> {
    if let Some(run_id) = &state.update_run_id {
        write_health_marker(&state.data_dir, run_id, env!("CARGO_PKG_VERSION"))
            .map_err(user)?;
    }
    Ok(())
}

#[tauri::command]
async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), UserError> {
    if state.game_running.load(Ordering::Relaxed) || state.installing.load(Ordering::Relaxed) {
        return Err(UserError::new("activity_in_progress"));
    }
    let runtime = state.runtime.clone();
    let logger = state.logger.clone();
    let data_dir = state.data_dir.clone();
    let update = tauri::async_runtime::spawn_blocking(move || {
        check_update(env!("CARGO_PKG_VERSION"), &runtime)
    })
    .await
    .map_err(|error| UserError::with_detail("update_failed", error.to_string()))?;
    if let Some(error) = update.error {
        return Err(UserError::with_detail("update_unavailable", error));
    }
    let decision = update.decision.ok_or_else(|| UserError::new("update_not_required"))?;
    let expected_version = decision.latest_version.clone();
    let public_key = state.runtime.update.public_key.clone();
    let updates_dir = state.data_dir.join("updates");
    let cancel = Arc::new(AtomicBool::new(false));
    let emit_handle = app.clone();
    let logger_for_download = logger.clone();
    let staged = tauri::async_runtime::spawn_blocking(move || {
        download_update(
            &decision,
            &updates_dir,
            &public_key,
            &cancel,
            &logger_for_download,
            &mut |current, total| {
                let _ = emit_handle.emit("update-progress", serde_json::json!({ "current": current, "total": total }));
            },
        )
    })
    .await
    .map_err(|error| UserError::with_detail("update_failed", error.to_string()))?
    .map_err(user)?;
    let exe = std::env::current_exe()
        .map_err(|error| UserError::with_detail("update_failed", error.to_string()))?;
    let app_dir = exe
        .parent()
        .ok_or_else(|| UserError::new("update_failed"))?
        .to_path_buf();
    let helper_name = if cfg!(windows) { "snb-updater.exe" } else { "snb-updater" };
    let helper = std::env::var_os("SNB_UPDATER_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| app_dir.join(helper_name));
    if !helper.is_file() {
        return Err(UserError::with_detail(
            "update_helper_missing",
            helper.to_string_lossy().to_string(),
        ));
    }
    let run_id = format!("{}-{}", now_ms(), std::process::id());
    let log_path = state.logger.logs_dir().join("updater.log");
    let backup_dir = snb_core::updater::backup_root(&data_dir);
    std::process::Command::new(&helper)
        .arg("--installer").arg(&staged)
        .arg("--app-exe").arg(&exe)
        .arg("--app-dir").arg(&app_dir)
        .arg("--data-dir").arg(&data_dir)
        .arg("--backup-dir").arg(&backup_dir)
        .arg("--run-id").arg(&run_id)
        .arg("--expected-version").arg(&expected_version)
        .arg("--log").arg(&log_path)
        .spawn()
        .map_err(|error| UserError::with_detail("update_helper_failed", error.to_string()))?;
    logger.info("update", "Transactional updater started; closing the current launcher");
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn refresh_server(state: State<'_, AppState>) -> ServerStatus {
    endpoint(&state)
        .map(|endpoint| ping(&endpoint, Duration::from_secs(4)))
        .unwrap_or_else(ServerStatus::unknown)
}

/// Reports whether Mojang's public metadata endpoint answers. The legacy
/// status.mojang.com feed is retired, so the launcher probes the same metadata
/// host it already depends on instead of inventing a status.
#[tauri::command]
async fn mojang_status() -> String {
    tauri::async_runtime::spawn_blocking(|| {
        const URL: &str =
            "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
        match snb_core::http::Downloader::new() {
            Ok(downloader) if downloader.probe(URL, Duration::from_secs(6)) => {
                "operational".to_string()
            }
            _ => "unavailable".to_string(),
        }
    })
    .await
    .unwrap_or_else(|_| "unknown".to_string())
}

struct ModMetadata {
    name: Option<String>,
    mod_id: Option<String>,
    version: Option<String>,
    authors: Option<String>,
    description: Option<String>,
    icon: Option<String>,
}

fn read_mod_metadata(path: &Path) -> ModMetadata {
    let Ok(file) = fs::File::open(path) else {
        return ModMetadata { name: None, mod_id: None, version: None, authors: None, description: None, icon: None };
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return ModMetadata { name: None, mod_id: None, version: None, authors: None, description: None, icon: None };
    };
    let mut text = None;
    for candidate in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Ok(entry) = archive.by_name(candidate) {
            let mut buffer = String::new();
            if entry.take(2 * 1024 * 1024).read_to_string(&mut buffer).is_ok() {
                text = Some(buffer);
                break;
            }
        }
    }
    let Some(text) = text else {
        return ModMetadata { name: None, mod_id: None, version: None, authors: None, description: None, icon: None };
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return ModMetadata { name: None, mod_id: None, version: None, authors: None, description: None, icon: None };
    };
    let Some(module) = value.get("mods").and_then(|mods| mods.as_array()).and_then(|mods| mods.first()) else {
        return ModMetadata { name: None, mod_id: None, version: None, authors: None, description: None, icon: None };
    };
    let get = |key: &str| module.get(key).and_then(|value| value.as_str()).map(str::to_string);
    let authors = module.get("authors").and_then(|value| {
        value.as_str().map(str::to_string).or_else(|| {
            value.as_array().map(|values| values.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
        })
    }).filter(|value| !value.is_empty());
    let logo = get("logoFile");
    let icon = logo.and_then(|logo| {
        let mut entry = archive.by_name(&logo).ok()?;
        if entry.size() > 256 * 1024 { return None; }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).ok()?;
        use base64::Engine;
        let mime = if logo.to_ascii_lowercase().ends_with(".jpg") || logo.to_ascii_lowercase().ends_with(".jpeg") { "image/jpeg" } else if logo.to_ascii_lowercase().ends_with(".webp") { "image/webp" } else { "image/png" };
        Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    });
    ModMetadata {
        name: get("displayName"),
        mod_id: get("modId"),
        version: get("version"),
        authors,
        description: get("description").map(|value| value.chars().take(240).collect()),
        icon,
    }
}

fn mod_version(name: &str) -> Option<String> {
    let stem = name.trim_end_matches(".jar");
    stem.rsplit_once('-')
        .map(|(_, version)| version.to_string())
        .filter(|version| version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
}

fn is_managed_mod(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("automodpack-")
}

fn add_jar(source: &Path, destination: &Path) -> Result<(), UserError> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| UserError::new("invalid_file_name"))?;
    if !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(UserError::new("mod_invalid"));
    }
    fs::create_dir_all(destination).map_err(io_error)?;
    let target = destination.join(name);
    if target.exists() {
        return Err(UserError::new("mod_exists").param("name", name));
    }
    let temp = destination.join(format!(".{name}.{}.tmp", now_ms()));
    fs::copy(source, &temp).map_err(io_error)?;
    fs::rename(temp, target).map_err(io_error)?;
    Ok(())
}

#[tauri::command]
fn list_mods(state: State<'_, AppState>) -> Vec<ModView> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let mut mods = Vec::new();
    for (directory, enabled) in [(&paths.mods, true), (&paths.mods_disabled, false)] {
        let Ok(entries) = fs::read_dir(directory) else { continue };
        for entry in entries.flatten().filter(|entry| entry.path().is_file()) {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !file_name.to_ascii_lowercase().ends_with(".jar") {
                continue;
            }
            let metadata = read_mod_metadata(&entry.path());
            mods.push(ModView {
                version: metadata.version.or_else(|| mod_version(&file_name)),
                managed: is_managed_mod(&file_name),
                enabled,
                size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                name: metadata.name,
                mod_id: metadata.mod_id,
                authors: metadata.authors,
                description: metadata.description,
                icon: metadata.icon,
                file_name,
            });
        }
    }
    mods.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
    mods
}

#[tauri::command]
fn add_mod_paths(state: State<'_, AppState>, paths: Vec<String>) -> Result<usize, UserError> {
    let settings = current_settings(&state);
    let game_paths = game_paths(&state, &settings);
    let mut added = 0;
    for path in paths {
        let source = PathBuf::from(path);
        if source.is_file() && add_jar(&source, &game_paths.mods).is_ok() {
            added += 1;
        }
    }
    Ok(added)
}

fn safe_world_dir(paths: &GamePaths, world: &str) -> Result<PathBuf, UserError> {
    if world.is_empty()
        || world == "."
        || world == ".."
        || Path::new(world).components().count() != 1
    {
        return Err(UserError::new("invalid_world_name"));
    }
    let directory = paths.saves.join(world);
    if !directory.is_dir() {
        return Err(UserError::new("world_missing"));
    }
    Ok(directory)
}

#[tauri::command]
fn list_worlds(state: State<'_, AppState>) -> Vec<String> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let Ok(entries) = fs::read_dir(&paths.saves) else { return Vec::new() };
    let mut worlds: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir() && entry.path().join("level.dat").is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    worlds.sort_by_key(|name| name.to_lowercase());
    worlds
}

#[tauri::command]
fn list_datapacks(state: State<'_, AppState>, world: String) -> Result<Vec<ContentView>, UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let directory = safe_world_dir(&paths, &world)?.join("datapacks");
    let Ok(entries) = fs::read_dir(directory) else { return Ok(Vec::new()) };
    let mut content: Vec<ContentView> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { return None; }
            Some(ContentView {
                name,
                size: if metadata.is_dir() { 0 } else { metadata.len() },
                directory: metadata.is_dir(),
            })
        })
        .collect();
    content.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(content)
}

#[tauri::command]
async fn pick_and_add_datapacks(
    state: State<'_, AppState>,
    world: String,
) -> Result<usize, UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let directory = safe_world_dir(&paths, &world)?.join("datapacks");
    tauri::async_runtime::spawn_blocking(move || {
        let files = rfd::FileDialog::new()
            .add_filter("Datapack zip", &["zip"])
            .set_title("Add datapacks")
            .pick_files()
            .unwrap_or_default();
        fs::create_dir_all(&directory).map_err(io_error)?;
        let mut added = 0;
        for file in files {
            if let Some(name) = file.file_name() {
                let destination = directory.join(name);
                if !destination.exists() && fs::copy(&file, destination).is_ok() {
                    added += 1;
                }
            }
        }
        Ok(added)
    })
    .await
    .unwrap_or_else(|_| Ok(0))
}

#[tauri::command]
fn remove_datapack(
    state: State<'_, AppState>,
    world: String,
    name: String,
) -> Result<(), UserError> {
    if name.is_empty() || Path::new(&name).components().count() != 1 {
        return Err(UserError::new("invalid_file_name"));
    }
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let target = safe_world_dir(&paths, &world)?.join("datapacks").join(&name);
    if target.is_dir() {
        fs::remove_dir_all(target).map_err(io_error)?;
    } else if target.is_file() {
        fs::remove_file(target).map_err(io_error)?;
    } else {
        return Err(UserError::new("content_missing"));
    }
    Ok(())
}

#[tauri::command]
fn open_datapacks(state: State<'_, AppState>, world: String) -> Result<(), UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let directory = safe_world_dir(&paths, &world)?.join("datapacks");
    fs::create_dir_all(&directory).map_err(io_error)?;
    opener::open(&directory).map_err(|error| UserError::with_detail("open_failed", error.to_string()))
}

#[tauri::command]
async fn pick_and_add_mods(state: State<'_, AppState>) -> Result<usize, UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    tauri::async_runtime::spawn_blocking(move || {
        let files = rfd::FileDialog::new()
            .add_filter("Minecraft mod", &["jar"])
            .set_title("Add Minecraft mods")
            .pick_files()
            .unwrap_or_default();
        let mut added = 0;
        for file in files {
            if add_jar(&file, &paths.mods).is_ok() {
                added += 1;
            }
        }
        Ok(added)
    })
    .await
    .unwrap_or_else(|_| Ok(0))
}

#[tauri::command]
fn toggle_mod(state: State<'_, AppState>, file_name: String, enabled: bool) -> Result<(), UserError> {
    if is_managed_mod(&file_name) {
        return Err(UserError::new("automodpack_protected"));
    }
    let name = Path::new(&file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| UserError::new("invalid_file_name"))?;
    if name != file_name || !file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(UserError::new("invalid_file_name"));
    }
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let (source, target_dir) = if enabled {
        (paths.mods_disabled.join(name), paths.mods.clone())
    } else {
        (paths.mods.join(name), paths.mods_disabled.clone())
    };
    if !source.is_file() {
        return Err(UserError::new("mod_missing"));
    }
    fs::create_dir_all(&target_dir).map_err(io_error)?;
    if target_dir.join(name).exists() {
        return Err(UserError::new("mod_exists"));
    }
    fs::rename(&source, target_dir.join(name)).map_err(io_error)?;
    Ok(())
}

#[tauri::command]
fn remove_mod(state: State<'_, AppState>, file_name: String) -> Result<(), UserError> {
    if is_managed_mod(&file_name) {
        return Err(UserError::new("automodpack_protected"));
    }
    if Path::new(&file_name).components().count() != 1 {
        return Err(UserError::new("invalid_file_name"));
    }
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let mut removed = false;
    for directory in [&paths.mods, &paths.mods_disabled] {
        let target = directory.join(&file_name);
        if target.is_file() {
            fs::remove_file(target).map_err(io_error)?;
            removed = true;
        }
    }
    if !removed {
        return Err(UserError::new("mod_missing"));
    }
    Ok(())
}

fn content_directory(paths: &GamePaths, kind: &str) -> Option<PathBuf> {
    match kind {
        "shader" => Some(paths.shaderpacks.clone()),
        "resourcepack" => Some(paths.resourcepacks.clone()),
        _ => None,
    }
}

#[tauri::command]
fn list_content(state: State<'_, AppState>, kind: String) -> Vec<ContentView> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let Some(directory) = content_directory(&paths, &kind) else { return Vec::new() };
    let Ok(entries) = fs::read_dir(directory) else { return Vec::new() };
    let mut content: Vec<ContentView> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            Some(ContentView {
                name,
                size: if metadata.is_dir() {
                    0
                } else {
                    metadata.len()
                },
                directory: metadata.is_dir(),
            })
        })
        .collect();
    content.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    content
}

#[tauri::command]
async fn pick_and_add_content(
    state: State<'_, AppState>,
    kind: String,
) -> Result<usize, UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let Some(directory) = content_directory(&paths, &kind) else {
        return Err(UserError::new("content_kind_invalid"));
    };
    tauri::async_runtime::spawn_blocking(move || {
        let files = rfd::FileDialog::new()
            .add_filter("Zip archive", &["zip"])
            .set_title("Add content")
            .pick_files()
            .unwrap_or_default();
        fs::create_dir_all(&directory).map_err(io_error)?;
        let mut added = 0;
        for file in files {
            if let Some(name) = file.file_name() {
                let destination = directory.join(name);
                if !destination.exists() && fs::copy(&file, destination).is_ok() {
                    added += 1;
                }
            }
        }
        Ok(added)
    })
    .await
    .unwrap_or_else(|_| Ok(0))
}

#[tauri::command]
fn remove_content(state: State<'_, AppState>, kind: String, name: String) -> Result<(), UserError> {
    if Path::new(&name).components().count() != 1 {
        return Err(UserError::new("invalid_file_name"));
    }
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let directory = content_directory(&paths, &kind).ok_or_else(|| UserError::new("content_kind_invalid"))?;
    let target = directory.join(&name);
    if target.is_dir() {
        fs::remove_dir_all(target).map_err(io_error)?;
    } else if target.is_file() {
        fs::remove_file(target).map_err(io_error)?;
    } else {
        return Err(UserError::new("content_missing"));
    }
    Ok(())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), UserError> {
    snb_core::util::validate_url(&url, false)
        .map_err(|_| UserError::new("invalid_url"))?;
    opener::open(&url).map_err(|error| UserError::with_detail("open_failed", error.to_string()))
}

#[tauri::command]
fn open_folder(state: State<'_, AppState>, kind: String) -> Result<(), UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let path = match kind.as_str() {
        "game" => paths.root,
        "logs" => state.logger.logs_dir().to_path_buf(),
        "mods" => paths.mods,
        "backups" => paths.backups,
        "shaders" => paths.shaderpacks,
        "resourcepacks" => paths.resourcepacks,
        "saves" => paths.saves,
        _ => return Err(UserError::new("folder_kind_invalid")),
    };
    fs::create_dir_all(&path).map_err(io_error)?;
    opener::open(&path).map_err(|error| UserError::with_detail("open_failed", error.to_string()))?;
    Ok(())
}

#[tauri::command]
fn get_backups(state: State<'_, AppState>) -> Vec<snb_core::backups::BackupEntry> {
    let settings = current_settings(&state);
    list_backups(&game_paths(&state, &settings))
}

#[tauri::command]
fn make_backup(state: State<'_, AppState>) -> Result<snb_core::backups::BackupEntry, UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let entry = create_backup(
        &paths,
        "manual",
        state.game_running.load(Ordering::Relaxed),
        settings.backup_retention,
        &state.logger,
    )
    .map_err(user)?;
    if let Ok(mut current) = state.settings.lock() {
        current.mark_backup_done(now_ms());
        let _ = save_settings_file(&state, &current);
    }
    Ok(entry)
}

#[tauri::command]
fn restore_worlds(
    state: State<'_, AppState>,
    file_name: String,
    confirmed: bool,
) -> Result<(), UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    restore_backup(
        &paths,
        &file_name,
        confirmed,
        state.game_running.load(Ordering::Relaxed),
        &state.logger,
    )
    .map(|_| ())
    .map_err(user)
}

#[tauri::command]
async fn export_diagnostics(state: State<'_, AppState>) -> Result<snb_core::diagnostics::DiagnosticsInfo, UserError> {
    let settings = current_settings(&state);
    let paths = game_paths(&state, &settings);
    let runtime = state.runtime.clone();
    let logger = state.logger.clone();
    let default_name = format!("sky-and-beyond-diagnostics-{}.zip", format_compact_utc(now_ms()));
    tauri::async_runtime::spawn_blocking(move || {
        let output = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Zip archive", &["zip"])
            .save_file()
            .ok_or_else(|| UserError::new("export_cancelled"))?;
        create_diagnostics_archive(
            &runtime,
            &paths,
            &settings,
            env!("CARGO_PKG_VERSION"),
            &output,
            &logger,
        )
        .map_err(user)
    })
    .await
    .map_err(|error| UserError::with_detail("export_failed", error.to_string()))?
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            fs::create_dir_all(&data_dir)?;
            let runtime = Arc::new(RuntimeConfig::from_json(
                include_str!("../resources/launcher-config.json"),
                cfg!(debug_assertions),
            )?);
            let update_run_id = std::env::args().find_map(|argument| {
                argument
                    .strip_prefix("--snb-update-run=")
                    .map(str::to_string)
            });
            let logger = Logger::new(data_dir.join("logs"))?;
            let settings_file = data_dir.join("settings.json");
            let mut settings = snb_core::util::read_json::<Settings>(&settings_file).unwrap_or_default();
            let total = memory_stats::memory_stats()
                .map(|stats| (stats.physical_mem / 1024 / 1024) as u64)
                .unwrap_or(0);
            settings.normalize_loaded(total);
            if !settings_file.exists() {
                write_json_atomic(&settings_file, &settings)?;
            }
            let loader = RemoteLoader::new(
                &runtime.remote_base_url,
                data_dir.join("remote-cache"),
                cfg!(debug_assertions),
                logger.clone(),
            )?;
            let remote = loader.load();
            app.manage(AppState {
                data_dir,
                runtime,
                settings: Arc::new(Mutex::new(settings)),
                logger,
                remote: Arc::new(Mutex::new(remote)),
                loader: Arc::new(loader),
                game_running: Arc::new(AtomicBool::new(false)),
                installing: Arc::new(AtomicBool::new(false)),
                install_cancel: Arc::new(AtomicBool::new(false)),
                update_run_id,
            });
            // The windows are declared with `create: false` in tauri.conf.json and
            // are built here, so a webview can only load once the state above is
            // registered. Creating them earlier let the frontend call `bootstrap`
            // before `manage()` had run, which rejected the first command of the
            // session with "state not managed for field state" and forced a retry.
            for window_config in app.config().app.windows.clone() {
                tauri::WebviewWindowBuilder::from_config(app.handle(), &window_config)?.build()?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            launcher_version,
            bootstrap,
            save_settings,
            pick_directory,
            pick_java_path,
            refresh_remote,
            install_game,
            cancel_install,
            repair_game,
            launch_game,
            open_settings_window,
            mark_update_ready,
            install_update,
            refresh_server,
            mojang_status,
            list_mods,
            add_mod_paths,
            pick_and_add_mods,
            toggle_mod,
            remove_mod,
            list_content,
            pick_and_add_content,
            remove_content,
            list_worlds,
            list_datapacks,
            pick_and_add_datapacks,
            remove_datapack,
            open_datapacks,
            open_folder,
            open_url,
            get_backups,
            make_backup,
            restore_worlds,
            export_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Sky & Beyond launcher");
}
