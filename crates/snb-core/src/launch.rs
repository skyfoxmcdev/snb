use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use serde::Serialize;

use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::instance::{apply_fullscreen, GamePaths};
use crate::java::inspect_java;
use crate::logging::Logger;
use crate::minecraft::{
    broken_installation, library_destination, library_is_allowed, logging_config_path,
    resolve_version, rules_allow, Argument, ArgumentValue, ResolvedVersion,
};
use crate::neoforge::profile_id;
use crate::settings::Settings;
use crate::util::{format_compact_utc, now_ms, split_args, username_is_valid};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPlan {
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: String,
    pub log_path: String,
    pub version_id: String,
}

fn substitute(template: &str, replacements: &HashMap<String, String>) -> String {
    let mut output = template.to_string();
    for (key, value) in replacements {
        output = output.replace(&format!("${{{key}}}"), value);
    }
    output
}

fn argument_values(argument: &Argument) -> Vec<String> {
    match argument {
        Argument::Plain(value) => vec![value.clone()],
        Argument::Conditional { value, .. } => match value {
            ArgumentValue::One(value) => vec![value.clone()],
            ArgumentValue::Many(values) => values.clone(),
        },
    }
}

fn collect_arguments(
    arguments: &[Argument],
    features: &HashMap<String, bool>,
    replacements: &HashMap<String, String>,
    strict: bool,
) -> AppResult<Vec<String>> {
    let mut collected = Vec::new();
    for argument in arguments {
        match argument {
            Argument::Plain(value) => collected.push(substitute(value, replacements)),
            Argument::Conditional { rules, .. } => {
                if rules_allow(rules, features) {
                    collected.extend(
                        argument_values(argument)
                            .into_iter()
                            .map(|value| substitute(&value, replacements)),
                    );
                }
            }
        }
    }
    if strict {
        if let Some(value) = collected.iter().find(|value| value.contains("${")) {
            return Err(AppError::with_detail(
                "unresolved_placeholder",
                format!("unresolved launch placeholder in {value}"),
            ));
        }
    }
    Ok(collected)
}

fn classpath_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

/// NeoForge's JVM arguments use `${version_name}` to identify the game JAR
/// that must be ignored on the module path, for example
/// `-DignoreList=client-extra,1.21.1.jar`. Game arguments still receive the
/// launched profile ID (such as `neoforge-21.1.248`).
fn version_name_replacements(
    replacements: &HashMap<String, String>,
    version_name: &str,
) -> HashMap<String, String> {
    let mut result = replacements.clone();
    result.insert("version_name".to_string(), version_name.to_string());
    result
}

fn validate_launch_inputs(paths: &GamePaths, version: &ResolvedVersion) -> AppResult<()> {
    let broken = broken_installation(paths, version);
    if !broken.is_empty() {
        return Err(AppError::with_detail(
            "installation_incomplete",
            format!(
                "{} installation file(s) are missing or unverified",
                broken.len()
            ),
        ));
    }
    Ok(())
}

pub fn build_launch_plan(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    settings: &Settings,
    java_path: &Path,
    launcher_version: &str,
    logger: &Arc<Logger>,
) -> AppResult<LaunchPlan> {
    if !username_is_valid(&settings.username) {
        return Err(AppError::new("username_invalid"));
    }
    if !java_path.is_file() {
        return Err(AppError::new("java_missing"));
    }
    let java = inspect_java(java_path, "launch")?;
    if java.major != runtime.java_major {
        return Err(AppError::new("java_version_mismatch")
            .param("actual", java.major.to_string())
            .param("required", runtime.java_major.to_string()));
    }
    let version_id = profile_id(runtime);
    let version = resolve_version(paths, &version_id)?;
    validate_launch_inputs(paths, &version)?;
    let arguments = version
        .arguments
        .as_ref()
        .ok_or_else(|| AppError::new("launch_arguments_missing"))?;
    let index = version
        .asset_index
        .as_ref()
        .ok_or_else(|| AppError::new("asset_index_missing"))?;
    let mut classpath = Vec::new();
    for library in version
        .libraries
        .iter()
        .filter(|library| library_is_allowed(library))
    {
        if let Some(path) = library_destination(library, paths) {
            if path.is_file() {
                classpath.push(path.to_string_lossy().to_string());
            }
        }
    }
    let client = paths.client_jar(&version.jar_version);
    if !client.is_file() {
        return Err(AppError::new("installation_incomplete"));
    }
    classpath.push(client.to_string_lossy().to_string());
    fs::create_dir_all(&paths.natives)?;
    let mut replacements = HashMap::new();
    for (key, value) in [
        ("auth_player_name", settings.username.clone()),
        ("auth_uuid", crate::util::offline_uuid(&settings.username)),
        ("auth_access_token", "0".to_string()),
        ("auth_session", "0".to_string()),
        ("user_type", "legacy".to_string()),
        ("user_properties", "{}".to_string()),
        ("clientid", String::new()),
        ("auth_xuid", String::new()),
        ("version_name", version.id.clone()),
        ("version_type", "release".to_string()),
        ("game_directory", paths.root.to_string_lossy().to_string()),
        ("assets_root", paths.assets.to_string_lossy().to_string()),
        ("assets_index_name", index.id.clone()),
        (
            "natives_directory",
            paths.natives.to_string_lossy().to_string(),
        ),
        (
            "library_directory",
            paths.libraries.to_string_lossy().to_string(),
        ),
        ("classpath", classpath.join(classpath_separator())),
        ("classpath_separator", classpath_separator().to_string()),
        ("launcher_name", "SkyAndBeyond".to_string()),
        ("launcher_version", launcher_version.to_string()),
        ("resolution_width", settings.width.to_string()),
        ("resolution_height", settings.height.to_string()),
        ("quickPlayPath", String::new()),
        ("quickPlaySingleplayer", String::new()),
        ("quickPlayMultiplayer", String::new()),
        ("quickPlayRealms", String::new()),
    ] {
        replacements.insert(key.to_string(), value);
    }
    let features = HashMap::from([
        ("has_custom_resolution".to_string(), true),
        ("is_demo_user".to_string(), false),
        ("has_quick_plays_support".to_string(), false),
        ("is_quick_play_singleplayer".to_string(), false),
        ("is_quick_play_multiplayer".to_string(), false),
        ("is_quick_play_realms".to_string(), false),
    ]);
    let mut jvm_arguments = vec![
        format!("-Xms{}M", settings.min_memory_mb),
        format!("-Xmx{}M", settings.max_memory_mb),
    ];
    if let Some(logging) = &version.logging {
        if let Some(path) = logging_config_path(paths, logging) {
            jvm_arguments.push(
                logging
                    .client
                    .argument
                    .replace("${path}", &path.to_string_lossy()),
            );
        }
    }
    let jvm_replacements = version_name_replacements(&replacements, &version.jar_version);
    jvm_arguments.extend(collect_arguments(
        &arguments.jvm,
        &features,
        &jvm_replacements,
        true,
    )?);
    let mut custom_jvm = split_args(&settings.jvm_args);
    jvm_arguments.append(&mut custom_jvm);
    let mut game_arguments = collect_arguments(&arguments.game, &features, &replacements, true)?;
    let mut custom_game = split_args(&settings.game_args);
    game_arguments.append(&mut custom_game);
    apply_fullscreen(&paths.root, settings.fullscreen)?;
    let mut command_arguments = jvm_arguments;
    command_arguments.push(version.main_class.clone());
    command_arguments.extend(game_arguments);
    let log_path = paths
        .logs
        .join(format!("game-{}.log", format_compact_utc(now_ms())));
    logger.info(
        "launch",
        format!(
            "Launch plan ready for {} with Java {} ({} MiB max memory)",
            version.id, java.version, settings.max_memory_mb
        ),
    );
    Ok(LaunchPlan {
        executable: java_path.to_string_lossy().to_string(),
        arguments: command_arguments,
        working_directory: paths.root.to_string_lossy().to_string(),
        log_path: log_path.to_string_lossy().to_string(),
        version_id: version.id,
    })
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

pub fn spawn_game(plan: &LaunchPlan) -> AppResult<Child> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&plan.log_path)?;
    let error_log = log.try_clone()?;
    let mut command = Command::new(&plan.executable);
    command
        .args(&plan.arguments)
        .current_dir(&plan.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log));
    hide_console(&mut command);
    command
        .spawn()
        .map_err(|error| AppError::with_detail("game_start_failed", error.to_string()))
}

#[allow(dead_code)]
pub fn plan_for_diagnostics(plan: &LaunchPlan) -> String {
    format!("{} {} argument(s)", plan.executable, plan.arguments.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_known_placeholders() {
        let mut map = HashMap::new();
        map.insert("auth_player_name".to_string(), "Steve".to_string());
        assert_eq!(
            substitute("--username ${auth_player_name}", &map),
            "--username Steve"
        );
    }

    #[test]
    fn jvm_version_name_uses_game_jar_version() {
        let mut replacements = HashMap::new();
        replacements.insert("version_name".to_string(), "neoforge-21.1.248".to_string());
        let jvm = version_name_replacements(&replacements, "1.21.1");
        assert_eq!(jvm.get("version_name").map(String::as_str), Some("1.21.1"));
        assert_eq!(
            replacements.get("version_name").map(String::as_str),
            Some("neoforge-21.1.248")
        );
        let argument = Argument::Plain("-DignoreList=client-extra,${version_name}.jar".to_string());
        let result = collect_arguments(&[argument], &HashMap::new(), &jvm, true).unwrap();
        assert_eq!(result, vec!["-DignoreList=client-extra,1.21.1.jar"]);
    }

    #[test]
    fn collects_conditional_arguments() {
        let argument: Argument = serde_json::from_str(
            r#"{"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}"]}"#,
        )
        .unwrap();
        let features = HashMap::from([("has_custom_resolution".to_string(), true)]);
        let replacements = HashMap::from([("resolution_width".to_string(), "1280".to_string())]);
        let result = collect_arguments(&[argument], &features, &replacements, true).unwrap();
        assert_eq!(result, vec!["--width", "1280"]);
    }

    #[test]
    fn missing_assets_make_plan_fail_safely() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        let settings = Settings {
            username: "Steve".to_string(),
            ..Default::default()
        };
        let result = build_launch_plan(
            &runtime,
            &paths,
            &settings,
            Path::new("/definitely/missing/java"),
            "0.1.0",
            &crate::logging::Logger::new(dir.path().join("logs")).unwrap(),
        );
        assert!(result.is_err());
        assert!(!result.unwrap_err().detail_text().contains("127.0.0.1"));
    }
}
