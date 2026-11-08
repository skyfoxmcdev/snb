use std::sync::Arc;

use serde::Serialize;

use crate::automodpack::{detect, AutomodpackStatus};
use crate::config::{RuntimeConfig, ServerEndpoint};
use crate::instance::{GamePaths, INSTANCE_FILE};
use crate::java::{detect_java, JavaReport};
use crate::minecraft::{
    assets_ready, library_destination, library_is_allowed, load_version_json, resolve_version,
};
use crate::neoforge::profile_id;
use crate::settings::Settings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentCheck {
    pub id: String,
    /// "ok", "warning", "missing" or "error".
    pub status: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessReport {
    pub ready: bool,
    pub components: Vec<ComponentCheck>,
    pub issues: Vec<String>,
    pub java: JavaReport,
    pub automodpack: AutomodpackStatus,
    pub server_configured: bool,
}

fn component(
    id: &str,
    ok: bool,
    status_if_missing: &str,
    code: Option<&str>,
) -> ComponentCheck {
    ComponentCheck {
        id: id.to_string(),
        status: if ok { "ok" } else { status_if_missing }.to_string(),
        code: if ok { None } else { code.map(str::to_string) },
    }
}

/// Fast, side-effect-free installation verification used before Play and after
/// updates. It verifies existence and markers, while the repair workflow
/// performs full checksum validation.
pub fn check_readiness(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    settings: &Settings,
    endpoint: Option<&ServerEndpoint>,
) -> ReadinessReport {
    let managed = paths.root.join("java").join(format!("runtime-{}", runtime.java_major));
    let java = detect_java(&settings.java_path, &managed, runtime.java_major);
    let java_ok = java.status == "ok";
    let instance_ok = paths.root.join(INSTANCE_FILE).is_file();
    let vanilla = load_version_json(&paths.version_json(&runtime.minecraft.version));
    let vanilla_ok = vanilla.is_ok();
    let client_ok = vanilla
        .as_ref()
        .map(|version| paths.client_jar(&version.id).is_file())
        .unwrap_or(false);
    let neoforge_id = profile_id(runtime);
    let resolved = if instance_ok {
        resolve_version(paths, &neoforge_id).ok()
    } else {
        None
    };
    let neoforge_ok = resolved.is_some();
    let libraries_ok = resolved.as_ref().map(|version| {
        version
            .libraries
            .iter()
            .filter(|library| library_is_allowed(library))
            .filter_map(|library| library_destination(library, paths))
            .all(|path| path.is_file())
    }).unwrap_or(false);
    let assets_ok = vanilla
        .as_ref()
        .ok()
        .and_then(|version| version.asset_index.as_ref())
        .map(|index| assets_ready(paths, index))
        .unwrap_or(false);
    let automodpack = detect(paths, &runtime.automodpack.version);
    let automodpack_ok = automodpack.valid;
    let mut components = vec![
        component("instance", instance_ok, "missing", Some("instance_missing")),
        component("java", java_ok, "missing", Some("java_missing")),
        component("minecraft", vanilla_ok && client_ok, "missing", Some("minecraft_missing")),
        component("libraries", libraries_ok, "missing", Some("libraries_missing")),
        component("assets", assets_ok, "missing", Some("assets_missing")),
        component("neoforge", neoforge_ok, "missing", Some("neoforge_missing")),
        component("automodpack", automodpack_ok, "missing", automodpack.message_code.as_deref()),
    ];
    let mut issues = Vec::new();
    for check in &components {
        if check.status != "ok" {
            if let Some(code) = &check.code {
                issues.push(code.clone());
            }
        }
    }
    let server_configured = endpoint.is_some();
    components.push(component(
        "server",
        server_configured,
        "warning",
        Some("server_endpoint_missing"),
    ));
    let ready = instance_ok
        && java_ok
        && vanilla_ok
        && client_ok
        && libraries_ok
        && assets_ok
        && neoforge_ok
        && automodpack_ok;
    ReadinessReport {
        ready,
        components,
        issues,
        java,
        automodpack,
        server_configured,
    }
}

#[allow(dead_code)]
pub fn readiness_log_line(report: &ReadinessReport) -> String {
    format!(
        "ready={} issues={}",
        report.ready,
        if report.issues.is_empty() {
            "none".to_string()
        } else {
            report.issues.join(",")
        }
    )
}

#[allow(dead_code)]
pub fn readiness_logger(_logger: &Arc<crate::logging::Logger>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_directory_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = RuntimeConfig::default();
        let paths = GamePaths::new(dir.path(), &runtime);
        let settings = Settings::default();
        let report = check_readiness(&runtime, &paths, &settings, None);
        assert!(!report.ready);
        assert!(report.issues.contains(&"instance_missing".to_string()));
        assert!(!report.server_configured);
    }
}
