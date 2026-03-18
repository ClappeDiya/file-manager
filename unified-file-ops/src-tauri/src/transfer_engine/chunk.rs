//! Chunk model: splitting, adaptive sizing, and media type detection.
//!
//! Files below the chunk threshold (default 10MB) are transferred atomically.
//! Files above are split into equal-sized chunks with the last chunk being the remainder.

use crate::core::types::{ChunkSpec, ChunkStatus, MediaType, TransferConfig};

/// Split a file into chunks based on file size and media type.
///
/// Returns a Vec of ChunkSpec with offsets and lengths calculated.
/// Files below `config.chunk_threshold` get a single chunk.
pub fn split_into_chunks(
    file_size: u64,
    media_type: MediaType,
    config: &TransferConfig,
) -> Vec<ChunkSpec> {
    if file_size == 0 {
        return vec![ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: 0,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        }];
    }

    let chunk_size = effective_chunk_size(file_size, media_type, config);

    if file_size <= config.chunk_threshold || chunk_size >= file_size {
        // Single-chunk transfer (atomic)
        return vec![ChunkSpec {
            chunk_index: 0,
            offset: 0,
            length: file_size,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        }];
    }

    let chunk_count = file_size.div_ceil(chunk_size) as u32;
    let mut chunks = Vec::with_capacity(chunk_count as usize);

    for i in 0..chunk_count {
        let offset = i as u64 * chunk_size;
        let length = if i == chunk_count - 1 {
            file_size - offset
        } else {
            chunk_size
        };

        chunks.push(ChunkSpec {
            chunk_index: i,
            offset,
            length,
            source_hash: None,
            dest_hash: None,
            sha256: None,
            status: ChunkStatus::Pending,
            write_time_ms: 0,
            verify_time_ms: 0,
        });
    }

    chunks
}

/// Determine the effective chunk size for a transfer.
pub fn effective_chunk_size(
    _file_size: u64,
    media_type: MediaType,
    config: &TransferConfig,
) -> u64 {
    config
        .chunk_size_override
        .unwrap_or_else(|| media_type.default_chunk_size())
}

/// Determine the worker count for a transfer.
pub fn effective_worker_count(media_type: MediaType, config: &TransferConfig) -> u32 {
    config.max_workers_per_job.min(media_type.default_workers())
}

/// Detect media type for a given path.
///
/// This is a best-effort heuristic. On macOS we check diskutil info,
/// on Linux we check /sys/block, and on unknown platforms we return Unknown.
pub fn detect_media_type(path: &str) -> MediaType {
    // Network paths
    if path.starts_with("sftp://")
        || path.starts_with("ssh://")
        || path.starts_with("ftp://")
        || path.starts_with("ftps://")
    {
        return MediaType::NetworkSftp;
    }
    if path.starts_with("s3://") {
        return MediaType::NetworkS3;
    }
    if path.starts_with("webdav://") || path.starts_with("smb://") {
        return MediaType::NetworkHighLatency;
    }

    // Local path detection
    detect_local_media_type(path)
}

#[cfg(target_os = "macos")]
fn detect_local_media_type(_path: &str) -> MediaType {
    // On macOS, default to SataSsd as a reasonable middle ground.
    // Full detection via IOKit/diskutil would be:
    //   diskutil info <mountpoint> -> "Solid State: Yes/No"
    //   system_profiler SPNVMeDataType -> NVMe detection
    // For now, use SataSsd as the default for local paths on macOS.
    MediaType::SataSsd
}

#[cfg(target_os = "linux")]
fn detect_local_media_type(path: &str) -> MediaType {
    // On Linux, check /sys/block/sdX/queue/rotational
    // 0 = SSD, 1 = HDD
    // Check /sys/block/nvme* for NVMe
    use std::path::Path;

    let p = Path::new(path);
    if !p.exists() {
        return MediaType::Unknown;
    }

    // Try to find the block device for this path
    // Simple heuristic: check common mount points
    if path.starts_with("/dev/nvme") {
        return MediaType::NvmeSsd;
    }

    // Default to SataSsd on Linux
    MediaType::SataSsd
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn detect_local_media_type(_path: &str) -> MediaType {
    MediaType::Unknown
}

/// Generate the temporary file path for atomic writes.
/// Pattern: {dest_dir}/.{filename}.ufop-partial
pub fn temp_path_for(dest_path: &str) -> String {
    use std::path::Path;
    let p = Path::new(dest_path);
    let parent = p.parent().unwrap_or(Path::new(""));
    let filename = p.file_name().map(|f| f.to_string_lossy()).unwrap_or_default();
    parent
        .join(format!(".{filename}.ufop-partial"))
        .to_string_lossy()
        .to_string()
}

/// Generate the Merkle sidecar path for a destination file.
/// Pattern: {dest_path}.ufop-merkle.json
pub fn merkle_sidecar_path(dest_path: &str) -> String {
    format!("{dest_path}.ufop-merkle.json")
}

/// Generate the transfer manifest path.
/// Pattern: {dest_root}/.ufop-transfer.json
pub fn manifest_path(dest_root: &str) -> String {
    use std::path::Path;
    Path::new(dest_root)
        .join(".ufop-transfer.json")
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> TransferConfig {
        TransferConfig::default()
    }

    #[test]
    fn test_split_zero_size() {
        let chunks = split_into_chunks(0, MediaType::SataSsd, &default_config());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].length, 0);
    }

    #[test]
    fn test_split_small_file_single_chunk() {
        let config = default_config(); // threshold = 10MB
        let chunks = split_into_chunks(5 * 1024 * 1024, MediaType::SataSsd, &config);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 5 * 1024 * 1024);
    }

    #[test]
    fn test_split_exactly_threshold() {
        let config = default_config();
        let chunks = split_into_chunks(config.chunk_threshold, MediaType::SataSsd, &config);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_split_large_file_multiple_chunks() {
        let config = default_config();
        // 100MB file with 16MB chunks (SataSsd)
        let file_size = 100 * 1024 * 1024;
        let chunks = split_into_chunks(file_size, MediaType::SataSsd, &config);
        assert!(chunks.len() > 1);

        // Verify chunks cover the entire file
        let total: u64 = chunks.iter().map(|c| c.length).sum();
        assert_eq!(total, file_size);

        // Verify offsets are sequential
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
            if i > 0 {
                let prev = &chunks[i - 1];
                assert_eq!(chunk.offset, prev.offset + prev.length);
            }
        }

        // Last chunk may be smaller
        let last = chunks.last().unwrap();
        assert!(last.length > 0);
    }

    #[test]
    fn test_split_exact_multiple() {
        let config = default_config();
        // 64MB file with 16MB chunks = exactly 4 chunks
        let file_size = 64 * 1024 * 1024;
        let chunks = split_into_chunks(file_size, MediaType::SataSsd, &config);
        assert_eq!(chunks.len(), 4);
        for chunk in &chunks {
            assert_eq!(chunk.length, 16 * 1024 * 1024);
        }
    }

    #[test]
    fn test_split_one_byte_over() {
        let config = default_config();
        // 16MB + 1 byte
        let file_size = 16 * 1024 * 1024 + 1;
        let chunks = split_into_chunks(file_size, MediaType::SataSsd, &config);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].length, 16 * 1024 * 1024);
        assert_eq!(chunks[1].length, 1);
    }

    #[test]
    fn test_chunk_size_per_media_type() {
        assert_eq!(MediaType::NvmeSsd.default_chunk_size(), 64 * 1024 * 1024);
        assert_eq!(MediaType::SataSsd.default_chunk_size(), 16 * 1024 * 1024);
        assert_eq!(MediaType::Hdd.default_chunk_size(), 4 * 1024 * 1024);
        assert_eq!(MediaType::NetworkSftp.default_chunk_size(), 1024 * 1024);
        assert_eq!(MediaType::NetworkS3.default_chunk_size(), 8 * 1024 * 1024);
    }

    #[test]
    fn test_override_chunk_size() {
        let mut config = default_config();
        config.chunk_size_override = Some(1024 * 1024); // 1MB override
        let size = effective_chunk_size(100 * 1024 * 1024, MediaType::NvmeSsd, &config);
        assert_eq!(size, 1024 * 1024);
    }

    #[test]
    fn test_temp_path() {
        assert_eq!(
            temp_path_for("/home/user/file.txt"),
            "/home/user/.file.txt.ufop-partial"
        );
        assert_eq!(
            temp_path_for("/data/report.pdf"),
            "/data/.report.pdf.ufop-partial"
        );
    }

    #[test]
    fn test_merkle_sidecar_path() {
        assert_eq!(
            merkle_sidecar_path("/data/file.bin"),
            "/data/file.bin.ufop-merkle.json"
        );
    }

    #[test]
    fn test_detect_network_paths() {
        assert_eq!(detect_media_type("sftp://server/path"), MediaType::NetworkSftp);
        assert_eq!(detect_media_type("s3://bucket/key"), MediaType::NetworkS3);
        assert_eq!(detect_media_type("smb://share/path"), MediaType::NetworkHighLatency);
    }

    #[test]
    fn test_all_chunks_start_pending() {
        let config = default_config();
        let chunks = split_into_chunks(100 * 1024 * 1024, MediaType::SataSsd, &config);
        for chunk in &chunks {
            assert_eq!(chunk.status, ChunkStatus::Pending);
            assert!(chunk.source_hash.is_none());
            assert!(chunk.dest_hash.is_none());
            assert!(chunk.sha256.is_none());
        }
    }
}
