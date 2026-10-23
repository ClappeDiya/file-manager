//! Storage backends for backup volumes, plus composable wrappers for retry
//! with exponential backoff, bandwidth throttling and soft deletion.
//!
//! The trait is synchronous: the engine runs on a blocking thread, and async
//! remote connectors are bridged by the desktop app (see
//! `src-tauri/src/backup_engine/connector_backend.rs`).

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A file in the destination folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
}

/// Flat, name-addressed volume store.
/// Object-lock (WORM) retention mode, as in S3 Object Lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LockMode {
    /// Users with special permission can still remove the lock.
    #[default]
    Governance,
    /// Nobody, including the account root, can delete before expiry.
    Compliance,
}

/// Destination capacity in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub total: u64,
    pub free: u64,
}

pub trait Backend: Send + Sync {
    /// Short human-readable description (`file:///…`, `sftp://…`).
    fn describe(&self) -> String;
    fn list(&self) -> Result<Vec<RemoteFile>, AppError>;
    fn put(&self, name: &str, data: &[u8]) -> Result<(), AppError>;
    fn get(&self, name: &str) -> Result<Vec<u8>, AppError>;
    fn delete(&self, name: &str) -> Result<(), AppError>;
    /// Create the destination folder when missing.
    fn create_folder(&self) -> Result<(), AppError>;
    /// Rename a volume. Default: copy + delete (needs delete permission).
    fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        let d = self.get(from)?;
        self.put(to, &d)?;
        self.delete(from)
    }
    /// Destination capacity, when the backend can report it.
    fn quota(&self) -> Option<Quota> {
        None
    }
    /// Whether [`Backend::lock`] is available.
    fn supports_lock(&self) -> bool {
        false
    }
    /// Protect a volume against deletion and overwrite until `until`.
    fn lock(&self, name: &str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> Result<(), AppError> {
        let _ = (name, until, mode);
        Err(AppError::validation(format!("{} does not support object locks", self.describe())))
    }
    /// Whether names may contain one `/` (a subfolder, for sharded layouts).
    fn supports_subfolders(&self) -> bool {
        false
    }
    /// Every file, including those one subfolder down as `dir/name`.
    /// Default: [`list`](Self::list) (backends without subfolders).
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.list()
    }
    /// Last-modified time of a file as the destination records it (its own
    /// clock), when the backend can tell. Used for the clock-skew check.
    fn modified(&self, name: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        let _ = name;
        Ok(None)
    }
    /// True when this connection refuses every write (see [`ReadOnlyBackend`]).
    fn is_read_only(&self) -> bool {
        false
    }
    /// Connectivity test: lists the folder. `create` makes it first.
    fn test(&self, create: bool) -> Result<usize, AppError> {
        if create {
            self.create_folder()?;
        }
        Ok(self.list()?.len())
    }
}

impl<T: Backend + ?Sized> Backend for Box<T> {
    fn describe(&self) -> String { (**self).describe() }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> { (**self).list() }
    fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> { (**self).put(n, d) }
    fn get(&self, n: &str) -> Result<Vec<u8>, AppError> { (**self).get(n) }
    fn delete(&self, n: &str) -> Result<(), AppError> { (**self).delete(n) }
    fn create_folder(&self) -> Result<(), AppError> { (**self).create_folder() }
    fn rename(&self, a: &str, b: &str) -> Result<(), AppError> { (**self).rename(a, b) }
    fn quota(&self) -> Option<Quota> { (**self).quota() }
    fn supports_lock(&self) -> bool { (**self).supports_lock() }
    fn lock(&self, n: &str, u: chrono::DateTime<chrono::Utc>, m: LockMode) -> Result<(), AppError> { (**self).lock(n, u, m) }
    fn supports_subfolders(&self) -> bool { (**self).supports_subfolders() }
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> { (**self).list_recursive() }
    fn modified(&self, n: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> { (**self).modified(n) }
    fn is_read_only(&self) -> bool { (**self).is_read_only() }
    fn test(&self, create: bool) -> Result<usize, AppError> { (**self).test(create) }
}

fn check_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::validation(format!("Invalid remote volume name '{name}'")));
    }
    Ok(())
}

/// A name, or `dir/name` with one subfolder level (sharded layouts).
fn check_path(name: &str) -> Result<(), AppError> {
    match name.split_once('/') {
        Some((dir, file)) => check_name(dir).and_then(|_| check_name(file)),
        None => check_name(name),
    }
}

/// Local disk, removable media or mounted share (including UNC paths).
#[derive(Debug, Clone)]
pub struct LocalBackend {
    root: PathBuf,
    /// Re-read the size after writing and fail on mismatch.
    pub verify_upload_size: bool,
    /// Write to a temporary name and rename into place (atomic). Off writes
    /// the final name directly, for shares where renames are slow or fail.
    pub use_move: bool,
    /// Flush each file to disk before reporting success.
    pub sync_writes: bool,
}

impl LocalBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), verify_upload_size: true, use_move: true, sync_writes: false }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Pick the first candidate path that exists (and, when `marker` is set,
    /// contains that marker file). Candidates may contain `*` for the
    /// drive/mount component, e.g. `*:\Backups` or `/media/*/Backups`.
    pub fn discover(candidates: &[String], marker: Option<&str>) -> Result<Self, AppError> {
        for c in candidates {
            let paths: Vec<PathBuf> = if c.contains('*') {
                expand_drive_wildcard(c)
            } else {
                vec![PathBuf::from(c)]
            };
            for p in paths {
                if !p.is_dir() {
                    continue;
                }
                if let Some(m) = marker {
                    if !p.join(m).exists() {
                        continue;
                    }
                }
                return Ok(Self::new(p));
            }
        }
        Err(AppError::file_op(
            "None of the configured destination paths is available",
            "Connect the backup drive (with its marker file) and try again.",
        ))
    }
}

fn expand_drive_wildcard(pattern: &str) -> Vec<PathBuf> {
    // Windows drive-letter wildcard: "*:\..." → A:..Z:
    if pattern.starts_with("*:") {
        return (b'A'..=b'Z')
            .map(|l| PathBuf::from(format!("{}{}", l as char, &pattern[1..])))
            .collect();
    }
    glob::glob(pattern)
        .map(|g| g.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

impl Backend for LocalBackend {
    fn describe(&self) -> String {
        format!("file://{}", self.root.display())
    }

    fn quota(&self) -> Option<Quota> {
        Some(Quota { total: super::meta::total_space(&self.root)?, free: super::meta::free_space(&self.root)? })
    }

    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        if !self.root.is_dir() {
            return Err(AppError::file_op(
                format!("Destination folder {} does not exist", self.root.display()),
                "Use 'Test connection' with 'create folder' or pick another destination.",
            ));
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.root)? {
            let e = e?;
            let md = e.metadata()?;
            if md.is_file() {
                out.push(RemoteFile { name: e.file_name().to_string_lossy().into_owned(), size: md.len() });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn put(&self, name: &str, data: &[u8]) -> Result<(), AppError> {
        check_path(name)?;
        let dest = self.root.join(name);
        if let Some((dir, _)) = name.split_once('/') {
            std::fs::create_dir_all(self.root.join(dir))?;
        }
        let write = |p: &Path| -> std::io::Result<()> {
            let mut f = std::fs::File::create(p)?;
            std::io::Write::write_all(&mut f, data)?;
            if self.sync_writes {
                f.sync_all()?;
            }
            Ok(())
        };
        if self.use_move {
            // Unique per write: two writers of one name (lock files) must
            // not share a temporary file.
            let tag = uuid::Uuid::new_v4().simple().to_string();
            let tmp = match name.split_once('/') {
                Some((dir, file)) => self.root.join(dir).join(format!(".{file}.{}.partial", &tag[..12])),
                None => self.root.join(format!(".{name}.{}.partial", &tag[..12])),
            };
            write(&tmp)?;
            std::fs::rename(&tmp, &dest)?;
        } else {
            write(&dest)?;
        }
        if self.verify_upload_size {
            let got = std::fs::metadata(&dest)?.len();
            if got != data.len() as u64 {
                return Err(AppError::transfer(
                    format!("Size check failed for {name}: wrote {} bytes, found {got}", data.len()),
                    "The destination may be full or unreliable.",
                ));
            }
        }
        Ok(())
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        check_path(name)?;
        Ok(std::fs::read(self.root.join(name))?)
    }

    fn delete(&self, name: &str) -> Result<(), AppError> {
        check_path(name)?;
        let r = match std::fs::remove_file(self.root.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => Ok(r?),
        };
        if let Some((dir, _)) = name.split_once('/') {
            // Drop the shard folder once empty (fails harmlessly otherwise).
            let _ = std::fs::remove_dir(self.root.join(dir));
        }
        r
    }

    fn supports_subfolders(&self) -> bool {
        true
    }

    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        let mut out = self.list()?;
        for e in std::fs::read_dir(&self.root)? {
            let e = e?;
            if !e.file_type()?.is_dir() {
                continue;
            }
            let dir = e.file_name().to_string_lossy().into_owned();
            if check_name(&dir).is_err() || dir.starts_with('.') {
                continue;
            }
            for f in std::fs::read_dir(e.path())? {
                let f = f?;
                let md = f.metadata()?;
                if md.is_file() {
                    out.push(RemoteFile { name: format!("{dir}/{}", f.file_name().to_string_lossy()), size: md.len() });
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn create_folder(&self) -> Result<(), AppError> {
        Ok(std::fs::create_dir_all(&self.root)?)
    }

    fn modified(&self, name: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        check_path(name)?;
        // On a mounted share the file server sets this time.
        Ok(std::fs::metadata(self.root.join(name))?.modified().ok().map(chrono::DateTime::<chrono::Utc>::from))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        check_path(from)?;
        check_path(to)?;
        if let Some((dir, _)) = to.split_once('/') {
            std::fs::create_dir_all(self.root.join(dir))?;
        }
        Ok(std::fs::rename(self.root.join(from), self.root.join(to))?)
    }
}

/// Retry policy for backend operations (the crate-wide policy).
pub use crate::retry::RetryPolicy;

/// Bandwidth limits in bytes/second (0 = unlimited).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Throttle {
    pub upload_bps: u64,
    pub download_bps: u64,
}

/// Wraps a backend with retry/backoff and throttling.
/// One machine-wide rate shared by every backend (all jobs together):
/// each transfer reserves its share of the time line, so the sum of
/// concurrent transfers stays at or below the rate.
pub struct SharedRate {
    bps: std::sync::atomic::AtomicU64,
    next: std::sync::Mutex<Option<Instant>>,
}

impl SharedRate {
    pub const fn new() -> Self {
        Self { bps: std::sync::atomic::AtomicU64::new(0), next: std::sync::Mutex::new(None) }
    }

    /// Bytes per second; 0 = unlimited.
    pub fn set(&self, bps: u64) {
        self.bps.store(bps, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.bps.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How long a transfer of `bytes` that started at `started` must still
    /// wait; reserves its slot.
    pub fn reserve(&self, bytes: usize, started: Instant) -> Duration {
        let bps = self.get();
        if bps == 0 {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        // Idle time is not banked: a slot starts no earlier than this transfer did.
        let start = next.map_or(started, |n| n.max(started));
        let end = start + Duration::from_secs_f64(bytes as f64 / bps as f64);
        *next = Some(end);
        end.saturating_duration_since(now)
    }

    fn wait(&self, bytes: usize, started: Instant) {
        let d = self.reserve(bytes, started);
        if !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}

impl Default for SharedRate {
    fn default() -> Self {
        Self::new()
    }
}

/// Machine-wide upload and download caps (on top of each job's own throttle).
pub static GLOBAL_UPLOAD: SharedRate = SharedRate::new();
pub static GLOBAL_DOWNLOAD: SharedRate = SharedRate::new();

pub struct RetryingBackend<B: Backend> {
    inner: B,
    policy: RetryPolicy,
    throttle: Throttle,
}

impl<B: Backend> RetryingBackend<B> {
    pub fn new(inner: B, policy: RetryPolicy, throttle: Throttle) -> Self {
        Self { inner, policy, throttle }
    }

    fn retry<T>(&self, what: &str, mut f: impl FnMut() -> Result<T, AppError>) -> Result<T, AppError> {
        crate::retry::retry_blocking(&self.policy, |_| f(), is_retryable, |attempt, e, d| {
            tracing::warn!("backup backend {what} failed (attempt {}), retrying in {:?}: {e}", attempt + 1, d);
        })
        .result
    }

    fn pace(bps: u64, bytes: usize, started: Instant) {
        if bps == 0 {
            return;
        }
        let want = Duration::from_secs_f64(bytes as f64 / bps as f64);
        let took = started.elapsed();
        if want > took {
            std::thread::sleep(want - took);
        }
    }
}

fn is_retryable(e: &AppError) -> bool {
    // Validation, passphrase and local-filesystem problems never get better
    // by retrying; network (connection/transfer) failures often do.
    !matches!(
        e,
        AppError::Encryption { .. } | AppError::PermissionDenied { .. } | AppError::Configuration { .. } | AppError::FileOperation { .. }
    )
}

impl<B: Backend> Backend for RetryingBackend<B> {
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn quota(&self) -> Option<Quota> {
        self.inner.quota()
    }
    fn supports_lock(&self) -> bool {
        self.inner.supports_lock()
    }
    fn lock(&self, n: &str, u: chrono::DateTime<chrono::Utc>, m: LockMode) -> Result<(), AppError> {
        self.inner.lock(n, u, m)
    }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.retry("list", || self.inner.list())
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<(), AppError> {
        let t = Instant::now();
        self.retry("put", || self.inner.put(name, data))?;
        Self::pace(self.throttle.upload_bps, data.len(), t);
        GLOBAL_UPLOAD.wait(data.len(), t);
        Ok(())
    }
    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        let t = Instant::now();
        let d = self.retry("get", || self.inner.get(name))?;
        Self::pace(self.throttle.download_bps, d.len(), t);
        GLOBAL_DOWNLOAD.wait(d.len(), t);
        Ok(d)
    }
    fn delete(&self, name: &str) -> Result<(), AppError> {
        self.retry("delete", || self.inner.delete(name))
    }
    fn create_folder(&self) -> Result<(), AppError> {
        self.retry("create folder", || self.inner.create_folder())
    }
    fn rename(&self, a: &str, b: &str) -> Result<(), AppError> {
        self.retry("rename", || self.inner.rename(a, b))
    }
    fn supports_subfolders(&self) -> bool {
        self.inner.supports_subfolders()
    }
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.retry("list", || self.inner.list_recursive())
    }
    fn modified(&self, n: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        self.retry("stat", || self.inner.modified(n))
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
}

/// Renames volumes to `{prefix}{name}` instead of deleting them.
pub struct SoftDeleteBackend<B: Backend> {
    inner: B,
    prefix: String,
}

impl<B: Backend> SoftDeleteBackend<B> {
    pub fn new(inner: B, prefix: impl Into<String>) -> Self {
        Self { inner, prefix: prefix.into() }
    }
}

impl<B: Backend> Backend for SoftDeleteBackend<B> {
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn quota(&self) -> Option<Quota> {
        self.inner.quota()
    }
    fn supports_lock(&self) -> bool {
        self.inner.supports_lock()
    }
    fn lock(&self, n: &str, u: chrono::DateTime<chrono::Utc>, m: LockMode) -> Result<(), AppError> {
        self.inner.lock(n, u, m)
    }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        Ok(self.inner.list()?.into_iter().filter(|f| !f.name.starts_with(&self.prefix)).collect())
    }
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        Ok(self.inner.list_recursive()?.into_iter().filter(|f| !super::layout::logical(&f.name).starts_with(&self.prefix)).collect())
    }
    fn supports_subfolders(&self) -> bool {
        self.inner.supports_subfolders()
    }
    fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> {
        self.inner.put(n, d)
    }
    fn get(&self, n: &str) -> Result<Vec<u8>, AppError> {
        self.inner.get(n)
    }
    fn delete(&self, n: &str) -> Result<(), AppError> {
        let target = match n.rsplit_once('/') {
            Some((dir, file)) => format!("{dir}/{}{file}", self.prefix),
            None => format!("{}{}", self.prefix, n),
        };
        self.inner.rename(n, &target)
    }
    fn create_folder(&self) -> Result<(), AppError> {
        self.inner.create_folder()
    }
    fn rename(&self, a: &str, b: &str) -> Result<(), AppError> {
        self.inner.rename(a, b)
    }
    fn modified(&self, n: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        self.inner.modified(n)
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
}

/// A connection that only reads: every write (upload, delete, rename,
/// folder creation, object lock) fails with a clear error before anything
/// reaches the destination. Restores, browsing and verification work.
pub struct ReadOnlyBackend<B: Backend> {
    inner: B,
}

impl<B: Backend> ReadOnlyBackend<B> {
    pub fn new(inner: B) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> B {
        self.inner
    }

    fn refuse(&self, what: &str, name: &str) -> AppError {
        AppError::PermissionDenied {
            message: format!("{} is connected read-only: cannot {what} {name}", self.inner.describe()),
            advice: "This job only restores and browses. Turn off 'Read-only' in the job's destination settings to back up or run maintenance.".into(),
        }
    }
}

impl<B: Backend> Backend for ReadOnlyBackend<B> {
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.inner.list()
    }
    fn put(&self, name: &str, _data: &[u8]) -> Result<(), AppError> {
        Err(self.refuse("upload", name))
    }
    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        self.inner.get(name)
    }
    fn delete(&self, name: &str) -> Result<(), AppError> {
        Err(self.refuse("delete", name))
    }
    fn create_folder(&self) -> Result<(), AppError> {
        // Nothing to create when the folder exists; never create one.
        self.inner.list().map(|_| ())
    }
    fn rename(&self, from: &str, _to: &str) -> Result<(), AppError> {
        Err(self.refuse("rename", from))
    }
    fn quota(&self) -> Option<Quota> {
        self.inner.quota()
    }
    fn supports_lock(&self) -> bool {
        false
    }
    fn lock(&self, name: &str, _until: chrono::DateTime<chrono::Utc>, _mode: LockMode) -> Result<(), AppError> {
        Err(self.refuse("lock", name))
    }
    fn supports_subfolders(&self) -> bool {
        self.inner.supports_subfolders()
    }
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.inner.list_recursive()
    }
    fn modified(&self, n: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        self.inner.modified(n)
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn test(&self, _create: bool) -> Result<usize, AppError> {
        Ok(self.inner.list()?.len())
    }
}

/// Exercise a destination with randomized upload/download/hash round-trips
/// (BackendTester equivalent). Returns per-step results.
pub fn test_backend_compatibility(
    backend: &dyn Backend,
    files: usize,
    min_size: usize,
    max_size: usize,
) -> Result<Vec<String>, AppError> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut log = Vec::new();
    backend.create_folder()?;
    let mut created = Vec::new();
    for i in 0..files.max(1) {
        let size = rng.gen_range(min_size..=max_size.max(min_size));
        let mut data = vec![0u8; size];
        rng.fill(&mut data[..]);
        let name = format!("ufop-backendtest-{}-{i}.bin", uuid::Uuid::new_v4().simple());
        let h = super::hash_bytes(super::HashAlgorithm::Sha256, &data);
        backend.put(&name, &data)?;
        created.push(name.clone());
        let listed = backend.list()?;
        let entry = listed.iter().find(|f| f.name == name).ok_or_else(|| {
            AppError::transfer(format!("{name} missing from listing after upload"), "The destination listing is inconsistent.")
        })?;
        if entry.size != size as u64 {
            log.push(format!("WARN {name}: listed size {} != {size}", entry.size));
        }
        let back = backend.get(&name)?;
        if super::hash_bytes(super::HashAlgorithm::Sha256, &back) != h {
            return Err(AppError::transfer(format!("{name}: downloaded content differs"), "The destination corrupts data."));
        }
        log.push(format!("OK {name} ({size} bytes) upload/list/download/hash"));
    }
    for n in &created {
        backend.delete(n)?;
    }
    if backend.list()?.iter().any(|f| created.contains(&f.name)) {
        log.push("WARN deleted test files still listed".to_string());
    } else {
        log.push(format!("OK deleted {} test file(s)", created.len()));
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_rate_splits_the_cap_between_concurrent_transfers() {
        let r = SharedRate::new();
        let t = Instant::now();
        assert_eq!(r.reserve(1_000_000, t), Duration::ZERO, "unlimited by default");
        r.set(1000);
        let waits: Vec<f64> = (0..3).map(|_| r.reserve(500, t).as_secs_f64()).collect();
        // Three 500-byte transfers at 1000 B/s end at 0.5 s, 1.0 s and 1.5 s.
        for (w, want) in waits.iter().zip([0.5, 1.0, 1.5]) {
            assert!((w - want).abs() < 0.05, "{waits:?}");
        }
        // A transfer that already took longer than its share does not wait.
        let r = SharedRate::new();
        r.set(1_000_000);
        let old = Instant::now() - Duration::from_secs(2);
        assert_eq!(r.reserve(1000, old), Duration::ZERO);
        // Idle time is not banked as credit.
        assert!(r.reserve(1_000_000, Instant::now()).as_secs_f64() > 0.9);
        r.set(0);
        assert_eq!(r.reserve(1_000_000, Instant::now()), Duration::ZERO);
    }

    #[test]
    fn local_put_modes() {
        let d = tempfile::tempdir().unwrap();
        for (mv, sync) in [(true, false), (false, true)] {
            let b = LocalBackend { use_move: mv, sync_writes: sync, ..LocalBackend::new(d.path()) };
            b.put(&format!("v-{mv}"), b"payload").unwrap();
            assert_eq!(b.get(&format!("v-{mv}")).unwrap(), b"payload");
        }
        assert!(!std::fs::read_dir(d.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".partial")));
    }

    #[test]
    fn local_backend_crud_and_names() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path().join("dest"));
        assert!(b.list().is_err());
        assert_eq!(b.test(true).unwrap(), 0);
        b.put("a.dblock", b"123").unwrap();
        assert_eq!(b.list().unwrap(), vec![RemoteFile { name: "a.dblock".into(), size: 3 }]);
        assert_eq!(b.get("a.dblock").unwrap(), b"123");
        assert!(b.put("../x", b"").is_err());
        b.rename("a.dblock", "b.dblock").unwrap();
        b.delete("b.dblock").unwrap();
        b.delete("b.dblock").unwrap();
        assert!(b.list().unwrap().is_empty());
    }

    #[test]
    fn soft_delete_renames() {
        let d = tempfile::tempdir().unwrap();
        let b = SoftDeleteBackend::new(LocalBackend::new(d.path()), "deleted-");
        b.put("v1", b"x").unwrap();
        b.delete("v1").unwrap();
        assert!(b.list().unwrap().is_empty());
        assert!(d.path().join("deleted-v1").exists());
    }

    #[test]
    fn discover_requires_marker() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().to_string_lossy().to_string();
        assert!(LocalBackend::discover(std::slice::from_ref(&p), Some("MARK")).is_err());
        std::fs::write(d.path().join("MARK"), b"").unwrap();
        assert_eq!(LocalBackend::discover(&[p], Some("MARK")).unwrap().root(), d.path());
    }

    #[test]
    fn local_errors_are_not_retried() {
        let d = tempfile::tempdir().unwrap();
        let b = RetryingBackend::new(LocalBackend::new(d.path().join("missing")), RetryPolicy { retries: 3, delay_ms: 5_000, ..Default::default() }, Throttle::default());
        let t = Instant::now();
        assert!(b.list().is_err());
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let p = RetryPolicy { retries: 3, delay_ms: 100, exponential: true, max_delay_ms: 350 };
        assert_eq!(p.delay_for(0), Duration::from_millis(100));
        assert_eq!(p.delay_for(1), Duration::from_millis(200));
        assert_eq!(p.delay_for(2), Duration::from_millis(350));
    }

    #[test]
    fn read_only_refuses_every_write() {
        let d = tempfile::tempdir().unwrap();
        let local = LocalBackend::new(d.path());
        local.put("a", b"1").unwrap();
        let ro = ReadOnlyBackend::new(LocalBackend::new(d.path()));
        assert!(ro.is_read_only() && !local.is_read_only());
        assert_eq!(ro.get("a").unwrap(), b"1");
        assert_eq!(ro.list().unwrap().len(), 1);
        assert!(ro.modified("a").unwrap().is_some());
        for e in [ro.put("b", b"2").unwrap_err(), ro.delete("a").unwrap_err(), ro.rename("a", "c").unwrap_err(), ro.lock("a", chrono::Utc::now(), LockMode::Governance).unwrap_err()] {
            assert!(matches!(e, AppError::PermissionDenied { .. }), "{e:?}");
            assert!(e.user_message().contains("read-only"));
        }
        assert!(!ro.supports_lock());
        assert_eq!(local.list().unwrap().len(), 1, "nothing was written");
        // Wrapped again (retry, box) it is still read-only.
        let boxed: Box<dyn Backend> = Box::new(RetryingBackend::new(ro, RetryPolicy::default(), Throttle::default()));
        assert!(boxed.is_read_only() && boxed.put("x", b"").is_err());
        // A missing folder is never created.
        assert!(ReadOnlyBackend::new(LocalBackend::new(d.path().join("nope"))).create_folder().is_err());
    }

    #[test]
    fn compatibility_tester_roundtrips() {
        let d = tempfile::tempdir().unwrap();
        let log = test_backend_compatibility(&LocalBackend::new(d.path()), 3, 10, 2000).unwrap();
        assert_eq!(log.iter().filter(|l| l.starts_with("OK")).count(), 4);
    }
}
