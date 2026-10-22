//! Versioned backup engine: block-level deduplication, per-block compression,
//! client-side AES-256-GCM encryption and remote volumes on any [`Backend`].
//!
//! Remote layout (all files live flat in the destination folder):
//!
//! - `{prefix}-repo.json` — unencrypted repository header: format version,
//!   KDF salt/params and a key-check value. Never contains secrets.
//! - `{prefix}-b{id}.dblock[.aes]` — packed data blocks.
//! - `{prefix}-i{id}.dindex[.aes]` — which blocks live in which dblock, so the
//!   local database can be rebuilt from the remote alone.
//! - `{prefix}-{UTC timestamp}.dlist[.aes]` — one backup version (file list).
//! - `{prefix}-verification.json` — optional unencrypted SHA-256 inventory.
//!
//! The local database (`db`) is a cache of that remote state; `maintenance`
//! can recreate it from the remote files when it is lost.

pub mod backend;
pub mod crypto;
pub mod db;
pub mod engine;
pub mod filecrypt;
pub mod filters;
pub mod gpg;
pub mod maintenance;
pub mod meta;
pub mod mirror;
pub mod modules;
pub mod par2;
pub mod photos;
pub mod recovery;
pub mod restore;
pub mod retention;
pub mod snapshot;
pub mod usn;
pub mod volume;

pub use backend::{Backend, LocalBackend, RemoteFile, RetryingBackend, SoftDeleteBackend};
pub use engine::{run_backup, BackupReport};
pub use filters::{FilterAction, FilterKind, FilterRule, SourceFilter};
pub use restore::{run_restore, RestoreOptions, RestoreReport};
pub use retention::RetentionPolicy;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Repository format version written to `{prefix}-repo.json`.
pub const REPO_FORMAT_VERSION: u32 = 1;

/// How symbolic links in the source are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkPolicy {
    /// Record the link itself (its target path), not the content.
    #[default]
    Store,
    /// Follow the link and back up what it points at.
    Follow,
    /// Skip links entirely.
    Ignore,
}

/// How files with several hard links are handled (Unix only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HardlinkPolicy {
    /// Back up every path.
    #[default]
    All,
    /// Back up only the first path seen for an inode.
    First,
    /// Skip files whose link count is above one.
    None,
}

/// What to do when a configured source path does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MissingSourcePolicy {
    /// Abort the backup.
    #[default]
    Fail,
    /// Log a warning and continue with the remaining sources.
    Warn,
}

/// Block / file hash algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgorithm {
    #[default]
    Sha256,
    Sha512,
}

/// Remote dblock container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VolumeFormat {
    /// Compact UFOP block pack (per-block deflate).
    #[default]
    Ufop,
    /// Standard ZIP archive (one entry per block, ZIP64 when needed).
    Zip,
}

/// Compression method for ZIP volumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ZipMethod {
    #[default]
    Deflate,
    Stored,
    Bzip2,
    Zstd,
}

/// Windows NTFS change-journal use for faster scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UsnPolicy {
    #[default]
    Off,
    /// Use it when available, silently fall back to a full scan.
    Auto,
    /// Use it; warn when falling back.
    On,
    /// Fail the backup when the journal cannot be used.
    Required,
}

/// Object-lock retention for uploaded volumes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ObjectLockOptions {
    /// Retention period from the time of upload.
    pub days: u32,
    pub mode: backend::LockMode,
}

/// Optional PAR2 recovery data for uploaded volumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Par2Options {
    pub enabled: bool,
    /// Redundancy percentage per volume (`par2 create -r`).
    pub redundancy_percent: u8,
    /// Path to the `par2` executable.
    pub binary: String,
}

impl Default for Par2Options {
    fn default() -> Self {
        Self { enabled: false, redundancy_percent: 10, binary: "par2".into() }
    }
}

/// Every tunable of a backup run. Defaults mirror sensible Duplicati defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupOptions {
    /// Remote file-name prefix (`duplicati` by default there; `ufop` here).
    pub prefix: String,
    /// Deduplication block size in bytes. Fixed once the repository exists.
    pub block_size: usize,
    /// Target remote volume (dblock) size in bytes.
    pub volume_size: usize,
    /// Deflate level 0–9; 0 stores without compression.
    pub compression_level: u32,
    /// Extensions (lower-case, no dot) stored without compression.
    pub no_compress_extensions: Vec<String>,
    pub block_hash: HashAlgorithm,
    pub file_hash: HashAlgorithm,
    pub symlink_policy: SymlinkPolicy,
    pub hardlink_policy: HardlinkPolicy,
    pub missing_source: MissingSourcePolicy,
    /// Refuse to create a version when the source set turned out empty.
    pub allow_empty_source: bool,
    /// Re-read every file even when size and timestamp are unchanged.
    pub disable_timestamp_check: bool,
    /// Trust timestamps alone (ignore size changes) when deciding to re-read.
    pub timestamps_only: bool,
    /// Do not record timestamps / permissions.
    pub skip_metadata: bool,
    /// Create a version even when nothing changed.
    pub upload_unchanged_backups: bool,
    /// Volumes sampled for verification after each backup (0 disables).
    pub verify_samples: usize,
    /// Run compaction automatically after retention deletes versions.
    pub auto_compact: bool,
    /// Wasted-space percentage that makes a volume eligible for compaction.
    pub compact_threshold_percent: u8,
    /// Upload `{prefix}-verification.json` with remote sizes and hashes.
    pub upload_verification_file: bool,
    /// Report what would happen without writing anything locally or remotely.
    pub dry_run: bool,
    /// Optional job configuration (secrets already stripped by the caller) to
    /// embed in each new file-list volume.
    pub task_config: Option<serde_json::Value>,
    /// Allow retention / delete to remove the very last version.
    pub allow_full_removal: bool,

    // ── Selection and metadata ──
    /// Ignore macOS Time Machine / `user.xdg.robots.backup` exclusion attributes.
    pub ignore_exclusion_xattrs: bool,
    /// Skip files another process holds an advisory (flock) lock on.
    pub respect_advisory_locks: bool,
    /// Only rescan these paths (files or folders); everything else is
    /// carried over unchanged from the previous version.
    pub changed_paths: Option<Vec<String>>,
    /// Paths known to be deleted since the previous version.
    pub deleted_paths: Vec<String>,
    pub usn_policy: UsnPolicy,
    /// Capture extended attributes.
    pub backup_xattrs: bool,
    /// Capture ACLs / security descriptors.
    pub backup_acls: bool,
    /// Capture NTFS alternate data streams (Windows).
    pub backup_alternate_streams: bool,

    // ── Volume format ──
    pub volume_format: VolumeFormat,
    pub zip_method: ZipMethod,
    /// Extra files attached to each new file-list volume.
    pub control_files: Vec<String>,
    pub par2: Par2Options,

    // ── Performance ──
    /// Hashing/compression threads (0 = automatic).
    pub hash_threads: usize,
    /// Volumes uploaded concurrently (1 = synchronous).
    pub upload_concurrency: usize,
    /// 1 (gentlest) … 10 (full speed).
    pub cpu_intensity: u8,
    /// Folder for temporary files (default: the system temp folder).
    pub temp_dir: Option<String>,
    /// Source read buffer in bytes (0 = block size).
    pub read_buffer_size: usize,
    /// Lower CPU and I/O priority of worker threads.
    pub background_io: bool,

    // ── Capacity and maintenance ──
    /// Assumed destination capacity when the backend cannot report one.
    pub quota_size: Option<u64>,
    /// Warn when free destination space falls below this percentage.
    pub quota_warning_percent: u8,
    /// Do not check destination capacity.
    pub quota_disable: bool,
    /// Compact at most this often (days); `None` = after every backup.
    pub auto_compact_interval_days: Option<u32>,
    /// Vacuum the local database at most this often (days).
    pub auto_vacuum_interval_days: Option<u32>,
    /// Compare remote file names case-insensitively.
    pub remote_case_insensitive: bool,
    /// Lock every new volume against deletion (S3 Object Lock and similar).
    pub object_lock: Option<ObjectLockOptions>,
    /// Read sources from a VSS / LVM / APFS snapshot.
    pub snapshot_policy: snapshot::SnapshotPolicy,
    pub snapshot: snapshot::SnapshotOptions,
    /// Volume encryption for new repositories (existing ones keep theirs).
    pub encryption_module: EncryptionModuleKind,
    pub gpg: GpgOptions,
    /// Argon2id cost for new AES repositories (`None` = the app default).
    pub kdf: Option<crypto::KdfParams>,
    /// What `photos://` (macOS Photos library) sources export.
    pub photos: photos::PhotosOptions,
}

/// How volumes are encrypted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionModuleKind {
    /// Built-in AES-256-GCM with an Argon2id-derived key.
    #[default]
    Aes,
    /// OpenPGP through the `gpg` executable.
    Gpg,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GpgOptions {
    /// Encrypt to these keys; empty = symmetric with the passphrase.
    pub recipients: Vec<String>,
    /// `gpg` binary (empty = `$UFOP_GPG` or `gpg`).
    pub executable: String,
    /// Keyring folder (empty = default / `$GNUPGHOME`).
    pub homedir: String,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            prefix: "ufop".to_string(),
            block_size: 100 * 1024,
            volume_size: 50 * 1024 * 1024,
            compression_level: 6,
            no_compress_extensions: default_no_compress_extensions(),
            block_hash: HashAlgorithm::Sha256,
            file_hash: HashAlgorithm::Sha256,
            symlink_policy: SymlinkPolicy::Store,
            hardlink_policy: HardlinkPolicy::All,
            missing_source: MissingSourcePolicy::Fail,
            allow_empty_source: false,
            disable_timestamp_check: false,
            timestamps_only: false,
            skip_metadata: false,
            upload_unchanged_backups: false,
            verify_samples: 1,
            auto_compact: true,
            compact_threshold_percent: 25,
            upload_verification_file: false,
            dry_run: false,
            task_config: None,
            allow_full_removal: false,
            ignore_exclusion_xattrs: false,
            respect_advisory_locks: false,
            changed_paths: None,
            deleted_paths: Vec::new(),
            usn_policy: UsnPolicy::Off,
            backup_xattrs: true,
            backup_acls: true,
            backup_alternate_streams: false,
            volume_format: VolumeFormat::Ufop,
            zip_method: ZipMethod::Deflate,
            control_files: Vec::new(),
            par2: Par2Options::default(),
            hash_threads: 0,
            upload_concurrency: 2,
            cpu_intensity: 10,
            temp_dir: None,
            read_buffer_size: 0,
            background_io: false,
            quota_size: None,
            quota_warning_percent: 10,
            quota_disable: false,
            auto_compact_interval_days: None,
            auto_vacuum_interval_days: None,
            remote_case_insensitive: false,
            object_lock: None,
            snapshot_policy: snapshot::SnapshotPolicy::Off,
            snapshot: snapshot::SnapshotOptions::default(),
            encryption_module: EncryptionModuleKind::Aes,
            gpg: GpgOptions::default(),
            kdf: None,
            photos: photos::PhotosOptions::default(),
        }
    }
}

impl BackupOptions {
    /// Worker threads for hashing/compression after applying CPU intensity.
    pub fn worker_threads(&self) -> usize {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let base = if self.hash_threads > 0 { self.hash_threads } else { cores.min(8) };
        let scaled = base * self.cpu_intensity.clamp(1, 10) as usize / 10;
        scaled.max(1)
    }

    pub fn temp_root(&self) -> std::path::PathBuf {
        self.temp_dir.as_ref().map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir)
    }
}

/// Already-compressed formats; recompressing them only burns CPU.
pub fn default_no_compress_extensions() -> Vec<String> {
    [
        "7z", "zip", "gz", "tgz", "bz2", "xz", "zst", "rar", "jpg", "jpeg", "png", "gif", "webp",
        "heic", "mp3", "mp4", "m4a", "m4v", "mkv", "mov", "avi", "aac", "ogg", "flac", "pdf",
        "docx", "xlsx", "pptx", "odt", "jar", "apk", "dmg", "iso",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Phase of a running operation, reported through [`Progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Preparing,
    Scanning,
    Uploading,
    Finalizing,
    Retention,
    Compacting,
    Verifying,
    Downloading,
    Restoring,
    Completed,
}

/// Live progress snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    pub files_processed: u64,
    pub files_total: u64,
    pub bytes_processed: u64,
    pub current_path: Option<String>,
}

/// Progress sink + cancellation flag shared by every long-running operation.
type ProgressFn = Arc<dyn Fn(&Progress) + Send + Sync>;

#[derive(Clone, Default)]
pub struct OpContext {
    cancel: Arc<AtomicBool>,
    progress: Option<ProgressFn>,
    /// Called once the sources are no longer read (snapshot can be released).
    released: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl OpContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_progress(mut self, f: impl Fn(&Progress) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(f));
        self
    }

    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = flag;
        self
    }

    pub fn with_source_released(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.released = Some(Arc::new(f));
        self
    }

    pub fn source_released(&self) {
        if let Some(f) = &self.released {
            f();
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub fn report(&self, p: &Progress) {
        if let Some(f) = &self.progress {
            f(p);
        }
    }

    pub(crate) fn check(&self) -> Result<(), crate::error::AppError> {
        if self.is_cancelled() {
            Err(crate::error::AppError::state(
                "The operation was cancelled",
                "Start it again when ready; completed work is kept.",
            ))
        } else {
            Ok(())
        }
    }
}

/// Kind of a backed-up entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

/// One entry of a backup version, as stored in the file-list volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Modification time, Unix seconds.
    pub mtime: Option<i64>,
    /// Unix permission bits.
    pub mode: Option<u32>,
    /// Full-file hash (hex), files only.
    pub hash: Option<String>,
    /// Ordered block hashes (hex), files only.
    pub blocks: Vec<String>,
    pub symlink_target: Option<String>,
    /// Extended attributes, ACLs, flags (absent in older backups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<meta::ExtMeta>,
}

/// One backup version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fileset {
    pub id: i64,
    /// UTC timestamp (RFC 3339).
    pub timestamp: String,
    pub volume_name: String,
    /// True when written from an interrupted backup.
    pub partial: bool,
    pub file_count: u64,
    pub total_size: u64,
}

pub(crate) fn hash_bytes(alg: HashAlgorithm, data: &[u8]) -> String {
    use sha2::Digest;
    match alg {
        HashAlgorithm::Sha256 => hex::encode(sha2::Sha256::digest(data)),
        HashAlgorithm::Sha512 => hex::encode(sha2::Sha512::digest(data)),
    }
}

/// Streaming file hasher for either algorithm.
pub(crate) enum FileHasher {
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl FileHasher {
    pub(crate) fn new(alg: HashAlgorithm) -> Self {
        use sha2::Digest;
        match alg {
            HashAlgorithm::Sha256 => Self::Sha256(sha2::Sha256::new()),
            HashAlgorithm::Sha512 => Self::Sha512(sha2::Sha512::new()),
        }
    }
    pub(crate) fn update(&mut self, d: &[u8]) {
        use sha2::Digest;
        match self {
            Self::Sha256(h) => h.update(d),
            Self::Sha512(h) => h.update(d),
        }
    }
    pub(crate) fn finish(self) -> String {
        use sha2::Digest;
        match self {
            Self::Sha256(h) => hex::encode(h.finalize()),
            Self::Sha512(h) => hex::encode(h.finalize()),
        }
    }
}

/// Short stable key for a string (e.g. a default database file name).
pub fn hash_file_name_key(s: &str) -> String {
    hash_bytes(HashAlgorithm::Sha256, s.as_bytes())[..16].to_string()
}

/// Hash a local file with the given algorithm.
pub fn hash_file(alg: HashAlgorithm, path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = FileHasher::new(alg);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finish())
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::Path;

    pub fn write(root: &Path, rel: &str, content: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
}
