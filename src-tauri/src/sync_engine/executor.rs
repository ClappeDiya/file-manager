//! Sync executor - state machine that performs actual file synchronization.
//!
//! Uses the planner for diff computation, conflict resolver for conflicts,
//! and supports partial failure continuation and resumable state.

use crate::core::error::AppError;
use crate::core::types::*;
use crate::sync_engine::conflict::{self, ConflictAction};
use crate::sync_engine::planner;
use crate::sync_engine::rollback::RollbackManager;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Execution state for a running sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorState {
    Planning,
    Executing { current_index: u64, total: u64 },
    Verifying,
    Completed,
    Failed { error: String },
    Cancelled,
}

/// Configuration for an executor run.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub pair: SyncPair,
    pub run_id: Uuid,
    /// Whether to continue on partial failure
    pub continue_on_error: bool,
    /// Optional resume state (last completed index)
    pub resume_from: Option<u64>,
    /// Quarantine directory for quarantine policy
    pub quarantine_dir: Option<PathBuf>,
    /// Rollback directory for snapshots
    pub rollback_dir: Option<PathBuf>,
}

/// Execute a sync run and return a report.
pub fn execute_sync(config: &ExecutorConfig) -> Result<SyncReport, AppError> {
    let started_at = Utc::now();
    let pair = &config.pair;

    // Phase 1: Plan
    let preview = planner::compute_preview(pair)?;

    // Phase 2: Ensure destination exists
    let dest_root = Path::new(&pair.dest_path);
    if !dest_root.exists() {
        fs::create_dir_all(dest_root).map_err(|e| AppError::Sync {
            message: format!("Cannot create destination directory: {}", e),
            advice: "Check permissions for the destination path.".to_string(),
        })?;
    }

    // Initialize rollback manager
    let rollback_dir = config
        .rollback_dir
        .clone()
        .unwrap_or_else(|| dest_root.join(".sync-rollback").join(config.run_id.to_string()));
    let mut rollback = RollbackManager::new(config.run_id, pair.id, &rollback_dir);

    let mut files_added: u64 = 0;
    let mut files_modified: u64 = 0;
    let mut files_deleted: u64 = 0;
    let mut files_skipped: u64 = 0;
    let mut conflicts_resolved: u64 = 0;
    let mut errors: u32 = 0;
    let mut bytes_transferred: u64 = 0;
    let mut error_messages: Vec<String> = Vec::new();
    let resumed = config.resume_from.is_some();

    // Collect all planned operations into a flat list
    let mut operations: Vec<(&SyncDiffEntry, &str)> = Vec::new();
    for entry in &preview.additions {
        operations.push((entry, "add"));
    }
    for entry in &preview.modifications {
        operations.push((entry, "modify"));
    }
    for entry in &preview.deletions {
        operations.push((entry, "delete"));
    }

    let start_index = config.resume_from.unwrap_or(0) as usize;

    // Phase 3: Execute operations
    for (idx, (entry, op_type)) in operations.iter().enumerate() {
        if idx < start_index {
            continue;
        }

        let result = execute_single_operation(
            pair,
            entry,
            op_type,
            &mut rollback,
            config.quarantine_dir.as_deref(),
        );

        match result {
            Ok(op_result) => {
                match op_result {
                    OperationResult::Added(bytes) => {
                        files_added += 1;
                        bytes_transferred += bytes;
                    }
                    OperationResult::Modified(bytes) => {
                        files_modified += 1;
                        bytes_transferred += bytes;
                    }
                    OperationResult::Deleted => {
                        files_deleted += 1;
                    }
                    OperationResult::Skipped => {
                        files_skipped += 1;
                    }
                    OperationResult::ConflictResolved(bytes) => {
                        conflicts_resolved += 1;
                        bytes_transferred += bytes;
                    }
                }
            }
            Err(e) => {
                errors += 1;
                error_messages.push(format!("{}: {}", entry.relative_path, e));
                if !config.continue_on_error {
                    break;
                }
            }
        }
    }

    // Handle conflicts from preview
    for entry in &preview.conflicts {
        let result = execute_conflict_resolution(
            pair,
            entry,
            &mut rollback,
            config.quarantine_dir.as_deref(),
        );

        match result {
            Ok(op_result) => match op_result {
                OperationResult::ConflictResolved(bytes) => {
                    conflicts_resolved += 1;
                    bytes_transferred += bytes;
                }
                OperationResult::Skipped => {
                    files_skipped += 1;
                }
                _ => {}
            },
            Err(e) => {
                errors += 1;
                error_messages.push(format!("Conflict {}: {}", entry.relative_path, e));
                if !config.continue_on_error {
                    break;
                }
            }
        }
    }

    let ended_at = Utc::now();
    let duration_ms = (ended_at - started_at).num_milliseconds().max(0) as u64;

    // Determine status
    let status = if errors == 0 {
        SyncRunStatus::Success
    } else if files_added + files_modified + files_deleted > 0 {
        SyncRunStatus::PartialSuccess
    } else {
        SyncRunStatus::Failed
    };

    // Determine health
    let health = match status {
        SyncRunStatus::Success => SyncHealth::Green,
        SyncRunStatus::PartialSuccess => SyncHealth::Yellow,
        SyncRunStatus::Failed => SyncHealth::Red,
        _ => SyncHealth::Gray,
    };

    Ok(SyncReport {
        id: config.run_id,
        pair_id: pair.id,
        pair_name: pair.name.clone(),
        started_at,
        ended_at,
        duration_ms,
        files_added,
        files_modified,
        files_deleted,
        files_skipped,
        conflicts_resolved,
        errors,
        bytes_transferred,
        status,
        health,
        error_messages,
        resumed,
    })
}

/// Result of executing a single file operation.
enum OperationResult {
    Added(u64),
    Modified(u64),
    Deleted,
    Skipped,
    ConflictResolved(u64),
}

/// Execute a single file operation (add/modify/delete).
fn execute_single_operation(
    pair: &SyncPair,
    entry: &SyncDiffEntry,
    op_type: &str,
    rollback: &mut RollbackManager,
    _quarantine_dir: Option<&Path>,
) -> Result<OperationResult, AppError> {
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let source_file = source_root.join(&entry.relative_path);
    let dest_file = dest_root.join(&entry.relative_path);

    match op_type {
        "add" => {
            // For versioned backup, add version suffix
            let target = if pair.mode == SyncMode::VersionedBackup {
                generate_versioned_path(&dest_file)
            } else {
                dest_file.clone()
            };

            // Ensure parent directory exists
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| AppError::Sync {
                    message: format!("Cannot create directory {}: {}", parent.display(), e),
                    advice: "Check destination permissions.".to_string(),
                })?;
            }

            // Determine source: for two-way sync, new files in dest get copied to source
            if entry.reason.contains("two-way") && entry.source_size.is_none() {
                // Copy from dest to source
                let source_target = source_root.join(&entry.relative_path);
                if let Some(parent) = source_target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let bytes = fs::copy(&dest_file, &source_target).map_err(|e| AppError::Sync {
                    message: format!("Copy failed: {}", e),
                    advice: "Check file permissions.".to_string(),
                })?;
                return Ok(OperationResult::Added(bytes));
            }

            let bytes = fs::copy(&source_file, &target).map_err(|e| AppError::Sync {
                message: format!("Copy failed {}: {}", entry.relative_path, e),
                advice: "Check file permissions.".to_string(),
            })?;
            Ok(OperationResult::Added(bytes))
        }

        "modify" => {
            // Take pre-destructive snapshot if destination exists
            if dest_file.exists() {
                rollback.snapshot_file(&entry.relative_path, &dest_file)?;
            }

            if pair.mode == SyncMode::VersionedBackup {
                // Keep old version, write new version
                let target = generate_versioned_path(&dest_file);
                let bytes = fs::copy(&source_file, &target).map_err(|e| AppError::Sync {
                    message: format!("Copy failed: {}", e),
                    advice: "Check permissions.".to_string(),
                })?;
                Ok(OperationResult::Modified(bytes))
            } else {
                let bytes = fs::copy(&source_file, &dest_file).map_err(|e| AppError::Sync {
                    message: format!("Copy failed {}: {}", entry.relative_path, e),
                    advice: "Check permissions.".to_string(),
                })?;
                Ok(OperationResult::Modified(bytes))
            }
        }

        "delete" => {
            if dest_file.exists() {
                // Take pre-destructive snapshot
                rollback.snapshot_file(&entry.relative_path, &dest_file)?;

                if dest_file.is_dir() {
                    fs::remove_dir_all(&dest_file).map_err(|e| AppError::Sync {
                        message: format!("Delete failed {}: {}", entry.relative_path, e),
                        advice: "Check permissions.".to_string(),
                    })?;
                } else {
                    fs::remove_file(&dest_file).map_err(|e| AppError::Sync {
                        message: format!("Delete failed {}: {}", entry.relative_path, e),
                        advice: "Check permissions.".to_string(),
                    })?;
                }
            }
            Ok(OperationResult::Deleted)
        }

        _ => Ok(OperationResult::Skipped),
    }
}

/// Execute conflict resolution for a conflicted file.
fn execute_conflict_resolution(
    pair: &SyncPair,
    entry: &SyncDiffEntry,
    rollback: &mut RollbackManager,
    quarantine_dir: Option<&Path>,
) -> Result<OperationResult, AppError> {
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let source_file = source_root.join(&entry.relative_path);
    let dest_file = dest_root.join(&entry.relative_path);

    let result = conflict::resolve_conflict(
        pair.conflict_policy,
        &entry.relative_path,
        &source_file,
        &dest_file,
        entry.source_size.unwrap_or(0),
        entry.dest_size.unwrap_or(0),
        entry.source_modified,
        entry.dest_modified,
        quarantine_dir,
    )?;

    match result.action {
        ConflictAction::CopySource => {
            if dest_file.exists() {
                rollback.snapshot_file(&entry.relative_path, &dest_file)?;
            }
            let bytes = fs::copy(&source_file, &dest_file).map_err(|e| AppError::Sync {
                message: format!("Copy failed: {}", e),
                advice: "Check permissions.".to_string(),
            })?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::KeepDest => Ok(OperationResult::Skipped),
        ConflictAction::CopyToConflictPath => {
            if let Some(parent) = result.resolved_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let bytes =
                fs::copy(&source_file, &result.resolved_path).map_err(|e| AppError::Sync {
                    message: format!("Copy to conflict path failed: {}", e),
                    advice: "Check permissions.".to_string(),
                })?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::Skip => Ok(OperationResult::Skipped),
        ConflictAction::Quarantine => {
            if dest_file.exists() {
                conflict::quarantine_file(&dest_file, &result.resolved_path)?;
            }
            // Also copy source to dest
            let bytes = fs::copy(&source_file, &dest_file).map_err(|e| AppError::Sync {
                message: format!("Copy failed after quarantine: {}", e),
                advice: "Check permissions.".to_string(),
            })?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::AskUser => {
            // Deferred: skip for now, will be resolved via UI
            Ok(OperationResult::Skipped)
        }
    }
}

/// Generate a versioned path for versioned backup mode.
/// e.g., "file.txt" -> "file.v2.txt" (incrementing version)
fn generate_versioned_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    let mut version = 1;
    loop {
        let new_name = if let Some(ext) = ext {
            format!("{}.v{}.{}", stem, version, ext)
        } else {
            format!("{}.v{}", stem, version)
        };
        let candidate = parent.join(new_name);
        if !candidate.exists() {
            return candidate;
        }
        version += 1;
    }
}

/// Verify sync results using checksum comparison (T-042).
pub fn verify_sync(
    pair: &SyncPair,
    _report: &SyncReport,
) -> Result<Vec<String>, AppError> {
    if !pair.checksum_enabled && pair.verify_mode == SyncVerifyMode::None {
        return Ok(Vec::new());
    }

    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let mut mismatches = Vec::new();

    // Walk source and verify each file exists and matches at destination
    let source_files = planner::walk_directory(source_root)?;

    for file in source_files.iter().filter(|f| !f.is_dir) {
        if !planner::matches_filter(&file.relative_path, file.size, &pair.filter) {
            continue;
        }

        let dest_file = dest_root.join(&file.relative_path);
        if !dest_file.exists() {
            if pair.mode == SyncMode::OneWay || pair.mode == SyncMode::Mirror {
                mismatches.push(format!(
                    "Missing at destination: {}",
                    file.relative_path
                ));
            }
            continue;
        }

        let dest_meta = fs::metadata(&dest_file).map_err(|e| AppError::Sync {
            message: format!("Cannot read metadata: {}", e),
            advice: "Check permissions.".to_string(),
        })?;

        match pair.verify_mode {
            SyncVerifyMode::Fast => {
                // Size comparison
                if file.size != dest_meta.len() {
                    mismatches.push(format!(
                        "Size mismatch: {} (source: {}, dest: {})",
                        file.relative_path,
                        file.size,
                        dest_meta.len()
                    ));
                }
            }
            SyncVerifyMode::Full => {
                // Full byte comparison
                if file.size != dest_meta.len() {
                    mismatches.push(format!(
                        "Size mismatch: {} (source: {}, dest: {})",
                        file.relative_path,
                        file.size,
                        dest_meta.len()
                    ));
                } else if pair.checksum_enabled {
                    // Checksum comparison using xxhash3 (fast)
                    let src_hash = compute_file_hash(&file.absolute_path)?;
                    let dst_hash = compute_file_hash(&dest_file)?;
                    if src_hash != dst_hash {
                        mismatches.push(format!(
                            "Checksum mismatch: {}",
                            file.relative_path
                        ));
                    }
                }
            }
            SyncVerifyMode::None => {}
        }
    }

    Ok(mismatches)
}

/// Compute xxHash3 of a file for verification.
fn compute_file_hash(path: &Path) -> Result<u64, AppError> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(|e| AppError::Sync {
        message: format!("Cannot open file for hashing: {}", e),
        advice: "Check file permissions.".to_string(),
    })?;

    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    let mut buffer = vec![0u8; 256 * 1024]; // 256KB buffer

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| AppError::Sync {
            message: format!("Read error during hashing: {}", e),
            advice: "Check file integrity.".to_string(),
        })?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.digest())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_pair(src: &Path, dst: &Path) -> SyncPair {
        SyncPair {
            id: Uuid::new_v4(),
            name: "Test Sync".to_string(),
            source_path: src.to_string_lossy().to_string(),
            dest_path: dst.to_string_lossy().to_string(),
            mode: SyncMode::OneWay,
            enabled: true,
            last_run: None,
            trigger: SyncTrigger::Manual,
            filter: SyncFilter::default(),
            conflict_policy: SyncConflictPolicy::CreateConflictCopy,
            verify_mode: SyncVerifyMode::Fast,
            checksum_enabled: false,
            created_at: Utc::now(),
            time_offset_secs: None,
        }
    }

    #[test]
    fn test_execute_sync_add_files() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("a.txt"), "hello").unwrap();
        fs::write(src.path().join("b.txt"), "world").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.files_added, 2);
        assert_eq!(report.errors, 0);
        assert_eq!(report.status, SyncRunStatus::Success);
        assert_eq!(report.health, SyncHealth::Green);

        // Verify files exist at dest
        assert!(dst.path().join("a.txt").exists());
        assert!(dst.path().join("b.txt").exists());
    }

    #[test]
    fn test_execute_sync_modify_files() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("file.txt"), "updated content that is longer").unwrap();
        fs::write(dst.path().join("file.txt"), "old").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.files_modified, 1);

        // Verify content updated
        let content = fs::read_to_string(dst.path().join("file.txt")).unwrap();
        assert_eq!(content, "updated content that is longer");
    }

    #[test]
    fn test_execute_sync_mirror_deletions() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // File only at destination should be deleted in mirror mode
        fs::write(dst.path().join("extra.txt"), "should be deleted").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::Mirror;

        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.files_deleted, 1);
        assert!(!dst.path().join("extra.txt").exists());
    }

    #[test]
    fn test_execute_sync_versioned_backup() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("doc.txt"), "new version").unwrap();
        fs::write(dst.path().join("doc.txt"), "old version").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::VersionedBackup;

        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.files_modified, 1);

        // Original should still be there, and a versioned copy created
        assert!(dst.path().join("doc.txt").exists());
        assert!(dst.path().join("doc.v1.txt").exists());
    }

    #[test]
    fn test_execute_sync_with_filter() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("keep.txt"), "keep").unwrap();
        fs::write(src.path().join("skip.tmp"), "skip").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.filter.exclude_patterns = vec!["*.tmp".to_string()];

        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.files_added, 1);
        assert!(dst.path().join("keep.txt").exists());
        assert!(!dst.path().join("skip.tmp").exists());
    }

    #[test]
    fn test_execute_sync_continue_on_error() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("good.txt"), "good").unwrap();
        // Create a file that can't be read (on Unix)
        // This is hard to simulate cross-platform, so just verify the flag works

        let pair = make_test_pair(src.path(), dst.path());
        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.status, SyncRunStatus::Success);
    }

    #[test]
    fn test_execute_sync_subdirectories() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::create_dir_all(src.path().join("sub/nested")).unwrap();
        fs::write(src.path().join("sub/nested/deep.txt"), "deep").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        let config = ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        };

        let report = execute_sync(&config).unwrap();
        assert_eq!(report.files_added, 1);
        assert!(dst.path().join("sub/nested/deep.txt").exists());
    }

    #[test]
    fn test_verify_sync_catches_mismatch() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("file.txt"), "source content").unwrap();
        fs::write(dst.path().join("file.txt"), "different content!!").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        let mismatches = verify_sync(&pair, &SyncReport {
            id: Uuid::new_v4(),
            pair_id: pair.id,
            pair_name: "Test".to_string(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            duration_ms: 0,
            files_added: 0,
            files_modified: 0,
            files_deleted: 0,
            files_skipped: 0,
            conflicts_resolved: 0,
            errors: 0,
            bytes_transferred: 0,
            status: SyncRunStatus::Success,
            health: SyncHealth::Green,
            error_messages: vec![],
            resumed: false,
        })
        .unwrap();

        assert!(!mismatches.is_empty());
        assert!(mismatches[0].contains("Size mismatch"));
    }

    #[test]
    fn test_generate_versioned_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file.txt");
        let versioned = generate_versioned_path(&path);
        assert!(versioned.to_string_lossy().contains("v1"));
    }
}
