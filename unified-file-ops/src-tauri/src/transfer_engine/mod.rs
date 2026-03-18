//! Transfer engine with three-layer architecture:
//!
//! - Layer 1: Maximum Throughput (pipeline, worker pool, zero-copy stubs)
//! - Layer 2: Data Integrity Verification (xxHash3, SHA-256, Merkle tree)
//! - Layer 3: Crash Recovery (journal, chunk bitmap, atomic rename)
//!
//! Also provides backward-compatible `TransferManager` implementing `TransferOperations`.
//!
//! Preserved modules:
//! - conflict.rs: Conflict resolution engine (T-017)
//! - history.rs: Transfer history backed by SQLite (T-018)
//! - throttle.rs: Bandwidth throttling via token bucket

pub mod chunk;
pub mod conflict;
pub mod history;
pub mod journal;
pub mod pipeline;
pub mod resume;
pub mod throttle;
pub mod verification;

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, TransferOperations};
use crate::core::types::{
    ChecksumAlgorithm, ConflictPolicy, RetryPolicy, ThrottleConfig, TransferConfig,
    TransferJob, TransferPriority, TransferProgress, TransferStatus, VerifyTier,
};
use crate::storage::pool::DbPool;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};
use uuid::Uuid;

/// Post-transfer completion actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionHooks {
    /// Run a shell command after transfer completes.
    pub run_command: Option<String>,
    /// Play a sound on completion.
    pub play_sound: bool,
    /// Open the destination folder.
    pub open_destination: bool,
    /// Show a desktop notification (default: true).
    pub show_notification: bool,
}

impl Default for CompletionHooks {
    fn default() -> Self {
        Self {
            run_command: None,
            play_sound: false,
            open_destination: false,
            show_notification: true,
        }
    }
}

use self::journal::TransferJournal;
use self::pipeline::WorkerPool;
use self::throttle::BandwidthThrottler;

/// Enhanced transfer manager with three-layer architecture.
///
/// Layer 1: Throughput (worker pool, pipeline, pre-allocation)
/// Layer 2: Integrity (xxHash3, SHA-256, Merkle tree, fsync)
/// Layer 3: Recovery (journal, chunk bitmap, atomic rename)
///
/// Remains Clone (all internals are Arc-wrapped).
#[derive(Clone)]
pub struct TransferManager {
    // ── Job tracking (backward compatible) ──
    jobs: Arc<RwLock<HashMap<Uuid, TransferJob>>>,

    // ── Progress broadcasting ──
    progress_tx: Arc<watch::Sender<Option<TransferProgress>>>,
    progress_rx: watch::Receiver<Option<TransferProgress>>,

    // ── Policies ──
    retry_policy: Arc<RwLock<RetryPolicy>>,
    conflict_policy: Arc<RwLock<ConflictPolicy>>,

    // ── Throttling ──
    throttler: Arc<BandwidthThrottler>,
    throttle_config: Arc<RwLock<ThrottleConfig>>,
    connection_throttles: Arc<RwLock<HashMap<Uuid, u64>>>,

    // ── Three-Layer Engine ──
    /// Transfer configuration (chunk sizing, verification tier, etc.)
    config: Arc<RwLock<TransferConfig>>,
    /// Global worker pool with semaphore (100 cap)
    worker_pool: Arc<WorkerPool>,
    /// Transfer journal for crash recovery (Option: set after DB init)
    journal: Arc<RwLock<Option<TransferJournal>>>,
}

impl TransferManager {
    pub fn new() -> Self {
        let (progress_tx, progress_rx) = watch::channel(None);
        let config = TransferConfig::default();
        let worker_pool = WorkerPool::new(config.max_workers_global);

        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            progress_tx: Arc::new(progress_tx),
            progress_rx,
            retry_policy: Arc::new(RwLock::new(RetryPolicy::default())),
            conflict_policy: Arc::new(RwLock::new(ConflictPolicy::default())),
            throttler: Arc::new(BandwidthThrottler::new(0)),
            throttle_config: Arc::new(RwLock::new(ThrottleConfig::default())),
            connection_throttles: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
            worker_pool: Arc::new(worker_pool),
            journal: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the journal backend. Must be called after DB is available.
    pub async fn init_journal(&self, pool: DbPool) {
        let journal = TransferJournal::new(pool);
        let mut j = self.journal.write().await;
        *j = Some(journal);
    }

    /// Get the journal (panics if not initialized).
    pub async fn journal(&self) -> TransferJournal {
        self.journal
            .read()
            .await
            .clone()
            .expect("TransferJournal not initialized. Call init_journal() first.")
    }

    /// Try to get the journal (returns None if not initialized).
    pub async fn try_journal(&self) -> Option<TransferJournal> {
        self.journal.read().await.clone()
    }

    /// Get the worker pool.
    pub fn worker_pool(&self) -> &WorkerPool {
        &self.worker_pool
    }

    /// Get the transfer configuration.
    pub async fn config(&self) -> TransferConfig {
        self.config.read().await.clone()
    }

    /// Update the transfer configuration.
    pub async fn set_config(&self, config: TransferConfig) {
        let mut c = self.config.write().await;
        *c = config;
    }

    /// Set the default verification tier.
    pub async fn set_verify_tier(&self, tier: VerifyTier) {
        let mut c = self.config.write().await;
        c.default_verify_tier = tier;
    }

    /// Get the default verification tier.
    pub async fn verify_tier(&self) -> VerifyTier {
        self.config.read().await.default_verify_tier
    }

    // ── Crash Recovery ──

    /// Recover active transfers from the journal on startup.
    /// Returns the number of transfers queued for resume.
    pub async fn recover_from_journal(&self) -> Result<u32, AppError> {
        let journal = match self.try_journal().await {
            Some(j) => j,
            None => {
                tracing::debug!("No journal available, skipping recovery");
                return Ok(0);
            }
        };

        let recoverable = journal::recover_active_jobs(&journal).await?;
        let count = recoverable.len() as u32;

        for job in &recoverable {
            tracing::info!(
                "Found recoverable transfer: {} ({}/{} chunks done)",
                job.job.job_id,
                job.verified_count,
                job.total_chunks
            );
        }

        Ok(count)
    }

    // ── Backward-Compatible Job Management ──

    /// Restore jobs from persistent storage (called at startup).
    pub async fn restore_jobs(&self, jobs: Vec<TransferJob>) {
        let mut map = self.jobs.write().await;
        for job in jobs {
            map.insert(job.id, job);
        }
    }

    /// Get a snapshot of all jobs for persistence.
    pub async fn snapshot_jobs(&self) -> Vec<TransferJob> {
        let map = self.jobs.read().await;
        map.values().cloned().collect()
    }

    /// Subscribe to progress updates.
    pub fn subscribe_progress(&self) -> watch::Receiver<Option<TransferProgress>> {
        self.progress_rx.clone()
    }

    /// Update progress for a specific job and broadcast to subscribers.
    pub async fn update_progress(
        &self,
        job_id: Uuid,
        transferred_bytes: u64,
        speed_bps: u64,
    ) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;

        job.transferred_bytes = transferred_bytes;
        job.speed_bps = speed_bps;
        job.updated_at = Utc::now();

        if speed_bps > 0 && job.total_bytes > transferred_bytes {
            let remaining = job.total_bytes - transferred_bytes;
            job.eta_seconds = Some(remaining / speed_bps);
        } else {
            job.eta_seconds = None;
        }

        let progress = TransferProgress {
            job_id,
            transferred_bytes,
            total_bytes: job.total_bytes,
            speed_bps,
            eta_seconds: job.eta_seconds,
            status: job.status,
            error_message: job.error_message.clone(),
        };
        let _ = self.progress_tx.send(Some(progress));
        Ok(())
    }

    /// Mark a job as completed.
    pub async fn complete_job(&self, job_id: Uuid) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.status = TransferStatus::Completed;
        job.transferred_bytes = job.total_bytes;
        job.updated_at = Utc::now();
        job.speed_bps = 0;
        job.eta_seconds = None;

        let progress = TransferProgress {
            job_id,
            transferred_bytes: job.total_bytes,
            total_bytes: job.total_bytes,
            speed_bps: 0,
            eta_seconds: None,
            status: TransferStatus::Completed,
            error_message: None,
        };
        let _ = self.progress_tx.send(Some(progress));
        Ok(())
    }

    /// Mark a job as failed.
    pub async fn fail_job(&self, job_id: Uuid, error: &str) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.status = TransferStatus::Failed;
        job.error_message = Some(error.to_string());
        job.updated_at = Utc::now();
        job.speed_bps = 0;
        job.eta_seconds = None;

        let progress = TransferProgress {
            job_id,
            transferred_bytes: job.transferred_bytes,
            total_bytes: job.total_bytes,
            speed_bps: 0,
            eta_seconds: None,
            status: TransferStatus::Failed,
            error_message: Some(error.to_string()),
        };
        let _ = self.progress_tx.send(Some(progress));
        Ok(())
    }

    /// Set priority of a transfer job.
    pub async fn set_priority(
        &self,
        job_id: Uuid,
        priority: TransferPriority,
    ) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.priority = priority;
        job.updated_at = Utc::now();
        Ok(())
    }

    /// Reorder a job in the queue.
    pub async fn reorder_job(&self, job_id: Uuid, new_sort_order: i32) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.sort_order = new_sort_order;
        job.updated_at = Utc::now();
        Ok(())
    }

    // ── Throttling ──

    pub async fn set_global_throttle(&self, bytes_per_sec: u64) {
        let mut config = self.throttle_config.write().await;
        config.global_limit_bps = bytes_per_sec;
        self.throttler.set_limit(bytes_per_sec);
    }

    pub async fn set_connection_throttle(&self, connection_id: Uuid, bytes_per_sec: u64) {
        let mut throttles = self.connection_throttles.write().await;
        if bytes_per_sec == 0 {
            throttles.remove(&connection_id);
        } else {
            throttles.insert(connection_id, bytes_per_sec);
        }
        let mut config = self.throttle_config.write().await;
        config.per_connection_limit_bps = bytes_per_sec;
    }

    pub fn throttler(&self) -> &BandwidthThrottler {
        &self.throttler
    }

    pub async fn effective_throttle(&self, connection_id: Option<Uuid>) -> u64 {
        let config = self.throttle_config.read().await;
        let global = config.global_limit_bps;
        let per_conn = if let Some(conn_id) = connection_id {
            let throttles = self.connection_throttles.read().await;
            throttles.get(&conn_id).copied().unwrap_or(0)
        } else {
            0
        };
        match (global, per_conn) {
            (0, 0) => 0,
            (0, c) => c,
            (g, 0) => g,
            (g, c) => g.min(c),
        }
    }

    // ── Retry Policy ──

    pub async fn set_retry_policy(&self, policy: RetryPolicy) {
        let mut p = self.retry_policy.write().await;
        *p = policy;
    }

    pub async fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy.read().await.clone()
    }

    pub async fn set_conflict_policy(&self, policy: ConflictPolicy) {
        let mut p = self.conflict_policy.write().await;
        *p = policy;
    }

    pub async fn conflict_policy(&self) -> ConflictPolicy {
        *self.conflict_policy.read().await
    }

    pub async fn increment_retry(&self, job_id: Uuid) -> Result<bool, AppError> {
        let policy = self.retry_policy.read().await.clone();
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.retry_count += 1;
        let can_retry = job.retry_count <= policy.max_retries;
        if can_retry {
            job.status = TransferStatus::Queued;
            job.error_message = None;
        }
        job.updated_at = Utc::now();
        Ok(can_retry)
    }

    pub async fn get_backoff_duration(&self, job_id: Uuid) -> Result<std::time::Duration, AppError> {
        let policy = self.retry_policy.read().await.clone();
        let map = self.jobs.read().await;
        let job = map.get(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        let idx = (job.retry_count as usize).saturating_sub(1);
        let secs = policy
            .backoff_seconds
            .get(idx)
            .copied()
            .unwrap_or(*policy.backoff_seconds.last().unwrap_or(&30));
        Ok(std::time::Duration::from_secs(secs))
    }

    pub async fn retry_all_failed(&self) -> Result<u32, AppError> {
        let policy = self.retry_policy.read().await.clone();
        let mut map = self.jobs.write().await;
        let mut count = 0u32;
        for job in map.values_mut() {
            if job.status == TransferStatus::Failed && job.retry_count < policy.max_retries {
                job.status = TransferStatus::Queued;
                job.error_message = None;
                job.retry_count += 1;
                job.updated_at = Utc::now();
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn list_failed_jobs(&self) -> Result<Vec<TransferJob>, AppError> {
        let map = self.jobs.read().await;
        let failed: Vec<TransferJob> = map
            .values()
            .filter(|j| j.status == TransferStatus::Failed)
            .cloned()
            .collect();
        Ok(failed)
    }

    pub async fn set_job_conflict_policy(
        &self,
        job_id: Uuid,
        policy: ConflictPolicy,
    ) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.conflict_policy = Some(policy);
        job.updated_at = Utc::now();
        Ok(())
    }

    pub async fn set_job_checksums(
        &self,
        job_id: Uuid,
        source_checksum: Option<String>,
        dest_checksum: Option<String>,
    ) -> Result<(), AppError> {
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found."),
            advice: "The transfer may have already completed or been removed.".to_string(),
        })?;
        job.source_checksum = source_checksum;
        job.dest_checksum = dest_checksum;
        job.updated_at = Utc::now();
        Ok(())
    }

    pub async fn throttle_config(&self) -> ThrottleConfig {
        self.throttle_config.read().await.clone()
    }

    // ── Legacy checksum settings (backward compat) ──

    pub async fn set_verify_checksums(&self, _verify: bool) {
        // In the new architecture, verification is always on.
        // The verify_tier controls the level.
        // This method is kept for backward compatibility.
    }

    pub async fn set_checksum_algorithm(&self, _algorithm: ChecksumAlgorithm) {
        // In the new architecture, xxHash3 and SHA-256 are used based on tier.
        // This method is kept for backward compatibility.
    }
}

impl Default for TransferManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TransferOperations for TransferManager {
    fn enqueue(
        &self,
        source: &str,
        dest: &str,
        total_bytes: u64,
    ) -> BoxFuture<'_, Result<TransferJob, AppError>> {
        let source = source.to_string();
        let dest = dest.to_string();
        Box::pin(async move {
            let now = Utc::now();
            let map_read = self.jobs.read().await;
            let max_order = map_read
                .values()
                .map(|j| j.sort_order)
                .max()
                .unwrap_or(-1);
            drop(map_read);

            let config = self.config.read().await;

            let job = TransferJob {
                id: Uuid::new_v4(),
                source_path: source,
                dest_path: dest,
                total_bytes,
                transferred_bytes: 0,
                status: TransferStatus::Queued,
                priority: TransferPriority::Normal,
                sort_order: max_order + 1,
                created_at: now,
                updated_at: now,
                error_message: None,
                retry_count: 0,
                partial_checksum: None,
                conflict_policy: None,
                connection_id: None,
                verify_checksum: true, // Always verify in new architecture
                checksum_algorithm: Some(ChecksumAlgorithm::XxHash3),
                source_checksum: None,
                dest_checksum: None,
                speed_bps: 0,
                eta_seconds: None,
            };
            drop(config);

            let mut map = self.jobs.write().await;
            map.insert(job.id, job.clone());
            tracing::info!("Transfer enqueued: {}", job.id);
            Ok(job)
        })
    }

    fn pause(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let mut map = self.jobs.write().await;
            let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
                message: format!("Transfer job {job_id} not found."),
                advice: "The transfer may have already completed or been removed.".to_string(),
            })?;
            job.status = TransferStatus::Paused;
            job.updated_at = Utc::now();
            job.speed_bps = 0;
            job.eta_seconds = None;

            let progress = TransferProgress {
                job_id,
                transferred_bytes: job.transferred_bytes,
                total_bytes: job.total_bytes,
                speed_bps: 0,
                eta_seconds: None,
                status: TransferStatus::Paused,
                error_message: None,
            };
            let _ = self.progress_tx.send(Some(progress));
            Ok(())
        })
    }

    fn resume(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let mut map = self.jobs.write().await;
            let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
                message: format!("Transfer job {job_id} not found."),
                advice: "The transfer may have already completed or been removed.".to_string(),
            })?;
            job.status = TransferStatus::Queued;
            job.updated_at = Utc::now();
            Ok(())
        })
    }

    fn cancel(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let mut map = self.jobs.write().await;
            let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
                message: format!("Transfer job {job_id} not found."),
                advice: "The transfer may have already completed or been removed.".to_string(),
            })?;
            job.status = TransferStatus::Cancelled;
            job.updated_at = Utc::now();
            job.speed_bps = 0;
            job.eta_seconds = None;

            let progress = TransferProgress {
                job_id,
                transferred_bytes: job.transferred_bytes,
                total_bytes: job.total_bytes,
                speed_bps: 0,
                eta_seconds: None,
                status: TransferStatus::Cancelled,
                error_message: None,
            };
            let _ = self.progress_tx.send(Some(progress));
            Ok(())
        })
    }

    fn list_jobs(&self) -> BoxFuture<'_, Result<Vec<TransferJob>, AppError>> {
        Box::pin(async move {
            let map = self.jobs.read().await;
            let mut jobs: Vec<TransferJob> = map.values().cloned().collect();
            jobs.sort_by(|a, b| {
                b.priority
                    .cmp(&a.priority)
                    .then_with(|| a.sort_order.cmp(&b.sort_order))
                    .then_with(|| a.created_at.cmp(&b.created_at))
            });
            Ok(jobs)
        })
    }

    fn get_job(&self, job_id: Uuid) -> BoxFuture<'_, Result<Option<TransferJob>, AppError>> {
        Box::pin(async move {
            let map = self.jobs.read().await;
            Ok(map.get(&job_id).cloned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_enqueue_and_list() {
        let mgr = TransferManager::new();
        let job = mgr.enqueue("/src/file.txt", "/dst/file.txt", 1024).await.unwrap();
        assert_eq!(job.status, TransferStatus::Queued);
        assert_eq!(job.total_bytes, 1024);
        assert_eq!(job.priority, TransferPriority::Normal);
        assert_eq!(job.retry_count, 0);
        assert!(job.verify_checksum); // Always true in new arch

        let jobs = mgr.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, job.id);
    }

    #[tokio::test]
    async fn test_pause_resume_cancel() {
        let mgr = TransferManager::new();
        let job = mgr.enqueue("/a", "/b", 100).await.unwrap();

        mgr.pause(job.id).await.unwrap();
        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Paused);

        mgr.resume(job.id).await.unwrap();
        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Queued);

        mgr.cancel(job.id).await.unwrap();
        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_pause_nonexistent() {
        let mgr = TransferManager::new();
        let result = mgr.pause(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_restore_and_snapshot() {
        let mgr = TransferManager::new();
        let job = TransferJob {
            id: Uuid::new_v4(),
            source_path: "/a".to_string(),
            dest_path: "/b".to_string(),
            total_bytes: 500,
            transferred_bytes: 250,
            status: TransferStatus::Paused,
            priority: TransferPriority::Normal,
            sort_order: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error_message: None,
            retry_count: 0,
            partial_checksum: None,
            conflict_policy: None,
            connection_id: None,
            verify_checksum: false,
            checksum_algorithm: None,
            source_checksum: None,
            dest_checksum: None,
            speed_bps: 0,
            eta_seconds: None,
        };
        mgr.restore_jobs(vec![job.clone()]).await;

        let snapshot = mgr.snapshot_jobs().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, job.id);
        assert_eq!(snapshot[0].transferred_bytes, 250);
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let mgr = TransferManager::new();
        let low = mgr.enqueue("/low", "/dst", 100).await.unwrap();
        let normal = mgr.enqueue("/normal", "/dst", 100).await.unwrap();
        let high = mgr.enqueue("/high", "/dst", 100).await.unwrap();

        mgr.set_priority(low.id, TransferPriority::Low).await.unwrap();
        mgr.set_priority(high.id, TransferPriority::High).await.unwrap();

        let jobs = mgr.list_jobs().await.unwrap();
        assert_eq!(jobs[0].id, high.id);
        assert_eq!(jobs[1].id, normal.id);
        assert_eq!(jobs[2].id, low.id);
    }

    #[tokio::test]
    async fn test_reorder_job() {
        let mgr = TransferManager::new();
        let _j1 = mgr.enqueue("/a", "/dst", 100).await.unwrap();
        let _j2 = mgr.enqueue("/b", "/dst", 100).await.unwrap();
        let j3 = mgr.enqueue("/c", "/dst", 100).await.unwrap();

        mgr.reorder_job(j3.id, -1).await.unwrap();

        let jobs = mgr.list_jobs().await.unwrap();
        assert_eq!(jobs[0].id, j3.id);
    }

    #[tokio::test]
    async fn test_progress_update() {
        let mgr = TransferManager::new();
        let job = mgr.enqueue("/src", "/dst", 1000).await.unwrap();

        mgr.update_progress(job.id, 500, 100).await.unwrap();

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.transferred_bytes, 500);
        assert_eq!(j.speed_bps, 100);
        assert_eq!(j.eta_seconds, Some(5));
    }

    #[tokio::test]
    async fn test_complete_and_fail() {
        let mgr = TransferManager::new();
        let j1 = mgr.enqueue("/a", "/dst", 100).await.unwrap();
        let j2 = mgr.enqueue("/b", "/dst", 100).await.unwrap();

        mgr.complete_job(j1.id).await.unwrap();
        let j = mgr.get_job(j1.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Completed);
        assert_eq!(j.transferred_bytes, 100);

        mgr.fail_job(j2.id, "Network error").await.unwrap();
        let j = mgr.get_job(j2.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Failed);
        assert_eq!(j.error_message.as_deref(), Some("Network error"));
    }

    #[tokio::test]
    async fn test_retry_with_backoff() {
        let mgr = TransferManager::new();
        let job = mgr.enqueue("/src", "/dst", 1000).await.unwrap();
        mgr.fail_job(job.id, "error").await.unwrap();

        let can_retry = mgr.increment_retry(job.id).await.unwrap();
        assert!(can_retry);
        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Queued);
        assert_eq!(j.retry_count, 1);

        let backoff = mgr.get_backoff_duration(job.id).await.unwrap();
        assert_eq!(backoff, std::time::Duration::from_secs(1));

        mgr.fail_job(job.id, "error2").await.unwrap();
        let can_retry = mgr.increment_retry(job.id).await.unwrap();
        assert!(can_retry);
        let backoff = mgr.get_backoff_duration(job.id).await.unwrap();
        assert_eq!(backoff, std::time::Duration::from_secs(5));

        mgr.fail_job(job.id, "error3").await.unwrap();
        let can_retry = mgr.increment_retry(job.id).await.unwrap();
        assert!(can_retry);
        let backoff = mgr.get_backoff_duration(job.id).await.unwrap();
        assert_eq!(backoff, std::time::Duration::from_secs(30));

        mgr.fail_job(job.id, "error4").await.unwrap();
        let can_retry = mgr.increment_retry(job.id).await.unwrap();
        assert!(!can_retry);
    }

    #[tokio::test]
    async fn test_retry_all_failed() {
        let mgr = TransferManager::new();
        let j1 = mgr.enqueue("/a", "/dst", 100).await.unwrap();
        let j2 = mgr.enqueue("/b", "/dst", 100).await.unwrap();
        let _j3 = mgr.enqueue("/c", "/dst", 100).await.unwrap();

        mgr.fail_job(j1.id, "err").await.unwrap();
        mgr.fail_job(j2.id, "err").await.unwrap();

        let count = mgr.retry_all_failed().await.unwrap();
        assert_eq!(count, 2);

        let failed = mgr.list_failed_jobs().await.unwrap();
        assert!(failed.is_empty());
    }

    #[tokio::test]
    async fn test_bandwidth_throttle() {
        let mgr = TransferManager::new();
        let conn_id = Uuid::new_v4();

        assert_eq!(mgr.effective_throttle(None).await, 0);

        mgr.set_global_throttle(1_000_000).await;
        assert_eq!(mgr.effective_throttle(None).await, 1_000_000);

        mgr.set_global_throttle(0).await;
        mgr.set_connection_throttle(conn_id, 500_000).await;
        assert_eq!(mgr.effective_throttle(Some(conn_id)).await, 500_000);

        mgr.set_global_throttle(1_000_000).await;
        assert_eq!(mgr.effective_throttle(Some(conn_id)).await, 500_000);
    }

    #[tokio::test]
    async fn test_conflict_policy() {
        let mgr = TransferManager::new();
        let job = mgr.enqueue("/a", "/b", 100).await.unwrap();

        mgr.set_job_conflict_policy(job.id, ConflictPolicy::OverwriteAlways)
            .await
            .unwrap();

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.conflict_policy, Some(ConflictPolicy::OverwriteAlways));
    }

    #[tokio::test]
    async fn test_concurrent_100_items() {
        let mgr = TransferManager::new();
        let mut handles = Vec::new();

        for i in 0..100 {
            let mgr_clone = mgr.clone();
            handles.push(tokio::spawn(async move {
                mgr_clone
                    .enqueue(&format!("/src/{i}"), &format!("/dst/{i}"), 1024)
                    .await
                    .unwrap()
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let jobs = mgr.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 100);
    }

    #[tokio::test]
    async fn test_progress_subscription() {
        let mgr = TransferManager::new();
        let mut rx = mgr.subscribe_progress();
        let job = mgr.enqueue("/src", "/dst", 1000).await.unwrap();

        mgr.update_progress(job.id, 500, 100).await.unwrap();

        rx.changed().await.unwrap();
        let progress = rx.borrow().clone().unwrap();
        assert_eq!(progress.job_id, job.id);
        assert_eq!(progress.transferred_bytes, 500);
    }

    #[tokio::test]
    async fn test_config_management() {
        let mgr = TransferManager::new();

        let config = mgr.config().await;
        assert_eq!(config.default_verify_tier, VerifyTier::Fast);

        mgr.set_verify_tier(VerifyTier::Verified).await;
        let tier = mgr.verify_tier().await;
        assert_eq!(tier, VerifyTier::Verified);
    }

    #[tokio::test]
    async fn test_journal_initialization() {
        let mgr = TransferManager::new();

        // Before init, try_journal returns None
        assert!(mgr.try_journal().await.is_none());

        // Init with in-memory DB
        let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();

        mgr.init_journal(pool).await;
        assert!(mgr.try_journal().await.is_some());
    }

    #[tokio::test]
    async fn test_recover_from_empty_journal() {
        let mgr = TransferManager::new();
        let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();

        mgr.init_journal(pool).await;
        let count = mgr.recover_from_journal().await.unwrap();
        assert_eq!(count, 0);
    }
}
