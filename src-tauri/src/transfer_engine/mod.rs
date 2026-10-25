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
pub mod retention;
pub mod throttle;
pub mod verification;

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, TransferOperations};
use crate::core::types::{
    ChecksumAlgorithm, ConflictPolicy, RetryPolicy, ThrottleConfig, TransferConfig,
    TransferJob, TransferPriority, TransferProgress, TransferStatus, VerifyTier,
};
use crate::storage::pool::DbPool;
use crate::transfer_engine::history::TransferHistory;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{watch, Mutex, Notify, RwLock};
use uuid::Uuid;

/// Cooperative pause/cancel signal for a running transfer job.
///
/// Workers check it at chunk boundaries; `Notify` wakes them on resume or
/// cancel. Created once per job (lazily via [`TransferManager::control`]) so
/// signalling a job that has not started yet still lands.
#[derive(Debug, Default)]
pub struct JobControl {
    cancelled: AtomicBool,
    paused: AtomicBool,
    notify: Notify,
}

impl JobControl {
    /// Cancel the transfer. Also clears pause so parked workers wake up.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Hold the transfer at the next chunk boundary.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resume a paused transfer.
    pub fn unpause(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Clear both flags — required when a job is re-queued (retry), so flags
    /// from its previous run don't abort the new one.
    pub fn reset(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.cancelled.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Park until unpaused. Callers must check `is_cancelled` after this
    /// returns — cancel wakes paused workers too.
    pub async fn wait_if_paused(&self) {
        loop {
            if !self.paused.load(Ordering::SeqCst) {
                return;
            }
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Register the waiter before re-checking the flag so a pause→resume
            // flip between check and await can't be missed.
            notified.as_mut().enable();
            if !self.paused.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

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

    /// Cooperative pause/cancel signals, one per job.
    controls: Arc<RwLock<HashMap<Uuid, Arc<JobControl>>>>,

    /// Jobs with a live executor task — guards against double-runs when a
    /// mid-flight-paused job is resumed (which re-spawns a runner).
    executors: Arc<Mutex<HashSet<Uuid>>>,

    /// Transfer history sink (Option: set after DB init, same as the journal).
    /// Every job that reaches a terminal status gets recorded so the
    /// Transfer History panel reflects real executions — without this the
    /// panel could only ever render "No transfer history".
    history: Arc<RwLock<Option<TransferHistory>>>,

    /// Called for every job that completes or fails (desktop notifications);
    /// added at start-up.
    terminal_hooks: Arc<std::sync::RwLock<Vec<TerminalHook>>>,
}

/// Listener for jobs reaching a final Completed or Failed status.
pub type TerminalHook = Arc<dyn Fn(&crate::core::types::TransferJob) + Send + Sync>;

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
            throttler: Arc::new(BandwidthThrottler::global()),
            throttle_config: Arc::new(RwLock::new(ThrottleConfig::default())),
            connection_throttles: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
            worker_pool: Arc::new(worker_pool),
            journal: Arc::new(RwLock::new(None)),
            controls: Arc::new(RwLock::new(HashMap::new())),
            executors: Arc::new(Mutex::new(HashSet::new())),
            history: Arc::new(RwLock::new(None)),
            terminal_hooks: Arc::new(std::sync::RwLock::new(Vec::new())),
        }
    }

    /// Listen for jobs that complete or fail.
    pub fn add_terminal_hook(&self, hook: TerminalHook) {
        self.terminal_hooks.write().unwrap_or_else(|e| e.into_inner()).push(hook);
    }

    /// Attach the history sink. Called once from app init, right after the
    /// journal — the DB pool exists by then in both init paths.
    pub async fn set_history(&self, history: TransferHistory) {
        *self.history.write().await = Some(history);
    }

    /// Get the pause/cancel control for a job, creating it if needed.
    pub async fn control(&self, job_id: Uuid) -> Arc<JobControl> {
        self.controls
            .write()
            .await
            .entry(job_id)
            .or_insert_with(|| Arc::new(JobControl::default()))
            .clone()
    }

    /// Spawn the executor task for a queued job unless one is already alive.
    /// Called by the enqueue command and by resume (a paused-before-start job
    /// has no executor left, so resume must start one).
    pub async fn spawn_runner(&self, job_id: Uuid) {
        {
            let mut set = self.executors.lock().await;
            if !set.insert(job_id) {
                return;
            }
        }
        let runner = self.clone();
        tokio::spawn(async move {
            runner.run_local_job(job_id).await;
            runner.executors.lock().await.remove(&job_id);
        });
    }

    /// Execute a queued local-file job through the three-layer pipeline
    /// (chunked copy, checksum verification, journal) and record the outcome.
    ///
    /// Remote endpoints are not handled here; they go through the connector
    /// commands. Existing destinations are never overwritten: such jobs fail
    /// with an explanation instead.
    pub async fn run_local_job(&self, job_id: Uuid) {
        let started_at = Utc::now();
        let started = std::time::Instant::now();
        if let Err(message) = self.run_local_job_inner(job_id).await {
            // A cancelled job already has its final status — don't overwrite it.
            if !self.control(job_id).await.is_cancelled() {
                let _ = self.fail_job(job_id, &message).await;
            }
        }
        self.record_history(job_id, started_at, started.elapsed()).await;
    }

    /// Write a terminal-state job to transfer history. Completed, failed and
    /// cancelled executions all belong in the audit trail; anything still
    /// mid-flight (queued/active/paused — possible when a runner exits early
    /// on a job that was never really started) is skipped.
    async fn record_history(
        &self,
        job_id: Uuid,
        started_at: chrono::DateTime<Utc>,
        elapsed: std::time::Duration,
    ) {
        let Some(history) = self.history.read().await.clone() else {
            return;
        };
        let Some(job) = self.jobs.read().await.get(&job_id).cloned() else {
            return;
        };
        if !matches!(
            job.status,
            TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
        ) {
            return;
        }
        let checksum_match = match (&job.source_checksum, &job.dest_checksum) {
            (Some(src), Some(dst)) => Some(src == dst),
            _ => None,
        };
        let _ = history
            .record_transfer(
                &job,
                started_at,
                Some(elapsed.as_millis() as u64),
                job.verify_checksum,
                checksum_match,
                None,
            )
            .await;
    }

    async fn run_local_job_inner(&self, job_id: Uuid) -> Result<(), String> {
        let control = self.control(job_id).await;
        let (source, dest) = {
            let mut map = self.jobs.write().await;
            let job = map.get_mut(&job_id).ok_or_else(|| "Transfer job not found".to_string())?;
            if job.status != TransferStatus::Queued {
                return Ok(());
            }
            job.status = TransferStatus::Active;
            job.updated_at = Utc::now();
            (job.source_path.clone(), job.dest_path.clone())
        };
        if control.is_cancelled() {
            return Ok(());
        }

        let meta = tokio::fs::metadata(&source)
            .await
            .map_err(|e| format!("Cannot read source {source}: {e}. For remote servers use the Connections panel."))?;
        if !meta.is_file() {
            return Err(format!("{source} is a folder; the queue transfers single files."));
        }
        // The queue runner is strictly local→local. A remote-looking
        // destination (sftp://, webdav://, …) used to fall through to
        // create_dir_all and would even materialise a literal "sftp:"
        // directory on disk. Remote transfers go through the connector
        // commands, so reject these up front.
        if dest.contains("://") {
            return Err(format!(
                "{dest} is a remote destination; the queue only transfers to local paths. Use the Connections panel."
            ));
        }
        let size = meta.len();

        // A destination that is (or ends like) a directory receives the source's file name.
        let dest_is_dir = dest.ends_with('/') || tokio::fs::metadata(&dest).await.map(|m| m.is_dir()).unwrap_or(false);
        let dest_file = if dest_is_dir {
            let name = std::path::Path::new(&source)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .ok_or_else(|| "Source has no file name".to_string())?;
            format!("{}/{}", dest.trim_end_matches('/'), name)
        } else {
            dest.clone()
        };
        if tokio::fs::metadata(&dest_file).await.is_ok() {
            return Err(format!("{dest_file} already exists; it was not overwritten."));
        }
        if let Some(parent) = std::path::Path::new(&dest_file).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Cannot create destination folder: {e}"))?;
        }

        {
            let mut map = self.jobs.write().await;
            if let Some(job) = map.get_mut(&job_id) {
                job.total_bytes = size;
                job.dest_path = dest_file.clone();
            }
        }

        let journal = self
            .try_journal()
            .await
            .ok_or_else(|| "Transfer journal is not ready yet; try again in a moment.".to_string())?;
        let config = self.config().await;
        let tier = config.default_verify_tier;
        let result = pipeline::execute_file_transfer(
            &source,
            &dest_file,
            size,
            tier,
            &config,
            &journal,
            self.worker_pool(),
            &job_id.to_string(),
            None,
            Some(control),
        )
        .await
        .map_err(|e| e.to_string())?;

        let _ = self.update_progress(job_id, result.file_size, result.speed_bps).await;
        // Persist the pipeline's verified hashes on the job so history can
        // report whether source and destination actually matched.
        let _ = self
            .set_job_checksums(
                job_id,
                Some(result.source_hash),
                Some(result.dest_hash),
            )
            .await;
        self.complete_job(job_id).await.map_err(|e| e.to_string())
    }

    /// Show work another engine runs — a connector upload or download for a
    /// folder comparison — as an active job in the queue, so it has the same
    /// progress row and history entry as engine copies. The queue runner
    /// never executes it (remote endpoints are labelled `protocol://…`, which
    /// it refuses); the caller ends it with [`TransferManager::finish_external`].
    pub async fn track_external(&self, source: &str, dest: &str, total_bytes: u64, connection_id: Option<Uuid>) -> Result<TransferJob, AppError> {
        let job = self.enqueue(source, dest, total_bytes).await?;
        let mut map = self.jobs.write().await;
        let job = map.get_mut(&job.id).ok_or_else(|| AppError::internal("Tracked transfer vanished"))?;
        job.status = TransferStatus::Active;
        job.external = true;
        job.connection_id = connection_id;
        // Connectors verify their own uploads; the engine computed no checksums.
        job.verify_checksum = false;
        job.updated_at = Utc::now();
        Ok(job.clone())
    }

    /// End a job started with [`TransferManager::track_external`]: completed
    /// with the bytes moved, or failed with the error (a job cancelled from
    /// the queue stays cancelled). Recorded in history.
    pub async fn finish_external(&self, job_id: Uuid, outcome: Result<u64, String>, started_at: chrono::DateTime<Utc>, elapsed: std::time::Duration) {
        let cancelled = self.jobs.read().await.get(&job_id).is_some_and(|j| j.status == TransferStatus::Cancelled);
        let _ = match outcome {
            _ if cancelled => Ok(()),
            Ok(bytes) => {
                if let Some(job) = self.jobs.write().await.get_mut(&job_id) {
                    job.total_bytes = bytes.max(job.total_bytes);
                }
                self.complete_job(job_id).await
            }
            Err(e) => self.fail_job(job_id, &e).await,
        };
        self.record_history(job_id, started_at, elapsed).await;
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

    /// Drop failed jobs from memory (their stored rows were pruned).
    /// Anything that is not failed any more is left alone.
    pub async fn forget_failed(&self, ids: &[Uuid]) {
        let mut map = self.jobs.write().await;
        for id in ids {
            if map.get(id).is_some_and(|j| j.status == TransferStatus::Failed) {
                map.remove(id);
            }
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
        let done = job.clone();
        drop(map);
        self.announce(&done);
        Ok(())
    }

    /// Tell the terminal hooks (desktop notifications) a job ended.
    fn announce(&self, job: &TransferJob) {
        for hook in self.terminal_hooks.read().unwrap_or_else(|e| e.into_inner()).iter() {
            hook(job);
        }
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
        let done = job.clone();
        drop(map);
        self.announce(&done);
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
        if job.external {
            return Err(external_only("retried"));
        }
        job.retry_count += 1;
        let can_retry = job.retry_count <= policy.max_retries;
        if can_retry {
            job.status = TransferStatus::Queued;
            job.error_message = None;
        }
        job.updated_at = Utc::now();
        drop(map);
        if can_retry {
            // A cancelled/failed job's control may still carry stale flags.
            self.control(job_id).await.reset();
        }
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

    /// Re-queue every failed job that still has retries left. Returns the
    /// re-queued job ids so the caller can spawn runners for them.
    pub async fn retry_all_failed(&self) -> Result<Vec<Uuid>, AppError> {
        let policy = self.retry_policy.read().await.clone();
        let mut map = self.jobs.write().await;
        let mut requeued = Vec::new();
        for job in map.values_mut() {
            if job.status == TransferStatus::Failed && !job.external && job.retry_count < policy.max_retries {
                job.status = TransferStatus::Queued;
                job.error_message = None;
                job.retry_count += 1;
                job.updated_at = Utc::now();
                requeued.push(job.id);
            }
        }
        drop(map);
        for id in &requeued {
            self.control(*id).await.reset();
        }
        Ok(requeued)
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
                external: false,
            };
            drop(config);

            let mut map = self.jobs.write().await;
            map.insert(job.id, job.clone());
            drop(map);
            // Create its control eagerly so pause/cancel before the executor
            // starts still land.
            let _ = self.control(job.id).await;
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
            if job.external {
                return Err(external_only("paused"));
            }
            // Pausing a finished job must not corrupt its status.
            if !matches!(
                job.status,
                TransferStatus::Queued | TransferStatus::Active
            ) {
                return Ok(());
            }
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
            drop(map);
            let _ = self.progress_tx.send(Some(progress));
            self.control(job_id).await.pause();
            Ok(())
        })
    }

    fn resume(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            {
                let mut map = self.jobs.write().await;
                let job = map.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
                    message: format!("Transfer job {job_id} not found."),
                    advice: "The transfer may have already completed or been removed.".to_string(),
                })?;
                if job.status != TransferStatus::Paused {
                    return Ok(());
                }
                job.status = TransferStatus::Queued;
                job.updated_at = Utc::now();
            }
            self.control(job_id).await.unpause();
            // A job paused before its executor started has no live runner, so
            // resume must spawn one. Mid-flight pauses keep theirs; the
            // executor-slot guard makes this a no-op then.
            self.spawn_runner(job_id).await;
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
            // Cancelling a finished job must not corrupt its status.
            if matches!(
                job.status,
                TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
            ) {
                return Ok(());
            }
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
            drop(map);
            let _ = self.progress_tx.send(Some(progress));
            // Signal the running executor: parked workers wake, in-flight
            // chunks stop at the next boundary.
            self.control(job_id).await.cancel();
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

/// Why the queue refuses to pause or retry a job another engine runs.
fn external_only(what: &str) -> AppError {
    AppError::Transfer {
        message: format!("This transfer runs through a connection and cannot be {what} from the queue."),
        advice: "Cancel it here, or run the action again from where it started (for example the compare window).".to_string(),
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
        // Resume now actually re-runs the job: it must have left Paused, and
        // may already be Active or Failed ("/a" does not exist).
        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert!(matches!(
            j.status,
            TransferStatus::Queued | TransferStatus::Active | TransferStatus::Failed
        ));

        // Cancel a job with no live executor — deterministic end state.
        let job2 = mgr.enqueue("/c", "/d", 10).await.unwrap();
        mgr.cancel(job2.id).await.unwrap();
        let j = mgr.get_job(job2.id).await.unwrap().unwrap();
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
            external: false,
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

        let requeued = mgr.retry_all_failed().await.unwrap();
        assert_eq!(requeued.len(), 2);

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

    async fn mgr_with_journal() -> TransferManager {
        let mgr = TransferManager::new();
        let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();
        mgr.init_journal(pool).await;
        mgr
    }

    /// Regression: enqueue used to only record a Queued job that never ran.
    /// run_local_job must actually copy the bytes and mark the job Completed.
    #[tokio::test]
    async fn test_run_local_job_copies_file() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("payload.bin");
        let dst = dir.path().join("out").join("payload.bin");
        let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &bytes).unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), dst.to_str().unwrap(), bytes.len() as u64)
            .await
            .unwrap();
        mgr.run_local_job(job.id).await;

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Completed);
        assert_eq!(j.transferred_bytes, bytes.len() as u64);
        assert_eq!(std::fs::read(&dst).unwrap(), bytes);
    }

    /// Regression: `record_transfer` existed but had no callers, so the
    /// Transfer History panel could only ever render "No transfer history".
    /// A finished job must leave a history row, including the checksum
    /// verdict the pipeline already computed.
    #[tokio::test]
    async fn test_run_local_job_records_history() {
        let mgr = TransferManager::new();
        let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();
        mgr.init_journal(pool.clone()).await;
        let history = TransferHistory::new(pool);
        mgr.set_history(history.clone()).await;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        let dst = dir.path().join("b.bin");
        std::fs::write(&src, b"history me").unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), dst.to_str().unwrap(), 10)
            .await
            .unwrap();
        mgr.run_local_job(job.id).await;

        let records = history
            .search(crate::core::types::TransferHistoryFilter::default())
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, job.id);
        assert_eq!(records[0].status, TransferStatus::Completed);
        assert_eq!(records[0].transferred_bytes, 10);
        assert_eq!(records[0].checksum_match, Some(true));
    }

    /// Regression: remote destinations must fail fast. Previously a
    /// `sftp://…` dest fell through to `create_dir_all`, materialising a
    /// literal `sftp:` directory on local disk and "succeeding" into a
    /// junk path.
    #[tokio::test]
    async fn test_run_local_job_rejects_remote_dest() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        std::fs::write(&src, b"data").unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), "sftp://example.com/dir/a.bin", 4)
            .await
            .unwrap();
        mgr.run_local_job(job.id).await;

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Failed);
        assert!(j
            .error_message
            .unwrap_or_default()
            .contains("remote destination"));
        assert!(!std::path::Path::new("sftp:").exists());
    }

    #[tokio::test]
    async fn test_run_local_job_directory_destination() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("report.txt");
        let dstdir = dir.path().join("inbox");
        std::fs::create_dir_all(&dstdir).unwrap();
        std::fs::write(&src, b"hello queue").unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), dstdir.to_str().unwrap(), 11)
            .await
            .unwrap();
        mgr.run_local_job(job.id).await;

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Completed);
        assert_eq!(
            std::fs::read(dstdir.join("report.txt")).unwrap(),
            b"hello queue"
        );
    }

    #[tokio::test]
    async fn test_run_local_job_never_overwrites() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("new.bin");
        let dst = dir.path().join("existing.bin");
        std::fs::write(&src, b"new data").unwrap();
        std::fs::write(&dst, b"original").unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), dst.to_str().unwrap(), 8)
            .await
            .unwrap();
        mgr.run_local_job(job.id).await;

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Failed);
        assert!(j.error_message.unwrap().contains("already exists"));
        assert_eq!(std::fs::read(&dst).unwrap(), b"original");
    }

    #[tokio::test]
    async fn test_run_local_job_missing_source_fails() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("nope.bin");
        let dst = dir.path().join("out.bin");

        let job = mgr
            .enqueue(src.to_str().unwrap(), dst.to_str().unwrap(), 0)
            .await
            .unwrap();
        mgr.run_local_job(job.id).await;

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Failed);
        assert!(j.error_message.unwrap().contains("Cannot read source"));
        assert!(!dst.exists());
    }

    async fn wait_until(mut f: impl FnMut() -> bool, what: &str) {
        for _ in 0..500 {
            if f() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// Pause must actually hold a running transfer at a chunk boundary, and
    /// resume must let the same executor finish — not strand the job.
    #[tokio::test]
    async fn test_run_local_job_pause_resume() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("paused.bin");
        let dst = dir.path().join("paused-out.bin");
        let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &bytes).unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), dst.to_str().unwrap(), bytes.len() as u64)
            .await
            .unwrap();

        // Arm the flag before the executor starts so it parks deterministically
        // inside the pipeline (wait_if_paused at the chunk boundary).
        let control = mgr.control(job.id).await;
        control.pause();
        mgr.spawn_runner(job.id).await;

        let dst_str = dst.to_str().unwrap().to_string();
        let tmp = chunk::temp_path_for(&dst_str);
        // Wait until the transfer is genuinely in-flight (temp preallocated).
        wait_until(|| std::path::Path::new(&tmp).exists(), "temp file to appear").await;

        mgr.pause(job.id).await.unwrap();
        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Paused);

        mgr.resume(job.id).await.unwrap();

        // The parked worker wakes and the same copy finishes.
        for _ in 0..500 {
            let j = mgr.get_job(job.id).await.unwrap().unwrap();
            if j.status == TransferStatus::Completed {
                assert_eq!(std::fs::read(&dst).unwrap(), bytes);
                return;
            }
            assert_ne!(j.status, TransferStatus::Failed, "job failed: {:?}", j.error_message);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("job did not complete after resume");
    }

    /// Cancel must stop a running transfer, remove the partial temp file, and
    /// leave the destination untouched.
    #[tokio::test]
    async fn test_run_local_job_cancel_cleans_up() {
        let mgr = mgr_with_journal().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("victim.bin");
        let dst = dir.path().join("victim-out.bin");
        std::fs::write(&src, vec![7u8; 2_000_000]).unwrap();

        let job = mgr
            .enqueue(src.to_str().unwrap(), dst.to_str().unwrap(), 2_000_000)
            .await
            .unwrap();

        // Park the executor mid-flight so cancel lands while temp exists.
        let control = mgr.control(job.id).await;
        control.pause();
        mgr.spawn_runner(job.id).await;

        let dst_str = dst.to_str().unwrap().to_string();
        let tmp = chunk::temp_path_for(&dst_str);
        wait_until(|| std::path::Path::new(&tmp).exists(), "temp file to appear").await;

        mgr.cancel(job.id).await.unwrap();

        // The executor wakes, aborts, removes the temp file and exits.
        wait_until(|| !std::path::Path::new(&tmp).exists(), "temp file to be removed").await;
        wait_until(
            || {
                let set = mgr.executors.try_lock().ok();
                set.map(|s| !s.contains(&job.id)).unwrap_or(false)
            },
            "executor to exit",
        )
        .await;

        let j = mgr.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(j.status, TransferStatus::Cancelled);
        assert!(!dst.exists());
    }

    /// Regression: connector transfers shown in the queue offered Pause and
    /// Retry that did nothing, and a cancelled one turned "completed" once
    /// the connector finished.
    #[tokio::test]
    async fn tracked_external_jobs_refuse_pause_and_retry_and_stay_cancelled() {
        let mgr = TransferManager::new();
        let job = mgr.track_external("sftp://Server/a.txt", "/tmp/a.txt", 10, None).await.unwrap();
        assert!(job.external);
        assert!(mgr.pause(job.id).await.is_err(), "pausing a connector transfer is refused");
        assert_eq!(mgr.get_job(job.id).await.unwrap().unwrap().status, TransferStatus::Active);
        mgr.cancel(job.id).await.unwrap();
        mgr.finish_external(job.id, Ok(10), Utc::now(), std::time::Duration::ZERO).await;
        assert_eq!(mgr.get_job(job.id).await.unwrap().unwrap().status, TransferStatus::Cancelled);

        let failed = mgr.track_external("sftp://Server/b.txt", "/tmp/b.txt", 10, None).await.unwrap();
        mgr.finish_external(failed.id, Err("boom".into()), Utc::now(), std::time::Duration::ZERO).await;
        assert!(mgr.increment_retry(failed.id).await.is_err(), "the queue cannot re-run a connector transfer");
        assert!(mgr.retry_all_failed().await.unwrap().is_empty());
        assert_eq!(mgr.get_job(failed.id).await.unwrap().unwrap().status, TransferStatus::Failed);
    }
}
