//! Sync executor - state machine that performs actual file synchronization.
//!
//! Uses the planner for diff computation, conflict resolver for conflicts,
//! and supports partial failure continuation and resumable state.

use crate::error::AppError;
use crate::sync::conflict::{self, ConflictAction};
use crate::sync::copier;
use crate::sync::planner;
use crate::sync::rollback::RollbackManager;
use crate::sync::versions::{StoredVersion, VersionIndex};
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

/// Callback told about every step as soon as it finished (called from
/// worker threads), for crash-recovery checkpoints.
pub type StepFn = Arc<dyn Fn(&str, &CompletedStep) + Send + Sync>;

/// Optional observers for a run: a progress callback, a cancel flag and a
/// finished-step callback.
#[derive(Default, Clone)]
pub struct RunHooks {
    pub progress: Option<ProgressFn>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub on_step: Option<StepFn>,
}

impl RunHooks {
    fn step_done(&self, entry: &SyncDiffEntry) {
        if let Some(f) = &self.on_step {
            f(&entry.relative_path, &CompletedStep::of(entry));
        }
    }

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
    /// Two-way: the baseline to record for the next run. Only set when the
    /// run finished every planned step without errors; otherwise the
    /// previous baseline stays.
    pub baseline: Option<SyncBaseline>,
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
    /// Steps an interrupted earlier run finished: planned steps it covers
    /// (same file, unchanged since) are skipped and the report says "resumed".
    pub resume_from: Option<SyncCheckpoint>,
    /// Quarantine directory for quarantine policy
    pub quarantine_dir: Option<PathBuf>,
    /// Rollback directory for snapshots
    pub rollback_dir: Option<PathBuf>,
    /// Two-way: the state after the last successful run (see
    /// [`SyncBaseline`]). `None` = first run: combine both sides, delete nothing.
    pub baseline: Option<SyncBaseline>,
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
    let stored_versions = StoredVersions {
        index: if pair.mode == SyncMode::VersionedBackup { VersionIndex::load(Path::new(&pair.dest_path)) } else { VersionIndex::default() },
        stored: Mutex::new(Vec::new()),
    };
    let plan = planner::plan(
        pair,
        &planner::PlanContext {
            baseline: config.baseline.as_ref(),
            versions: (pair.mode == SyncMode::VersionedBackup).then_some(&stored_versions.index),
            ..Default::default()
        },
    )?;
    let preview = &plan.preview;

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
    // Steps the interrupted run already did are not repeated (a versioned
    // backup would otherwise keep a second copy of each).
    let already_done = |e: &SyncDiffEntry| config.resume_from.as_ref().is_some_and(|c| c.covers(e));
    let mut resumed_skips: u64 = 0;

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
    for (entries, op) in [(&preview.additions, "add"), (&preview.modifications, "modify"), (&preview.deletions, "delete")] {
        for entry in entries.iter() {
            if already_done(entry) {
                resumed_skips += 1;
            } else {
                operations.push((entry, op));
            }
        }
    }
    files_skipped += resumed_skips;
    let additions: Vec<&SyncDiffEntry> = operations.iter().filter(|(_, op)| *op == "add").map(|(e, _)| *e).collect();
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
    let parallel_adds = if opts.threads > 1 && pair.mode != SyncMode::VersionedBackup {
        additions.len()
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
                    let entry = additions[idx];
                    tracker.start_file(&entry.relative_path);
                    let r = copier::with_retries(opts, || add_file(pair, entry, None));
                    if r.is_ok() {
                        hooks.step_done(entry);
                    }
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
            if idx < parallel_adds {
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
                execute_single_operation(pair, entry, op_type, &mut rollback, &stored_versions)
            });
            tracker.finish_file(match &result {
                Ok(OperationResult::Added(b)) | Ok(OperationResult::Modified(b)) => *b,
                _ => 0,
            });
            let failed = result.is_err();
            if !failed {
                hooks.step_done(entry);
            }
            outcomes.push((idx, result));
            if failed && !config.continue_on_error {
                halted = true;
                break;
            }
        }
    }

    // Two-way: what each finished step means for the next baseline.
    let mut baseline_updates: Vec<(String, Option<BaselineEntry>)> = Vec::new();
    for (idx, result) in outcomes {
        let (entry, op_type) = operations[idx];
        if pair.mode == SyncMode::TwoWay {
            match &result {
                Ok(OperationResult::Deleted) => baseline_updates.push((entry.relative_path.clone(), None)),
                Ok(OperationResult::Added(_)) | Ok(OperationResult::Modified(_)) => {
                    baseline_updates.push((entry.relative_path.clone(), observed_entry(pair, &entry.relative_path)))
                }
                _ => {}
            }
        }
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
                    && entry.direction == SyncDirection::ToDest
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
    let mut conflicts_halted = false;
    if !halted && !stopped_for_hours && !was_cancelled {
        for entry in &preview.conflicts {
            if hooks.cancelled() {
                was_cancelled = true;
                break;
            }
            tracker.start_file(&entry.relative_path);
            let mut copies = Vec::new();
            let result = execute_conflict_resolution(
                pair,
                entry,
                &mut rollback,
                config.quarantine_dir.as_deref(),
                &mut quarantined,
                &mut pending_conflicts,
                &mut copies,
            );
            // Two-way "keep both": the conflict copy is on both sides now.
            for rel in copies.into_iter().filter(|_| pair.mode == SyncMode::TwoWay) {
                let entry = observed_entry(pair, &rel);
                baseline_updates.push((rel, entry));
            }
            tracker.finish_file(match &result {
                Ok(OperationResult::ConflictResolved(b)) => *b,
                _ => 0,
            });

            match result {
                Ok(op_result) => match op_result {
                    OperationResult::ConflictResolved(bytes) => {
                        conflicts_resolved += 1;
                        bytes_transferred += bytes;
                        if pair.mode == SyncMode::TwoWay {
                            baseline_updates.push((entry.relative_path.clone(), observed_entry(pair, &entry.relative_path)));
                        }
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
                        conflicts_halted = true;
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

    // Versioned backup: record the new versions, then drop the oldest ones
    // beyond `keep_versions`.
    let mut versions_pruned: u64 = 0;
    if pair.mode == SyncMode::VersionedBackup {
        let StoredVersions { mut index, stored } = stored_versions;
        let stored = stored.into_inner().unwrap_or_default();
        let keep = pair.run_options.keep_versions.filter(|k| *k > 0);
        if !stored.is_empty() || keep.is_some() {
            for (rel, v) in stored {
                index.record(&rel, v);
            }
            if let Some(keep) = keep.filter(|_| !was_cancelled) {
                match index.prune(dest_root, keep) {
                    Ok(r) => {
                        versions_pruned = r.removed.len() as u64;
                        log_lines.extend(r.removed.iter().map(|p| format!("\t*Old version\t\t{p}")));
                        errors += r.errors.len() as u32;
                        error_messages.extend(r.errors);
                    }
                    Err(e) => {
                        errors += 1;
                        error_messages.push(format!("Pruning old versions: {e}"));
                    }
                }
            }
            if let Err(e) = index.save(dest_root) {
                errors += 1;
                error_messages.push(format!("Could not save the version index: {e}"));
            }
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
        versions_pruned,
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
            let filter = crate::sync::filter::CompiledSyncFilter::new(&pair.filter);
            planner::walk_directory_with(Path::new(&pair.source_path), opts).and_then(|v| {
                let filter = filter?;
                Ok(v.into_iter().filter(|f| !f.is_dir && filter.accepts(&f.relative_path, f.size)).collect())
            })
        };
        if let Err(e) = state.map_err(|e| std::io::Error::other(e.to_string())).and_then(|s| planner::save_inventory(dest_root, &s)) {
            tracing::warn!("Could not record the destination inventory of {}: {e}", pair.name);
        }
    }

    if let Err(e) = copier::write_log(pair, &report, &log_lines) {
        tracing::warn!("Could not write sync log for {}: {}", pair.name, e);
    }

    // Two-way: record the new baseline only when every planned step ran
    // cleanly. A failed, cancelled or paused run keeps the old one, so the
    // next run re-derives anything this one left half done.
    let baseline = (pair.mode == SyncMode::TwoWay
        && report.status == SyncRunStatus::Success
        && report.errors == 0
        && !stopped_for_hours
        && !was_cancelled
        && !halted
        && !conflicts_halted)
        .then(|| {
            let mut next = match &config.baseline {
                Some(b) if b.matches(pair) => b.clone(),
                _ => SyncBaseline::for_pair(pair),
            };
            for (path, entry) in plan.settled.iter().cloned().chain(baseline_updates) {
                match entry {
                    Some(e) => next.entries.insert(path, e),
                    None => next.entries.remove(&path),
                };
            }
            next.run_id = Some(config.run_id);
            next.recorded_at = Some(Utc::now());
            next
        });

    Ok(RunOutcome {
        report,
        snapshots: rollback.snapshots,
        quarantined,
        pending_conflicts,
        baseline,
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
fn add_file(pair: &SyncPair, entry: &SyncDiffEntry, versions: Option<&StoredVersions>) -> Result<OperationResult, AppError> {
    let opts = &pair.copy_options;
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let source_file = source_root.join(&entry.relative_path);
    let dest_file = dest_root.join(&entry.relative_path);

    // Two-way: files new in the destination are copied to the source.
    if entry.direction == SyncDirection::ToSource {
        let bytes = copier::copy_file(&dest_file, &source_file, opts)?;
        return Ok(OperationResult::Added(bytes));
    }

    // For versioned backup, add version suffix
    if pair.mode == SyncMode::VersionedBackup {
        return Ok(OperationResult::Added(store_version(pair, entry, versions)?));
    }
    let bytes = copier::copy_file(&source_file, &dest_file, opts)?;
    Ok(OperationResult::Added(bytes))
}

/// Versioned backup: the index as the run started, and the versions the
/// run stores (source path, version).
struct StoredVersions {
    index: VersionIndex,
    stored: Mutex<Vec<(String, StoredVersion)>>,
}

/// Versioned backup: copy the source file to its next version name and
/// note the version for the index.
fn store_version(pair: &SyncPair, entry: &SyncDiffEntry, versions: Option<&StoredVersions>) -> Result<u64, AppError> {
    let dest_root = Path::new(&pair.dest_path);
    let start = versions.and_then(|v| v.index.latest(&entry.relative_path)).map_or(1, |v| v.seq + 1);
    let (target, seq) = versioned_path_from(&dest_root.join(&entry.relative_path), pair.run_options.version_template.as_deref(), start)?;
    let bytes = copier::copy_file(&Path::new(&pair.source_path).join(&entry.relative_path), &target, &pair.copy_options)?;
    if let (Some(out), Ok(rel)) = (versions, target.strip_prefix(dest_root)) {
        let version = StoredVersion {
            path: rel.to_string_lossy().to_string(),
            size: entry.source_size.unwrap_or(bytes),
            source_modified: entry.source_modified,
            stored_at: Utc::now(),
            seq,
        };
        if let Ok(mut v) = out.stored.lock() {
            v.push((entry.relative_path.clone(), version));
        }
    }
    Ok(bytes)
}

/// Execute a single file operation (add/modify/delete).
fn execute_single_operation(
    pair: &SyncPair,
    entry: &SyncDiffEntry,
    op_type: &str,
    rollback: &mut RollbackManager,
    versions: &StoredVersions,
) -> Result<OperationResult, AppError> {
    let opts = &pair.copy_options;
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let source_file = source_root.join(&entry.relative_path);
    let dest_file = dest_root.join(&entry.relative_path);

    match op_type {
        "add" => add_file(pair, entry, Some(versions)),

        "modify" if entry.direction == SyncDirection::ToSource => {
            // Two-way: the destination changed; bring the source up to date.
            rollback.snapshot_on(SyncSide::Source, &entry.relative_path, &source_file)?;
            let bytes = copier::copy_file(&dest_file, &source_file, opts)?;
            Ok(OperationResult::Modified(bytes))
        }

        "delete" if entry.direction == SyncDirection::ToSource => {
            // Two-way: deleted in the destination since the last sync.
            if source_file.exists() {
                rollback.snapshot_on(SyncSide::Source, &entry.relative_path, &source_file)?;
                fs::remove_file(&source_file).map_err(|e| AppError::Sync {
                    message: format!("Delete failed {}: {}", entry.relative_path, e),
                    advice: "Check permissions.".to_string(),
                })?;
            }
            Ok(OperationResult::Deleted)
        }

        "modify" => {
            // Take pre-destructive snapshot if destination exists
            if dest_file.exists() {
                rollback.snapshot_file(&entry.relative_path, &dest_file)?;
            }

            if pair.mode == SyncMode::VersionedBackup {
                // Keep old version, write new version
                Ok(OperationResult::Modified(store_version(pair, entry, Some(versions))?))
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
    copies: &mut Vec<String>,
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

    let two_way = pair.mode == SyncMode::TwoWay;
    match result.action {
        ConflictAction::CopySource => {
            if dest_file.exists() {
                rollback.snapshot_file(&entry.relative_path, &dest_file)?;
            }
            let bytes = copier::copy_file(&source_file, &dest_file, opts)?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        // Two-way: the destination's version wins on both sides.
        ConflictAction::KeepDest if two_way => {
            rollback.snapshot_on(SyncSide::Source, &entry.relative_path, &source_file)?;
            let bytes = copier::copy_file(&dest_file, &source_file, opts)?;
            Ok(OperationResult::ConflictResolved(bytes))
        }
        ConflictAction::KeepDest => Ok(OperationResult::Skipped),
        // Two-way: keep both versions on both sides — the destination's at
        // the original name, the source's as the conflict copy.
        ConflictAction::CopyToConflictPath if two_way => {
            let source_twin = |p: &Path| p.strip_prefix(dest_root).map(|rel| source_root.join(rel)).unwrap_or_else(|_| p.to_path_buf());
            let conflict_dest = conflict::generate_conflict_path_with(&dest_file, |p| p.exists() || source_twin(p).exists());
            let conflict_source = source_twin(&conflict_dest);
            let mut bytes = copier::copy_file(&source_file, &conflict_dest, opts)?;
            bytes += copier::copy_file(&source_file, &conflict_source, opts)?;
            rollback.snapshot_on(SyncSide::Source, &entry.relative_path, &source_file)?;
            bytes += copier::copy_file(&dest_file, &source_file, opts)?;
            if let Ok(rel) = conflict_dest.strip_prefix(dest_root) {
                copies.push(rel.to_string_lossy().to_string());
            }
            Ok(OperationResult::ConflictResolved(bytes))
        }
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
        direction: SyncDirection::ToDest,
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
        &mut Vec::new(),
    )?;
    Ok((rollback.snapshots, quarantined))
}

/// What both sides of `rel` look like now, as a baseline entry. `None` when
/// either side cannot be read (the path is then left out of the baseline,
/// so the next run compares it afresh).
fn observed_entry(pair: &SyncPair, rel: &str) -> Option<BaselineEntry> {
    let s = fs::metadata(Path::new(&pair.source_path).join(rel)).ok()?;
    let d = fs::metadata(Path::new(&pair.dest_path).join(rel)).ok()?;
    if !s.is_file() || !d.is_file() {
        return None;
    }
    Some(BaselineEntry {
        size: s.len(),
        source_modified: s.modified().ok().map(chrono::DateTime::<Utc>::from),
        dest_modified: d.modified().ok().map(chrono::DateTime::<Utc>::from),
        hash: None,
    })
}

/// Default name for kept versions: `file.txt` -> `file.v2.txt`.
pub const DEFAULT_VERSION_TEMPLATE: &str = "{name}.v{num}.{ext}";

/// Name for the next kept version of `path` from the pair's template; the
/// first free one. A template without `{num}` gets ` (n)` added on clashes.
#[cfg(test)]
fn versioned_path(path: &Path, template: Option<&str>) -> Result<PathBuf, AppError> {
    versioned_path_from(path, template, 1).map(|(p, _)| p)
}

/// [`versioned_path`] trying version numbers from `start` (numbers freed by
/// pruning are not reused); also returns the number used.
fn versioned_path_from(path: &Path, template: Option<&str>, start: u64) -> Result<(PathBuf, u64), AppError> {
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
    for n in start.max(1)..start.max(1) + 100_000u64 {
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
            return Ok((candidate, n));
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
    if o.keep_versions == Some(0) {
        return Err(AppError::Sync {
            message: "Keep at least one version of each file.".to_string(),
            advice: "Set the number of versions to keep to 1 or more, or clear it to keep every version.".to_string(),
        });
    }
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
    let filter = crate::sync::filter::CompiledSyncFilter::new(&pair.filter)?;

    for file in source_files.iter().filter(|f| !f.is_dir) {
        if !filter.accepts(&file.relative_path, file.size) {
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
    use crate::sync::rollback;

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
            baseline: None,
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
            baseline: None,
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
            baseline: None,
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
            baseline: None,
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
            baseline: None,
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
            baseline: None,
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
            baseline: None,
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
                versions_pruned: 0,
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
            baseline: None,
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
        let ok = SyncRunOptions { settle_secs: 5, completion_marker: Some("_DONE".into()), version_template: Some("{name}.{now:%Y}.{ext}".into()), keep_versions: Some(3) };
        assert!(validate_run_options(&ok).is_ok());
        assert!(validate_run_options(&SyncRunOptions { version_template: Some("{nope}".into()), ..Default::default() }).is_err());
        for bad in ["a/b", "..", ".ufop-sync-inventory.json"] {
            assert!(validate_run_options(&SyncRunOptions { completion_marker: Some(bad.into()), ..Default::default() }).is_err(), "{bad}");
        }
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
            baseline: None,
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
        ExecutorConfig { pair, run_id: Uuid::new_v4(), continue_on_error: true, resume_from: None, quarantine_dir: None, rollback_dir: None, baseline: None }
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

    // ── Two-way baseline (three-way diff) ──

    fn two_way(src: &Path, dst: &Path) -> SyncPair {
        let mut pair = make_test_pair(src, dst);
        pair.mode = SyncMode::TwoWay;
        pair
    }

    /// One two-way run with `baseline`; returns the outcome. The caller keeps
    /// the new baseline only when the run produced one — as the app does.
    fn run_two_way(pair: &SyncPair, baseline: &mut Option<SyncBaseline>) -> RunOutcome {
        let rollback = TempDir::new().unwrap().keep();
        let outcome = execute_sync_with(
            &ExecutorConfig { baseline: baseline.clone(), rollback_dir: Some(rollback), ..cfg(pair.clone()) },
            &RunHooks::default(),
        )
        .unwrap();
        if let Some(b) = &outcome.baseline {
            *baseline = Some(b.clone());
        }
        outcome
    }

    #[test]
    fn two_way_delete_on_source_is_propagated_not_resurrected() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("gone.txt"), "x").unwrap();
        fs::write(src.path().join("keep.txt"), "k").unwrap();
        let pair = two_way(src.path(), dst.path());
        let mut base = None;
        assert_eq!(run_two_way(&pair, &mut base).report.files_added, 2);
        assert_eq!(base.as_ref().unwrap().entries.len(), 2);

        fs::remove_file(src.path().join("gone.txt")).unwrap();
        let preview = planner::plan(&pair, &planner::PlanContext { baseline: base.as_ref(), ..Default::default() }).unwrap().preview;
        assert_eq!(preview.total_deletions, 1);
        assert_eq!(preview.deletions[0].direction, SyncDirection::ToDest);
        assert_eq!(preview.total_additions, 0, "must not plan to copy it back");

        let out = run_two_way(&pair, &mut base);
        assert_eq!(out.report.files_deleted, 1);
        assert!(!src.path().join("gone.txt").exists(), "deleted file came back to the source");
        assert!(!dst.path().join("gone.txt").exists(), "deletion not propagated to the destination");
        assert!(!base.as_ref().unwrap().entries.contains_key("gone.txt"));
        // A third run is a no-op.
        let again = run_two_way(&pair, &mut base);
        assert_eq!(again.report.files_added + again.report.files_deleted + again.report.files_modified, 0);
    }

    #[test]
    fn two_way_delete_and_edit_on_destination_flow_back_to_source() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "a").unwrap();
        fs::write(src.path().join("b.txt"), "b").unwrap();
        fs::write(src.path().join("c.txt"), "c").unwrap();
        let pair = two_way(src.path(), dst.path());
        let mut base = None;
        run_two_way(&pair, &mut base);

        fs::remove_file(dst.path().join("a.txt")).unwrap();
        fs::write(dst.path().join("b.txt"), "b edited in the destination").unwrap();
        let out = run_two_way(&pair, &mut base);
        assert_eq!(out.report.files_deleted, 1);
        assert_eq!(out.report.files_modified, 1);
        assert!(!src.path().join("a.txt").exists());
        assert_eq!(fs::read_to_string(src.path().join("b.txt")).unwrap(), "b edited in the destination");
        // Source-side changes are snapshotted and can be rolled back.
        assert!(out.snapshots.iter().all(|s| s.side == SyncSide::Source));
        let restored = rollback::rollback_pair_run(&out.snapshots, src.path(), dst.path()).unwrap();
        assert_eq!(restored, 2);
        assert_eq!(fs::read_to_string(src.path().join("a.txt")).unwrap(), "a");
        assert_eq!(fs::read_to_string(src.path().join("b.txt")).unwrap(), "b");
    }

    #[test]
    fn two_way_edit_on_both_sides_is_a_conflict() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("doc.txt"), "v1").unwrap();
        let mut pair = two_way(src.path(), dst.path());
        let mut base = None;
        run_two_way(&pair, &mut base);

        fs::write(src.path().join("doc.txt"), "edited in source").unwrap();
        fs::write(dst.path().join("doc.txt"), "edited in destination!").unwrap();
        let preview = planner::plan(&pair, &planner::PlanContext { baseline: base.as_ref(), ..Default::default() }).unwrap().preview;
        assert_eq!(preview.total_conflicts, 1);
        assert!(preview.conflicts[0].reason.contains("both"));
        assert_eq!(preview.total_modifications, 0);

        // Skip leaves it unresolved: no baseline entry advance, still a conflict next time.
        pair.conflict_policy = SyncConflictPolicy::Skip;
        run_two_way(&pair, &mut base);
        let again = planner::plan(&pair, &planner::PlanContext { baseline: base.as_ref(), ..Default::default() }).unwrap().preview;
        assert_eq!(again.total_conflicts, 1);

        // Keep both: both sides end up with both versions.
        pair.conflict_policy = SyncConflictPolicy::CreateConflictCopy;
        let out = run_two_way(&pair, &mut base);
        assert_eq!(out.report.conflicts_resolved, 1);
        assert_eq!(fs::read_to_string(src.path().join("doc.txt")).unwrap(), "edited in destination!");
        assert_eq!(fs::read_to_string(dst.path().join("doc.txt")).unwrap(), "edited in destination!");
        let copies = |dir: &Path| {
            fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains("(conflict")).map(|e| fs::read_to_string(e.path()).unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(copies(src.path()), vec!["edited in source".to_string()]);
        assert_eq!(copies(dst.path()), vec!["edited in source".to_string()]);
        assert!(base.as_ref().unwrap().entries.keys().any(|k| k.contains("(conflict")), "the conflict copy is tracked too");
        // Converged: the next run has nothing to do.
        let after = planner::plan(&pair, &planner::PlanContext { baseline: base.as_ref(), ..Default::default() }).unwrap().preview;
        assert_eq!(after.total_conflicts + after.total_additions + after.total_modifications + after.total_deletions, 0);
    }

    #[test]
    fn two_way_rename_is_a_delete_plus_add() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("old-name.txt"), "content").unwrap();
        let pair = two_way(src.path(), dst.path());
        let mut base = None;
        run_two_way(&pair, &mut base);

        fs::rename(src.path().join("old-name.txt"), src.path().join("new-name.txt")).unwrap();
        let out = run_two_way(&pair, &mut base);
        assert_eq!((out.report.files_added, out.report.files_deleted), (1, 1));
        assert!(!dst.path().join("old-name.txt").exists());
        assert!(!src.path().join("old-name.txt").exists());
        assert_eq!(fs::read_to_string(dst.path().join("new-name.txt")).unwrap(), "content");
    }

    #[test]
    fn two_way_first_run_never_deletes() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("only-in-source.txt"), "s").unwrap();
        fs::write(dst.path().join("only-in-dest.txt"), "d").unwrap();
        let pair = two_way(src.path(), dst.path());
        let preview = planner::plan(&pair, &planner::PlanContext::default()).unwrap().preview;
        assert_eq!(preview.total_deletions, 0);
        assert_eq!(preview.total_additions, 2);
        assert!(preview.notes.iter().any(|n| n.contains("First two-way run")));
        let mut base = None;
        let out = run_two_way(&pair, &mut base);
        assert_eq!((out.report.files_added, out.report.files_deleted), (2, 0));
        for dir in [src.path(), dst.path()] {
            assert!(dir.join("only-in-source.txt").exists() && dir.join("only-in-dest.txt").exists());
        }
        assert_eq!(base.unwrap().entries.len(), 2);
    }

    #[test]
    fn two_way_failed_run_does_not_advance_the_baseline() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "a").unwrap();
        let pair = two_way(src.path(), dst.path());
        let mut base = None;
        run_two_way(&pair, &mut base);
        let before = base.clone();

        // A new source file whose copy fails (a folder of that name is in the way).
        fs::write(src.path().join("blocked"), "b").unwrap();
        fs::create_dir(dst.path().join("blocked")).unwrap();
        fs::write(src.path().join("new.txt"), "n").unwrap();
        let out = run_two_way(&pair, &mut base);
        assert!(out.report.errors > 0);
        assert!(out.baseline.is_none(), "a run with errors must not record a baseline");
        assert_eq!(base, before);
        // Cancelled runs keep the old baseline too.
        let cancel = Arc::new(AtomicBool::new(true));
        let out = execute_sync_with(&ExecutorConfig { baseline: base.clone(), ..cfg(pair.clone()) }, &RunHooks { cancel: Some(cancel), ..Default::default() }).unwrap();
        assert_eq!(out.report.status, SyncRunStatus::Cancelled);
        assert!(out.baseline.is_none());
    }

    #[test]
    fn two_way_changed_after_delete_is_copied_back() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("f.txt"), "v1").unwrap();
        let pair = two_way(src.path(), dst.path());
        let mut base = None;
        run_two_way(&pair, &mut base);
        fs::remove_file(dst.path().join("f.txt")).unwrap();
        fs::write(src.path().join("f.txt"), "v2, edited after the delete").unwrap();
        let out = run_two_way(&pair, &mut base);
        assert_eq!((out.report.files_added, out.report.files_deleted), (1, 0));
        assert_eq!(fs::read_to_string(dst.path().join("f.txt")).unwrap(), "v2, edited after the delete");
    }

    #[test]
    fn two_way_safety_stop_when_a_side_is_emptied() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        for i in 0..6 {
            fs::write(src.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let mut pair = two_way(src.path(), dst.path());
        let mut base = None;
        run_two_way(&pair, &mut base);
        // The destination is emptied (an unmounted share looks the same).
        for i in 0..6 {
            fs::remove_file(dst.path().join(format!("f{i}.txt"))).unwrap();
        }
        let err = execute_sync_with(&ExecutorConfig { baseline: base.clone(), ..cfg(pair.clone()) }, &RunHooks::default()).unwrap_err();
        assert!(err.to_string().contains("Safety stop"), "{err}");
        assert_eq!(fs::read_dir(src.path()).unwrap().count(), 6, "nothing deleted");
        // A missing destination folder is refused as well.
        let gone = TempDir::new().unwrap().path().join("not-mounted");
        let moved = SyncPair { dest_path: gone.to_string_lossy().to_string(), ..pair.clone() };
        let b = SyncBaseline { dest_root: moved.dest_path.clone(), ..base.clone().unwrap() };
        assert!(planner::plan(&moved, &planner::PlanContext { baseline: Some(&b), ..Default::default() }).is_err());
        // Deliberate: turning the limit off lets the deletions through.
        pair.copy_options.two_way_max_delete_percent = 0;
        let out = run_two_way(&pair, &mut base);
        assert_eq!(out.report.files_deleted, 6);
    }

    #[test]
    fn two_way_baseline_for_other_folders_is_ignored() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(dst.path().join("d.txt"), "d").unwrap();
        let pair = two_way(src.path(), dst.path());
        let mut stale = SyncBaseline { source_root: "/elsewhere".into(), dest_root: "/other".into(), ..Default::default() };
        stale.entries.insert("d.txt".into(), BaselineEntry { size: 1, source_modified: None, dest_modified: None, hash: None });
        let preview = planner::plan(&pair, &planner::PlanContext { baseline: Some(&stale), ..Default::default() }).unwrap().preview;
        assert_eq!(preview.total_deletions, 0);
        assert_eq!(preview.total_additions, 1);
    }

    #[test]
    fn diff_trees_is_pure_and_cancellable() {
        let file = |p: &str, size: u64| planner::FileMeta {
            relative_path: p.into(),
            absolute_path: PathBuf::from(p),
            size,
            modified: Some(Utc::now() - chrono::Duration::days(1)),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            readonly: false,
            attributes: 0,
        };
        let pair = SyncPair { source_path: "/nowhere/src".into(), dest_path: "/nowhere/dst".into(), ..SyncPair::default() };
        let plan = planner::diff_trees(&pair, &[file("a", 1), file("b", 2)], &[file("b", 3)], &planner::PlanContext::default()).unwrap();
        assert_eq!((plan.preview.total_additions, plan.preview.total_modifications), (1, 1));
        let stop = AtomicBool::new(true);
        assert!(planner::diff_trees(&pair, &[file("a", 1)], &[], &planner::PlanContext { cancel: Some(&stop), ..Default::default() }).is_err());
    }

    // ── Crash-recovery checkpoints ──

    #[test]
    fn resume_skips_steps_an_interrupted_run_finished() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "alpha").unwrap();
        fs::write(src.path().join("b.txt"), "bravo").unwrap();
        set_age_days(&src.path().join("a.txt"), 1);
        set_age_days(&src.path().join("b.txt"), 1);
        let mut pair = make_test_pair(src.path(), dst.path());
        // Without kept timestamps every rerun would copy everything again.
        pair.copy_options.copy_timestamps = false;
        let steps = Arc::new(Mutex::new(SyncCheckpoint { pair_id: pair.id, ..Default::default() }));
        let rec = steps.clone();
        let hooks = RunHooks {
            on_step: Some(Arc::new(move |path: &str, step: &CompletedStep| {
                rec.lock().unwrap().completed.insert(path.to_string(), step.clone());
            })),
            ..Default::default()
        };
        let first = execute_sync_with(&cfg(pair.clone()), &hooks).unwrap();
        assert_eq!(first.report.files_added, 2);
        let mut checkpoint = steps.lock().unwrap().clone();
        assert_eq!(checkpoint.completed.len(), 2);

        // Pretend the run died after both copies; b.txt then changed.
        fs::write(src.path().join("b.txt"), "bravo, edited after the crash").unwrap();
        set_age_days(&src.path().join("b.txt"), 1);
        let resumed = execute_sync_with(&ExecutorConfig { resume_from: Some(checkpoint.clone()), ..cfg(pair.clone()) }, &RunHooks::default()).unwrap().report;
        assert!(resumed.resumed);
        assert_eq!(resumed.files_modified, 1, "only the changed file is copied again");
        assert_eq!(fs::read_to_string(dst.path().join("b.txt")).unwrap(), "bravo, edited after the crash");

        // Without the checkpoint both files would be copied again.
        let plain = run_with(pair.clone());
        assert_eq!(plain.files_modified, 2);
        assert!(!plain.resumed);
        // Steps of another kind (a delete recorded for a file now to be copied) never match.
        checkpoint.completed.get_mut("a.txt").unwrap().action = SyncAction::Delete;
        let p = planner::compute_preview(&pair).unwrap();
        let a = p.modifications.iter().find(|e| e.relative_path == "a.txt").unwrap();
        assert!(!checkpoint.covers(a));
    }

    // ── Versioned backup ──

    /// Regression: versioned mode writes every copy under a version name and
    /// never at the plain path, so the next run saw "doc.txt" missing at the
    /// destination and stored another, identical version — every run.
    #[test]
    fn versioned_backup_keeps_no_duplicate_versions_of_unchanged_files() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("doc.txt"), "v1").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::VersionedBackup;
        assert_eq!(run_with(pair.clone()).files_added, 1);
        let second = run_with(pair.clone());
        assert_eq!(second.files_added + second.files_modified, 0, "an unchanged file got another version");
        let stored = || fs::read_dir(dst.path()).unwrap().flatten().filter(|e| !planner::is_internal_entry(&e.file_name().to_string_lossy())).count();
        assert_eq!(stored(), 1);
        // A real change does get a new version.
        fs::write(src.path().join("doc.txt"), "v2, longer").unwrap();
        let third = run_with(pair);
        assert_eq!(third.files_added + third.files_modified, 1);
        assert_eq!(stored(), 2);
    }

    #[test]
    fn versioned_backup_prunes_beyond_keep_versions() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::VersionedBackup;
        pair.run_options.keep_versions = Some(2);
        let mut pruned = 0;
        fs::write(src.path().join("stable.txt"), "never changes").unwrap();
        for i in 1..=4 {
            fs::write(src.path().join("doc.txt"), "x".repeat(i)).unwrap();
            let r = run_with(pair.clone());
            assert_eq!(r.errors, 0, "{:?}", r.error_messages);
            pruned += r.versions_pruned;
        }
        assert_eq!(pruned, 2);
        let index = VersionIndex::load(dst.path());
        let kept: Vec<String> = index.files["doc.txt"].iter().map(|v| v.path.clone()).collect();
        assert_eq!(kept, vec!["doc.v3.txt".to_string(), "doc.v4.txt".to_string()]);
        assert!(!dst.path().join("doc.v1.txt").exists() && !dst.path().join("doc.v2.txt").exists());
        assert_eq!(fs::read_to_string(dst.path().join("doc.v4.txt")).unwrap(), "xxxx");
        assert_eq!(index.files["stable.txt"].len(), 1, "unchanged files keep a single version");
        // Unlimited (the default) never removes anything; the index is never synced.
        assert!(planner::is_internal_entry(crate::sync::versions::VERSIONS_FILE));
        assert!(validate_run_options(&SyncRunOptions { keep_versions: Some(0), ..Default::default() }).is_err());
    }

    #[test]
    fn versioned_backup_purge_never_deletes_versions() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::write(src.path().join("a.txt"), "a").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::VersionedBackup;
        pair.copy_options.purge = true;
        run_with(pair.clone());
        let again = planner::compute_preview(&pair).unwrap();
        assert_eq!(again.total_deletions, 0, "stored versions are not extras");
        assert_eq!(again.total_additions + again.total_modifications, 0);
    }
}
