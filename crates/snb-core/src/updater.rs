use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{LocalizedText, RuntimeConfig};
use crate::error::{AppError, AppResult};
use crate::http::{Downloader, ExpectedFile};
use crate::logging::Logger;
use crate::util::{sanitize_file_name, sha256_file, validate_url};

pub const UPDATE_SCHEMA_VERSION: u32 = 1;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateManifest {
    pub schema_version: u32,
    pub version: String,
    pub minimum_version: String,
    #[serde(default = "default_true")]
    pub mandatory: bool,
    pub released_at: String,
    pub notes: LocalizedText,
    pub platforms: BTreeMap<String, UpdateArtifact>,
}

impl Default for UpdateManifest {
    fn default() -> Self {
        Self {
            schema_version: UPDATE_SCHEMA_VERSION,
            version: "0.0.0".to_string(),
            minimum_version: "0.0.0".to_string(),
            mandatory: true,
            released_at: String::new(),
            notes: LocalizedText::default(),
            platforms: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateArtifact {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Base64 Ed25519 signature over the exact installer bytes. Required if a
    /// public key is configured at build time.
    pub signature: Option<String>,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDecision {
    pub current_version: String,
    pub latest_version: String,
    pub mandatory: bool,
    pub minimum_version: String,
    pub notes: LocalizedText,
    pub artifact: UpdateArtifact,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// True when a new mandatory launcher release must be installed.
    pub required: bool,
    pub decision: Option<UpdateDecision>,
    /// A blocking error only when an update could not be checked/accepted.
    pub error: Option<String>,
}

pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "x86",
        other => other,
    };
    format!("{os}-{arch}")
}

fn parse_version(value: &str, label: &str) -> AppResult<semver::Version> {
    semver::Version::parse(value.trim()).map_err(|error| {
        AppError::with_detail("update_manifest_invalid", format!("{label}: {error}"))
    })
}

fn validate_manifest(manifest: &UpdateManifest) -> AppResult<()> {
    if manifest.schema_version != UPDATE_SCHEMA_VERSION {
        return Err(AppError::new("update_schema_unsupported"));
    }
    parse_version(&manifest.version, "version")?;
    parse_version(&manifest.minimum_version, "minimumVersion")?;
    if manifest.platforms.is_empty() {
        return Err(AppError::new("update_manifest_invalid"));
    }
    for (platform, artifact) in &manifest.platforms {
        if platform.is_empty()
            || artifact.sha256.len() != 64
            || !artifact.sha256.chars().all(|c| c.is_ascii_hexdigit())
            || artifact.size == 0
            || sanitize_file_name(&artifact.file_name).is_err()
        {
            return Err(AppError::new("update_manifest_invalid"));
        }
        validate_url(&artifact.url, false)?;
        if let Some(signature) = &artifact.signature {
            let decoded = base64::engine::general_purpose::STANDARD.decode(signature);
            if decoded.map(|bytes| bytes.len() != 64).unwrap_or(true) {
                return Err(AppError::new("update_manifest_invalid"));
            }
        }
    }
    Ok(())
}

pub fn parse_manifest(text: &str) -> AppResult<UpdateManifest> {
    let manifest: UpdateManifest = serde_json::from_str(text)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Fetching the manifest is never cached: a cached manifest could silently
/// authorize an obsolete mandatory state or bypass a rejected release.
pub fn fetch_manifest(runtime: &RuntimeConfig) -> AppResult<UpdateManifest> {
    let downloader = Downloader::new()?;
    let text = downloader.fetch_text(
        &runtime.update_manifest_url(),
        1024 * 1024,
        Duration::from_secs(20),
    )?;
    parse_manifest(&text)
}

pub fn decide(
    current_version: &str,
    manifest: &UpdateManifest,
    platform: &str,
) -> AppResult<Option<UpdateDecision>> {
    let current = parse_version(current_version, "current version")?;
    let latest = parse_version(&manifest.version, "latest version")?;
    let minimum = parse_version(&manifest.minimum_version, "minimumVersion")?;
    let required = latest > current || current < minimum;
    if !required {
        return Ok(None);
    }
    let artifact = manifest
        .platforms
        .get(platform)
        .cloned()
        .ok_or_else(|| {
            AppError::with_detail(
                "update_unavailable",
                format!("no update artifact published for {platform}"),
            )
        })?;
    Ok(Some(UpdateDecision {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        mandatory: manifest.mandatory || current < minimum,
        minimum_version: minimum.to_string(),
        notes: manifest.notes.clone(),
        artifact,
        platform: platform.to_string(),
    }))
}

/// Checks the mandatory update state. In debug builds the network check is
/// bypassed so local development against the placeholder repository remains
/// possible; release builds always require a valid manifest.
pub fn check(current_version: &str, runtime: &RuntimeConfig) -> UpdateCheck {
    if cfg!(debug_assertions) && std::env::var("SNB_FORCE_UPDATE_CHECK").is_err() {
        return UpdateCheck {
            required: false,
            decision: None,
            error: None,
        };
    }
    match fetch_manifest(runtime)
        .and_then(|manifest| decide(current_version, &manifest, &platform_key()))
    {
        Ok(Some(decision)) if decision.mandatory => UpdateCheck {
            required: true,
            decision: Some(decision),
            error: None,
        },
        Ok(Some(decision)) => UpdateCheck {
            required: false,
            decision: Some(decision),
            error: None,
        },
        Ok(None) => UpdateCheck {
            required: false,
            decision: None,
            error: None,
        },
        Err(error) => UpdateCheck {
            required: true,
            decision: None,
            error: Some(error.user.code),
        },
    }
}

pub fn verify_signature(
    bytes: &[u8],
    public_key_base64: &str,
    signature_base64: Option<&str>,
) -> AppResult<()> {
    if public_key_base64.trim().is_empty() {
        return Ok(());
    }
    let signature = signature_base64.ok_or_else(|| AppError::new("update_signature_missing"))?;
    let signature = Signature::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(signature.trim())
            .map_err(|_| AppError::new("update_signature_invalid"))?,
    )
    .map_err(|_| AppError::new("update_signature_invalid"))?;
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_base64.trim())
        .map_err(|_| AppError::new("update_signature_invalid"))?;
    let key = VerifyingKey::from_bytes(
        key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| AppError::new("update_signature_invalid"))?,
    )
    .map_err(|_| AppError::new("update_signature_invalid"))?;
    key.verify(bytes, &signature)
        .map_err(|_| AppError::new("update_signature_invalid"))
}

/// Downloads and verifies the installer, leaving it in a staging directory. The
/// active installation is untouched until the external helper takes over.
pub fn download_update(
    decision: &UpdateDecision,
    updates_dir: &Path,
    public_key_base64: &str,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<PathBuf> {
    fs::create_dir_all(updates_dir)?;
    let file_name = sanitize_file_name(&decision.artifact.file_name)?;
    if !(file_name.to_ascii_lowercase().ends_with(".exe")
        || file_name.to_ascii_lowercase().ends_with(".msi")
        || file_name.to_ascii_lowercase().ends_with(".deb")
        || file_name.to_ascii_lowercase().ends_with(".app.tar.gz"))
    {
        return Err(AppError::new("update_manifest_invalid"));
    }
    let staged = updates_dir.join(format!("{}.verified", file_name));
    let download_path = updates_dir.join(&file_name);
    let downloader = Downloader::new()?;
    downloader.download(
        &decision.artifact.url,
        &download_path,
        &ExpectedFile::sha256(decision.artifact.sha256.clone(), Some(decision.artifact.size)),
        |current, total| progress(current, total),
        cancel,
    )?;
    let bytes = fs::read(&download_path)?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !digest.eq_ignore_ascii_case(&decision.artifact.sha256) {
        let _ = fs::remove_file(&download_path);
        return Err(AppError::new("update_checksum_invalid"));
    }
    verify_signature(
        &bytes,
        public_key_base64,
        decision.artifact.signature.as_deref(),
    )?;
    fs::write(&staged, &bytes)?;
    let _ = fs::remove_file(&download_path);
    logger.info(
        "update",
        format!(
            "Update {} downloaded and verified ({} bytes)",
            decision.latest_version,
            bytes.len()
        ),
    );
    Ok(staged)
}

pub fn backup_root(data_dir: &Path) -> PathBuf {
    data_dir.join("update-backups")
}

pub fn health_marker(data_dir: &Path) -> PathBuf {
    data_dir.join("update-health.json")
}

pub fn write_health_marker(data_dir: &Path, run_id: &str, version: &str) -> AppResult<()> {
    let value = serde_json::json!({
        "runId": run_id,
        "version": version,
        "atMs": crate::util::now_ms(),
    });
    crate::util::write_json_atomic(&health_marker(data_dir), &value)
}

#[allow(dead_code)]
pub fn updater_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("updater.log")
}

#[allow(dead_code)]
pub fn artifact_hash(path: &Path) -> AppResult<String> {
    sha256_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn manifest_json(version: &str) -> String {
        serde_json::json!({
            "schemaVersion": 1,
            "version": version,
            "minimumVersion": "0.1.0",
            "mandatory": true,
            "releasedAt": "2026-11-07T00:00:00Z",
            "notes": {"en": "Big update", "fr": "Grande mise à jour"},
            "platforms": {
                "windows-x86_64": {
                    "url": "https://github.com/example/releases/download/v0.2.0/setup.exe",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "size": 1234,
                    "signature": null,
                    "fileName": "setup.exe"
                }
            }
        })
        .to_string()
    }

    #[test]
    fn manifest_is_validated_and_update_is_mandatory() {
        let manifest = parse_manifest(&manifest_json("0.2.0")).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        let decision = decide("0.1.0", &manifest, "windows-x86_64").unwrap().unwrap();
        assert!(decision.mandatory);
        assert_eq!(decision.latest_version, "0.2.0");
        assert!(decide("0.2.0", &manifest, "windows-x86_64").unwrap().is_none());
        assert!(parse_manifest("{bad").is_err());
        assert!(parse_manifest(&manifest_json("not-semver")).is_err());
    }

    #[test]
    fn old_versions_below_minimum_are_always_blocked() {
        let mut manifest = parse_manifest(&manifest_json("0.2.0")).unwrap();
        manifest.mandatory = false;
        let decision = decide("0.0.9", &manifest, "windows-x86_64").unwrap().unwrap();
        assert!(decision.mandatory);
    }

    #[test]
    fn missing_platform_is_an_explicit_error() {
        let manifest = parse_manifest(&manifest_json("0.2.0")).unwrap();
        assert!(decide("0.1.0", &manifest, "linux-x86_64")
            .unwrap_err()
            .code()
            .eq("update_unavailable"));
    }

    #[test]
    fn ed25519_signatures_are_verified() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let public = base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().as_bytes());
        let signature = base64::engine::general_purpose::STANDARD.encode(signing.sign(b"installer bytes").to_bytes());
        assert!(verify_signature(b"installer bytes", &public, Some(&signature)).is_ok());
        assert!(verify_signature(b"tampered", &public, Some(&signature)).is_err());
        assert!(verify_signature(b"installer bytes", &public, None).is_err());
        assert!(verify_signature(b"installer bytes", "", None).is_ok());
    }

    #[test]
    fn manifest_rejects_http_and_bad_platform_entries() {
        let mut value: serde_json::Value = serde_json::from_str(&manifest_json("0.2.0")).unwrap();
        value["platforms"]["windows-x86_64"]["url"] = serde_json::json!("http://evil.example/setup.exe");
        assert!(parse_manifest(&value.to_string()).is_err());
        let mut value: serde_json::Value = serde_json::from_str(&manifest_json("0.2.0")).unwrap();
        value["platforms"]["windows-x86_64"]["fileName"] = serde_json::json!("../setup.exe");
        assert!(parse_manifest(&value.to_string()).is_err());
    }

    #[test]
    fn repository_manifest_examples_parse() {
        parse_manifest(include_str!(
            "../../../repository/launcher/update-manifest.json"
        ))
        .unwrap();
        parse_manifest(include_str!(
            "../../../repository/launcher/update-manifest-beta.json"
        ))
        .unwrap();
    }

    #[test]
    fn health_marker_records_run_id() {
        let dir = tempfile::tempdir().unwrap();
        write_health_marker(dir.path(), "run-123", "0.2.0").unwrap();
        let text = fs::read_to_string(health_marker(dir.path())).unwrap();
        assert!(text.contains("run-123"));
        assert!(text.contains("0.2.0"));
    }
}
