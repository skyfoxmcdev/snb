use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::RuntimeConfig;
use crate::error::{AppError, AppResult};
use crate::http::{Downloader, ExpectedFile};
use crate::instance::GamePaths;
use crate::java::run_command_capture;
use crate::logging::Logger;
use crate::util::{
    backup_path, human_file_name_from_url, now_ms, read_json, safe_join, sanitize_file_name,
    sha1_bytes, split_args, validate_url, write_bytes_atomic, write_json_atomic,
};

pub const ASSETS_MARKER: &str = "assets-complete.json";
pub const ASSET_INDEX_SCHEMA_VERSION: u32 = 1;
const VERSION_MANIFEST_MAX_BYTES: usize = 20 * 1024 * 1024;
const VERSION_JSON_MAX_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSummary {
    pub id: String,
    pub url: String,
    pub sha1: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub release_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct VersionJson {
    pub id: String,
    pub inherits_from: Option<String>,
    pub main_class: String,
    pub assets: Option<String>,
    pub asset_index: Option<AssetIndexRef>,
    pub downloads: Option<VersionDownloads>,
    pub libraries: Vec<Library>,
    pub arguments: Option<Arguments>,
    pub minecraft_arguments: Option<String>,
    pub logging: Option<LoggingConfig>,
    pub jar: Option<String>,
    #[serde(rename = "type")]
    pub version_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct VersionDownloads {
    pub client: Option<DownloadInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadInfo {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub total_size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Library {
    pub name: String,
    pub downloads: Option<LibraryDownloads>,
    pub natives: Option<HashMap<String, String>>,
    pub rules: Vec<Rule>,
    pub extract: Option<ExtractRules>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryDownloads {
    pub artifact: Option<Artifact>,
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Artifact {
    pub path: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub url: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtractRules {
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Arguments {
    pub game: Vec<Argument>,
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Argument {
    Plain(String),
    Conditional { rules: Vec<Rule>, value: ArgumentValue },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}

impl<'de> Deserialize<'de> for Argument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawArgument {
            Plain(String),
            Conditional { rules: Vec<Rule>, value: ArgumentValue },
        }
        match RawArgument::deserialize(deserializer)? {
            RawArgument::Plain(value) => Ok(Argument::Plain(value)),
            RawArgument::Conditional { rules, value } => {
                Ok(Argument::Conditional { rules, value })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Rule {
    pub action: String,
    pub os: Option<OsRule>,
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LoggingConfig {
    pub client: LoggingClient,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LoggingClient {
    pub argument: String,
    pub file: Artifact,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AssetIndexFile {
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedVersion {
    pub id: String,
    pub main_class: String,
    pub version_type: Option<String>,
    pub asset_index: Option<AssetIndexRef>,
    pub libraries: Vec<Library>,
    pub arguments: Option<Arguments>,
    pub logging: Option<LoggingConfig>,
    pub client: Option<DownloadInfo>,
    pub jar_version: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetProgress {
    pub bytes: u64,
    pub total_bytes: u64,
    pub files: u64,
    pub total_files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AssetsMarker {
    schema_version: u32,
    index_id: String,
    index_sha1: String,
    object_count: u64,
    total_size: u64,
    completed_at_ms: u64,
}

struct AssetJob {
    url: String,
    destination: PathBuf,
    sha1: String,
    size: u64,
}

pub fn current_os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        "linux" => "linux",
        other => other,
    }
}

pub fn current_os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "x86" => "x86",
        "aarch64" => "arm64",
        other => other,
    }
}

fn rule_matches(rule: &Rule, features: &HashMap<String, bool>) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != current_os_name() {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if arch != current_os_arch() {
                return false;
            }
        }
        // The launcher targets current Windows 10/11 and does not attempt to
        // emulate historical OS-version-gated arguments.
        if os.version.is_some() {
            return false;
        }
    }
    if let Some(required) = &rule.features {
        for (feature, expected) in required {
            if features.get(feature).copied().unwrap_or(false) != *expected {
                return false;
            }
        }
    }
    true
}

/// Mojang rules default to disallowed when at least one rule is present. Rules
/// are evaluated in order; the last matching rule wins.
pub fn rules_allow(rules: &[Rule], features: &HashMap<String, bool>) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, features) {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

pub fn load_version_json(path: &Path) -> AppResult<VersionJson> {
    let version: VersionJson = read_json(path)?;
    if version.id.is_empty() {
        return Err(AppError::new("version_json_invalid"));
    }
    Ok(version)
}

pub fn find_version<'a>(manifest: &'a VersionManifest, id: &str) -> AppResult<&'a VersionSummary> {
    manifest
        .versions
        .iter()
        .find(|version| version.id == id)
        .ok_or_else(|| AppError::new("version_not_found").param("version", id))
}

fn validate_main_class(version: &VersionJson) -> AppResult<()> {
    if version.main_class.trim().is_empty() {
        return Err(AppError::with_detail(
            "version_json_invalid",
            "mainClass is missing",
        ));
    }
    Ok(())
}

/// Downloads and verifies the pinned vanilla version metadata. A valid cached
/// JSON is reused immediately; `force_refresh` is used by repair.
pub fn ensure_version_metadata(
    runtime: &RuntimeConfig,
    paths: &GamePaths,
    force_refresh: bool,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
) -> AppResult<VersionJson> {
    let target = paths.version_json(&runtime.minecraft.version);
    if !force_refresh {
        if let Ok(version) = load_version_json(&target) {
            if version.id == runtime.minecraft.version {
                validate_main_class(&version)?;
                return Ok(version);
            }
        }
    }
    let downloader = Downloader::new()?;
    logger.info("minecraft", "Retrieving the official Mojang version manifest");
    let manifest_bytes = downloader.fetch_bytes(
        &runtime.minecraft.manifest_url,
        VERSION_MANIFEST_MAX_BYTES,
        Duration::from_secs(30),
    )?;
    let manifest: VersionManifest = serde_json::from_slice(&manifest_bytes)?;
    let summary = find_version(&manifest, &runtime.minecraft.version)?.clone();
    validate_url(&summary.url, false)?;
    let bytes = downloader.fetch_bytes(
        &summary.url,
        VERSION_JSON_MAX_BYTES,
        Duration::from_secs(45),
    )?;
    if !summary.sha1.is_empty() && !sha1_bytes(&bytes).eq_ignore_ascii_case(&summary.sha1) {
        return Err(AppError::new("metadata_checksum_mismatch"));
    }
    let version: VersionJson = serde_json::from_slice(&bytes)?;
    if version.id != runtime.minecraft.version {
        return Err(AppError::new("version_json_invalid"));
    }
    validate_main_class(&version)?;
    if target.exists() {
        let _ = fs::copy(&target, backup_path(&target));
    }
    write_bytes_atomic(&target, &bytes)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::new("cancelled"));
    }
    logger.info(
        "minecraft",
        format!(
            "Version metadata ready for Minecraft {} ({})",
            version.id,
            summary.release_time.as_deref().unwrap_or("unknown release date")
        ),
    );
    Ok(version)
}

pub fn ensure_client_jar(
    version: &VersionJson,
    paths: &GamePaths,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<PathBuf> {
    let client = version
        .downloads
        .as_ref()
        .and_then(|downloads| downloads.client.as_ref())
        .ok_or_else(|| AppError::new("client_metadata_missing"))?;
    validate_url(&client.url, false)?;
    let destination = paths.client_jar(&version.id);
    let downloader = Downloader::new()?;
    downloader.download(
        &client.url,
        &destination,
        &ExpectedFile::sha1(&client.sha1, Some(client.size)),
        |current, total| progress(current, total),
        cancel,
    )?;
    logger.info(
        "minecraft",
        format!("Client JAR verified for Minecraft {}", version.id),
    );
    Ok(destination)
}

pub fn maven_path(name: &str) -> Option<String> {
    let coordinate = name.split('@').next().unwrap_or(name);
    let parts: Vec<&str> = coordinate.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let group = parts[0];
    let artifact = parts[1];
    let version = parts[2];
    let classifier = parts.get(3).copied();
    if group.is_empty()
        || artifact.is_empty()
        || version.is_empty()
        || parts.iter().any(|part| part.split('/').any(|segment| segment == ".."))
    {
        return None;
    }
    let group_path = group.replace('.', "/");
    let file = match classifier {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    Some(format!("{group_path}/{artifact}/{version}/{file}"))
}

pub fn artifact_relative_path(library: &Library) -> Option<String> {
    library
        .downloads
        .as_ref()
        .and_then(|downloads| downloads.artifact.as_ref())
        .and_then(|artifact| artifact.path.clone())
        .or_else(|| maven_path(&library.name))
}

pub fn library_destination(library: &Library, paths: &GamePaths) -> Option<PathBuf> {
    let relative = artifact_relative_path(library)?;
    safe_join(&paths.libraries, Path::new(&relative)).ok()
}

fn artifact_url(library: &Library, artifact: &Artifact) -> Option<String> {
    if let Some(url) = &artifact.url {
        if !url.trim().is_empty() {
            return Some(url.clone());
        }
    }
    let path = artifact_relative_path(library)?;
    if let Some(base) = &library.url {
        Some(format!("{}/{}", base.trim_end_matches('/'), path))
    } else {
        Some(format!("https://libraries.minecraft.net/{path}"))
    }
}

pub fn library_is_allowed(library: &Library) -> bool {
    rules_allow(&library.rules, &HashMap::new())
}

fn expected_for_artifact(artifact: &Artifact) -> ExpectedFile {
    ExpectedFile {
        sha1: artifact.sha1.clone(),
        sha256: None,
        size: artifact.size,
    }
}

struct LibraryJob {
    name: String,
    url: String,
    destination: PathBuf,
    expected: ExpectedFile,
}

fn build_library_jobs(version: &VersionJson, paths: &GamePaths) -> AppResult<Vec<LibraryJob>> {
    let mut jobs = Vec::new();
    let mut seen = HashSet::new();
    for library in version.libraries.iter().filter(|library| library_is_allowed(library)) {
        let Some(artifact) = library
            .downloads
            .as_ref()
            .and_then(|downloads| downloads.artifact.as_ref())
        else {
            continue;
        };
        let Some(destination) = library_destination(library, paths) else {
            continue;
        };
        let key = destination.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let Some(url) = artifact_url(library, artifact) else {
            continue;
        };
        validate_url(&url, false)?;
        jobs.push(LibraryJob {
            name: library.name.clone(),
            url,
            destination,
            expected: expected_for_artifact(artifact),
        });
    }
    Ok(jobs)
}

/// Installs every allowed library artifact for the vanilla version. Downloads
/// reuse verification and resumable behavior from `Downloader`.
pub fn install_libraries(
    version: &VersionJson,
    paths: &GamePaths,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(u64, u64),
) -> AppResult<usize> {
    let jobs = build_library_jobs(version, paths)?;
    let total = jobs.len() as u64;
    let downloader = Downloader::new()?;
    for (index, job) in jobs.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("cancelled"));
        }
        downloader
            .download(
                &job.url,
                &job.destination,
                &job.expected,
                |_, _| {},
                cancel,
            )
            .map_err(|error| {
                AppError::with_detail(
                    "library_install_failed",
                    format!("{}: {}", job.name, error.detail_text()),
                )
            })?;
        progress(index as u64 + 1, total);
    }
    logger.info(
        "minecraft",
        format!("{total} library artifact(s) verified"),
    );
    Ok(jobs.len())
}

pub fn asset_index_path(paths: &GamePaths, index: &AssetIndexRef) -> PathBuf {
    paths.asset_indexes.join(format!("{}.json", index.id))
}

pub fn logging_config_path(paths: &GamePaths, logging: &LoggingConfig) -> Option<PathBuf> {
    let name = logging
        .client
        .file
        .url
        .as_deref()
        .and_then(human_file_name_from_url)
        .or_else(|| logging.client.file.path.clone())
        .and_then(|name| sanitize_file_name(&name).ok())
        .unwrap_or_else(|| "client-logging.xml".to_string());
    Some(paths.assets.join("log_configs").join(name))
}

fn load_asset_index(paths: &GamePaths, index: &AssetIndexRef) -> AppResult<AssetIndexFile> {
    let path = asset_index_path(paths, index);
    let file: AssetIndexFile = read_json(&path)?;
    if file.objects.is_empty() {
        return Err(AppError::new("asset_index_invalid"));
    }
    Ok(file)
}

fn build_asset_jobs(paths: &GamePaths, file: &AssetIndexFile) -> AppResult<Vec<AssetJob>> {
    let mut jobs = Vec::with_capacity(file.objects.len());
    for object in file.objects.values() {
        let hash = object.hash.to_ascii_lowercase();
        if hash.len() != 40 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AppError::new("asset_index_invalid"));
        }
        let relative = format!("{}/{}", &hash[..2], hash);
        let destination = safe_join(&paths.asset_objects, Path::new(&relative))?;
        jobs.push(AssetJob {
            url: format!("https://resources.download.minecraft.net/{relative}"),
            destination,
            sha1: hash,
            size: object.size,
        });
    }
    Ok(jobs)
}

fn count_asset_files(paths: &GamePaths) -> u64 {
    walkdir::WalkDir::new(&paths.asset_objects)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .count() as u64
}

fn marker_path(paths: &GamePaths) -> PathBuf {
    paths.managed.join(ASSETS_MARKER)
}

pub fn assets_ready(paths: &GamePaths, index: &AssetIndexRef) -> bool {
    let Some((marker, _)) = crate::util::read_json_with_backup::<AssetsMarker>(&marker_path(paths))
    else {
        return false;
    };
    marker.schema_version == ASSET_INDEX_SCHEMA_VERSION
        && marker.index_id == index.id
        && marker.index_sha1.eq_ignore_ascii_case(&index.sha1)
        && asset_index_path(paths, index).is_file()
        && count_asset_files(paths) >= marker.object_count
}

fn missing_asset_bytes(jobs: &[AssetJob]) -> u64 {
    jobs.iter()
        .filter(|job| !job.destination.is_file())
        .map(|job| job.size)
        .sum()
}

/// Installs the asset index, logging configuration and all asset objects.
/// Existing objects are checksum-verified. Progress is derived from actual
/// byte counts, never from synthetic estimates.
#[allow(clippy::too_many_arguments)]
pub fn ensure_assets(
    version: &VersionJson,
    paths: &GamePaths,
    force_verify: bool,
    cancel: &AtomicBool,
    logger: &Arc<Logger>,
    progress: &mut dyn FnMut(&AssetProgress),
) -> AppResult<(u64, u64)> {
    fs::create_dir_all(&paths.asset_objects)?;
    fs::create_dir_all(&paths.asset_indexes)?;
    fs::create_dir_all(&paths.assets.join("log_configs"))?;
    fs::create_dir_all(&paths.managed)?;
    let index = version
        .asset_index
        .as_ref()
        .ok_or_else(|| AppError::new("asset_index_missing"))?;
    validate_url(&index.url, false)?;
    let downloader = Downloader::new()?;
    let index_path = asset_index_path(paths, index);
    downloader.download(
        &index.url,
        &index_path,
        &ExpectedFile::sha1(&index.sha1, Some(index.size)),
        |_, _| {},
        cancel,
    )?;
    if let Some(logging) = &version.logging {
        if let Some(destination) = logging_config_path(paths, logging) {
            if let Some(url) = &logging.client.file.url {
                validate_url(url, false)?;
                downloader.download(
                    url,
                    &destination,
                    &expected_for_artifact(&logging.client.file),
                    |_, _| {},
                    cancel,
                )?;
            }
        }
    }
    let file = load_asset_index(paths, index)?;
    let jobs = build_asset_jobs(paths, &file)?;
    let total_files = jobs.len() as u64;
    let total_bytes: u64 = jobs.iter().map(|job| job.size).sum();
    if !force_verify && assets_ready(paths, index) {
        progress(&AssetProgress {
            bytes: total_bytes,
            total_bytes,
            files: total_files,
            total_files,
        });
        logger.info("assets", "Asset objects already verified");
        return Ok((total_files, total_bytes));
    }
    if total_bytes > 0 {
        let free = crate::util::available_space(&paths.asset_objects)?;
        let missing = missing_asset_bytes(&jobs);
        if missing.saturating_add(256 * 1024 * 1024) > free {
            return Err(AppError::with_detail(
                "disk_full",
                format!("assets need {missing} bytes, only {free} available"),
            ));
        }
    }
    let _ = fs::remove_file(marker_path(paths));
    logger.info(
        "assets",
        format!("Downloading or verifying {total_files} asset object(s)"),
    );
    let mut bytes_done: u64 = 0;
    let mut files_done: u64 = 0;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    progress(&AssetProgress {
        bytes: 0,
        total_bytes,
        files: 0,
        total_files,
    });
    for job in &jobs {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("cancelled"));
        }
        let before = bytes_done;
        downloader.download(
            &job.url,
            &job.destination,
            &ExpectedFile::sha1(&job.sha1, Some(job.size)),
            |current, _| {
                bytes_done = before.saturating_add(current);
                if last_emit.elapsed() >= Duration::from_millis(300) {
                    progress(&AssetProgress {
                        bytes: bytes_done.min(total_bytes),
                        total_bytes,
                        files: files_done,
                        total_files,
                    });
                    last_emit = Instant::now();
                }
            },
            cancel,
        )?;
        bytes_done = before.saturating_add(job.size);
        files_done += 1;
        if last_emit.elapsed() >= Duration::from_millis(300) || files_done == total_files {
            progress(&AssetProgress {
                bytes: bytes_done.min(total_bytes),
                total_bytes,
                files: files_done,
                total_files,
            });
            last_emit = Instant::now();
        }
    }
    let marker = AssetsMarker {
        schema_version: ASSET_INDEX_SCHEMA_VERSION,
        index_id: index.id.clone(),
        index_sha1: index.sha1.clone(),
        object_count: total_files,
        total_size: total_bytes,
        completed_at_ms: now_ms(),
    };
    write_json_atomic(&marker_path(paths), &marker)?;
    logger.info(
        "assets",
        format!("{total_files} asset object(s) verified ({total_bytes} bytes)"),
    );
    Ok((total_files, total_bytes))
}

pub fn resolve_version(paths: &GamePaths, id: &str) -> AppResult<ResolvedVersion> {
    let mut chain: Vec<VersionJson> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = id.to_string();
    for _ in 0..6 {
        if !seen.insert(current.clone()) {
            return Err(AppError::new("version_inheritance_cycle"));
        }
        let version = load_version_json(&paths.version_json(&current))?;
        let parent = version.inherits_from.clone();
        chain.push(version);
        match parent {
            Some(parent) if !parent.is_empty() => current = parent,
            _ => break,
        }
    }
    if chain.len() == 6 && chain.last().and_then(|v| v.inherits_from.clone()).is_some() {
        return Err(AppError::new("version_inheritance_too_deep"));
    }
    let child = chain
        .first()
        .ok_or_else(|| AppError::new("version_not_found"))?;
    let main_class = chain
        .iter()
        .find(|version| !version.main_class.trim().is_empty())
        .map(|version| version.main_class.clone())
        .ok_or_else(|| AppError::new("version_json_invalid"))?;
    let asset_index = chain
        .iter()
        .find_map(|version| version.asset_index.clone());
    let logging = chain.iter().find_map(|version| version.logging.clone());
    let client = chain
        .iter()
        .find_map(|version| version.downloads.as_ref().and_then(|d| d.client.clone()));
    let jar_version = chain
        .iter()
        .find_map(|version| version.jar.clone())
        .or_else(|| {
            chain
                .iter()
                .find(|version| {
                    version
                        .downloads
                        .as_ref()
                        .and_then(|downloads| downloads.client.as_ref())
                        .is_some()
                })
                .map(|version| version.id.clone())
        })
        .unwrap_or_else(|| child.id.clone());
    let mut libraries = Vec::new();
    let mut library_names = HashSet::new();
    for version in chain.iter().rev() {
        for library in &version.libraries {
            if library_names.insert(library.name.clone()) {
                libraries.push(library.clone());
            }
        }
    }
    let merged_arguments = {
        let mut merged = Arguments::default();
        let mut any = false;
        for version in chain.iter().rev() {
            if let Some(arguments) = &version.arguments {
                any = true;
                merged.game.extend(arguments.game.iter().cloned());
                merged.jvm.extend(arguments.jvm.iter().cloned());
            }
        }
        if !any {
            if let Some(legacy) = chain
                .iter()
                .rev()
                .find_map(|version| version.minecraft_arguments.clone())
            {
                merged.game = split_args(&legacy)
                    .into_iter()
                    .map(Argument::Plain)
                    .collect();
                any = true;
            }
        }
        any.then_some(merged)
    };
    Ok(ResolvedVersion {
        id: child.id.clone(),
        main_class,
        version_type: child.version_type.clone(),
        asset_index,
        libraries,
        arguments: merged_arguments,
        logging,
        client,
        jar_version,
    })
}

pub fn broken_installation(paths: &GamePaths, version: &ResolvedVersion) -> Vec<String> {
    let mut issues = Vec::new();
    if !paths.client_jar(&version.jar_version).is_file() {
        issues.push("client_jar".to_string());
    }
    if let Some(index) = &version.asset_index {
        if !asset_index_path(paths, index).is_file() {
            issues.push("asset_index".to_string());
        }
        if !assets_ready(paths, index) {
            issues.push("assets".to_string());
        }
    } else {
        issues.push("asset_index".to_string());
    }
    if version
        .logging
        .as_ref()
        .and_then(|logging| logging_config_path(paths, logging))
        .map(|path| !path.is_file())
        .unwrap_or(false)
    {
        issues.push("logging_config".to_string());
    }
    for library in version
        .libraries
        .iter()
        .filter(|library| library_is_allowed(library))
    {
        if let Some(path) = library_destination(library, paths) {
            if !path.is_file() {
                issues.push(format!("library:{}", library.name));
            }
        }
    }
    issues
}

/// Small utility used by diagnostics to report a Java/install-independent
/// classpath without starting the game.
pub fn classpath_entries(version: &ResolvedVersion, paths: &GamePaths) -> Vec<String> {
    let mut entries = Vec::new();
    for library in version
        .libraries
        .iter()
        .filter(|library| library_is_allowed(library))
    {
        if let Some(path) = library_destination(library, paths) {
            entries.push(path.to_string_lossy().to_string());
        }
    }
    entries
}

#[allow(dead_code)]
pub fn java_runtime_probe(java: &Path) -> AppResult<String> {
    let mut command = std::process::Command::new(java);
    command.arg("-version");
    let output = run_command_capture(command, Duration::from_secs(10), None)?;
    Ok(output.combined())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;

    fn runtime() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    fn paths(root: &Path) -> GamePaths {
        GamePaths::new(root, &runtime())
    }

    fn write_version(paths: &GamePaths, id: &str, json: &str) {
        let dir = paths.version_dir(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{id}.json")), json).unwrap();
    }

    #[test]
    fn rules_match_os_and_features() {
        let features = HashMap::new();
        let windows = vec![Rule {
            action: "allow".to_string(),
            os: Some(OsRule {
                name: Some("windows".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }];
        assert_eq!(rules_allow(&windows, &features), cfg!(windows));
        let allow_all_desktop = vec![
            Rule {
                action: "disallow".to_string(),
                os: Some(OsRule {
                    name: Some("osx".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Rule {
                action: "allow".to_string(),
                ..Default::default()
            },
        ];
        assert!(rules_allow(&allow_all_desktop, &features));
        let feature_rule = vec![Rule {
            action: "allow".to_string(),
            features: Some(HashMap::from([(
                "has_custom_resolution".to_string(),
                true,
            )])),
            ..Default::default()
        }];
        assert!(!rules_allow(&feature_rule, &features));
        let enabled = HashMap::from([("has_custom_resolution".to_string(), true)]);
        assert!(rules_allow(&feature_rule, &enabled));
    }

    #[test]
    fn maven_coordinate_paths_are_safe() {
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-linux").as_deref(),
            Some("org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar")
        );
        assert_eq!(
            maven_path("com.example:demo:1.0@jar").as_deref(),
            Some("com/example/demo/1.0/demo-1.0.jar")
        );
        assert!(maven_path("bad").is_none());
        assert!(maven_path("com.example:../../evil:1").is_none());
    }

    #[test]
    fn resolves_inheritance_and_merges_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        write_version(
            &paths,
            "1.21.1",
            r#"{
                "id":"1.21.1","mainClass":"net.minecraft.client.main.Main",
                "assetIndex":{"id":"17","sha1":"abc","size":10,"totalSize":100,"url":"https://example.com/assets.json"},
                "downloads":{"client":{"sha1":"abc","size":10,"url":"https://example.com/client.jar"}},
                "libraries":[
                    {"name":"org.example:base:1","downloads":{"artifact":{"path":"org/example/base/1/base-1.jar","sha1":"abc","size":1,"url":"https://example.com/base.jar"}}}
                ],
                "arguments":{"jvm":["-cp","${classpath}"],"game":["--username","${auth_player_name}"]},
                "type":"release"
            }"#,
        );
        write_version(
            &paths,
            "neoforge-21.1.248",
            r#"{
                "id":"neoforge-21.1.248","inheritsFrom":"1.21.1",
                "mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher",
                "libraries":[
                    {"name":"org.example:base:1","downloads":{"artifact":{"path":"org/example/base/1/base-1.jar","sha1":"abc","size":1,"url":"https://example.com/base.jar"}}},
                    {"name":"net.neoforged:forge:1","downloads":{"artifact":{"path":"net/neoforged/forge/1/forge-1.jar","sha1":"def","size":1,"url":"https://example.com/forge.jar"}}}
                ],
                "arguments":{"game":["--launchTarget","forgeclient"]}
            }"#,
        );
        let resolved = resolve_version(&paths, "neoforge-21.1.248").unwrap();
        assert_eq!(
            resolved.main_class,
            "cpw.mods.bootstraplauncher.BootstrapLauncher"
        );
        assert_eq!(resolved.jar_version, "1.21.1");
        assert_eq!(resolved.libraries.len(), 2);
        assert_eq!(resolved.asset_index.as_ref().unwrap().id, "17");
        let arguments = resolved.arguments.unwrap();
        let game: Vec<String> = arguments
            .game
            .iter()
            .map(|argument| match argument {
                Argument::Plain(value) => value.clone(),
                Argument::Conditional { value, .. } => match value {
                    ArgumentValue::One(value) => value.clone(),
                    ArgumentValue::Many(values) => values.join(" "),
                },
            })
            .collect();
        assert_eq!(&game[..2], &["--username", "${auth_player_name}"]);
        assert_eq!(&game[2..], &["--launchTarget", "forgeclient"]);
    }

    #[test]
    fn legacy_arguments_are_supported() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        write_version(
            &paths,
            "legacy",
            r#"{"id":"legacy","mainClass":"Main","minecraftArguments":"--username ${auth_player_name} --gameDir ${game_directory}"}"#,
        );
        let resolved = resolve_version(&paths, "legacy").unwrap();
        assert_eq!(resolved.arguments.unwrap().game.len(), 4);
    }

    #[test]
    fn assets_marker_detects_deleted_objects() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        fs::create_dir_all(&paths.managed).unwrap();
        fs::create_dir_all(&paths.asset_indexes).unwrap();
        fs::create_dir_all(paths.asset_objects.join("aa")).unwrap();
        fs::write(paths.asset_indexes.join("17.json"), b"{}").unwrap();
        fs::write(paths.asset_objects.join("aa/hash"), b"x").unwrap();
        let index = AssetIndexRef {
            id: "17".to_string(),
            sha1: "abc".to_string(),
            size: 2,
            total_size: 1,
            url: "https://example.com/index.json".to_string(),
        };
        let marker = AssetsMarker {
            schema_version: 1,
            index_id: "17".to_string(),
            index_sha1: "abc".to_string(),
            object_count: 1,
            total_size: 1,
            completed_at_ms: now_ms(),
        };
        fs::write(marker_path(&paths), serde_json::to_vec(&marker).unwrap()).unwrap();
        assert!(assets_ready(&paths, &index));
        fs::remove_file(paths.asset_objects.join("aa/hash")).unwrap();
        assert!(!assets_ready(&paths, &index));
    }

    #[test]
    fn library_destinations_stay_inside_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let library = Library {
            name: "org.example:demo:1".to_string(),
            downloads: Some(LibraryDownloads {
                artifact: Some(Artifact {
                    path: Some("../escape.jar".to_string()),
                    ..Default::default()
                }),
                classifiers: None,
            }),
            ..Default::default()
        };
        assert!(library_destination(&library, &paths).is_none());
    }

    #[test]
    fn allowed_libraries_include_plain_rules() {
        let library: Library = serde_json::from_str(
            r#"{"name":"a:b:1","downloads":{"artifact":{"url":"https://example.com/a.jar","sha1":"abc","size":1}}}"#,
        )
        .unwrap();
        assert!(library_is_allowed(&library));
        assert_eq!(artifact_relative_path(&library).as_deref(), Some("a/b/1/b-1.jar"));
    }
}
