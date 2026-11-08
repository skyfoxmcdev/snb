use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::AppResult;
use crate::util::{format_readable_utc, now_ms, redact_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

struct State {
    file: Option<File>,
    size: u64,
}

pub struct Logger {
    directory: PathBuf,
    log_path: PathBuf,
    max_bytes: u64,
    keep_files: usize,
    state: Mutex<State>,
}

impl Logger {
    pub fn new(directory: impl AsRef<Path>) -> AppResult<Arc<Self>> {
        Self::with_limits(directory, 5 * 1024 * 1024, 5)
    }

    pub fn with_limits(
        directory: impl AsRef<Path>,
        max_bytes: u64,
        keep_files: usize,
    ) -> AppResult<Arc<Self>> {
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(&directory)?;
        let log_path = directory.join("launcher.log");
        let size = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        Ok(Arc::new(Self {
            directory,
            log_path,
            max_bytes: max_bytes.max(512),
            keep_files: keep_files.max(1),
            state: Mutex::new(State {
                file: Some(file),
                size,
            }),
        }))
    }

    pub fn logs_dir(&self) -> &Path {
        &self.directory
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn info(&self, scope: &str, message: impl AsRef<str>) {
        self.write(LogLevel::Info, scope, message.as_ref());
    }

    pub fn warn(&self, scope: &str, message: impl AsRef<str>) {
        self.write(LogLevel::Warn, scope, message.as_ref());
    }

    pub fn error(&self, scope: &str, message: impl AsRef<str>) {
        self.write(LogLevel::Error, scope, message.as_ref());
    }

    fn write(&self, level: LogLevel, scope: &str, message: &str) {
        let scope = scope
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
            .take(40)
            .collect::<String>();
        let line = format!(
            "{} {} [{}] {}\n",
            format_readable_utc(now_ms()),
            level.label(),
            if scope.is_empty() { "app" } else { &scope },
            message
        );
        let redacted = redact_text(&line);
        let redacted = if redacted.chars().count() > 16_384 {
            format!("{}\n", redacted.chars().take(16_384).collect::<String>())
        } else {
            redacted
        };

        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.size + redacted.len() as u64 > self.max_bytes {
            self.rotate(&mut state);
        }
        let new_size = state.size.saturating_add(redacted.len() as u64);
        let written = state
            .file
            .as_mut()
            .map(|file| {
                file.write_all(redacted.as_bytes()).is_ok()
                    && file.flush().is_ok()
            })
            .unwrap_or(false);
        if written {
            state.size = new_size;
        }
    }

    fn rotate(&self, state: &mut State) {
        state.file.take();
        for index in (1..self.keep_files).rev() {
            let from = self.directory.join(format!("launcher.{index}.log"));
            let to = self
                .directory
                .join(format!("launcher.{}.log", index + 1));
            if from.exists() {
                let _ = fs::remove_file(&to);
                let _ = fs::rename(&from, &to);
            }
        }
        let first = self.directory.join("launcher.1.log");
        let _ = fs::remove_file(&first);
        let _ = fs::rename(&self.log_path, &first);
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            Ok(file) => {
                state.file = Some(file);
                state.size = 0;
            }
            Err(_) => {
                state.file = None;
                state.size = 0;
            }
        }
    }

    /// All rotated and current log files, newest first.
    pub fn log_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if self.log_path.exists() {
            files.push(self.log_path.clone());
        }
        for index in 1..=self.keep_files {
            let path = self.directory.join(format!("launcher.{index}.log"));
            if path.exists() {
                files.push(path);
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_redacted_lines() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::with_limits(dir.path(), 64 * 1024, 2).unwrap();
        logger.info("test", "connecting to 192.0.2.44 and accessToken=secret-value");
        let contents = fs::read_to_string(logger.log_path()).unwrap();
        assert!(contents.contains("[ip masquee]"));
        assert!(!contents.contains("192.0.2.44"));
        assert!(!contents.contains("secret-value"));
        assert!(contents.contains("INFO [test]"));
    }

    #[test]
    fn rotates_without_losing_files() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::with_limits(dir.path(), 1_500, 2).unwrap();
        for index in 0..50 {
            logger.info("rotate", format!("line {index} abcdefghijklmnopqrstuvwxyz"));
        }
        assert!(logger.log_path().exists());
        assert!(dir.path().join("launcher.1.log").exists());
        assert!(!dir.path().join("launcher.3.log").exists());
    }

    #[test]
    fn log_files_are_discoverable() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::new(dir.path()).unwrap();
        logger.info("test", "hello");
        assert_eq!(logger.log_files().len(), 1);
        assert!(logger.log_files()[0].ends_with("launcher.log"));
    }
}
