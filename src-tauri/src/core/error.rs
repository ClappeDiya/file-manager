use serde::Serialize;
use thiserror::Error;

/// Unified error type for the application.
/// All errors include a user-facing message explaining what happened,
/// why it happened, and what the user can do about it.
#[derive(Debug, Error, Serialize)]
pub enum AppError {
    #[error("File operation failed: {message}. {advice}")]
    FileOperation { message: String, advice: String },

    #[error("Database error: {message}. {advice}")]
    Database { message: String, advice: String },

    #[error("Connection failed: {message}. {advice}")]
    Connection { message: String, advice: String },

    #[error("Transfer failed: {message}. {advice}")]
    Transfer { message: String, advice: String },

    #[error("Permission denied: {message}. {advice}")]
    PermissionDenied { message: String, advice: String },

    #[error("Configuration error: {message}. {advice}")]
    Configuration { message: String, advice: String },

    #[error("{message}")]
    Internal { message: String },
}

impl AppError {
    pub fn file_op(message: impl Into<String>, advice: impl Into<String>) -> Self {
        Self::FileOperation {
            message: message.into(),
            advice: advice.into(),
        }
    }

    pub fn database(message: impl Into<String>, advice: impl Into<String>) -> Self {
        Self::Database {
            message: message.into(),
            advice: advice.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        let advice = match err.kind() {
            std::io::ErrorKind::NotFound => "Check that the file or folder exists and the path is correct.",
            std::io::ErrorKind::PermissionDenied => "Check file permissions or try running with appropriate access.",
            std::io::ErrorKind::AlreadyExists => "A file or folder with this name already exists. Try a different name.",
            _ => "If this persists, restart the application or check your system resources.",
        };
        Self::FileOperation {
            message: err.to_string(),
            advice: advice.to_string(),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database {
            message: format!("Database operation failed: {err}"),
            advice: "If this persists, try restarting the application. Your data is safe.".to_string(),
        }
    }
}

// AppError implements Serialize, so Tauri's blanket impl
// `From<T: Serialize> for InvokeError` handles IPC transport automatically.
// No manual From impl needed.
