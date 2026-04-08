use crate::core::error::AppError;
use crate::core::types::FileEntry;
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

/// DRY helper: record a successful filesystem operation in the unified ledger.
///
/// Fail-open by construction — `OperationLedger::record` swallows DB errors and
/// only logs them at warn level. Callers do not need to handle failure.
///
/// Records a single subject/target pair for batches; for multi-source ops we
/// emit one record per pair so the timeline / undo views can show each item.
async fn record_fs_ok(
    ledger: &OperationLedger,
    kind: &str,
    sources: &[String],
    dests: &[String],
    correlation_id: &str,
) {
    // Pair sources to dests where possible; otherwise record each side alone.
    let pair_count = sources.len().max(dests.len()).max(1);
    for i in 0..pair_count {
        let subject = sources.get(i).cloned();
        let target = dests.get(i).cloned();
        let summary = match (&subject, &target) {
            (Some(s), Some(t)) => format!("{kind}: {s} → {t}"),
            (Some(s), None) => format!("{kind}: {s}"),
            (None, Some(t)) => format!("{kind}: {t}"),
            (None, None) => kind.to_string(),
        };
        let mut ev = RecordEvent::new(LedgerEngine::Fs, kind)
            .status(LedgerStatus::Ok)
            .summary(summary)
            .correlation(correlation_id);
        if let Some(s) = subject {
            ev = ev.subject(s);
        }
        if let Some(t) = target {
            ev = ev.target(t);
        }
        ledger.record(ev).await;
    }
}

/// Result of a file operation, used for undo support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOpResult {
    pub success: bool,
    pub operation: String,
    pub source_paths: Vec<String>,
    pub dest_paths: Vec<String>,
    pub undo_id: String,
}

/// Progress update for long-running operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOpProgress {
    pub operation_id: String,
    pub total_items: u64,
    pub completed_items: u64,
    pub current_item: String,
    pub bytes_total: u64,
    pub bytes_completed: u64,
}

/// Copy files/directories to a destination.
/// Supports multiple source paths for multi-selection.
#[tauri::command]
pub async fn copy_files(
    source_paths: Vec<String>,
    dest_dir: String,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    copy_files_impl(source_paths, dest_dir, &ledger).await
}

pub async fn copy_files_impl(
    source_paths: Vec<String>,
    dest_dir: String,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    let dest_path = Path::new(&dest_dir);
    if !dest_path.exists() {
        return Err(AppError::file_op(
            format!("Destination directory does not exist: {dest_dir}"),
            "Check that the destination folder exists.",
        ));
    }

    let mut dest_paths = Vec::new();

    for source in &source_paths {
        let src = Path::new(source);
        if !src.exists() {
            return Err(AppError::file_op(
                format!("Source not found: {source}"),
                "The file or folder may have been moved or deleted.",
            ));
        }

        let file_name = src
            .file_name()
            .ok_or_else(|| AppError::file_op("Invalid source path", "Check the file path."))?;
        let mut target = dest_path.join(file_name);

        // Handle name conflicts by appending " (copy)" or " (copy N)"
        if target.exists() {
            let stem = target
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let ext = target
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();

            let mut counter = 1;
            loop {
                let new_name = if counter == 1 {
                    format!("{stem} (copy){ext}")
                } else {
                    format!("{stem} (copy {counter}){ext}")
                };
                target = dest_path.join(&new_name);
                if !target.exists() {
                    break;
                }
                counter += 1;
            }
        }

        if src.is_dir() {
            copy_dir_recursive(src, &target)?;
        } else {
            std::fs::copy(src, &target).map_err(|e| {
                AppError::file_op(
                    format!("Failed to copy {}: {e}", src.display()),
                    "Check disk space and permissions.",
                )
            })?;
        }

        dest_paths.push(target.to_string_lossy().to_string());
    }

    let undo_id = Uuid::new_v4().to_string();
    record_fs_ok(ledger, "copy", &source_paths, &dest_paths, &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: "copy".to_string(),
        source_paths,
        dest_paths,
        undo_id,
    })
}

/// Move files/directories to a destination.
#[tauri::command]
pub async fn move_files(
    source_paths: Vec<String>,
    dest_dir: String,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    move_files_impl(source_paths, dest_dir, &ledger).await
}

pub async fn move_files_impl(
    source_paths: Vec<String>,
    dest_dir: String,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    let dest_path = Path::new(&dest_dir);
    if !dest_path.exists() {
        return Err(AppError::file_op(
            format!("Destination directory does not exist: {dest_dir}"),
            "Check that the destination folder exists.",
        ));
    }

    let mut dest_paths = Vec::new();

    for source in &source_paths {
        let src = Path::new(source);
        if !src.exists() {
            return Err(AppError::file_op(
                format!("Source not found: {source}"),
                "The file or folder may have been moved or deleted.",
            ));
        }

        let file_name = src
            .file_name()
            .ok_or_else(|| AppError::file_op("Invalid source path", "Check the file path."))?;
        let target = dest_path.join(file_name);

        if target.exists() {
            return Err(AppError::file_op(
                format!("File already exists at destination: {}", target.display()),
                "Rename the file or choose a different destination.",
            ));
        }

        std::fs::rename(src, &target).map_err(|e| {
            AppError::file_op(
                format!("Failed to move {}: {e}", src.display()),
                "Check permissions. If moving across drives, this may require a copy + delete.",
            )
        })?;

        dest_paths.push(target.to_string_lossy().to_string());
    }

    let undo_id = Uuid::new_v4().to_string();
    record_fs_ok(ledger, "move", &source_paths, &dest_paths, &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: "move".to_string(),
        source_paths,
        dest_paths,
        undo_id,
    })
}

/// Rename a file or directory.
#[tauri::command]
pub async fn rename_file(
    path: String,
    new_name: String,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    rename_file_impl(path, new_name, &ledger).await
}

pub async fn rename_file_impl(
    path: String,
    new_name: String,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    let src = Path::new(&path);
    if !src.exists() {
        return Err(AppError::file_op(
            format!("Not found: {path}"),
            "The file or folder may have been moved or deleted.",
        ));
    }

    // Validate new name
    if new_name.is_empty() || new_name.contains('/') || new_name.contains('\\') {
        return Err(AppError::file_op(
            "Invalid file name",
            "File names cannot be empty or contain path separators.",
        ));
    }

    let parent = src.parent().ok_or_else(|| {
        AppError::file_op("Cannot rename root", "Cannot rename the root directory.")
    })?;
    let target = parent.join(&new_name);

    if target.exists() {
        return Err(AppError::file_op(
            format!("A file named '{new_name}' already exists"),
            "Choose a different name.",
        ));
    }

    std::fs::rename(src, &target).map_err(|e| {
        AppError::file_op(
            format!("Failed to rename: {e}"),
            "Check file permissions.",
        )
    })?;

    let undo_id = Uuid::new_v4().to_string();
    let source_paths = vec![path];
    let dest_paths = vec![target.to_string_lossy().to_string()];
    record_fs_ok(ledger, "rename", &source_paths, &dest_paths, &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: "rename".to_string(),
        source_paths,
        dest_paths,
        undo_id,
    })
}

/// Duplicate files (copy to same directory with " (copy)" suffix).
#[tauri::command]
pub async fn duplicate_files(
    paths: Vec<String>,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    duplicate_files_impl(paths, &ledger).await
}

pub async fn duplicate_files_impl(
    paths: Vec<String>,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    let mut dest_paths = Vec::new();

    for path in &paths {
        let src = Path::new(path);
        if !src.exists() {
            return Err(AppError::file_op(
                format!("Not found: {path}"),
                "The file or folder may have been moved or deleted.",
            ));
        }

        let parent = src.parent().ok_or_else(|| {
            AppError::file_op("Cannot duplicate root", "Cannot duplicate the root directory.")
        })?;

        let stem = src
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let ext = src
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();

        let mut counter = 1;
        let target;
        loop {
            let new_name = if counter == 1 {
                format!("{stem} (copy){ext}")
            } else {
                format!("{stem} (copy {counter}){ext}")
            };
            let candidate = parent.join(&new_name);
            if !candidate.exists() {
                target = candidate;
                break;
            }
            counter += 1;
            if counter > 1000 {
                return Err(AppError::file_op(
                    "Too many copies",
                    "Clean up duplicate files first.",
                ));
            }
        }

        if src.is_dir() {
            copy_dir_recursive(src, &target)?;
        } else {
            std::fs::copy(src, &target).map_err(|e| {
                AppError::file_op(
                    format!("Failed to duplicate: {e}"),
                    "Check disk space and permissions.",
                )
            })?;
        }

        dest_paths.push(target.to_string_lossy().to_string());
    }

    let undo_id = Uuid::new_v4().to_string();
    record_fs_ok(ledger, "duplicate", &paths, &dest_paths, &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: "duplicate".to_string(),
        source_paths: paths,
        dest_paths,
        undo_id,
    })
}

/// Delete files/directories. Uses OS trash by default.
/// Set `permanent` to true for permanent deletion (Shift+Delete).
#[tauri::command]
pub async fn delete_files(
    paths: Vec<String>,
    permanent: bool,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    delete_files_impl(paths, permanent, &ledger).await
}

pub async fn delete_files_impl(
    paths: Vec<String>,
    permanent: bool,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    for path in &paths {
        let p = Path::new(path);
        if !p.exists() {
            continue; // Already gone, skip silently
        }

        if permanent {
            if p.is_dir() {
                std::fs::remove_dir_all(p).map_err(|e| {
                    AppError::file_op(
                        format!("Failed to delete directory {}: {e}", p.display()),
                        "Check that the directory is not in use and you have permission.",
                    )
                })?;
            } else {
                std::fs::remove_file(p).map_err(|e| {
                    AppError::file_op(
                        format!("Failed to delete file {}: {e}", p.display()),
                        "Check that the file is not in use and you have permission.",
                    )
                })?;
            }
        } else {
            // Move to OS trash
            // On macOS, move to ~/.Trash
            // On Linux, use freedesktop trash spec
            // On Windows, use shell API
            move_to_trash(p)?;
        }
    }

    let undo_id = Uuid::new_v4().to_string();
    let kind = if permanent { "delete_permanent" } else { "delete_trash" };
    record_fs_ok(ledger, kind, &paths, &[], &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: kind.to_string(),
        source_paths: paths,
        dest_paths: vec![],
        undo_id,
    })
}

/// Create a new directory.
#[tauri::command]
pub async fn create_directory(
    path: String,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    create_directory_impl(path, &ledger).await
}

pub async fn create_directory_impl(
    path: String,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(AppError::file_op(
            format!("Already exists: {path}"),
            "Choose a different name.",
        ));
    }

    std::fs::create_dir_all(p).map_err(|e| {
        AppError::file_op(
            format!("Failed to create directory: {e}"),
            "Check that you have write permission to the parent folder.",
        )
    })?;

    let undo_id = Uuid::new_v4().to_string();
    let dest_paths = vec![path];
    record_fs_ok(ledger, "create_folder", &[], &dest_paths, &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: "create_folder".to_string(),
        source_paths: vec![],
        dest_paths,
        undo_id,
    })
}

/// Create a new empty file.
#[tauri::command]
pub async fn create_file(
    path: String,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<FileOpResult, AppError> {
    create_file_impl(path, &ledger).await
}

pub async fn create_file_impl(
    path: String,
    ledger: &OperationLedger,
) -> Result<FileOpResult, AppError> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(AppError::file_op(
            format!("Already exists: {path}"),
            "Choose a different name.",
        ));
    }

    // Ensure parent directory exists
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::file_op(
                    format!("Cannot create parent directory: {e}"),
                    "Check write permissions.",
                )
            })?;
        }
    }

    std::fs::File::create(p).map_err(|e| {
        AppError::file_op(
            format!("Failed to create file: {e}"),
            "Check disk space and permissions.",
        )
    })?;

    let undo_id = Uuid::new_v4().to_string();
    let dest_paths = vec![path];
    record_fs_ok(ledger, "create_file", &[], &dest_paths, &undo_id).await;

    Ok(FileOpResult {
        success: true,
        operation: "create_file".to_string(),
        source_paths: vec![],
        dest_paths,
        undo_id,
    })
}

/// Undo a previous file operation by reversing it.
#[tauri::command]
pub async fn undo_file_operation(
    operation: String,
    source_paths: Vec<String>,
    dest_paths: Vec<String>,
) -> Result<bool, AppError> {
    match operation.as_str() {
        "copy" | "duplicate" => {
            // Undo copy/duplicate = delete the copies
            for dest in &dest_paths {
                let p = Path::new(dest);
                if p.exists() {
                    if p.is_dir() {
                        std::fs::remove_dir_all(p).map_err(|e| {
                            AppError::file_op(format!("Undo failed: {e}"), "Manual cleanup may be needed.")
                        })?;
                    } else {
                        std::fs::remove_file(p).map_err(|e| {
                            AppError::file_op(format!("Undo failed: {e}"), "Manual cleanup may be needed.")
                        })?;
                    }
                }
            }
        }
        "move" | "rename" => {
            // Undo move/rename = move back
            for (dest, src) in dest_paths.iter().zip(source_paths.iter()) {
                let d = Path::new(dest);
                if d.exists() {
                    std::fs::rename(d, src).map_err(|e| {
                        AppError::file_op(format!("Undo failed: {e}"), "Manual restoration may be needed.")
                    })?;
                }
            }
        }
        "create_folder" | "create_file" => {
            // Undo create = delete
            for dest in &dest_paths {
                let p = Path::new(dest);
                if p.exists() {
                    if p.is_dir() {
                        std::fs::remove_dir(p).map_err(|e| {
                            AppError::file_op(
                                format!("Undo failed: {e}"),
                                "The folder may not be empty. Manual cleanup may be needed.",
                            )
                        })?;
                    } else {
                        std::fs::remove_file(p).map_err(|e| {
                            AppError::file_op(format!("Undo failed: {e}"), "Manual cleanup may be needed.")
                        })?;
                    }
                }
            }
        }
        _ => {
            return Err(AppError::file_op(
                format!("Cannot undo operation: {operation}"),
                "This operation type does not support undo.",
            ));
        }
    }

    Ok(true)
}

/// Get file/directory metadata for a single path.
#[tauri::command]
pub async fn get_file_metadata(path: String) -> Result<FileEntry, AppError> {
    crate::fs_engine::get_metadata_pub(&path)
}

// ──────────────────────────────────────────────
// Cloud trash-aware deletion commands
// ──────────────────────────────────────────────

/// Move a cloud file/folder to the provider's trash/recycle bin.
///
/// Behaviour per provider:
/// - **google_drive**: Sets `trashed: true` via PATCH (Google Drive Trash).
/// - **onedrive**: DELETE `/me/drive/items/{id}` which moves to OneDrive Recycle Bin.
/// - **dropbox**: `files/delete_v2` which soft-deletes (recoverable for retention period).
#[tauri::command]
pub async fn cloud_delete_to_trash(
    protocol: String,
    file_id: String,
    connection_id: String,
) -> Result<(), AppError> {
    tracing::info!(
        "cloud_delete_to_trash: protocol={}, file_id={}, connection_id={}",
        protocol, file_id, connection_id
    );

    match protocol.as_str() {
        "google_drive" => {
            // Google Drive: move to trash (permanent=false)
            let connector = crate::connectors::google_drive::GoogleDriveConnector::new();
            connector.delete_file(&file_id, false).await
        }
        "onedrive" => {
            // OneDrive: DELETE moves to recycle bin by default
            let connector = crate::connectors::onedrive::OneDriveConnector::new();
            connector.delete_item(&file_id).await
        }
        "dropbox" => {
            // Dropbox: delete_v2 is a soft delete (recoverable)
            let connector = crate::connectors::dropbox::DropboxConnector::new();
            connector.delete(&file_id).await
        }
        other => Err(AppError::FileOperation {
            message: format!("Unsupported cloud protocol for trash deletion: {other}"),
            advice: "Supported protocols: google_drive, onedrive, dropbox.".to_string(),
        }),
    }
}

/// Permanently delete a cloud file/folder, bypassing the trash/recycle bin.
///
/// Behaviour per provider:
/// - **google_drive**: HTTP DELETE on the file (permanent removal, no recovery).
/// - **onedrive**: DELETE + permanentDelete (removes from recycle bin too).
/// - **dropbox**: `files/permanently_delete` endpoint (immediate permanent removal).
#[tauri::command]
pub async fn cloud_delete_permanently(
    protocol: String,
    file_id: String,
    connection_id: String,
) -> Result<(), AppError> {
    tracing::info!(
        "cloud_delete_permanently: protocol={}, file_id={}, connection_id={}",
        protocol, file_id, connection_id
    );

    match protocol.as_str() {
        "google_drive" => {
            // Google Drive: permanent delete (permanent=true)
            let connector = crate::connectors::google_drive::GoogleDriveConnector::new();
            connector.delete_file(&file_id, true).await
        }
        "onedrive" => {
            // OneDrive: delete + purge from recycle bin
            let connector = crate::connectors::onedrive::OneDriveConnector::new();
            connector.delete_item_permanently(&file_id).await
        }
        "dropbox" => {
            // Dropbox: permanent deletion endpoint
            let connector = crate::connectors::dropbox::DropboxConnector::new();
            connector.permanently_delete(&file_id).await
        }
        other => Err(AppError::FileOperation {
            message: format!("Unsupported cloud protocol for permanent deletion: {other}"),
            advice: "Supported protocols: google_drive, onedrive, dropbox.".to_string(),
        }),
    }
}

// ──────────────────────────────────────────────
// Remote permissions and ownership commands
// ──────────────────────────────────────────────

/// Set file permissions (chmod) on a remote SFTP path.
///
/// `mode` is an octal string like "755" or "644".
/// `recursive` applies the mode to all children if the path is a directory.
#[tauri::command]
pub async fn set_permissions(
    protocol: String,
    path: String,
    mode: String,
    recursive: bool,
    sftp: tauri::State<'_, Arc<crate::connectors::sftp::SftpConnector>>,
    ftp: tauri::State<'_, Arc<crate::connectors::ftp::FtpConnector>>,
) -> Result<(), AppError> {
    // Parse octal mode string
    let mode_val = u32::from_str_radix(mode.trim(), 8).map_err(|_| {
        AppError::file_op(
            format!("Invalid permission mode: {mode}"),
            "Use an octal string like \"755\" or \"644\".",
        )
    })?;

    match protocol.as_str() {
        "sftp" => {
            sftp.chmod(&path, mode_val, recursive).await?;
        }
        "ftp" | "ftps" => {
            ftp.chmod(&path, &mode).await?;
            // FTP SITE CHMOD does not support recursive — would need client-side recursion
        }
        other => {
            return Err(AppError::file_op(
                format!("chmod not supported for protocol: {other}"),
                "Permissions can only be changed on SFTP and FTP connections.",
            ));
        }
    }

    tracing::info!("Set permissions {mode} on {path} (recursive={recursive})");
    Ok(())
}

/// Set file ownership (chown) on a remote SFTP path.
///
/// `uid` and `gid` are numeric Unix user/group IDs.
#[tauri::command]
pub async fn set_owner(
    path: String,
    uid: u32,
    gid: u32,
    sftp: tauri::State<'_, Arc<crate::connectors::sftp::SftpConnector>>,
) -> Result<(), AppError> {
    sftp.chown(&path, uid, gid).await?;
    tracing::info!("Set ownership {uid}:{gid} on {path}");
    Ok(())
}

/// Create a symbolic link on a remote SFTP server.
#[tauri::command]
pub async fn create_symlink(
    target: String,
    link_path: String,
    sftp: tauri::State<'_, Arc<crate::connectors::sftp::SftpConnector>>,
) -> Result<(), AppError> {
    sftp.create_symlink(&target, &link_path).await?;
    tracing::info!("Created symlink {link_path} -> {target}");
    Ok(())
}

/// Read the target of a symbolic link on a remote SFTP server.
#[tauri::command]
pub async fn read_symlink(
    path: String,
    sftp: tauri::State<'_, Arc<crate::connectors::sftp::SftpConnector>>,
) -> Result<String, AppError> {
    sftp.readlink(&path).await
}

/// Resolve a path to its real (absolute) path on a remote SFTP server.
#[tauri::command]
pub async fn resolve_path(
    path: String,
    sftp: tauri::State<'_, Arc<crate::connectors::sftp::SftpConnector>>,
) -> Result<String, AppError> {
    sftp.realpath(&path).await
}

/// Open a file with the OS default application.
#[tauri::command]
pub async fn open_file_with_default(path: String) -> Result<(), AppError> {
    let status = {
        #[cfg(target_os = "macos")]
        { std::process::Command::new("open").arg(&path).status() }
        #[cfg(target_os = "windows")]
        { std::process::Command::new("cmd").args(["/C", "start", "", &path]).status() }
        #[cfg(target_os = "linux")]
        { std::process::Command::new("xdg-open").arg(&path).status() }
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(AppError::file_op(
            format!("Default app exited with {s} for {path}"),
            "Check you have a default application set for this file type.",
        )),
        Err(e) => Err(AppError::file_op(
            format!("Failed to open {path}: {e}"),
            "Check the file exists and your OS default app handler is available.",
        )),
    }
}

// ──────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dst).map_err(|e| {
        AppError::file_op(
            format!("Cannot create directory {}: {e}", dst.display()),
            "Check disk space and permissions.",
        )
    })?;

    for entry in std::fs::read_dir(src).map_err(|e| {
        AppError::file_op(
            format!("Cannot read directory {}: {e}", src.display()),
            "Check read permissions.",
        )
    })? {
        let entry = entry.map_err(|e| {
            AppError::file_op(format!("Error reading entry: {e}"), "Check permissions.")
        })?;
        let target = dst.join(entry.file_name());

        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| {
                AppError::file_op(
                    format!("Failed to copy {}: {e}", entry.path().display()),
                    "Check disk space and permissions.",
                )
            })?;
        }
    }

    Ok(())
}

/// Move a file/directory to the OS trash.
fn move_to_trash(path: &Path) -> Result<(), AppError> {
    // Cross-platform trash implementation
    // On macOS: move to ~/.Trash/
    // On Linux: use ~/.local/share/Trash/ (freedesktop spec)
    // Fallback: use rename to a .trash directory

    #[cfg(target_os = "macos")]
    {
        let trash_dir = directories::UserDirs::new()
            .map(|d| d.home_dir().join(".Trash"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.trash"));

        if !trash_dir.exists() {
            std::fs::create_dir_all(&trash_dir).map_err(|e| {
                AppError::file_op(
                    format!("Cannot access Trash: {e}"),
                    "Try permanent delete instead.",
                )
            })?;
        }

        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut target = trash_dir.join(&file_name);

        // Handle name conflicts in trash
        if target.exists() {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let ext = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
            target = trash_dir.join(format!("{stem} {timestamp}{ext}"));
        }

        std::fs::rename(path, &target).map_err(|e| {
            AppError::file_op(
                format!("Failed to move to Trash: {e}"),
                "Try permanent delete instead.",
            )
        })?;
    }

    #[cfg(target_os = "linux")]
    {
        let trash_dir = directories::UserDirs::new()
            .map(|d| d.home_dir().join(".local/share/Trash/files"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/.trash"));

        if !trash_dir.exists() {
            std::fs::create_dir_all(&trash_dir).map_err(|e| {
                AppError::file_op(
                    format!("Cannot access Trash: {e}"),
                    "Try permanent delete instead.",
                )
            })?;
        }

        let file_name = path.file_name().unwrap_or_default();
        let target = trash_dir.join(file_name);

        std::fs::rename(path, &target).map_err(|e| {
            AppError::file_op(
                format!("Failed to move to Trash: {e}"),
                "Try permanent delete instead.",
            )
        })?;
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, fall back to permanent delete for now
        // (proper implementation would use shell COM API)
        if path.is_dir() {
            std::fs::remove_dir_all(path).map_err(|e| {
                AppError::file_op(
                    format!("Failed to delete: {e}"),
                    "Check permissions.",
                )
            })?;
        } else {
            std::fs::remove_file(path).map_err(|e| {
                AppError::file_op(
                    format!("Failed to delete: {e}"),
                    "Check permissions.",
                )
            })?;
        }
    }

    Ok(())
}

// ──────────────────────────────────────────────
// Copy Path / Copy URL helpers (Transmit parity)
// ──────────────────────────────────────────────

/// Result of a path copy operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyPathResult {
    pub path: String,
    pub variant: String,
}

/// Copy the absolute local/remote path of a file.
#[tauri::command]
pub async fn copy_path(path: String) -> Result<CopyPathResult, AppError> {
    Ok(CopyPathResult {
        path: path.clone(),
        variant: "absolute".to_string(),
    })
}

/// Copy the remote path (as displayed in the file browser) for a file.
#[tauri::command]
pub async fn copy_remote_path(
    path: String,
    remote_base: Option<String>,
) -> Result<CopyPathResult, AppError> {
    let remote = if let Some(base) = remote_base {
        path.strip_prefix(&base).unwrap_or(&path).to_string()
    } else {
        path.clone()
    };
    Ok(CopyPathResult {
        path: remote,
        variant: "remote".to_string(),
    })
}

/// Build a full protocol URL for a file (e.g., sftp://host/path).
#[tauri::command]
pub async fn copy_url(
    path: String,
    protocol: String,
    host: String,
    port: Option<u16>,
    username: Option<String>,
) -> Result<CopyPathResult, AppError> {
    let user_part = username
        .filter(|u| !u.is_empty())
        .map(|u| format!("{u}@"))
        .unwrap_or_default();

    let port_part = port
        .filter(|&p| !is_default_port(&protocol, p))
        .map(|p| format!(":{p}"))
        .unwrap_or_default();

    let scheme = match protocol.as_str() {
        "sftp" => "sftp",
        "ftp" => "ftp",
        "ftps" => "ftps",
        "webdav" => "https",
        "smb" => "smb",
        "nfs" => "nfs",
        "s3" => "s3",
        _ => &protocol,
    };

    let url = format!("{scheme}://{user_part}{host}{port_part}{path}");
    Ok(CopyPathResult {
        path: url,
        variant: "url".to_string(),
    })
}

/// Copy the relative path of a file (relative to a base directory).
#[tauri::command]
pub async fn copy_relative_path(
    path: String,
    base_path: String,
) -> Result<CopyPathResult, AppError> {
    let relative = path
        .strip_prefix(&base_path)
        .unwrap_or(&path)
        .trim_start_matches('/')
        .to_string();
    Ok(CopyPathResult {
        path: if relative.is_empty() { ".".to_string() } else { relative },
        variant: "relative".to_string(),
    })
}

fn is_default_port(protocol: &str, port: u16) -> bool {
    matches!(
        (protocol, port),
        ("sftp", 22) | ("ftp", 21) | ("ftps", 990) | ("webdav", 443)
        | ("smb", 445) | ("nfs", 2049) | ("s3", 443)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DbPool;
    use std::fs;
    use tempfile::tempdir;

    /// Build a fresh in-memory ledger for tests. The `operation_ledger` table
    /// does not need to exist for `record()` to be called — it's fail-open.
    /// We exercise the IPC command path, not the ledger persistence path
    /// (that's covered by `crate::ledger::tests`).
    fn test_ledger() -> OperationLedger {
        let pool = DbPool::open_in_memory().unwrap();
        OperationLedger::new(pool)
    }

    #[tokio::test]
    async fn test_create_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt").to_string_lossy().to_string();
        let l = test_ledger();
        let result = create_file_impl(file_path.clone(), &l).await.unwrap();
        assert!(result.success);
        assert_eq!(result.operation, "create_file");
        assert!(Path::new(&file_path).exists());
    }

    #[tokio::test]
    async fn test_create_directory_cmd() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path().join("subdir").to_string_lossy().to_string();
        let l = test_ledger();
        let result = create_directory_impl(dir_path.clone(), &l).await.unwrap();
        assert!(result.success);
        assert_eq!(result.operation, "create_folder");
        assert!(Path::new(&dir_path).is_dir());
    }

    #[tokio::test]
    async fn test_rename_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("original.txt");
        fs::write(&src, "content").unwrap();

        let l = test_ledger();
        let result = rename_file_impl(
            src.to_string_lossy().to_string(),
            "renamed.txt".to_string(),
            &l,
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(!src.exists());
        assert!(dir.path().join("renamed.txt").exists());
    }

    #[tokio::test]
    async fn test_copy_files() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.txt");
        fs::write(&src, "hello").unwrap();

        let dest_dir = dir.path().join("dest");
        fs::create_dir(&dest_dir).unwrap();

        let l = test_ledger();
        let result = copy_files_impl(
            vec![src.to_string_lossy().to_string()],
            dest_dir.to_string_lossy().to_string(),
            &l,
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(src.exists());
        assert!(dest_dir.join("source.txt").exists());
    }

    #[tokio::test]
    async fn test_copy_files_conflict_resolution() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("file.txt");
        fs::write(&src, "hello").unwrap();

        fs::write(dir.path().join("file.txt"), "existing").unwrap();

        let l = test_ledger();
        let result = copy_files_impl(
            vec![src.to_string_lossy().to_string()],
            dir.path().to_string_lossy().to_string(),
            &l,
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(dir.path().join("file (copy).txt").exists());
    }

    #[tokio::test]
    async fn test_move_files() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.txt");
        fs::write(&src, "hello").unwrap();

        let dest_dir = dir.path().join("dest");
        fs::create_dir(&dest_dir).unwrap();

        let l = test_ledger();
        let result = move_files_impl(
            vec![src.to_string_lossy().to_string()],
            dest_dir.to_string_lossy().to_string(),
            &l,
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(!src.exists());
        assert!(dest_dir.join("source.txt").exists());
    }

    #[tokio::test]
    async fn test_duplicate_files() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("file.txt");
        fs::write(&src, "content").unwrap();

        let l = test_ledger();
        let result = duplicate_files_impl(vec![src.to_string_lossy().to_string()], &l)
            .await
            .unwrap();
        assert!(result.success);
        assert!(src.exists());
        assert!(dir.path().join("file (copy).txt").exists());
    }

    #[tokio::test]
    async fn test_delete_files_permanent() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("delete_me.txt");
        fs::write(&src, "content").unwrap();

        let l = test_ledger();
        let result = delete_files_impl(
            vec![src.to_string_lossy().to_string()],
            true,
            &l,
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn test_undo_copy() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.txt");
        fs::write(&src, "hello").unwrap();

        let dest_dir = dir.path().join("dest");
        fs::create_dir(&dest_dir).unwrap();

        let l = test_ledger();
        let result = copy_files_impl(
            vec![src.to_string_lossy().to_string()],
            dest_dir.to_string_lossy().to_string(),
            &l,
        )
        .await
        .unwrap();

        let undo_result = undo_file_operation(
            result.operation,
            result.source_paths,
            result.dest_paths.clone(),
        )
        .await
        .unwrap();
        assert!(undo_result);
        assert!(!Path::new(&result.dest_paths[0]).exists());
    }

    #[tokio::test]
    async fn test_undo_rename() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("original.txt");
        fs::write(&src, "content").unwrap();

        let l = test_ledger();
        let result = rename_file_impl(
            src.to_string_lossy().to_string(),
            "renamed.txt".to_string(),
            &l,
        )
        .await
        .unwrap();

        undo_file_operation(
            result.operation,
            result.source_paths,
            result.dest_paths,
        )
        .await
        .unwrap();

        assert!(src.exists());
        assert!(!dir.path().join("renamed.txt").exists());
    }

    #[tokio::test]
    async fn test_undo_create_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("new.txt").to_string_lossy().to_string();
        let l = test_ledger();
        let result = create_file_impl(file_path.clone(), &l).await.unwrap();

        undo_file_operation(
            result.operation,
            result.source_paths,
            result.dest_paths,
        )
        .await
        .unwrap();

        assert!(!Path::new(&file_path).exists());
    }

    #[tokio::test]
    async fn test_rename_invalid_name() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("file.txt");
        fs::write(&src, "content").unwrap();

        let l = test_ledger();
        let result = rename_file_impl(
            src.to_string_lossy().to_string(),
            "invalid/name".to_string(),
            &l,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_copy_dir_recursive() {
        let dir = tempdir().unwrap();

        // Create source directory structure
        let src_dir = dir.path().join("src_dir");
        fs::create_dir_all(src_dir.join("subdir")).unwrap();
        fs::write(src_dir.join("file1.txt"), "content1").unwrap();
        fs::write(src_dir.join("subdir/file2.txt"), "content2").unwrap();

        let dest_dir = dir.path().join("dest");
        fs::create_dir(&dest_dir).unwrap();

        let l = test_ledger();
        let result = copy_files_impl(
            vec![src_dir.to_string_lossy().to_string()],
            dest_dir.to_string_lossy().to_string(),
            &l,
        )
        .await
        .unwrap();
        assert!(result.success);
        assert!(dest_dir.join("src_dir/file1.txt").exists());
        assert!(dest_dir.join("src_dir/subdir/file2.txt").exists());
    }
}
