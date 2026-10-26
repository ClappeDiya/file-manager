//! How long failed transfers stay retryable. A failed job is kept across
//! restarts (its error and what it had copied) so it can be retried later;
//! this bounds that data by age and by total size, so a machine that fails
//! thousands of transfers does not carry them forever.

use crate::core::error::AppError;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Config-table key of the saved limits.
pub const CONFIG_KEY: &str = "transfer.failed_retention";
/// Bookkeeping bytes counted for every kept job on top of its stored text.
const ROW_OVERHEAD: i64 = 256;

/// The limits on retained failed transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FailedRetention {
    /// Failed jobs older than this are dropped.
    pub max_age_days: u32,
    /// The newest failed jobs are kept while their stored data fits in this many KiB.
    pub max_total_kb: u32,
}

impl Default for FailedRetention {
    fn default() -> Self {
        Self { max_age_days: 30, max_total_kb: 4096 }
    }
}

impl FailedRetention {
    pub const MAX_AGE_DAYS: (u32, u32) = (1, 3650);
    pub const MAX_TOTAL_KB: (u32, u32) = (16, 1_048_576);

    /// The limits clamped to what the settings accept.
    pub fn clamped(self) -> Self {
        Self {
            max_age_days: self.max_age_days.clamp(Self::MAX_AGE_DAYS.0, Self::MAX_AGE_DAYS.1),
            max_total_kb: self.max_total_kb.clamp(Self::MAX_TOTAL_KB.0, Self::MAX_TOTAL_KB.1),
        }
    }

    /// The saved limits, or the defaults.
    pub async fn load(repo: &Repository) -> FailedRetention {
        match repo.get_config(CONFIG_KEY).await {
            Ok(Some(json)) => serde_json::from_str::<FailedRetention>(&json).map(Self::clamped).unwrap_or_default(),
            _ => Self::default(),
        }
    }

    /// Saves (clamped) limits and returns what was saved.
    pub async fn save(self, repo: &Repository) -> Result<FailedRetention, AppError> {
        let limits = self.clamped();
        repo.set_config(CONFIG_KEY, &serde_json::to_string(&limits)?).await?;
        Ok(limits)
    }
}

/// Deletes the stored failed jobs that are past `limits`: older than the age,
/// then the oldest ones beyond the size budget. Returns the removed ids.
pub async fn prune_failed(repo: &Repository, limits: FailedRetention) -> Result<Vec<Uuid>, AppError> {
    let limits = limits.clamped();
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(limits.max_age_days as i64)).to_rfc3339();
    let budget = limits.max_total_kb as i64 * 1024;
    repo.pool()
        .execute(move |conn| {
            let rows: Vec<(String, String, i64)> = {
                let mut st = conn.prepare(
                    "SELECT id, updated_at,
                            LENGTH(source_path) + LENGTH(dest_path) + COALESCE(LENGTH(error_message), 0) + COALESCE(LENGTH(partial_checksum), 0)
                            + COALESCE(LENGTH(source_checksum), 0) + COALESCE(LENGTH(dest_checksum), 0) + ?1
                     FROM transfer_history WHERE status = 'failed' ORDER BY updated_at DESC, id",
                )?;
                let rows = st.query_map([ROW_OVERHEAD], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
                rows
            };
            let mut used = 0i64;
            let mut doomed = Vec::new();
            for (id, updated, size) in rows {
                // Newest first: once one is too old or over budget, so is every later one.
                if doomed.is_empty() && updated >= cutoff && used + size <= budget {
                    used += size;
                } else {
                    doomed.push(id);
                }
            }
            let tx = conn.unchecked_transaction()?;
            for id in &doomed {
                tx.execute("DELETE FROM transfer_history WHERE id = ?1", [id])?;
            }
            tx.commit()?;
            Ok(doomed.iter().filter_map(|id| Uuid::parse_str(id).ok()).collect())
        })
        .await
}

/// Startup: drop what the saved limits no longer keep, before the queue is restored.
pub async fn enforce_at_startup(repo: &Repository) {
    let limits = FailedRetention::load(repo).await;
    match prune_failed(repo, limits).await {
        Ok(removed) if !removed.is_empty() => tracing::info!("Dropped {} failed transfer(s) past the retention limits", removed.len()),
        Ok(_) => {}
        Err(e) => tracing::warn!("Could not apply the failed-transfer retention (non-fatal): {e}"),
    }
}

/// Save every job that fails the moment it fails, so it is still there (and
/// retryable) after a crash or a force-quit, not only after a clean exit.
pub fn persist_failures(manager: &crate::transfer_engine::TransferManager, repo: Repository) {
    manager.add_terminal_hook(std::sync::Arc::new(move |job| {
        if job.status != crate::core::types::TransferStatus::Failed {
            return;
        }
        let (repo, job) = (repo.clone(), job.clone());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                use crate::core::traits::StorageOperations;
                if let Err(e) = StorageOperations::save_transfer_queue(&repo, std::slice::from_ref(&job)).await {
                    tracing::warn!("Could not save the failed transfer {}: {e}", job.id);
                }
            });
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::StorageOperations;
    use crate::core::types::{TransferJob, TransferPriority, TransferStatus};
    use chrono::{Duration, Utc};

    fn failed(name: &str, age_days: i64, error: &str) -> TransferJob {
        let at = Utc::now() - Duration::days(age_days);
        TransferJob {
            id: Uuid::new_v4(),
            source_path: format!("/src/{name}"),
            dest_path: format!("/dst/{name}"),
            total_bytes: 10,
            transferred_bytes: 0,
            status: TransferStatus::Failed,
            priority: TransferPriority::Normal,
            sort_order: 0,
            created_at: at,
            updated_at: at,
            error_message: Some(error.to_string()),
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
        }
    }

    async fn stored(repo: &Repository) -> Vec<String> {
        let mut v: Vec<String> = StorageOperations::load_transfer_queue(repo).await.unwrap().into_iter().map(|j| j.source_path).collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn old_failed_jobs_are_dropped_and_recent_ones_kept() {
        let repo = Repository::open_in_memory().await.unwrap();
        let jobs = [failed("recent", 1, "x"), failed("middling", 20, "x"), failed("ancient", 90, "x")];
        StorageOperations::save_transfer_queue(&repo, &jobs).await.unwrap();

        let removed = prune_failed(&repo, FailedRetention { max_age_days: 30, max_total_kb: 4096 }).await.unwrap();

        assert_eq!(removed, vec![jobs[2].id]);
        assert_eq!(stored(&repo).await, ["/src/middling", "/src/recent"]);
    }

    #[tokio::test]
    async fn the_size_cap_keeps_the_newest_and_drops_the_rest() {
        let repo = Repository::open_in_memory().await.unwrap();
        let big = "e".repeat(6 * 1024);
        let jobs = [failed("newest", 1, &big), failed("second", 2, &big), failed("third", 3, &big)];
        StorageOperations::save_transfer_queue(&repo, &jobs).await.unwrap();

        // 16 KiB fits two of the ~6 KiB rows.
        let removed = prune_failed(&repo, FailedRetention { max_age_days: 30, max_total_kb: 16 }).await.unwrap();

        assert_eq!(removed, vec![jobs[2].id]);
        assert_eq!(stored(&repo).await, ["/src/newest", "/src/second"]);
    }

    #[tokio::test]
    async fn only_failed_jobs_are_pruned() {
        let repo = Repository::open_in_memory().await.unwrap();
        let mut paused = failed("paused", 400, "x");
        paused.status = TransferStatus::Paused;
        StorageOperations::save_transfer_queue(&repo, &[paused, failed("old", 400, "x")]).await.unwrap();

        prune_failed(&repo, FailedRetention::default()).await.unwrap();

        assert_eq!(stored(&repo).await, ["/src/paused"]);
    }

    #[tokio::test]
    async fn a_job_is_saved_the_moment_it_fails() {
        use crate::core::traits::TransferOperations;
        let repo = Repository::open_in_memory().await.unwrap();
        let mgr = crate::transfer_engine::TransferManager::new();
        persist_failures(&mgr, repo.clone());
        let job = mgr.enqueue("/src/a.bin", "/dst/a.bin", 10).await.unwrap();
        assert!(StorageOperations::load_transfer_queue(&repo).await.unwrap().is_empty(), "queued jobs are saved on exit, as before");

        mgr.fail_job(job.id, "network down").await.unwrap();

        let mut restored = Vec::new();
        for _ in 0..100 {
            restored = StorageOperations::load_transfer_queue(&repo).await.unwrap();
            if !restored.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(restored.len(), 1);
        assert_eq!((restored[0].status, restored[0].error_message.as_deref()), (TransferStatus::Failed, Some("network down")));
    }

    #[tokio::test]
    async fn a_restored_failed_job_waits_for_the_user_to_retry_it() {
        let repo = Repository::open_in_memory().await.unwrap();
        let job = failed("report.bin", 2, "timed out");
        StorageOperations::save_transfer_queue(&repo, std::slice::from_ref(&job)).await.unwrap();

        // "Restart": a fresh manager restores the queue.
        let mgr = crate::transfer_engine::TransferManager::new();
        mgr.restore_jobs(StorageOperations::load_transfer_queue(&repo).await.unwrap()).await;

        let listed = mgr.list_failed_jobs().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].error_message.as_deref(), Some("timed out"));
        // Nothing retried it on its own.
        assert_eq!(mgr.snapshot_jobs().await[0].status, TransferStatus::Failed);
        // Pruning drops it from memory as well.
        mgr.forget_failed(&[job.id]).await;
        assert!(mgr.snapshot_jobs().await.is_empty());
        // The user's retry re-queues it.
        mgr.restore_jobs(vec![job.clone()]).await;
        assert_eq!(mgr.retry_all_failed().await.unwrap(), vec![job.id]);
    }

    #[tokio::test]
    async fn limits_are_saved_clamped_and_default_when_unset() {
        let repo = Repository::open_in_memory().await.unwrap();
        assert_eq!(FailedRetention::load(&repo).await, FailedRetention { max_age_days: 30, max_total_kb: 4096 });
        let saved = FailedRetention { max_age_days: 0, max_total_kb: 0 }.save(&repo).await.unwrap();
        assert_eq!(saved, FailedRetention { max_age_days: 1, max_total_kb: 16 });
        assert_eq!(FailedRetention::load(&repo).await, saved);
        repo.set_config(CONFIG_KEY, "not json").await.unwrap();
        assert_eq!(FailedRetention::load(&repo).await, FailedRetention::default());
    }
}
