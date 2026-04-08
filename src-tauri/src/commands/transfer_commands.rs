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
    let job = manager.enqueue(&source, &dest, total_bytes).await?;
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
    let count = manager.retry_all_failed().await?;
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
    history: State<'_, TransferHistory>,
) -> Result<u32, AppError> {
    history.cleanup_old_records().await
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
