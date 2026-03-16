use crate::core::error::AppError;
use crate::core::types::FileEntry;
use chrono::{DateTime, Utc};
use std::fs;
use std::path::Path;

/// List directory contents, returning file entries sorted by name.
#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<FileEntry>, AppError> {
    let dir_path = Path::new(&path);

    if !dir_path.exists() {
        return Err(AppError::file_op(
            format!("Directory not found: {path}"),
            "Check that the path exists and you have permission to access it.",
        ));
    }

    if !dir_path.is_dir() {
        return Err(AppError::file_op(
            format!("Not a directory: {path}"),
            "The specified path is a file, not a directory.",
        ));
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(dir_path).map_err(|e| {
        AppError::file_op(
            format!("Cannot read directory: {e}"),
            "Check permissions for this folder.",
        )
    })?;

    for entry_result in read_dir {
        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let name = entry.file_name().to_string_lossy().to_string();
        let file_path = entry.path().to_string_lossy().to_string();
        let is_hidden = name.starts_with('.');

        let modified: Option<DateTime<Utc>> = metadata
            .modified()
            .ok()
            .map(DateTime::from);

        let created: Option<DateTime<Utc>> = metadata
            .created()
            .ok()
            .map(DateTime::from);

        let extension = if metadata.is_file() {
            Path::new(&name)
                .extension()
                .map(|e| e.to_string_lossy().to_string())
        } else {
            None
        };

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            Some(format!("{:o}", metadata.permissions().mode() & 0o777))
        };

        #[cfg(not(unix))]
        let permissions = None;

        entries.push(FileEntry {
            name,
            path: file_path,
            is_dir: metadata.is_dir(),
            is_symlink: metadata.is_symlink(),
            size: metadata.len(),
            modified,
            created,
            is_hidden,
            extension,
            permissions,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
        })
    });

    Ok(entries)
}
