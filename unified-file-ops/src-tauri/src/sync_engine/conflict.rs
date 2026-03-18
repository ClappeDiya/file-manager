//! Sync conflict resolution - 7 deterministic policies.
//!
//! Policies: ask, source_wins, dest_wins, newest_wins,
//!           create_conflict_copy, skip, quarantine.

use crate::core::error::AppError;
use crate::core::types::*;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Result of resolving a single conflict.
#[derive(Debug, Clone)]
pub struct ConflictResult {
    /// The action to take
    pub action: ConflictAction,
    /// The resolved destination path (may differ for conflict copies)
    pub resolved_path: PathBuf,
    /// Whether the file was quarantined
    pub quarantined: bool,
}

/// Action to take after conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictAction {
    /// Copy source to destination (overwrite)
    CopySource,
    /// Keep destination as-is
    KeepDest,
    /// Copy source to a conflict-renamed path
    CopyToConflictPath,
    /// Skip this file entirely
    Skip,
    /// Move to quarantine directory
    Quarantine,
    /// Ask the user (deferred resolution)
    AskUser,
}

/// Resolve a conflict for a single file using the given policy.
#[allow(clippy::too_many_arguments)]
pub fn resolve_conflict(
    policy: SyncConflictPolicy,
    relative_path: &str,
    _source_path: &Path,
    dest_path: &Path,
    _source_size: u64,
    _dest_size: u64,
    source_modified: Option<chrono::DateTime<Utc>>,
    dest_modified: Option<chrono::DateTime<Utc>>,
    quarantine_dir: Option<&Path>,
) -> Result<ConflictResult, AppError> {
    match policy {
        SyncConflictPolicy::Ask => Ok(ConflictResult {
            action: ConflictAction::AskUser,
            resolved_path: dest_path.to_path_buf(),
            quarantined: false,
        }),

        SyncConflictPolicy::SourceWins => Ok(ConflictResult {
            action: ConflictAction::CopySource,
            resolved_path: dest_path.to_path_buf(),
            quarantined: false,
        }),

        SyncConflictPolicy::DestWins => Ok(ConflictResult {
            action: ConflictAction::KeepDest,
            resolved_path: dest_path.to_path_buf(),
            quarantined: false,
        }),

        SyncConflictPolicy::NewestWins => {
            let source_newer = match (source_modified, dest_modified) {
                (Some(s), Some(d)) => s > d,
                (Some(_), None) => true,
                _ => false,
            };

            if source_newer {
                Ok(ConflictResult {
                    action: ConflictAction::CopySource,
                    resolved_path: dest_path.to_path_buf(),
                    quarantined: false,
                })
            } else {
                Ok(ConflictResult {
                    action: ConflictAction::KeepDest,
                    resolved_path: dest_path.to_path_buf(),
                    quarantined: false,
                })
            }
        }

        SyncConflictPolicy::CreateConflictCopy => {
            let conflict_path = generate_conflict_path(dest_path);
            Ok(ConflictResult {
                action: ConflictAction::CopyToConflictPath,
                resolved_path: conflict_path,
                quarantined: false,
            })
        }

        SyncConflictPolicy::Skip => Ok(ConflictResult {
            action: ConflictAction::Skip,
            resolved_path: dest_path.to_path_buf(),
            quarantined: false,
        }),

        SyncConflictPolicy::Quarantine => {
            let qdir = quarantine_dir.ok_or_else(|| AppError::Sync {
                message: "Quarantine directory not configured.".to_string(),
                advice: "Set up a quarantine directory for this sync pair.".to_string(),
            })?;

            let quarantine_path = qdir.join(relative_path);
            Ok(ConflictResult {
                action: ConflictAction::Quarantine,
                resolved_path: quarantine_path,
                quarantined: true,
            })
        }
    }
}

/// Generate a conflict copy path with (conflict YYYY-MM-DD) suffix.
/// e.g., "document.txt" -> "document (conflict 2026-03-15).txt"
pub fn generate_conflict_path(original: &Path) -> PathBuf {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = original.extension().and_then(|s| s.to_str());

    let new_name = if let Some(ext) = ext {
        format!("{} (conflict {}).{}", stem, date, ext)
    } else {
        format!("{} (conflict {})", stem, date)
    };

    let parent = original.parent().unwrap_or_else(|| Path::new("."));
    let mut path = parent.join(&new_name);

    // If the conflict path already exists, add a counter
    let mut counter = 2;
    while path.exists() {
        let new_name = if let Some(ext) = ext {
            format!("{} (conflict {} {}).{}", stem, date, counter, ext)
        } else {
            format!("{} (conflict {} {})", stem, date, counter)
        };
        path = parent.join(new_name);
        counter += 1;
    }

    path
}

/// Execute the quarantine action: move the destination file to quarantine directory.
pub fn quarantine_file(
    dest_path: &Path,
    quarantine_path: &Path,
) -> Result<(), AppError> {
    if let Some(parent) = quarantine_path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::Sync {
            message: format!("Cannot create quarantine directory: {}", e),
            advice: "Check permissions on the quarantine directory.".to_string(),
        })?;
    }

    if dest_path.exists() {
        fs::copy(dest_path, quarantine_path).map_err(|e| AppError::Sync {
            message: format!("Cannot quarantine file: {}", e),
            advice: "Check permissions.".to_string(),
        })?;
    }

    Ok(())
}

/// Log a conflict resolution to the database (via pool closure).
#[allow(clippy::too_many_arguments)]
pub fn create_conflict_log_entry(
    pair_id: Uuid,
    run_id: Uuid,
    relative_path: &str,
    policy: SyncConflictPolicy,
    source_size: u64,
    dest_size: u64,
    source_modified: Option<chrono::DateTime<Utc>>,
    dest_modified: Option<chrono::DateTime<Utc>>,
    result_path: &str,
) -> Vec<(String, String)> {
    // Return key-value pairs for logging
    vec![
        ("pair_id".to_string(), pair_id.to_string()),
        ("run_id".to_string(), run_id.to_string()),
        ("relative_path".to_string(), relative_path.to_string()),
        ("policy_applied".to_string(), policy.as_str().to_string()),
        ("source_size".to_string(), source_size.to_string()),
        ("dest_size".to_string(), dest_size.to_string()),
        (
            "source_modified".to_string(),
            source_modified
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
        ),
        (
            "dest_modified".to_string(),
            dest_modified
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
        ),
        ("result_path".to_string(), result_path.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_source_wins_policy() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        fs::write(&src, "source").unwrap();
        fs::write(&dst, "dest").unwrap();

        let result = resolve_conflict(
            SyncConflictPolicy::SourceWins,
            "file.txt",
            &src,
            &dst,
            6,
            4,
            Some(Utc::now()),
            Some(Utc::now()),
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::CopySource);
    }

    #[test]
    fn test_dest_wins_policy() {
        let dir = TempDir::new().unwrap();
        let result = resolve_conflict(
            SyncConflictPolicy::DestWins,
            "file.txt",
            &dir.path().join("src"),
            &dir.path().join("dst"),
            6,
            4,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::KeepDest);
    }

    #[test]
    fn test_newest_wins_source_newer() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(1);
        let dir = TempDir::new().unwrap();

        let result = resolve_conflict(
            SyncConflictPolicy::NewestWins,
            "file.txt",
            &dir.path().join("src"),
            &dir.path().join("dst"),
            6,
            4,
            Some(now),
            Some(old),
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::CopySource);
    }

    #[test]
    fn test_newest_wins_dest_newer() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(1);
        let dir = TempDir::new().unwrap();

        let result = resolve_conflict(
            SyncConflictPolicy::NewestWins,
            "file.txt",
            &dir.path().join("src"),
            &dir.path().join("dst"),
            6,
            4,
            Some(old),
            Some(now),
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::KeepDest);
    }

    #[test]
    fn test_create_conflict_copy() {
        let dir = TempDir::new().unwrap();
        let dst = dir.path().join("document.txt");
        fs::write(&dst, "existing").unwrap();

        let result = resolve_conflict(
            SyncConflictPolicy::CreateConflictCopy,
            "document.txt",
            &dir.path().join("src"),
            &dst,
            100,
            50,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::CopyToConflictPath);
        assert!(result
            .resolved_path
            .to_string_lossy()
            .contains("(conflict"));
    }

    #[test]
    fn test_skip_policy() {
        let dir = TempDir::new().unwrap();
        let result = resolve_conflict(
            SyncConflictPolicy::Skip,
            "file.txt",
            &dir.path().join("src"),
            &dir.path().join("dst"),
            6,
            4,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::Skip);
    }

    #[test]
    fn test_quarantine_policy() {
        let dir = TempDir::new().unwrap();
        let qdir = dir.path().join("quarantine");

        let result = resolve_conflict(
            SyncConflictPolicy::Quarantine,
            "file.txt",
            &dir.path().join("src"),
            &dir.path().join("dst"),
            6,
            4,
            None,
            None,
            Some(&qdir),
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::Quarantine);
        assert!(result.quarantined);
    }

    #[test]
    fn test_ask_policy() {
        let dir = TempDir::new().unwrap();
        let result = resolve_conflict(
            SyncConflictPolicy::Ask,
            "file.txt",
            &dir.path().join("src"),
            &dir.path().join("dst"),
            6,
            4,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.action, ConflictAction::AskUser);
    }

    #[test]
    fn test_generate_conflict_path_format() {
        let path = Path::new("/tmp/document.txt");
        let conflict = generate_conflict_path(path);
        let name = conflict.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("document (conflict "));
        assert!(name.ends_with(".txt"));
    }

    #[test]
    fn test_generate_conflict_path_no_extension() {
        let path = Path::new("/tmp/Makefile");
        let conflict = generate_conflict_path(path);
        let name = conflict.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("Makefile (conflict "));
        assert!(!name.contains('.'));
    }

    #[test]
    fn test_all_seven_policies_deterministic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        let qdir = dir.path().join("quarantine");
        fs::write(&src, "s").unwrap();
        fs::write(&dst, "d").unwrap();

        let now = Utc::now();
        let policies = [
            SyncConflictPolicy::Ask,
            SyncConflictPolicy::SourceWins,
            SyncConflictPolicy::DestWins,
            SyncConflictPolicy::NewestWins,
            SyncConflictPolicy::CreateConflictCopy,
            SyncConflictPolicy::Skip,
            SyncConflictPolicy::Quarantine,
        ];

        for policy in &policies {
            let result = resolve_conflict(
                *policy,
                "file.txt",
                &src,
                &dst,
                1,
                1,
                Some(now),
                Some(now - chrono::Duration::hours(1)),
                Some(&qdir),
            )
            .unwrap();

            // Each policy should produce a deterministic result
            match policy {
                SyncConflictPolicy::Ask => assert_eq!(result.action, ConflictAction::AskUser),
                SyncConflictPolicy::SourceWins => {
                    assert_eq!(result.action, ConflictAction::CopySource)
                }
                SyncConflictPolicy::DestWins => {
                    assert_eq!(result.action, ConflictAction::KeepDest)
                }
                SyncConflictPolicy::NewestWins => {
                    assert_eq!(result.action, ConflictAction::CopySource)
                }
                SyncConflictPolicy::CreateConflictCopy => {
                    assert_eq!(result.action, ConflictAction::CopyToConflictPath)
                }
                SyncConflictPolicy::Skip => assert_eq!(result.action, ConflictAction::Skip),
                SyncConflictPolicy::Quarantine => {
                    assert_eq!(result.action, ConflictAction::Quarantine)
                }
            }
        }
    }
}
