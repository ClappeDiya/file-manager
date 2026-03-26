//! External Editor Commands
//!
//! Features:
//! - File-type-to-app mapping table (CRUD)
//! - "Open With..." system app listing
//! - Watch temp file for changes and re-upload to remote
//! - Configurable double-click behavior

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Types ──

/// An editor mapping: file extension -> application path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorMapping {
    pub id: String,
    /// File extension (e.g., "py", "js", "txt") or "*" for default.
    pub extension: String,
    /// Application name for display.
    pub app_name: String,
    /// Full path to the application executable.
    pub app_path: String,
    /// Optional CLI arguments (e.g., "--wait" for VS Code).
    pub args: Vec<String>,
}

/// What happens on double-click.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DoubleClickBehavior {
    /// Open in system default app.
    #[default]
    Open,
    /// Open in mapped external editor.
    Edit,
    /// Show in preview pane.
    Preview,
    /// Start transfer (download/upload).
    Transfer,
}

/// Result of opening a file in an external editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenResult {
    pub success: bool,
    pub temp_path: Option<String>,
    pub watching: bool,
}

/// Status of a watched file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStatus {
    pub temp_path: String,
    pub remote_path: String,
    pub protocol: String,
    pub connection_id: String,
    pub last_modified: Option<String>,
    pub upload_count: u32,
    pub active: bool,
}

/// In-memory store for editor mappings and watch state.
pub struct EditorState {
    mappings: RwLock<Vec<EditorMapping>>,
    double_click: RwLock<DoubleClickBehavior>,
    watched_files: RwLock<HashMap<String, WatchEntry>>,
}

struct WatchEntry {
    temp_path: PathBuf,
    remote_path: String,
    protocol: String,
    connection_id: String,
    last_modified: Option<std::time::SystemTime>,
    upload_count: u32,
    active: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            mappings: RwLock::new(Vec::new()),
            double_click: RwLock::new(DoubleClickBehavior::default()),
            watched_files: RwLock::new(HashMap::new()),
        }
    }
}

// ── Commands ──

/// List all editor mappings.
#[tauri::command]
pub async fn list_editor_mappings(
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<Vec<EditorMapping>, AppError> {
    Ok(state.mappings.read().await.clone())
}

/// Save or update an editor mapping.
#[tauri::command]
pub async fn save_editor_mapping(
    mapping: EditorMapping,
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<(), AppError> {
    let mut mappings = state.mappings.write().await;
    if let Some(existing) = mappings.iter_mut().find(|m| m.id == mapping.id) {
        *existing = mapping;
    } else {
        mappings.push(mapping);
    }
    Ok(())
}

/// Delete an editor mapping by ID.
#[tauri::command]
pub async fn delete_editor_mapping(
    id: String,
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<(), AppError> {
    let mut mappings = state.mappings.write().await;
    mappings.retain(|m| m.id != id);
    Ok(())
}

/// Get the configured double-click behavior.
#[tauri::command]
pub async fn get_double_click_behavior(
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<DoubleClickBehavior, AppError> {
    Ok(state.double_click.read().await.clone())
}

/// Set the double-click behavior.
#[tauri::command]
pub async fn set_double_click_behavior(
    behavior: DoubleClickBehavior,
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<(), AppError> {
    *state.double_click.write().await = behavior;
    Ok(())
}

/// Find the best editor for a given file extension.
#[tauri::command]
pub async fn resolve_editor(
    extension: String,
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<Option<EditorMapping>, AppError> {
    let mappings = state.mappings.read().await;
    // First try exact extension match, then wildcard
    let result = mappings
        .iter()
        .find(|m| m.extension.eq_ignore_ascii_case(&extension))
        .or_else(|| mappings.iter().find(|m| m.extension == "*"))
        .cloned();
    Ok(result)
}

/// Open a local file in an external editor.
/// If the file is remote, `temp_path` should be provided (pre-downloaded).
#[tauri::command]
pub async fn open_in_editor(
    file_path: String,
    editor_path: Option<String>,
    editor_args: Option<Vec<String>>,
    #[allow(unused_variables)]
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<OpenResult, AppError> {
    let path = Path::new(&file_path);
    if !path.exists() {
        return Err(AppError::file_op(
            format!("File not found: {file_path}"),
            "The file may have been moved or deleted.",
        ));
    }

    let args = editor_args.unwrap_or_default();

    if let Some(editor) = editor_path {
        // Validate that the editor path exists and is executable
        let editor_p = Path::new(&editor);
        if !editor_p.exists() {
            return Err(AppError::file_op(
                format!("Editor not found: {editor}"),
                "Check that the editor path is correct and the application is installed.",
            ));
        }
        if !editor_p.is_file() {
            return Err(AppError::file_op(
                format!("Editor path is not a file: {editor}"),
                "Provide the path to an executable file, not a directory.",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&editor)
                .map_err(|e| AppError::file_op(
                    format!("Cannot read editor metadata: {e}"),
                    "Check file permissions.",
                ))?
                .permissions();
            if perms.mode() & 0o111 == 0 {
                return Err(AppError::file_op(
                    format!("Editor is not executable: {editor}"),
                    "Set the executable bit: chmod +x on the editor binary.",
                ));
            }
        }

        // Open with specified editor
        let mut cmd = std::process::Command::new(&editor);
        for arg in &args {
            cmd.arg(arg);
        }
        cmd.arg(&file_path);
        cmd.spawn().map_err(|e| {
            AppError::file_op(
                format!("Failed to launch editor '{}': {e}", editor),
                "Check that the editor path is correct and the application is installed.",
            )
        })?;
    } else {
        // Open with system default
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(&file_path)
                .spawn()
                .map_err(|e| AppError::file_op(format!("Failed to open: {e}"), "Check file permissions."))?;
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(&file_path)
                .spawn()
                .map_err(|e| AppError::file_op(format!("Failed to open: {e}"), "Check that xdg-open is available."))?;
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &file_path])
                .spawn()
                .map_err(|e| AppError::file_op(format!("Failed to open: {e}"), "Check file associations."))?;
        }
    }

    Ok(OpenResult {
        success: true,
        temp_path: Some(file_path),
        watching: false,
    })
}

/// Start watching a temp file for changes. When the user saves in their editor,
/// this detects the modification and signals a re-upload.
#[tauri::command]
pub async fn start_file_watch(
    temp_path: String,
    remote_path: String,
    protocol: String,
    connection_id: String,
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<(), AppError> {
    let path = PathBuf::from(&temp_path);
    if !path.exists() {
        return Err(AppError::file_op(
            format!("Temp file not found: {temp_path}"),
            "The temporary file may have been deleted.",
        ));
    }

    let last_modified = path.metadata().ok().and_then(|m| m.modified().ok());

    let entry = WatchEntry {
        temp_path: path,
        remote_path,
        protocol,
        connection_id,
        last_modified,
        upload_count: 0,
        active: true,
    };

    state.watched_files.write().await.insert(temp_path, entry);
    Ok(())
}

/// Stop watching a temp file.
#[tauri::command]
pub async fn stop_file_watch(
    temp_path: String,
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<(), AppError> {
    let mut watches = state.watched_files.write().await;
    if let Some(entry) = watches.get_mut(&temp_path) {
        entry.active = false;
    }
    Ok(())
}

/// Check all watched files for modifications. Returns paths that changed.
#[tauri::command]
pub async fn check_watched_files(
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<Vec<WatchStatus>, AppError> {
    let mut watches = state.watched_files.write().await;
    let mut changed = Vec::new();

    for (key, entry) in watches.iter_mut() {
        if !entry.active {
            continue;
        }

        if let Ok(metadata) = entry.temp_path.metadata() {
            if let Ok(modified) = metadata.modified() {
                let file_changed = entry
                    .last_modified
                    .map(|prev| modified > prev)
                    .unwrap_or(true);

                if file_changed {
                    entry.last_modified = Some(modified);
                    entry.upload_count += 1;
                    changed.push(WatchStatus {
                        temp_path: key.clone(),
                        remote_path: entry.remote_path.clone(),
                        protocol: entry.protocol.clone(),
                        connection_id: entry.connection_id.clone(),
                        last_modified: Some(
                            chrono::DateTime::<chrono::Utc>::from(modified)
                                .to_rfc3339(),
                        ),
                        upload_count: entry.upload_count,
                        active: entry.active,
                    });
                }
            }
        }
    }

    Ok(changed)
}

/// List all currently watched files.
#[tauri::command]
pub async fn list_watched_files(
    state: tauri::State<'_, Arc<EditorState>>,
) -> Result<Vec<WatchStatus>, AppError> {
    let watches = state.watched_files.read().await;
    Ok(watches
        .iter()
        .map(|(key, entry)| WatchStatus {
            temp_path: key.clone(),
            remote_path: entry.remote_path.clone(),
            protocol: entry.protocol.clone(),
            connection_id: entry.connection_id.clone(),
            last_modified: entry.last_modified.map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()
            }),
            upload_count: entry.upload_count,
            active: entry.active,
        })
        .collect())
}
