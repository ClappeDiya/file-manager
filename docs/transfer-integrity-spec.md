# Transfer Integrity Architecture -- Technical Specification

Version: 1.0
Applies to: Unified File Operations Platform PRD V5, Section 14.10
Author: Architecture Team
Status: Approved for implementation

---

## 1. Overview

This specification defines the engineering implementation of the three-layer transfer integrity system: maximum throughput, data integrity verification, and crash recovery. It applies to all transfer contexts: local-to-local, local-to-remote, remote-to-local, remote-to-remote relay, peer-to-peer, and drive-to-drive.

All transfer paths -- including sync operations, migration workflows, and backup operations -- use this same underlying engine. The verification tier may differ by context, but the architecture is shared.

---

## 2. Chunk Model

### 2.1 Chunk sizing strategy

Files below the chunk threshold (default 10MB) are transferred as a single atomic unit with no chunking. Files above the threshold are split into equal-sized chunks, with the last chunk being the remainder.

Chunk size selection (adaptive, based on detected media type):

| Media Type | Chunk Size |
|---|---|
| NVMe SSD (local) | 64 MB |
| SATA SSD (local) | 16 MB |
| HDD (local) | 4 MB |
| USB 3.x external | 8 MB |
| USB 2.0 external | 2 MB |
| Network (SFTP/FTP) | 1 MB |
| Network (S3 multipart) | 8 MB (S3 minimum 5MB) |
| Network (high-latency) | 512 KB |
| LAN peer transfer | 4 MB |

Media type detection:
- Linux: /sys/block/sdX/queue/rotational + /sys/block/sdX/queue/nr_requests
- macOS: diskutil info + IOKit device properties
- Windows: WMI Win32_DiskDrive MediaType + DeviceIO queries
- Network: measured RTT during connection test

The user may override chunk size in Advanced mode settings.

### 2.2 Worker pool

Worker count per transfer job:

| Transfer Context | Workers |
|---|---|
| Local-to-local (same drive) | 4 |
| Local-to-local (cross drive) | 8 |
| Local-to-remote (single conn) | 4 |
| Local-to-remote (multi conn) | up to 20 |
| Remote-to-local | up to 20 |
| Peer LAN | 8 |
| Total across all active jobs | 100 (hard cap) |

Worker implementation: tokio::spawn tasks with a tokio::sync::Semaphore for global concurrency control.

Each worker handles one chunk lifecycle:
1. Acquire semaphore permit
2. Read chunk from source at offset (chunk_index * chunk_size)
3. Compute fast hash during read
4. Write chunk to destination at same offset
5. Optionally re-read and verify (Tier 2+)
6. Update journal bitmap
7. Release semaphore permit

### 2.3 Chunk data structure

```rust
struct ChunkSpec {
    chunk_index: u32,
    offset: u64,
    length: u64,
    source_hash: Option<[u8; 8]>,   // xxHash3 (8 bytes)
    dest_hash: Option<[u8; 8]>,     // xxHash3 after write-back read
    sha256: Option<[u8; 32]>,       // SHA-256 for Tier 2+
    status: ChunkStatus,
    write_time_ms: u32,
    verify_time_ms: u32,
}

enum ChunkStatus {
    Pending,
    Reading,
    Writing,
    Verifying,
    Verified,
    Failed { reason: String, attempts: u8 },
    Skipped,  // already verified on resume
}
```

---

## 3. Layer 1: Throughput Engine

### 3.1 Zero-copy I/O paths

**Linux (kernel 5.10+):**
- local-to-local: splice() between two file descriptors via a pipe buffer
- local-to-socket: sendfile() directly from file fd to socket fd
- Implementation: nix::sys::sendfile::sendfile(), nix::fcntl::splice()

**macOS (APFS):**
- same-volume copy: copyfile() with COPYFILE_CLONE flag (instant COW clone, zero I/O)
- Detection: compare source and dest volume UUIDs via statfs()
- Fallback for cross-volume: standard buffered I/O

**Windows:**
- CopyFileEx() with COPY_FILE_NO_BUFFERING for large files
- TransmitFile() for network socket transfers

**Fallback (all platforms):**
- Aligned 256KB buffers via std::alloc::alloc_zeroed with alignment matching the filesystem block size (typically 4096 bytes)

### 3.2 io_uring integration (Linux only)

For local-to-local transfers on Linux kernel 5.19+:

```rust
// Pseudocode for io_uring batch submission
let ring = IoUring::new(32)?;   // 32-entry submission queue
for chunk in chunk_batch.iter() {
    let read_op = opcode::Read::new(source_fd, buf_ptr, chunk.length)
        .offset(chunk.offset)
        .build()
        .user_data(chunk.chunk_index);
    ring.submission().push(&read_op)?;
}
ring.submit_and_wait(chunk_batch.len())?;
// Process completions, submit write batch...
```

Benefits: eliminates per-chunk syscall overhead. For 640 chunks, reduces syscalls from 1280 to 40.

Feature detection: check kernel version and io_uring capability at startup. Gracefully fallback to standard tokio::fs if unavailable.

### 3.3 Direct I/O

For files above 100MB, use O_DIRECT (Linux) or F_NOCACHE (macOS) to bypass the OS page cache. This prevents cache pollution, double-buffering, and unpredictable write-back timing.

Requirements for O_DIRECT:
- Buffers must be sector-aligned (512 bytes or 4096 bytes)
- Read/write sizes must be sector-aligned
- File offsets must be sector-aligned

Implementation: allocate aligned buffers using posix_memalign() or std::alloc with Layout::from_size_align(chunk_size, 4096).

### 3.4 Read-ahead pipeline

The pipeline maintains three stages running concurrently per worker:

- Stage A: Reading chunk N+2 from source into buffer A
- Stage B: Computing hash of chunk N+1 in buffer B
- Stage C: Writing chunk N from buffer C to destination

The stages rotate buffers (triple-buffering) so that source I/O, CPU hashing, and destination I/O happen simultaneously.

Implementation: three tokio::spawn tasks per worker communicating via tokio::sync::mpsc channels.

Pipeline depth is configurable: 2 (lower memory), 3 (default), or 4 (maximum overlap for high-latency destinations).

### 3.5 Pre-allocation

Before writing begins:

```rust
#[cfg(target_os = "linux")]
fn preallocate(fd: RawFd, size: u64) -> Result<()> {
    let ret = unsafe { libc::fallocate(fd, 0, 0, size as libc::off_t) };
    if ret != 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}

#[cfg(target_os = "macos")]
fn preallocate(fd: RawFd, size: u64) -> Result<()> {
    let ret = unsafe { libc::ftruncate(fd, size as libc::off_t) };
    if ret != 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}

#[cfg(target_os = "windows")]
fn preallocate(handle: HANDLE, size: u64) -> Result<()> {
    // SetFilePointerEx + SetEndOfFile
}
```

Pre-allocation ensures: contiguous block allocation on HDD, early space exhaustion detection, and parallel workers can write to known offsets without racing on file extension.

---

## 4. Layer 2: Integrity Verification

### 4.1 Hash computation

**xxHash3 (Tier 1 -- fast integrity):**
- 64-bit hash, hardware-accelerated (AVX2/NEON)
- Throughput: approximately 30 GB/s on modern CPUs
- Computed inline during chunk read (zero additional I/O)
- Crate: xxhash-rust (xxh3 feature)
- Purpose: catch transmission errors, truncation, reordering

**SHA-256 (Tier 2-3 -- cryptographic integrity):**
- 256-bit hash, hardware-accelerated (SHA-NI on x86, ARMv8-CE on ARM)
- Throughput: approximately 2-4 GB/s on modern CPUs with hardware support
- Computed as a second pass for full-file verification
- Crate: ring::digest
- Purpose: cryptographic guarantee of byte-identical copy

### 4.2 Verification modes

**Tier 1 (Fast):**
```
read(chunk) -> compute xxHash3 during read -> write(chunk) -> done
After all chunks: compute full-file xxHash3 on destination, compare to source
```

**Tier 2 (Verified):**
```
read(chunk) -> compute xxHash3 during read -> write(chunk) ->
fsync(chunk) -> re-read(chunk from dest) -> compare xxHash3 ->
if mismatch: retry up to 3 times, then fail chunk
After all chunks: compute full-file SHA-256 on destination, compare to source
```

**Tier 3 (Mission-critical):**
```
read(chunk) -> compute SHA-256 during read -> write(chunk) ->
fsync(chunk) -> re-read(chunk from dest) -> compare SHA-256 ->
if mismatch: retry up to 3 times, then fail chunk
Build Merkle tree from per-chunk SHA-256 hashes
Store Merkle tree as sidecar (.ufop-merkle.json)
After all chunks: verify Merkle root matches expected
Log full audit trail with timestamps
```

### 4.3 Merkle tree structure

```rust
struct MerkleTree {
    leaf_hashes: Vec<[u8; 32]>,    // SHA-256 per chunk
    tree_nodes: Vec<[u8; 32]>,     // internal nodes
    root: [u8; 32],                // root hash
    chunk_count: u32,
    chunk_size: u64,
    file_size: u64,
    algorithm: String,             // "SHA-256"
}
```

Tree construction: standard binary Merkle tree. If chunk count is not a power of 2, the last odd node is promoted to the next level.

Corruption localization: if full-file verification fails, walk the Merkle tree to identify exactly which chunks are corrupted. Re-transfer only those chunks.

Sidecar format (.ufop-merkle.json):
```json
{
  "version": 1,
  "algorithm": "SHA-256",
  "file_size": 10737418240,
  "chunk_size": 16777216,
  "chunk_count": 640,
  "root_hash": "a1b2c3...",
  "leaf_hashes": ["d4e5f6...", "..."],
  "created_at": "2026-03-15T12:00:00Z"
}
```

### 4.4 fsync strategy

- Tier 1: fsync once after all chunks written (before atomic rename)
- Tier 2: fsync every 8 chunks (configurable via transfer settings)
- Tier 3: fsync after every chunk

```rust
fn sync_to_disk(fd: RawFd) -> Result<()> {
    #[cfg(target_os = "linux")]
    { unsafe { libc::fdatasync(fd) }; }
    #[cfg(target_os = "macos")]
    { unsafe { libc::fcntl(fd, libc::F_FULLFSYNC) }; }
    #[cfg(target_os = "windows")]
    { FlushFileBuffers(handle); }
    Ok(())
}
```

Note: macOS fcntl(F_FULLFSYNC) is required because fsync() on macOS does NOT guarantee data is on physical media. F_FULLFSYNC forces a disk cache flush.

---

## 5. Layer 3: Crash Recovery

### 5.1 Transfer journal schema

```sql
CREATE TABLE transfer_jobs (
    job_id          TEXT PRIMARY KEY,
    source_path     TEXT NOT NULL,
    dest_path       TEXT NOT NULL,
    temp_path       TEXT NOT NULL,
    file_size       INTEGER NOT NULL,
    chunk_size      INTEGER NOT NULL,
    chunk_count     INTEGER NOT NULL,
    verify_tier     INTEGER NOT NULL,
    status          TEXT NOT NULL,
    source_hash     TEXT,
    dest_hash       TEXT,
    started_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    completed_at    TEXT,
    error           TEXT
);

CREATE TABLE transfer_chunks (
    job_id          TEXT NOT NULL REFERENCES transfer_jobs(job_id),
    chunk_index     INTEGER NOT NULL,
    offset          INTEGER NOT NULL,
    length          INTEGER NOT NULL,
    status          TEXT NOT NULL,
    source_hash     TEXT,
    dest_hash       TEXT,
    attempts        INTEGER DEFAULT 0,
    verified_at     TEXT,
    PRIMARY KEY (job_id, chunk_index)
);
```

Journal is stored in the main application SQLite database in WAL mode. WAL mode ensures that a crash during journal writes does not corrupt the journal itself.

### 5.2 Recovery procedure

On application startup:

1. Query transfer_jobs WHERE status = active
2. For each active job:
   a. Verify temp file (.ufop-partial) exists
   b. Query transfer_chunks for this job
   c. Build bitmap: verified chunks = skip, pending/failed = re-transfer
   d. Verify each completed chunk hash via quick re-read and comparison
   e. If any previously verified chunk now fails: mark as pending (disk degraded)
   f. Resume transfer from first pending chunk
   g. After all chunks verified: full-file hash, atomic rename, mark complete
3. For jobs where temp file is missing:
   a. Mark all chunks as pending
   b. Restart transfer from scratch (pre-allocation will re-create the temp file)

### 5.3 Atomic rename

All writes target a temporary file: {dest_dir}/.{filename}.ufop-partial

After full verification:
```rust
std::fs::rename(temp_path, final_path)?;
```

- POSIX: rename() is atomic within the same filesystem
- Windows: ReplaceFile() for atomic replacement if destination exists, or MoveFileEx() with MOVEFILE_REPLACE_EXISTING

If the destination already exists and conflict policy is ask/skip, the rename step checks for conflicts before proceeding.

### 5.4 Multi-file transfer manifest

For batch transfers (multiple files in one job):

```json
{
  "version": 1,
  "job_id": "tf_20260315_abc123",
  "source_root": "/Users/md/Documents",
  "dest_root": "sftp://server/backup",
  "verify_tier": 2,
  "files": [
    {
      "relative_path": "report.pdf",
      "size": 5242880,
      "chunk_count": 1,
      "source_hash": "sha256:a1b2c3...",
      "status": "verified"
    },
    {
      "relative_path": "data/export.csv",
      "size": 1073741824,
      "chunk_count": 64,
      "source_hash": "sha256:d4e5f6...",
      "status": "in_progress",
      "chunks_verified": 42,
      "chunks_total": 64
    }
  ],
  "started_at": "2026-03-15T12:00:00Z",
  "updated_at": "2026-03-15T12:05:00Z"
}
```

Manifest location: {dest_root}/.ufop-transfer.json (written atomically via write-to-temp then rename)

---

## 6. Network Protocol Adaptations

### 6.1 SFTP
- Parallel streams: open multiple SFTP channels on the same SSH connection
- Chunk writes via sftp.write_vectored() to specific offsets
- Resume via SFTP stat() to get current file size + positional writes
- Server support detection: check for openssh-sftp posix-rename extension

### 6.2 S3 / S3-compatible
- Multipart upload API: each chunk becomes one part
- Part minimum: 5MB (S3 requirement), maximum: 5GB
- Parallel part uploads with up to 20 concurrent connections
- CompleteMultipartUpload assembles parts server-side
- Resume: ListParts to find completed parts, re-upload missing
- ETag verification: each part returns ETag (MD5), compare to source chunk hash

### 6.3 FTP/FTPS
- REST command for resume from byte offset
- APPE command for append (fallback resume)
- Single-stream per connection (FTP protocol limitation)
- Multiple connections for parallel chunks (check server connection limit)

### 6.4 WebDAV
- Content-Range PUT for partial uploads where server supports it
- Chunked transfer encoding for large files
- LOCK before write, UNLOCK after (prevent concurrent modification)

### 6.5 SMB
- SMB2 multi-credit for parallel I/O on single connection
- Large MTU negotiation for reduced packet overhead
- Oplock for exclusive write access during transfer
- Resume via positional writes to existing file

### 6.6 Peer-to-peer (LAN)
- Custom TLS-encrypted TCP protocol
- Chunk-based protocol: sender sends chunk header (index, offset, length, hash) followed by chunk data; receiver verifies hash and ACKs
- Parallel streams over single connection using multiplexed channels
- Automatic MTU discovery for optimal packet size

---

## 7. User-Facing Integration

### 7.1 Transfer panel display

For each active transfer, the UI shows:
- Overall progress bar (chunks verified / total)
- Current throughput (MB/s, averaged over last 5 seconds)
- ETA (based on rolling throughput average)
- Verification tier badge (Fast / Verified / Mission-critical)
- Per-chunk progress (expandable in Advanced mode)

### 7.2 Verification tier selection

Simple mode automatic tier selection based on context:

| Context | Default Tier |
|---|---|
| Local casual copy | Tier 1 (Fast) |
| Cloud upload/download | Tier 1 (Fast) |
| Sync operations | Tier 2 (Verified) |
| Backup to external drive | Tier 2 (Verified) |
| Migration workflows | Tier 2 (Verified) |
| User explicitly requests verified | Tier 2 (Verified) |
| User explicitly requests mission-critical | Tier 3 (Mission-critical) |

Advanced mode: per-transfer tier override available.

Enterprise: policy can enforce minimum tier per connector type.

### 7.3 Verification failure UX

When a chunk fails verification:
- Simple mode: "Some files needed extra checking. The transfer will take a bit longer to make sure everything is perfect."
- Advanced mode: "Chunk 42/640 of report.pdf failed verification (hash mismatch). Retrying (attempt 2/3). Source hash: a1b2c3. Destination hash: d4e5f6."

When full-file hash fails after all chunks verified:
- Both modes: "Final verification found an issue with [filename]. The file will be re-transferred to ensure a perfect copy."

When a file is unrecoverable after max retries:
- Both modes: clear failure with suggested action (check disk health, try different destination, contact support).

### 7.4 Post-transfer report

After every transfer, a summary is available:
- Total files, total bytes
- Duration, average throughput
- Verification tier used
- Chunks re-transferred (if any)
- Verification result (pass/fail per file)
- For Tier 3: Merkle tree sidecar location

---

## 8. Testing Requirements

### 8.1 Unit tests
- Chunk splitting: edge cases (1 byte file, exactly chunk-size file, max file size)
- Hash computation: known test vectors for xxHash3 and SHA-256
- Journal: write, crash-simulate, recover, verify state
- Bitmap: set, clear, persist, reload
- Atomic rename: concurrent access, cross-filesystem, existing destination
- Pre-allocation: insufficient space, permission denied, readonly fs

### 8.2 Integration tests
- Full transfer lifecycle: local SSD to local SSD, 10GB file
- Crash simulation: kill process at random points, verify clean recovery
- Corrupted chunk: inject bit flip in destination, verify detection and re-transfer
- Network interruption: disconnect during SFTP transfer, verify resume
- USB disconnect: remove drive during transfer, verify journal state
- Parallel transfers: 10 concurrent transfers, verify no corruption or deadlock
- Tier escalation: start Tier 1, upgrade to Tier 2 mid-transfer

### 8.3 Performance benchmarks
- Measure throughput for each media type against theoretical maximum
- Measure verification overhead (Tier 1 vs Tier 2 vs Tier 3 vs no verification)
- Measure recovery time from crash (time from app launch to resume)
- Measure memory usage under maximum worker count
- Compare against OS native copy (Finder, Explorer, cp) for baseline

### 8.4 Stress tests
- 100GB file transfer with Tier 3 verification
- 100,000 small files (1KB each) batch transfer
- Transfer with source disk at 99% capacity
- Transfer to destination with fragmented filesystem
- Transfer over high-latency network (500ms RTT simulated)
- Transfer over lossy network (5% packet loss simulated)

---

## 9. Configuration Parameters

All configurable via Settings (Advanced mode) or enterprise policy:

| Parameter | Default | Description |
|---|---|---|
| transfer.chunk_size_override | null (adaptive) | Manual chunk size override |
| transfer.max_workers_per_job | 8 | Max parallel workers per job |
| transfer.max_workers_global | 100 | Max workers across all jobs |
| transfer.default_verify_tier | 1 | Default verification tier (1/2/3) |
| transfer.fsync_interval | 8 | Chunks between fsync calls |
| transfer.pipeline_depth | 3 | Read-ahead pipeline depth (2/3/4) |
| transfer.retry_max_attempts | 3 | Per-chunk retry count |
| transfer.retry_backoff_base_ms | 1000 | Exponential backoff base |
| transfer.use_direct_io | true (over 100MB) | Bypass OS page cache |
| transfer.use_io_uring | true (Linux 5.19+) | Use io_uring for async I/O |
| transfer.use_zero_copy | true (where supported) | Use sendfile/splice/COW |
| transfer.preallocate | true | Pre-allocate destination file |
| transfer.merkle_sidecar | true (Tier 3) | Write Merkle tree sidecar |
| transfer.journal_retention_days | 30 | Days to keep completed journals |

---

## 10. Implementation Order

This spec maps to the following tasks in the build sequence:

- T-015: Transfer Queue Engine (core queue + chunk model + journal)
- T-016: Resume and Retry Logic (crash recovery + bitmap + partial retry)
- T-018: Post-Transfer Verification (hash computation + tier model + Merkle tree)

The three-layer architecture should be built bottom-up:
1. Journal and chunk model first (Layer 3 foundation)
2. Hash computation and verification tiers (Layer 2)
3. Zero-copy, io_uring, and pipeline optimizations (Layer 1)

This ensures that even during development, every transfer has crash recovery before throughput optimizations are added.

---

## 11. Security Considerations

- Chunk hashes are integrity checks, not authentication. They protect against corruption and accidental modification, not malicious tampering. For tamper-resistance, use the encryption layer (AES-256-GCM) defined in PRD Section 18.3.

- Transfer journals may contain file paths and sizes but never file contents. Journals are stored in the encrypted application database.

- Merkle tree sidecars contain hashes only, not file contents. They may be stored alongside files on the destination. Enterprise policy can disable sidecar generation if hash metadata is considered sensitive.

- io_uring requires no special privileges on modern Linux kernels. O_DIRECT requires the process to have write permission to the destination. No elevated privileges are needed for any throughput optimization.

---

This specification is complete and ready for implementation.
