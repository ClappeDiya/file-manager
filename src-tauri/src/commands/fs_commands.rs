use crate::core::error::AppError;
use crate::core::types::FileEntry;
use crate::fs_engine;

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
