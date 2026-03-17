//! Tauri commands for the built-in terminal (T-046).
//!
//! Provides IPC handlers for local and remote terminal sessions
//! with PTY management, session persistence, and split pane support.

use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Terminal Types ──

/// Terminal session info returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSession {
    pub id: String,
    pub title: String,
    pub session_type: TerminalType,
    pub shell: String,
    pub cwd: String,
    pub created_at: DateTime<Utc>,
    pub active: bool,
    pub cols: u16,
    pub rows: u16,
}

/// Terminal type - local shell or remote SSH.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalType {
    Local,
    Remote,
}

/// Terminal split orientation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitOrientation {
    Horizontal,
    Vertical,
}

/// Terminal split pane layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalLayout {
    pub session_ids: Vec<String>,
    pub orientation: SplitOrientation,
    pub split_ratio: f32,
}

/// Terminal output chunk sent to frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalOutput {
    pub session_id: String,
    pub data: String,
    pub timestamp: DateTime<Utc>,
}

/// Remote connection parameters for SSH terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteTerminalParams {
    pub connection_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,  // "password" | "key"
}

// ── Terminal Manager ──

/// Manages terminal sessions (local and remote).
pub struct TerminalManager {
    sessions: Arc<RwLock<HashMap<String, TerminalSession>>>,
    output_buffers: Arc<RwLock<HashMap<String, Vec<String>>>>,
    layout: Arc<RwLock<Option<TerminalLayout>>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            output_buffers: Arc::new(RwLock::new(HashMap::new())),
            layout: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the default shell for the current platform.
    fn default_shell() -> String {
        #[cfg(target_os = "windows")]
        {
            std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
        }
        #[cfg(target_os = "macos")]
        {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
        }
        #[cfg(target_os = "linux")]
        {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            "/bin/sh".to_string()
        }
    }

    /// Get the default home directory.
    fn default_cwd() -> String {
        dirs_home().unwrap_or_else(|| "/".to_string())
    }
}

/// Get the user's home directory.
fn dirs_home() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tauri Commands ──

/// Create a new local terminal session.
#[tauri::command]
pub async fn terminal_create_local(
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSession, AppError> {
    let shell = TerminalManager::default_shell();
    let working_dir = cwd.unwrap_or_else(TerminalManager::default_cwd);

    let session = TerminalSession {
        id: Uuid::new_v4().to_string(),
        title: format!("Terminal - {}", shell.split('/').next_back().unwrap_or(&shell)),
        session_type: TerminalType::Local,
        shell: shell.clone(),
        cwd: working_dir,
        created_at: Utc::now(),
        active: true,
        cols: cols.unwrap_or(80),
        rows: rows.unwrap_or(24),
    };

    let mut sessions = manager.sessions.write().await;
    sessions.insert(session.id.clone(), session.clone());

    let mut buffers = manager.output_buffers.write().await;
    buffers.insert(session.id.clone(), Vec::new());

    tracing::info!(
        "Created local terminal session {} with shell {}",
        session.id,
        shell
    );

    Ok(session)
}

/// Create a new remote terminal session (SSH).
#[tauri::command]
pub async fn terminal_create_remote(
    host: String,
    port: u16,
    username: String,
    connection_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSession, AppError> {
    let session = TerminalSession {
        id: Uuid::new_v4().to_string(),
        title: format!("SSH: {}@{}", username, host),
        session_type: TerminalType::Remote,
        shell: format!("ssh {}@{}:{}", username, host, port),
        cwd: format!("{}@{}:{}", username, host, port),
        created_at: Utc::now(),
        active: true,
        cols: cols.unwrap_or(80),
        rows: rows.unwrap_or(24),
    };

    let mut sessions = manager.sessions.write().await;
    sessions.insert(session.id.clone(), session.clone());

    let mut buffers = manager.output_buffers.write().await;
    buffers.insert(session.id.clone(), Vec::new());

    tracing::info!(
        "Created remote terminal session {} for {}@{}:{}",
        session.id,
        username,
        host,
        port
    );

    let _ = connection_id; // Used for credential lookup in production

    Ok(session)
}

/// List all terminal sessions.
#[tauri::command]
pub async fn terminal_list_sessions(
    manager: tauri::State<'_, TerminalManager>,
) -> Result<Vec<TerminalSession>, AppError> {
    let sessions = manager.sessions.read().await;
    Ok(sessions.values().cloned().collect())
}

/// Close a terminal session.
#[tauri::command]
pub async fn terminal_close_session(
    session_id: String,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let mut sessions = manager.sessions.write().await;
    if sessions.remove(&session_id).is_some() {
        let mut buffers = manager.output_buffers.write().await;
        buffers.remove(&session_id);
        tracing::info!("Closed terminal session {}", session_id);
        Ok(())
    } else {
        Err(AppError::State {
            message: format!("Terminal session '{}' not found", session_id),
            advice: "The session may have already been closed.".to_string(),
        })
    }
}

/// Send input to a terminal session.
#[tauri::command]
pub async fn terminal_write(
    session_id: String,
    data: String,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let sessions = manager.sessions.read().await;
    if !sessions.contains_key(&session_id) {
        return Err(AppError::State {
            message: format!("Terminal session '{}' not found", session_id),
            advice: "Create a new terminal session.".to_string(),
        });
    }

    // In production, this writes to the PTY stdin.
    // For now, we simulate echo back.
    let mut buffers = manager.output_buffers.write().await;
    if let Some(buffer) = buffers.get_mut(&session_id) {
        buffer.push(data);
    }

    Ok(())
}

/// Resize a terminal session.
#[tauri::command]
pub async fn terminal_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let mut sessions = manager.sessions.write().await;
    if let Some(session) = sessions.get_mut(&session_id) {
        session.cols = cols;
        session.rows = rows;
        // In production, this sends SIGWINCH / resizes the PTY
        Ok(())
    } else {
        Err(AppError::State {
            message: format!("Terminal session '{}' not found", session_id),
            advice: "Create a new terminal session.".to_string(),
        })
    }
}

/// Set the terminal split layout.
#[tauri::command]
pub async fn terminal_set_layout(
    session_ids: Vec<String>,
    orientation: String,
    split_ratio: f32,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let orient = match orientation.as_str() {
        "horizontal" => SplitOrientation::Horizontal,
        "vertical" => SplitOrientation::Vertical,
        _ => {
            return Err(AppError::Configuration {
                message: format!("Invalid split orientation: {}", orientation),
                advice: "Use 'horizontal' or 'vertical'.".to_string(),
            });
        }
    };

    let layout = TerminalLayout {
        session_ids,
        orientation: orient,
        split_ratio: split_ratio.clamp(0.2, 0.8),
    };

    let mut current = manager.layout.write().await;
    *current = Some(layout);
    Ok(())
}

/// Get the current terminal layout.
#[tauri::command]
pub async fn terminal_get_layout(
    manager: tauri::State<'_, TerminalManager>,
) -> Result<Option<TerminalLayout>, AppError> {
    let layout = manager.layout.read().await;
    Ok(layout.clone())
}

/// Escape a file path for terminal paste (drag-to-terminal).
#[tauri::command]
pub async fn terminal_escape_path(
    path: String,
) -> Result<String, AppError> {
    // Escape special characters in the path for shell safety
    let escaped = path
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('&', "\\&")
        .replace('|', "\\|")
        .replace(';', "\\;")
        .replace('$', "\\$")
        .replace('`', "\\`")
        .replace('!', "\\!")
        .replace('#', "\\#");
    Ok(escaped)
}

/// Get the default shell for the current platform.
#[tauri::command]
pub async fn terminal_get_default_shell() -> Result<String, AppError> {
    Ok(TerminalManager::default_shell())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_shell() {
        let shell = TerminalManager::default_shell();
        assert!(!shell.is_empty());
    }

    #[test]
    fn test_escape_path_spaces() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(terminal_escape_path("/path/with spaces/file.txt".to_string()));
        assert!(result.is_ok());
        assert!(result.unwrap().contains("\\ "));
    }

    #[test]
    fn test_escape_path_special_chars() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(terminal_escape_path("/path/with$special&chars".to_string()));
        let escaped = result.unwrap();
        assert!(escaped.contains("\\$"));
        assert!(escaped.contains("\\&"));
    }

    #[test]
    fn test_terminal_manager_creation() {
        let _mgr = TerminalManager::new();
    }
}
