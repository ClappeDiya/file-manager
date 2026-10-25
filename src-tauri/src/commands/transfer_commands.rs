use crate::core::error::AppError;
use crate::core::traits::TransferOperations;
use crate::core::types::{
    ChecksumAlgorithm, ConflictPolicy, ExportFormat, RetryPolicy, ThrottleConfig,
    TransferConfig, TransferHistoryFilter, TransferHistoryRecord, TransferJob,
    TransferPriority, TransferStatus, VerifyTier,
};
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::transfer_engine::conflict::{self, ConflictAction, ConflictCheck};
use crate::storage::Repository;
use crate::transfer_engine::history::TransferHistory;
use crate::transfer_engine::retention::{self, FailedRetention};
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

/// Config-table key of the bandwidth timetable.
pub const BANDWIDTH_SCHEDULE_KEY: &str = "bandwidth.schedule";

/// The bandwidth timetable: windows (weekdays, start, end) with upload and
/// download limits. While one is active, its upload limit is shared by
/// backups, sync copies and file transfers, and both limits replace the
/// backup machine-wide caps.
#[tauri::command]
pub async fn get_bandwidth_schedule() -> Result<Vec<ufop_core::throttle::BandwidthWindow>, AppError> {
    Ok(ufop_core::throttle::schedule())
}

/// Validate, save and apply a new bandwidth timetable.
#[tauri::command]
pub async fn set_bandwidth_schedule(
    windows: Vec<ufop_core::throttle::BandwidthWindow>,
    repo: State<'_, crate::storage::Repository>,
) -> Result<Vec<ufop_core::throttle::BandwidthWindow>, AppError> {
    ufop_core::throttle::validate_schedule(&windows)?;
    repo.set_config(BANDWIDTH_SCHEDULE_KEY, &serde_json::to_string(&windows)?).await?;
    ufop_core::throttle::set_schedule(windows.clone())?;
    Ok(windows)
}

/// Apply the saved bandwidth timetable (startup).
pub async fn load_bandwidth_schedule(repo: &crate::storage::Repository) -> Result<usize, AppError> {
    let Some(json) = repo.get_config(BANDWIDTH_SCHEDULE_KEY).await? else { return Ok(0) };
    let windows: Vec<ufop_core::throttle::BandwidthWindow> = serde_json::from_str(&json)?;
    let n = windows.len();
    ufop_core::throttle::set_schedule(windows)?;
    Ok(n)
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

/// Search transfer history. `date_from` / `date_to` (RFC 3339 instants)
/// keep the transfers that started in that range.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn search_transfer_history(
    path_contains: Option<String>,
    status: Option<String>,
    connection_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    date_from: Option<String>,
    date_to: Option<String>,
    history: State<'_, TransferHistory>,
) -> Result<Vec<TransferHistoryRecord>, AppError> {
    let (date_from, date_to) = crate::core::date_range::parse_range(date_from.as_deref(), date_to.as_deref())?;
    let filter = TransferHistoryFilter {
        path_contains,
        status: status.map(|s| TransferStatus::from_str_lossy(&s)),
        connection_id: connection_id.and_then(|s| Uuid::parse_str(&s).ok()),
        date_from,
        date_to,
        limit,
        offset,
    };
    history.search(filter).await
}

/// Export transfer history (with the same filters as the search).
#[tauri::command]
pub async fn export_transfer_history(
    format: String,
    path_contains: Option<String>,
    status: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    history: State<'_, TransferHistory>,
) -> Result<String, AppError> {
    let fmt = match format.as_str() {
        "csv" => ExportFormat::Csv,
        "json" => ExportFormat::Json,
        _ => ExportFormat::Json,
    };
    let (date_from, date_to) = crate::core::date_range::parse_range(date_from.as_deref(), date_to.as_deref())?;
    let filter = TransferHistoryFilter {
        path_contains,
        status: status.map(|s| TransferStatus::from_str_lossy(&s)),
        date_from,
        date_to,
        ..Default::default()
    };
    history.export(filter, fmt).await
}

/// Clean up old transfer history records, and the failed transfers kept for
/// retry that are past the retention limits (age and total size). Returns
/// how many records were removed in all.
#[tauri::command]
pub async fn cleanup_transfer_history(
    older_than_days: Option<u32>,
    history: State<'_, TransferHistory>,
    manager: State<'_, TransferManager>,
    repo: State<'_, Repository>,
) -> Result<u32, AppError> {
    let history_removed = match older_than_days {
        // The user's "older than N days" choice; without one, the default retention.
        Some(days) => history.cleanup_older_than(days.max(1)).await?,
        None => history.cleanup_old_records().await?,
    };
    let retry_removed = prune_failed_transfers_now(&manager, &repo).await?;
    Ok(history_removed + retry_removed)
}

/// Drop the stored failed transfers beyond the saved retention limits, and
/// from memory too. Returns how many were dropped.
async fn prune_failed_transfers_now(manager: &TransferManager, repo: &Repository) -> Result<u32, AppError> {
    let limits = FailedRetention::load(repo).await;
    let removed = retention::prune_failed(repo, limits).await?;
    manager.forget_failed(&removed).await;
    Ok(removed.len() as u32)
}

/// How long, and how much, failed transfers are kept for retry.
#[tauri::command]
pub async fn get_failed_transfer_retention(repo: State<'_, Repository>) -> Result<FailedRetention, AppError> {
    Ok(FailedRetention::load(&repo).await)
}

/// Change the retention of failed transfers (values are clamped to sane
/// bounds) and apply it now. Returns the limits in force.
#[tauri::command]
pub async fn set_failed_transfer_retention(
    max_age_days: u32,
    max_total_kb: u32,
    manager: State<'_, TransferManager>,
    repo: State<'_, Repository>,
) -> Result<FailedRetention, AppError> {
    let saved = FailedRetention { max_age_days, max_total_kb }.save(&repo).await?;
    prune_failed_transfers_now(&manager, &repo).await?;
    Ok(saved)
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
mod bandwidth_schedule_tests {
    use super::*;
    use chrono::Datelike;

    #[tokio::test]
    async fn timetable_is_saved_and_restored() {
        // Changes the process-wide timetable.
        let _rates = crate::transfer_engine::throttle::global_rates_test_lock().await;
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        assert_eq!(load_bandwidth_schedule(&repo).await.unwrap(), 0);
        // A day that is not today, so the shared limiter never applies it
        // while other tests run.
        let other_day = ((chrono::Local::now().weekday().num_days_from_monday() + 3) % 7) as u8;
        let windows = vec![ufop_core::throttle::BandwidthWindow {
            days: vec![other_day],
            start: "09:00".into(),
            end: "17:00".into(),
            upload_bps: 512 * 1024,
            download_bps: 0,
        }];
        ufop_core::throttle::validate_schedule(&windows).unwrap();
        repo.set_config(BANDWIDTH_SCHEDULE_KEY, &serde_json::to_string(&windows).unwrap()).await.unwrap();
        assert_eq!(load_bandwidth_schedule(&repo).await.unwrap(), 1);
        assert_eq!(get_bandwidth_schedule().await.unwrap(), windows);
        ufop_core::throttle::set_schedule(Vec::new()).unwrap();
        // Invalid windows are refused before anything is stored.
        let bad = vec![ufop_core::throttle::BandwidthWindow { days: vec![9], start: "09:00".into(), end: "10:00".into(), ..Default::default() }];
        assert!(ufop_core::throttle::validate_schedule(&bad).is_err());
    }
}
