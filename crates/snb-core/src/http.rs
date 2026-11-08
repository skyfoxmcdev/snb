use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::RANGE;
use reqwest::StatusCode;

use crate::error::{AppError, AppResult};
use crate::util::{available_space, redact_text, sha1_file, sha256_file, url_is_loopback};

#[derive(Debug, Clone, Default)]
pub struct ExpectedFile {
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

impl ExpectedFile {
    pub fn sha1(value: impl Into<String>, size: Option<u64>) -> Self {
        Self {
            sha1: Some(value.into().to_ascii_lowercase()),
            sha256: None,
            size,
        }
    }

    pub fn sha256(value: impl Into<String>, size: Option<u64>) -> Self {
        Self {
            sha1: None,
            sha256: Some(value.into().to_ascii_lowercase()),
            size,
        }
    }

    pub fn sha1_and_sha256(
        sha1: impl Into<String>,
        sha256: impl Into<String>,
        size: Option<u64>,
    ) -> Self {
        Self {
            sha1: Some(sha1.into().to_ascii_lowercase()),
            sha256: Some(sha256.into().to_ascii_lowercase()),
            size,
        }
    }

    pub fn matches(&self, path: &Path) -> bool {
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        if let Some(size) = self.size {
            if metadata.len() != size {
                return false;
            }
        }
        if let Some(expected) = &self.sha1 {
            match sha1_file(path) {
                Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
                _ => return false,
            }
        }
        if let Some(expected) = &self.sha256 {
            match sha256_file(path) {
                Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
                _ => return false,
            }
        }
        if self.sha1.is_none() && self.sha256.is_none() && self.size.is_none() {
            return false;
        }
        true
    }

    pub fn has_integrity(&self) -> bool {
        self.sha1.is_some() || self.sha256.is_some()
    }
}

fn loopback_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else {
            let url = attempt.url();
            let allowed = url.scheme() == "https"
                || (url.scheme() == "http" && url_is_loopback(url));
            if allowed {
                attempt.follow()
            } else {
                attempt.error("redirect to a non-HTTPS URL was refused")
            }
        }
    })
}

#[derive(Clone)]
pub struct Downloader {
    client: Client,
}

impl Downloader {
    pub fn new() -> AppResult<Self> {
        let client = Client::builder()
            .user_agent("SkyAndBeyondLauncher/0.1.0")
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(30 * 60))
            .redirect(loopback_redirect_policy())
            .build()
            .map_err(|error| network_error("build HTTP client", error))?;
        Ok(Self { client })
    }

    pub fn fetch_bytes(
        &self,
        url: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> AppResult<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .timeout(timeout)
            .send()
            .map_err(|error| network_error(url, error))?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_status_error(url, status));
        }
        if let Some(length) = response.content_length() {
            if length > max_bytes as u64 {
                return Err(AppError::with_detail(
                    "response_too_large",
                    format!("{url} announced {length} bytes"),
                ));
            }
        }
        let mut body = Vec::new();
        response
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| network_io_error(url, error))?;
        if body.len() > max_bytes {
            return Err(AppError::with_detail(
                "response_too_large",
                format!("{url} exceeded {max_bytes} bytes"),
            ));
        }
        Ok(body)
    }

    pub fn fetch_text(
        &self,
        url: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> AppResult<String> {
        let bytes = self.fetch_bytes(url, max_bytes, timeout)?;
        String::from_utf8(bytes).map_err(Into::into)
    }

    /// Cheap reachability probe for public service status displays. It sends a
    /// HEAD request and reports success only for a 2xx answer, so a launcher
    /// status dot never claims a service is up without a real response.
    pub fn probe(&self, url: &str, timeout: Duration) -> bool {
        self.client
            .head(url)
            .timeout(timeout)
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    /// Downloads a file into a sibling `.part` file, resuming when possible. The
    /// final rename only happens after size and checksum verification.
    pub fn download(
        &self,
        url: &str,
        destination: &Path,
        expected: &ExpectedFile,
        mut progress: impl FnMut(u64, u64),
        cancel: &AtomicBool,
    ) -> AppResult<u64> {
        fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| AppError::new("invalid_file_name"))?,
        )?;

        if destination.exists() && expected.matches(destination) {
            let size = fs::metadata(destination).map(|m| m.len()).unwrap_or(0);
            progress(size, size);
            return Ok(size);
        }

        if let Some(size) = expected.size {
            if let Ok(free) = available_space(destination) {
                if free < size.saturating_add(64 * 1024 * 1024) {
                    return Err(AppError::with_detail(
                        "disk_full",
                        format!("need {size} bytes plus headroom, only {free} available"),
                    ));
                }
            }
        }

        let file_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::new("invalid_file_name"))?;
        let part = destination.with_file_name(format!(".{file_name}.part"));

        let mut last_error: Option<AppError> = None;
        for attempt in 0..2 {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::new("cancelled"));
            }
            match self.download_once(url, destination, &part, expected, &mut progress, cancel) {
                Ok(size) => return Ok(size),
                Err(error) => {
                    let retryable = matches!(
                        error.code(),
                        "network_error"
                            | "checksum_mismatch"
                            | "download_incomplete"
                            | "download_resume_failed"
                    );
                    let checksum = error.code() == "checksum_mismatch";
                    if checksum {
                        let _ = fs::remove_file(&part);
                    }
                    last_error = Some(error);
                    if !retryable || attempt == 1 || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| AppError::new("download_failed")))
    }

    fn download_once(
        &self,
        url: &str,
        destination: &Path,
        part: &Path,
        expected: &ExpectedFile,
        progress: &mut impl FnMut(u64, u64),
        cancel: &AtomicBool,
    ) -> AppResult<u64> {
        let existing = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if existing > 0 {
            if expected.size == Some(existing) && expected.matches(part) {
                finalize(part, destination)?;
                progress(existing, existing);
                return Ok(existing);
            }
            if let Some(total) = expected.size {
                if existing > total {
                    let _ = fs::remove_file(part);
                }
            }
        }
        let offset = fs::metadata(part).map(|m| m.len()).unwrap_or(0);

        let mut request = self.client.get(url);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let response = request
            .send()
            .map_err(|error| network_error(url, error))?;
        let status = response.status();

        let (append, total) = match status {
            StatusCode::PARTIAL_CONTENT => {
                let total = expected
                    .size
                    .or_else(|| {
                        response
                            .content_length()
                            .map(|length| offset.saturating_add(length))
                    })
                    .unwrap_or(0);
                (true, total)
            }
            StatusCode::OK => {
                let total = expected.size.or_else(|| response.content_length()).unwrap_or(0);
                (false, total)
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                if expected.size == Some(offset) && expected.matches(part) {
                    finalize(part, destination)?;
                    progress(offset, offset);
                    return Ok(offset);
                }
                let _ = fs::remove_file(part);
                return Err(AppError::with_detail(
                    "download_resume_failed",
                    "server rejected the resume range",
                ));
            }
            other => return Err(http_status_error(url, other)),
        };

        if let Ok(free) = available_space(destination.parent().unwrap_or(Path::new("."))) {
            if total > offset && free < (total - offset).saturating_add(32 * 1024 * 1024) {
                return Err(AppError::new("disk_full"));
            }
        }

        let mut file = if append {
            OpenOptions::new().append(true).open(part)?
        } else {
            File::create(part)?
        };
        let mut downloaded = if append { offset } else { 0 };
        if total > 0 {
            progress(downloaded, total);
        }

        let mut reader = response;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = file.flush();
                return Err(AppError::new("cancelled"));
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|error| network_io_error(url, error))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])?;
            downloaded = downloaded.saturating_add(read as u64);
            progress(downloaded, total.max(downloaded));
        }
        file.flush()?;
        file.sync_all()?;
        drop(file);

        let actual_size = fs::metadata(part)?.len();
        if let Some(size) = expected.size {
            if actual_size != size {
                return Err(AppError::with_detail(
                    "download_incomplete",
                    format!("expected {size} bytes, got {actual_size}"),
                ));
            }
        }
        if expected.sha1.is_some() || expected.sha256.is_some() {
            if !expected.matches(part) {
                let _ = fs::remove_file(part);
                let file_name = file_name_of(destination);
                return Err(AppError::with_detail(
                    "checksum_mismatch",
                    format!("checksum verification failed for {file_name}"),
                ));
            }
        }
        finalize(part, destination)?;
        progress(actual_size, actual_size);
        Ok(actual_size)
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string()
}

fn finalize(part: &Path, destination: &Path) -> AppResult<()> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    if let Err(error) = fs::rename(part, destination) {
        if error.kind() == std::io::ErrorKind::AlreadyExists
            || error.kind() == std::io::ErrorKind::PermissionDenied
        {
            let _ = fs::remove_file(destination);
            fs::rename(part, destination)?;
        } else {
            return Err(error.into());
        }
    }
    Ok(())
}

fn network_error(url: &str, error: reqwest::Error) -> AppError {
    AppError::with_detail(
        "network_error",
        redact_text(&format!("{}: {}", redact_text(url), error)),
    )
}

fn network_io_error(url: &str, error: std::io::Error) -> AppError {
    AppError::with_detail(
        "network_error",
        redact_text(&format!("{}: {}", redact_text(url), error)),
    )
}

fn http_status_error(url: &str, status: StatusCode) -> AppError {
    let code = if status == StatusCode::NOT_FOUND {
        "resource_not_found"
    } else if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
        "resource_forbidden"
    } else if status.is_server_error() {
        "server_error"
    } else {
        "download_failed"
    };
    AppError::with_detail(
        code,
        format!("HTTP {} for {}", status.as_u16(), redact_text(url)),
    )
}

pub fn part_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    destination.with_file_name(format!(".{name}.part"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestServer;
    use crate::util::sha256_bytes;

    #[test]
    fn downloads_and_verifies_checksum() {
        let server = TestServer::new();
        let body: &[u8] = b"hello verified world";
        server.set("/file.bin", body.to_vec());
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        let expected =
            ExpectedFile::sha256(sha256_bytes(body), Some(body.len() as u64));
        let downloader = Downloader::new().unwrap();
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let bytes = downloader
            .download(
                &server.url("/file.bin"),
                &destination,
                &expected,
                |current, total| seen.push((current, total)),
                &cancel,
            )
            .unwrap();
        assert_eq!(bytes, body.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), body);
        assert!(!part_path(&destination).exists());
        assert!(seen
            .iter()
            .any(|(current, _)| *current == body.len() as u64));
    }

    #[test]
    fn resumes_a_partial_download() {
        let server = TestServer::new();
        server.set("/resume.bin", b"0123456789abcdef".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("resume.bin");
        let part = part_path(&destination);
        fs::write(&part, b"012345").unwrap();
        let expected = ExpectedFile::sha256(
            sha256_bytes(b"0123456789abcdef"),
            Some(16),
        );
        let downloader = Downloader::new().unwrap();
        let cancel = AtomicBool::new(false);
        downloader
            .download(
                &server.url("/resume.bin"),
                &destination,
                &expected,
                |_, _| {},
                &cancel,
            )
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"0123456789abcdef");
        let requests = server.requests();
        assert!(requests
            .iter()
            .any(|request| request.headers.get("range").map(String::as_str)
                == Some("bytes=6-")));
    }

    #[test]
    fn rejects_a_wrong_checksum_and_cleans_partial_data() {
        let server = TestServer::new();
        let body: &[u8] = b"not the expected bytes";
        server.set("/bad.bin", body.to_vec());
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("bad.bin");
        let expected = ExpectedFile::sha256(sha256_bytes(b"different"), Some(body.len() as u64));
        let downloader = Downloader::new().unwrap();
        let cancel = AtomicBool::new(false);
        let error = downloader
            .download(
                &server.url("/bad.bin"),
                &destination,
                &expected,
                |_, _| {},
                &cancel,
            )
            .unwrap_err();
        assert_eq!(error.code(), "checksum_mismatch");
        assert!(!destination.exists());
        assert!(!part_path(&destination).exists());
    }

    #[test]
    fn skips_existing_valid_file_without_network() {
        let server = TestServer::new();
        server.set("/cached.bin", b"cached".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("cached.bin");
        fs::write(&destination, b"cached").unwrap();
        let expected = ExpectedFile::sha256(sha256_bytes(b"cached"), Some(6));
        let downloader = Downloader::new().unwrap();
        let cancel = AtomicBool::new(false);
        downloader
            .download(
                &server.url("/cached.bin"),
                &destination,
                &expected,
                |_, _| {},
                &cancel,
            )
            .unwrap();
        assert!(server.requests().is_empty());
    }

    #[test]
    fn cancellation_stops_download() {
        let server = TestServer::new();
        server.set("/huge.bin", vec![7u8; 1_000_000]);
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("huge.bin");
        let downloader = Downloader::new().unwrap();
        let cancel = AtomicBool::new(true);
        let error = downloader
            .download(
                &server.url("/huge.bin"),
                &destination,
                &ExpectedFile::default(),
                |_, _| {},
                &cancel,
            )
            .unwrap_err();
        assert_eq!(error.code(), "cancelled");
        assert!(!destination.exists());
    }

    #[test]
    fn network_failure_is_reported_as_such() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let downloader = Downloader::new().unwrap();
        let error = downloader
            .download(
                &format!("http://127.0.0.1:{port}/down.bin"),
                &dir.path().join("down.bin"),
                &ExpectedFile::default(),
                |_, _| {},
                &AtomicBool::new(false),
            )
            .unwrap_err();
        assert_eq!(error.code(), "network_error");
    }

    #[test]
    fn fetch_limits_response_size() {
        let server = TestServer::new();
        server.set("/big.txt", vec![b'a'; 2000]);
        let downloader = Downloader::new().unwrap();
        let error = downloader
            .fetch_bytes(&server.url("/big.txt"), 100, Duration::from_secs(2))
            .unwrap_err();
        assert_eq!(error.code(), "response_too_large");
    }
}
