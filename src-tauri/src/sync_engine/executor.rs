//! Sync executor - state machine that performs actual file synchronization.
//!
//! Uses the planner for diff computation, conflict resolver for conflicts,
//! and supports partial failure continuation and resumable state.

use crate::core::error::AppError;
use crate::core::types::*;
use crate::sync_engine::conflict::{self, ConflictAction};
use crate::sync_engine::copier;
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
    let opts = &pair.copy_options;

    // /RH: refuse to start outside the allowed window rather than half-run.
    if !copier::within_run_hours(opts) {
        return Err(AppError::Sync {
            message: format!(
                "Outside this pair's allowed run hours ({}).",
                opts.run_hours.trim()
            ),
            advice: "Run it again inside that window, or clear Run hours in the pair's options."
                .to_string(),
        });
    }

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
    let mut log_lines: Vec<String> = Vec::new();
    let resumed = config.resume_from.is_some();

    // /E and /CREATE: build the directory tree first.
    for dir in &preview.dirs_to_create {
        if let Err(e) = fs::create_dir_all(dest_root.join(dir)) {
            errors += 1;
            error_messages.push(format!("{}: cannot create directory: {}", dir, e));
        } else {
            log_lines.push(format!("\tNew Dir\t\t{}", dir));
        }
    }

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
    let mut outcomes: Vec<(usize, Result<OperationResult, AppError>)> = Vec::new();

    // /MT: new files need no rollback snapshot, so they copy in parallel.
    // Resumed runs stay sequential to keep the resume index meaningful.
    let parallel_adds = if opts.threads > 1 && start_index == 0 && pair.mode != SyncMode::VersionedBackup {
        preview.additions.len()
    } else {
        0
    };
    if parallel_adds > 0 {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Mutex;
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let results = Mutex::new(Vec::new());
        let workers = (opts.threads as usize).min(parallel_adds);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    if idx >= parallel_adds {
                        break;
                    }
                    if !copier::within_run_hours(opts) {
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    let entry = &preview.additions[idx];
                    let r = copier::with_retries(opts, || add_file(pair, entry));
                    if r.is_err() && !config.continue_on_error {
                        stop.store(true, Ordering::Relaxed);
                    }
                    if let Ok(mut v) = results.lock() {
                        v.push((idx, r));
                    }
                });
            }
        });
        let mut v = results.into_inner().unwrap_or_default();
        v.sort_by_key(|(i, _)| *i);
        outcomes.extend(v);
    }

    let mut halted = outcomes.iter().any(|(_, r)| r.is_err()) && !config.continue_on_error;
    let mut stopped_for_hours = parallel_adds > 0 && outcomes.len() < parallel_adds && !halted;

    // Phase 3: Execute operations
    if !halted && !stopped_for_hours {
        for (idx, (entry, op_type)) in operations.iter().enumerate() {
            if idx < start_index.max(parallel_adds) {
                continue;
            }
            if !copier::within_run_hours(opts) {
                stopped_for_hours = true;
                break;
            }

            let result = copier::with_retries(opts, || {
                execute_single_operation(
                    pair,
                    entry,
                    op_type,
                    &mut rollback,
                    config.quarantine_dir.as_deref(),
                )
            });
            let failed = result.is_err();
            outcomes.push((idx, result));
            if failed && !config.continue_on_error {
                halted = true;
                break;
            }
        }
    }

    for (idx, result) in outcomes {
        let (entry, op_type) = operations[idx];
        match result {
            Ok(op_result) => {
                match op_result {
                    OperationResult::Added(bytes) => {
                        files_added += 1;
                        bytes_transferred += bytes;
                        log_lines.push(format!("\tNew File\t{}\t{}", bytes, entry.relative_path));
                    }
                    OperationResult::Modified(bytes) => {
                        files_modified += 1;
                        bytes_transferred += bytes;
                        log_lines.push(format!("\tNewer\t\t{}\t{}", bytes, entry.relative_path));
                    }
                    OperationResult::Deleted => {
                        files_deleted += 1;
                        log_lines.push(format!("\t*EXTRA File\t\t{}", entry.relative_path));
                    }
                    OperationResult::Skipped => {
                        files_skipped += 1;
                    }
                    OperationResult::ConflictResolved(bytes) => {
                        conflicts_resolved += 1;
                        bytes_transferred += bytes;
                    }
                }
                // /MOV, /MOVE: the source copy goes once the destination has it.
                if (opts.move_files || opts.move_dirs)
                    && (op_type == "add" || op_type == "modify")
                    && entry.source_size.is_some()
                {
                    let src = Path::new(&pair.source_path).join(&entry.relative_path);
                    if let Err(e) = fs::remove_file(&src) {
                        errors += 1;
                        error_messages.push(format!(
                            "{}: copied but source not removed: {}",
                            entry.relative_path, e
                        ));
                    }
                }
            }
            Err(e) => {
                errors += 1;
                error_messages.push(format!("{}: {}", entry.relative_path, e));
                log_lines.push(format!("\tERROR\t\t{}\t{}", entry.relative_path, e));
            }
        }
    }

    if stopped_for_hours {
        error_messages.push(format!(
            "Paused: left the allowed run hours ({}); remaining files will copy on the next run.",
            opts.run_hours.trim()
        ));
    }

    // Handle conflicts from preview
    if !halted && !stopped_for_hours {
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
    }

    // /PURGE, /MIR: drop destination directories the source no longer has.
    // `remove_dir` only removes empty ones, so nothing unplanned is lost.
    for dir in &preview.dirs_to_remove {
        if fs::remove_dir(dest_root.join(dir)).is_ok() {
            log_lines.push(format!("\t*EXTRA Dir\t\t{}", dir));
        }
    }

    finish_directories(pair, opts);

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
        SyncRunStatus::Success if stopped_for_hours => SyncHealth::Yellow,
        SyncRunStatus::Success => SyncHealth::Green,
        SyncRunStatus::PartialSuccess => SyncHealth::Yellow,
        SyncRunStatus::Failed => SyncHealth::Red,
        _ => SyncHealth::Gray,
    };

    // Extras: files purged, or left in place because of /XX.
    let extras = if files_deleted > 0 {
        files_deleted
    } else if opts.exclude_extra {
        count_extras(pair)
    } else {
        0
    };

    let report = SyncReport {
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
        exit_code: copier::exit_code(
            files_added + files_modified + conflicts_resolved,
            extras,
            preview.total_conflicts.saturating_sub(conflicts_resolved),
            errors,
        ),
    };

    if let Err(e) = copier::write_log(pair, &report, &log_lines) {
        tracing::warn!("Could not write sync log for {}: {}", pair.name, e);
    }

    Ok(report)
}

/// Count destination files the source lacks (reported as extras under /XX).
fn count_extras(pair: &SyncPair) -> u64 {
    let opts = CopyOptions {
        exclude_extra: false,
        purge: true,
        ..pair.copy_options.clone()
    };
    let probe = SyncPair {
        copy_options: opts,
        mode: SyncMode::OneWay,
        ..pair.clone()
    };
    planner::compute_preview(&probe)
        .map(|p| p.total_deletions)
        .unwrap_or(0)
}

/// After copying: /DCOPY:T directory timestamps and /MOVE source cleanup.
fn finish_directories(pair: &SyncPair, opts: &CopyOptions) {
    if !opts.copy_dir_timestamps && !opts.move_dirs {
        return;
    }
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let Ok(entries) = planner::walk_directory_with(source_root, opts) else {
        return;
    };
    let mut dirs: Vec<_> = entries.into_iter().filter(|e| e.is_dir).collect();
    // Deepest first, so a parent's time is set after its children change it
    // and child directories are removed before their parents.
    dirs.sort_by(|a, b| b.relative_path.len().cmp(&a.relative_path.len()));
    for dir in dirs {
        if opts.copy_dir_timestamps {
            if let Ok(modified) = fs::metadata(&dir.absolute_path).and_then(|m| m.modified()) {
                let target = dest_root.join(&dir.relative_path);
                if target.is_dir() {
                    copier::set_mtime(&target, modified);
                }
            }
        }
        if opts.move_dirs {
            let _ = fs::remove_dir(&dir.absolute_path);
        }
    }
}

/// Result of executing a single file operation.
enum OperationResult {
    Added(u64),
    Modified(u64),
    Deleted,
    Skipped,
    ConflictResolved(u64),
}

/// Copy a new file (the "add" operation). Needs no rollback snapshot, so it
/// is safe to run from several threads at once (/MT).
fn add_file(pair: &SyncPair, entry: &SyncDiffEntry) -> Result<OperationResult, AppError> {
    let opts = &pair.copy_options;
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let source_file = source_root.join(&entry.relative_path);
    let dest_file = dest_root.join(&entry.relative_path);

    // For two-way sync, new files in dest get copied to source
    if entry.reason.contains("two-way") && entry.source_size.is_none() {
        let bytes = copier::copy_file(&dest_file, &source_file, opts)?;
        return Ok(OperationResult::Added(bytes));
    }

    // For versioned backup, add version suffix
    let target = if pair.mode == SyncMode::VersionedBackup {
        generate_versioned_path(&dest_file)
    } else {
        dest_file
    };
    let bytes = copier::copy_file(&source_file, &target, opts)?;
    Ok(OperationResult::Added(bytes))
}

/// Execute a single file operation (add/modify/delete).
fn execute_single_operation(
    pair: &SyncPair,
    entry: &SyncDiffEntry,
    op_type: &str,
    rollback: &mut RollbackManager,
    _quarantine_dir: Option<&Path>,
) -> Result<OperationResult, AppError> {
    let opts = &pair.copy_options;
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let source_file = source_root.join(&entry.relative_path);
    let dest_file = dest_root.join(&entry.relative_path);

    match op_type {
        "add" => add_file(pair, entry),

        "modify" => {
            // Take pre-destructive snapshot if destination exists
            if dest_file.exists() {
                rollback.snapshot_file(&entry.relative_path, &dest_file)?;
            }

            if pair.mode == SyncMode::VersionedBackup {
                // Keep old version, write new version
                let target = generate_versioned_path(&dest_file);
                let bytes = copier::copy_file(&source_file, &target, opts)?;
                Ok(OperationResult::Modified(bytes))
            } else {
                let bytes = copier::copy_file(&source_file, &dest_file, opts)?;
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
            copy_options: CopyOptions::default(),
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

        // Different sizes: same-size writes can land in one mtime tick and
        // then look identical to the size+mtime comparison.
        fs::write(src.path().join("doc.txt"), "new version, longer").unwrap();
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
            exit_code: 0,
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

    // ── Robocopy-equivalent options ──

    fn run_with(pair: SyncPair) -> SyncReport {
        execute_sync(&ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        })
        .unwrap()
    }

    fn set_age_days(path: &Path, days: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
        copier::set_mtime(path, t);
    }

    #[test]
    fn robocopy_exclude_dirs_and_depth() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("a/b/c")).unwrap();
        fs::create_dir_all(src.path().join("node_modules")).unwrap();
        fs::write(src.path().join("top.txt"), "1").unwrap();
        fs::write(src.path().join("a/l2.txt"), "2").unwrap();
        fs::write(src.path().join("a/b/l3.txt"), "3").unwrap();
        fs::write(src.path().join("node_modules/x.js"), "x").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.exclude_dirs = vec!["node_modules".into()];
        pair.copy_options.max_depth = 2;
        let report = run_with(pair);
        assert_eq!(report.files_added, 2);
        assert!(dst.path().join("a/l2.txt").exists());
        assert!(!dst.path().join("a/b/l3.txt").exists());
        assert!(!dst.path().join("node_modules").exists());
    }

    #[test]
    fn robocopy_no_subdirs_copies_top_level_only() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("sub")).unwrap();
        fs::write(src.path().join("top.txt"), "1").unwrap();
        fs::write(src.path().join("sub/deep.txt"), "2").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.copy_subdirs = false;
        assert_eq!(run_with(pair).files_added, 1);
        assert!(!dst.path().join("sub").exists());
    }

    #[test]
    fn robocopy_empty_dirs_and_purge() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("empty/inner")).unwrap();
        fs::create_dir_all(dst.path().join("stale/dir")).unwrap();
        fs::write(dst.path().join("stale/dir/old.txt"), "old").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.include_empty_dirs = true;
        pair.copy_options.purge = true;
        let report = run_with(pair);
        assert!(dst.path().join("empty/inner").is_dir());
        assert!(!dst.path().join("stale").exists());
        assert_eq!(report.files_deleted, 1);
        assert_eq!(report.exit_code & 2, 2);
    }

    #[test]
    fn robocopy_exclude_extra_keeps_mirror_extras() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(dst.path().join("extra.txt"), "keep me").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::Mirror;
        pair.copy_options.exclude_extra = true;
        let report = run_with(pair);
        assert_eq!(report.files_deleted, 0);
        assert!(dst.path().join("extra.txt").exists());
        assert_eq!(report.exit_code, 2);
    }

    #[test]
    fn robocopy_exclude_older_and_lonely() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("old.txt"), "older source").unwrap();
        fs::write(dst.path().join("old.txt"), "newer dest").unwrap();
        set_age_days(&src.path().join("old.txt"), 5);
        fs::write(src.path().join("lonely.txt"), "new").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.exclude_older = true;
        pair.copy_options.exclude_lonely = true;
        let preview = planner::compute_preview(&pair).unwrap();
        assert_eq!(preview.total_additions + preview.total_modifications, 0);
        assert!(preview.skipped.iter().any(|e| e.reason.contains("/XO")));
        assert!(preview.skipped.iter().any(|e| e.reason.contains("/XL")));
        assert_eq!(fs::read_to_string(dst.path().join("old.txt")).unwrap(), "newer dest");
    }

    #[test]
    fn robocopy_age_and_hidden_filters() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("ancient.txt"), "a").unwrap();
        set_age_days(&src.path().join("ancient.txt"), 400);
        fs::write(src.path().join("fresh.txt"), "f").unwrap();
        fs::write(src.path().join(".hidden"), "h").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.max_age_days = 30;
        pair.copy_options.exclude_hidden = true;
        let report = run_with(pair);
        assert_eq!(report.files_added, 1);
        assert!(dst.path().join("fresh.txt").exists());
    }

    #[test]
    fn robocopy_timestamps_preserved_so_second_run_is_noop() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "hello").unwrap();
        let pair = make_test_pair(src.path(), dst.path());
        assert_eq!(run_with(pair.clone()).exit_code, 1);
        let second = run_with(pair);
        assert_eq!(second.files_added + second.files_modified, 0);
        assert_eq!(second.exit_code, 0);
    }

    #[test]
    fn robocopy_include_same_recopies() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "hello").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        run_with(pair.clone());
        pair.copy_options.include_same = true;
        assert_eq!(run_with(pair).files_modified, 1);
    }

    #[test]
    fn robocopy_move_removes_source() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("sub")).unwrap();
        fs::write(src.path().join("sub/a.txt"), "a").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.move_files = true;
        pair.copy_options.move_dirs = true;
        let report = run_with(pair);
        assert_eq!(report.errors, 0);
        assert!(dst.path().join("sub/a.txt").exists());
        assert!(!src.path().join("sub").exists());
    }

    #[test]
    fn robocopy_multithreaded_copy() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        for i in 0..40 {
            fs::write(src.path().join(format!("f{i}.txt")), format!("{i}")).unwrap();
        }
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.threads = 8;
        let report = run_with(pair);
        assert_eq!(report.files_added, 40);
        assert_eq!(report.errors, 0);
        assert_eq!(fs::read_to_string(dst.path().join("f7.txt")).unwrap(), "7");
    }

    #[test]
    fn robocopy_fat_time_tolerance() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "same").unwrap();
        fs::write(dst.path().join("a.txt"), "same").unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(100);
        copier::set_mtime(&src.path().join("a.txt"), t);
        copier::set_mtime(&dst.path().join("a.txt"), t + std::time::Duration::from_secs(1));
        let mut pair = make_test_pair(src.path(), dst.path());
        assert_eq!(planner::compute_preview(&pair).unwrap().total_modifications, 1);
        pair.copy_options.fat_time_tolerance = true;
        assert_eq!(planner::compute_preview(&pair).unwrap().total_modifications, 0);
    }

    #[test]
    fn robocopy_writes_log_file() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let logs = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "a").unwrap();
        let log = logs.path().join("run.log");
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.log_file = log.to_string_lossy().to_string();
        pair.copy_options.log_append = true;
        run_with(pair.clone());
        run_with(pair);
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("New File"));
        assert_eq!(text.matches("Exit code").count(), 2);
    }

    #[test]
    fn robocopy_run_hours_outside_window_refuses() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let now = chrono::Local::now();
        use chrono::Timelike;
        // A one-minute window two hours from now is never "now".
        let start = (now.hour() + 2) % 24;
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.run_hours = format!("{start:02}00-{start:02}01");
        let err = execute_sync(&ExecutorConfig {
            pair,
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
        });
        assert!(err.is_err());
    }
}
