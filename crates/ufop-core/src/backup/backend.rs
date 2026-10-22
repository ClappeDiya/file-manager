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
}

fn check_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::validation(format!("Invalid remote volume name '{name}'")));
    }
    Ok(())
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
        check_name(name)?;
        let dest = self.root.join(name);
        let write = |p: &Path| -> std::io::Result<()> {
            let mut f = std::fs::File::create(p)?;
            std::io::Write::write_all(&mut f, data)?;
            if self.sync_writes {
                f.sync_all()?;
            }
            Ok(())
        };
        if self.use_move {
            let tmp = self.root.join(format!(".{name}.partial"));
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
        check_name(name)?;
        Ok(std::fs::read(self.root.join(name))?)
    }

    fn delete(&self, name: &str) -> Result<(), AppError> {
        check_name(name)?;
        match std::fs::remove_file(self.root.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => Ok(r?),
        }
    }

    fn create_folder(&self) -> Result<(), AppError> {
        Ok(std::fs::create_dir_all(&self.root)?)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        check_name(from)?;
        check_name(to)?;
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
        Ok(())
    }
    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        let t = Instant::now();
        let d = self.retry("get", || self.inner.get(name))?;
        Self::pace(self.throttle.download_bps, d.len(), t);
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
    fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> {
        self.inner.put(n, d)
    }
    fn get(&self, n: &str) -> Result<Vec<u8>, AppError> {
        self.inner.get(n)
    }
    fn delete(&self, n: &str) -> Result<(), AppError> {
        self.inner.rename(n, &format!("{}{}", self.prefix, n))
    }
    fn create_folder(&self) -> Result<(), AppError> {
        self.inner.create_folder()
    }
    fn rename(&self, a: &str, b: &str) -> Result<(), AppError> {
        self.inner.rename(a, b)
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
    fn compatibility_tester_roundtrips() {
        let d = tempfile::tempdir().unwrap();
        let log = test_backend_compatibility(&LocalBackend::new(d.path()), 3, 10, 2000).unwrap();
        assert_eq!(log.iter().filter(|l| l.starts_with("OK")).count(), 4);
    }
}
