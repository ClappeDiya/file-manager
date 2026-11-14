//! Copy or delete selected differences of a comparison, then re-compare
//! just the entries that changed.
//!
//! - Local → local copies are transfer-engine jobs (the queue's verified,
//!   journalled, pausable copy) into a hidden sibling
//!   (`.<name>.ufop-compare-<tag>`), moved into place once complete; the
//!   copy keeps the source's modification time. The queue does not keep
//!   these jobs over a restart ([`is_staging`]), and a staging file left by
//!   a copy the app closed during is removed when a copy next goes to that
//!   folder.
//! - Copies to or from a connection go through its connector (upload,
//!   download, or download-then-upload between two connections) and show
//!   in the transfer queue as tracked jobs, which Cancel there stops.
//!   Remote entries are addressed by the paths the connection listed them
//!   at ([`ResolvedSide::abs`]), new ones by their folder's listed path and
//!   their name ([`ResolvedSide::abs_new`]): on Google Drive those are IDs.
//! - A copy that replaces a different local file first moves the old one
//!   to the Trash (on Windows `move_to_trash` deletes it: no Trash there
//!   yet), or deletes it when the user chose permanent. On a connection the
//!   upload overwrites the old file: nothing of it is kept. A copy that is
//!   not meant to replace anything never overwrites a file that appeared
//!   since the comparison, on a local folder or a connection.
//! - Every step's path must be a plain path inside the side's root
//!   ([`ResolvedSide::check_relative`]); anything else is refused.
//! - Local deletes go through the file manager's own delete (Trash unless
//!   permanent, and not on Windows yet); connection deletes through the
//!   connector's remove, which most protocols cannot undo. A folder is
//!   removed only once nothing is left in it (a local one atomically, so
//!   never to the Trash: it is empty).
//!
//! Everything is recorded in the operation ledger. Local copies of new files
//! and local folders created are recorded as `fs` copy / create-folder
//! operations, so Undo removes them (`undo_ids`: the copies, then the
//! folders); replacements and deletes are recorded as such (for a local
//! folder outside Windows, the Trash holds what they removed). What is done
//! on a connection is recorded as `connector` operations, which Undo cannot
//! reverse.

use super::{ComparePhase, CompareProgress, ProgressFn, ResolvedSide};
use crate::commands::file_ops_commands::{delete_files_impl, move_to_trash, record_fs_ok};
use crate::core::error::AppError;
use crate::core::traits::TransferOperations;
use crate::core::types::TransferStatus;
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::sync_engine::compare::{self, ActionPlan, ActionStep, CompareAction, CompareEntry, CompareOptions, CompareSideId, CompareSummary, ListingError, LocalSidesHasher, SideListing, SkippedEntry};
use crate::sync_engine::planner::FileMeta;
use crate::transfer_engine::TransferManager;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// What running an action needs besides the two sides.
pub struct ApplyEnv<'a> {
    pub transfer: &'a TransferManager,
    pub ledger: &'a OperationLedger,
    /// Delete for good (and replace without keeping the old file) instead
    /// of moving to the Trash.
    pub permanent: bool,
    pub cancel: &'a CancellationToken,
    pub progress: &'a ProgressFn,
    pub task_id: &'a str,
}

/// A step that did not work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedStep {
    pub path: String,
    pub error: String,
}

/// How an action went, and the comparison entries it changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub task_id: String,
    pub action: CompareAction,
    /// Paths copied, created or deleted.
    pub done: Vec<String>,
    pub failed: Vec<FailedStep>,
    pub skipped: Vec<SkippedEntry>,
    /// Folders left in place because they still hold something.
    pub kept: Vec<String>,
    pub notes: Vec<String>,
    pub cancelled: bool,
    /// Ledger id of the copies Undo can remove, when there are any.
    pub undo_id: Option<String>,
    /// Every ledger id "Undo copies" reverses, in order: the copies, then
    /// the local folders made for them.
    #[serde(default)]
    pub undo_ids: Vec<String>,
    /// Re-compared entries (folders above them included) …
    pub changed: Vec<CompareEntry>,
    /// … and paths now on neither side.
    pub removed: Vec<String>,
    pub summary: CompareSummary,
}

/// What [`execute`] did.
#[derive(Debug, Default)]
pub struct ExecReport {
    pub done: Vec<String>,
    pub failed: Vec<FailedStep>,
    pub kept: Vec<String>,
    pub notes: Vec<String>,
    pub cancelled: bool,
    pub undo_id: Option<String>,
    /// Ledger ids to undo, in order (copies, then the folders made for them).
    pub undo_ids: Vec<String>,
    /// Paths to re-compare.
    pub touched: HashSet<String>,
}

/// Ledger records, gathered while the steps run.
#[derive(Default)]
struct Records {
    /// Local copies of files that were not there (undoable).
    copied: (Vec<String>, Vec<String>),
    /// Local files replaced.
    replaced: (Vec<String>, Vec<String>),
    /// Local folders created (undoable).
    created: Vec<String>,
    /// Ledger id the created folders were recorded under.
    created_id: Option<String>,
    uploads_with_new_time: u64,
    /// Names in each folder of a connection the copies go to, listed once
    /// per action to check that a new file is not already there.
    remote_names: HashMap<String, HashSet<String>>,
    /// Local folders already cleared of stale staging files.
    swept: HashSet<PathBuf>,
}

fn side<'a>(id: CompareSideId, left: &'a ResolvedSide, right: &'a ResolvedSide) -> &'a ResolvedSide {
    match id {
        CompareSideId::Left => left,
        CompareSideId::Right => right,
    }
}

/// Run `plan` over the two sides, step by step (stopping between steps when
/// cancelled). Never fails as a whole: each step's error is reported.
pub async fn execute(plan: &ActionPlan, left: &ResolvedSide, right: &ResolvedSide, env: &ApplyEnv<'_>) -> ExecReport {
    let mut report = ExecReport::default();
    let mut rec = Records::default();
    let total = plan.steps.len() as u64;
    let mut pending_deletes: Vec<(String, String)> = Vec::new();
    let mut created_recorded = false;

    for (i, step) in plan.steps.iter().enumerate() {
        if env.cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }
        let mut p = CompareProgress::new(env.task_id, ComparePhase::Applying);
        (p.done, p.total, p.current) = (i as u64, total, Some(step.path().to_string()));
        (env.progress)(p);
        if let Err(error) = ResolvedSide::check_relative(step.path()) {
            report.failed.push(FailedStep { path: step.path().to_string(), error });
            continue;
        }
        report.touched.insert(step.path().to_string());

        // Local file deletes run as one batch (one ledger operation), flushed
        // before anything else (the folders they empty) runs.
        if let ActionStep::Delete { side: s, path, is_dir: false, .. } = step {
            if let Ok(Some(local)) = side(*s, left, right).local_path(path) {
                pending_deletes.push((path.clone(), local.to_string_lossy().to_string()));
                continue;
            }
        }
        flush_deletes(&mut pending_deletes, env, &mut report).await;
        // Folders are created before any copy: record them first, so Undo
        // removes the copies, then the (then empty) folders.
        if !created_recorded && matches!(step, ActionStep::Copy { .. }) {
            created_recorded = true;
            record_created(&mut rec, env.ledger).await;
        }

        let result = match step {
            ActionStep::MakeDir { side: s, path } => make_dir(side(*s, left, right), path, env.ledger, &mut rec).await,
            ActionStep::Copy { from, to, path, size, modified, replace } => {
                copy_file(side(*from, left, right), side(*to, left, right), path, *size, *modified, *replace, env, &mut rec).await
            }
            ActionStep::Delete { side: s, path, is_dir, .. } => {
                let target = side(*s, left, right);
                if *is_dir {
                    match delete_dir(target, path, env).await {
                        Ok(true) => Ok(()),
                        Ok(false) => {
                            report.kept.push(path.clone());
                            continue;
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    delete_remote_file(target, path, env.ledger).await
                }
            }
        };
        match result {
            Ok(()) => report.done.push(step.path().to_string()),
            Err(error) => report.failed.push(FailedStep { path: step.path().to_string(), error }),
        }
    }
    flush_deletes(&mut pending_deletes, env, &mut report).await;
    if !created_recorded {
        record_created(&mut rec, env.ledger).await;
    }

    if !rec.copied.1.is_empty() {
        let id = uuid::Uuid::new_v4().to_string();
        record_fs_ok(env.ledger, "copy", &rec.copied.0, &rec.copied.1, &id).await;
        report.undo_id = Some(id.clone());
        report.undo_ids.push(id);
    }
    // Undoing the copies first leaves the folders made for them empty.
    report.undo_ids.extend(rec.created_id.take());
    if !rec.replaced.1.is_empty() {
        let id = uuid::Uuid::new_v4().to_string();
        record_fs_ok(env.ledger, "overwrite", &rec.replaced.0, &rec.replaced.1, &id).await;
        // `move_to_trash` deletes on Windows (no Trash support there yet).
        let kept = if env.permanent || cfg!(windows) { "deleted" } else { "moved to the Trash" };
        report.notes.push(format!("{} replaced file(s): the previous versions were {kept}.", rec.replaced.1.len()));
    }
    if rec.uploads_with_new_time > 0 {
        report.notes.push(format!(
            "{} file(s) uploaded to a connection may carry the upload time as their modification time; compare by size only (or checksum) if they still show as different.",
            rec.uploads_with_new_time
        ));
    }
    if !report.kept.is_empty() {
        report.notes.push(format!("{} folder(s) were kept because they still hold files that are not part of this comparison.", report.kept.len()));
    }
    let mut p = CompareProgress::new(env.task_id, ComparePhase::Applying);
    (p.done, p.total) = (report.done.len() as u64 + report.failed.len() as u64, total);
    (env.progress)(p);
    report
}

/// Record the local folders created so far as one undoable operation,
/// deepest first (undoing it removes each folder once it is empty).
async fn record_created(rec: &mut Records, ledger: &OperationLedger) {
    if !rec.created.is_empty() {
        let mut created = std::mem::take(&mut rec.created);
        created.sort_by_key(|p| std::cmp::Reverse(Path::new(p).components().count()));
        let id = uuid::Uuid::new_v4().to_string();
        record_fs_ok(ledger, "create_folder", &[], &created, &id).await;
        rec.created_id = Some(id);
    }
}

/// Delete local files in one batch through the file manager's delete; a
/// failure stops that batch at the file that failed, so the rest are
/// retried after it.
async fn flush_deletes(pending: &mut Vec<(String, String)>, env: &ApplyEnv<'_>, report: &mut ExecReport) {
    let mut remaining = std::mem::take(pending);
    while !remaining.is_empty() {
        let paths: Vec<String> = remaining.iter().map(|(_, abs)| abs.clone()).collect();
        match delete_files_impl(paths, env.permanent, env.ledger).await {
            Ok(_) => {
                report.done.extend(remaining.drain(..).map(|(rel, _)| rel));
            }
            Err(e) => {
                // The batch runs in order and skips what is already gone:
                // the first file still there is the one that failed.
                match remaining.iter().position(|(_, abs)| std::fs::symlink_metadata(abs).is_ok()) {
                    Some(i) => {
                        report.done.extend(remaining.drain(..i).map(|(rel, _)| rel));
                        let (rel, _) = remaining.remove(0);
                        report.failed.push(FailedStep { path: rel, error: e.to_string() });
                    }
                    None => report.done.extend(remaining.drain(..).map(|(rel, _)| rel)),
                }
            }
        }
    }
}

fn connector_event(kind: &str, status: LedgerStatus, subject: &str, target: Option<&str>, bytes: Option<u64>) -> RecordEvent {
    let summary = match target {
        Some(t) => format!("{kind}: {subject} → {t}"),
        None => format!("{kind}: {subject}"),
    };
    let mut ev = RecordEvent::new(LedgerEngine::Connector, kind).status(status).subject(subject).summary(summary);
    if let Some(t) = target {
        ev = ev.target(t);
    }
    if let Some(b) = bytes {
        ev = ev.bytes(b as i64);
    }
    ev
}

async fn make_dir(target: &ResolvedSide, rel: &str, ledger: &OperationLedger, rec: &mut Records) -> Result<(), String> {
    match target {
        ResolvedSide::Local { .. } => {
            let path = target.local_path(rel)?.ok_or("not a local folder")?;
            if path.is_dir() {
                return Ok(());
            }
            crate::commands::policy_commands::enforce_path(&path).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&path).map_err(|e| format!("Cannot create folder {}: {e}", path.display()))?;
            rec.created.push(path.to_string_lossy().to_string());
            Ok(())
        }
        ResolvedSide::Remote { connector, .. } => {
            let abs = target.abs_new(rel)?;
            match connector.make_directory(&abs).await {
                Ok(()) => {
                    ledger.record(connector_event("create_folder", LedgerStatus::Ok, &target.label(rel), None, None)).await;
                    // What goes in it is addressed by the path the
                    // connection lists it at (an ID on Google Drive).
                    if let Err(e) = remote_lookup(target, rel).await {
                        tracing::debug!("Could not list the folder {abs} was made in: {e}");
                    }
                    Ok(())
                }
                // Some servers refuse to create a folder that exists.
                Err(e) => match remote_lookup(target, rel).await {
                    Ok(Some(found)) if found.is_dir => {
                        tracing::debug!("{abs} already exists: {e}");
                        Ok(())
                    }
                    _ => Err(e.to_string()),
                },
            }
        }
    }
}

/// List the folder `rel` is in on a remote side, noting where the
/// connection lists each item in it.
async fn list_parent(target: &ResolvedSide, rel: &str) -> Result<Vec<crate::core::types::FileEntry>, String> {
    let ResolvedSide::Remote { connector, .. } = target else { return Err("not a connection".to_string()) };
    let parent = rel.rsplit_once('/').map_or("", |(p, _)| p);
    let dir = target.abs(parent)?;
    let listing = connector.list_remote(&dir).await.map_err(|e| e.to_string())?;
    for e in &listing {
        let name = e.name.trim_end_matches('/');
        let child = if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") };
        target.remember(&child, &e.path);
    }
    Ok(listing)
}

/// What `rel` is on a remote side now: `Ok(None)` only when the folder it is
/// in lists without it; a listing that fails is an error, never "gone".
async fn remote_lookup(target: &ResolvedSide, rel: &str) -> Result<Option<crate::core::types::FileEntry>, String> {
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    let found = list_parent(target, rel).await?.into_iter().find(|e| e.name.trim_end_matches('/') == name);
    if found.is_none() {
        target.forget(rel);
    }
    Ok(found)
}

/// Whether `rel` is on a remote side now. Each folder is listed once per
/// action (`cache`); a folder that cannot be listed is an error, so the
/// caller never writes where it could not look.
async fn remote_exists(target: &ResolvedSide, rel: &str, cache: &mut HashMap<String, HashSet<String>>) -> Result<bool, String> {
    let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    if !cache.contains_key(parent) {
        let listing = list_parent(target, rel).await?;
        cache.insert(parent.to_string(), listing.into_iter().map(|e| e.name.trim_end_matches('/').to_string()).collect());
    }
    Ok(cache.get(parent).is_some_and(|names| names.contains(name)))
}

/// Marks the hidden sibling a local copy is written to.
const STAGING_MARK: &str = ".ufop-compare-";

/// Hidden sibling a local copy is written to before it is moved into place.
pub(crate) fn staging_for(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tag = &uuid::Uuid::new_v4().simple().to_string()[..8];
    dest.with_file_name(crate::connectors::staging_name(&name, &format!("{STAGING_MARK}{tag}")))
}

/// Whether `path` is a staging file of a local copy
/// (`.<name>.ufop-compare-<8 hex digits>`). The transfer queue does not
/// keep such jobs over a restart: the copy they belong to ended with the app.
pub fn is_staging(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.strip_prefix('.')
        .and_then(|n| n.rsplit_once(STAGING_MARK))
        .is_some_and(|(stem, tag)| !stem.is_empty() && tag.len() == 8 && tag.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Remove staging files in `dir` left by local copies that were interrupted
/// (the app closed mid-copy), except those an unfinished queue job writes.
async fn sweep_staging(dir: &Path, transfer: &TransferManager) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let stale: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && is_staging(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    if stale.is_empty() {
        return;
    }
    let busy: HashSet<PathBuf> = transfer
        .snapshot_jobs()
        .await
        .into_iter()
        .filter(|j| !matches!(j.status, TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled))
        .map(|j| PathBuf::from(j.dest_path))
        .collect();
    for path in stale.into_iter().filter(|p| !busy.contains(p)) {
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!("Removed a staging file left by an interrupted copy: {}", path.display()),
            Err(e) => tracing::debug!("Could not remove {}: {e}", path.display()),
        }
    }
}

/// Move a finished local copy at `staging` to `dest`, keeping `modified`.
/// A file already at `dest` is replaced only when the plan said so (moved
/// to the Trash first unless `permanent`); otherwise the copy is dropped.
fn settle_local(staging: &Path, dest: &Path, modified: Option<DateTime<Utc>>, replace: bool, permanent: bool) -> Result<(), String> {
    let discard = |e: String| {
        let _ = std::fs::remove_file(staging);
        e
    };
    if let Some(t) = modified {
        let _ = std::fs::File::options().write(true).open(staging).and_then(|f| f.set_modified(t.into()));
    }
    if std::fs::symlink_metadata(dest).is_ok() {
        if !replace {
            return Err(discard(format!("{} appeared after the comparison; it was not overwritten. Compare again.", dest.display())));
        }
        let gone = if permanent { std::fs::remove_file(dest).map_err(|e| e.to_string()) } else { move_to_trash(dest).map_err(|e| e.to_string()) };
        gone.map_err(|e| discard(format!("Could not move the old {} out of the way: {e}", dest.display())))?;
    }
    std::fs::rename(staging, dest).map_err(|e| discard(format!("Copied, but could not move the copy into place as {}: {e}", dest.display())))
}

/// Wait for transfer-engine job `id` to end; cancelling stops the job.
async fn wait_for_job(transfer: &TransferManager, id: uuid::Uuid, cancel: &CancellationToken) -> Result<(), String> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = transfer.cancel(id).await;
                return Err("Cancelled.".to_string());
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
        match transfer.get_job(id).await {
            Ok(Some(job)) => match job.status {
                TransferStatus::Completed => return Ok(()),
                TransferStatus::Failed => return Err(job.error_message.unwrap_or_else(|| "The transfer failed.".to_string())),
                TransferStatus::Cancelled => return Err("The transfer was cancelled.".to_string()),
                _ => {}
            },
            Ok(None) => return Err("The transfer job disappeared from the queue.".to_string()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Why a tracked connector transfer stopped before it finished.
const CANCELLED_IN_QUEUE: &str = "Cancelled from the transfer queue.";

/// Run a connector transfer as a tracked queue job. Cancelling the action,
/// or the job's row in the queue, drops the transfer (the connector stops
/// at its next step) and leaves the job cancelled.
async fn tracked<F>(env: &ApplyEnv<'_>, source: &str, dest: &str, size: u64, connection: Option<uuid::Uuid>, work: F) -> Result<u64, String>
where
    F: std::future::Future<Output = Result<u64, AppError>>,
{
    let transfer = env.transfer;
    let (started_at, started) = (Utc::now(), std::time::Instant::now());
    let job = transfer.track_external(source, dest, size, connection).await.ok();
    let job_id = job.as_ref().map(|j| j.id);
    let stopped = async {
        loop {
            tokio::select! {
                _ = env.cancel.cancelled() => return "Cancelled.",
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
            if let Some(id) = job_id {
                if matches!(transfer.get_job(id).await, Ok(Some(j)) if j.status == TransferStatus::Cancelled) {
                    return CANCELLED_IN_QUEUE;
                }
            }
        }
    };
    let result = tokio::select! {
        r = work => r.map_err(|e| e.to_string()),
        why = stopped => Err(why.to_string()),
    };
    if let Some(id) = job_id {
        if env.cancel.is_cancelled() {
            let _ = transfer.cancel(id).await;
        }
        transfer.finish_external(id, result.clone(), started_at, started.elapsed()).await;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn copy_file(
    from: &ResolvedSide,
    to: &ResolvedSide,
    rel: &str,
    size: u64,
    modified: Option<DateTime<Utc>>,
    replace: bool,
    env: &ApplyEnv<'_>,
    rec: &mut Records,
) -> Result<(), String> {
    let (src_label, dest_label) = (from.label(rel), to.label(rel));
    match to {
        ResolvedSide::Local { .. } => {
            let dest = to.local_path(rel)?.ok_or("not a local folder")?;
            crate::commands::policy_commands::enforce_path(&dest).map_err(|e| e.to_string())?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create folder {}: {e}", parent.display()))?;
                if rec.swept.insert(parent.to_path_buf()) {
                    sweep_staging(parent, env.transfer).await;
                }
            }
            let staging = staging_for(&dest);
            let staging_str = staging.to_string_lossy().to_string();
            match from {
                ResolvedSide::Local { .. } => {
                    let src = from.abs(rel)?;
                    let job = env.transfer.enqueue(&src, &staging_str, size).await.map_err(|e| e.to_string())?;
                    env.transfer.spawn_runner(job.id).await;
                    if let Err(e) = wait_for_job(env.transfer, job.id, env.cancel).await {
                        let _ = std::fs::remove_file(&staging);
                        return Err(e);
                    }
                }
                ResolvedSide::Remote { connector, .. } => {
                    let remote = from.abs(rel)?;
                    let got = tracked(env, &src_label, &dest.to_string_lossy(), size, from.connection_id(), connector.download(&remote, &staging_str)).await;
                    if let Err(e) = got {
                        let _ = std::fs::remove_file(&staging);
                        return Err(e);
                    }
                }
            }
            settle_local(&staging, &dest, modified, replace, env.permanent)?;
            let list = if replace { &mut rec.replaced } else { &mut rec.copied };
            list.0.push(src_label);
            list.1.push(dest.to_string_lossy().to_string());
            Ok(())
        }
        ResolvedSide::Remote { connector, .. } => {
            // Uploads name the folder (as the connection listed it) and the
            // file's name; the file's own listed path may be an ID.
            let remote = to.abs_new(rel)?;
            // A file the comparison did not see on the connection is never
            // overwritten (uploads replace silently), like local copies.
            if !replace {
                match remote_exists(to, rel, &mut rec.remote_names).await {
                    Ok(false) => {}
                    Ok(true) => return Err(format!("{dest_label} appeared after the comparison; it was not overwritten. Compare again.")),
                    Err(e) => return Err(format!("Could not check whether {dest_label} already exists, so it was not copied: {e}")),
                }
            }
            let (local, temp) = match from {
                ResolvedSide::Local { .. } => (from.abs(rel)?, None),
                ResolvedSide::Remote { connector: source, .. } => {
                    // Between two connections the bytes pass through a temp file.
                    let temp = ufop_core::session::user_temp_subdir("compare").join(uuid::Uuid::new_v4().simple().to_string());
                    let temp_str = temp.to_string_lossy().to_string();
                    let got = source.download(&from.abs(rel)?, &temp_str).await;
                    if let Err(e) = got {
                        let _ = std::fs::remove_file(&temp);
                        return Err(e.to_string());
                    }
                    (temp_str, Some(temp))
                }
            };
            let sent = tracked(env, &src_label, &dest_label, size, to.connection_id(), connector.upload(&local, &remote)).await;
            if let Some(t) = temp {
                let _ = std::fs::remove_file(t);
            }
            let kind = if replace { "overwrite" } else { "copy" };
            match sent {
                Ok(bytes) => {
                    env.ledger.record(connector_event(kind, LedgerStatus::Ok, &src_label, Some(&dest_label), Some(bytes))).await;
                    rec.uploads_with_new_time += 1;
                    Ok(())
                }
                Err(e) => {
                    // A new file stopped half way is not left behind.
                    if !replace && (env.cancel.is_cancelled() || e == CANCELLED_IN_QUEUE) {
                        let partial = match remote_lookup(to, rel).await {
                            Ok(Some(found)) if !found.path.is_empty() => found.path,
                            _ => remote.clone(),
                        };
                        if connector.remove(&partial, false).await.is_ok() {
                            to.forget(rel);
                        }
                    }
                    env.ledger.record(connector_event(kind, LedgerStatus::Failed, &src_label, Some(&dest_label), None)).await;
                    Err(e)
                }
            }
        }
    }
}

async fn delete_remote_file(target: &ResolvedSide, rel: &str, ledger: &OperationLedger) -> Result<(), String> {
    let ResolvedSide::Remote { connector, .. } = target else { return Err("not a connection".to_string()) };
    let result = connector.remove(&target.abs(rel)?, false).await;
    let status = if result.is_ok() { LedgerStatus::Ok } else { LedgerStatus::Failed };
    ledger.record(connector_event("delete", status, &target.label(rel), None, None)).await;
    result.map(|_| target.forget(rel)).map_err(|e| e.to_string())
}

/// Remove a folder once it is empty: `Ok(false)` when something is still in it.
async fn delete_dir(target: &ResolvedSide, rel: &str, env: &ApplyEnv<'_>) -> Result<bool, String> {
    match target {
        ResolvedSide::Local { .. } => {
            let path = target.local_path(rel)?.ok_or("not a local folder")?;
            if !path.exists() {
                return Ok(true);
            }
            #[cfg(test)]
            before_empty_folder_delete(&path);
            // `remove_dir` removes the folder only while it is empty, in one
            // step: a file saved into it meanwhile keeps the folder (a check
            // first and a recursive delete after could take that file along).
            // So an empty folder is removed, not moved to the Trash, even
            // without "permanent": it holds nothing to restore, and the Trash
            // has no "only if empty" move.
            match std::fs::remove_dir(&path) {
                Ok(()) => {
                    let id = uuid::Uuid::new_v4().to_string();
                    record_fs_ok(env.ledger, "delete_permanent", &[path.to_string_lossy().to_string()], &[], &id).await;
                    Ok(true)
                }
                Err(e) => {
                    let has_entries = std::fs::read_dir(&path).map(|mut d| d.next().is_some()).unwrap_or(false);
                    if has_entries {
                        Ok(false)
                    } else if !path.exists() {
                        Ok(true)
                    } else {
                        Err(format!("Cannot delete {}: {e}", path.display()))
                    }
                }
            }
        }
        ResolvedSide::Remote { connector, .. } => {
            let abs = target.abs(rel)?;
            match connector.list_remote(&abs).await {
                Ok(list) if !list.is_empty() => return Ok(false),
                Ok(_) => {}
                // Already gone only when the folder it was in lists without
                // it; when that cannot be listed either, it is a failure.
                Err(e) => match remote_lookup(target, rel).await {
                    Ok(None) => return Ok(true),
                    Ok(Some(_)) | Err(_) => return Err(e.to_string()),
                },
            }
            let result = connector.remove(&abs, true).await;
            let status = if result.is_ok() { LedgerStatus::Ok } else { LedgerStatus::Failed };
            env.ledger.record(connector_event("delete", status, &target.label(rel), None, None)).await;
            result.map(|_| {
                target.forget(rel);
                true
            })
            .map_err(|e| e.to_string())
        }
    }
}

// ── Re-comparing what an action touched ──

/// What `rel` is on a side right now: `Ok(None)` when it is not there.
fn local_meta(side: &ResolvedSide, rel: &str) -> Result<Option<FileMeta>, String> {
    let path = side.local_path(rel)?.ok_or("not a local folder")?;
    let meta = match std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path)) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    Ok(Some(FileMeta {
        relative_path: rel.to_string(),
        absolute_path: path,
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified: meta.modified().ok().map(DateTime::<Utc>::from),
        is_dir: meta.is_dir(),
        is_symlink: false,
        hidden: rel.rsplit('/').next().is_some_and(|n| n.starts_with('.')),
        readonly: false,
        attributes: 0,
    }))
}

/// The current state of `paths` on one side, as a listing.
async fn stat_side(side: &ResolvedSide, paths: &[String]) -> SideListing {
    let mut listing = SideListing::default();
    let add = |rel: &str, r: Result<Option<FileMeta>, String>, listing: &mut SideListing| match r {
        Ok(Some(m)) => listing.entries.push(m),
        Ok(None) => {}
        Err(message) => listing.errors.push(ListingError { path: rel.to_string(), is_dir: false, message }),
    };
    match side {
        ResolvedSide::Local { .. } => {
            for rel in paths {
                add(rel, local_meta(side, rel), &mut listing);
            }
        }
        ResolvedSide::Remote { connector, .. } => {
            // One listing per parent folder; below a folder that is gone,
            // everything is gone.
            let mut sorted: Vec<&String> = paths.iter().collect();
            sorted.sort_by_key(|p| p.matches('/').count());
            let mut listings: HashMap<String, Result<Vec<crate::core::types::FileEntry>, String>> = HashMap::new();
            let mut absent: HashSet<String> = HashSet::new();
            for rel in sorted {
                let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
                if absent.contains(parent) {
                    absent.insert(rel.clone());
                    side.forget(rel);
                    continue;
                }
                if !listings.contains_key(parent) {
                    let got = match side.abs(parent) {
                        Ok(abs) => connector.list_remote(&abs).await.map_err(|e| e.to_string()),
                        Err(e) => Err(e),
                    };
                    listings.insert(parent.to_string(), got);
                }
                let found = match &listings[parent] {
                    Ok(list) => Ok(list.iter().find(|e| e.name.trim_end_matches('/') == name).map(|e| FileMeta {
                        relative_path: rel.clone(),
                        // Where the connection lists it (an ID on Google
                        // Drive); what is below it is listed from there.
                        absolute_path: PathBuf::from(if e.path.is_empty() { side.abs_new(rel).unwrap_or_default() } else { e.path.clone() }),
                        size: if e.is_dir { 0 } else { e.size },
                        modified: e.modified,
                        is_dir: e.is_dir,
                        is_symlink: false,
                        hidden: name.starts_with('.'),
                        readonly: false,
                        attributes: 0,
                    })),
                    Err(e) => Err(e.clone()),
                };
                match &found {
                    Ok(Some(m)) => side.remember(rel, &m.absolute_path.to_string_lossy()),
                    Ok(None) => {
                        absent.insert(rel.clone());
                        side.forget(rel);
                    }
                    Err(_) => {}
                }
                add(rel, found, &mut listing);
            }
        }
    }
    listing
}

/// Re-compare `touched` paths on both sides: their entries now (paths on
/// neither side are left out).
pub async fn refresh(left: &ResolvedSide, right: &ResolvedSide, opts: &CompareOptions, touched: &HashSet<String>) -> Result<Vec<CompareEntry>, AppError> {
    let paths: Vec<String> = touched.iter().cloned().collect();
    let (l, r) = tokio::join!(stat_side(left, &paths), stat_side(right, &paths));
    let hasher = LocalSidesHasher { left_local: left.is_local(), right_local: right.is_local() };
    let opts = opts.clone();
    tokio::task::spawn_blocking(move || compare::compare_entries(&l, &r, &opts, Some(&hasher), None).map(|(entries, _)| entries))
        .await
        .map_err(|e| AppError::internal(format!("Comparison task failed: {e}")))?
}

/// Every path an action covers (for re-comparing): the steps' paths.
pub fn touched_by(plan: &ActionPlan) -> HashSet<String> {
    plan.steps.iter().map(|s| s.path().to_string()).collect()
}

/// Tests: runs just before an empty local folder is removed (to put a file
/// in it at the worst moment).
#[cfg(test)]
pub(crate) static BEFORE_EMPTY_FOLDER_DELETE: std::sync::Mutex<Option<FolderHook>> = std::sync::Mutex::new(None);
#[cfg(test)]
pub(crate) type FolderHook = Box<dyn Fn(&Path) + Send>;

#[cfg(test)]
fn before_empty_folder_delete(path: &Path) {
    if let Some(f) = BEFORE_EMPTY_FOLDER_DELETE.lock().unwrap().as_ref() {
        f(path);
    }
}
