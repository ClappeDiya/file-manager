use crate::commands::file_ops_commands::record_fs_ok;
use crate::core::error::AppError;
use crate::core::types::FileEntry;
use crate::fs_engine;
use crate::ledger::OperationLedger;

/// List directory contents, returning file entries sorted by name.
/// Delegates to the fs_engine module for actual filesystem access.
#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<FileEntry>, AppError> {
    // Delegate to fs_engine (which implements FsOperations trait)
    fs_engine::list_directory_impl(&path)
}

/// Open a native folder picker dialog and return the selected path.
/// Uses the async callback pattern with a oneshot channel to avoid
/// blocking the tokio runtime (which deadlocks NSOpenPanel on macOS).
#[tauri::command]
pub async fn pick_folder(
    window: tauri::Window,
    title: Option<String>,
) -> Result<Option<String>, AppError> {
    use tauri_plugin_dialog::DialogExt;
    use tauri::Manager;

    let dialog = window.app_handle().dialog().file();

    // Set parent window — critical on macOS so the dialog appears
    // attached to the app window instead of behind it.
    #[cfg(any(windows, target_os = "macos"))]
    let dialog = dialog.set_parent(&window);

    let dialog = if let Some(t) = title {
        dialog.set_title(t)
    } else {
        dialog
    };

    // Use async pick_folder with a oneshot channel so we don't block
    // the tokio thread. blocking_pick_folder() can deadlock on macOS
    // when called from an async Tauri command.
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |folder| {
        let _ = tx.send(folder.map(|p| p.to_string()));
    });

    rx.await.map_err(|_| AppError::Internal {
        message: "Folder picker was cancelled or failed to respond".to_string(),
    })
}

/// Maximum byte size for full-content text read/write. Matches the
/// `MAX_FILE_SIZE` constant in `src/components/text-editor.tsx` so
/// the backend and frontend agree on the 1 MB in-app-editor ceiling.
const TEXT_EDITOR_MAX_BYTES: u64 = 1024 * 1024;

/// Read a text file fully into a string for the in-app text editor
/// (iter 32). The 1 MB cap matches the editor's own limit and keeps
/// the tokio runtime from stalling on huge files. Invalid UTF-8 is
/// returned as lossy-decoded text so the editor can still display
/// and re-save it, but the caller should treat round-trip loss as a
/// known risk. File I/O runs on `spawn_blocking` to avoid blocking
/// async executors.
#[tauri::command]
pub async fn read_text_file_full(path: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || -> Result<String, AppError> {
        let p = std::path::Path::new(&path);
        let meta = std::fs::metadata(p).map_err(|e| {
            AppError::file_op(
                format!("Cannot read file metadata: {e}"),
                "Check that the file exists and is readable.",
            )
        })?;
        if meta.len() > TEXT_EDITOR_MAX_BYTES {
            return Err(AppError::validation(format!(
                "File too large for in-app editor: {} bytes (max {} bytes). Use an external editor.",
                meta.len(),
                TEXT_EDITOR_MAX_BYTES
            )));
        }
        let bytes = std::fs::read(p).map_err(|e| {
            AppError::file_op(
                format!("Cannot read file: {e}"),
                "Check file permissions and that the file is not locked.",
            )
        })?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    })
    .await
    .map_err(|e| AppError::internal(format!("Text-file read task failed: {e}")))?
}

/// Write a text file fully for the in-app editor (iter 32). Uses a
/// write-then-rename pattern so a crash mid-save leaves the original
/// file untouched rather than a zero-byte or half-written stub. The
/// temp file lives in the SAME directory as the destination so the
/// rename is guaranteed to be atomic on every POSIX filesystem and
/// on NTFS. The caller must pre-validate the 1 MB cap, but the
/// backend double-checks as a defence-in-depth gate.
///
/// Iter 35: now records every successful save in the unified
/// `OperationLedger` under kind `"edit_text"`. This makes in-app
/// edits visible to:
///   - the activity-timeline panel (iter 20 ledger tail poll)
///   - cross-pane refresh (same tail poll fires `dispatchRefresh`
///     on the path, so every pane viewing the file's parent dir
///     picks up the new mtime automatically)
///   - audit exports / undo history
/// The recording is fail-open by construction (see `record_fs_ok`),
/// so a transient ledger DB hiccup can never block the user's save.
#[tauri::command]
pub async fn write_text_file_full(
    path: String,
    content: String,
    ledger: tauri::State<'_, OperationLedger>,
) -> Result<(), AppError> {
    if content.len() as u64 > TEXT_EDITOR_MAX_BYTES {
        return Err(AppError::validation(format!(
            "Content too large for in-app editor: {} bytes (max {} bytes).",
            content.len(),
            TEXT_EDITOR_MAX_BYTES
        )));
    }
    let path_for_rename = path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let p = std::path::Path::new(&path_for_rename);
        let parent = p.parent().ok_or_else(|| {
            AppError::validation("Invalid file path: no parent directory".to_string())
        })?;
        let file_name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let tmp_path = parent.join(format!(".{}.{}.tmp", file_name, std::process::id()));
        std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
            AppError::file_op(
                format!("Cannot write temp file: {e}"),
                "Check disk space and directory write permissions.",
            )
        })?;
        if let Err(e) = std::fs::rename(&tmp_path, p) {
            // Best-effort cleanup so we don't leave a stray .tmp file
            // on failure. The original file is still untouched.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(AppError::file_op(
                format!("Cannot replace file atomically: {e}"),
                "The original file is unchanged; check permissions on the destination.",
            ));
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(format!("Text-file write task failed: {e}")))??;

    // Record the successful save in the unified operation ledger
    // AFTER the rename lands on disk. A fresh correlation id per
    // save keeps every edit individually addressable in the
    // timeline (unlike batch operations which share an id across
    // paired records).
    let correlation_id = uuid::Uuid::new_v4().to_string();
    record_fs_ok(&ledger, "edit_text", &[], &[path], &correlation_id).await;
    Ok(())
}
