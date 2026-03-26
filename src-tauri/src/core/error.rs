use serde::Serialize;
use thiserror::Error;

/// Unified error type for the application.
/// All errors include a user-facing message explaining what happened,
/// why it happened, and what the user can do about it.
///
/// Internal details are logged server-side but NEVER sent to the frontend.
///
/// # Note on path exposure in error messages
///
/// `FileOperation` errors intentionally include file paths in their messages.
/// This is by design: UFOP is a file manager, and users need to see which
/// file or folder caused the error. Paths are user-supplied and already
/// visible in the UI, so including them in error messages does not leak
/// additional information. Only the `Internal` variant sanitizes its message.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("File operation failed: {message}. {advice}")]
    FileOperation { message: String, advice: String },

    #[error("Database error: {message}. {advice}")]
    Database { message: String, advice: String },

    #[error("Connection failed: {message}. {advice}")]
    Connection { message: String, advice: String },

    #[error("Transfer failed: {message}. {advice}")]
    Transfer { message: String, advice: String },

    #[error("Sync failed: {message}. {advice}")]
    Sync { message: String, advice: String },

    #[error("Permission denied: {message}. {advice}")]
    PermissionDenied { message: String, advice: String },

    #[error("Configuration error: {message}. {advice}")]
    Configuration { message: String, advice: String },

    #[error("State error: {message}. {advice}")]
    State { message: String, advice: String },

    #[error("Encryption error: {message}. {advice}")]
    Encryption { message: String, advice: String },

    /// Internal errors log the real cause server-side but present a
    /// sanitised message to the user so secrets are never leaked.
    #[error("{message}")]
    Internal { message: String },
}

/// Serialized representation sent over IPC. This ensures we never
/// accidentally leak stack traces or internal details.
#[derive(Debug, Serialize)]
struct ErrorPayload {
    error_type: &'static str,
    message: String,
    advice: String,
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let payload = match self {
            Self::FileOperation { message, advice } => ErrorPayload {
                error_type: "file_operation",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Database { message, advice } => ErrorPayload {
                error_type: "database",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Connection { message, advice } => ErrorPayload {
                error_type: "connection",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Transfer { message, advice } => ErrorPayload {
                error_type: "transfer",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Sync { message, advice } => ErrorPayload {
                error_type: "sync",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::PermissionDenied { message, advice } => ErrorPayload {
                error_type: "permission_denied",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Configuration { message, advice } => ErrorPayload {
                error_type: "configuration",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::State { message, advice } => ErrorPayload {
                error_type: "state",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Encryption { message, advice } => ErrorPayload {
                error_type: "encryption",
                message: message.clone(),
                advice: advice.clone(),
            },
            Self::Internal { .. } => ErrorPayload {
                error_type: "internal",
                message: "An unexpected error occurred.".to_string(),
                advice: "If this persists, restart the application.".to_string(),
            },
        };
        payload.serialize(serializer)
    }
}

// ── Convenience constructors ──

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

    pub fn state(message: impl Into<String>, advice: impl Into<String>) -> Self {
        Self::State {
            message: message.into(),
            advice: advice.into(),
        }
    }

    pub fn encryption(message: impl Into<String>, advice: impl Into<String>) -> Self {
        Self::Encryption {
            message: message.into(),
            advice: advice.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        let msg: String = message.into();
        tracing::error!("Internal error: {msg}");
        Self::Internal { message: msg }
    }

    /// Convenience for "not found" errors (maps to FileOperation).
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::FileOperation {
            message: message.into(),
            advice: "Check that the path exists and you have access.".to_string(),
        }
    }

    /// Convenience for validation errors (maps to FileOperation).
    pub fn validation(message: impl Into<String>) -> Self {
        Self::FileOperation {
            message: message.into(),
            advice: "Check your input and try again.".to_string(),
        }
    }

    /// Convenience for connection errors.
    pub fn connection(message: impl Into<String>, advice: impl Into<String>) -> Self {
        Self::Connection {
            message: message.into(),
            advice: advice.into(),
        }
    }

    /// Convenience for transfer errors.
    pub fn transfer(message: impl Into<String>, advice: impl Into<String>) -> Self {
        Self::Transfer {
            message: message.into(),
            advice: advice.into(),
        }
    }
}

// ── From impls for library errors ──

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        let advice = match err.kind() {
            std::io::ErrorKind::NotFound => {
                "Check that the file or folder exists and the path is correct."
            }
            std::io::ErrorKind::PermissionDenied => {
                "Check file permissions or try running with appropriate access."
            }
            std::io::ErrorKind::AlreadyExists => {
                "A file or folder with this name already exists. Try a different name."
            }
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
        tracing::error!("SQLite error: {err}");
        Self::Database {
            message: "Database operation failed.".to_string(),
            advice: "If this persists, try restarting the application. Your data is safe."
                .to_string(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        tracing::error!("JSON serialization error: {err}");
        Self::State {
            message: "Failed to process application state.".to_string(),
            advice: "The application will use default settings.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_error_does_not_leak() {
        let err = AppError::internal("secret database password: hunter2");
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("hunter2"));
        assert!(json.contains("unexpected error"));
    }

    #[test]
    fn test_file_op_error_serialization() {
        let err = AppError::file_op("Not found", "Check the path");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("Not found"));
        assert!(json.contains("Check the path"));
        assert!(json.contains("file_operation"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let app_err: AppError = io_err.into();
        match &app_err {
            AppError::FileOperation { advice, .. } => {
                assert!(advice.contains("exists"));
            }
            _ => panic!("Expected FileOperation variant"),
        }
    }

    #[test]
    fn test_database_error_serialization() {
        let err = AppError::database("Query failed", "Retry later");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("database"));
    }
}
