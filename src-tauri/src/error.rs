use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Claude Code not found. Install it or set its path in Settings.")]
    ClaudeNotFound,
    #[error("Claude Code failed: {0}")]
    Claude(String),
    #[error("{0}")]
    Voice(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// The UI gets the plain message.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
