use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{
    Appearance, LauncherInfo, NewsFeed, ServerConfig, ServerEndpoint, CONFIG_SCHEMA_VERSION,
};
use crate::error::{AppError, AppResult};
use crate::http::Downloader;
use crate::logging::Logger;
use crate::util::{
    contains_ip_literal, now_ms, read_json, redact_text, validate_url, write_json_without_backup,
};

const REMOTE_TIMEOUT: Duration = Duration::from_secs(6);
const REMOTE_MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSource {
    /// "remote", "cache", or "default".
    pub kind: String,
    pub updated_at_ms: Option<u64>,
}

impl FileSource {
    fn new(kind: &str, updated_at_ms: Option<u64>) -> Self {
        Self {
            kind: kind.to_string(),
            updated_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSources {
    pub server: FileSource,
    pub launcher: FileSource,
    pub appearance: FileSource,
    pub news: FileSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWarning {
    pub file: String,
    pub code: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSnapshot {
    pub server: ServerConfig,
    pub launcher: LauncherInfo,
    pub appearance: Appearance,
    pub news: NewsFeed,
    pub online: bool,
    pub from_cache: bool,
    pub last_updated_ms: Option<u64>,
    pub warnings: Vec<RemoteWarning>,
    pub sources: RemoteSources,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointRefresh {
    pub endpoint: Option<ServerEndpoint>,
    pub from_cache: bool,
    pub warnings: Vec<RemoteWarning>,
}

trait RemoteConfig: DeserializeOwned + Serialize + Default + Send + 'static {
    const NAME: &'static str;
    const RELATIVE_PATH: &'static str;
    fn validate(self) -> AppResult<Self>;
}

impl RemoteConfig for ServerConfig {
    const NAME: &'static str = "server";
    const RELATIVE_PATH: &'static str = "config/server.json";
    fn validate(self) -> AppResult<Self> {
        ServerConfig::validate(self)
    }
}

impl RemoteConfig for LauncherInfo {
    const NAME: &'static str = "launcher";
    const RELATIVE_PATH: &'static str = "config/launcher.json";
    fn validate(self) -> AppResult<Self> {
        LauncherInfo::validate(self)
    }
}

impl RemoteConfig for Appearance {
    const NAME: &'static str = "appearance";
    const RELATIVE_PATH: &'static str = "config/appearance.json";
    fn validate(self) -> AppResult<Self> {
        Appearance::validate(self)
    }
}

impl RemoteConfig for NewsFeed {
    const NAME: &'static str = "news";
    const RELATIVE_PATH: &'static str = "config/news.json";
    fn validate(self) -> AppResult<Self> {
        NewsFeed::validate(self)
    }
}

struct Outcome<T> {
    value: T,
    source: FileSource,
    warning: Option<RemoteWarning>,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct CacheMeta {
    fetched_at_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CachedEndpoint {
    endpoint: ServerEndpoint,
    saved_at_ms: u64,
}

pub struct RemoteLoader {
    base_url: Url,
    cache_dir: PathBuf,
    downloader: Downloader,
    logger: Arc<Logger>,
}

impl RemoteLoader {
    pub fn new(
        base_url: &str,
        cache_dir: impl AsRef<Path>,
        allow_loopback_http: bool,
        logger: Arc<Logger>,
    ) -> AppResult<Self> {
        let mut url = validate_url(base_url, allow_loopback_http).map_err(|error| {
            AppError::with_detail("remote_base_invalid", error.detail_text().to_string())
        })?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err(AppError::new("remote_base_invalid"));
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        let cache_dir = cache_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self {
            base_url: url,
            cache_dir,
            downloader: Downloader::new()?,
            logger,
        })
    }

    fn url_for(&self, relative: &str) -> String {
        format!("{}{}", self.base_url, relative)
    }

    fn cache_path<T: RemoteConfig>(&self) -> PathBuf {
        self.cache_dir.join(format!("{}.json", T::NAME))
    }

    fn meta_path<T: RemoteConfig>(&self) -> PathBuf {
        self.cache_dir.join(format!("{}.meta.json", T::NAME))
    }

    fn fetch_config<T: RemoteConfig>(&self) -> Outcome<T> {
        let file = T::NAME.to_string();
        let url = self.url_for(T::RELATIVE_PATH);
        let mut failure_warning: Option<RemoteWarning>;

        match self
            .downloader
            .fetch_text(&url, REMOTE_MAX_BYTES, REMOTE_TIMEOUT)
        {
            Ok(text) => {
                if contains_ip_literal(&text) {
                    let warning = RemoteWarning {
                        file: file.clone(),
                        code: "config_ip_literal".to_string(),
                        detail: Some(
                            "The remote file contained an IP address literal and was rejected"
                                .to_string(),
                        ),
                    };
                    self.logger.warn(
                        "config",
                        format!("{} rejected: IP address literal present", T::NAME),
                    );
                    failure_warning = Some(warning);
                } else {
                    match serde_json::from_str::<T>(&text)
                        .map_err(AppError::from)
                        .and_then(T::validate)
                    {
                        Ok(value) => {
                            let now = now_ms();
                            if let Err(error) = write_json_without_backup(
                                &self.cache_path::<T>(),
                                &value,
                            ) {
                                self.logger.warn(
                                    "config",
                                    format!(
                                        "Could not cache {}: {}",
                                        T::NAME,
                                        redact_text(error.detail_text())
                                    ),
                                );
                            } else {
                                let _ = write_json_without_backup(
                                    &self.meta_path::<T>(),
                                    &CacheMeta {
                                        fetched_at_ms: now,
                                    },
                                );
                            }
                            self.logger
                                .info("config", format!("Loaded remote {}", T::NAME));
                            return Outcome {
                                value,
                                source: FileSource::new("remote", Some(now)),
                                warning: None,
                            };
                        }
                        Err(error) => {
                            self.logger.warn(
                                "config",
                                format!(
                                    "Remote {} is invalid ({}): {}",
                                    T::NAME,
                                    error.code(),
                                    redact_text(error.detail_text())
                                ),
                            );
                            failure_warning = Some(RemoteWarning {
                                file: file.clone(),
                                code: error.code().to_string(),
                                detail: error.user.detail,
                            });
                        }
                    }
                }
            }
            Err(error) => {
                self.logger.warn(
                    "config",
                    format!(
                        "Remote {} unavailable ({}): {}",
                        T::NAME,
                        error.code(),
                        redact_text(error.detail_text())
                    ),
                );
                failure_warning = Some(RemoteWarning {
                    file: file.clone(),
                    code: error.code().to_string(),
                    detail: error.user.detail,
                });
            }
        }

        let cache_path = self.cache_path::<T>();
        if let Ok(value) = read_json::<T>(&cache_path).and_then(T::validate) {
            let fetched_at = read_json::<CacheMeta>(&self.meta_path::<T>())
                .ok()
                .map(|meta| meta.fetched_at_ms);
            self.logger
                .info("config", format!("Using cached {}", T::NAME));
            return Outcome {
                value,
                source: FileSource::new("cache", fetched_at),
                warning: failure_warning,
            };
        }

        self.logger.warn(
            "config",
            format!("Using built-in defaults for {}", T::NAME),
        );
        Outcome {
            value: T::default(),
            source: FileSource::new("default", None),
            warning: failure_warning,
        }
    }

    pub fn load(&self) -> RemoteSnapshot {
        let (server_out, launcher_out, appearance_out, news_out) = std::thread::scope(|scope| {
            let server = scope.spawn(|| self.fetch_config::<ServerConfig>());
            let launcher = scope.spawn(|| self.fetch_config::<LauncherInfo>());
            let appearance = scope.spawn(|| self.fetch_config::<Appearance>());
            let news = scope.spawn(|| self.fetch_config::<NewsFeed>());
            (
                server.join().ok(),
                launcher.join().ok(),
                appearance.join().ok(),
                news.join().ok(),
            )
        });

        let server_out = server_out.unwrap_or_else(|| Outcome {
            value: ServerConfig::default(),
            source: FileSource::new("default", None),
            warning: Some(RemoteWarning {
                file: "server".to_string(),
                code: "config_loader_failed".to_string(),
                detail: None,
            }),
        });
        let launcher_out = launcher_out.unwrap_or_else(|| Outcome {
            value: LauncherInfo::default(),
            source: FileSource::new("default", None),
            warning: Some(RemoteWarning {
                file: "launcher".to_string(),
                code: "config_loader_failed".to_string(),
                detail: None,
            }),
        });
        let appearance_out = appearance_out.unwrap_or_else(|| Outcome {
            value: Appearance::default(),
            source: FileSource::new("default", None),
            warning: Some(RemoteWarning {
                file: "appearance".to_string(),
                code: "config_loader_failed".to_string(),
                detail: None,
            }),
        });
        let news_out = news_out.unwrap_or_else(|| Outcome {
            value: NewsFeed::default(),
            source: FileSource::new("default", None),
            warning: Some(RemoteWarning {
                file: "news".to_string(),
                code: "config_loader_failed".to_string(),
                detail: None,
            }),
        });

        let mut warnings = Vec::new();
        for warning in [
            &server_out.warning,
            &launcher_out.warning,
            &appearance_out.warning,
            &news_out.warning,
        ] {
            if let Some(warning) = warning {
                warnings.push(warning.clone());
            }
        }

        let server_from_remote = server_out.source.kind == "remote";
        let mut server = server_out.value;
        let last_valid = read_last_valid_endpoint(&self.cache_dir);
        match &server.endpoint {
            Some(endpoint) => {
                if server_from_remote {
                    self.save_last_valid_endpoint(endpoint);
                }
            }
            None => {
                if let Some(endpoint) = last_valid {
                    server.endpoint = Some(endpoint);
                    warnings.push(RemoteWarning {
                        file: "server".to_string(),
                        code: "server_endpoint_cached".to_string(),
                        detail: None,
                    });
                } else {
                    warnings.push(RemoteWarning {
                        file: "server".to_string(),
                        code: "server_endpoint_missing".to_string(),
                        detail: None,
                    });
                }
            }
        }

        let sources = RemoteSources {
            server: server_out.source,
            launcher: launcher_out.source,
            appearance: appearance_out.source,
            news: news_out.source,
        };
        let online = [
            &sources.server,
            &sources.launcher,
            &sources.appearance,
            &sources.news,
        ]
        .iter()
        .all(|source| source.kind == "remote");
        let from_cache = [
            &sources.server,
            &sources.launcher,
            &sources.appearance,
            &sources.news,
        ]
        .iter()
        .any(|source| source.kind == "cache");
        let last_updated_ms = [
            sources.server.updated_at_ms,
            sources.launcher.updated_at_ms,
            sources.appearance.updated_at_ms,
            sources.news.updated_at_ms,
        ]
        .into_iter()
        .flatten()
        .max();

        self.logger.info(
            "config",
            format!(
                "Configuration ready (online={online}, fromCache={from_cache}, warnings={})",
                warnings.len()
            ),
        );

        RemoteSnapshot {
            server,
            launcher: launcher_out.value,
            appearance: appearance_out.value,
            news: news_out.value,
            online,
            from_cache,
            last_updated_ms,
            warnings,
            sources,
        }
    }

    pub fn refresh_endpoint(&self) -> EndpointRefresh {
        let outcome = self.fetch_config::<ServerConfig>();
        let mut warnings = Vec::new();
        if let Some(warning) = outcome.warning {
            warnings.push(warning);
        }
        let from_cache = outcome.source.kind == "cache";
        let mut endpoint = outcome.value.endpoint;
        if let Some(found) = &endpoint {
            if outcome.source.kind == "remote" {
                self.save_last_valid_endpoint(found);
            }
        } else if let Some(cached) = read_last_valid_endpoint(&self.cache_dir) {
            endpoint = Some(cached);
            warnings.push(RemoteWarning {
                file: "server".to_string(),
                code: "server_endpoint_cached".to_string(),
                detail: None,
            });
        } else {
            warnings.push(RemoteWarning {
                file: "server".to_string(),
                code: "server_endpoint_missing".to_string(),
                detail: None,
            });
        }
        EndpointRefresh {
            endpoint,
            from_cache,
            warnings,
        }
    }

    fn save_last_valid_endpoint(&self, endpoint: &ServerEndpoint) {
        let path = self.cache_dir.join("last-valid-endpoint.json");
        let value = CachedEndpoint {
            endpoint: endpoint.clone(),
            saved_at_ms: now_ms(),
        };
        if let Err(error) = write_json_without_backup(&path, &value) {
            self.logger.warn(
                "config",
                format!(
                    "Could not cache the server endpoint: {}",
                    redact_text(error.detail_text())
                ),
            );
        }
    }

    pub fn cached_endpoint(cache_dir: impl AsRef<Path>) -> Option<ServerEndpoint> {
        read_last_valid_endpoint(cache_dir.as_ref())
    }
}

fn read_last_valid_endpoint(cache_dir: &Path) -> Option<ServerEndpoint> {
    let path = cache_dir.join("last-valid-endpoint.json");
    let cached: CachedEndpoint = read_json(&path).ok()?;
    ServerEndpoint::parse(&cached.endpoint.host, cached.endpoint.port).ok()
}

/// A flat, JSON-friendly server summary used by startup and diagnostics. It
/// deliberately never contains a resolved IP address.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSummary {
    pub schema_version: u32,
    pub online: bool,
    pub endpoint_host: Option<String>,
    pub endpoint_port: Option<u16>,
    pub sources: BTreeMap<String, String>,
}

impl From<&RemoteSnapshot> for RemoteSummary {
    fn from(snapshot: &RemoteSnapshot) -> Self {
        let mut sources = BTreeMap::new();
        sources.insert("server".to_string(), snapshot.sources.server.kind.clone());
        sources.insert("launcher".to_string(), snapshot.sources.launcher.kind.clone());
        sources.insert(
            "appearance".to_string(),
            snapshot.sources.appearance.kind.clone(),
        );
        sources.insert("news".to_string(), snapshot.sources.news.kind.clone());
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            online: snapshot.online,
            endpoint_host: snapshot.server.endpoint.as_ref().map(|e| e.host.clone()),
            endpoint_port: snapshot.server.endpoint.as_ref().map(|e| e.port),
            sources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestServer;

    fn logger(dir: &Path) -> Arc<Logger> {
        Logger::new(dir.join("logs")).unwrap()
    }

    fn launcher_json() -> String {
        r#"{
            "schemaVersion": 1,
            "latestVersion": "0.1.0",
            "minimumVersion": "0.1.0",
            "announcements": [],
            "buttons": [],
            "links": {},
            "maintenance": {"active": false}
        }"#
        .to_string()
    }

    fn appearance_json() -> String {
        r##"{
            "schemaVersion": 1,
            "colors": {"background": "#060912"},
            "lightColors": {},
            "backgroundOverlay": 0.4
        }"##
        .to_string()
    }

    fn news_json() -> String {
        r#"{"schemaVersion":1,"items":[]}"#.to_string()
    }

    fn server_json(host: &str, port: u16) -> String {
        format!(
            r#"{{
                "schemaVersion": 1,
                "endpoint": {{"host": "{host}", "port": {port}}},
                "displayName": "Sky & Beyond",
                "description": {{"en": "A server", "fr": "Un serveur"}}
            }}"#
        )
    }

    fn serve_all(server: &TestServer, server_config: String) {
        server.set("/config/server.json", server_config);
        server.set("/config/launcher.json", launcher_json());
        server.set("/config/appearance.json", appearance_json());
        server.set("/config/news.json", news_json());
    }

    #[test]
    fn loads_all_files_and_endpoint() {
        let http = TestServer::new();
        serve_all(&http, server_json("skyandbeyond.playit.gg", 25565));
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        let snapshot = loader.load();
        assert!(snapshot.online);
        assert!(!snapshot.from_cache);
        assert_eq!(
            snapshot.server.endpoint.as_ref().unwrap().address(),
            "skyandbeyond.playit.gg:25565"
        );
        assert!(snapshot.warnings.is_empty());
        assert_eq!(snapshot.sources.server.kind, "remote");
        assert!(snapshot.last_updated_ms.is_some());
    }

    #[test]
    fn endpoint_changes_are_picked_up() {
        let http = TestServer::new();
        serve_all(&http, server_json("skyandbeyond.playit.gg", 25565));
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        assert_eq!(
            loader.load().server.endpoint.unwrap().port,
            25565
        );
        serve_all(&http, server_json("new-endpoint.playit.gg", 25600));
        let snapshot = loader.load();
        let endpoint = snapshot.server.endpoint.unwrap();
        assert_eq!(endpoint.host, "new-endpoint.playit.gg");
        assert_eq!(endpoint.port, 25600);
    }

    #[test]
    fn falls_back_to_cache_when_offline() {
        let http = TestServer::new();
        serve_all(&http, server_json("skyandbeyond.playit.gg", 25565));
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        assert!(loader.load().online);
        drop(http);
        let snapshot = loader.load();
        assert!(!snapshot.online);
        assert!(snapshot.from_cache);
        assert_eq!(
            snapshot.server.endpoint.as_ref().unwrap().address(),
            "skyandbeyond.playit.gg:25565"
        );
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.code == "network_error"));
        assert_eq!(snapshot.sources.server.kind, "cache");
    }

    #[test]
    fn invalid_news_does_not_break_other_configuration() {
        let http = TestServer::new();
        serve_all(&http, server_json("skyandbeyond.playit.gg", 25565));
        http.set("/config/news.json", "{ this is not json");
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        let snapshot = loader.load();
        assert!(!snapshot.online);
        assert_eq!(snapshot.sources.news.kind, "default");
        assert_eq!(snapshot.sources.server.kind, "remote");
        assert!(snapshot.news.items.is_empty());
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.file == "news" && warning.code == "invalid_json"));
    }

    #[test]
    fn rejects_ip_literals_anywhere_in_remote_configuration() {
        let http = TestServer::new();
        let payload = server_json("skyandbeyond.playit.gg", 25565).replace(
            "\"A server\"",
            "\"Connect to 192.0.2.51\"",
        );
        serve_all(&http, payload);
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        let snapshot = loader.load();
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.file == "server" && warning.code == "config_ip_literal"));
        assert!(snapshot.server.endpoint.is_none());
    }

    #[test]
    fn uses_cached_endpoint_when_remote_removes_it() {
        let http = TestServer::new();
        serve_all(&http, server_json("skyandbeyond.playit.gg", 25565));
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        assert!(loader.load().server.endpoint.is_some());
        serve_all(
            &http,
            r#"{"schemaVersion":1,"displayName":"Sky & Beyond"}"#.to_string(),
        );
        let snapshot = loader.load();
        assert_eq!(
            snapshot.server.endpoint.as_ref().unwrap().address(),
            "skyandbeyond.playit.gg:25565"
        );
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.code == "server_endpoint_cached"));
    }

    #[test]
    fn offline_without_cache_returns_safe_defaults() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let base = format!("http://127.0.0.1:{port}/");
        let loader = RemoteLoader::new(&base, dir.path(), true, logger(dir.path())).unwrap();
        let snapshot = loader.load();
        assert!(!snapshot.online);
        assert!(!snapshot.from_cache);
        assert_eq!(snapshot.sources.server.kind, "default");
        assert!(snapshot.server.endpoint.is_none());
        assert!(snapshot.warnings.len() >= 4);
    }

    #[test]
    fn endpoint_only_refresh_uses_cache() {
        let http = TestServer::new();
        serve_all(&http, server_json("skyandbeyond.playit.gg", 25565));
        let dir = tempfile::tempdir().unwrap();
        let loader = RemoteLoader::new(&http.base_url(), dir.path(), true, logger(dir.path())).unwrap();
        loader.load();
        drop(http);
        let refresh = loader.refresh_endpoint();
        assert_eq!(
            refresh.endpoint.unwrap().address(),
            "skyandbeyond.playit.gg:25565"
        );
        assert!(refresh
            .warnings
            .iter()
            .any(|warning| warning.code == "network_error"));
    }

    #[test]
    fn rejects_insecure_remote_base() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RemoteLoader::new("http://example.com/", dir.path(), true, logger(dir.path())).is_err());
        assert!(RemoteLoader::new("https://example.com/", dir.path(), false, logger(dir.path())).is_ok());
        assert!(RemoteLoader::new("file:///etc/", dir.path(), true, logger(dir.path())).is_err());
    }
}
