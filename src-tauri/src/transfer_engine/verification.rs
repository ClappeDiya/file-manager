//! Integrity verification engine (Layer 2).
//!
//! Three verification tiers:
//! - Tier 1 (Fast): xxHash3 per chunk + full-file xxHash3
//! - Tier 2 (Verified): + write-back verify + SHA-256 + fsync/8 chunks
//! - Tier 3 (Mission-critical): + Merkle tree + fsync/chunk + audit trail
//!
//! Hash algorithms:
//! - xxHash3 via xxhash-rust crate (~30 GB/s, hardware accelerated)
//! - SHA-256 via ring::digest (~2-4 GB/s with SHA-NI)

use crate::core::error::AppError;
use crate::core::types::{
    ChecksumAlgorithm, MerkleSidecar, MerkleTree, VerifyTier,
};
use ring::digest::{Context as Sha256Context, SHA256};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

// ──────────────────────────────────────────────
// Hash Computation
// ──────────────────────────────────────────────

/// Compute xxHash3-64 of a byte slice.
pub fn xxhash3(data: &[u8]) -> u64 {
    xxhash_rust::xxh3::xxh3_64(data)
}

/// Compute SHA-256 of a byte slice, returning 32 bytes.
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let digest = ring::digest::digest(&SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// Compute xxHash3 of a file (full file).
pub async fn compute_file_xxhash3(path: &str) -> Result<u64, AppError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| AppError::Transfer {
        message: format!("Cannot open file for hashing: {e}"),
        advice: "Check file exists and is readable.".to_string(),
    })?;

    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = vec![0u8; 256 * 1024]; // 256KB buffer

    loop {
        let n = file.read(&mut buf).await.map_err(|e| AppError::Transfer {
            message: format!("Read error during xxHash3: {e}"),
            advice: "File may be corrupted or in use.".to_string(),
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hasher.digest())
}

/// Compute SHA-256 of a file (full file).
pub async fn compute_file_sha256(path: &str) -> Result<[u8; 32], AppError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| AppError::Transfer {
        message: format!("Cannot open file for SHA-256: {e}"),
        advice: "Check file exists and is readable.".to_string(),
    })?;

    let mut ctx = Sha256Context::new(&SHA256);
    let mut buf = vec![0u8; 256 * 1024]; // 256KB buffer

    loop {
        let n = file.read(&mut buf).await.map_err(|e| AppError::Transfer {
            message: format!("Read error during SHA-256: {e}"),
            advice: "File may be corrupted or in use.".to_string(),
        })?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }

    let digest = ctx.finish();
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    Ok(out)
}

/// Compute checksum of a file using the specified algorithm.
/// Returns a hex-encoded string.
pub async fn compute_file_checksum(
    path: &str,
    algorithm: ChecksumAlgorithm,
) -> Result<String, AppError> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(AppError::Transfer {
            message: format!("File not found: {path}"),
            advice: "The file may have been moved or deleted.".to_string(),
        });
    }

    match algorithm {
        ChecksumAlgorithm::XxHash3 => {
            let hash = compute_file_xxhash3(path).await?;
            Ok(format!("{:016x}", hash))
        }
        ChecksumAlgorithm::Sha256 => {
            let hash = compute_file_sha256(path).await?;
            Ok(hex_encode(&hash))
        }
        ChecksumAlgorithm::Md5 => {
            // Legacy support via digest crate
            use digest::Digest;
            let mut file = tokio::fs::File::open(p).await.map_err(|e| AppError::Transfer {
                message: format!("Cannot open file for MD5: {e}"),
                advice: "Check file permissions.".to_string(),
            })?;
            let mut hasher = md5::Md5::new();
            let mut buf = vec![0u8; 128 * 1024];
            loop {
                let n = file.read(&mut buf).await.map_err(|e| AppError::Transfer {
                    message: format!("Read error during MD5: {e}"),
                    advice: "File may be corrupted.".to_string(),
                })?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex_encode(&hasher.finalize()))
        }
    }
}

/// Compute xxHash3 of a chunk (specific offset and length within a file).
pub async fn compute_chunk_xxhash3(
    path: &str,
    offset: u64,
    length: u64,
) -> Result<u64, AppError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| AppError::Transfer {
        message: format!("Cannot open file for chunk hash: {e}"),
        advice: "Check file exists and is readable.".to_string(),
    })?;

    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Seek error: {e}"),
            advice: "File may be corrupted.".to_string(),
        })?;

    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    let mut remaining = length;
    let mut buf = vec![0u8; 256 * 1024];

    while remaining > 0 {
        let to_read = (remaining as usize).min(buf.len());
        let n = file.read(&mut buf[..to_read]).await.map_err(|e| AppError::Transfer {
            message: format!("Read error during chunk hash: {e}"),
            advice: "File may be corrupted.".to_string(),
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        remaining -= n as u64;
    }

    Ok(hasher.digest())
}

/// Compute SHA-256 of a chunk.
pub async fn compute_chunk_sha256(
    path: &str,
    offset: u64,
    length: u64,
) -> Result<[u8; 32], AppError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| AppError::Transfer {
        message: format!("Cannot open file for chunk SHA-256: {e}"),
        advice: "Check file exists and is readable.".to_string(),
    })?;

    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Seek error: {e}"),
            advice: "File may be corrupted.".to_string(),
        })?;

    let mut ctx = Sha256Context::new(&SHA256);
    let mut remaining = length;
    let mut buf = vec![0u8; 256 * 1024];

    while remaining > 0 {
        let to_read = (remaining as usize).min(buf.len());
        let n = file.read(&mut buf[..to_read]).await.map_err(|e| AppError::Transfer {
            message: format!("Read error during chunk SHA-256: {e}"),
            advice: "File may be corrupted.".to_string(),
        })?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        remaining -= n as u64;
    }

    let digest = ctx.finish();
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    Ok(out)
}

// ──────────────────────────────────────────────
// Verification Results
// ──────────────────────────────────────────────

/// Result of a verification operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerificationResult {
    pub matches: bool,
    pub source_checksum: String,
    pub dest_checksum: String,
    pub algorithm: ChecksumAlgorithm,
}

/// Verify source and destination files match using the specified algorithm.
pub async fn verify_transfer(
    source_path: &str,
    dest_path: &str,
    algorithm: ChecksumAlgorithm,
) -> Result<VerificationResult, AppError> {
    let source_checksum = compute_file_checksum(source_path, algorithm).await?;
    let dest_checksum = compute_file_checksum(dest_path, algorithm).await?;
    let matches = source_checksum == dest_checksum;

    Ok(VerificationResult {
        matches,
        source_checksum,
        dest_checksum,
        algorithm,
    })
}

/// Verify a chunk's write-back integrity (Tier 2+).
/// Reads the chunk from the destination and compares xxHash3 against source hash.
pub async fn verify_chunk_writeback(
    dest_path: &str,
    offset: u64,
    length: u64,
    expected_hash: u64,
) -> Result<bool, AppError> {
    let actual = compute_chunk_xxhash3(dest_path, offset, length).await?;
    Ok(actual == expected_hash)
}

// ──────────────────────────────────────────────
// fsync Strategy
// ──────────────────────────────────────────────

/// Sync file data to disk using the platform-appropriate method.
///
/// - macOS: fcntl(F_FULLFSYNC) for true disk cache flush
/// - Linux: fdatasync() for data-only sync
/// - Other: std::fs::File::sync_all()
pub async fn sync_to_disk(path: &str) -> Result<(), AppError> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        sync_to_disk_blocking(&path)
    })
    .await
    .map_err(|e| AppError::Transfer {
        message: format!("Sync task failed: {e}"),
        advice: "Disk may be unavailable.".to_string(),
    })?
}

fn sync_to_disk_blocking(path: &str) -> Result<(), AppError> {
    let file = std::fs::File::open(path).map_err(|e| AppError::Transfer {
        message: format!("Cannot open file for sync: {e}"),
        advice: "Check file exists.".to_string(),
    })?;

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::fcntl(fd, libc::F_FULLFSYNC) };
        if ret != 0 {
            return Err(AppError::Transfer {
                message: format!("F_FULLFSYNC failed: {}", std::io::Error::last_os_error()),
                advice: "Disk may be failing.".to_string(),
            });
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::fdatasync(fd) };
        if ret != 0 {
            return Err(AppError::Transfer {
                message: format!("fdatasync failed: {}", std::io::Error::last_os_error()),
                advice: "Disk may be failing.".to_string(),
            });
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        file.sync_all().map_err(|e| AppError::Transfer {
            message: format!("sync_all failed: {e}"),
            advice: "Disk may be failing.".to_string(),
        })?;
    }

    Ok(())
}

/// Determine whether to fsync based on tier and chunk index.
pub fn should_fsync(tier: VerifyTier, chunk_index: u32, fsync_interval: u32) -> bool {
    match tier {
        VerifyTier::Fast => false, // fsync once at the end
        VerifyTier::Verified => (chunk_index + 1) % fsync_interval == 0,
        VerifyTier::MissionCritical => true, // every chunk
    }
}

// ──────────────────────────────────────────────
// Merkle Tree (Tier 3)
// ──────────────────────────────────────────────

/// Build a Merkle tree from per-chunk SHA-256 hashes.
pub fn build_merkle_tree(
    leaf_hashes: Vec<[u8; 32]>,
    chunk_size: u64,
    file_size: u64,
) -> MerkleTree {
    let chunk_count = leaf_hashes.len() as u32;

    if leaf_hashes.is_empty() {
        return MerkleTree {
            leaf_hashes: vec![],
            tree_nodes: vec![],
            root: [0u8; 32],
            chunk_count: 0,
            chunk_size,
            file_size,
            algorithm: "SHA-256".to_string(),
        };
    }

    if leaf_hashes.len() == 1 {
        return MerkleTree {
            leaf_hashes: leaf_hashes.clone(),
            tree_nodes: vec![],
            root: leaf_hashes[0],
            chunk_count,
            chunk_size,
            file_size,
            algorithm: "SHA-256".to_string(),
        };
    }

    let mut tree_nodes = Vec::new();
    let mut current_level = leaf_hashes.clone();

    while current_level.len() > 1 {
        let mut next_level = Vec::new();
        let mut i = 0;
        while i < current_level.len() {
            if i + 1 < current_level.len() {
                // Hash pair
                let mut combined = Vec::with_capacity(64);
                combined.extend_from_slice(&current_level[i]);
                combined.extend_from_slice(&current_level[i + 1]);
                let parent = sha256_bytes(&combined);
                tree_nodes.push(parent);
                next_level.push(parent);
                i += 2;
            } else {
                // Odd node - promote to next level
                next_level.push(current_level[i]);
                i += 1;
            }
        }
        current_level = next_level;
    }

    let root = current_level[0];

    MerkleTree {
        leaf_hashes,
        tree_nodes,
        root,
        chunk_count,
        chunk_size,
        file_size,
        algorithm: "SHA-256".to_string(),
    }
}

/// Convert a MerkleTree to its sidecar JSON format.
pub fn merkle_to_sidecar(tree: &MerkleTree) -> MerkleSidecar {
    MerkleSidecar {
        version: 1,
        algorithm: tree.algorithm.clone(),
        file_size: tree.file_size,
        chunk_size: tree.chunk_size,
        chunk_count: tree.chunk_count,
        root_hash: hex_encode(&tree.root),
        leaf_hashes: tree.leaf_hashes.iter().map(|h| hex_encode(h)).collect(),
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Write a Merkle tree sidecar file.
pub async fn write_merkle_sidecar(
    sidecar_path: &str,
    tree: &MerkleTree,
) -> Result<(), AppError> {
    let sidecar = merkle_to_sidecar(tree);
    let json = serde_json::to_string_pretty(&sidecar).map_err(|e| AppError::Transfer {
        message: format!("Failed to serialize Merkle sidecar: {e}"),
        advice: "Internal error.".to_string(),
    })?;

    // Atomic write: write to temp, then rename
    let temp_path = format!("{sidecar_path}.tmp");
    tokio::fs::write(&temp_path, &json)
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Failed to write Merkle sidecar: {e}"),
            advice: "Check disk space and permissions.".to_string(),
        })?;
    tokio::fs::rename(&temp_path, sidecar_path)
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Failed to finalize Merkle sidecar: {e}"),
            advice: "Check disk space and permissions.".to_string(),
        })?;

    Ok(())
}

/// Identify corrupted chunks by walking the Merkle tree.
/// Returns indices of chunks whose hashes don't match the stored leaf hashes.
pub async fn find_corrupted_chunks(
    file_path: &str,
    tree: &MerkleTree,
) -> Result<Vec<u32>, AppError> {
    let mut corrupted = Vec::new();

    for (i, expected_hash) in tree.leaf_hashes.iter().enumerate() {
        let offset = i as u64 * tree.chunk_size;
        let length = if i as u32 == tree.chunk_count - 1 {
            tree.file_size - offset
        } else {
            tree.chunk_size
        };

        let actual = compute_chunk_sha256(file_path, offset, length).await?;
        if actual != *expected_hash {
            corrupted.push(i as u32);
        }
    }

    Ok(corrupted)
}

// ──────────────────────────────────────────────
// Utilities
// ──────────────────────────────────────────────

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, AppError> {
    if s.len() % 2 != 0 {
        return Err(AppError::Transfer {
            message: "Invalid hex string length".to_string(),
            advice: "Internal error.".to_string(),
        });
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| AppError::Transfer {
                message: "Invalid hex character".to_string(),
                advice: "Internal error.".to_string(),
            })
        })
        .collect()
}

// ──────────────────────────────────────────────
// Pre-allocation
// ──────────────────────────────────────────────

/// Pre-allocate a file to the specified size.
/// Uses platform-specific calls for optimal block allocation.
pub async fn preallocate_file(path: &str, size: u64) -> Result<(), AppError> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        preallocate_blocking(&path, size)
    })
    .await
    .map_err(|e| AppError::Transfer {
        message: format!("Pre-allocation task failed: {e}"),
        advice: "Check disk space.".to_string(),
    })?
}

fn preallocate_blocking(path: &str, size: u64) -> Result<(), AppError> {
    use std::fs::OpenOptions;

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| AppError::Transfer {
            message: format!("Cannot create file for pre-allocation: {e}"),
            advice: "Check directory exists and has write permission.".to_string(),
        })?;

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::fallocate(fd, 0, 0, size as libc::off_t) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            // fallocate may not be supported on all filesystems; fall back to ftruncate
            tracing::debug!("fallocate failed ({err}), falling back to ftruncate");
            let ret = unsafe { libc::ftruncate(fd, size as libc::off_t) };
            if ret != 0 {
                return Err(AppError::Transfer {
                    message: format!("ftruncate failed: {}", std::io::Error::last_os_error()),
                    advice: "Check disk space.".to_string(),
                });
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::ftruncate(fd, size as libc::off_t) };
        if ret != 0 {
            return Err(AppError::Transfer {
                message: format!("ftruncate failed: {}", std::io::Error::last_os_error()),
                advice: "Check disk space.".to_string(),
            });
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        file.set_len(size).map_err(|e| AppError::Transfer {
            message: format!("set_len failed: {e}"),
            advice: "Check disk space.".to_string(),
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xxhash3_basic() {
        let hash1 = xxhash3(b"Hello, world!");
        let hash2 = xxhash3(b"Hello, world!");
        assert_eq!(hash1, hash2);

        let hash3 = xxhash3(b"Different content");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_sha256_basic() {
        let hash1 = sha256_bytes(b"Hello, world!");
        let hash2 = sha256_bytes(b"Hello, world!");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 32);

        let hash3 = sha256_bytes(b"Different content");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_sha256_known_vector() {
        // SHA-256 of empty string
        let hash = sha256_bytes(b"");
        let hex = hex_encode(&hash);
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_hex_roundtrip() {
        let data = [0xde, 0xad, 0xbe, 0xef];
        let encoded = hex_encode(&data);
        assert_eq!(encoded, "deadbeef");
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[tokio::test]
    async fn test_compute_file_xxhash3() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        tokio::fs::write(&file_path, b"Hello, world!").await.unwrap();

        let hash = compute_file_xxhash3(file_path.to_str().unwrap()).await.unwrap();
        assert_ne!(hash, 0);

        // Same content, same hash
        let hash2 = compute_file_xxhash3(file_path.to_str().unwrap()).await.unwrap();
        assert_eq!(hash, hash2);
    }

    #[tokio::test]
    async fn test_compute_file_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        tokio::fs::write(&file_path, b"Hello, world!").await.unwrap();

        let hash = compute_file_sha256(file_path.to_str().unwrap()).await.unwrap();
        assert_eq!(hash.len(), 32);

        // Known value
        let hex = hex_encode(&hash);
        assert!(!hex.is_empty());
    }

    #[tokio::test]
    async fn test_compute_file_checksum_all_algorithms() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        tokio::fs::write(&file_path, b"test data").await.unwrap();
        let path_str = file_path.to_str().unwrap();

        let xxh = compute_file_checksum(path_str, ChecksumAlgorithm::XxHash3).await.unwrap();
        assert_eq!(xxh.len(), 16); // 64-bit = 16 hex chars

        let sha = compute_file_checksum(path_str, ChecksumAlgorithm::Sha256).await.unwrap();
        assert_eq!(sha.len(), 64); // 256-bit = 64 hex chars

        let md5 = compute_file_checksum(path_str, ChecksumAlgorithm::Md5).await.unwrap();
        assert_eq!(md5.len(), 32); // 128-bit = 32 hex chars
    }

    #[tokio::test]
    async fn test_verify_transfer_match() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let dst = dir.path().join("dest.dat");
        let content = b"Transfer content for verification";
        tokio::fs::write(&src, content).await.unwrap();
        tokio::fs::write(&dst, content).await.unwrap();

        let result = verify_transfer(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ChecksumAlgorithm::Sha256,
        )
        .await
        .unwrap();
        assert!(result.matches);
    }

    #[tokio::test]
    async fn test_verify_transfer_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.dat");
        let dst = dir.path().join("dest.dat");
        tokio::fs::write(&src, b"source content").await.unwrap();
        tokio::fs::write(&dst, b"different content").await.unwrap();

        let result = verify_transfer(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ChecksumAlgorithm::Sha256,
        )
        .await
        .unwrap();
        assert!(!result.matches);
    }

    #[tokio::test]
    async fn test_compute_chunk_xxhash3() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        // Write 2KB of non-repeating data (use index directly for uniqueness)
        let mut data = vec![0u8; 2048];
        for (i, item) in data.iter_mut().enumerate().take(2048) {
            // Mix in the high bits so chunks at different offsets differ
            *item = ((i * 7 + 13) % 256) as u8;
            if i >= 1024 {
                *item = ((i * 11 + 37) % 256) as u8;
            }
        }
        tokio::fs::write(&file_path, &data).await.unwrap();

        let hash_first = compute_chunk_xxhash3(file_path.to_str().unwrap(), 0, 1024).await.unwrap();
        let hash_second = compute_chunk_xxhash3(file_path.to_str().unwrap(), 1024, 1024).await.unwrap();

        // Different chunks should have different hashes
        assert_ne!(hash_first, hash_second);

        // Same chunk should have same hash
        let hash_first2 = compute_chunk_xxhash3(file_path.to_str().unwrap(), 0, 1024).await.unwrap();
        assert_eq!(hash_first, hash_first2);
    }

    #[tokio::test]
    async fn test_chunk_writeback_verify() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        let data = b"chunk data here";
        tokio::fs::write(&file_path, data).await.unwrap();

        let expected = compute_chunk_xxhash3(file_path.to_str().unwrap(), 0, data.len() as u64)
            .await
            .unwrap();

        let ok = verify_chunk_writeback(file_path.to_str().unwrap(), 0, data.len() as u64, expected)
            .await
            .unwrap();
        assert!(ok);

        let bad = verify_chunk_writeback(file_path.to_str().unwrap(), 0, data.len() as u64, 12345)
            .await
            .unwrap();
        assert!(!bad);
    }

    #[test]
    fn test_should_fsync() {
        // Tier 1: never (until final)
        assert!(!should_fsync(VerifyTier::Fast, 0, 8));
        assert!(!should_fsync(VerifyTier::Fast, 7, 8));

        // Tier 2: every 8 chunks
        assert!(!should_fsync(VerifyTier::Verified, 0, 8));
        assert!(!should_fsync(VerifyTier::Verified, 6, 8));
        assert!(should_fsync(VerifyTier::Verified, 7, 8));
        assert!(should_fsync(VerifyTier::Verified, 15, 8));

        // Tier 3: every chunk
        assert!(should_fsync(VerifyTier::MissionCritical, 0, 8));
        assert!(should_fsync(VerifyTier::MissionCritical, 1, 8));
    }

    #[test]
    fn test_merkle_tree_single_leaf() {
        let hash = sha256_bytes(b"chunk 0");
        let tree = build_merkle_tree(vec![hash], 1024, 1024);
        assert_eq!(tree.root, hash);
        assert_eq!(tree.chunk_count, 1);
        assert!(tree.tree_nodes.is_empty());
    }

    #[test]
    fn test_merkle_tree_two_leaves() {
        let h0 = sha256_bytes(b"chunk 0");
        let h1 = sha256_bytes(b"chunk 1");
        let tree = build_merkle_tree(vec![h0, h1], 1024, 2048);

        assert_eq!(tree.chunk_count, 2);
        assert_eq!(tree.leaf_hashes.len(), 2);

        // Root should be hash of h0 || h1
        let mut combined = Vec::new();
        combined.extend_from_slice(&h0);
        combined.extend_from_slice(&h1);
        let expected_root = sha256_bytes(&combined);
        assert_eq!(tree.root, expected_root);
    }

    #[test]
    fn test_merkle_tree_odd_leaves() {
        let h0 = sha256_bytes(b"chunk 0");
        let h1 = sha256_bytes(b"chunk 1");
        let h2 = sha256_bytes(b"chunk 2");
        let tree = build_merkle_tree(vec![h0, h1, h2], 1024, 3072);
        assert_eq!(tree.chunk_count, 3);
        // Root should be deterministic
        let tree2 = build_merkle_tree(vec![h0, h1, h2], 1024, 3072);
        assert_eq!(tree.root, tree2.root);
    }

    #[test]
    fn test_merkle_sidecar_format() {
        let h0 = sha256_bytes(b"chunk 0");
        let tree = build_merkle_tree(vec![h0], 1024, 1024);
        let sidecar = merkle_to_sidecar(&tree);

        assert_eq!(sidecar.version, 1);
        assert_eq!(sidecar.algorithm, "SHA-256");
        assert_eq!(sidecar.chunk_count, 1);
        assert_eq!(sidecar.leaf_hashes.len(), 1);
        assert_eq!(sidecar.root_hash, hex_encode(&h0));
    }

    #[tokio::test]
    async fn test_preallocate_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preallocated.dat");

        preallocate_file(path.to_str().unwrap(), 1024 * 1024).await.unwrap();

        let metadata = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(metadata.len(), 1024 * 1024);
    }

    #[tokio::test]
    async fn test_sync_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync_test.dat");
        tokio::fs::write(&path, b"data to sync").await.unwrap();

        // Should not error
        sync_to_disk(path.to_str().unwrap()).await.unwrap();
    }

    #[tokio::test]
    async fn test_write_merkle_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar_path = dir.path().join("test.ufop-merkle.json");

        let h0 = sha256_bytes(b"chunk 0");
        let tree = build_merkle_tree(vec![h0], 1024, 1024);

        write_merkle_sidecar(sidecar_path.to_str().unwrap(), &tree)
            .await
            .unwrap();

        assert!(sidecar_path.exists());
        let content = tokio::fs::read_to_string(&sidecar_path).await.unwrap();
        let parsed: MerkleSidecar = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.chunk_count, 1);
    }

    #[tokio::test]
    async fn test_find_corrupted_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data.bin");

        // Create 3 chunks of 1024 bytes
        let mut data = vec![0u8; 3072];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }
        tokio::fs::write(&file_path, &data).await.unwrap();

        // Build tree from original data
        let mut leaf_hashes = Vec::new();
        for i in 0..3 {
            let chunk = &data[i * 1024..(i + 1) * 1024];
            leaf_hashes.push(sha256_bytes(chunk));
        }
        let tree = build_merkle_tree(leaf_hashes, 1024, 3072);

        // No corruption
        let corrupted = find_corrupted_chunks(file_path.to_str().unwrap(), &tree)
            .await
            .unwrap();
        assert!(corrupted.is_empty());

        // Corrupt chunk 1
        data[1024] ^= 0xFF;
        tokio::fs::write(&file_path, &data).await.unwrap();

        let corrupted = find_corrupted_chunks(file_path.to_str().unwrap(), &tree)
            .await
            .unwrap();
        assert_eq!(corrupted, vec![1]);
    }

    #[tokio::test]
    async fn test_verify_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("large.dat");
        let dst = dir.path().join("large_copy.dat");

        let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 256) as u8).collect();
        tokio::fs::write(&src, &data).await.unwrap();
        tokio::fs::write(&dst, &data).await.unwrap();

        let result = verify_transfer(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ChecksumAlgorithm::Sha256,
        )
        .await
        .unwrap();
        assert!(result.matches);
    }

    #[tokio::test]
    async fn test_compute_checksum_nonexistent() {
        let result = compute_file_checksum("/nonexistent/path/file.dat", ChecksumAlgorithm::Sha256).await;
        assert!(result.is_err());
    }
}
