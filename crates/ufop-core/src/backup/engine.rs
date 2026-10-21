//! Backup run: scan → deduplicate → pack/compress/encrypt → upload → write
//! the version's file list → retention → compaction → sampled verification.
//!
//! Files are read, chunked, hashed and compressed by a pool of worker
//! threads; the coordinating thread deduplicates blocks and packs volumes;
//! sealed volumes are uploaded by a separate pool (with PAR2 recovery data
//! when enabled). The local database is only touched by the coordinating
//! thread.

use super::backend::Backend;
use super::crypto::{KdfParams, VolumeCipher};
use super::db::BackupDb;
use super::filters::SourceFilter;
use super::par2::{self, Par2};
use super::retention::RetentionPolicy;
use super::volume::{self, BlockVolumeWriter, ControlFile, FileListVolume, IndexVolume, IndexedBlock, RepoHeader, VolumeKind};
use super::{
    hash_bytes, meta, BackupOptions, EntryKind, FileEntry, FileHasher, HashAlgorithm, MissingSourcePolicy, OpContext, Phase, Progress, SymlinkPolicy,
    UsnPolicy, REPO_FORMAT_VERSION,
};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

/// An opened repository: backend + local database + key material.
pub struct Repo<'a> {
    pub backend: &'a dyn Backend,
    pub db: &'a BackupDb,
    pub cipher: VolumeCipher,
    pub header: RepoHeader,
    pub prefix: String,
    pub par2: Option<Par2>,
    pub case_insensitive: bool,
}

impl<'a> Repo<'a> {
    /// Open (or, with `create`, initialise) the repository at `backend`.
    /// The block size, hash algorithms and volume format of an existing
    /// repository win over `options`, because they cannot change later.
    pub fn open(
        backend: &'a dyn Backend,
        db: &'a BackupDb,
        passphrase: Option<&str>,
        options: &BackupOptions,
        create: bool,
        kdf: KdfParams,
    ) -> Result<Self, AppError> {
        Self::open_with_cipher(backend, db, passphrase, None, options, create, kdf)
    }

    /// Like [`Repo::open`], with an optional external encryption module
    /// (e.g. GPG) instead of the passphrase-derived AES key.
    pub fn open_with_cipher(
        backend: &'a dyn Backend,
        db: &'a BackupDb,
        passphrase: Option<&str>,
        external: Option<VolumeCipher>,
        options: &BackupOptions,
        create: bool,
        kdf: KdfParams,
    ) -> Result<Self, AppError> {
        let header_name = volume::repo_header_name(&options.prefix);
        let listing = if create && !options.dry_run {
            backend.create_folder()?;
            backend.list()?
        } else {
            match backend.list() {
                Ok(l) => l,
                // A brand-new destination folder: a dry run must not create it.
                Err(_) if create => Vec::new(),
                Err(e) => return Err(e),
            }
        };
        let found = listing.iter().find(|f| f.name == header_name || (options.remote_case_insensitive && f.name.eq_ignore_ascii_case(&header_name)));
        let (header, cipher) = if let Some(hf) = found {
            let header: RepoHeader = serde_json::from_slice(&backend.get(&hf.name)?)?;
            if header.format_version > REPO_FORMAT_VERSION {
                return Err(AppError::validation(format!("This backup was written by a newer version (format {}); update the app", header.format_version)));
            }
            let cipher = match (header.encrypted, external) {
                (true, Some(ext)) => ext,
                (true, None) => {
                    let pass = passphrase.filter(|p| !p.is_empty()).ok_or_else(|| AppError::validation("This backup is encrypted; enter its passphrase"))?;
                    let salt = hex::decode(header.salt.as_deref().unwrap_or_default()).map_err(|_| AppError::validation("Repository header salt is malformed"))?;
                    let c = VolumeCipher::derive(pass, &salt, header.kdf.unwrap_or_default())?;
                    c.verify_key_check(header.key_check.as_deref().unwrap_or_default())?;
                    c
                }
                (false, _) => VolumeCipher::plain(),
            };
            (header, cipher)
        } else {
            if !create {
                return Err(AppError::file_op(format!("No backup found at {}", backend.describe()), "Check the destination and file-name prefix."));
            }
            let (cipher, salt) = match (external, passphrase.filter(|p| !p.is_empty())) {
                (Some(ext), _) => (ext, None),
                (None, Some(p)) => {
                    let salt = VolumeCipher::random_salt();
                    (VolumeCipher::derive(p, &salt, kdf)?, Some(hex::encode(&salt)))
                }
                (None, None) => (VolumeCipher::plain(), None),
            };
            let aes = salt.is_some();
            let header = RepoHeader {
                format_version: REPO_FORMAT_VERSION,
                encrypted: cipher.is_encrypted(),
                salt,
                kdf: aes.then_some(kdf),
                key_check: if aes { cipher.key_check()? } else { None },
                block_size: options.block_size.max(1024),
                block_hash: options.block_hash,
                file_hash: options.file_hash,
                volume_format: options.volume_format,
            };
            if !options.dry_run {
                backend.create_folder()?;
                backend.put(&header_name, &serde_json::to_vec_pretty(&header)?)?;
            }
            (header, cipher)
        };
        let par2 = options.par2.enabled.then(|| Par2::new(options.par2.clone(), options.temp_root()));
        Ok(Self { backend, db, cipher, header, prefix: options.prefix.clone(), par2, case_insensitive: options.remote_case_insensitive })
    }

    fn par2_key(name: &str) -> String {
        format!("par2:{name}")
    }

    /// Upload PAR2 recovery files for a volume and remember their names.
    pub(crate) fn put_recovery(&self, name: &str, data: &[u8]) -> Result<(), AppError> {
        let Some(p) = self.par2.as_ref() else { return Ok(()) };
        let files = p.create(name, data)?;
        for (n, d) in &files {
            self.backend.put(n, d)?;
        }
        let names: Vec<&String> = files.iter().map(|(n, _)| n).collect();
        self.db.set_kv(&Self::par2_key(name), &serde_json::to_string(&names)?)
    }

    pub(crate) fn put_volume(&self, kind: VolumeKind, name: &str, data: &[u8], dry_run: bool) -> Result<i64, AppError> {
        let h = hash_bytes(HashAlgorithm::Sha256, data);
        if dry_run {
            return Ok(-1);
        }
        self.db.add_volume(name, kind, data.len() as u64, &h, "uploading")?;
        self.backend.put(name, data)?;
        if kind != VolumeKind::Index {
            self.put_recovery(name, data)?;
        }
        self.db.set_volume_state(name, "uploaded")?;
        Ok(self.db.volume_by_name(name)?.map(|v| v.id).unwrap_or(-1))
    }

    /// Delete a volume, its index link and PAR2 files remotely and locally.
    pub(crate) fn delete_volume_files(&self, name: &str) -> Result<(), AppError> {
        self.backend.delete(name)?;
        if let Some(list) = self.db.get_kv(&Self::par2_key(name))? {
            for n in serde_json::from_str::<Vec<String>>(&list).unwrap_or_default() {
                let _ = self.backend.delete(&n);
            }
        }
        self.db.delete_volume(name)
    }

    /// Download a volume, check its recorded SHA-256 and, when PAR2 data
    /// exists, repair a damaged copy (restore / compact paths).
    pub(crate) fn get_volume(&self, name: &str) -> Result<Vec<u8>, AppError> {
        self.get_volume_ex(name, true)
    }

    /// Verification uses `allow_repair = false` so damage is reported.
    pub(crate) fn get_volume_ex(&self, name: &str, allow_repair: bool) -> Result<Vec<u8>, AppError> {
        let (hash, rec) = self.volume_check_info(name)?;
        self.fetcher().fetch(name, hash.as_deref(), &rec, allow_repair)
    }

    /// Recorded hash and PAR2 file names of a volume (for [`VolumeFetcher`]).
    pub(crate) fn volume_check_info(&self, name: &str) -> Result<(Option<String>, Vec<String>), AppError> {
        let hash = self.db.volume_by_name(name)?.map(|v| v.hash).filter(|h| !h.is_empty());
        let rec = self.db.get_kv(&Self::par2_key(name))?.and_then(|l| serde_json::from_str(&l).ok()).unwrap_or_default();
        Ok((hash, rec))
    }

    /// Database-free volume downloader that worker threads can share.
    pub(crate) fn fetcher(&self) -> VolumeFetcher<'_> {
        VolumeFetcher { backend: self.backend, par2: self.par2.as_ref(), case_insensitive: self.case_insensitive }
    }

    /// Write a sealed file list for `files` and record the version.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_filelist(
        &self,
        ts: DateTime<Utc>,
        partial: bool,
        files: &[FileEntry],
        task_config: Option<serde_json::Value>,
        control_files: Vec<ControlFile>,
        dry_run: bool,
    ) -> Result<String, AppError> {
        let name = volume::filelist_volume_name(&self.prefix, ts, &self.cipher);
        let fl = FileListVolume { timestamp: ts.to_rfc3339(), partial, files: files.to_vec(), task_config, control_files };
        let data = volume::seal_json(&self.cipher, &fl)?;
        self.put_volume(VolumeKind::FileList, &name, &data, dry_run)?;
        if !dry_run {
            self.db.add_fileset(&fl.timestamp, &name, partial, files)?;
            // Kept so rewrites (purge, repair) preserve them.
            self.db.set_kv(&format!("dlist_extra:{}", fl.timestamp), &serde_json::to_string(&DlistExtra { task_config: fl.task_config, control_files: fl.control_files })?)?;
        }
        Ok(fl.timestamp)
    }

    /// Re-upload a version's file list (same name), preserving extras.
    pub(crate) fn rewrite_filelist(&self, timestamp: &str, partial: bool, volume_name: &str, files: &[FileEntry]) -> Result<Vec<u8>, AppError> {
        let extra: DlistExtra = self.db.get_kv(&format!("dlist_extra:{timestamp}"))?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let fl = FileListVolume { timestamp: timestamp.to_string(), partial, files: files.to_vec(), task_config: extra.task_config, control_files: extra.control_files };
        let data = volume::seal_json(&self.cipher, &fl)?;
        self.backend.put(volume_name, &data)?;
        self.put_recovery(volume_name, &data)?;
        self.db.add_volume(volume_name, VolumeKind::FileList, data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
        Ok(data)
    }

    /// Compare the local volume inventory with the remote listing.
    pub fn consistency_check(&self) -> Result<ConsistencyReport, AppError> {
        let norm = |n: &str| if self.case_insensitive { n.to_lowercase() } else { n.to_string() };
        let remote: HashMap<String, u64> = self.backend.list()?.into_iter().map(|f| (norm(&f.name), f.size)).collect();
        let mut r = ConsistencyReport::default();
        let vols = self.db.volumes(None)?;
        for v in &vols {
            match remote.get(&norm(&v.name)) {
                None if v.state == "uploaded" || v.state == "verified" => r.missing.push(v.name.clone()),
                None => r.incomplete.push(v.name.clone()),
                Some(sz) if *sz != v.size => r.size_mismatch.push(v.name.clone()),
                _ => {}
            }
        }
        let known: HashSet<String> = vols.into_iter().map(|v| norm(&v.name)).collect();
        for name in remote.keys() {
            if volume::classify(&norm(&self.prefix), name).is_some() && !known.contains(name) {
                r.extra.push(name.clone());
            }
        }
        r.db_problems = self.db.integrity_problems()?;
        Ok(r)
    }
}

/// Downloads a volume, checks it against its recorded SHA-256 and, when
/// PAR2 recovery files exist, repairs (and re-uploads) a damaged copy.
pub(crate) struct VolumeFetcher<'a> {
    backend: &'a dyn Backend,
    par2: Option<&'a Par2>,
    case_insensitive: bool,
}

impl VolumeFetcher<'_> {
    pub(crate) fn fetch(&self, name: &str, expected: Option<&str>, recovery: &[String], allow_repair: bool) -> Result<Vec<u8>, AppError> {
        let data = self.backend.get(name).or_else(|e| {
            // Case-changing backends: retry with the listed spelling.
            if self.case_insensitive {
                if let Some(actual) = self.backend.list()?.into_iter().find(|f| f.name.eq_ignore_ascii_case(name)) {
                    return self.backend.get(&actual.name);
                }
            }
            Err(e)
        })?;
        let Some(expected) = expected else { return Ok(data) };
        if expected == hash_bytes(HashAlgorithm::Sha256, &data) {
            return Ok(data);
        }
        if allow_repair && !recovery.is_empty() {
            let fallback;
            let p = match self.par2 {
                Some(p) => p,
                None => {
                    fallback = Par2::new(Default::default(), std::env::temp_dir());
                    &fallback
                }
            };
            let rec: Vec<(String, Vec<u8>)> = recovery.iter().filter_map(|n| self.backend.get(n).ok().map(|d| (n.clone(), d))).collect();
            if !rec.is_empty() {
                if let Ok(fixed) = p.repair(name, &data, &rec) {
                    if hash_bytes(HashAlgorithm::Sha256, &fixed) == expected {
                        tracing::warn!("Repaired damaged volume {name} with PAR2 data");
                        // Heal the remote copy too.
                        let _ = self.backend.put(name, &fixed);
                        return Ok(fixed);
                    }
                }
            }
        }
        Err(AppError::transfer(
            format!("Remote volume {name} does not match its recorded hash"),
            "The file was modified or damaged on the destination. Run Repair or list-broken-files.",
        ))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DlistExtra {
    task_config: Option<serde_json::Value>,
    #[serde(default)]
    control_files: Vec<ControlFile>,
}

/// Result of [`Repo::consistency_check`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsistencyReport {
    /// Recorded as uploaded but absent remotely.
    pub missing: Vec<String>,
    /// Upload never finished.
    pub incomplete: Vec<String>,
    pub size_mismatch: Vec<String>,
    /// Remote volumes the local database does not know.
    pub extra: Vec<String>,
    pub db_problems: Vec<String>,
}

impl ConsistencyReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.size_mismatch.is_empty() && self.db_problems.is_empty()
    }
}

/// Summary of one backup run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupReport {
    /// Timestamp of the version created, if any.
    pub version: Option<String>,
    pub skipped_unchanged: bool,
    pub partial: bool,
    pub dry_run: bool,
    pub files_examined: u64,
    pub files_changed: u64,
    pub files_added: u64,
    pub files_deleted: u64,
    pub bytes_examined: u64,
    pub bytes_uploaded: u64,
    pub new_blocks: u64,
    pub volumes_uploaded: u64,
    pub versions_deleted: u64,
    pub volumes_compacted: u64,
    pub volumes_verified: u64,
    /// Paths rescanned in incremental (changed-path / USN) mode.
    pub rescanned_paths: u64,
    pub usn_used: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}

pub(crate) struct Pending {
    writer: BlockVolumeWriter,
    sizes: Vec<(String, u64)>,
    hashes: HashSet<String>,
}

impl Pending {
    pub(crate) fn for_repo(repo: &Repo, opts: &BackupOptions) -> Self {
        Self { writer: BlockVolumeWriter::with_format(repo.header.volume_format, opts.zip_method), sizes: Vec::new(), hashes: HashSet::new() }
    }

    pub(crate) fn add_block(&mut self, hash: String, data: &[u8], level: u32) -> Result<(), AppError> {
        let (flag, enc) = volume::encode_block(self.writer.format(), data, level)?;
        self.add_encoded(hash, flag, enc, data.len() as u64);
        Ok(())
    }

    pub(crate) fn add_encoded(&mut self, hash: String, flag: u8, data: Vec<u8>, raw_len: u64) {
        self.writer.add_encoded(hash.clone(), flag, data);
        self.sizes.push((hash.clone(), raw_len));
        self.hashes.insert(hash);
    }

    pub(crate) fn byte_size(&self) -> usize {
        self.writer.byte_size()
    }

    fn take(&mut self) -> Option<(BlockVolumeWriter, Vec<(String, u64)>)> {
        if self.writer.is_empty() {
            return None;
        }
        let fmt = self.writer.format();
        let w = std::mem::replace(&mut self.writer, BlockVolumeWriter::with_format(fmt, Default::default()));
        self.hashes.clear();
        Some((w, std::mem::take(&mut self.sizes)))
    }
}

/// A sealed block volume plus its index, ready to upload.
struct SealedVolume {
    block_name: String,
    block_data: Vec<u8>,
    index_name: String,
    index_data: Vec<u8>,
    blocks: Vec<(String, u64)>,
}

fn seal_pending(repo: &Repo, writer: BlockVolumeWriter, blocks: Vec<(String, u64)>) -> Result<SealedVolume, AppError> {
    let block_name = volume::block_volume_name(&repo.prefix, &repo.cipher);
    let block_data = repo.cipher.seal(&writer.finish())?;
    let idx = IndexVolume { block_volume: block_name.clone(), blocks: blocks.iter().map(|(h, s)| IndexedBlock { hash: h.clone(), size: *s }).collect() };
    let index_name = volume::index_volume_name(&repo.prefix, &repo.cipher);
    let index_data = volume::seal_json(&repo.cipher, &idx)?;
    Ok(SealedVolume { block_name, block_data, index_name, index_data, blocks })
}

/// Record an uploaded volume in the database (coordinating thread only).
fn record_volume(repo: &Repo, v: &SealedVolume, par2_names: &[String]) -> Result<(), AppError> {
    let vid = repo.db.add_volume(&v.block_name, VolumeKind::Block, v.block_data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &v.block_data), "uploaded")?;
    repo.db.add_volume(&v.index_name, VolumeKind::Index, v.index_data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &v.index_data), "uploaded")?;
    repo.db.set_kv(&format!("index:{}", v.block_name), &v.index_name)?;
    if !par2_names.is_empty() {
        repo.db.set_kv(&Repo::par2_key(&v.block_name), &serde_json::to_string(par2_names)?)?;
    }
    repo.db.begin()?;
    for (h, s) in &v.blocks {
        repo.db.add_block(h, *s, vid)?;
    }
    repo.db.commit()
}

/// Synchronous flush (used by compaction and the final flush).
pub(crate) fn flush(repo: &Repo, p: &mut Pending, opts: &BackupOptions, report: &mut BackupReport) -> Result<(), AppError> {
    let Some((w, blocks)) = p.take() else { return Ok(()) };
    let v = seal_pending(repo, w, blocks)?;
    report.bytes_uploaded += (v.block_data.len() + v.index_data.len()) as u64;
    report.volumes_uploaded += 1;
    if opts.dry_run {
        return Ok(());
    }
    repo.db.add_volume(&v.block_name, VolumeKind::Block, v.block_data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &v.block_data), "uploading")?;
    repo.backend.put(&v.block_name, &v.block_data)?;
    let par2_names = match &repo.par2 {
        Some(p) => upload_recovery(repo.backend, p, &v.block_name, &v.block_data)?,
        None => Vec::new(),
    };
    repo.backend.put(&v.index_name, &v.index_data)?;
    record_volume(repo, &v, &par2_names)
}

fn upload_recovery(backend: &dyn Backend, p: &Par2, name: &str, data: &[u8]) -> Result<Vec<String>, AppError> {
    let files = p.create(name, data)?;
    for (n, d) in &files {
        backend.put(n, d)?;
    }
    Ok(files.into_iter().map(|(n, _)| n).collect())
}

pub(crate) fn mtime_of(md: &std::fs::Metadata) -> Option<i64> {
    md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
}

#[allow(unused_variables)]
pub(crate) fn mode_of(md: &std::fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return Some(md.permissions().mode());
    }
    #[allow(unreachable_code)]
    None
}

struct Scanner<'f> {
    filter: &'f SourceFilter,
    opts: &'f BackupOptions,
    #[cfg_attr(not(unix), allow(dead_code))]
    seen_inodes: HashSet<(u64, u64)>,
    out: Vec<(PathBuf, FileEntry)>,
    warnings: Vec<String>,
}

impl Scanner<'_> {
    fn entry(&self, path: &Path, kind: EntryKind, md: &std::fs::Metadata) -> FileEntry {
        let o = self.opts;
        let meta = (!o.skip_metadata && (o.backup_xattrs || o.backup_acls))
            .then(|| meta::capture(path, meta::MetaOptions { xattrs: o.backup_xattrs, acls: o.backup_acls }))
            .filter(|m| !m.is_empty());
        FileEntry {
            path: path.to_string_lossy().into_owned(),
            kind,
            size: if kind == EntryKind::File { md.len() } else { 0 },
            mtime: if o.skip_metadata { None } else { mtime_of(md) },
            mode: if o.skip_metadata { None } else { mode_of(md) },
            hash: None,
            blocks: Vec::new(),
            symlink_target: None,
            meta,
        }
    }

    #[allow(unused_variables)]
    fn hardlink_skip(&mut self, md: &std::fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if md.nlink() > 1 {
                use super::HardlinkPolicy;
                match self.opts.hardlink_policy {
                    HardlinkPolicy::All => {}
                    HardlinkPolicy::None => return true,
                    HardlinkPolicy::First => return !self.seen_inodes.insert((md.dev(), md.ino())),
                }
            }
        }
        false
    }

    /// Add one entry; returns the metadata when it is a directory to descend into.
    fn visit(&mut self, path: &Path) -> Option<std::fs::Metadata> {
        let lmd = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                self.warnings.push(format!("Cannot read {}: {e}", path.display()));
                return None;
            }
        };
        if !self.opts.ignore_exclusion_xattrs && meta::has_exclusion_attribute(path) {
            return None;
        }
        let md = if lmd.file_type().is_symlink() {
            match self.opts.symlink_policy {
                SymlinkPolicy::Ignore => return None,
                SymlinkPolicy::Store => {
                    if self.filter.evaluate(&path.to_string_lossy(), false) == Some(super::FilterAction::Exclude) {
                        return None;
                    }
                    let mut e = self.entry(path, EntryKind::Symlink, &lmd);
                    e.symlink_target = std::fs::read_link(path).ok().map(|t| t.to_string_lossy().into_owned());
                    self.out.push((path.to_path_buf(), e));
                    return None;
                }
                SymlinkPolicy::Follow => match std::fs::metadata(path) {
                    Ok(m) => m,
                    Err(e) => {
                        self.warnings.push(format!("Broken link {}: {e}", path.display()));
                        return None;
                    }
                },
            }
        } else {
            lmd
        };
        if !self.filter.accepts(path, &md) {
            return None;
        }
        if md.is_dir() {
            self.out.push((path.to_path_buf(), self.entry(path, EntryKind::Dir, &md)));
            return Some(md);
        }
        if md.is_file() && !self.hardlink_skip(&md) {
            self.out.push((path.to_path_buf(), self.entry(path, EntryKind::File, &md)));
            if self.opts.backup_alternate_streams {
                for (name, size) in meta::alternate_streams(path) {
                    let sp = PathBuf::from(format!("{}:{name}", path.to_string_lossy()));
                    let mut e = self.entry(path, EntryKind::File, &md);
                    e.path = sp.to_string_lossy().into_owned();
                    e.size = size;
                    e.meta = None;
                    self.out.push((sp, e));
                }
            }
        }
        None
    }

    fn children(&mut self, path: &Path) -> Vec<PathBuf> {
        match std::fs::read_dir(path) {
            Ok(rd) => {
                let mut c: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
                c.sort();
                c
            }
            Err(e) => {
                self.warnings.push(format!("Cannot list {}: {e}", path.display()));
                Vec::new()
            }
        }
    }

    fn walk(&mut self, path: &Path, depth: usize) {
        if depth > 256 {
            self.warnings.push(format!("Skipping {}: directory nesting too deep", path.display()));
            return;
        }
        if self.visit(path).is_some() {
            for c in self.children(path) {
                self.walk(&c, depth + 1);
            }
        }
    }
}

/// Scanned entries (local path + entry) and warnings.
pub type ScanResult = (Vec<(PathBuf, FileEntry)>, Vec<String>);

fn check_sources<'a>(sources: &'a [PathBuf], opts: &BackupOptions, warnings: &mut Vec<String>) -> Result<Vec<&'a PathBuf>, AppError> {
    let mut ok = Vec::new();
    for src in sources {
        if std::fs::symlink_metadata(src).is_err() {
            match opts.missing_source {
                MissingSourcePolicy::Fail => {
                    return Err(AppError::file_op(
                        format!("Source {} does not exist", src.display()),
                        "Reconnect the drive or remove the source from the job (or set missing sources to 'warn').",
                    ))
                }
                MissingSourcePolicy::Warn => {
                    warnings.push(format!("Source {} is missing; skipped", src.display()));
                    continue;
                }
            }
        }
        ok.push(src);
    }
    Ok(ok)
}

fn drop_empty_dirs(out: &mut Vec<(PathBuf, FileEntry)>) {
    let file_paths: Vec<PathBuf> = out.iter().filter(|(_, e)| e.kind != EntryKind::Dir).map(|(p, _)| p.clone()).collect();
    out.retain(|(p, e)| e.kind != EntryKind::Dir || file_paths.iter().any(|f| f.starts_with(p) && f != p));
}

/// Enumerate the selected source entries (also used for dry-run previews).
pub fn scan_sources(sources: &[PathBuf], filter: &SourceFilter, opts: &BackupOptions) -> Result<ScanResult, AppError> {
    let mut s = Scanner { filter, opts, seen_inodes: HashSet::new(), out: Vec::new(), warnings: Vec::new() };
    for src in check_sources(sources, opts, &mut s.warnings)? {
        s.walk(src, 0);
    }
    let mut out = s.out;
    if filter.exclude_empty_folders {
        drop_empty_dirs(&mut out);
    }
    Ok((out, s.warnings))
}

/// Incremental scan: start from the previous version, then rescan only
/// `changed` paths (recursively), `shallow` folders (direct children) and
/// drop `deleted` paths.
pub fn scan_incremental(
    sources: &[PathBuf],
    prev: &[FileEntry],
    changed: &[String],
    shallow: &[String],
    deleted: &[String],
    filter: &SourceFilter,
    opts: &BackupOptions,
) -> Result<(ScanResult, u64), AppError> {
    let mut s = Scanner { filter, opts, seen_inodes: HashSet::new(), out: Vec::new(), warnings: Vec::new() };
    let roots = check_sources(sources, opts, &mut s.warnings)?;
    let in_sources = |p: &Path| roots.iter().any(|r| p.starts_with(r));
    let changed: Vec<PathBuf> = changed.iter().map(PathBuf::from).filter(|p| in_sources(p)).collect();
    let shallow: Vec<PathBuf> = shallow.iter().map(PathBuf::from).filter(|p| in_sources(p)).collect();
    let deleted: Vec<PathBuf> = deleted.iter().map(PathBuf::from).collect();

    // Keep previous entries outside the rescanned areas.
    let mut kept: Vec<(PathBuf, FileEntry)> = prev
        .iter()
        .filter(|e| {
            let p = Path::new(&e.path);
            !changed.iter().chain(deleted.iter()).any(|c| p.starts_with(c)) && !shallow.iter().any(|d| p.parent() == Some(d.as_path()))
        })
        .map(|e| (PathBuf::from(&e.path), e.clone()))
        .collect();
    let prev_dirs: HashSet<&str> = prev.iter().filter(|e| e.kind == EntryKind::Dir).map(|e| e.path.as_str()).collect();
    let mut rescanned = 0u64;
    for c in &changed {
        if c.exists() {
            s.walk(c, 0);
            rescanned += 1;
        }
    }
    for d in &shallow {
        if !d.is_dir() {
            // The folder itself went away: drop its subtree.
            kept.retain(|(p, _)| !p.starts_with(d));
            continue;
        }
        rescanned += 1;
        let now: Vec<PathBuf> = s.children(d);
        for child in &now {
            let was_dir = prev_dirs.contains(child.to_string_lossy().as_ref());
            if s.visit(child).is_some() && !was_dir {
                // New folder: scan it fully.
                for g in s.children(child) {
                    s.walk(&g, 1);
                }
            }
        }
        // Sub-folders that disappeared take their subtree with them.
        let now_set: HashSet<&PathBuf> = now.iter().collect();
        let gone: Vec<PathBuf> = prev_dirs.iter().map(PathBuf::from).filter(|p| p.parent() == Some(d.as_path()) && !now_set.contains(p)).collect();
        kept.retain(|(p, _)| !gone.iter().any(|g| p.starts_with(g)));
    }
    // Fresh entries first so the stable sort + dedup keeps them over stale ones.
    let mut fresh = std::mem::take(&mut s.out);
    fresh.append(&mut kept);
    let mut kept = fresh;
    kept.sort_by(|a, b| a.0.cmp(&b.0));
    kept.dedup_by(|a, b| a.0 == b.0);
    if filter.exclude_empty_folders {
        drop_empty_dirs(&mut kept);
    }
    Ok(((kept, s.warnings), rescanned))
}

fn compress_level(path: &Path, opts: &BackupOptions) -> u32 {
    let skip = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| opts.no_compress_extensions.iter().any(|x| x.eq_ignore_ascii_case(&e)));
    if skip {
        0
    } else {
        opts.compression_level
    }
}

fn unchanged(prev: &FileEntry, cur: &FileEntry, opts: &BackupOptions) -> bool {
    if opts.disable_timestamp_check || prev.kind != EntryKind::File || prev.mtime.is_none() {
        return false;
    }
    prev.mtime == cur.mtime && (opts.timestamps_only || prev.size == cur.size)
}

fn same_version(a: &[FileEntry], b: &[FileEntry]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.path == y.path && x.kind == y.kind && x.size == y.size && x.mtime == y.mtime && x.hash == y.hash && x.symlink_target == y.symlink_target && x.mode == y.mode && x.meta == y.meta
        })
}

fn progress(ctx: &OpContext, phase: Phase, done: u64, total: u64, bytes: u64, cur: Option<&str>) {
    ctx.report(&Progress { phase, files_processed: done, files_total: total, bytes_processed: bytes, current_path: cur.map(String::from) });
}

/// Next version timestamp, strictly after the newest existing one.
fn next_timestamp(db: &BackupDb) -> Result<DateTime<Utc>, AppError> {
    use chrono::Timelike;
    let now = Utc::now();
    let now = now.with_nanosecond(0).unwrap_or(now);
    let latest = db.filesets()?.first().and_then(|f| DateTime::parse_from_rfc3339(&f.timestamp).ok()).map(|t| t.with_timezone(&Utc));
    Ok(match latest {
        Some(l) if l >= now => l + chrono::Duration::seconds(1),
        _ => now,
    })
}

// ── Worker pipeline ──

enum Msg {
    Block { idx: usize, hash: String, flag: u8, data: Vec<u8>, raw_len: u64 },
    Done { idx: usize, size: u64, hash: String },
    Failed { idx: usize, error: String },
}

struct WorkerCfg {
    block_size: usize,
    read_buf: usize,
    block_hash: HashAlgorithm,
    file_hash: HashAlgorithm,
    format: super::VolumeFormat,
    respect_locks: bool,
    background: bool,
    /// Pause after each block (CPU intensity below 10).
    pause: std::time::Duration,
}

fn read_file(cfg: &WorkerCfg, idx: usize, path: &Path, level: u32, tx: &mpsc::SyncSender<Msg>, stop: &dyn Fn() -> bool) -> Result<(), String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if cfg.respect_locks && meta::is_advisory_locked(&f) {
        return Err("locked by another program (advisory lock)".into());
    }
    let mut f = std::io::BufReader::with_capacity(cfg.read_buf.max(cfg.block_size), f);
    let mut buf = vec![0u8; cfg.block_size];
    let mut fh = FileHasher::new(cfg.file_hash);
    let mut size = 0u64;
    loop {
        if stop() {
            return Err("cancelled".into());
        }
        let mut filled = 0;
        while filled < cfg.block_size {
            let n = f.read(&mut buf[filled..]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        let chunk = &buf[..filled];
        fh.update(chunk);
        size += filled as u64;
        let hash = hash_bytes(cfg.block_hash, chunk);
        let (flag, data) = volume::encode_block(cfg.format, chunk, level).map_err(|e| e.user_message())?;
        tx.send(Msg::Block { idx, hash, flag, data, raw_len: filled as u64 }).map_err(|_| "pipeline closed".to_string())?;
        if !cfg.pause.is_zero() {
            std::thread::sleep(cfg.pause);
        }
        if filled < cfg.block_size {
            break;
        }
    }
    tx.send(Msg::Done { idx, size, hash: fh.finish() }).map_err(|_| "pipeline closed".to_string())
}

struct UploadResult {
    volume: SealedVolume,
    par2_names: Vec<String>,
    error: Option<String>,
}

fn upload_one(backend: &dyn Backend, par2: Option<&Par2>, v: SealedVolume) -> UploadResult {
    let r = (|| -> Result<Vec<String>, AppError> {
        backend.put(&v.block_name, &v.block_data)?;
        let names = match par2 {
            Some(p) => upload_recovery(backend, p, &v.block_name, &v.block_data)?,
            None => Vec::new(),
        };
        backend.put(&v.index_name, &v.index_data)?;
        Ok(names)
    })();
    match r {
        Ok(par2_names) => UploadResult { volume: v, par2_names, error: None },
        Err(e) => UploadResult { volume: v, par2_names: Vec::new(), error: Some(e.user_message()) },
    }
}

/// Persisted USN journal state key.
const USN_KEY: &str = "usn_state";

/// Run a backup of `sources` into the repository at `backend`.
#[allow(clippy::too_many_arguments)]
pub fn run_backup(
    backend: &dyn Backend,
    db: &BackupDb,
    sources: &[PathBuf],
    filter: &SourceFilter,
    opts: &BackupOptions,
    retention: &RetentionPolicy,
    passphrase: Option<&str>,
    kdf: KdfParams,
    ctx: &OpContext,
) -> Result<BackupReport, AppError> {
    run_backup_with_cipher(backend, db, sources, filter, opts, retention, passphrase, None, kdf, ctx)
}

/// [`run_backup`] with an optional external encryption module.
#[allow(clippy::too_many_arguments)]
pub fn run_backup_with_cipher(
    backend: &dyn Backend,
    db: &BackupDb,
    sources: &[PathBuf],
    filter: &SourceFilter,
    opts: &BackupOptions,
    retention: &RetentionPolicy,
    passphrase: Option<&str>,
    external: Option<VolumeCipher>,
    kdf: KdfParams,
    ctx: &OpContext,
) -> Result<BackupReport, AppError> {
    let started = Instant::now();
    let mut report = BackupReport { dry_run: opts.dry_run, ..Default::default() };
    retention.validate()?;
    if sources.is_empty() {
        return Err(AppError::validation("The backup job has no source folders"));
    }
    if opts.background_io {
        meta::set_background_priority();
    }
    progress(ctx, Phase::Preparing, 0, 0, 0, None);
    let repo = Repo::open_with_cipher(backend, db, passphrase, external, opts, true, kdf)?;
    if let Some(p) = &repo.par2 {
        if !p.available() {
            return Err(AppError::Configuration { message: format!("PAR2 is enabled but '{}' cannot be run", opts.par2.binary), advice: "Install par2cmdline or turn PAR2 off.".into() });
        }
    }

    let cons = if opts.dry_run && db.volumes(None)?.is_empty() { Default::default() } else { repo.consistency_check()? };
    if !cons.missing.is_empty() {
        return Err(AppError::transfer(format!("{} remote volume(s) are missing from the destination", cons.missing.len()), "Run Repair (or Recreate database) before the next backup."));
    }
    for name in &cons.incomplete {
        // Interrupted uploads from an earlier run: forget them, data is re-uploaded.
        if !opts.dry_run {
            let _ = repo.delete_volume_files(name);
        }
    }
    if !cons.extra.is_empty() {
        report.warnings.push(format!("{} remote file(s) are unknown to the local database; run Repair if they belong to this backup", cons.extra.len()));
    }

    let prev_set = db.filesets()?.into_iter().find(|f| !f.partial);
    let prev_list: Vec<FileEntry> = match &prev_set {
        Some(fs) => db.files(fs.id)?,
        None => Vec::new(),
    };

    // ── Scan (full, or incremental from changed paths / the USN journal) ──
    progress(ctx, Phase::Scanning, 0, 0, 0, None);
    let mut usn_next: Option<super::usn::UsnState> = None;
    let mut shallow: Vec<String> = Vec::new();
    let mut changed: Option<Vec<String>> = opts.changed_paths.clone();
    if opts.usn_policy != UsnPolicy::Off && changed.is_none() && !prev_list.is_empty() {
        let state: super::usn::UsnState = db.get_kv(USN_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let srcs: Vec<String> = sources.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        match super::usn::changes(&srcs, &state) {
            Ok(c) if c.full_scan_volumes.is_empty() => {
                shallow = c.changed_dirs;
                changed = Some(Vec::new());
                usn_next = Some(c.next_state);
                report.usn_used = true;
            }
            Ok(c) => {
                usn_next = Some(c.next_state);
                if opts.usn_policy == UsnPolicy::On {
                    report.warnings.push("Change journal state was missing or reset; performed a full scan".into());
                }
            }
            Err(e) => match opts.usn_policy {
                UsnPolicy::Required => return Err(e),
                UsnPolicy::On => report.warnings.push(format!("Change journal unavailable ({}); performed a full scan", e.user_message())),
                _ => {}
            },
        }
    }
    let (mut entries, warnings) = match (&changed, prev_list.is_empty()) {
        (Some(c), false) => {
            let (r, n) = scan_incremental(sources, &prev_list, c, &shallow, &opts.deleted_paths, filter, opts)?;
            report.rescanned_paths = n;
            r
        }
        _ => scan_sources(sources, filter, opts)?,
    };
    report.warnings.extend(warnings);
    if entries.iter().all(|(_, e)| e.kind == EntryKind::Dir) && !opts.allow_empty_source {
        return Err(AppError::validation("The source selection contains no files; refusing to create an empty version"));
    }

    let prev_files: HashMap<String, FileEntry> = prev_list.into_iter().map(|f| (f.path.clone(), f)).collect();
    let total = entries.len() as u64;
    let mut results: Vec<Option<FileEntry>> = vec![None; entries.len()];
    let mut work: VecDeque<(usize, PathBuf, u32)> = VecDeque::new();
    for (i, (path, entry)) in entries.iter_mut().enumerate() {
        if entry.kind != EntryKind::File {
            results[i] = Some(entry.clone());
            continue;
        }
        report.files_examined += 1;
        report.bytes_examined += entry.size;
        match prev_files.get(&entry.path) {
            Some(prev) if unchanged(prev, entry, opts) && prev.blocks.iter().all(|b| db.has_block(b).unwrap_or(false)) => {
                let mut e = prev.clone();
                e.mode = entry.mode;
                e.meta = entry.meta.clone();
                results[i] = Some(e);
                continue;
            }
            Some(_) => report.files_changed += 1,
            None => report.files_added += 1,
        }
        progress(ctx, Phase::Uploading, i as u64, total, report.bytes_examined, Some(&entry.path));
        work.push_back((i, path.clone(), compress_level(path, opts)));
    }

    // ── Parallel read/hash/compress → dedup/pack → concurrent upload ──
    let abort = Arc::new(AtomicBool::new(false));
    let stop = {
        let abort = abort.clone();
        let ctx = ctx.clone();
        move || abort.load(Ordering::SeqCst) || ctx.is_cancelled()
    };
    let cfg = WorkerCfg {
        block_size: repo.header.block_size,
        read_buf: opts.read_buffer_size,
        block_hash: repo.header.block_hash,
        file_hash: repo.header.file_hash,
        format: repo.header.volume_format,
        respect_locks: opts.respect_advisory_locks,
        background: opts.background_io,
        pause: std::time::Duration::from_micros((10 - opts.cpu_intensity.clamp(1, 10) as u64) * 500),
    };
    let mut pending = Pending::for_repo(&repo, opts);
    let mut in_flight: HashSet<String> = HashSet::new();
    let mut file_blocks: HashMap<usize, Vec<String>> = HashMap::new();
    let mut upload_error: Option<String> = None;
    let mut done_count = 0u64;
    let queue = Mutex::new(work);
    let threads = opts.worker_threads();
    let uploaders = opts.upload_concurrency.max(1);

    std::thread::scope(|scope| -> Result<(), AppError> {
        let (tx, rx) = mpsc::sync_channel::<Msg>(256);
        for _ in 0..threads {
            let tx = tx.clone();
            let (queue, cfg, stop) = (&queue, &cfg, &stop);
            scope.spawn(move || {
                if cfg.background {
                    meta::set_background_priority();
                }
                loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                    let Some((idx, path, level)) = next else { break };
                    if stop() {
                        break;
                    }
                    if let Err(error) = read_file(cfg, idx, &path, level, &tx, stop) {
                        if tx.send(Msg::Failed { idx, error }).is_err() {
                            break;
                        }
                    }
                }
            });
        }
        drop(tx);

        let (job_tx, job_rx) = mpsc::sync_channel::<SealedVolume>(uploaders);
        let job_rx = Arc::new(Mutex::new(job_rx));
        let (res_tx, res_rx) = mpsc::channel::<UploadResult>();
        if !opts.dry_run {
            for _ in 0..uploaders {
                let (job_rx, res_tx, par2) = (job_rx.clone(), res_tx.clone(), repo.par2.as_ref());
                scope.spawn(move || loop {
                    let job = job_rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                    let Ok(v) = job else { break };
                    if res_tx.send(upload_one(backend, par2, v)).is_err() {
                        break;
                    }
                });
            }
        }
        drop(res_tx);

        let handle_result = |r: UploadResult, in_flight: &mut HashSet<String>, upload_error: &mut Option<String>| -> Result<(), AppError> {
            match r.error {
                Some(e) => {
                    *upload_error = Some(format!("Upload of {} failed: {e}", r.volume.block_name));
                    abort.store(true, Ordering::SeqCst);
                }
                None => {
                    record_volume(&repo, &r.volume, &r.par2_names)?;
                    for (h, _) in &r.volume.blocks {
                        in_flight.remove(h);
                    }
                }
            }
            Ok(())
        };

        let send_volume = |pending: &mut Pending, in_flight: &mut HashSet<String>, report: &mut BackupReport| -> Result<(), AppError> {
            let Some((w, blocks)) = pending.take() else { return Ok(()) };
            let v = seal_pending(&repo, w, blocks)?;
            report.bytes_uploaded += (v.block_data.len() + v.index_data.len()) as u64;
            report.volumes_uploaded += 1;
            if opts.dry_run {
                return Ok(());
            }
            repo.db.add_volume(&v.block_name, VolumeKind::Block, v.block_data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &v.block_data), "uploading")?;
            in_flight.extend(v.blocks.iter().map(|(h, _)| h.clone()));
            job_tx.send(v).map_err(|_| AppError::internal("upload pool stopped"))
        };

        for msg in rx {
            while let Ok(r) = res_rx.try_recv() {
                handle_result(r, &mut in_flight, &mut upload_error)?;
            }
            match msg {
                Msg::Block { idx, hash, flag, data, raw_len } => {
                    let known = pending.hashes.contains(&hash) || in_flight.contains(&hash) || db.has_block(&hash)?;
                    if !known {
                        pending.add_encoded(hash.clone(), flag, data, raw_len);
                        report.new_blocks += 1;
                        if pending.byte_size() >= opts.volume_size {
                            send_volume(&mut pending, &mut in_flight, &mut report)?;
                        }
                    }
                    file_blocks.entry(idx).or_default().push(hash);
                }
                Msg::Done { idx, size, hash } => {
                    let mut e = entries[idx].1.clone();
                    e.blocks = file_blocks.remove(&idx).unwrap_or_default();
                    e.size = size;
                    e.hash = Some(hash);
                    done_count += 1;
                    progress(ctx, Phase::Uploading, done_count, total, report.bytes_examined, Some(&e.path));
                    results[idx] = Some(e);
                }
                Msg::Failed { idx, error } => {
                    file_blocks.remove(&idx);
                    if error != "cancelled" {
                        let path = &entries[idx].1.path;
                        report.warnings.push(format!("Skipped {path}: {error}"));
                        if let Some(prev) = prev_files.get(path) {
                            // Keep the previous good copy in this version.
                            results[idx] = Some(prev.clone());
                        }
                    }
                }
            }
        }
        // All files read: flush the last volume unless the run was stopped by an upload error.
        if upload_error.is_none() {
            send_volume(&mut pending, &mut in_flight, &mut report)?;
        }
        drop(job_tx);
        for r in res_rx {
            handle_result(r, &mut in_flight, &mut upload_error)?;
        }
        Ok(())
    })?;

    if let Some(e) = upload_error {
        report.errors.push(e);
        report.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(report);
    }
    let done: Vec<FileEntry> = results.into_iter().flatten().collect();
    let present: HashSet<&str> = done.iter().map(|e| e.path.as_str()).collect();
    report.files_deleted = prev_files.keys().filter(|p| !present.contains(p.as_str())).count() as u64;
    progress(ctx, Phase::Finalizing, total, total, report.bytes_examined, None);

    let control_files = read_control_files(&opts.control_files, &mut report)?;
    if ctx.is_cancelled() {
        // Synthetic partial version: everything finished so far stays restorable.
        if !done.is_empty() {
            let ts = next_timestamp(db)?;
            report.version = Some(repo.write_filelist(ts, true, &done, opts.task_config.clone(), control_files, opts.dry_run)?);
            report.partial = true;
        }
        report.errors.push("The operation was cancelled. Start it again when ready; completed work is kept.".into());
        report.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(report);
    }

    let mut prev_sorted: Vec<FileEntry> = prev_files.into_values().collect();
    prev_sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut cur_sorted = done.clone();
    cur_sorted.sort_by(|a, b| a.path.cmp(&b.path));
    if !prev_sorted.is_empty() && same_version(&prev_sorted, &cur_sorted) && !opts.upload_unchanged_backups && control_files.is_empty() {
        report.skipped_unchanged = true;
    } else {
        let ts = next_timestamp(db)?;
        report.version = Some(repo.write_filelist(ts, false, &done, opts.task_config.clone(), control_files, opts.dry_run)?);
    }
    if let (Some(state), false) = (&usn_next, opts.dry_run) {
        db.set_kv(USN_KEY, &serde_json::to_string(state)?)?;
    }
    // Sources are no longer needed: scripts may release snapshots/locks now.
    ctx.source_released();

    if !opts.dry_run {
        progress(ctx, Phase::Retention, 0, 0, 0, None);
        report.versions_deleted = super::maintenance::apply_retention(&repo, retention, opts.allow_full_removal)? as u64;
        if opts.auto_compact && interval_due(db, "last_compact", opts.auto_compact_interval_days)? {
            progress(ctx, Phase::Compacting, 0, 0, 0, None);
            report.volumes_compacted = super::maintenance::compact(&repo, opts, ctx)?.volumes_removed as u64;
            db.set_kv("last_compact", &Utc::now().to_rfc3339())?;
        }
        if opts.auto_vacuum_interval_days.is_some() && interval_due(db, "last_vacuum", opts.auto_vacuum_interval_days)? {
            db.vacuum()?;
            db.set_kv("last_vacuum", &Utc::now().to_rfc3339())?;
        }
        if opts.upload_verification_file {
            super::maintenance::upload_verification_file(&repo)?;
        }
        if opts.verify_samples > 0 {
            progress(ctx, Phase::Verifying, 0, 0, 0, None);
            let v = super::maintenance::verify(&repo, Some(opts.verify_samples), false)?;
            report.volumes_verified = v.checked as u64;
            report.errors.extend(v.errors);
        }
        if let Some(w) = quota_warning(&repo, opts)? {
            report.warnings.push(w);
        }
    }
    progress(ctx, Phase::Completed, total, total, report.bytes_examined, None);
    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

fn read_control_files(paths: &[String], report: &mut BackupReport) -> Result<Vec<ControlFile>, AppError> {
    use base64::Engine;
    let mut out = Vec::new();
    for p in paths.iter().filter(|p| !p.trim().is_empty()) {
        match std::fs::read(p) {
            Ok(d) => out.push(ControlFile {
                name: Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone()),
                data: base64::engine::general_purpose::STANDARD.encode(d),
            }),
            Err(e) => report.warnings.push(format!("Control file {p}: {e}")),
        }
    }
    Ok(out)
}

/// Whether an interval-based maintenance task is due (`None` = always).
fn interval_due(db: &BackupDb, key: &str, days: Option<u32>) -> Result<bool, AppError> {
    let Some(days) = days else { return Ok(true) };
    let last = db.get_kv(key)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|t| t.with_timezone(&Utc));
    Ok(last.is_none_or(|l| Utc::now() - l >= chrono::Duration::days(days as i64)))
}

/// Warn when the destination is running out of space.
fn quota_warning(repo: &Repo, opts: &BackupOptions) -> Result<Option<String>, AppError> {
    if opts.quota_disable {
        return Ok(None);
    }
    let used: u64 = repo.db.volumes(None)?.iter().map(|v| v.size).sum();
    let (total, free) = match (opts.quota_size, repo.backend.quota()) {
        (Some(q), _) => (q, q.saturating_sub(used)),
        (None, Some(q)) => (q.total, q.free),
        (None, None) => return Ok(None),
    };
    if total == 0 {
        return Ok(None);
    }
    let pct_free = free.saturating_mul(100) / total;
    if free == 0 || pct_free < opts.quota_warning_percent as u64 {
        return Ok(Some(format!(
            "The destination is nearly full: {} free of {} ({}%); the backup uses {}",
            human(free),
            human(total),
            pct_free,
            human(used)
        )));
    }
    Ok(None)
}

fn human(b: u64) -> String {
    let mut v = b as f64;
    for u in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 {
            return format!("{v:.1} {u}");
        }
        v /= 1024.0;
    }
    format!("{v:.1} PB")
}

/// Names of PAR2 files belonging to `name` in a listing (re-export for tools).
pub fn recovery_files(name: &str, listing: &[super::RemoteFile]) -> Vec<String> {
    par2::recovery_names(name, listing.iter().map(|f| f.name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::test_util::write;
    use crate::backup::{Par2Options, VolumeFormat, ZipMethod};

    pub(crate) fn opts() -> BackupOptions {
        BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 2, ..Default::default() }
    }

    #[test]
    fn backup_dedups_and_skips_unchanged() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "a.txt", &vec![b'a'; 5000]);
        write(src.path(), "sub/b.txt", &vec![b'a'; 5000]); // identical content → deduplicated
        write(src.path(), "sub/c.bin", b"small");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = opts();
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.version.is_some());
        assert_eq!(r.files_added, 3);
        // 5000 bytes of 'a' in 1 KiB blocks → two distinct blocks (full + tail) + "small".
        assert_eq!(r.new_blocks, 3);
        assert!(r.errors.is_empty(), "{:?}", r.errors);

        let r2 = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r2.skipped_unchanged);
        assert_eq!(db.filesets().unwrap().len(), 1);

        // Wrong passphrase is rejected before anything is written.
        let e = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            Some("nope"), KdfParams::insecure_fast(), &OpContext::new());
        assert!(e.is_err());
    }

    #[test]
    fn empty_source_and_missing_source_policies() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let mut o = opts();
        let run = |o: &BackupOptions, s: Vec<PathBuf>| run_backup(&be, &db, &s, &SourceFilter::default(), o,
            &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new());
        assert!(run(&o, vec![src.path().into()]).is_err());
        let missing = src.path().join("nope");
        assert!(run(&o, vec![missing.clone()]).is_err());
        write(src.path(), "f", b"1");
        o.missing_source = MissingSourcePolicy::Warn;
        let r = run(&o, vec![src.path().into(), missing]).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("missing")));
    }

    #[test]
    fn dry_run_writes_nothing() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "x", b"hello");
        let be = LocalBackend::new(dst.path().join("repo"));
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { dry_run: true, ..opts() };
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.dry_run && r.bytes_uploaded > 0);
        assert!(!dst.path().join("repo").exists());
        assert!(db.filesets().unwrap().is_empty());
    }

    #[test]
    fn cancelled_backup_writes_partial_version() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "x", b"hello");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let ctx = OpContext::new();
        let c2 = ctx.clone();
        // Cancel as soon as the upload phase starts reporting the file.
        let ctx = ctx.with_progress(move |p| if p.phase == Phase::Uploading && p.current_path.as_deref().map(|s| s.ends_with('x')).unwrap_or(false) { c2.cancel() });
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &opts(), &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &ctx).unwrap();
        assert!(r.partial);
        assert!(!r.errors.is_empty());
        assert!(db.filesets().unwrap()[0].partial);
    }

    fn restore_all(be: &LocalBackend, db: &BackupDb, o: &BackupOptions, pw: Option<&str>, target: &Path) -> crate::backup::restore::RestoreReport {
        let repo = Repo::open(be, db, pw, o, false, KdfParams::insecure_fast()).unwrap();
        let ro = crate::backup::restore::RestoreOptions { target: Some(target.to_path_buf()), ..Default::default() };
        crate::backup::restore::run_restore(&repo, &ro, &OpContext::new()).unwrap()
    }

    #[test]
    fn zip_volume_format_roundtrip() {
        for m in [ZipMethod::Deflate, ZipMethod::Stored, ZipMethod::Bzip2, ZipMethod::Zstd] {
            let src = tempfile::tempdir().unwrap();
            let dst = tempfile::tempdir().unwrap();
            let out = tempfile::tempdir().unwrap();
            write(src.path(), "a.txt", &b"zip me ".repeat(900));
            let be = LocalBackend::new(dst.path());
            let db = BackupDb::open_in_memory().unwrap();
            let o = BackupOptions { volume_format: VolumeFormat::Zip, zip_method: m, ..opts() };
            let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
                None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            // Unencrypted ZIP block volumes open with any ZIP tool.
            let dblock = std::fs::read_dir(dst.path()).unwrap().filter_map(Result::ok)
                .find(|e| e.file_name().to_string_lossy().contains(".dblock")).unwrap();
            let bytes = std::fs::read(dblock.path()).unwrap();
            assert!(zip::ZipArchive::new(std::io::Cursor::new(bytes)).is_ok(), "{m:?}");
            let rr = restore_all(&be, &db, &o, None, out.path());
            assert_eq!(rr.files_restored, 1);
            let restored = walkdir_file(out.path(), "a.txt");
            assert_eq!(std::fs::read(restored).unwrap(), b"zip me ".repeat(900));
        }
    }

    fn walkdir_file(root: &Path, name: &str) -> PathBuf {
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p) } else if p.file_name().unwrap() == name { return p }
            }
        }
        panic!("{name} not restored")
    }

    #[test]
    fn par2_heals_damaged_volume_on_restore() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let content: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
        write(src.path(), "data.bin", &content);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { volume_size: 1 << 20, par2: Par2Options { enabled: true, redundancy_percent: 20, ..Default::default() }, ..opts() };
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let names: Vec<String> = std::fs::read_dir(dst.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n.ends_with(".par2")), "{names:?}");
        let dblock = names.iter().find(|n| n.contains(".dblock") && !n.ends_with(".par2")).unwrap();
        let p = dst.path().join(dblock);
        let mut d = std::fs::read(&p).unwrap();
        for b in d.iter_mut().skip(100).take(40) { *b ^= 0xff; }
        std::fs::write(&p, &d).unwrap();
        // Verify reports the damage instead of silently repairing it.
        let repo = Repo::open(&be, &db, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
        assert!(!crate::backup::maintenance::verify(&repo, None, true).unwrap().errors.is_empty());
        let rr = restore_all(&be, &db, &o, Some("pw"), out.path());
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert_eq!(std::fs::read(walkdir_file(out.path(), "data.bin")).unwrap(), content);
        // The remote copy was healed.
        assert!(crate::backup::maintenance::verify(&repo, None, true).unwrap().errors.is_empty());
    }

    #[test]
    fn changed_paths_rescan_only_those_paths() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "keep/a.txt", b"one");
        write(src.path(), "hot/b.txt", b"two");
        write(src.path(), "gone/c.txt", b"three");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let run = |o: &BackupOptions| run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), o,
            &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        run(&opts());
        tick();
        write(src.path(), "hot/b.txt", b"two-changed");
        write(src.path(), "hot/new.txt", b"new");
        // A change outside the listed paths is NOT picked up in this mode.
        write(src.path(), "keep/a.txt", b"one-changed-unlisted");
        std::fs::remove_dir_all(src.path().join("gone")).unwrap();
        let o = BackupOptions {
            changed_paths: Some(vec![src.path().join("hot").to_string_lossy().into_owned()]),
            deleted_paths: vec![src.path().join("gone").to_string_lossy().into_owned()],
            ..opts()
        };
        let r = run(&o);
        assert_eq!(r.rescanned_paths, 1);
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("new.txt")));
        assert!(!paths.iter().any(|p| p.contains("gone")), "{paths:?}");
        let a = files.iter().find(|f| f.path.ends_with("a.txt")).unwrap();
        assert_eq!(a.size, 3, "unlisted change kept the previous entry");
        let b = files.iter().find(|f| f.path.ends_with("b.txt")).unwrap();
        assert_eq!(b.size, 11);
    }

    fn tick() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    #[test]
    fn control_files_are_stored_and_preserved() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let ctl = tempfile::tempdir().unwrap();
        write(src.path(), "f", b"1");
        write(ctl.path(), "manifest.txt", b"control!");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { control_files: vec![ctl.path().join("manifest.txt").to_string_lossy().into_owned()], ..opts() };
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        let repo = Repo::open(&be, &db, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
        let fs = &db.filesets().unwrap()[0];
        let fl: FileListVolume = volume::open_json(&repo.cipher, &be.get(&fs.volume_name).unwrap()).unwrap();
        assert_eq!(fl.control_files.len(), 1);
        assert_eq!(fl.control_files[0].name, "manifest.txt");
        // A rewrite (purge/repair) keeps them.
        repo.rewrite_filelist(&fs.timestamp, fs.partial, &fs.volume_name, &db.files(fs.id).unwrap()).unwrap();
        let fl: FileListVolume = volume::open_json(&repo.cipher, &be.get(&fs.volume_name).unwrap()).unwrap();
        assert_eq!(fl.control_files.len(), 1);
    }

    #[test]
    fn quota_warning_when_destination_nearly_full() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "f", &vec![7u8; 3000]);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { quota_size: Some(1000), quota_warning_percent: 10, ..opts() };
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("nearly full")), "{:?}", r.warnings);
        let o = BackupOptions { quota_disable: true, ..o };
        write(src.path(), "g", b"x");
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(!r.warnings.iter().any(|w| w.contains("nearly full")));
        assert!(be.quota().is_some());
    }

    #[test]
    fn parallel_hashing_matches_single_thread() {
        let src = tempfile::tempdir().unwrap();
        for i in 0..40 {
            write(src.path(), &format!("d{}/f{i}.bin", i % 5), &vec![(i % 9) as u8; 1500 + i * 37]);
        }
        let mut blocks = Vec::new();
        for threads in [1usize, 4] {
            let dst = tempfile::tempdir().unwrap();
            let be = LocalBackend::new(dst.path());
            let db = BackupDb::open_in_memory().unwrap();
            let o = BackupOptions { hash_threads: threads, upload_concurrency: threads, ..opts() };
            let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
                None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            assert_eq!(r.files_added, 40);
            let mut f = db.files(db.filesets().unwrap()[0].id).unwrap();
            f.sort_by(|a, b| a.path.cmp(&b.path));
            blocks.push(f.into_iter().map(|e| (e.path.replace(&*src.path().to_string_lossy(), ""), e.blocks)).collect::<Vec<_>>());
            let repo = Repo::open(&be, &db, None, &o, false, KdfParams::insecure_fast()).unwrap();
            assert!(repo.consistency_check().unwrap().is_clean());
        }
        assert_eq!(blocks[0], blocks[1]);
    }

    #[test]
    fn source_released_hook_fires() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "f", b"1");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let hit = Arc::new(AtomicBool::new(false));
        let h = hit.clone();
        let ctx = OpContext::new().with_source_released(move || h.store(true, Ordering::SeqCst));
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &opts(), &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &ctx).unwrap();
        assert!(hit.load(Ordering::SeqCst));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn xattrs_backed_up_and_exclusion_attribute_honoured() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(src.path(), "tagged.txt", b"t");
        write(src.path(), "skip.txt", b"s");
        if xattr::set(src.path().join("tagged.txt"), "user.color", b"blue").is_err() {
            return; // filesystem without user xattrs
        }
        xattr::set(src.path().join("skip.txt"), "user.xdg.robots.backup", b"false").unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = opts();
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        assert!(!files.iter().any(|f| f.path.ends_with("skip.txt")));
        let t = files.iter().find(|f| f.path.ends_with("tagged.txt")).unwrap();
        assert!(t.meta.as_ref().unwrap().xattrs.contains_key("user.color"));
        restore_all(&be, &db, &o, None, out.path());
        let p = walkdir_file(out.path(), "tagged.txt");
        assert_eq!(xattr::get(p, "user.color").unwrap().as_deref(), Some(&b"blue"[..]));
    }
}
