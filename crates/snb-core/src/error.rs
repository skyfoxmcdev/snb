use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub type AppResult<T> = Result<T, AppError>;

/// An error that is safe to show to the player. `code` is translated by the UI;
/// `detail` is technical context and is only shown as a secondary detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl UserError {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
            params: BTreeMap::new(),
        }
    }

    pub fn with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
            params: BTreeMap::new(),
        }
    }

    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{} ({detail})", self.code),
            None => write!(f, "{}", self.code),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppError {
    pub user: UserError,
}

impl AppError {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            user: UserError::new(code),
        }
    }

    pub fn with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            user: UserError::with_detail(code, detail),
        }
    }

    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.user.params.insert(key.into(), value.into());
        self
    }

    pub fn code(&self) -> &str {
        &self.user.code
    }

    pub fn detail_text(&self) -> &str {
        self.user.detail.as_deref().unwrap_or("")
    }

    pub fn into_user(self) -> UserError {
        self.user
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user)
    }
}

impl std::error::Error for AppError {}

impl From<UserError> for AppError {
    fn from(value: UserError) -> Self {
        Self { user: value }
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        let code = match error.kind() {
            ErrorKind::NotFound => "file_not_found",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::AlreadyExists => "file_already_exists",
            ErrorKind::UnexpectedEof => "file_truncated",
            _ if is_disk_full(&error) => "disk_full",
            _ => "file_error",
        };
        Self::with_detail(code, error.to_string())
    }
}

fn is_disk_full(error: &std::io::Error) -> bool {
    // ENOSPC on Unix, ERROR_DISK_FULL / ERROR_HANDLE_DISK_FULL on Windows.
    matches!(error.raw_os_error(), Some(28) | Some(112) | Some(39))
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        let code = if error.is_eof() {
            "file_truncated"
        } else {
            "invalid_json"
        };
        Self::with_detail(code, error.to_string())
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::with_detail("archive_error", error.to_string())
    }
}

impl From<toml::de::Error> for AppError {
    fn from(error: toml::de::Error) -> Self {
        Self::with_detail("invalid_toml", error.to_string())
    }
}

impl From<url::ParseError> for AppError {
    fn from(error: url::ParseError) -> Self {
        Self::with_detail("invalid_url", error.to_string())
    }
}

impl From<semver::Error> for AppError {
    fn from(error: semver::Error) -> Self {
        Self::with_detail("invalid_version", error.to_string())
    }
}

impl From<walkdir::Error> for AppError {
    fn from(error: walkdir::Error) -> Self {
        Self::with_detail("file_error", error.to_string())
    }
}

impl From<base64::DecodeError> for AppError {
    fn from(error: base64::DecodeError) -> Self {
        Self::with_detail("invalid_signature", error.to_string())
    }
}

impl From<std::num::ParseIntError> for AppError {
    fn from(error: std::num::ParseIntError) -> Self {
        Self::with_detail("invalid_number", error.to_string())
    }
}

impl From<std::string::FromUtf8Error> for AppError {
    fn from(error: std::string::FromUtf8Error) -> Self {
        Self::with_detail("invalid_encoding", error.to_string())
    }
}
