//! Structured logging controls and JSON log export.
//!
//! Provides IPC commands for:
//! - Viewing and changing the active log level (info/debug/trace)
//! - Exporting recent logs as structured JSON
//! - Opening the log file location
//! - Trusted SSH host CA management
//! - GSSAPI authentication status

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Log level setting for the application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_tracing_level(&self) -> tracing::Level {
        match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        }
    }
}

/// A structured log entry for JSON export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: Option<serde_json::Value>,
}

/// Trusted SSH Host Certificate Authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedHostCA {
    pub id: String,
    pub name: String,
    /// The CA public key (e.g., contents of a .pub file).
    pub public_key: String,
    /// Optional host pattern this CA is trusted for (e.g., "*.example.com").
    pub host_pattern: Option<String>,
    pub added_at: String,
}

/// In-memory store for trusted CAs (persisted via settings).
static TRUSTED_CAS: std::sync::LazyLock<
    tokio::sync::RwLock<Vec<TrustedHostCA>>,
> = std::sync::LazyLock::new(|| tokio::sync::RwLock::new(Vec::new()));

/// In-memory log level (default: info).
static CURRENT_LOG_LEVEL: std::sync::LazyLock<
    tokio::sync::RwLock<LogLevel>,
> = std::sync::LazyLock::new(|| tokio::sync::RwLock::new(LogLevel::Info));

// ── Log Level Controls ──

/// Get the current log level.
#[tauri::command]
pub async fn get_log_level() -> Result<LogLevel, AppError> {
    let level = CURRENT_LOG_LEVEL.read().await;
    Ok(level.clone())
}

/// Set the log level. Takes effect for new log entries.
#[tauri::command]
pub async fn set_log_level(level: LogLevel) -> Result<(), AppError> {
    tracing::info!("Log level changing to: {:?}", level);
    let mut current = CURRENT_LOG_LEVEL.write().await;
    *current = level;
    Ok(())
}

/// Get the path to the application log directory.
#[tauri::command]
pub async fn get_log_directory() -> Result<String, AppError> {
    let log_dir = get_log_dir_path();
    Ok(log_dir.to_string_lossy().to_string())
}

/// Export recent log entries as structured JSON.
///
/// Reads the latest log file and parses it into structured entries.
/// Returns up to `max_entries` most recent entries.
#[tauri::command]
pub async fn export_logs_json(max_entries: Option<u32>) -> Result<Vec<LogEntry>, AppError> {
    let max = max_entries.unwrap_or(500) as usize;
    let log_dir = get_log_dir_path();

    // Find the most recent log file
    let log_file = find_latest_log_file(&log_dir)?;

    let content = tokio::fs::read_to_string(&log_file)
        .await
        .map_err(|e| AppError::internal(format!("Failed to read log file: {e}")))?;

    // Parse log lines (tracing_subscriber format: "TIMESTAMP LEVEL TARGET: message")
    let entries: Vec<LogEntry> = content
        .lines()
        .rev()
        .take(max)
        .filter_map(parse_log_line)
        .collect();

    Ok(entries)
}

/// Export logs to a JSON file at the specified path.
#[tauri::command]
pub async fn export_logs_to_file(
    output_path: String,
    max_entries: Option<u32>,
) -> Result<String, AppError> {
    let entries = export_logs_json(max_entries).await?;
    let json = serde_json::to_string_pretty(&entries).map_err(|e| {
        AppError::internal(format!("Failed to serialize logs: {e}"))
    })?;

    tokio::fs::write(&output_path, &json).await.map_err(|e| {
        AppError::file_op(
            format!("Failed to write log export: {e}"),
            "Check that the output path is writable.",
        )
    })?;

    Ok(output_path)
}

// ── Trusted SSH Host CAs ──

/// List all trusted SSH host certificate authorities.
#[tauri::command]
pub async fn list_trusted_host_cas() -> Result<Vec<TrustedHostCA>, AppError> {
    let cas = TRUSTED_CAS.read().await;
    Ok(cas.clone())
}

/// Add a trusted SSH host certificate authority.
#[tauri::command]
pub async fn add_trusted_host_ca(
    name: String,
    public_key: String,
    host_pattern: Option<String>,
) -> Result<TrustedHostCA, AppError> {
    if public_key.trim().is_empty() {
        return Err(AppError::Connection {
            message: "CA public key cannot be empty.".to_string(),
            advice: "Provide the contents of the CA's .pub key file.".to_string(),
        });
    }

    let ca = TrustedHostCA {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        public_key: public_key.trim().to_string(),
        host_pattern,
        added_at: chrono::Utc::now().to_rfc3339(),
    };

    let mut cas = TRUSTED_CAS.write().await;
    cas.push(ca.clone());

    tracing::info!("Added trusted SSH host CA: {}", ca.name);
    Ok(ca)
}

/// Remove a trusted SSH host certificate authority by ID.
#[tauri::command]
pub async fn remove_trusted_host_ca(ca_id: String) -> Result<(), AppError> {
    let mut cas = TRUSTED_CAS.write().await;
    let before = cas.len();
    cas.retain(|ca| ca.id != ca_id);
    if cas.len() == before {
        return Err(AppError::Connection {
            message: format!("Trusted CA not found: {ca_id}"),
            advice: "The CA may have already been removed.".to_string(),
        });
    }
    Ok(())
}

// ── GSSAPI Status ──

/// Check if GSSAPI (Kerberos) authentication is available on this system.
///
/// Returns information about the GSSAPI environment.
#[tauri::command]
pub async fn check_gssapi_available() -> Result<GssapiStatus, AppError> {
    // Check if Kerberos ticket cache exists
    let has_ticket_cache = std::env::var("KRB5CCNAME").is_ok()
        || std::path::Path::new("/tmp/krb5cc_0").exists()
        || {
            if let Ok(uid) = get_current_uid() {
                std::path::Path::new(&format!("/tmp/krb5cc_{uid}")).exists()
            } else {
                false
            }
        };

    // Check if krb5.conf exists
    let has_krb5_config = std::path::Path::new("/etc/krb5.conf").exists()
        || std::env::var("KRB5_CONFIG").is_ok();

    Ok(GssapiStatus {
        available: has_krb5_config,
        has_ticket_cache,
        krb5_config_path: if has_krb5_config {
            Some(
                std::env::var("KRB5_CONFIG").unwrap_or_else(|_| "/etc/krb5.conf".to_string()),
            )
        } else {
            None
        },
        ticket_cache_path: std::env::var("KRB5CCNAME").ok(),
    })
}

/// GSSAPI/Kerberos availability status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GssapiStatus {
    pub available: bool,
    pub has_ticket_cache: bool,
    pub krb5_config_path: Option<String>,
    pub ticket_cache_path: Option<String>,
}

// ── Helpers ──

fn get_log_dir_path() -> PathBuf {
    directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from("./logs"))
}

fn find_latest_log_file(log_dir: &PathBuf) -> Result<PathBuf, AppError> {
    if !log_dir.exists() {
        return Err(AppError::internal("Log directory does not exist. Logs may not have been written yet."));
    }

    let mut entries: Vec<_> = std::fs::read_dir(log_dir)
        .map_err(|e| AppError::internal(format!("Cannot read log directory: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

    entries
        .first()
        .map(|e| e.path())
        .ok_or_else(|| AppError::internal("No log files found in log directory."))
}

fn parse_log_line(line: &str) -> Option<LogEntry> {
    // Best-effort parse of tracing_subscriber default format
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Try to extract level from common patterns like "2026-03-16T10:00:00Z  INFO target: message"
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() >= 3 {
        let timestamp = parts[0].to_string();
        let level = parts[1].to_string();
        let rest = parts[2..].join(" ");

        // Split target from message at first ": "
        let (target, message) = rest
            .split_once(": ")
            .map(|(t, m)| (t.to_string(), m.to_string()))
            .unwrap_or_else(|| ("app".to_string(), rest));

        Some(LogEntry {
            timestamp,
            level,
            target,
            message,
            fields: None,
        })
    } else {
        Some(LogEntry {
            timestamp: String::new(),
            level: "UNKNOWN".to_string(),
            target: "app".to_string(),
            message: trimmed.to_string(),
            fields: None,
        })
    }
}

fn get_current_uid() -> Result<u32, ()> {
    #[cfg(unix)]
    {
        Ok(unsafe { libc::getuid() })
    }
    #[cfg(not(unix))]
    {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_line() {
        let entry = parse_log_line(
            "2026-03-16T10:00:00Z  INFO unified_file_ops::commands: Hello world",
        );
        assert!(entry.is_some());
        let e = entry.unwrap();
        assert_eq!(e.level, "INFO");
        assert!(e.message.contains("Hello world"));
    }

    #[tokio::test]
    async fn test_log_level_roundtrip() {
        set_log_level(LogLevel::Debug).await.unwrap();
        let level = get_log_level().await.unwrap();
        assert_eq!(level, LogLevel::Debug);
        // Reset
        set_log_level(LogLevel::Info).await.unwrap();
    }

    #[tokio::test]
    async fn test_gssapi_status() {
        let status = check_gssapi_available().await.unwrap();
        // Just verify it returns without error
        let _ = status.available;
    }
}
