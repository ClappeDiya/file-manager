//! Transfer journal for crash recovery (Layer 3).
//!
//! Uses SQLite WAL-mode tables (transfer_jobs + transfer_chunks) to maintain
//! a write-ahead log of all active transfers. On crash recovery, the journal
//! identifies exactly which chunks need re-transfer via a chunk bitmap.

use crate::core::error::AppError;
use crate::core::types::{ChunkSpec, ChunkStatus, TransferJobStatus, VerifyTier};
use crate::storage::pool::DbPool;
use chrono::Utc;

/// Transfer journal backed by SQLite.
#[derive(Clone, Debug)]
pub struct TransferJournal {
    pool: DbPool,
}

/// A row from the transfer_jobs table.
#[derive(Debug, Clone)]
pub struct JournalJob {
    pub job_id: String,
    pub source_path: String,
    pub dest_path: String,
    pub temp_path: String,
    pub file_size: u64,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub verify_tier: VerifyTier,
    pub status: TransferJobStatus,
    pub source_hash: Option<String>,
    pub dest_hash: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

/// A row from the transfer_chunks table.
#[derive(Debug, Clone)]
pub struct JournalChunk {
    pub job_id: String,
    pub chunk_index: u32,
    pub offset: u64,
    pub length: u64,
    pub status: String,
    pub source_hash: Option<String>,
    pub dest_hash: Option<String>,
    pub attempts: u32,
    pub verified_at: Option<String>,
}

impl TransferJournal {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Create a new journal entry for a transfer job, including all chunk rows.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_job(
        &self,
        job_id: &str,
        source_path: &str,
        dest_path: &str,
        temp_path: &str,
        file_size: u64,
        chunk_size: u64,
        chunks: &[ChunkSpec],
        verify_tier: VerifyTier,
    ) -> Result<(), AppError> {
        let job_id = job_id.to_string();
        let source_path = source_path.to_string();
        let dest_path = dest_path.to_string();
        let temp_path = temp_path.to_string();
        let now = Utc::now().to_rfc3339();
        let tier = verify_tier.as_int();
        let chunk_count = chunks.len() as u32;
        let chunks: Vec<(u32, u64, u64)> = chunks
            .iter()
            .map(|c| (c.chunk_index, c.offset, c.length))
            .collect();

        self.pool
            .execute(move |conn| {
                let tx = conn.unchecked_transaction()?;

                tx.execute(
                    "INSERT INTO transfer_jobs (job_id, source_path, dest_path, temp_path,
                     file_size, chunk_size, chunk_count, verify_tier, status,
                     started_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?9)",
                    rusqlite::params![
                        job_id,
                        source_path,
                        dest_path,
                        temp_path,
                        file_size as i64,
                        chunk_size as i64,
                        chunk_count as i64,
                        tier,
                        now,
                    ],
                )?;

                for (idx, offset, length) in &chunks {
                    tx.execute(
                        "INSERT INTO transfer_chunks (job_id, chunk_index, offset, length, status)
                         VALUES (?1, ?2, ?3, ?4, 'pending')",
                        rusqlite::params![
                            job_id,
                            *idx as i64,
                            *offset as i64,
                            *length as i64,
                        ],
                    )?;
                }

                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Update the status of a single chunk.
    pub async fn update_chunk_status(
        &self,
        job_id: &str,
        chunk_index: u32,
        status: &ChunkStatus,
        source_hash: Option<&str>,
        dest_hash: Option<&str>,
    ) -> Result<(), AppError> {
        let job_id = job_id.to_string();
        let status_str = status.as_str().to_string();
        let source_hash = source_hash.map(|s| s.to_string());
        let dest_hash = dest_hash.map(|s| s.to_string());
        let now = Utc::now().to_rfc3339();
        let verified_at = if matches!(status, ChunkStatus::Verified) {
            Some(now.clone())
        } else {
            None
        };

        self.pool
            .execute(move |conn| {
                conn.execute(
                    "UPDATE transfer_chunks SET status = ?1, source_hash = ?2, dest_hash = ?3,
                     attempts = attempts + 1, verified_at = ?4
                     WHERE job_id = ?5 AND chunk_index = ?6",
                    rusqlite::params![
                        status_str,
                        source_hash,
                        dest_hash,
                        verified_at,
                        job_id,
                        chunk_index as i64,
                    ],
                )?;

                // Update job timestamp
                conn.execute(
                    "UPDATE transfer_jobs SET updated_at = ?1 WHERE job_id = ?2",
                    rusqlite::params![now, job_id],
                )?;

                Ok(())
            })
            .await
    }

    /// Mark a job as completed with final hashes.
    pub async fn complete_job(
        &self,
        job_id: &str,
        source_hash: Option<&str>,
        dest_hash: Option<&str>,
    ) -> Result<(), AppError> {
        let job_id = job_id.to_string();
        let source_hash = source_hash.map(|s| s.to_string());
        let dest_hash = dest_hash.map(|s| s.to_string());
        let now = Utc::now().to_rfc3339();

        self.pool
            .execute(move |conn| {
                conn.execute(
                    "UPDATE transfer_jobs SET status = 'completed', source_hash = ?1,
                     dest_hash = ?2, completed_at = ?3, updated_at = ?3
                     WHERE job_id = ?4",
                    rusqlite::params![source_hash, dest_hash, now, job_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Mark a job as failed with an error message.
    pub async fn fail_job(&self, job_id: &str, error: &str) -> Result<(), AppError> {
        let job_id = job_id.to_string();
        let error = error.to_string();
        let now = Utc::now().to_rfc3339();

        self.pool
            .execute(move |conn| {
                conn.execute(
                    "UPDATE transfer_jobs SET status = 'failed', error = ?1, updated_at = ?2
                     WHERE job_id = ?3",
                    rusqlite::params![error, now, job_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Mark a job as cancelled.
    pub async fn cancel_job(&self, job_id: &str) -> Result<(), AppError> {
        let job_id = job_id.to_string();
        let now = Utc::now().to_rfc3339();

        self.pool
            .execute(move |conn| {
                conn.execute(
                    "UPDATE transfer_jobs SET status = 'cancelled', updated_at = ?1
                     WHERE job_id = ?2",
                    rusqlite::params![now, job_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Query all active jobs (for crash recovery on startup).
    pub async fn list_active_jobs(&self) -> Result<Vec<JournalJob>, AppError> {
        self.pool
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT job_id, source_path, dest_path, temp_path, file_size,
                            chunk_size, chunk_count, verify_tier, status,
                            source_hash, dest_hash, started_at, updated_at,
                            completed_at, error
                     FROM transfer_jobs WHERE status = 'active'
                     ORDER BY started_at ASC",
                )?;
                let jobs = stmt
                    .query_map([], |row| {
                        Ok(JournalJob {
                            job_id: row.get(0)?,
                            source_path: row.get(1)?,
                            dest_path: row.get(2)?,
                            temp_path: row.get(3)?,
                            file_size: row.get::<_, i64>(4)? as u64,
                            chunk_size: row.get::<_, i64>(5)? as u64,
                            chunk_count: row.get::<_, i64>(6)? as u32,
                            verify_tier: VerifyTier::from_int(row.get::<_, i32>(7)?),
                            status: TransferJobStatus::from_str_lossy(&row.get::<_, String>(8)?),
                            source_hash: row.get(9)?,
                            dest_hash: row.get(10)?,
                            started_at: row.get(11)?,
                            updated_at: row.get(12)?,
                            completed_at: row.get(13)?,
                            error: row.get(14)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(jobs)
            })
            .await
    }

    /// Get all chunks for a job (for building the resume bitmap).
    pub async fn get_chunks(&self, job_id: &str) -> Result<Vec<JournalChunk>, AppError> {
        let job_id = job_id.to_string();
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT job_id, chunk_index, offset, length, status,
                            source_hash, dest_hash, attempts, verified_at
                     FROM transfer_chunks WHERE job_id = ?1
                     ORDER BY chunk_index ASC",
                )?;
                let chunks = stmt
                    .query_map(rusqlite::params![job_id], |row| {
                        Ok(JournalChunk {
                            job_id: row.get(0)?,
                            chunk_index: row.get::<_, i64>(1)? as u32,
                            offset: row.get::<_, i64>(2)? as u64,
                            length: row.get::<_, i64>(3)? as u64,
                            status: row.get(4)?,
                            source_hash: row.get(5)?,
                            dest_hash: row.get(6)?,
                            attempts: row.get::<_, i64>(7)? as u32,
                            verified_at: row.get(8)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(chunks)
            })
            .await
    }

    /// Get counts of chunks by status for a job.
    pub async fn chunk_status_counts(
        &self,
        job_id: &str,
    ) -> Result<ChunkCounts, AppError> {
        let job_id = job_id.to_string();
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT status, COUNT(*) FROM transfer_chunks
                     WHERE job_id = ?1 GROUP BY status",
                )?;
                let mut counts = ChunkCounts::default();
                let rows = stmt.query_map(rusqlite::params![job_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
                })?;
                for (status, count) in rows.flatten() {
                    match status.as_str() {
                        "pending" => counts.pending = count,
                        "reading" => counts.reading = count,
                        "writing" => counts.writing = count,
                        "verifying" => counts.verifying = count,
                        "verified" => counts.verified = count,
                        "failed" => counts.failed = count,
                        "skipped" => counts.skipped = count,
                        _ => {}
                    }
                }
                Ok(counts)
            })
            .await
    }

    /// Build a chunk bitmap: returns indices of chunks that need transfer.
    /// A chunk needs transfer if it is pending, reading, writing, or failed.
    pub async fn pending_chunk_indices(&self, job_id: &str) -> Result<Vec<u32>, AppError> {
        let job_id = job_id.to_string();
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT chunk_index FROM transfer_chunks
                     WHERE job_id = ?1 AND status IN ('pending', 'reading', 'writing', 'failed')
                     ORDER BY chunk_index ASC",
                )?;
                let indices = stmt
                    .query_map(rusqlite::params![job_id], |row| {
                        Ok(row.get::<_, i64>(0)? as u32)
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(indices)
            })
            .await
    }

    /// Clean up completed/failed/cancelled jobs older than retention_days.
    pub async fn cleanup_old_jobs(&self, retention_days: u32) -> Result<u32, AppError> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        self.pool
            .execute(move |conn| {
                // Chunks are cascade-deleted
                let deleted = conn.execute(
                    "DELETE FROM transfer_jobs
                     WHERE status IN ('completed', 'failed', 'cancelled')
                     AND updated_at < ?1",
                    rusqlite::params![cutoff_str],
                )?;
                Ok(deleted as u32)
            })
            .await
    }

    /// Delete a specific job and its chunks.
    pub async fn delete_job(&self, job_id: &str) -> Result<(), AppError> {
        let job_id = job_id.to_string();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM transfer_jobs WHERE job_id = ?1",
                    rusqlite::params![job_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Reset all in-progress chunks (reading/writing/verifying) back to pending.
    /// Used during crash recovery.
    pub async fn reset_in_progress_chunks(&self, job_id: &str) -> Result<u32, AppError> {
        let job_id = job_id.to_string();
        self.pool
            .execute(move |conn| {
                let count = conn.execute(
                    "UPDATE transfer_chunks SET status = 'pending'
                     WHERE job_id = ?1 AND status IN ('reading', 'writing', 'verifying')",
                    rusqlite::params![job_id],
                )?;
                Ok(count as u32)
            })
            .await
    }
}

/// Summary counts of chunks by status.
#[derive(Debug, Clone, Default)]
pub struct ChunkCounts {
    pub pending: u32,
    pub reading: u32,
    pub writing: u32,
    pub verifying: u32,
    pub verified: u32,
    pub failed: u32,
    pub skipped: u32,
}

impl ChunkCounts {
    pub fn total(&self) -> u32 {
        self.pending + self.reading + self.writing + self.verifying + self.verified + self.failed + self.skipped
    }

    pub fn is_all_done(&self) -> bool {
        self.pending == 0 && self.reading == 0 && self.writing == 0 && self.verifying == 0 && self.failed == 0
    }
}

/// Crash recovery procedure.
///
/// Steps:
/// 1. Query transfer_jobs WHERE status = 'active'
/// 2. For each active job:
///    a. Verify temp file (.ufop-partial) exists
///    b. Query transfer_chunks for this job
///    c. Reset in-progress chunks to pending
///    d. Build bitmap: verified = skip, pending/failed = re-transfer
/// 3. For jobs where temp file is missing:
///    a. Mark all chunks as pending (restart from scratch)
pub async fn recover_active_jobs(journal: &TransferJournal) -> Result<Vec<RecoverableJob>, AppError> {
    let active_jobs = journal.list_active_jobs().await?;
    let mut recoverable = Vec::new();

    for job in active_jobs {
        let temp_exists = std::path::Path::new(&job.temp_path).exists();

        // Reset in-progress chunks
        let reset_count = journal.reset_in_progress_chunks(&job.job_id).await?;
        if reset_count > 0 {
            tracing::info!(
                "Recovery: reset {} in-progress chunks for job {}",
                reset_count,
                job.job_id
            );
        }

        if !temp_exists {
            // Temp file missing - need to restart from scratch
            tracing::warn!(
                "Recovery: temp file {} missing for job {}, will restart",
                job.temp_path,
                job.job_id
            );
            // Reset all chunks to pending
            let job_id = job.job_id.clone();
            journal
                .pool
                .execute(move |conn| {
                    conn.execute(
                        "UPDATE transfer_chunks SET status = 'pending', source_hash = NULL, dest_hash = NULL, attempts = 0
                         WHERE job_id = ?1",
                        rusqlite::params![job_id],
                    )?;
                    Ok(())
                })
                .await?;
        }

        let pending = journal.pending_chunk_indices(&job.job_id).await?;
        let counts = journal.chunk_status_counts(&job.job_id).await?;

        recoverable.push(RecoverableJob {
            job,
            temp_file_exists: temp_exists,
            pending_chunks: pending,
            verified_count: counts.verified,
            total_chunks: counts.total(),
        });
    }

    Ok(recoverable)
}

/// A job found during crash recovery with its recovery state.
#[derive(Debug)]
pub struct RecoverableJob {
    pub job: JournalJob,
    pub temp_file_exists: bool,
    pub pending_chunks: Vec<u32>,
    pub verified_count: u32,
    pub total_chunks: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pool::DbPool;

    async fn setup_db() -> DbPool {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_create_and_query_job() {
        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);

        let chunks = vec![
            ChunkSpec {
                chunk_index: 0,
                offset: 0,
                length: 1024,
                source_hash: None,
                dest_hash: None,
                sha256: None,
                status: ChunkStatus::Pending,
                write_time_ms: 0,
                verify_time_ms: 0,
            },
            ChunkSpec {
                chunk_index: 1,
                offset: 1024,
                length: 512,
                source_hash: None,
                dest_hash: None,
                sha256: None,
                status: ChunkStatus::Pending,
                write_time_ms: 0,
                verify_time_ms: 0,
            },
        ];

        journal
            .create_job(
                "test-job-1",
                "/src/file.bin",
                "/dst/file.bin",
                "/dst/.file.bin.ufop-partial",
                1536,
                1024,
                &chunks,
                VerifyTier::Fast,
            )
            .await
            .unwrap();

        let active = journal.list_active_jobs().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].job_id, "test-job-1");
        assert_eq!(active[0].file_size, 1536);
        assert_eq!(active[0].chunk_count, 2);
        assert_eq!(active[0].verify_tier, VerifyTier::Fast);

        let chunks = journal.get_chunks("test-job-1").await.unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 1024);
        assert_eq!(chunks[1].offset, 1024);
        assert_eq!(chunks[1].length, 512);
    }

    #[tokio::test]
    async fn test_update_chunk_status() {
        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);

        let chunks = vec![ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: 1024,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        }];

        journal
            .create_job("job-2", "/src", "/dst", "/tmp", 1024, 1024, &chunks, VerifyTier::Verified)
            .await
            .unwrap();

        // Update to verified
        journal
            .update_chunk_status("job-2", 0, &ChunkStatus::Verified, Some("abc123"), Some("abc123"))
            .await
            .unwrap();

        let chunks = journal.get_chunks("job-2").await.unwrap();
        assert_eq!(chunks[0].status, "verified");
        assert_eq!(chunks[0].source_hash.as_deref(), Some("abc123"));
        assert!(chunks[0].verified_at.is_some());
    }

    #[tokio::test]
    async fn test_complete_and_fail_job() {
        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);

        let chunks = vec![ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: 100,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        }];

        journal
            .create_job("job-c", "/src", "/dst", "/tmp", 100, 100, &chunks, VerifyTier::Fast)
            .await
            .unwrap();

        journal
            .complete_job("job-c", Some("hash-s"), Some("hash-d"))
            .await
            .unwrap();

        let active = journal.list_active_jobs().await.unwrap();
        assert!(active.is_empty()); // completed jobs are not active

        // Test fail
        journal
            .create_job("job-f", "/src2", "/dst2", "/tmp2", 100, 100, &chunks, VerifyTier::Fast)
            .await
            .unwrap();
        journal.fail_job("job-f", "disk full").await.unwrap();
        let active = journal.list_active_jobs().await.unwrap();
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn test_pending_chunk_bitmap() {
        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);

        let chunks: Vec<ChunkSpec> = (0..5)
            .map(|i| ChunkSpec {
                chunk_index: i,
                offset: i as u64 * 1024,
                length: 1024,
                source_hash: None,
                dest_hash: None,
                sha256: None,
                status: ChunkStatus::Pending,
                write_time_ms: 0,
                verify_time_ms: 0,
            })
            .collect();

        journal
            .create_job("job-bm", "/src", "/dst", "/tmp", 5120, 1024, &chunks, VerifyTier::Fast)
            .await
            .unwrap();

        // Mark chunks 0, 2, 4 as verified
        for idx in [0, 2, 4] {
            journal
                .update_chunk_status("job-bm", idx, &ChunkStatus::Verified, None, None)
                .await
                .unwrap();
        }

        let pending = journal.pending_chunk_indices("job-bm").await.unwrap();
        assert_eq!(pending, vec![1, 3]);

        let counts = journal.chunk_status_counts("job-bm").await.unwrap();
        assert_eq!(counts.verified, 3);
        assert_eq!(counts.pending, 2);
    }

    #[tokio::test]
    async fn test_reset_in_progress_chunks() {
        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);

        let chunks: Vec<ChunkSpec> = (0..3)
            .map(|i| ChunkSpec {
                chunk_index: i,
                offset: i as u64 * 100,
                length: 100,
                source_hash: None,
                dest_hash: None,
                sha256: None,
                status: ChunkStatus::Pending,
                write_time_ms: 0,
                verify_time_ms: 0,
            })
            .collect();

        journal
            .create_job("job-reset", "/s", "/d", "/t", 300, 100, &chunks, VerifyTier::Fast)
            .await
            .unwrap();

        // Simulate crash: chunk 0 verified, chunk 1 writing, chunk 2 reading
        journal
            .update_chunk_status("job-reset", 0, &ChunkStatus::Verified, None, None)
            .await
            .unwrap();
        journal
            .update_chunk_status("job-reset", 1, &ChunkStatus::Writing, None, None)
            .await
            .unwrap();
        journal
            .update_chunk_status("job-reset", 2, &ChunkStatus::Reading, None, None)
            .await
            .unwrap();

        let reset = journal.reset_in_progress_chunks("job-reset").await.unwrap();
        assert_eq!(reset, 2);

        let pending = journal.pending_chunk_indices("job-reset").await.unwrap();
        assert_eq!(pending, vec![1, 2]);
    }

    #[tokio::test]
    async fn test_cleanup_old_jobs() {
        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);

        let chunks = vec![ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: 100,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        }];

        journal
            .create_job("old-job", "/s", "/d", "/t", 100, 100, &chunks, VerifyTier::Fast)
            .await
            .unwrap();
        journal.complete_job("old-job", None, None).await.unwrap();

        // Cleanup with 0 retention should delete it
        let deleted = journal.cleanup_old_jobs(0).await.unwrap();
        assert_eq!(deleted, 1);
    }
}
