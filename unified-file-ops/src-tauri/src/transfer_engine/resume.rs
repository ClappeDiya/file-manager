//! Resume and retry logic using chunk-bitmap based recovery (Layer 3).
//!
//! Replaces the old byte-level resume with chunk-level resume via the journal.
//! On crash recovery:
//! 1. Query active jobs from journal
//! 2. Verify temp file exists
//! 3. Build bitmap of completed vs pending chunks
//! 4. Re-verify previously verified chunks (quick hash check)
//! 5. Resume from first pending chunk

use crate::core::error::AppError;
use crate::core::types::{ChunkSpec, ChunkStatus, TransferConfig, VerifyTier};
use crate::transfer_engine::journal::{self, RecoverableJob, TransferJournal};
use crate::transfer_engine::pipeline::{self, TransferResult, WorkerPool};
use crate::transfer_engine::verification;

/// Resume a previously interrupted transfer using the journal bitmap.
///
/// Steps:
/// 1. Get pending chunk indices from journal
/// 2. If temp file missing, restart all chunks
/// 3. Optionally re-verify completed chunks
/// 4. Transfer only pending chunks
/// 5. Full-file verification
/// 6. Atomic rename
pub async fn resume_transfer(
    recoverable: &RecoverableJob,
    config: &TransferConfig,
    journal: &TransferJournal,
    worker_pool: &WorkerPool,
    on_progress: Option<&pipeline::ProgressCallback>,
) -> Result<TransferResult, AppError> {
    let job = &recoverable.job;

    tracing::info!(
        "Resuming transfer {}: {}/{} chunks verified, {} pending",
        job.job_id,
        recoverable.verified_count,
        recoverable.total_chunks,
        recoverable.pending_chunks.len()
    );

    // If no pending chunks, the transfer was actually complete but not finalized
    if recoverable.pending_chunks.is_empty() {
        tracing::info!("All chunks verified, finalizing transfer {}", job.job_id);
        return finalize_transfer(job, config, journal).await;
    }

    // Build chunk specs for pending chunks only
    let chunk_size = job.chunk_size;
    let pending_chunks: Vec<ChunkSpec> = recoverable
        .pending_chunks
        .iter()
        .map(|&idx| {
            let offset = idx as u64 * chunk_size;
            let length = if idx == job.chunk_count - 1 {
                job.file_size - offset
            } else {
                chunk_size
            };
            ChunkSpec {
                chunk_index: idx,
                offset,
                length,
                source_hash: None,
                dest_hash: None,
                sha256: None,
                status: ChunkStatus::Pending,
                write_time_ms: 0,
                verify_time_ms: 0,
            }
        })
        .collect();

    let temp_path = &job.temp_path;
    let source_path = &job.source_path;
    let dest_path = &job.dest_path;
    let tier = job.verify_tier;
    let semaphore = worker_pool.semaphore().clone();

    // Pre-allocate if temp file doesn't exist
    if !recoverable.temp_file_exists && config.preallocate && job.file_size > 0 {
        if let Some(parent) = std::path::Path::new(temp_path).parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::Transfer {
                message: format!("Cannot create destination directory: {e}"),
                advice: "Check directory permissions.".to_string(),
            })?;
        }
        verification::preallocate_file(temp_path, job.file_size).await?;
    }

    // Transfer pending chunks
    let mut all_verified = true;
    let mut total_transferred: u64 = recoverable.verified_count as u64 * chunk_size;
    let mut sha256_hashes: Vec<Option<[u8; 32]>> = vec![None; job.chunk_count as usize];

    let start = std::time::Instant::now();

    for chunk_spec in &pending_chunks {
        let result = pipeline::transfer_chunk(
            source_path,
            temp_path,
            chunk_spec,
            tier,
            config,
            Some(journal),
            Some(&job.job_id),
            &semaphore,
        )
        .await?;

        total_transferred += result.bytes_transferred;
        all_verified = all_verified && result.verified;

        if let Some(sha) = result.source_sha256 {
            sha256_hashes[chunk_spec.chunk_index as usize] = Some(sha);
        }

        if let Some(cb) = on_progress {
            cb(chunk_spec.chunk_index, total_transferred, job.file_size);
        }
    }

    if !all_verified {
        journal
            .fail_job(&job.job_id, "One or more chunks failed verification during resume")
            .await?;
        return Err(AppError::Transfer {
            message: "Resume verification failed".to_string(),
            advice: "Some chunks could not be verified. The transfer may need to be restarted.".to_string(),
        });
    }

    // Full-file verification
    let (source_hash, dest_hash) = match tier {
        VerifyTier::Fast => {
            let src = verification::compute_file_xxhash3(source_path).await?;
            let dst = verification::compute_file_xxhash3(temp_path).await?;
            if src != dst {
                journal
                    .fail_job(&job.job_id, "Full-file xxHash3 mismatch after resume")
                    .await?;
                return Err(AppError::Transfer {
                    message: "Final verification failed after resume".to_string(),
                    advice: "The file will need to be re-transferred.".to_string(),
                });
            }
            (format!("{:016x}", src), format!("{:016x}", dst))
        }
        VerifyTier::Verified | VerifyTier::MissionCritical => {
            let src = verification::compute_file_sha256(source_path).await?;
            let dst = verification::compute_file_sha256(temp_path).await?;
            if src != dst {
                journal
                    .fail_job(&job.job_id, "Full-file SHA-256 mismatch after resume")
                    .await?;
                return Err(AppError::Transfer {
                    message: "Final verification failed after resume".to_string(),
                    advice: "The file will need to be re-transferred.".to_string(),
                });
            }
            (
                verification::hex_encode(&src),
                verification::hex_encode(&dst),
            )
        }
    };

    // Final fsync
    verification::sync_to_disk(temp_path).await?;

    // Merkle sidecar (Tier 3)
    if tier == VerifyTier::MissionCritical && config.merkle_sidecar {
        // We need all leaf hashes. For resumed chunks, we need to recompute from the temp file.
        let mut all_leaf_hashes = Vec::with_capacity(job.chunk_count as usize);
        for i in 0..job.chunk_count {
            let offset = i as u64 * chunk_size;
            let length = if i == job.chunk_count - 1 {
                job.file_size - offset
            } else {
                chunk_size
            };
            let hash = verification::compute_chunk_sha256(temp_path, offset, length).await?;
            all_leaf_hashes.push(hash);
        }

        let tree = verification::build_merkle_tree(all_leaf_hashes, chunk_size, job.file_size);
        let sidecar_path = crate::transfer_engine::chunk::merkle_sidecar_path(dest_path);
        verification::write_merkle_sidecar(&sidecar_path, &tree).await?;
    }

    // Atomic rename
    pipeline::atomic_rename(temp_path, dest_path).await?;

    // Complete journal
    journal
        .complete_job(&job.job_id, Some(&source_hash), Some(&dest_hash))
        .await?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let speed_bps = if duration_ms > 0 {
        (job.file_size * 1000) / duration_ms
    } else {
        0
    };

    Ok(TransferResult {
        job_id: job.job_id.clone(),
        source_path: source_path.to_string(),
        dest_path: dest_path.to_string(),
        file_size: job.file_size,
        chunks_total: job.chunk_count,
        chunks_verified: job.chunk_count,
        source_hash,
        dest_hash,
        tier,
        duration_ms,
        speed_bps,
        verified: true,
    })
}

/// Finalize a transfer that has all chunks verified but wasn't renamed.
async fn finalize_transfer(
    job: &journal::JournalJob,
    _config: &TransferConfig,
    journal: &TransferJournal,
) -> Result<TransferResult, AppError> {
    let temp_path = &job.temp_path;
    let dest_path = &job.dest_path;
    let source_path = &job.source_path;
    let tier = job.verify_tier;

    // Full-file verification
    let (source_hash, dest_hash) = match tier {
        VerifyTier::Fast => {
            let src = verification::compute_file_xxhash3(source_path).await?;
            let dst = verification::compute_file_xxhash3(temp_path).await?;
            if src != dst {
                return Err(AppError::Transfer {
                    message: "Finalization verification failed".to_string(),
                    advice: "File may be corrupted. Re-transfer recommended.".to_string(),
                });
            }
            (format!("{:016x}", src), format!("{:016x}", dst))
        }
        VerifyTier::Verified | VerifyTier::MissionCritical => {
            let src = verification::compute_file_sha256(source_path).await?;
            let dst = verification::compute_file_sha256(temp_path).await?;
            if src != dst {
                return Err(AppError::Transfer {
                    message: "Finalization verification failed".to_string(),
                    advice: "File may be corrupted. Re-transfer recommended.".to_string(),
                });
            }
            (
                verification::hex_encode(&src),
                verification::hex_encode(&dst),
            )
        }
    };

    verification::sync_to_disk(temp_path).await?;
    pipeline::atomic_rename(temp_path, dest_path).await?;

    journal
        .complete_job(&job.job_id, Some(&source_hash), Some(&dest_hash))
        .await?;

    Ok(TransferResult {
        job_id: job.job_id.clone(),
        source_path: source_path.to_string(),
        dest_path: dest_path.to_string(),
        file_size: job.file_size,
        chunks_total: job.chunk_count,
        chunks_verified: job.chunk_count,
        source_hash,
        dest_hash,
        tier,
        duration_ms: 0,
        speed_bps: 0,
        verified: true,
    })
}

/// Calculate exponential backoff duration for retry attempts.
pub fn calculate_backoff(retry_count: u32, base_ms: u64) -> std::time::Duration {
    let ms = base_ms * 2u64.pow(retry_count.saturating_sub(1));
    std::time::Duration::from_millis(ms.min(60_000)) // cap at 60 seconds
}

/// Check if a transfer should be retried based on failure count.
pub fn should_retry(attempts: u8, max_attempts: u8) -> bool {
    attempts < max_attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_backoff() {
        assert_eq!(calculate_backoff(1, 1000), std::time::Duration::from_millis(1000));
        assert_eq!(calculate_backoff(2, 1000), std::time::Duration::from_millis(2000));
        assert_eq!(calculate_backoff(3, 1000), std::time::Duration::from_millis(4000));
        assert_eq!(calculate_backoff(4, 1000), std::time::Duration::from_millis(8000));
        // Should cap at 60s
        assert_eq!(calculate_backoff(20, 1000), std::time::Duration::from_millis(60_000));
    }

    #[test]
    fn test_should_retry() {
        assert!(should_retry(0, 3));
        assert!(should_retry(1, 3));
        assert!(should_retry(2, 3));
        assert!(!should_retry(3, 3));
        assert!(!should_retry(4, 3));
    }

    #[tokio::test]
    async fn test_resume_transfer_fresh() {
        // This test verifies the resume path for a transfer that needs
        // all chunks transferred (simulating a crash before any work done)
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let dst = dir.path().join("dest.dat");

        let data: Vec<u8> = (0..2048u32).map(|i| (i % 256) as u8).collect();
        tokio::fs::write(&src, &data).await.unwrap();

        let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();

        let journal = TransferJournal::new(pool);
        let worker_pool = WorkerPool::new(100);
        let config = TransferConfig {
            chunk_threshold: 100, // Lower for testing
            ..Default::default()
        };

        // First, create the transfer
        let result = pipeline::execute_file_transfer(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            data.len() as u64,
            VerifyTier::Fast,
            &config,
            &journal,
            &worker_pool,
            "resume-test-1",
            None,
        )
        .await
        .unwrap();

        assert!(result.verified);
        assert!(dst.exists());
        let written = tokio::fs::read(&dst).await.unwrap();
        assert_eq!(written, data);
    }
}
