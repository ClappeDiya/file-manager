//! File system engine - core file operations (copy, move, rename, delete, etc.)
//!
//! Implements `FsOperations` trait for local filesystem access.

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, FsOperations};
use crate::core::types::FileEntry;
use chrono::{DateTime, Utc};
use std::path::Path;

/// Local filesystem implementation.
pub struct LocalFs;

impl LocalFs {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LocalFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FsOperations for LocalFs {
    fn list_directory(&self, path: &str) -> BoxFuture<'_, Result<Vec<FileEntry>, AppError>> {
        let path = path.to_string();
        Box::pin(async move { list_directory_impl(&path) })
    }

    fn create_directory(&self, path: &str) -> BoxFuture<'_, Result<(), AppError>> {
        let path = path.to_string();
        Box::pin(async move {
            std::fs::create_dir_all(&path).map_err(|e| {
                AppError::file_op(
                    format!("Cannot create directory: {e}"),
                    "Check that you have write permissions to the parent folder.",
                )
            })
        })
    }

    fn delete_entry(&self, path: &str, recursive: bool) -> BoxFuture<'_, Result<(), AppError>> {
        let path = path.to_string();
        Box::pin(async move {
            let p = Path::new(&path);
            if p.is_dir() {
                if recursive {
                    std::fs::remove_dir_all(p)
                } else {
                    std::fs::remove_dir(p)
                }
            } else {
                std::fs::remove_file(p)
            }
            .map_err(|e| {
                AppError::file_op(
                    format!("Cannot delete: {e}"),
                    "Check that the item is not in use and you have permission.",
                )
            })
        })
    }

    fn rename_entry(&self, from: &str, to: &str) -> BoxFuture<'_, Result<(), AppError>> {
        let from = from.to_string();
        let to = to.to_string();
        Box::pin(async move {
            std::fs::rename(&from, &to).map_err(|e| {
                AppError::file_op(
                    format!("Cannot rename: {e}"),
                    "Check that the destination path is valid and writable.",
                )
            })
        })
    }

    fn get_metadata(&self, path: &str) -> BoxFuture<'_, Result<FileEntry, AppError>> {
        let path = path.to_string();
        Box::pin(async move { get_metadata_impl(&path) })
    }
}

/// Shared listing implementation used by both the trait and the Tauri command.
pub fn list_directory_impl(path: &str) -> Result<Vec<FileEntry>, AppError> {
    let dir_path = Path::new(path);

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

    let read_dir = std::fs::read_dir(dir_path).map_err(|e| {
        AppError::file_op(
            format!("Cannot read directory: {e}"),
            "Check permissions for this folder.",
        )
    })?;

    let mut entries = Vec::new();

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

        let modified: Option<DateTime<Utc>> = metadata.modified().ok().map(DateTime::from);
        let created: Option<DateTime<Utc>> = metadata.created().ok().map(DateTime::from);

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
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

/// Public metadata accessor for use by other commands.
pub fn get_metadata_pub(path: &str) -> Result<FileEntry, AppError> {
    get_metadata_impl(path)
}

fn get_metadata_impl(path: &str) -> Result<FileEntry, AppError> {
    let p = Path::new(path);
    let metadata = std::fs::metadata(p)?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let is_hidden = name.starts_with('.');

    let modified: Option<DateTime<Utc>> = metadata.modified().ok().map(DateTime::from);
    let created: Option<DateTime<Utc>> = metadata.created().ok().map(DateTime::from);

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

    Ok(FileEntry {
        name,
        path: path.to_string(),
        is_dir: metadata.is_dir(),
        is_symlink: metadata.is_symlink(),
        size: metadata.len(),
        modified,
        created,
        is_hidden,
        extension,
        permissions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_directory_nonexistent() {
        let result = list_directory_impl("/this/path/does/not/exist/ever");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_directory_tmp() {
        // /tmp always exists on macOS/Linux
        let result = list_directory_impl("/tmp");
        assert!(result.is_ok());
    }

    #[test]
    fn test_local_fs_default() {
        let _fs = LocalFs;
    }

    #[tokio::test]
    async fn test_local_fs_trait_list() {
        let fs = LocalFs::new();
        let result = fs.list_directory("/tmp").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_local_fs_create_and_delete() {
        let fs = LocalFs::new();
        let test_dir = format!("/tmp/ufop_test_{}", uuid::Uuid::new_v4());
        fs.create_directory(&test_dir).await.unwrap();
        assert!(Path::new(&test_dir).exists());
        fs.delete_entry(&test_dir, false).await.unwrap();
        assert!(!Path::new(&test_dir).exists());
    }

    #[tokio::test]
    async fn test_local_fs_get_metadata() {
        let fs = LocalFs::new();
        let result = fs.get_metadata("/tmp").await;
        assert!(result.is_ok());
        let entry = result.unwrap();
        assert!(entry.is_dir);
    }
}
