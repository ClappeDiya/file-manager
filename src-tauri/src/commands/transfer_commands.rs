use crate::core::error::AppError;
use crate::core::traits::TransferOperations;
use crate::core::types::{
    ChecksumAlgorithm, ConflictPolicy, ExportFormat, RetryPolicy, ThrottleConfig,
    TransferConfig, TransferHistoryFilter, TransferHistoryRecord, TransferJob,
    TransferPriority, TransferStatus, VerifyTier,
};
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::transfer_engine::conflict::{self, ConflictAction, ConflictCheck};
use crate::transfer_engine::history::TransferHistory;
use crate::transfer_engine::verification::{self, VerificationResult};
use crate::transfer_engine::TransferManager;
use tauri::State;
use uuid::Uuid;

/// DRY ledger helper for the transfer engine. Builds a `RecordEvent`
/// with the supplied fields and records it (fail-open).
#[allow(clippy::too_many_arguments)]
async fn record_xfer(
    ledger: &OperationLedger,
    kind: &str,
    status: LedgerStatus,
    job_id: &str,
    summary: String,
    subject: Option<String>,
    target: Option<String>,
    bytes: Option<u64>,
) {
    let mut ev = RecordEvent::new(LedgerEngine::Transfer, kind)
        .status(status)
        .correlation(job_id)
        .summary(summary);
    if let Some(s) = subject {
        ev = ev.subject(s);
    }
    if let Some(t) = target {
        ev = ev.target(t);
    }
    if let Some(b) = bytes {
        ev = ev.bytes(b as i64);
    }
    ledger.record(ev).await;
}

// ── T-015: Transfer Queue Engine ──

/// Enqueue a new transfer job.
#[tauri::command]
pub async fn enqueue_transfer(
    source: String,
    dest: String,
    total_bytes: u64,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<TransferJob, AppError> {
    // `dest` may name the file or the folder it goes into.
    let target = match std::path::Path::new(&dest) {
        d if d.is_dir() => d.join(std::path::Path::new(&source).file_name().unwrap_or_default()),
        d => d.to_path_buf(),
    };
    super::policy_commands::enforce_path(&target)?;
    let job = manager.enqueue(&source, &dest, total_bytes).await?;
    // Run it: a queued job otherwise never starts.
    manager.spawn_runner(job.id).await;
    record_xfer(
        &ledger,
        "enqueue",
        LedgerStatus::Ok,
        &job.id.to_string(),
        format!("enqueue: {source} → {dest}"),
        Some(source),
        Some(dest),
        Some(total_bytes),
    )
    .await;
    Ok(job)
}

/// An item a pane move renamed on the server (same connection).
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct MovedItem {
    pub from: String,
    pub to: String,
}

/// What `enqueue_location_transfer` started.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocationTransferOutcome {
    /// One queued job per file.
    pub jobs: Vec<TransferJob>,
    /// Moved on the server, immediately (a move within one connection).
    pub moved: Vec<MovedItem>,
    /// Destination folders created for the copied folders.
    pub folders_created: usize,
    /// Items a move converted on the way (Google documents exported to
    /// files) and so kept at the source: pane paths of the originals.
    pub kept: Vec<String>,
}

/// Copy or move pane items into `dest_dir` when either end is a saved
/// connection (`conn://<id>/<path>`): files become transfer-queue jobs
/// (upload, download, server-side copy or relay), folders are created first,
/// and a move deletes each source once its copy completed. A move within one
/// connection is a server-side rename where the connector has one. Between
/// two folders of this computer use `copy_files` / `move_files`.
#[tauri::command]
pub async fn enqueue_location_transfer(
    items: Vec<crate::transfer_engine::location::LocationItem>,
    dest_dir: String,
    mode: crate::transfer_engine::location::TransferMode,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<LocationTransferOutcome, AppError> {
    let resolver = manager.remote_resolver().ok_or_else(|| AppError::connection("Connections are not available yet.", "Try again in a moment."))?;
    let outcome = location_transfer(&manager, resolver.as_ref(), items, &dest_dir, mode).await?;
    record_xfer(
        &ledger,
        "enqueue",
        LedgerStatus::Ok,
        &outcome.jobs.first().map(|j| j.id.to_string()).unwrap_or_else(|| Uuid::new_v4().to_string()),
        format!("{} {} job(s), {} moved on the server → {dest_dir}", if mode == crate::transfer_engine::location::TransferMode::Move { "move" } else { "copy" }, outcome.jobs.len(), outcome.moved.len()),
        None,
        Some(dest_dir.clone()),
        Some(outcome.jobs.iter().map(|j| j.total_bytes).sum()),
    )
    .await;
    Ok(outcome)
}

pub(crate) async fn location_transfer(
    manager: &TransferManager,
    resolver: &dyn crate::transfer_engine::location::RemoteResolver,
    items: Vec<crate::transfer_engine::location::LocationItem>,
    dest_dir: &str,
    mode: crate::transfer_engine::location::TransferMode,
) -> Result<LocationTransferOutcome, AppError> {
    location_transfer_with(manager, resolver, items, dest_dir, mode, &ufop_core::policy::active()).await
}

/// [`location_transfer`] checked against the filename rules `policy`.
pub(crate) async fn location_transfer_with(
    manager: &TransferManager,
    resolver: &dyn crate::transfer_engine::location::RemoteResolver,
    items: Vec<crate::transfer_engine::location::LocationItem>,
    dest_dir: &str,
    mode: crate::transfer_engine::location::TransferMode,
    policy: &ufop_core::policy::PolicySet,
) -> Result<LocationTransferOutcome, AppError> {
    use crate::transfer_engine::location::{self as loc, Location, TransferMode};
    let dest = Location::parse(dest_dir)?;
    let mut sources = Vec::with_capacity(items.len());
    for item in &items {
        sources.push(Location::parse(&item.path)?);
    }
    if !dest.is_remote() && sources.iter().all(|s| !s.is_remote()) {
        return Err(AppError::validation("Both ends are on this computer: use the regular copy or move."));
    }
    // A move inside one connection: rename on the server where possible.
    let mut moved = Vec::new();
    let mut rest = Vec::new();
    for (item, source) in items.into_iter().zip(sources) {
        let (Location::Remote { connection_id, path: from }, Location::Remote { path: dest_path, .. }) = (&source, &dest) else {
            rest.push(item);
            continue;
        };
        if mode != TransferMode::Move || !source.same_root(&dest) {
            rest.push(item);
            continue;
        }
        if loc::same_remote_folder(&loc::remote_parent(from), dest_path) {
            continue; // already there
        }
        // Never ask the server to move a folder into itself.
        if loc::is_within(&source, &dest) {
            return Err(AppError::validation(format!("Cannot put {} inside itself", item.name)));
        }
        let conn = resolver.connector(*connection_id).await?;
        let listing = conn.list_remote(dest_path).await?;
        // Already listed there, however the connector spells the folder
        // (Google Drive lists a shared-drive root's items under
        // `/shared-drives/<id>/`, which the pane calls `/`).
        if listing.iter().any(|e| loc::at_path(e, from)) {
            continue;
        }
        let name = loc::free_name(&item.name, &mut loc::taken_names(listing))?;
        let to = loc::join_remote(dest_path, &name);
        match conn.rename_entry(from, &to).await {
            // Where it is listed now (Google Drive: its id under the new folder).
            Ok(()) => moved.push(MovedItem { from: item.path.clone(), to: Location::Remote { connection_id: *connection_id, path: conn.renamed_path(from, &to) }.to_location_string() }),
            // No server-side move here: copy, then delete.
            Err(e) => {
                tracing::info!("Server-side move of {from} unavailable ({e}); copying instead");
                rest.push(item);
            }
        }
    }

    let mut plan = loc::plan_transfer(&rest, &dest, resolver).await?;
    // Every file landing on this computer (those inside folders, export and
    // keep-both names too) is checked before anything is created or queued.
    let local_targets: Vec<std::path::PathBuf> = plan.files.iter().filter_map(|f| match Location::parse(&f.dest) {
        Ok(Location::Local(p)) => Some(std::path::PathBuf::from(p)),
        _ => None,
    }).collect();
    super::policy_commands::enforce(policy.check_all(&local_targets))?;
    loc::create_dirs(&mut plan, resolver).await?;
    let mut jobs = Vec::with_capacity(plan.files.len());
    for f in &plan.files {
        jobs.push(manager.enqueue_location_job(&f.source, &f.dest, f.size).await?);
    }
    if mode == TransferMode::Move {
        // A converted copy is not the original: keep that one (its folder
        // then stays too, as it is not empty).
        let pairs: Vec<_> = jobs.iter().zip(&plan.files).filter(|(_, f)| !f.converted).map(|(j, f)| (j.id, f.source.clone())).collect();
        if pairs.is_empty() {
            for dir in &plan.source_dirs {
                let _ = loc::remove_empty_dir(&Location::parse(dir)?, resolver).await;
            }
        } else {
            manager.register_move(pairs, plan.source_dirs.clone()).await;
        }
    }
    let kept = if mode == TransferMode::Move { plan.files.iter().filter(|f| f.converted).map(|f| f.source.clone()).collect() } else { Vec::new() };
    for job in &jobs {
        manager.spawn_runner(job.id).await;
    }
    Ok(LocationTransferOutcome { jobs, moved, folders_created: plan.dirs.len(), kept })
}

/// Pause a transfer job.
#[tauri::command]
pub async fn pause_transfer(
    job_id: String,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.pause(id).await?;
    record_xfer(
        &ledger,
        "pause",
        LedgerStatus::Ok,
        &job_id,
        format!("pause: {job_id}"),
        None,
        None,
        None,
    )
    .await;
    Ok(())
}

/// Resume a paused transfer job.
#[tauri::command]
pub async fn resume_transfer(
    job_id: String,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.resume(id).await?;
    record_xfer(
        &ledger,
        "resume",
        LedgerStatus::Ok,
        &job_id,
        format!("resume: {job_id}"),
        None,
        None,
        None,
    )
    .await;
    Ok(())
}

/// Cancel a transfer job.
#[tauri::command]
pub async fn cancel_transfer(
    job_id: String,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.cancel(id).await?;
    record_xfer(
        &ledger,
        "cancel",
        LedgerStatus::Cancelled,
        &job_id,
        format!("cancel: {job_id}"),
        None,
        None,
        None,
    )
    .await;
    Ok(())
}

/// List all transfer jobs.
#[tauri::command]
pub async fn list_transfers(
    manager: State<'_, TransferManager>,
) -> Result<Vec<TransferJob>, AppError> {
    manager.list_jobs().await
}

/// Get a single transfer job.
#[tauri::command]
pub async fn get_transfer(
    job_id: String,
    manager: State<'_, TransferManager>,
) -> Result<Option<TransferJob>, AppError> {
    let id = parse_uuid(&job_id)?;
    manager.get_job(id).await
}

/// Set priority of a transfer job.
#[tauri::command]
pub async fn set_transfer_priority(
    job_id: String,
    priority: String,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    let p = TransferPriority::from_str_lossy(&priority);
    manager.set_priority(id, p).await
}

/// Reorder a transfer job in the queue.
#[tauri::command]
pub async fn reorder_transfer(
    job_id: String,
    new_sort_order: i32,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.reorder_job(id, new_sort_order).await
}

/// Set global bandwidth throttle.
#[tauri::command]
pub async fn set_global_throttle(
    bytes_per_sec: u64,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    manager.set_global_throttle(bytes_per_sec).await;
    Ok(())
}

/// Set per-connection bandwidth throttle.
#[tauri::command]
pub async fn set_connection_throttle(
    connection_id: String,
    bytes_per_sec: u64,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&connection_id)?;
    manager.set_connection_throttle(id, bytes_per_sec).await;
    Ok(())
}

/// Get current throttle configuration.
#[tauri::command]
pub async fn get_throttle_config(
    manager: State<'_, TransferManager>,
) -> Result<ThrottleConfig, AppError> {
    Ok(manager.throttle_config().await)
}

/// Update transfer progress (for internal use or simulation).
#[tauri::command]
pub async fn update_transfer_progress(
    job_id: String,
    transferred_bytes: u64,
    speed_bps: u64,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.update_progress(id, transferred_bytes, speed_bps).await
}

/// Complete a transfer job.
#[tauri::command]
pub async fn complete_transfer(
    job_id: String,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.complete_job(id).await?;
    // Pull the finished job so we can record bytes/source/dest in the timeline.
    let (subject, target, bytes) = match manager.get_job(id).await.ok().flatten() {
        Some(j) => (Some(j.source_path), Some(j.dest_path), Some(j.total_bytes)),
        None => (None, None, None),
    };
    record_xfer(
        &ledger,
        "complete",
        LedgerStatus::Ok,
        &job_id,
        format!("complete: {job_id}"),
        subject,
        target,
        bytes,
    )
    .await;
    Ok(())
}

/// Fail a transfer job.
#[tauri::command]
pub async fn fail_transfer(
    job_id: String,
    error: String,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    manager.fail_job(id, &error).await?;
    record_xfer(
        &ledger,
        "fail",
        LedgerStatus::Failed,
        &job_id,
        format!("fail: {error}"),
        None,
        None,
        None,
    )
    .await;
    Ok(())
}

// ── T-016: Resume and Retry Logic ──

/// Retry a failed transfer job.
#[tauri::command]
pub async fn retry_transfer(
    job_id: String,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<bool, AppError> {
    let id = parse_uuid(&job_id)?;
    let allowed = manager.increment_retry(id).await?;
    if allowed {
        // Re-queued jobs need an executor, just like fresh enqueues.
        manager.spawn_runner(id).await;
        record_xfer(
            &ledger,
            "retry",
            LedgerStatus::Ok,
            &job_id,
            format!("retry: {job_id}"),
            None,
            None,
            None,
        )
        .await;
    }
    Ok(allowed)
}

/// Retry all failed transfer jobs.
#[tauri::command]
pub async fn retry_all_failed(
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<u32, AppError> {
    let requeued = manager.retry_all_failed().await?;
    let count = requeued.len() as u32;
    for id in requeued {
        manager.spawn_runner(id).await;
    }
    if count > 0 {
        record_xfer(
            &ledger,
            "retry_all",
            LedgerStatus::Ok,
            "",
            format!("retry_all_failed: {count} jobs"),
            None,
            None,
            None,
        )
        .await;
    }
    Ok(count)
}

/// List only failed transfer jobs.
#[tauri::command]
pub async fn list_failed_transfers(
    manager: State<'_, TransferManager>,
) -> Result<Vec<TransferJob>, AppError> {
    manager.list_failed_jobs().await
}

/// Set the global retry policy.
#[tauri::command]
pub async fn set_retry_policy(
    max_retries: u32,
    backoff_seconds: Vec<u64>,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    manager
        .set_retry_policy(RetryPolicy {
            max_retries,
            backoff_seconds,
        })
        .await;
    Ok(())
}

// ── T-017: Conflict Resolution ──

/// Check for a conflict at the destination path.
#[tauri::command]
pub async fn check_conflict(
    source_path: String,
    dest_path: String,
) -> Result<ConflictCheck, AppError> {
    conflict::check_conflict(&source_path, &dest_path).await
}

/// Resolve a conflict with the given policy.
#[tauri::command]
pub async fn resolve_conflict(
    job_id: String,
    source_path: String,
    dest_path: String,
    policy: String,
) -> Result<ConflictAction, AppError> {
    let id = parse_uuid(&job_id)?;
    let p = ConflictPolicy::from_str_lossy(&policy);
    let (action, _resolution) = conflict::resolve_conflict(id, &source_path, &dest_path, p).await?;
    Ok(action)
}

/// Set the global conflict policy.
#[tauri::command]
pub async fn set_conflict_policy(
    policy: String,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let p = ConflictPolicy::from_str_lossy(&policy);
    manager.set_conflict_policy(p).await;
    Ok(())
}

/// Set conflict policy for a specific transfer job.
#[tauri::command]
pub async fn set_transfer_conflict_policy(
    job_id: String,
    policy: String,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&job_id)?;
    let p = ConflictPolicy::from_str_lossy(&policy);
    manager.set_job_conflict_policy(id, p).await
}

// ── T-018: Post-Transfer Verification and History ──

/// Verify a completed transfer with checksums.
#[tauri::command]
pub async fn verify_transfer(
    source_path: String,
    dest_path: String,
    algorithm: String,
) -> Result<VerificationResult, AppError> {
    let alg = ChecksumAlgorithm::from_str_lossy(&algorithm);
    verification::verify_transfer(&source_path, &dest_path, alg).await
}

/// Compute checksum of a file.
#[tauri::command]
pub async fn compute_checksum(
    path: String,
    algorithm: String,
) -> Result<String, AppError> {
    let alg = ChecksumAlgorithm::from_str_lossy(&algorithm);
    verification::compute_file_checksum(&path, alg).await
}

/// Enable/disable global checksum verification.
#[tauri::command]
pub async fn set_verify_checksums(
    verify: bool,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    manager.set_verify_checksums(verify).await;
    Ok(())
}

/// Set checksum algorithm.
#[tauri::command]
pub async fn set_checksum_algorithm(
    algorithm: String,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let alg = ChecksumAlgorithm::from_str_lossy(&algorithm);
    manager.set_checksum_algorithm(alg).await;
    Ok(())
}

/// Search transfer history.
#[tauri::command]
pub async fn search_transfer_history(
    path_contains: Option<String>,
    status: Option<String>,
    connection_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    history: State<'_, TransferHistory>,
) -> Result<Vec<TransferHistoryRecord>, AppError> {
    let filter = TransferHistoryFilter {
        path_contains,
        status: status.map(|s| TransferStatus::from_str_lossy(&s)),
        connection_id: connection_id.and_then(|s| Uuid::parse_str(&s).ok()),
        date_from: None,
        date_to: None,
        limit,
        offset,
    };
    history.search(filter).await
}

/// Export transfer history.
#[tauri::command]
pub async fn export_transfer_history(
    format: String,
    path_contains: Option<String>,
    status: Option<String>,
    history: State<'_, TransferHistory>,
) -> Result<String, AppError> {
    let fmt = match format.as_str() {
        "csv" => ExportFormat::Csv,
        "json" => ExportFormat::Json,
        _ => ExportFormat::Json,
    };
    let filter = TransferHistoryFilter {
        path_contains,
        status: status.map(|s| TransferStatus::from_str_lossy(&s)),
        ..Default::default()
    };
    history.export(filter, fmt).await
}

/// Clean up old transfer history records.
#[tauri::command]
pub async fn cleanup_transfer_history(
    older_than_days: Option<u32>,
    history: State<'_, TransferHistory>,
) -> Result<u32, AppError> {
    match older_than_days {
        // The user's "older than N days" choice; without one, the default retention.
        Some(days) => history.cleanup_older_than(days.max(1)).await,
        None => history.cleanup_old_records().await,
    }
}

// ── Three-Layer Architecture Commands ──

/// Set the verification tier for new transfers.
#[tauri::command]
pub async fn set_verify_tier(
    tier: String,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    let t = VerifyTier::from_str_lossy(&tier);
    manager.set_verify_tier(t).await;
    Ok(())
}

/// Get the current verification tier.
#[tauri::command]
pub async fn get_verify_tier(
    manager: State<'_, TransferManager>,
) -> Result<String, AppError> {
    let tier = manager.verify_tier().await;
    Ok(tier.as_str().to_string())
}

/// Get the transfer engine configuration.
#[tauri::command]
pub async fn get_transfer_config(
    manager: State<'_, TransferManager>,
) -> Result<TransferConfig, AppError> {
    Ok(manager.config().await)
}

/// Update the transfer engine configuration.
#[tauri::command]
pub async fn set_transfer_config(
    config: TransferConfig,
    manager: State<'_, TransferManager>,
) -> Result<(), AppError> {
    manager.set_config(config).await;
    Ok(())
}

// ── Completion Hooks ──

/// Get the global completion hooks configuration.
#[tauri::command]
pub async fn get_completion_hooks(
    repo: State<'_, crate::storage::Repository>,
) -> Result<crate::transfer_engine::CompletionHooks, AppError> {
    use crate::core::traits::StorageOperations;

    let run_command = StorageOperations::get_config(repo.inner(), "completion_run_command")
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    let play_sound = StorageOperations::get_config(repo.inner(), "completion_play_sound")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    let open_destination = StorageOperations::get_config(repo.inner(), "completion_open_destination")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    let show_notification = StorageOperations::get_config(repo.inner(), "completion_show_notification")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(true);

    Ok(crate::transfer_engine::CompletionHooks {
        run_command,
        play_sound,
        open_destination,
        show_notification,
    })
}

/// Set the global completion hooks configuration.
#[tauri::command]
pub async fn set_completion_hooks(
    hooks: crate::transfer_engine::CompletionHooks,
    repo: State<'_, crate::storage::Repository>,
) -> Result<(), AppError> {
    use crate::core::traits::StorageOperations;

    StorageOperations::set_config(
        repo.inner(),
        "completion_run_command",
        &hooks.run_command.clone().unwrap_or_default(),
    )
    .await?;
    StorageOperations::set_config(
        repo.inner(),
        "completion_play_sound",
        &hooks.play_sound.to_string(),
    )
    .await?;
    StorageOperations::set_config(
        repo.inner(),
        "completion_open_destination",
        &hooks.open_destination.to_string(),
    )
    .await?;
    StorageOperations::set_config(
        repo.inner(),
        "completion_show_notification",
        &hooks.show_notification.to_string(),
    )
    .await?;
    tracing::info!("Completion hooks updated: {:?}", hooks);
    Ok(())
}

fn parse_uuid(s: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(s).map_err(|_| AppError::Transfer {
        message: format!("Invalid transfer ID: {s}"),
        advice: "Provide a valid UUID.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer_engine::location::tests::{DynResolver, MemConnector, MemResolver};
    use crate::transfer_engine::location::{LocationItem, TransferMode};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn item(path: String, name: &str, is_dir: bool, size: u64) -> LocationItem {
        LocationItem { path, name: name.into(), is_dir, size, extension: None }
    }

    async fn settle(mgr: &TransferManager, jobs: &[TransferJob]) {
        for _ in 0..500 {
            let all = mgr.list_jobs().await.unwrap();
            if jobs.iter().all(|j| all.iter().any(|a| a.id == j.id && matches!(a.status, TransferStatus::Completed | TransferStatus::Failed))) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("jobs did not settle");
    }

    fn drive_resolver(id: Uuid, drive: &crate::connectors::google_drive::fake_drive::FakeDrive) -> Arc<DynResolver> {
        Arc::new(DynResolver(HashMap::from([(id, drive.connector.clone() as Arc<dyn crate::connectors::Connector>)])))
    }

    /// Regression: Google Drive paths are folder-id chains, but a folder
    /// copy planned its subfolders and files by name, so copying any folder
    /// into Drive created the top folder and then failed (Drive: 404 for
    /// parent id "up"), leaving the copy half done. Everything below a new
    /// folder now goes under the id Drive gave it.
    #[tokio::test(flavor = "multi_thread")]
    async fn copying_a_folder_into_google_drive_uses_the_new_folder_ids() {
        let drive = crate::connectors::google_drive::fake_drive::start(&[("f1", "Folder 1", "root", true)]).await;
        let id = Uuid::new_v4();
        let resolver = drive_resolver(id, &drive);
        let mgr = TransferManager::new();
        mgr.set_remote_resolver(resolver.clone());
        let local = tempfile::tempdir().unwrap();
        let folder = local.path().join("up");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("inner/f.txt"), "f").unwrap();
        std::fs::write(folder.join("top.txt"), "t").unwrap();

        let out = location_transfer(&mgr, resolver.as_ref(), vec![item(folder.to_string_lossy().into(), "up", true, 0)], &format!("conn://{id}/f1"), TransferMode::Copy).await.unwrap();
        assert_eq!((out.folders_created, out.jobs.len()), (2, 2));
        settle(&mgr, &out.jobs).await;
        let all = mgr.list_jobs().await.unwrap();
        assert!(all.iter().filter(|j| out.jobs.iter().any(|o| o.id == j.id)).all(|j| matches!(j.status, TransferStatus::Completed)), "{all:?}");
        let up = drive.id_of("up", "f1").expect("the folder is created in f1");
        let inner = drive.id_of("inner", &up).expect("its subfolder is created inside it, by its id");
        assert!(drive.id_of("top.txt", &up).is_some(), "top.txt lands in the new folder");
        assert!(drive.id_of("f.txt", &inner).is_some(), "f.txt lands in the new subfolder");
        // Every folder create named a real parent id.
        let parents: Vec<String> = drive.mock.requests("/api/files").into_iter().filter(|r| r.method == "POST").map(|r| r.json()["parents"][0].as_str().unwrap_or_default().to_string()).collect();
        assert_eq!(parents, vec!["f1".to_string(), up]);
    }

    /// A move inside one Google Drive reports where the item is now: its id
    /// under the new parent (the name is not part of a Drive path).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_side_move_in_google_drive_reports_the_id_path() {
        let drive = crate::connectors::google_drive::fake_drive::start(&[("f1", "Folder 1", "root", true), ("r1", "Reports", "root", true), ("x9", "a.txt", "f1", false)]).await;
        let id = Uuid::new_v4();
        let resolver = drive_resolver(id, &drive);
        let mgr = TransferManager::new();
        let out = location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/f1/x9"), "a.txt", false, 1)], &format!("conn://{id}/r1"), TransferMode::Move).await.unwrap();
        assert!(out.jobs.is_empty());
        assert_eq!(out.moved, vec![MovedItem { from: format!("conn://{id}/f1/x9"), to: format!("conn://{id}/r1/x9") }]);
        assert_eq!(drive.items.lock().unwrap()["x9"].1, "r1");
    }

    /// A move of a folder into itself (or below it) inside one connection is
    /// refused before the server is asked to rename it there.
    #[tokio::test]
    async fn moving_a_folder_into_itself_is_refused_before_any_server_rename() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/a/x.txt", "x")], &["/a", "/a/b"]));
        let resolver = Arc::new(MemResolver(HashMap::from([(id, conn.clone())])));
        let mgr = TransferManager::new();
        for dest in ["/a/b", "/a", "/a/b/"] {
            let err = location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/a"), "a", true, 0)], &format!("conn://{id}{dest}"), TransferMode::Move).await.unwrap_err();
            assert!(err.to_string().contains("inside itself"), "{dest}: {err}");
        }
        assert!(conn.renames.lock().unwrap().is_empty(), "the server was never asked");
        assert_eq!(conn.file("/a/x.txt").as_deref(), Some("x"));
        // A sibling folder is fine.
        conn.dirs.lock().unwrap().insert("/c".into());
        location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/a"), "a", true, 0)], &format!("conn://{id}/c"), TransferMode::Move).await.unwrap();
        assert_eq!(conn.file("/c/a/x.txt").as_deref(), Some("x"));
    }

    /// Regression: a OneDrive pane opened at `/` lists items in Graph's form
    /// (`/drive/root:/a.txt`), whose parent `/drive/root:` did not equal the
    /// pane's `/`, so moving an item into the folder it is already in renamed
    /// it to `a (copy).txt` on the server. Both spellings are one folder.
    #[tokio::test]
    async fn moving_a_onedrive_item_into_its_own_folder_is_a_no_op() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/drive/root:/a.txt", "a"), ("/drives/LIB1/root:/Q3/b.txt", "b")], &["/drives/LIB1/root:/Q3"]));
        let resolver = Arc::new(MemResolver(HashMap::from([(id, conn.clone())])));
        let mgr = TransferManager::new();
        for (path, name, dest) in [("/drive/root:/a.txt", "a.txt", "/"), ("/drive/root:/a.txt", "a.txt", "/drive/root:/"), ("/drives/LIB1/root:/Q3/b.txt", "b.txt", "/Q3/")] {
            let out = location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}{path}"), name, false, 1)], &format!("conn://{id}{dest}"), TransferMode::Move).await.unwrap();
            assert!(out.jobs.is_empty() && out.moved.is_empty(), "{path} -> {dest}: {out:?}");
        }
        assert!(conn.renames.lock().unwrap().is_empty(), "the server was never asked to rename");
        assert_eq!(conn.file("/drive/root:/a.txt").as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn pane_transfers_between_this_computer_and_connections() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/inbox/r.txt", "remote"), ("/dst/up", "taken")], &["/inbox", "/dst"]));
        let resolver = Arc::new(MemResolver(HashMap::from([(id, conn.clone())])));
        let mgr = TransferManager::new();
        mgr.set_remote_resolver(resolver.clone());
        let local = tempfile::tempdir().unwrap();
        let folder = local.path().join("up");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("inner/f.txt"), "f").unwrap();

        // Copy a local folder up: folders first (keep-both name), one job per file.
        let out = location_transfer(&mgr, resolver.as_ref(), vec![item(folder.to_string_lossy().into(), "up", true, 0)], &format!("conn://{id}/dst"), TransferMode::Copy).await.unwrap();
        assert_eq!(out.folders_created, 2);
        assert_eq!(out.jobs.len(), 1);
        settle(&mgr, &out.jobs).await;
        assert_eq!(conn.file("/dst/up (copy)/inner/f.txt").as_deref(), Some("f"));
        assert!(folder.join("inner/f.txt").exists(), "a copy keeps the source");

        // Move a remote file down: downloaded, then the remote original deleted.
        let out = location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/inbox/r.txt"), "r.txt", false, 6)], local.path().to_str().unwrap(), TransferMode::Move).await.unwrap();
        settle(&mgr, &out.jobs).await;
        for _ in 0..200 {
            if conn.file("/inbox/r.txt").is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(std::fs::read_to_string(local.path().join("r.txt")).unwrap(), "remote");
        assert!(conn.file("/inbox/r.txt").is_none());

        // A move inside one connection is a server-side rename, no job.
        let out = location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/dst/up"), "up", false, 5)], &format!("conn://{id}/inbox"), TransferMode::Move).await.unwrap();
        assert!(out.jobs.is_empty());
        assert_eq!(out.moved, vec![MovedItem { from: format!("conn://{id}/dst/up"), to: format!("conn://{id}/inbox/up") }]);
        assert_eq!(conn.file("/inbox/up").as_deref(), Some("taken"));

        // Two folders of this computer: not this command's job.
        let err = location_transfer(&mgr, resolver.as_ref(), vec![item(folder.to_string_lossy().into(), "up", true, 0)], local.path().to_str().unwrap(), TransferMode::Copy).await.unwrap_err();
        assert!(err.to_string().contains("Both ends"));
    }

    /// Regression: a same-connection move picked its name by exact match,
    /// so `A.txt` moved into a folder holding `a.txt` kept its name and a
    /// case-insensitive server (Dropbox, SFTP on Windows) replaced `a.txt`.
    #[tokio::test]
    async fn a_move_onto_a_name_differing_only_by_case_keeps_both() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/s/A.txt", "new"), ("/d/a.txt", "old")], &["/s", "/d"]));
        let resolver = Arc::new(MemResolver(HashMap::from([(id, conn.clone())])));
        let mgr = TransferManager::new();
        let out = location_transfer(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/s/A.txt"), "A.txt", false, 3)], &format!("conn://{id}/d"), TransferMode::Move).await.unwrap();
        assert_eq!(out.moved, vec![MovedItem { from: format!("conn://{id}/s/A.txt"), to: format!("conn://{id}/d/A (copy).txt") }]);
        assert_eq!((conn.file("/d/a.txt").as_deref(), conn.file("/d/A (copy).txt").as_deref()), (Some("old"), Some("new")));
    }

    /// Regression: a download onto this computer checked only the dropped
    /// items' own names against the filename policy, so the files inside a
    /// remote folder (and export / keep-both names) were never checked. Every
    /// planned file is checked, before anything is created or queued.
    #[tokio::test]
    async fn downloads_check_every_planned_file_against_the_filename_policy() {
        use ufop_core::policy::{FilenameRule, PolicySet};
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/r/tools/x.exe", "mz"), ("/r/tools/ok.txt", "ok"), ("/r/Plan", "doc")], &["/r", "/r/tools"]));
        conn.native.lock().unwrap().insert("/r/Plan".into());
        let resolver = Arc::new(MemResolver(HashMap::from([(id, conn.clone())])));
        let mgr = TransferManager::new();
        let local = tempfile::tempdir().unwrap();
        let blocking = PolicySet::new(vec![FilenameRule { blocked_extensions: vec!["exe".into(), "docx".into()], ..FilenameRule::new("t", "") }]).unwrap();
        let dest = local.path().to_str().unwrap();
        let err = location_transfer_with(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/r/tools"), "tools", true, 0)], dest, TransferMode::Copy, &blocking).await.unwrap_err();
        assert!(err.to_string().contains("Blocked by filename policy"), "{err}");
        assert!(mgr.list_jobs().await.unwrap().is_empty(), "no job was queued");
        assert!(!local.path().join("tools").exists(), "no folder was created");
        // A Google document's export name is checked too (`Plan` → `Plan.docx`).
        let mut doc = item(format!("conn://{id}/r/Plan"), "Plan", false, 3);
        doc.extension = Some("gdoc".into());
        assert!(location_transfer_with(&mgr, resolver.as_ref(), vec![doc], dest, TransferMode::Copy, &blocking).await.is_err());
        assert!(mgr.list_jobs().await.unwrap().is_empty());
        // Without the rule the folder downloads.
        let out = location_transfer_with(&mgr, resolver.as_ref(), vec![item(format!("conn://{id}/r/tools"), "tools", true, 0)], dest, TransferMode::Copy, &PolicySet::default()).await.unwrap();
        assert_eq!(out.jobs.len(), 2);
    }

    /// A Google Drive connection whose root is a shared drive: the pane is at
    /// `/`, but `/` lists its items as `/shared-drives/<id>/<item>`.
    struct SharedDriveRoot(Arc<MemConnector>);

    type Fut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AppError>> + Send + 'a>>;

    impl crate::connectors::Connector for SharedDriveRoot {
        fn connect(&self, _: &crate::core::types::ConnectionProfile) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn list_remote(&self, path: &str) -> Fut<'_, Vec<crate::core::types::FileEntry>> {
            self.0.list_remote(if path == "/" { "/shared-drives/D" } else { path })
        }
        fn rename_entry(&self, from: &str, to: &str) -> Fut<'_, ()> {
            self.0.rename_entry(from, to)
        }
    }

    /// Regression: in a Drive whose root is a shared drive (or Shared with
    /// me), an item's listed parent `/shared-drives/D` is not spelled like
    /// the pane's `/`, so dropping or cut+pasting it into the folder it is in
    /// renamed it to `a (copy).txt` on the server. An item the destination
    /// already lists is left alone.
    #[tokio::test]
    async fn moving_an_item_into_the_folder_that_lists_it_is_a_no_op() {
        let id = Uuid::new_v4();
        let mem = Arc::new(MemConnector::with(&[("/shared-drives/D/a.txt", "a")], &["/shared-drives/D", "/shared-drives/D/f"]));
        let resolver = Arc::new(DynResolver(HashMap::from([(id, Arc::new(SharedDriveRoot(mem.clone())) as Arc<dyn crate::connectors::Connector>)])));
        let mgr = TransferManager::new();
        let items = vec![item(format!("conn://{id}/shared-drives/D/a.txt"), "a.txt", false, 1), item(format!("conn://{id}/shared-drives/D/f"), "f", true, 0)];
        let out = location_transfer(&mgr, resolver.as_ref(), items, &format!("conn://{id}/"), TransferMode::Move).await.unwrap();
        assert!(out.jobs.is_empty() && out.moved.is_empty(), "{out:?}");
        assert!(mem.renames.lock().unwrap().is_empty(), "the server was never asked to rename");
        assert_eq!(mem.file("/shared-drives/D/a.txt").as_deref(), Some("a"));
    }

    /// Moving a Google document out of its connection exports it (a lossy
    /// conversion): the copy is made, the original is kept and reported,
    /// while the other files of the move are moved.
    #[tokio::test]
    async fn moving_a_document_out_of_its_connection_keeps_the_original() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let ca = Arc::new(MemConnector::with(&[("/r/Plan", "doc"), ("/r/n.txt", "n")], &["/r"]));
        ca.native.lock().unwrap().insert("/r/Plan".into());
        let cb = Arc::new(MemConnector::with(&[], &["/in"]));
        let resolver = Arc::new(MemResolver(HashMap::from([(a, ca.clone()), (b, cb.clone())])));
        let mgr = TransferManager::new();
        mgr.set_remote_resolver(resolver.clone());
        let mut doc = item(format!("conn://{a}/r/Plan"), "Plan", false, 3);
        doc.extension = Some("gdoc".into());
        let out = location_transfer(&mgr, resolver.as_ref(), vec![doc, item(format!("conn://{a}/r/n.txt"), "n.txt", false, 1)], &format!("conn://{b}/in"), TransferMode::Move).await.unwrap();
        assert_eq!(out.kept, vec![format!("conn://{a}/r/Plan")]);
        settle(&mgr, &out.jobs).await;
        for _ in 0..200 {
            if ca.file("/r/n.txt").is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(ca.file("/r/n.txt").is_none(), "an ordinary file is moved");
        assert_eq!(cb.file("/in/Plan.docx").as_deref(), Some("doc"), "the export has the format's extension");
        assert_eq!(ca.file("/r/Plan").as_deref(), Some("doc"), "the document itself is kept");
        assert!(ca.dirs.lock().unwrap().contains("/r"));
    }
}
