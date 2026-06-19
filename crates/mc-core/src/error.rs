//! Structured, matchable error type for the whole core. UI layers turn specific
//! variants into specific guidance (e.g. XSTS error codes → human hints).

use std::path::PathBuf;

/// The result type used throughout `mc-core`.
pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("download failed for {url}: {reason}")]
    Download { url: String, reason: String },

    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    Checksum { path: PathBuf, expected: String, actual: String },

    #[error("failed to parse {what}: {source}")]
    Parse {
        what: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("version {0} not found")]
    VersionNotFound(String),

    #[error("no Java {major} found and auto-install is disabled")]
    JavaNotFound { major: u8 },

    #[error("Java at {path} is invalid: {reason}")]
    JavaInvalid { path: PathBuf, reason: String },

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("Xbox auth error {code}: {hint}")]
    Xsts { code: u64, hint: String },

    #[error("instance {0} not found")]
    InstanceNotFound(String),

    #[error("launch failed: {0}")]
    Launch(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("zip error: {0}")]
    Zip(String),

    #[error("{0}")]
    Other(String),
}

impl CoreError {
    /// Construct an [`CoreError::Io`] attaching the offending path.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        CoreError::Io { path: path.into(), source }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        CoreError::Other(msg.into())
    }
}

/// Extension trait to attach a path to an `io::Result`.
pub trait IoResultExt<T> {
    fn with_path(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T> IoResultExt<T> for std::io::Result<T> {
    fn with_path(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|e| CoreError::io(path, e))
    }
}
