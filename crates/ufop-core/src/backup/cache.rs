//! Bounded local cache of downloaded volumes.
//!
//! Previewing a file, browsing a mounted version and re-reading the same
//! data during a restore would otherwise download the same volumes again
//! and again. The cache keeps **sealed** volumes (exactly the bytes the
//! destination holds, still encrypted for encrypted backups), so it is as
//! safe at rest as the destination itself.
//!
//! - Entries are files `<dir>/<ns>/<sha256 of name>.vol`; `ns` separates
//!   repositories (destination + prefix), so two backups never share entries.
//! - Every read checks the entry against the volume's recorded SHA-256 (and
//!   its size). A damaged or truncated entry is deleted and reported as a
//!   miss, so the caller downloads the volume again.
//! - Least recently used entries are evicted once the total passes
//!   `max_bytes`. The access time is the entry's modification time, which a
//!   hit refreshes, so the order survives restarts and is shared by every
//!   process using the folder.
//! - Writes go to a temporary file that is renamed into place, so readers in
//!   other threads or processes never see half an entry.

use super::{hash_bytes, HashAlgorithm};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

/// Default size limit: 2 GiB.
pub const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SUFFIX: &str = ".vol";

/// Settings of the volume cache (stored by the app; the CLI takes flags).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSettings {
    /// Cache folder; `None` = the app's cache folder.
    pub dir: Option<String>,
    /// Size limit in bytes (0 = cache off).
    pub max_bytes: u64,
    /// Also cache the volumes of full restores (off: a large restore would
    /// evict everything else and gain nothing, because each volume is read once).
    pub use_for_restore: bool,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self { dir: None, max_bytes: DEFAULT_MAX_BYTES, use_for_restore: false }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CacheStats {
    pub dir: String,
    pub max_bytes: u64,
    pub entries: u64,
    pub bytes: u64,
    /// Since this program started.
    pub hits: u64,
    pub misses: u64,
    /// Entries that failed their check and were dropped.
    pub corrupt: u64,
    pub evictions: u64,
    /// hits / (hits + misses), 0 when nothing was looked up.
    pub hit_rate: f64,
}

/// A size-bounded, concurrency-safe store of sealed volumes.
#[derive(Debug)]
pub struct VolumeCache {
    dir: PathBuf,
    max_bytes: u64,
    /// Serialises eviction scans within this process (other processes are
    /// safe through temp+rename; a concurrent eviction at worst removes an
    /// entry the other side then downloads again).
    evict: Mutex<()>,
    hits: AtomicU64,
    misses: AtomicU64,
    corrupt: AtomicU64,
    evictions: AtomicU64,
}

/// Namespace of a repository: destination description + file prefix.
pub fn namespace(destination: &str, prefix: &str) -> String {
    hash_bytes(HashAlgorithm::Sha256, format!("{destination}\n{prefix}").as_bytes())[..16].to_string()
}

impl VolumeCache {
    pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> Result<Self, AppError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| AppError::file_op(format!("Cannot create the volume cache folder {}: {e}", dir.display()), "Pick another cache folder in Settings."))?;
        Ok(Self { dir, max_bytes, evict: Mutex::new(()), hits: AtomicU64::new(0), misses: AtomicU64::new(0), corrupt: AtomicU64::new(0), evictions: AtomicU64::new(0) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    fn entry_path(&self, ns: &str, name: &str) -> PathBuf {
        let ns: String = ns.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        self.dir.join(if ns.is_empty() { "default".into() } else { ns }).join(format!("{}{SUFFIX}", hash_bytes(HashAlgorithm::Sha256, name.as_bytes())))
    }

    /// The cached sealed volume, when present and intact. `sha256` is the
    /// volume's recorded hash; without one nothing is served, because an
    /// unchecked entry could be stale.
    pub fn get(&self, ns: &str, name: &str, sha256: Option<&str>, size: Option<u64>) -> Option<Vec<u8>> {
        let Some(expected) = sha256.filter(|h| !h.is_empty()) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let p = self.entry_path(ns, name);
        let data = match std::fs::read(&p) {
            Ok(d) => d,
            Err(_) => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        if size.is_some_and(|s| s != data.len() as u64) || hash_bytes(HashAlgorithm::Sha256, &data) != expected {
            tracing::warn!("Volume cache entry for {name} is damaged; it is removed and downloaded again");
            let _ = std::fs::remove_file(&p);
            self.corrupt.fetch_add(1, Ordering::Relaxed);
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        // Refresh the access time for LRU.
        if let Ok(f) = std::fs::File::options().write(true).open(&p) {
            let _ = f.set_modified(SystemTime::now());
        }
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(data)
    }

    /// Store a sealed volume (the caller checked it), then evict down to the limit.
    pub fn put(&self, ns: &str, name: &str, data: &[u8]) -> Result<(), AppError> {
        if self.max_bytes == 0 || data.len() as u64 > self.max_bytes {
            return Ok(());
        }
        let p = self.entry_path(ns, name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = p.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, data)?;
        if let Err(e) = std::fs::rename(&tmp, &p) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        self.evict_to(self.max_bytes);
        Ok(())
    }

    /// Every entry: (path, size, last access).
    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let mut out = Vec::new();
        let Ok(namespaces) = std::fs::read_dir(&self.dir) else { return out };
        for ns in namespaces.flatten().filter(|e| e.path().is_dir()) {
            let Ok(files) = std::fs::read_dir(ns.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let Ok(md) = f.metadata() else { continue };
                if name.ends_with(SUFFIX) {
                    out.push((f.path(), md.len(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
                } else if name.contains(".tmp-") && md.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600) {
                    // Left behind by a crashed writer.
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        out
    }

    /// Remove least recently used entries until the total is at most `limit`.
    fn evict_to(&self, limit: u64) {
        let _g = self.evict.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        if total <= limit {
            return;
        }
        entries.sort_by_key(|e| e.2);
        for (p, size, _) in entries {
            if total <= limit {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total = total.saturating_sub(size);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn stats(&self) -> CacheStats {
        let entries = self.entries();
        let (hits, misses) = (self.hits.load(Ordering::Relaxed), self.misses.load(Ordering::Relaxed));
        CacheStats {
            dir: self.dir.to_string_lossy().into_owned(),
            max_bytes: self.max_bytes,
            entries: entries.len() as u64,
            bytes: entries.iter().map(|e| e.1).sum(),
            hits,
            misses,
            corrupt: self.corrupt.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            hit_rate: if hits + misses == 0 { 0.0 } else { hits as f64 / (hits + misses) as f64 },
        }
    }

    /// Delete every entry; returns the bytes freed.
    pub fn clear(&self) -> Result<u64, AppError> {
        let _g = self.evict.lock().unwrap_or_else(|e| e.into_inner());
        let mut freed = 0;
        for (p, size, _) in self.entries() {
            if std::fs::remove_file(&p).is_ok() {
                freed += size;
            }
        }
        Ok(freed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(d: &[u8]) -> String {
        hash_bytes(HashAlgorithm::Sha256, d)
    }

    fn age(c: &VolumeCache, ns: &str, name: &str, secs_ago: u64) {
        let f = std::fs::File::options().write(true).open(c.entry_path(ns, name)).unwrap();
        f.set_modified(SystemTime::now() - std::time::Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn hit_miss_and_namespaces() {
        let d = tempfile::tempdir().unwrap();
        let c = VolumeCache::new(d.path(), 1 << 20).unwrap();
        assert!(c.get("a", "v1", Some(&h(b"one")), None).is_none());
        c.put("a", "v1", b"one").unwrap();
        assert_eq!(c.get("a", "v1", Some(&h(b"one")), Some(3)).unwrap(), b"one");
        assert!(c.get("b", "v1", Some(&h(b"one")), None).is_none(), "other repository");
        assert!(c.get("a", "v1", None, None).is_none(), "never served without a hash to check");
        let s = c.stats();
        assert_eq!((s.entries, s.bytes, s.hits, s.misses), (1, 3, 1, 3));
        assert!((s.hit_rate - 0.25).abs() < 1e-9);
        assert_ne!(namespace("file:///x", "p"), namespace("file:///x", "q"));
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let d = tempfile::tempdir().unwrap();
        let c = VolumeCache::new(d.path(), 300).unwrap();
        let blob = |b: u8| vec![b; 100];
        c.put("n", "a", &blob(1)).unwrap();
        age(&c, "n", "a", 300);
        c.put("n", "b", &blob(2)).unwrap();
        age(&c, "n", "b", 200);
        c.put("n", "c", &blob(3)).unwrap();
        age(&c, "n", "c", 100);
        // Reading `a` makes it the most recently used.
        assert!(c.get("n", "a", Some(&h(&blob(1))), None).is_some());
        c.put("n", "d", &blob(4)).unwrap();
        assert!(c.get("n", "b", Some(&h(&blob(2))), None).is_none(), "b was the least recently used");
        assert!(c.get("n", "a", Some(&h(&blob(1))), None).is_some());
        assert!(c.get("n", "c", Some(&h(&blob(3))), None).is_some());
        assert!(c.get("n", "d", Some(&h(&blob(4))), None).is_some());
        let s = c.stats();
        assert!(s.bytes <= 300, "{s:?}");
        assert_eq!(s.evictions, 1);
        // Larger than the whole cache: not stored at all.
        c.put("n", "huge", &vec![0u8; 400]).unwrap();
        assert_eq!(c.stats().entries, 3);
    }

    #[test]
    fn damaged_entry_is_dropped_and_counted() {
        let d = tempfile::tempdir().unwrap();
        let c = VolumeCache::new(d.path(), 1 << 20).unwrap();
        c.put("n", "v", b"good data").unwrap();
        std::fs::write(c.entry_path("n", "v"), b"bad data!").unwrap();
        assert!(c.get("n", "v", Some(&h(b"good data")), None).is_none());
        assert!(!c.entry_path("n", "v").exists(), "removed so the next fetch replaces it");
        assert_eq!(c.stats().corrupt, 1);
        // Truncated entry with a size check.
        c.put("n", "w", b"0123456789").unwrap();
        assert!(c.get("n", "w", Some(&h(b"0123456789")), Some(11)).is_none());
    }

    #[test]
    fn clear_and_concurrent_writers() {
        let d = tempfile::tempdir().unwrap();
        let c = std::sync::Arc::new(VolumeCache::new(d.path(), 1 << 20).unwrap());
        std::thread::scope(|s| {
            for t in 0..8u8 {
                let c = c.clone();
                s.spawn(move || {
                    for i in 0..20u8 {
                        let data = vec![t ^ i; 1000];
                        c.put("n", &format!("v{}", i % 5), &data).unwrap();
                        // Whatever is there is always a complete entry.
                        if let Ok(read) = std::fs::read(c.entry_path("n", &format!("v{}", i % 5))) {
                            assert_eq!(read.len(), 1000);
                        }
                    }
                });
            }
        });
        assert_eq!(c.stats().entries, 5);
        assert_eq!(c.clear().unwrap(), 5000);
        assert_eq!(c.stats().entries, 0);
        // Off when the limit is 0.
        let off = VolumeCache::new(d.path().join("off"), 0).unwrap();
        off.put("n", "v", b"x").unwrap();
        assert_eq!(off.stats().entries, 0);
    }
}
