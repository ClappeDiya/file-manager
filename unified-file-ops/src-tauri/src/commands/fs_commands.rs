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
