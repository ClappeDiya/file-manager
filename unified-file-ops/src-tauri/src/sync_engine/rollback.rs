//! Sync rollback - pre-destructive snapshots and undo capability.
//!
//! Before any delete or overwrite, the original file is backed up.
//! Users can undo the last sync run within 7 days.

use crate::core::error::AppError;
use crate::core::types::*;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Manages pre-destructive snapshots for a single sync run.
pub struct RollbackManager {
    pub run_id: Uuid,
    pub pair_id: Uuid,
    pub rollback_dir: PathBuf,
    pub snapshots: Vec<SyncSnapshot>,
}

impl RollbackManager {
    pub fn new(run_id: Uuid, pair_id: Uuid, rollback_dir: &Path) -> Self {
        Self {
            run_id,
            pair_id,
            rollback_dir: rollback_dir.to_path_buf(),
            snapshots: Vec::new(),
        }
    }

    /// Take a pre-destructive snapshot of a file before it is deleted or overwritten.
    pub fn snapshot_file(
        &mut self,
        relative_path: &str,
        original_path: &Path,
    ) -> Result<(), AppError> {
        if !original_path.exists() {
            return Ok(());
        }

        let backup_path = self.rollback_dir.join(relative_path);

        // Ensure parent directory exists
        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::Sync {
                message: format!("Cannot create rollback directory: {}", e),
                advice: "Check permissions for the sync rollback directory.".to_string(),
            })?;
        }

        // Copy the original file to backup
        if original_path.is_dir() {
            copy_dir_recursive(original_path, &backup_path)?;
        } else {
            fs::copy(original_path, &backup_path).map_err(|e| AppError::Sync {
                message: format!("Cannot create backup for {}: {}", relative_path, e),
                advice: "Check disk space and permissions.".to_string(),
            })?;
        }

        let metadata = fs::metadata(original_path).ok();
        let original_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let original_modified = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<Utc>::from);

        self.snapshots.push(SyncSnapshot {
            id: Uuid::new_v4(),
            run_id: self.run_id,
            pair_id: self.pair_id,
            relative_path: relative_path.to_string(),
            action: if original_path.exists() {
                "overwrite".to_string()
            } else {
                "delete".to_string()
            },
            backup_path: backup_path.to_string_lossy().to_string(),
            original_size,
            original_modified,
            created_at: Utc::now(),
        });

        Ok(())
    }

    /// Get all snapshots taken during this run.
    pub fn get_snapshots(&self) -> &[SyncSnapshot] {
        &self.snapshots
    }
}

/// Undo a sync run by restoring all snapshot files.
pub fn rollback_sync_run(
    snapshots: &[SyncSnapshot],
    dest_root: &Path,
) -> Result<u64, AppError> {
    let mut restored: u64 = 0;

    for snapshot in snapshots.iter().rev() {
        let backup = Path::new(&snapshot.backup_path);
        let target = dest_root.join(&snapshot.relative_path);

        if !backup.exists() {
            tracing::warn!(
                "Rollback: backup file missing for {}",
                snapshot.relative_path
            );
            continue;
        }

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::Sync {
                message: format!("Cannot create directory for rollback: {}", e),
                advice: "Check permissions.".to_string(),
            })?;
        }

        if backup.is_dir() {
            copy_dir_recursive(backup, &target)?;
        } else {
            fs::copy(backup, &target).map_err(|e| AppError::Sync {
                message: format!("Cannot restore {}: {}", snapshot.relative_path, e),
                advice: "Check permissions and disk space.".to_string(),
            })?;
        }

        restored += 1;
    }

    Ok(restored)
}

/// Check if a rollback is still within the 7-day window.
pub fn is_rollback_valid(snapshots: &[SyncSnapshot]) -> bool {
    if snapshots.is_empty() {
        return false;
    }

    let oldest = snapshots
        .iter()
        .map(|s| s.created_at)
        .min()
        .unwrap_or_else(Utc::now);

    let seven_days = chrono::Duration::days(7);
    Utc::now() - oldest < seven_days
}

/// Clean up rollback data older than the specified number of days.
pub fn cleanup_rollback_data(
    rollback_base_dir: &Path,
    max_age_days: u32,
) -> Result<u64, AppError> {
    if !rollback_base_dir.exists() {
        return Ok(0);
    }

    let mut cleaned: u64 = 0;
    let cutoff = Utc::now() - chrono::Duration::days(max_age_days as i64);

    let entries = fs::read_dir(rollback_base_dir).map_err(|e| AppError::Sync {
        message: format!("Cannot read rollback directory: {}", e),
        advice: "Check permissions.".to_string(),
    })?;

    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<Utc>::from);

        if let Some(mod_time) = modified {
            if mod_time < cutoff {
                if let Err(e) = fs::remove_dir_all(&path) {
                    tracing::warn!(
                        "Cannot remove old rollback data {}: {}",
                        path.display(),
                        e
                    );
                } else {
                    cleaned += 1;
                }
            }
        }
    }

    Ok(cleaned)
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dst).map_err(|e| AppError::Sync {
        message: format!("Cannot create directory {}: {}", dst.display(), e),
        advice: "Check permissions.".to_string(),
    })?;

    for entry in fs::read_dir(src).map_err(|e| AppError::Sync {
        message: format!("Cannot read directory {}: {}", src.display(), e),
        advice: "Check permissions.".to_string(),
    })? {
        let entry = entry.map_err(|e| AppError::Sync {
            message: format!("Read error: {}", e),
            advice: "Check permissions.".to_string(),
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| AppError::Sync {
                message: format!("Copy error: {}", e),
                advice: "Check permissions.".to_string(),
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_snapshot_file() {
        let dir = TempDir::new().unwrap();
        let rollback_dir = dir.path().join("rollback");
        let original = dir.path().join("file.txt");
        fs::write(&original, "original content").unwrap();

        let mut mgr = RollbackManager::new(Uuid::new_v4(), Uuid::new_v4(), &rollback_dir);
        mgr.snapshot_file("file.txt", &original).unwrap();

        assert_eq!(mgr.snapshots.len(), 1);
        assert!(rollback_dir.join("file.txt").exists());

        let backup_content = fs::read_to_string(rollback_dir.join("file.txt")).unwrap();
        assert_eq!(backup_content, "original content");
    }

    #[test]
    fn test_rollback_restores_files() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        let rollback_dir = dir.path().join("rollback");
        fs::create_dir_all(&dest).unwrap();

        // Create original file
        let original = dest.join("file.txt");
        fs::write(&original, "original").unwrap();

        // Take snapshot
        let mut mgr = RollbackManager::new(Uuid::new_v4(), Uuid::new_v4(), &rollback_dir);
        mgr.snapshot_file("file.txt", &original).unwrap();

        // "Sync" overwrites the file
        fs::write(&original, "overwritten by sync").unwrap();
        assert_eq!(fs::read_to_string(&original).unwrap(), "overwritten by sync");

        // Rollback
        let restored = rollback_sync_run(&mgr.snapshots, &dest).unwrap();
        assert_eq!(restored, 1);

        // Verify restored content
        assert_eq!(fs::read_to_string(&original).unwrap(), "original");
    }

    #[test]
    fn test_rollback_restores_deleted_files() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("dest");
        let rollback_dir = dir.path().join("rollback");
        fs::create_dir_all(&dest).unwrap();

        let original = dest.join("deleted.txt");
        fs::write(&original, "will be deleted").unwrap();

        let mut mgr = RollbackManager::new(Uuid::new_v4(), Uuid::new_v4(), &rollback_dir);
        mgr.snapshot_file("deleted.txt", &original).unwrap();

        // "Delete" the file
        fs::remove_file(&original).unwrap();
        assert!(!original.exists());

        // Rollback
        let restored = rollback_sync_run(&mgr.snapshots, &dest).unwrap();
        assert_eq!(restored, 1);
        assert!(original.exists());
        assert_eq!(fs::read_to_string(&original).unwrap(), "will be deleted");
    }

    #[test]
    fn test_is_rollback_valid() {
        let recent_snapshot = SyncSnapshot {
            id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            pair_id: Uuid::new_v4(),
            relative_path: "file.txt".to_string(),
            action: "overwrite".to_string(),
            backup_path: "/tmp/backup".to_string(),
            original_size: 100,
            original_modified: None,
            created_at: Utc::now(),
        };

        assert!(is_rollback_valid(&[recent_snapshot]));
        assert!(!is_rollback_valid(&[]));
    }

    #[test]
    fn test_snapshot_nested_directory() {
        let dir = TempDir::new().unwrap();
        let rollback_dir = dir.path().join("rollback");
        let nested_dir = dir.path().join("sub").join("nested");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(nested_dir.join("deep.txt"), "deep content").unwrap();

        let mut mgr = RollbackManager::new(Uuid::new_v4(), Uuid::new_v4(), &rollback_dir);
        mgr.snapshot_file("sub/nested", &dir.path().join("sub").join("nested"))
            .unwrap();

        assert_eq!(mgr.snapshots.len(), 1);
        assert!(rollback_dir.join("sub/nested/deep.txt").exists());
    }

    #[test]
    fn test_cleanup_rollback_data() {
        let dir = TempDir::new().unwrap();
        let rollback_base = dir.path().join("rollback");
        fs::create_dir_all(rollback_base.join("old-run")).unwrap();
        fs::write(rollback_base.join("old-run/file.txt"), "old").unwrap();

        // Clean with 0 days max age = remove everything
        let cleaned = cleanup_rollback_data(&rollback_base, 0).unwrap();
        assert_eq!(cleaned, 1);
    }
}
