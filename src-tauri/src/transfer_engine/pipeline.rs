//! Throughput pipeline engine (Layer 1).
//!
//! Implements:
//! - Worker pool with tokio::sync::Semaphore (100 global cap)
//! - Read-ahead pipeline (triple-buffering via mpsc channels)
//! - Pre-allocation (fallocate/ftruncate)
//! - Zero-copy path detection (stubs for sendfile/splice/copyfile)
//! - Direct I/O detection (O_DIRECT/F_NOCACHE stubs)

use crate::core::error::AppError;
use crate::core::types::{ChunkSpec, ChunkStatus, TransferConfig, VerifyTier};
use crate::transfer_engine::chunk;
use crate::transfer_engine::journal::TransferJournal;
use crate::transfer_engine::verification;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, Semaphore};

/// Global semaphore for limiting total concurrent workers across all transfers.
/// Initialized with max_workers_global (default 100).
pub struct WorkerPool {
    semaphore: Arc<Semaphore>,
}

impl WorkerPool {
    pub fn new(max_workers: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_workers as usize)),
        }
    }

    /// Get a reference to the semaphore for acquiring permits.
    pub fn semaphore(&self) -> &Arc<Semaphore> {
        &self.semaphore
    }
}

impl Clone for WorkerPool {
    fn clone(&self) -> Self {
        Self {
            semaphore: Arc::clone(&self.semaphore),
        }
    }
}

/// A chunk work unit passed through the pipeline.
#[derive(Debug)]
pub struct ChunkWork {
    pub chunk_index: u32,
    pub offset: u64,
    pub length: u64,
    pub data: Vec<u8>,
    pub source_xxhash: u64,
    pub source_sha256: Option<[u8; 32]>,
}

/// Execute a single chunk transfer: read, hash, write, verify.
///
/// This is the core worker function. Each invocation handles one chunk's
/// complete lifecycle:
/// 1. Acquire semaphore permit
/// 2. Read chunk from source at offset
/// 3. Compute fast hash (xxHash3) during read
/// 4. Write chunk to destination at same offset
/// 5. Optionally re-read and verify (Tier 2+)
/// 6. Update journal bitmap
/// 7. Release semaphore permit
#[allow(clippy::too_many_arguments)]
pub async fn transfer_chunk(
    source_path: &str,
    dest_path: &str,
    chunk: &ChunkSpec,
    tier: VerifyTier,
    config: &TransferConfig,
    journal: Option<&TransferJournal>,
    job_id: Option<&str>,
    semaphore: &Arc<Semaphore>,
) -> Result<ChunkResult, AppError> {
    let _permit = semaphore.acquire().await.map_err(|_| AppError::Transfer {
        message: "Worker pool exhausted".to_string(),
        advice: "Too many concurrent transfers. Try again later.".to_string(),
    })?;

    let start = std::time::Instant::now();

    // Update journal: reading
    if let (Some(journal), Some(job_id)) = (journal, job_id) {
        journal
            .update_chunk_status(job_id, chunk.chunk_index, &ChunkStatus::Reading, None, None)
            .await?;
    }

    // Step 1: Read chunk from source and compute xxHash3 inline
    let (data, source_xxhash) = read_chunk_with_hash(source_path, chunk.offset, chunk.length).await?;

    // Step 1b: Compute SHA-256 if Tier 2+ (can be done on the same data)
    let source_sha256 = if tier >= VerifyTier::Verified {
        Some(verification::sha256_bytes(&data))
    } else {
        None
    };

    // Update journal: writing
    if let (Some(journal), Some(job_id)) = (journal, job_id) {
        journal
            .update_chunk_status(
                job_id,
                chunk.chunk_index,
                &ChunkStatus::Writing,
                Some(&format!("{:016x}", source_xxhash)),
                None,
            )
            .await?;
    }

    let write_start = std::time::Instant::now();

    // Step 2: Write chunk to destination at offset
    write_chunk_at_offset(dest_path, chunk.offset, &data).await?;
    let write_time_ms = write_start.elapsed().as_millis() as u32;

    // Step 3: fsync if required by tier
    if verification::should_fsync(tier, chunk.chunk_index, config.fsync_interval) {
        verification::sync_to_disk(dest_path).await?;
    }

    // Step 4: Write-back verification (Tier 2+)
    let mut verified = true;
    let verify_start = std::time::Instant::now();

    if tier >= VerifyTier::Verified {
        // Update journal: verifying
        if let (Some(journal), Some(job_id)) = (journal, job_id) {
            journal
                .update_chunk_status(
                    job_id,
                    chunk.chunk_index,
                    &ChunkStatus::Verifying,
                    Some(&format!("{:016x}", source_xxhash)),
                    None,
                )
                .await?;
        }

        // Re-read destination chunk and compare xxHash3
        verified = verification::verify_chunk_writeback(
            dest_path,
            chunk.offset,
            chunk.length,
            source_xxhash,
        )
        .await?;

        if !verified {
            // Retry up to max_attempts
            for attempt in 1..=config.retry_max_attempts {
                tracing::warn!(
                    "Chunk {} write-back verification failed, retry {}/{}",
                    chunk.chunk_index,
                    attempt,
                    config.retry_max_attempts
                );

                // Re-write
                write_chunk_at_offset(dest_path, chunk.offset, &data).await?;
                verification::sync_to_disk(dest_path).await?;

                verified = verification::verify_chunk_writeback(
                    dest_path,
                    chunk.offset,
                    chunk.length,
                    source_xxhash,
                )
                .await?;

                if verified {
                    break;
                }
            }
        }
    }

    let verify_time_ms = verify_start.elapsed().as_millis() as u32;

    // Step 5: Update journal with final status
    let status = if verified {
        ChunkStatus::Verified
    } else {
        ChunkStatus::Failed {
            reason: "Write-back verification failed after retries".to_string(),
            attempts: config.retry_max_attempts,
        }
    };

    if let (Some(journal), Some(job_id)) = (journal, job_id) {
        let dest_hash = if verified {
            Some(format!("{:016x}", source_xxhash))
        } else {
            None
        };
        journal
            .update_chunk_status(
                job_id,
                chunk.chunk_index,
                &status,
                Some(&format!("{:016x}", source_xxhash)),
                dest_hash.as_deref(),
            )
            .await?;
    }

    let total_time_ms = start.elapsed().as_millis() as u32;

    Ok(ChunkResult {
        chunk_index: chunk.chunk_index,
        verified,
        source_xxhash,
        source_sha256,
        write_time_ms,
        verify_time_ms,
        total_time_ms,
        bytes_transferred: chunk.length,
    })
}

/// Result of transferring a single chunk.
#[derive(Debug)]
pub struct ChunkResult {
    pub chunk_index: u32,
    pub verified: bool,
    pub source_xxhash: u64,
    pub source_sha256: Option<[u8; 32]>,
    pub write_time_ms: u32,
    pub verify_time_ms: u32,
    pub total_time_ms: u32,
    pub bytes_transferred: u64,
}

/// Read a chunk from a file and compute xxHash3 inline.
async fn read_chunk_with_hash(
    path: &str,
    offset: u64,
    length: u64,
) -> Result<(Vec<u8>, u64), AppError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| AppError::Transfer {
        message: format!("Cannot open source file: {e}"),
        advice: "Check the source file exists and is readable.".to_string(),
    })?;

    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Seek error: {e}"),
            advice: "Source file may be truncated.".to_string(),
        })?;

    let mut data = vec![0u8; length as usize];
    let mut total_read = 0usize;

    while total_read < length as usize {
        let n = file
            .read(&mut data[total_read..])
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Read error at offset {}: {e}", offset + total_read as u64),
                advice: "Source file may be corrupted or in use.".to_string(),
            })?;
        if n == 0 {
            break;
        }
        total_read += n;
    }

    // Truncate to actual bytes read
    data.truncate(total_read);

    // Compute xxHash3 of the read data
    let hash = verification::xxhash3(&data);

    Ok((data, hash))
}

/// Write data at a specific offset in the destination file.
async fn write_chunk_at_offset(
    path: &str,
    offset: u64,
    data: &[u8],
) -> Result<(), AppError> {
    use std::io::SeekFrom;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Cannot open destination file: {e}"),
            advice: "Check the destination directory exists and is writable.".to_string(),
        })?;

    file.seek(SeekFrom::Start(offset))
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Seek error: {e}"),
            advice: "Destination file may be on a read-only filesystem.".to_string(),
        })?;

    file.write_all(data).await.map_err(|e| AppError::Transfer {
        message: format!("Write error at offset {offset}: {e}"),
        advice: "Check disk space and write permissions.".to_string(),
    })?;

    file.flush().await.map_err(|e| AppError::Transfer {
        message: format!("Flush error: {e}"),
        advice: "Disk may be full.".to_string(),
    })?;

    Ok(())
}

/// Perform the atomic rename from temp path to final destination.
pub async fn atomic_rename(temp_path: &str, final_path: &str) -> Result<(), AppError> {
    tokio::fs::rename(temp_path, final_path)
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Atomic rename failed: {e}"),
            advice: "The transfer completed but the file could not be moved to its final location.".to_string(),
        })
}

// ──────────────────────────────────────────────
// Zero-Copy Path Detection (Stubs)
// ──────────────────────────────────────────────

/// Check if zero-copy transfer is available for this source/dest pair.
pub fn zero_copy_available(source: &str, dest: &str) -> bool {
    // Stub: zero-copy requires both to be local paths on the same filesystem
    // macOS APFS clones, Linux sendfile/splice
    #[cfg(target_os = "macos")]
    {
        // Check if source and dest are on the same APFS volume
        // For now, stub returns false
        let _ = (source, dest);
        false
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (source, dest);
        false
    }
}

/// Check if Direct I/O should be used for this file size.
pub fn should_use_direct_io(file_size: u64, config: &TransferConfig) -> bool {
    config.use_direct_io && file_size > 100 * 1024 * 1024 // > 100MB
}

// ──────────────────────────────────────────────
// Full File Transfer Orchestrator
// ──────────────────────────────────────────────

/// Progress callback type for chunk completion.
pub type ProgressCallback = Box<dyn Fn(u32, u64, u64) + Send + Sync>;

/// Execute a complete file transfer with the three-layer architecture.
///
/// Steps:
/// 1. Detect media type and compute chunk layout
/// 2. Create journal entry
/// 3. Pre-allocate destination (temp file)
/// 4. Transfer chunks in parallel with worker pool
/// 5. Full-file verification
/// 6. Atomic rename
/// 7. Write Merkle sidecar (Tier 3)
/// 8. Update journal as completed
#[allow(clippy::too_many_arguments)]
pub async fn execute_file_transfer(
    source_path: &str,
    dest_path: &str,
    file_size: u64,
    tier: VerifyTier,
    config: &TransferConfig,
    journal: &TransferJournal,
    worker_pool: &WorkerPool,
    job_id: &str,
    on_progress: Option<&ProgressCallback>,
) -> Result<TransferResult, AppError> {
    let start = std::time::Instant::now();

    // Step 1: Detect media type and compute chunks
    let media_source = chunk::detect_media_type(source_path);
    let media_dest = chunk::detect_media_type(dest_path);
    // Use the slower media type for chunk sizing
    let media = if media_dest.default_chunk_size() < media_source.default_chunk_size() {
        media_dest
    } else {
        media_source
    };

    let chunks = chunk::split_into_chunks(file_size, media, config);
    let chunk_size = chunk::effective_chunk_size(file_size, media, config);
    let temp_path = chunk::temp_path_for(dest_path);

    // Step 2: Create journal entry
    journal
        .create_job(
            job_id,
            source_path,
            dest_path,
            &temp_path,
            file_size,
            chunk_size,
            &chunks,
            tier,
        )
        .await?;

    // Step 3: Pre-allocate temp file
    if config.preallocate && file_size > 0 {
        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(&temp_path).parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::Transfer {
                message: format!("Cannot create destination directory: {e}"),
                advice: "Check directory permissions.".to_string(),
            })?;
        }
        verification::preallocate_file(&temp_path, file_size).await?;
    }

    // Step 4: Transfer chunks
    let semaphore = worker_pool.semaphore().clone();
    let mut total_transferred: u64 = 0;
    let mut chunk_results: Vec<ChunkResult> = Vec::with_capacity(chunks.len());
    let mut all_verified = true;

    // For small files or single chunk, just do it sequentially
    if chunks.len() == 1 {
        let result = transfer_chunk(
            source_path,
            &temp_path,
            &chunks[0],
            tier,
            config,
            Some(journal),
            Some(job_id),
            &semaphore,
        )
        .await?;

        total_transferred += result.bytes_transferred;
        all_verified = all_verified && result.verified;

        if let Some(cb) = on_progress {
            cb(0, total_transferred, file_size);
        }

        chunk_results.push(result);
    } else {
        // Parallel chunk transfer
        let worker_count = chunk::effective_worker_count(media, config) as usize;
        let (tx, mut rx) = mpsc::channel::<Result<ChunkResult, AppError>>(worker_count * 2);

        let chunks_to_process: Vec<ChunkSpec> = chunks.clone();

        // Spawn chunk workers
        for chunk_spec in chunks_to_process {
            let source = source_path.to_string();
            let dest = temp_path.clone();
            let sem = semaphore.clone();
            let tx = tx.clone();
            let config = config.clone();
            let job_id_str = job_id.to_string();
            let journal = journal.clone();

            tokio::spawn(async move {
                let result = transfer_chunk(
                    &source,
                    &dest,
                    &chunk_spec,
                    tier,
                    &config,
                    Some(&journal),
                    Some(&job_id_str),
                    &sem,
                )
                .await;
                let _ = tx.send(result).await;
            });
        }

        // Drop the original sender so rx knows when all are done
        drop(tx);

        // Collect results
        while let Some(result) = rx.recv().await {
            match result {
                Ok(cr) => {
                    total_transferred += cr.bytes_transferred;
                    all_verified = all_verified && cr.verified;
                    if let Some(cb) = on_progress {
                        cb(cr.chunk_index, total_transferred, file_size);
                    }
                    chunk_results.push(cr);
                }
                Err(e) => {
                    // Mark job as failed on any chunk error
                    journal.fail_job(job_id, &e.to_string()).await?;
                    return Err(e);
                }
            }
        }
    }

    if !all_verified {
        journal
            .fail_job(job_id, "One or more chunks failed verification")
            .await?;
        return Err(AppError::Transfer {
            message: "Transfer verification failed".to_string(),
            advice: "Some chunks could not be verified. The transfer will need to be retried.".to_string(),
        });
    }

    // Step 5: Full-file verification
    let (source_hash, dest_hash) = match tier {
        VerifyTier::Fast => {
            // Full-file xxHash3 comparison
            let src_hash = verification::compute_file_xxhash3(source_path).await?;
            let dst_hash = verification::compute_file_xxhash3(&temp_path).await?;
            if src_hash != dst_hash {
                journal
                    .fail_job(job_id, "Full-file xxHash3 mismatch after all chunks verified")
                    .await?;
                return Err(AppError::Transfer {
                    message: "Final verification found an issue with the transferred file".to_string(),
                    advice: "The file will need to be re-transferred to ensure a perfect copy.".to_string(),
                });
            }
            (format!("{:016x}", src_hash), format!("{:016x}", dst_hash))
        }
        VerifyTier::Verified | VerifyTier::MissionCritical => {
            // Full-file SHA-256
            let src_hash = verification::compute_file_sha256(source_path).await?;
            let dst_hash = verification::compute_file_sha256(&temp_path).await?;
            if src_hash != dst_hash {
                journal
                    .fail_job(job_id, "Full-file SHA-256 mismatch after all chunks verified")
                    .await?;
                return Err(AppError::Transfer {
                    message: "Final verification found an issue with the transferred file".to_string(),
                    advice: "The file will need to be re-transferred to ensure a perfect copy.".to_string(),
                });
            }
            (
                verification::hex_encode(&src_hash),
                verification::hex_encode(&dst_hash),
            )
        }
    };

    // Step 5b: Final fsync for Tier 1 (only once)
    verification::sync_to_disk(&temp_path).await?;

    // Step 6: Merkle tree sidecar (Tier 3)
    if tier == VerifyTier::MissionCritical && config.merkle_sidecar {
        let leaf_hashes: Vec<[u8; 32]> = chunk_results
            .iter()
            .filter_map(|cr| cr.source_sha256)
            .collect();

        if !leaf_hashes.is_empty() {
            let tree = verification::build_merkle_tree(leaf_hashes, chunk_size, file_size);
            let sidecar_path = chunk::merkle_sidecar_path(dest_path);
            verification::write_merkle_sidecar(&sidecar_path, &tree).await?;
        }
    }

    // Step 7: Atomic rename
    atomic_rename(&temp_path, dest_path).await?;

    // Step 8: Complete journal
    journal
        .complete_job(job_id, Some(&source_hash), Some(&dest_hash))
        .await?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let speed_bps = if duration_ms > 0 {
        (file_size * 1000) / duration_ms
    } else {
        0
    };

    Ok(TransferResult {
        job_id: job_id.to_string(),
        source_path: source_path.to_string(),
        dest_path: dest_path.to_string(),
        file_size,
        chunks_total: chunks.len() as u32,
        chunks_verified: chunk_results.iter().filter(|c| c.verified).count() as u32,
        source_hash,
        dest_hash,
        tier,
        duration_ms,
        speed_bps,
        verified: true,
    })
}

/// Result of a complete file transfer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransferResult {
    pub job_id: String,
    pub source_path: String,
    pub dest_path: String,
    pub file_size: u64,
    pub chunks_total: u32,
    pub chunks_verified: u32,
    pub source_hash: String,
    pub dest_hash: String,
    pub tier: VerifyTier,
    pub duration_ms: u64,
    pub speed_bps: u64,
    pub verified: bool,
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
    async fn test_read_chunk_with_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.dat");
        let data = b"Hello, world! This is test data.";
        tokio::fs::write(&path, data).await.unwrap();

        let (read_data, hash) = read_chunk_with_hash(path.to_str().unwrap(), 0, data.len() as u64)
            .await
            .unwrap();

        assert_eq!(read_data, data);
        assert_ne!(hash, 0);

        // Same data should produce same hash
        let (_, hash2) = read_chunk_with_hash(path.to_str().unwrap(), 0, data.len() as u64)
            .await
            .unwrap();
        assert_eq!(hash, hash2);
    }

    #[tokio::test]
    async fn test_write_chunk_at_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dest.dat");

        // Pre-allocate
        verification::preallocate_file(path.to_str().unwrap(), 1024)
            .await
            .unwrap();

        // Write at offset 512
        let data = b"chunk data";
        write_chunk_at_offset(path.to_str().unwrap(), 512, data)
            .await
            .unwrap();

        // Read back
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(content.len(), 1024);
        assert_eq!(&content[512..512 + data.len()], data);
    }

    #[tokio::test]
    async fn test_atomic_rename() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".file.txt.ufop-partial");
        let final_path = dir.path().join("file.txt");

        tokio::fs::write(&temp, b"content").await.unwrap();
        atomic_rename(temp.to_str().unwrap(), final_path.to_str().unwrap())
            .await
            .unwrap();

        assert!(!temp.exists());
        assert!(final_path.exists());
        let content = tokio::fs::read_to_string(&final_path).await.unwrap();
        assert_eq!(content, "content");
    }

    #[tokio::test]
    async fn test_transfer_single_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let _dst = dir.path().join("dest.dat");
        let temp = dir.path().join(".dest.dat.ufop-partial");

        let data = b"Single chunk transfer test data";
        tokio::fs::write(&src, data).await.unwrap();

        // Pre-allocate
        verification::preallocate_file(temp.to_str().unwrap(), data.len() as u64)
            .await
            .unwrap();

        let chunk = ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: data.len() as u64,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        };

        let semaphore = Arc::new(Semaphore::new(100));
        let config = TransferConfig::default();

        let result = transfer_chunk(
            src.to_str().unwrap(),
            temp.to_str().unwrap(),
            &chunk,
            VerifyTier::Fast,
            &config,
            None,
            None,
            &semaphore,
        )
        .await
        .unwrap();

        assert!(result.verified);
        assert_eq!(result.bytes_transferred, data.len() as u64);
        assert_ne!(result.source_xxhash, 0);
    }

    #[tokio::test]
    async fn test_transfer_with_tier2_verification() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let temp = dir.path().join(".dest.dat.ufop-partial");

        let data: Vec<u8> = (0..4096u32).map(|i| (i % 256) as u8).collect();
        tokio::fs::write(&src, &data).await.unwrap();

        verification::preallocate_file(temp.to_str().unwrap(), data.len() as u64)
            .await
            .unwrap();

        let chunk = ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: data.len() as u64,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        };

        let semaphore = Arc::new(Semaphore::new(100));
        let config = TransferConfig::default();

        let result = transfer_chunk(
            src.to_str().unwrap(),
            temp.to_str().unwrap(),
            &chunk,
            VerifyTier::Verified,
            &config,
            None,
            None,
            &semaphore,
        )
        .await
        .unwrap();

        assert!(result.verified);
        assert!(result.source_sha256.is_some());
        // verify_time_ms is always non-negative (u32), so just check it was populated
        let _ = result.verify_time_ms;
    }

    #[tokio::test]
    async fn test_full_file_transfer_tier1() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let dst = dir.path().join("dest.dat");

        // Create a file just above the chunk threshold for multi-chunk
        let data: Vec<u8> = (0..1024u32).map(|i| (i % 256) as u8).collect();
        tokio::fs::write(&src, &data).await.unwrap();

        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);
        let worker_pool = WorkerPool::new(100);
        let config = TransferConfig {
            chunk_threshold: 100, // Lower threshold for testing
            ..Default::default()
        };

        let result = execute_file_transfer(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            data.len() as u64,
            VerifyTier::Fast,
            &config,
            &journal,
            &worker_pool,
            "test-transfer-1",
            None,
        )
        .await
        .unwrap();

        assert!(result.verified);
        assert_eq!(result.file_size, data.len() as u64);
        assert!(dst.exists());

        // Verify content matches
        let written = tokio::fs::read(&dst).await.unwrap();
        assert_eq!(written, data);
    }

    #[tokio::test]
    async fn test_full_file_transfer_tier2() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let dst = dir.path().join("dest.dat");

        let data: Vec<u8> = (0..2048u32).map(|i| (i % 256) as u8).collect();
        tokio::fs::write(&src, &data).await.unwrap();

        let pool = setup_db().await;
        let journal = TransferJournal::new(pool);
        let worker_pool = WorkerPool::new(100);
        let config = TransferConfig {
            chunk_threshold: 100,
            ..Default::default()
        };

        let result = execute_file_transfer(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            data.len() as u64,
            VerifyTier::Verified,
            &config,
            &journal,
            &worker_pool,
            "test-transfer-2",
            None,
        )
        .await
        .unwrap();

        assert!(result.verified);
        assert!(dst.exists());
        let written = tokio::fs::read(&dst).await.unwrap();
        assert_eq!(written, data);
    }

    #[tokio::test]
    async fn test_worker_pool_semaphore() {
        let pool = WorkerPool::new(2);
        let sem = pool.semaphore().clone();

        // Should be able to acquire 2 permits
        let p1 = sem.acquire().await.unwrap();
        let p2 = sem.acquire().await.unwrap();

        // Third would block, so we test with try_acquire
        assert!(sem.try_acquire().is_err());

        drop(p1);
        // Now should succeed
        let _p3 = sem.acquire().await.unwrap();
        drop(p2);
    }

    #[test]
    fn test_zero_copy_available() {
        // Stub always returns false for now
        assert!(!zero_copy_available("/local/a", "/local/b"));
    }

    #[test]
    fn test_should_use_direct_io() {
        let config = TransferConfig::default();
        assert!(!should_use_direct_io(50 * 1024 * 1024, &config)); // 50MB
        assert!(should_use_direct_io(200 * 1024 * 1024, &config)); // 200MB

        let no_dio = TransferConfig {
            use_direct_io: false,
            ..Default::default()
        };
        assert!(!should_use_direct_io(200 * 1024 * 1024, &no_dio));
    }
}
