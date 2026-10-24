//! Sync executor - state machine that performs actual file synchronization.
//!
//! Uses the planner for diff computation, conflict resolver for conflicts,
//! and supports partial failure continuation and resumable state.

use crate::error::AppError;
use crate::sync::conflict::{self, ConflictAction};
use crate::sync::copier;
use crate::sync::planner;
use crate::sync::rollback::RollbackManager;
use crate::sync_types::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Live progress of a running sync, polled by the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncProgress {
    pub pair_id: Uuid,
    pub run_id: Uuid,
    /// "planning", "copying" or "finishing"
    pub phase: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current_file: Option<String>,
    pub started_at: chrono::DateTime<Utc>,
    pub cancel_requested: bool,
}

/// Callback receiving progress snapshots (called from worker threads).
pub type ProgressFn = Arc<dyn Fn(&SyncProgress) + Send + Sync>;

/// Optional observers for a run: a progress callback and a cancel flag.
#[derive(Default, Clone)]
pub struct RunHooks {
    pub progress: Option<ProgressFn>,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl RunHooks {
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(false)
    }
}

/// Everything a run produced besides the report: rollback snapshots, files
/// moved to quarantine, and conflicts waiting for the user ("Ask" policy).
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub report: SyncReport,
    pub snapshots: Vec<SyncSnapshot>,
    pub quarantined: Vec<QuarantineEntry>,
    pub pending_conflicts: Vec<SyncConflictItem>,
}

/// Thread-safe progress counters shared by parallel copy workers.
struct Tracker<'a> {
    hooks: &'a RunHooks,
    base: SyncProgress,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    current: Mutex<Option<String>>,
}

impl Tracker<'_> {
    fn emit(&self, phase: &str) {
        if let Some(cb) = &self.hooks.progress {
            let mut p = self.base.clone();
            p.phase = phase.to_string();
            p.files_done = self.files_done.load(Ordering::Relaxed);
            p.bytes_done = self.bytes_done.load(Ordering::Relaxed);
            p.current_file = self.current.lock().ok().and_then(|c| c.clone());
            p.cancel_requested = self.hooks.cancelled();
            cb(&p);
        }
    }

    fn start_file(&self, path: &str) {
        if let Ok(mut c) = self.current.lock() {
            *c = Some(path.to_string());
        }
        self.emit("copying");
    }

    fn finish_file(&self, bytes: u64) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
        self.bytes_done.fetch_add(bytes, Ordering::Relaxed);
        self.emit("copying");
    }
}

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
    execute_sync_with(config, &RunHooks::default()).map(|o| o.report)
}

/// Execute a sync run with progress reporting and cancellation.
pub fn execute_sync_with(
    config: &ExecutorConfig,
    hooks: &RunHooks,
) -> Result<RunOutcome, AppError> {
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
    let planning = SyncProgress {
        pair_id: pair.id,
        run_id: config.run_id,
        phase: "planning".to_string(),
        files_done: 0,
        files_total: 0,
        bytes_done: 0,
        bytes_total: 0,
        current_file: None,
        started_at,
        cancel_requested: false,
    };
    if let Some(cb) = &hooks.progress {
        cb(&planning);
    }
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
        // Never inside the destination: a mirror run would treat the
        // snapshots as extra files and delete them.
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("ufop-sync-rollback")
                .join(pair.id.to_string())
                .join(config.run_id.to_string())
        });
    let mut rollback = RollbackManager::new(config.run_id, pair.id, &rollback_dir);

    let mut files_added: u64 = 0;
    let mut files_modified: u64 = 0;
    let mut files_deleted: u64 = 0;
    // Files the plan left alone (filters, /XO, /XL…) count as skipped too,
    // so the report agrees with the preview.
    let mut files_skipped: u64 = preview.total_skipped;
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
    let tracker = Tracker {
        hooks,
        base: SyncProgress {
            files_total: (operations.len() + preview.conflicts.len()) as u64,
            bytes_total: preview.total_bytes,
            ..planning
        },
        files_done: AtomicU64::new(0),
        bytes_done: AtomicU64::new(0),
        current: Mutex::new(None),
    };
    tracker.emit("copying");
    let mut was_cancelled: bool;
    let mut quarantined: Vec<QuarantineEntry> = Vec::new();
    let mut pending_conflicts: Vec<SyncConflictItem> = Vec::new();
    let mut outcomes: Vec<(usize, Result<OperationResult, AppError>)> = Vec::new();

    // /MT: new files need no rollback snapshot, so they copy in parallel.
    // Resumed runs stay sequential to keep the resume index meaningful.
    let parallel_adds =
        if opts.threads > 1 && start_index == 0 && pair.mode != SyncMode::VersionedBackup {
            preview.additions.len()
        } else {
            0
        };
    if parallel_adds > 0 {
        use std::sync::atomic::AtomicUsize;
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let results = Mutex::new(Vec::new());
        let workers = (opts.threads as usize).min(parallel_adds);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    if stop.load(Ordering::Relaxed) || hooks.cancelled() {
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
                    tracker.start_file(&entry.relative_path);
                    let r = copier::with_retries(opts, || add_file(pair, entry));
                    tracker.finish_file(match &r {
                        Ok(OperationResult::Added(b)) => *b,
                        _ => 0,
                    });
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
    was_cancelled = hooks.cancelled();
    let mut stopped_for_hours =
        parallel_adds > 0 && outcomes.len() < parallel_adds && !halted && !was_cancelled;

    // Phase 3: Execute operations
    if !halted && !stopped_for_hours && !was_cancelled {
        for (idx, (entry, op_type)) in operations.iter().enumerate() {
            if idx < start_index.max(parallel_adds) {
                continue;
            }
            if hooks.cancelled() {
                was_cancelled = true;
                break;
            }
            if !copier::within_run_hours(opts) {
                stopped_for_hours = true;
                break;
            }

            tracker.start_file(&entry.relative_path);
            let result = copier::with_retries(opts, || {
                execute_single_operation(
                    pair,
                    entry,
                    op_type,
                    &mut rollback,
                    config.quarantine_dir.as_deref(),
                )
            });
            tracker.finish_file(match &result {
                Ok(OperationResult::Added(b)) | Ok(OperationResult::Modified(b)) => *b,
                _ => 0,
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

    if was_cancelled {
        let done = tracker.files_done.load(Ordering::Relaxed);
        error_messages.push(format!(
            "Cancelled after {} of {} items; run again to finish.",
            done, tracker.base.files_total
        ));
    }

    // Handle conflicts from preview
    if !halted && !stopped_for_hours && !was_cancelled {
        for entry in &preview.conflicts {
            if hooks.cancelled() {
                was_cancelled = true;
                break;
            }
            tracker.start_file(&entry.relative_path);
            let result = execute_conflict_resolution(
                pair,
                entry,
                &mut rollback,
                config.quarantine_dir.as_deref(),
                &mut quarantined,
                &mut pending_conflicts,
            );
            tracker.finish_file(match &result {
                Ok(OperationResult::ConflictResolved(b)) => *b,
                _ => 0,
            });

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

    // /TIMFIX, /SECFIX: bring unchanged files' timestamps and security in line.
    if !was_cancelled {
        for rel in &preview.fixups {
            let src = Path::new(&pair.source_path).join(rel);
            let dst = dest_root.join(rel);
            if opts.fix_timestamps {
                if let Ok(modified) = fs::metadata(&src).and_then(|m| m.modified()) {
                    copier::set_mtime(&dst, modified);
                }
            }
            if opts.fix_security {
                let security_only = CopyOptions {
                    archive_reset: false,
                    add_attributes: String::new(),
                    remove_attributes: String::new(),
                    ..opts.clone()
                };
                if let Err(e) = super::winattr::after_copy(&src, &dst, &security_only) {
                    errors += 1;
                    error_messages.push(format!("{}: {}", rel, e));
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

    if let Ok(mut c) = tracker.current.lock() {
        *c = None;
    }
    tracker.emit("finishing");
    finish_directories(pair, opts);

    let ended_at = Utc::now();
    let duration_ms = (ended_at - started_at).num_milliseconds().max(0) as u64;

    // Determine status
    let status = if was_cancelled {
        SyncRunStatus::Cancelled
    } else if errors == 0 {
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
        SyncRunStatus::Cancelled => SyncHealth::Yellow,
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

    if report.status == SyncRunStatus::Success {
        if let Err(e) = write_completion_marker(pair, &report) {
            tracing::warn!("Could not write the completion marker for {}: {e}", pair.name);
        }
    }

    // Record the destination state for trusted-inventory runs.
    if pair.dest_state == DestStateMode::TrustedInventory && report.errors == 0 && !hooks.cancelled() {
        let state = if pair.mode == SyncMode::TwoWay {
            planner::walk_directory_with(dest_root, opts)
        } else {
            // One-way / mirror: the destination now holds the selected source files.
            planner::walk_directory_with(Path::new(&pair.source_path), opts).map(|v| {
                v.into_iter().filter(|f| !f.is_dir && planner::matches_filter(&f.relative_path, f.size, &pair.filter)).collect()
            })
        };
        if let Err(e) = state.map_err(|e| std::io::Error::other(e.to_string())).and_then(|s| planner::save_inventory(dest_root, &s)) {
            tracing::warn!("Could not record the destination inventory of {}: {e}", pair.name);
        }
    }

    if let Err(e) = copier::write_log(pair, &report, &log_lines) {
        tracing::warn!("Could not write sync log for {}: {}", pair.name, e);
    }

    Ok(RunOutcome {
        report,
        snapshots: rollback.snapshots,
        quarantined,
        pending_conflicts,
    })
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
    dirs.sort_by_key(|d| std::cmp::Reverse(d.relative_path.len()));
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
        versioned_path(&dest_file, pair.run_options.version_template.as_deref())?
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
                let target = versioned_path(&dest_file, pair.run_options.version_template.as_deref())?;
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
#[allow(clippy::too_many_arguments)]
fn execute_conflict_resolution(
    pair: &SyncPair,
    entry: &SyncDiffEntry,
    rollback: &mut RollbackManager,
    quarantine_dir: Option<&Path>,
    quarantined: &mut Vec<QuarantineEntry>,
    pending: &mut Vec<SyncConflictItem>,
) -> Result<OperationResult, AppError> {
    let opts = &pair.copy_options;
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
            let bytes = copier::copy_file(&source_file, &dest_file, opts)?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::KeepDest => Ok(OperationResult::Skipped),
        ConflictAction::CopyToConflictPath => {
            let bytes = copier::copy_file(&source_file, &result.resolved_path, opts)?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::Skip => Ok(OperationResult::Skipped),
        ConflictAction::Quarantine => {
            if dest_file.exists() {
                conflict::quarantine_file(&dest_file, &result.resolved_path)?;
                quarantined.push(QuarantineEntry {
                    id: Uuid::new_v4(),
                    pair_id: pair.id,
                    original_path: entry.relative_path.clone(),
                    quarantine_path: result.resolved_path.to_string_lossy().to_string(),
                    source_size: entry.source_size.unwrap_or(0),
                    dest_size: entry.dest_size.unwrap_or(0),
                    source_modified: entry.source_modified,
                    dest_modified: entry.dest_modified,
                    quarantined_at: Utc::now(),
                    resolved: false,
                });
            }
            let bytes = copier::copy_file(&source_file, &dest_file, opts)?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::AskUser => {
            // Left untouched until the user picks a resolution in the UI.
            pending.push(SyncConflictItem {
                id: Uuid::new_v4(),
                pair_id: pair.id,
                relative_path: entry.relative_path.clone(),
                source_size: entry.source_size.unwrap_or(0),
                dest_size: entry.dest_size.unwrap_or(0),
                source_modified: entry.source_modified,
                dest_modified: entry.dest_modified,
                resolution: None,
                created_at: Utc::now(),
            });
            Ok(OperationResult::Skipped)
        }
    }
}

/// Apply a resolution the user picked for an "Ask" conflict. Returns the
/// rollback snapshots and any quarantine entry the resolution produced.
pub fn apply_manual_resolution(
    pair: &SyncPair,
    item: &SyncConflictItem,
    policy: SyncConflictPolicy,
    quarantine_dir: &Path,
    rollback_dir: &Path,
) -> Result<(Vec<SyncSnapshot>, Vec<QuarantineEntry>), AppError> {
    if policy == SyncConflictPolicy::Ask {
        return Err(AppError::Sync {
            message: "Choose how to resolve this conflict.".to_string(),
            advice:
                "Pick Source wins, Destination wins, Newest wins, Keep both, Skip or Quarantine."
                    .to_string(),
        });
    }
    let run_id = Uuid::new_v4();
    let resolving = SyncPair {
        conflict_policy: policy,
        ..pair.clone()
    };
    let entry = SyncDiffEntry {
        relative_path: item.relative_path.clone(),
        action: SyncAction::Conflict,
        source_size: Some(item.source_size),
        dest_size: Some(item.dest_size),
        source_modified: item.source_modified,
        dest_modified: item.dest_modified,
        reason: "Manual resolution".to_string(),
        path_length_warning: None,
    };
    let mut rollback =
        RollbackManager::new(run_id, pair.id, &rollback_dir.join(run_id.to_string()));
    let mut quarantined = Vec::new();
    let mut pending = Vec::new();
    execute_conflict_resolution(
        &resolving,
        &entry,
        &mut rollback,
        Some(quarantine_dir),
        &mut quarantined,
        &mut pending,
    )?;
    Ok((rollback.snapshots, quarantined))
}

/// Default name for kept versions: `file.txt` -> `file.v2.txt`.
pub const DEFAULT_VERSION_TEMPLATE: &str = "{name}.v{num}.{ext}";

/// Name for the next kept version of `path` from the pair's template; the
/// first free one. A template without `{num}` gets ` (n)` added on clashes.
fn versioned_path(path: &Path, template: Option<&str>) -> Result<PathBuf, AppError> {
    use crate::naming::{render, sanitize_segment, TokenContext, UnknownTokens};
    let template = template.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(DEFAULT_VERSION_TEMPLATE);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let has_ext = path.extension().is_some();
    let mut base = TokenContext::for_path(path);
    // A first version has no file yet: date tokens use the run time.
    if base.modified.is_none() {
        base.modified = Some(Utc::now());
    }
    let numbered = template.contains("{num") || template.contains("{counter");
    for n in 1..100_000u64 {
        let mut name = render(template, &base.clone().with_counter(n, 0), UnknownTokens::Error).map_err(|e| AppError::Sync {
            message: format!("Version name template \"{template}\" is invalid: {e}"),
            advice: "Fix the version naming template in the pair's options.".to_string(),
        })?;
        // "{name}.{ext}" on a file without an extension would end in a dot.
        if !has_ext {
            name = name.trim_end_matches('.').to_string();
        }
        if !numbered && n > 1 {
            name = match name.rsplit_once('.') {
                Some((stem, ext)) if has_ext => format!("{stem} ({n}).{ext}"),
                _ => format!("{name} ({n})"),
            };
        }
        let candidate = parent.join(sanitize_segment(name));
        if candidate != path && !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(AppError::Sync {
        message: format!("No free version name for {}", path.display()),
        advice: "Clean up old versions at the destination.".to_string(),
    })
}

/// Reject run options that would fail every run: a broken version
/// template or a marker name that isn't a plain file name.
pub fn validate_run_options(o: &crate::sync_types::SyncRunOptions) -> Result<(), AppError> {
    if let Some(t) = o.version_template.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let ctx = crate::naming::TokenContext {
            file_name: "file.txt".into(),
            stem: "file".into(),
            ext: "txt".into(),
            modified: Some(Utc::now()),
            ..Default::default()
        }
        .with_counter(1, 0);
        crate::naming::render(t, &ctx, crate::naming::UnknownTokens::Error).map_err(|e| AppError::Sync {
            message: format!("Version name template \"{t}\" is invalid: {e}"),
            advice: "Use tokens like {name}, {ext}, {num} and {now:%Y-%m-%d}.".to_string(),
        })?;
    }
    o.notify.validate().map_err(|message| AppError::Sync { message, advice: "Pick a notification profile and results of success, warning or error.".to_string() })?;
    if let Some(m) = o.completion_marker.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        if m.contains(['/', '\\']) || m == "." || m == ".." || planner::is_internal_entry(m) {
            return Err(AppError::Sync {
                message: format!("Completion marker \"{m}\" must be a plain file name"),
                advice: "Use a name like _SYNC_COMPLETE; it is written at the destination root.".to_string(),
            });
        }
    }
    Ok(())
}

/// Write the pair's completion marker at the destination root.
fn write_completion_marker(pair: &SyncPair, report: &SyncReport) -> std::io::Result<()> {
    let Some(name) = pair.run_options.completion_marker.as_deref().map(str::trim).filter(|m| !m.is_empty()) else {
        return Ok(());
    };
    let summary = serde_json::json!({
        "pair": pair.name,
        "run_id": report.id,
        "finished_at": report.ended_at,
        "files_added": report.files_added,
        "files_modified": report.files_modified,
        "files_deleted": report.files_deleted,
        "bytes_transferred": report.bytes_transferred,
    });
    let path = Path::new(&pair.dest_path).join(crate::naming::sanitize_segment(name.to_string()));
    crate::fsutil::write_private_atomic(&path, &serde_json::to_vec_pretty(&summary).map_err(std::io::Error::other)?)
}

/// Verify sync results using checksum comparison (T-042).
pub fn verify_sync(pair: &SyncPair, _report: &SyncReport) -> Result<Vec<String>, AppError> {
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
                mismatches.push(format!("Missing at destination: {}", file.relative_path));
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
                    let src_hash = compute_file_hash(&file.absolute_path, pair.checksum_algorithm)?;
                    let dst_hash = compute_file_hash(&dest_file, pair.checksum_algorithm)?;
                    if src_hash != dst_hash {
                        mismatches.push(format!("Checksum mismatch: {}", file.relative_path));
                    }
                }
            }
            SyncVerifyMode::None => {}
        }
    }

    Ok(mismatches)
}

/// Hash a file for verification (xxHash3 or SHA-256, hex).
pub fn compute_file_hash(path: &Path, alg: SyncHashAlgorithm) -> Result<String, AppError> {
    use sha2::Digest;
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(|e| AppError::Sync {
        message: format!("Cannot open file for hashing: {}", e),
        advice: "Check file permissions.".to_string(),
    })?;

    let mut xx = xxhash_rust::xxh3::Xxh3::new();
    let mut sha = sha2::Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024]; // 256KB buffer

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| AppError::Sync {
            message: format!("Read error during hashing: {}", e),
            advice: "Check file integrity.".to_string(),
        })?;
        if bytes_read == 0 {
            break;
        }
        match alg {
            SyncHashAlgorithm::Xxh3 => xx.update(&buffer[..bytes_read]),
            SyncHashAlgorithm::Sha256 => sha.update(&buffer[..bytes_read]),
        }
    }

    Ok(match alg {
        SyncHashAlgorithm::Xxh3 => format!("{:016x}", xx.digest()),
        SyncHashAlgorithm::Sha256 => format!("{:x}", sha.finalize()),
    })
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
            ..Default::default()
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

        fs::write(
            src.path().join("file.txt"),
            "updated content that is longer",
        )
        .unwrap();
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
        let mismatches = verify_sync(
            &pair,
            &SyncReport {
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
            },
        )
        .unwrap();

        assert!(!mismatches.is_empty());
        assert!(mismatches[0].contains("Size mismatch"));
    }

    #[test]
    fn test_generate_versioned_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file.txt");
        assert_eq!(versioned_path(&path, None).unwrap(), dir.path().join("file.v1.txt"));
        fs::write(dir.path().join("file.v1.txt"), "x").unwrap();
        assert_eq!(versioned_path(&path, Some("  ")).unwrap(), dir.path().join("file.v2.txt"));
        let bare = dir.path().join("README");
        assert_eq!(versioned_path(&bare, None).unwrap(), dir.path().join("README.v1"));
    }

    #[test]
    fn version_template_names_and_clashes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("report.pdf");
        fs::write(&path, "x").unwrap();
        let t = Some("{name} (old {num:3}).{ext}");
        assert_eq!(versioned_path(&path, t).unwrap(), dir.path().join("report (old 001).pdf"));
        // No counter in the template: clashes get " (n)".
        let dated = Some("{name}-{now:%Y}.{ext}");
        let first = versioned_path(&path, dated).unwrap();
        fs::write(&first, "x").unwrap();
        let second = versioned_path(&path, dated).unwrap();
        assert!(second.file_name().unwrap().to_string_lossy().ends_with(" (2).pdf"), "{second:?}");
        // Slashes can't escape the folder; unknown tokens are errors.
        assert_eq!(versioned_path(&path, Some("../{name}.{ext}")).unwrap().parent().unwrap(), dir.path());
        assert!(versioned_path(&path, Some("{nmae}.{ext}")).is_err());
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
    fn settle_window_defers_files_still_being_written() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("old.txt"), "done").unwrap();
        set_age_days(&src.path().join("old.txt"), 1);
        fs::write(src.path().join("growing.log"), "partial").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.run_options.settle_secs = 60;
        let preview = planner::compute_preview(&pair).unwrap();
        assert!(preview.skipped.iter().any(|e| e.relative_path == "growing.log" && e.reason.starts_with("Still changing")));
        let report = run_with(pair.clone());
        assert_eq!(report.files_added, 1);
        assert!(dst.path().join("old.txt").exists() && !dst.path().join("growing.log").exists());
        // Once it has settled, the next run copies it.
        set_age_days(&src.path().join("growing.log"), 1);
        assert_eq!(run_with(pair).files_added, 1);
        assert!(dst.path().join("growing.log").exists());
    }

    #[test]
    fn completion_marker_written_on_success_and_never_purged() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "a").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::Mirror;
        pair.run_options.completion_marker = Some("_SYNC_COMPLETE".into());
        let report = run_with(pair.clone());
        assert_eq!(report.status, SyncRunStatus::Success);
        let marker: serde_json::Value = serde_json::from_slice(&fs::read(dst.path().join("_SYNC_COMPLETE")).unwrap()).unwrap();
        assert_eq!(marker["files_added"], 1);
        assert_eq!(marker["run_id"], serde_json::json!(report.id));
        // A mirror run must not delete it as an extra; it is rewritten.
        let again = run_with(pair);
        assert_eq!(again.files_deleted, 0);
        assert!(dst.path().join("_SYNC_COMPLETE").exists());
    }

    #[test]
    fn filename_policy_blocks_files_in_sync() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("ok.txt"), "a").unwrap();
        fs::write(src.path().join("bad.zz-sync-policy"), "b").unwrap();
        // Unique extension: the active set is process-wide.
        crate::policy::set_active(
            crate::policy::PolicySet::new(vec![crate::policy::FilenameRule {
                blocked_extensions: vec!["zz-sync-policy".into()],
                ..crate::policy::FilenameRule::new("t", "")
            }])
            .unwrap(),
        );
        let pair = make_test_pair(src.path(), dst.path());
        let preview = planner::compute_preview(&pair).unwrap();
        let report = run_with(pair);
        crate::policy::set_active(crate::policy::PolicySet::default());
        assert!(preview.skipped.iter().any(|e| e.relative_path == "bad.zz-sync-policy" && e.reason.starts_with("Blocked by filename policy")));
        assert_eq!(report.files_added, 1);
        assert!(!dst.path().join("bad.zz-sync-policy").exists());
    }

    #[test]
    fn run_options_validation() {
        use crate::sync_types::SyncRunOptions;
        let ok = SyncRunOptions { settle_secs: 5, completion_marker: Some("_DONE".into()), version_template: Some("{name}.{now:%Y}.{ext}".into()), ..Default::default() };
        assert!(validate_run_options(&ok).is_ok());
        assert!(validate_run_options(&SyncRunOptions { version_template: Some("{nope}".into()), ..Default::default() }).is_err());
        for bad in ["a/b", "..", ".ufop-sync-inventory.json"] {
            assert!(validate_run_options(&SyncRunOptions { completion_marker: Some(bad.into()), ..Default::default() }).is_err(), "{bad}");
        }
        // Result notifications: a known result and a profile to send to.
        let notify = |p: Option<&str>, on: &[&str]| SyncRunOptions { notify: crate::run_notify::RunNotify { notify_profile_id: p.map(Into::into), notify_on: on.iter().map(|s| s.to_string()).collect(), min_bytes: None }, ..Default::default() };
        assert!(validate_run_options(&notify(Some("p"), &["error"])).is_ok());
        assert!(validate_run_options(&notify(Some("p"), &["boom"])).is_err());
        assert!(validate_run_options(&notify(None, &["error"])).is_err());
    }

    #[test]
    fn versioned_backup_uses_template() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("doc.txt"), "v1").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::VersionedBackup;
        pair.run_options.version_template = Some("{name}~{num}.{ext}".into());
        run_with(pair.clone());
        assert!(dst.path().join("doc~1.txt").exists());
        pair.run_options.version_template = Some("{bogus}".into());
        fs::write(src.path().join("new.txt"), "n").unwrap();
        let bad = run_with(pair);
        assert!(bad.errors > 0 && bad.error_messages.iter().any(|m| m.contains("template")));
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
        assert_eq!(
            fs::read_to_string(dst.path().join("old.txt")).unwrap(),
            "newer dest"
        );
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
        copier::set_mtime(
            &dst.path().join("a.txt"),
            t + std::time::Duration::from_secs(1),
        );
        let mut pair = make_test_pair(src.path(), dst.path());
        assert_eq!(
            planner::compute_preview(&pair).unwrap().total_modifications,
            1
        );
        pair.copy_options.fat_time_tolerance = true;
        assert_eq!(
            planner::compute_preview(&pair).unwrap().total_modifications,
            0
        );
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

    #[test]
    fn robocopy_timfix_realigns_unchanged_files() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "same").unwrap();
        fs::write(dst.path().join("a.txt"), "same").unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(500);
        copier::set_mtime(&src.path().join("a.txt"), t);
        copier::set_mtime(
            &dst.path().join("a.txt"),
            t + std::time::Duration::from_secs(1),
        );
        let mut pair = make_test_pair(src.path(), dst.path());
        // With /FFT the 1 s drift counts as "same", so nothing is copied…
        pair.copy_options.fat_time_tolerance = true;
        pair.copy_options.fix_timestamps = true;
        let preview = planner::compute_preview(&pair).unwrap();
        assert_eq!(preview.total_modifications, 0);
        assert_eq!(preview.fixups, vec!["a.txt".to_string()]);
        // …but /TIMFIX still lines the timestamps up exactly.
        run_with(pair);
        assert_eq!(
            fs::metadata(dst.path().join("a.txt"))
                .unwrap()
                .modified()
                .unwrap(),
            t
        );
    }

    #[test]
    fn report_counts_filtered_files_as_skipped() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("keep.txt"), "k").unwrap();
        fs::write(src.path().join("skip.log"), "s").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.filter.exclude_patterns = vec!["*.log".to_string()];
        let report = run_with(pair);
        assert_eq!(report.files_added, 1);
        assert_eq!(report.files_skipped, 1);
    }

    fn cfg(pair: SyncPair) -> ExecutorConfig {
        ExecutorConfig { pair, run_id: Uuid::new_v4(), continue_on_error: true, resume_from: None, quarantine_dir: None, rollback_dir: None }
    }

    #[test]
    fn trusted_inventory_skips_destination_scan() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "one").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.dest_state = DestStateMode::TrustedInventory;
        let r = execute_sync(&cfg(pair.clone())).unwrap();
        assert_eq!(r.files_added, 1);
        assert!(dst.path().join(planner::INVENTORY_FILE).exists());
        // Change the destination behind the engine's back: the inventory is
        // trusted, so nothing is copied again.
        fs::write(dst.path().join("a.txt"), "tampered!").unwrap();
        let p = planner::compute_preview(&pair).unwrap();
        assert_eq!(p.total_additions + p.total_modifications, 0, "trusted state");
        // A scan notices the difference.
        pair.dest_state = DestStateMode::Scan;
        let p = planner::compute_preview(&pair).unwrap();
        assert_eq!(p.total_modifications, 1);
        // The inventory file itself is never synced or purged.
        assert!(p.deletions.iter().all(|d| d.relative_path != planner::INVENTORY_FILE));
    }

    #[test]
    fn blind_upload_copies_everything() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "same").unwrap();
        fs::write(dst.path().join("a.txt"), "same").unwrap();
        fs::write(dst.path().join("extra.txt"), "x").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.dest_state = DestStateMode::BlindUpload;
        let p = planner::compute_preview(&pair).unwrap();
        assert_eq!(p.total_additions, 1);
        assert!(p.deletions.is_empty(), "blind mode never deletes unseen files");
    }

    #[test]
    fn sha256_verification() {
        let d = TempDir::new().unwrap();
        let f = d.path().join("x");
        fs::write(&f, "abc").unwrap();
        assert_eq!(compute_file_hash(&f, SyncHashAlgorithm::Sha256).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(compute_file_hash(&f, SyncHashAlgorithm::Xxh3).unwrap().len(), 16);
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a"), "data").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.verify_mode = SyncVerifyMode::Full;
        pair.checksum_enabled = true;
        pair.checksum_algorithm = SyncHashAlgorithm::Sha256;
        let r = execute_sync(&cfg(pair.clone())).unwrap();
        assert!(verify_sync(&pair, &r).unwrap().is_empty());
        fs::write(dst.path().join("a"), "dat4").unwrap();
        assert_eq!(verify_sync(&pair, &r).unwrap().len(), 1);
    }
}
