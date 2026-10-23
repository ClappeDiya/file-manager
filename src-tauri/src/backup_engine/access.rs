//! Reading backups without restoring them: resolve a version by time,
//! read or preview one file, export one file, and the local volume cache
//! those reads go through. The engine side is `ufop_core::backup::{select,
//! access, cache}`.

use super::{blocking, lock, lock_options, maintenance, versions, BackupJob, BackupManager};
use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use ufop_core::backup::access::{find_entry, open_file, BlockSource};
use ufop_core::backup::cache::{CacheSettings, CacheStats, VolumeCache};
use ufop_core::backup::db::BackupDb;
use ufop_core::backup::engine::Repo;
use ufop_core::backup::lock::{self as repo_lock, LockKind};
use ufop_core::backup::select::VersionSelector;
use ufop_core::backup::{Backend, EntryKind, Fileset, OpContext};

/// Config-table key of the volume cache settings.
pub const CACHE_KEY: &str = "backup.volume_cache";
/// Largest range one `read_file` call returns (preview).
pub const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
/// Largest file a preview copies to a temporary file.
pub const MAX_PREVIEW_BYTES: u64 = 512 * 1024 * 1024;

/// The version a read refers to: a selector, or a number (0 = newest).
pub fn selector_of(version: Option<usize>, selector: Option<VersionSelector>) -> VersionSelector {
    selector.unwrap_or(VersionSelector::Index { index: version.unwrap_or(0) })
}

/// A selector from query parameters: `time` ("as of", RFC 3339 or local
/// `2026-10-01 09:00`) wins over `version` (`latest`, `oldest`, a number
/// or a version id); neither = the newest version.
pub fn query_selector(version: Option<&str>, time: Option<&str>, allow_partial: bool) -> Result<VersionSelector, AppError> {
    VersionSelector::from_parts(version, time, allow_partial)
}

/// A backed-up file being streamed: what is sent and the data in pieces.
pub struct FileStream {
    pub path: String,
    pub size: u64,
    pub mtime: Option<i64>,
    pub etag: String,
    /// `Some((first, last))` for a satisfiable `Range` request.
    pub range: Option<(u64, u64)>,
    pub data: tokio::sync::mpsc::Receiver<Result<Vec<u8>, std::io::Error>>,
}

/// A resolved version: its number and the version itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedVersion {
    pub index: usize,
    pub fileset: Fileset,
}

/// Bytes of a backed-up file (base64), from `offset`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunk {
    pub path: String,
    pub version: usize,
    pub timestamp: String,
    /// Size of the whole file.
    pub size: u64,
    pub offset: u64,
    pub length: u64,
    pub data: String,
    /// More of the file follows (`offset + length < size`).
    pub truncated: bool,
    pub mime_type: String,
}

/// A backed-up file copied to a local file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFile {
    pub path: String,
    pub local_path: String,
    pub bytes: u64,
    pub version: usize,
    pub timestamp: String,
}

/// A repository opened for reading (inputs gathered on the async side).
pub(super) struct ReadSetup {
    pub job: BackupJob,
    pub backend: Arc<dyn Backend>,
    pub pass: Option<String>,
    pub db_path: PathBuf,
    pub db_key: Option<ufop_core::backup::db::DbKey>,
    pub db_encrypt: bool,
    pub kdf: ufop_core::backup::crypto::KdfParams,
    pub cache: Option<Arc<VolumeCache>>,
}

impl ReadSetup {
    /// Open the database and repository under a shared lock and run `f`.
    pub fn run<T>(&self, source: Option<&str>, name: &str, f: impl FnOnce(&Repo, &BackupDb) -> Result<T, AppError>) -> Result<T, AppError> {
        let job = versions::with_identity(&self.job);
        let prefix = job.options.prefix.as_str();
        let ctx = OpContext::new();
        let o = maintenance::JobDbOptions { key: self.db_key.as_ref(), encrypt: self.db_encrypt };
        let (db, _) = maintenance::open_job_db(&self.db_path, &o, Some(self.backend.as_ref()), self.pass.as_deref(), prefix, &ctx)?;
        let (v, _) = repo_lock::with_lock(self.backend.as_ref(), prefix, LockKind::Shared, name, &lock_options(&job.options), &ctx, || {
            let repo = Repo::open(self.backend.as_ref(), &db, self.pass.as_deref(), &job.options, false, self.kdf)?.with_cache(self.cache.clone());
            if let Some(s) = ufop_core::backup::shared::selector_scope(source) {
                db.set_scope(s);
            }
            f(&repo, &db)
        })?;
        Ok(v)
    }

    /// The job's database (for loaders that outlive [`ReadSetup::run`]).
    pub fn open_db(&self) -> Result<BackupDb, AppError> {
        let o = maintenance::JobDbOptions { key: self.db_key.as_ref(), encrypt: self.db_encrypt };
        Ok(maintenance::open_job_db(&self.db_path, &o, None, None, &self.job.options.prefix, &OpContext::new())?.0)
    }
}

fn default_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| ufop_core::session::user_temp_subdir("cache"))
}

fn mime_of(path: &str) -> String {
    ufop_core::webdav::content_type(path).split(';').next().unwrap_or("application/octet-stream").to_string()
}

fn resolve_in(db: &BackupDb, sel: &VersionSelector) -> Result<(usize, Fileset), AppError> {
    let sets = db.filesets()?;
    let i = sel.resolve(&sets)?;
    Ok((i, sets[i].clone()))
}

impl BackupManager {
    // ── Volume cache ──

    pub async fn cache_settings(&self) -> Result<CacheSettings, AppError> {
        Ok(self.config_get(CACHE_KEY).await?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub async fn set_cache_settings(&self, s: CacheSettings) -> Result<CacheStats, AppError> {
        if let Some(d) = s.dir.as_deref().filter(|d| !d.trim().is_empty()) {
            if !Path::new(d).is_absolute() {
                return Err(AppError::validation("The cache folder must be an absolute path"));
            }
        }
        self.config_set(CACHE_KEY, serde_json::to_string(&s)?).await?;
        *lock(&self.inner.volume_cache) = None;
        self.cache_stats().await
    }

    /// The process-wide volume cache (`None` when its size limit is 0).
    pub async fn volume_cache(&self) -> Result<Option<Arc<VolumeCache>>, AppError> {
        if let Some(c) = lock(&self.inner.volume_cache).clone() {
            return Ok(Some(c));
        }
        let s = self.cache_settings().await?;
        if s.max_bytes == 0 {
            return Ok(None);
        }
        let dir = s.dir.filter(|d| !d.trim().is_empty()).map(PathBuf::from).unwrap_or_else(|| default_cache_dir().join("backup-volumes"));
        let c = Arc::new(blocking(move || VolumeCache::new(dir, s.max_bytes)).await?);
        *lock(&self.inner.volume_cache) = Some(c.clone());
        Ok(Some(c))
    }

    pub async fn cache_stats(&self) -> Result<CacheStats, AppError> {
        let s = self.cache_settings().await?;
        match self.volume_cache().await? {
            Some(c) => blocking(move || Ok(c.stats())).await,
            None => Ok(CacheStats { dir: s.dir.unwrap_or_default(), ..Default::default() }),
        }
    }

    /// Delete every cached volume; returns the bytes freed.
    pub async fn clear_cache(&self) -> Result<u64, AppError> {
        match self.volume_cache().await? {
            Some(c) => blocking(move || c.clear()).await,
            None => Ok(0),
        }
    }

    /// Inputs for reading a job's repository.
    pub(super) async fn read_setup(&self, job_id: &str) -> Result<ReadSetup, AppError> {
        let job = self.resolve_job_secrets(&self.get_job(job_id).await?).await?;
        let backend: Arc<dyn Backend> = Arc::from(self.build_backend(&job).await?);
        let pass = if job.encrypted { self.passphrase(&job.id).await? } else { None };
        if job.encrypted && pass.is_none() {
            return Err(AppError::validation("This job's passphrase is missing from the credential store; edit the job to set it"));
        }
        let (db_path, db_key, db_encrypt) = self.open_job_db(&job).await?;
        let cache = self.volume_cache().await?;
        Ok(ReadSetup { job, backend, pass, db_path, db_key, db_encrypt, kdf: self.inner.kdf, cache })
    }

    // ── Versions by time ──

    /// The version a selector picks (as of a time, newest, oldest, number, id).
    pub async fn resolve_version(&self, job_id: &str, selector: VersionSelector, source: Option<String>) -> Result<ResolvedVersion, AppError> {
        self.with_db_in(job_id, source, move |db| resolve_in(db, &selector).map(|(index, fileset)| ResolvedVersion { index, fileset })).await
    }

    // ── One file ──

    /// Up to [`MAX_READ_BYTES`] of a backed-up file, from `offset`.
    pub async fn read_file(&self, job_id: &str, selector: VersionSelector, path: String, offset: u64, len: Option<u64>, source: Option<String>) -> Result<FileChunk, AppError> {
        let setup = self.read_setup(job_id).await?;
        let len = len.unwrap_or(MAX_READ_BYTES).min(MAX_READ_BYTES);
        blocking(move || {
            setup.run(source.as_deref(), "read_file", |repo, db| {
                let (index, fs) = resolve_in(db, &selector)?;
                let entries = db.files(fs.id)?;
                let e = find_entry(&entries, &path).ok_or_else(|| AppError::not_found(format!("{path} is not in this version")))?;
                let src = BlockSource::new(repo)?;
                let mut r = open_file(&src, e)?;
                let data = r.read_range(offset, len)?;
                Ok(FileChunk {
                    path: e.path.clone(),
                    version: index,
                    timestamp: fs.timestamp.clone(),
                    size: e.size,
                    offset,
                    length: data.len() as u64,
                    truncated: offset + (data.len() as u64) < e.size,
                    data: base64::engine::general_purpose::STANDARD.encode(&data),
                    mime_type: mime_of(&e.path),
                })
            })
        })
        .await
    }

    /// Stream one backed-up file (a `Range` header value selects one range;
    /// an unsatisfiable range is an error). The repository's shared lock is
    /// held until the stream ends.
    pub async fn stream_file(&self, job_id: &str, selector: VersionSelector, path: String, range: Option<String>, source: Option<String>) -> Result<FileStream, AppError> {
        let setup = self.read_setup(job_id).await?;
        let (meta_tx, meta_rx) = tokio::sync::oneshot::channel::<Result<(String, u64, Option<i64>, String, Option<(u64, u64)>), AppError>>();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::task::spawn_blocking(move || {
            let mut meta_tx = Some(meta_tx);
            let r = setup.run(source.as_deref(), "read_file", |repo, db| {
                let (_, fs) = resolve_in(db, &selector)?;
                let entries = db.files(fs.id)?;
                let e = find_entry(&entries, &path).ok_or_else(|| AppError::not_found(format!("{path} is not in this version")))?;
                let src = BlockSource::new(repo)?;
                let mut r = open_file(&src, e)?;
                let range = match ufop_core::webdav::parse_range(range.as_deref(), e.size) {
                    Some(Err(())) => return Err(AppError::validation(format!("Range not satisfiable (the file has {} bytes)", e.size))),
                    Some(Ok(r)) => Some(r),
                    None => None,
                };
                let (start, count) = range.map(|(a, b)| (a, b - a + 1)).unwrap_or((0, e.size));
                let etag = format!("\"{}\"", e.hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("0"));
                if let Some(m) = meta_tx.take() {
                    let _ = m.send(Ok((e.path.clone(), e.size, e.mtime, etag, range)));
                }
                use std::io::{Read, Seek};
                r.seek(std::io::SeekFrom::Start(start))?;
                let mut left = count;
                while left > 0 {
                    let mut buf = vec![0u8; (256 * 1024).min(left as usize)];
                    let n = match r.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(err) => {
                            let _ = tx.blocking_send(Err(err));
                            break;
                        }
                    };
                    buf.truncate(n);
                    left -= n as u64;
                    if tx.blocking_send(Ok(buf)).is_err() {
                        break; // the client went away
                    }
                }
                Ok(())
            });
            if let (Err(e), Some(m)) = (r, meta_tx.take()) {
                let _ = m.send(Err(e));
            }
        });
        let (path, size, mtime, etag, range) = meta_rx.await.map_err(|_| AppError::internal("file stream ended early"))??;
        Ok(FileStream { path, size, mtime, etag, range, data: rx })
    }

    /// Copy one backed-up file to `dest` (a file path; parent folders are created).
    pub async fn export_file(&self, job_id: &str, selector: VersionSelector, path: String, dest: String, source: Option<String>) -> Result<ExportedFile, AppError> {
        let setup = self.read_setup(job_id).await?;
        blocking(move || export_with(&setup, source.as_deref(), &selector, &path, Path::new(&dest), u64::MAX)).await
    }

    /// Copy a backed-up file to a temporary preview file and return it, for
    /// the app's preview pane. Earlier previews older than an hour are removed.
    pub async fn preview_file(&self, job_id: &str, selector: VersionSelector, path: String, source: Option<String>) -> Result<ExportedFile, AppError> {
        let setup = self.read_setup(job_id).await?;
        let dir = self.preview_dir();
        blocking(move || {
            cleanup_previews(&dir, std::time::Duration::from_secs(3600));
            let name = Path::new(&path.replace('\\', "/")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            let dest = dir.join(uuid::Uuid::new_v4().simple().to_string()).join(name);
            export_with(&setup, source.as_deref(), &selector, &path, &dest, MAX_PREVIEW_BYTES)
        })
        .await
    }

    /// Remove every preview file (app exit).
    pub fn cleanup_all_previews(&self) {
        cleanup_previews(&self.preview_dir(), std::time::Duration::ZERO);
    }

    /// Temporary copies for the preview pane (inside the app's data folder).
    fn preview_dir(&self) -> PathBuf {
        self.inner.data_dir.join("preview")
    }
}

fn cleanup_previews(dir: &Path, older_than: std::time::Duration) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map_or(true, |age| age >= older_than);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn export_with(setup: &ReadSetup, source: Option<&str>, selector: &VersionSelector, path: &str, dest: &Path, max: u64) -> Result<ExportedFile, AppError> {
    setup.run(source, "export_file", |repo, db| {
        let (index, fs) = resolve_in(db, selector)?;
        let entries = db.files(fs.id)?;
        let e = find_entry(&entries, path).ok_or_else(|| AppError::not_found(format!("{path} is not in this version")))?;
        if e.kind != EntryKind::File {
            return Err(AppError::validation(format!("{path} is not a file")));
        }
        if e.size > max {
            return Err(AppError::validation(format!("{path} is too large to preview ({} bytes); export it instead", e.size)));
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let src = BlockSource::new(repo)?;
        let mut r = open_file(&src, e)?;
        let tmp = dest.with_file_name(format!(".{}.ufop-export", dest.file_name().unwrap_or_default().to_string_lossy()));
        let res = (|| -> Result<u64, AppError> {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            let n = r.copy_to(&mut f)?;
            std::io::Write::flush(&mut f)?;
            Ok(n)
        })();
        let bytes = match res {
            Ok(n) => n,
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(err);
            }
        };
        std::fs::rename(&tmp, dest)?;
        if let Some(m) = e.mtime {
            if let Ok(f) = std::fs::File::options().write(true).open(dest) {
                let _ = f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(m.max(0) as u64));
            }
        }
        Ok(ExportedFile { path: e.path.clone(), local_path: dest.to_string_lossy().into_owned(), bytes, version: index, timestamp: fs.timestamp.clone() })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_default_to_the_version_number() {
        assert_eq!(selector_of(None, None), VersionSelector::Index { index: 0 });
        assert_eq!(selector_of(Some(3), None), VersionSelector::Index { index: 3 });
        assert_eq!(selector_of(Some(3), Some(VersionSelector::Latest { allow_partial: false })), VersionSelector::Latest { allow_partial: false });
        assert_eq!(mime_of("a.PNG"), "image/png");
        assert_eq!(mime_of("a.bin"), "application/octet-stream");
        assert!(matches!(query_selector(Some("2"), Some("2026-10-01T09:00:00Z"), false).unwrap(), VersionSelector::AsOf { .. }), "time wins");
    }

    #[test]
    fn old_previews_are_cleaned() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a")).unwrap();
        std::fs::write(d.path().join("a/f.txt"), b"x").unwrap();
        cleanup_previews(d.path(), std::time::Duration::from_secs(3600));
        assert!(d.path().join("a/f.txt").exists(), "recent previews stay");
        cleanup_previews(d.path(), std::time::Duration::ZERO);
        assert!(!d.path().join("a").exists());
    }
}
