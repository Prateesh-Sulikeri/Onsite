use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Claude Code not found. Install it or set its path in Settings.")]
    ClaudeNotFound,
    #[error("Claude Code failed: {0}")]
    Claude(String),
    /// A provider API call failed; the text is already plain.
    #[error("{0}")]
    Ai(String),
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

/// For log lines that carry text we do not control (provider error bodies, CLI output): one line, at most 300 chars.
/// Logs must never hold speech, transcripts, resume, job description or keys; see `docs/engineering/REPORT.md`.
pub fn log_clip(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= 300 {
        one
    } else {
        format!("{}...", one.chars().take(300).collect::<String>())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
