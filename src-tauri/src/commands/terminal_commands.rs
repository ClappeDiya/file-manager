//! Tauri commands for the built-in terminal (T-046).
//!
//! Provides IPC handlers for local and remote terminal sessions
//! with PTY management, session persistence, and split pane support.

use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::Emitter;
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
    ptys: Arc<StdMutex<HashMap<String, PtyHandle>>>,
    layout: Arc<RwLock<Option<TerminalLayout>>>,
}

/// Replay buffer kept per session so a re-opened panel can restore its screen.
const SCROLLBACK_MAX: usize = 512 * 1024;

/// A live pseudo-terminal: the master side, its input writer and the child shell.
pub(crate) struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    scrollback: Arc<StdMutex<Scrollback>>,
}

/// Recent output plus the total number of bytes ever produced, so a viewer can
/// splice replay and live events without duplicating or dropping output.
#[derive(Default)]
struct Scrollback {
    buf: Vec<u8>,
    total: u64,
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Spawn `program` on a new PTY. `on_output` is called from a reader thread
/// with each chunk of raw bytes; `on_exit` once when the child's output ends.
pub(crate) fn spawn_pty(
    program: &str,
    args: &[String],
    cwd: &str,
    cols: u16,
    rows: u16,
    on_output: impl Fn(&[u8], u64) + Send + 'static,
    on_exit: impl FnOnce() + Send + 'static,
) -> Result<PtyHandle, AppError> {
    let pty_err = |what: &str, e: &dyn std::fmt::Display| AppError::State {
        message: format!("Terminal: {what}: {e}"),
        advice: "Check that the shell exists and is executable, then try again.".to_string(),
    };
    let pair = native_pty_system()
        .openpty(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| pty_err("cannot open pty", &e))?;

    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    if std::path::Path::new(cwd).is_dir() {
        cmd.cwd(cwd);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let child = pair.slave.spawn_command(cmd).map_err(|e| pty_err("cannot start shell", &e))?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| pty_err("cannot read pty", &e))?;
    let writer = pair.master.take_writer().map_err(|e| pty_err("cannot write pty", &e))?;
    let scrollback = Arc::new(StdMutex::new(Scrollback::default()));

    let sb = scrollback.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut offset = 0;
                    if let Ok(mut sb) = sb.lock() {
                        offset = sb.total;
                        sb.total += n as u64;
                        sb.buf.extend_from_slice(&buf[..n]);
                        if sb.buf.len() > SCROLLBACK_MAX {
                            let excess = sb.buf.len() - SCROLLBACK_MAX;
                            sb.buf.drain(..excess);
                        }
                    }
                    on_output(&buf[..n], offset);
                }
            }
        }
        on_exit();
    });

    Ok(PtyHandle { master: pair.master, writer, child, scrollback })
}

#[derive(Clone, Serialize)]
struct TerminalOutputEvent {
    session_id: String,
    /// Base64 of the raw bytes (keeps multi-byte UTF-8 sequences intact across chunks).
    data: String,
    /// Total bytes the session had produced before this chunk.
    offset: u64,
}

#[derive(Clone, Serialize)]
pub struct TerminalScrollback {
    /// Base64 of the buffered output.
    data: String,
    /// Total bytes produced so far; live events with `offset < end_offset` are already included.
    end_offset: u64,
}

#[derive(Clone, Serialize)]
struct TerminalExitEvent {
    session_id: String,
}

fn spawn_for_app(
    app: &tauri::AppHandle,
    session_id: &str,
    program: &str,
    args: &[String],
    cwd: &str,
    cols: u16,
    rows: u16,
) -> Result<PtyHandle, AppError> {
    let (a1, a2) = (app.clone(), app.clone());
    let (id1, id2) = (session_id.to_string(), session_id.to_string());
    spawn_pty(
        program,
        args,
        cwd,
        cols,
        rows,
        move |bytes, offset| {
            let _ = a1.emit(
                "terminal-output",
                TerminalOutputEvent {
                    session_id: id1.clone(),
                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                    offset,
                },
            );
        },
        move || {
            let _ = a2.emit("terminal-exit", TerminalExitEvent { session_id: id2 });
        },
    )
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            ptys: Arc::new(StdMutex::new(HashMap::new())),
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

/// Create a new local terminal session backed by a real PTY.
#[tauri::command]
pub async fn terminal_create_local(
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    app: tauri::AppHandle,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSession, AppError> {
    let shell = TerminalManager::default_shell();
    let working_dir = cwd.unwrap_or_else(TerminalManager::default_cwd);
    let (cols, rows) = (cols.unwrap_or(80), rows.unwrap_or(24));

    let session = TerminalSession {
        id: Uuid::new_v4().to_string(),
        title: format!("Terminal - {}", shell.rsplit(['/', '\\']).next().unwrap_or(&shell)),
        session_type: TerminalType::Local,
        shell: shell.clone(),
        cwd: working_dir.clone(),
        created_at: Utc::now(),
        active: true,
        cols,
        rows,
    };

    #[cfg(unix)]
    let args = vec!["-l".to_string()];
    #[cfg(not(unix))]
    let args: Vec<String> = Vec::new();
    let handle = spawn_for_app(&app, &session.id, &shell, &args, &working_dir, cols, rows)?;
    manager.ptys.lock().map_err(lock_err)?.insert(session.id.clone(), handle);
    manager.sessions.write().await.insert(session.id.clone(), session.clone());

    tracing::info!("Created local terminal session {} with shell {}", session.id, shell);
    Ok(session)
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> AppError {
    AppError::State {
        message: "Terminal state is unavailable".to_string(),
        advice: "Restart the application.".to_string(),
    }
}

/// Reject host/user values that ssh would parse as options or that contain
/// characters outside the set valid in hostnames, IPs and unix user names.
fn validate_ssh_token(kind: &str, value: &str, allow_colon: bool) -> Result<(), AppError> {
    let ok = !value.is_empty()
        && !value.starts_with('-')
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@') || (allow_colon && c == ':')
        });
    if ok {
        Ok(())
    } else {
        Err(AppError::Configuration {
            message: format!("Invalid SSH {kind}: '{value}'"),
            advice: format!("Use only letters, digits, '.', '_' and '-' in the {kind}."),
        })
    }
}

/// Create a new remote terminal session by running the system `ssh` client on
/// a PTY, so keys, agents, known_hosts and password prompts all behave as usual.
#[tauri::command]
// Each IPC argument is a named parameter of the command.
#[allow(clippy::too_many_arguments)]
pub async fn terminal_create_remote(
    host: String,
    port: u16,
    username: String,
    connection_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    app: tauri::AppHandle,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalSession, AppError> {
    validate_ssh_token("host", &host, true)?;
    validate_ssh_token("username", &username, false)?;
    let _ = connection_id;
    let (cols, rows) = (cols.unwrap_or(80), rows.unwrap_or(24));

    let session = TerminalSession {
        id: Uuid::new_v4().to_string(),
        title: format!("SSH: {}@{}", username, host),
        session_type: TerminalType::Remote,
        shell: format!("ssh {}@{}:{}", username, host, port),
        cwd: format!("{}@{}:{}", username, host, port),
        created_at: Utc::now(),
        active: true,
        cols,
        rows,
    };

    let args = vec![
        "-p".to_string(),
        port.to_string(),
        "-o".to_string(),
        "ServerAliveInterval=30".to_string(),
        format!("{username}@{host}"),
    ];
    let cwd = TerminalManager::default_cwd();
    let handle = spawn_for_app(&app, &session.id, "ssh", &args, &cwd, cols, rows)?;
    manager.ptys.lock().map_err(lock_err)?.insert(session.id.clone(), handle);
    manager.sessions.write().await.insert(session.id.clone(), session.clone());

    tracing::info!("Created remote terminal session {} for {}@{}:{}", session.id, username, host, port);
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

/// Close a terminal session, killing its shell.
#[tauri::command]
pub async fn terminal_close_session(
    session_id: String,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let mut sessions = manager.sessions.write().await;
    if sessions.remove(&session_id).is_some() {
        manager.ptys.lock().map_err(lock_err)?.remove(&session_id);
        tracing::info!("Closed terminal session {}", session_id);
        Ok(())
    } else {
        Err(AppError::State {
            message: format!("Terminal session '{}' not found", session_id),
            advice: "The session may have already been closed.".to_string(),
        })
    }
}

fn missing_session(session_id: &str) -> AppError {
    AppError::State {
        message: format!("Terminal session '{}' not found", session_id),
        advice: "Create a new terminal session.".to_string(),
    }
}

/// Send input (keystrokes / pasted text) to a terminal session's PTY.
#[tauri::command]
pub async fn terminal_write(
    session_id: String,
    data: String,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let mut ptys = manager.ptys.lock().map_err(lock_err)?;
    let pty = ptys.get_mut(&session_id).ok_or_else(|| missing_session(&session_id))?;
    pty.writer
        .write_all(data.as_bytes())
        .and_then(|_| pty.writer.flush())
        .map_err(|e| AppError::State {
            message: format!("Could not write to terminal: {e}"),
            advice: "The shell may have exited. Open a new terminal.".to_string(),
        })
}

/// Resize a terminal session's PTY (sends SIGWINCH to the foreground process).
#[tauri::command]
pub async fn terminal_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<(), AppError> {
    let mut sessions = manager.sessions.write().await;
    let session = sessions.get_mut(&session_id).ok_or_else(|| missing_session(&session_id))?;
    session.cols = cols;
    session.rows = rows;
    let ptys = manager.ptys.lock().map_err(lock_err)?;
    if let Some(pty) = ptys.get(&session_id) {
        pty.master
            .resize(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
            .map_err(|e| AppError::State {
                message: format!("Could not resize terminal: {e}"),
                advice: "Try again.".to_string(),
            })?;
    }
    Ok(())
}

/// Buffered output of a session (base64) so a re-mounted view can restore its screen.
#[tauri::command]
pub async fn terminal_read_scrollback(
    session_id: String,
    manager: tauri::State<'_, TerminalManager>,
) -> Result<TerminalScrollback, AppError> {
    let ptys = manager.ptys.lock().map_err(lock_err)?;
    let pty = ptys.get(&session_id).ok_or_else(|| missing_session(&session_id))?;
    let sb = pty.scrollback.lock().map_err(lock_err)?;
    Ok(TerminalScrollback {
        data: base64::engine::general_purpose::STANDARD.encode(&sb.buf),
        end_offset: sb.total,
    })
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

    #[cfg(unix)]
    #[test]
    fn pty_runs_a_command_and_reports_exit() {
        use std::sync::mpsc;
        use std::time::Duration;
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let handle = spawn_pty(
            "/bin/sh",
            &["-c".to_string(), "echo UFOP_PTY_OK".to_string()],
            "/",
            80,
            24,
            move |b, _| {
                let _ = tx.send(b.to_vec());
            },
            move || {
                let _ = done_tx.send(());
            },
        )
        .unwrap();
        done_rx.recv_timeout(Duration::from_secs(10)).expect("pty should report exit");
        let out: Vec<u8> = rx.try_iter().flatten().collect();
        assert!(String::from_utf8_lossy(&out).contains("UFOP_PTY_OK"));
        assert!(String::from_utf8_lossy(&handle.scrollback.lock().unwrap().buf).contains("UFOP_PTY_OK"));
    }

    #[cfg(unix)]
    #[test]
    fn pty_input_reaches_the_shell_and_resize_works() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut handle = spawn_pty("/bin/sh", &[], "/", 80, 24, move |b, _| { let _ = tx.send(b.to_vec()); }, || {}).unwrap();
        handle.master.resize(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 }).unwrap();
        handle.writer.write_all(b"echo UFOP_$((20+22))\n").unwrap();
        handle.writer.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = String::new();
        while Instant::now() < deadline && !seen.contains("UFOP_42") {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                seen.push_str(&String::from_utf8_lossy(&chunk));
            }
        }
        assert!(seen.contains("UFOP_42"), "shell output was: {seen:?}");
    }

    #[test]
    fn ssh_tokens_reject_option_injection() {
        assert!(validate_ssh_token("host", "example.com", true).is_ok());
        assert!(validate_ssh_token("host", "::1", true).is_ok());
        assert!(validate_ssh_token("host", "-oProxyCommand=evil", true).is_err());
        assert!(validate_ssh_token("username", "bob;rm", false).is_err());
        assert!(validate_ssh_token("username", "", false).is_err());
    }
}
