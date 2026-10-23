//! Backup run: scan → deduplicate → pack/compress/encrypt → upload → write
//! the version's file list → retention → compaction → sampled verification.
//!
//! Files are read, chunked, hashed and compressed by a pool of worker
//! threads; the coordinating thread deduplicates blocks and packs volumes;
//! sealed volumes are uploaded by a separate pool (with PAR2 recovery data
//! when enabled). The local database is only touched by the coordinating
//! thread.

use super::backend::Backend;
use super::chunker::Chunker;
use super::reader::{read_input, Input, Msg, StreamSource, Work, WorkerCfg};
use super::crypto::{KdfParams, VolumeCipher};
use super::layout::LayoutBackend;
use super::db::BackupDb;
use super::filters::SourceFilter;
use super::par2::{self, Par2};
use super::retention::RetentionPolicy;
use super::volume::{self, BlockVolumeWriter, ControlFile, FileListVolume, IndexVolume, IndexedBlock, RepoHeader, VolumeKind};
use super::{
    hash_bytes, meta, BackupOptions, EntryKind, FileEntry, HashAlgorithm, MissingSourcePolicy, OpContext, Phase, Progress, SymlinkPolicy,
    UsnPolicy,
};
use super::errors::{ErrorClass, ErrorTracker};
use super::snapshot::SnapshotPolicy;
use super::versions::{VersionMeta, VersionStats};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

/// An opened repository: backend + local database + key material.
pub struct Repo<'a> {
    /// The destination seen through the repository layout (logical names).
    pub backend: LayoutBackend<'a>,
    pub db: &'a BackupDb,
    pub cipher: VolumeCipher,
    pub header: RepoHeader,
    pub prefix: String,
    pub par2: Option<Par2>,
    pub case_insensitive: bool,
    /// An upgraded repository still offers rollback to format 1 until the
    /// first format-2 write (see [`Repo::note_write`]).
    rollback_pending: AtomicBool,
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
    ///
    /// A shared repository (see [`super::shared`]) scopes `db` to the
    /// source of `options.identity` and brings it up to date from the
    /// destination first.
    pub fn open_with_cipher(
        backend: &'a dyn Backend,
        db: &'a BackupDb,
        passphrase: Option<&str>,
        external: Option<VolumeCipher>,
        options: &BackupOptions,
        create: bool,
        kdf: KdfParams,
    ) -> Result<Self, AppError> {
        let repo = Self::open_unsynced(backend, db, passphrase, external, options, create, kdf)?;
        if super::shared::is_shared(&repo.header) {
            db.set_scope(Some(options.identity.resolve().source_id()));
            let r = super::shared::sync_from_remote(&repo, &OpContext::new())?;
            for w in r.warnings {
                tracing::warn!("{w}");
            }
        }
        Ok(repo)
    }

    /// [`Repo::open_with_cipher`] without the shared-repository scope and
    /// sync (database rebuilds, header-level tools).
    pub(crate) fn open_unsynced(
        backend: &'a dyn Backend,
        db: &'a BackupDb,
        passphrase: Option<&str>,
        external: Option<VolumeCipher>,
        options: &BackupOptions,
        create: bool,
        kdf: KdfParams,
    ) -> Result<Self, AppError> {
        let header_name = volume::repo_header_name(&options.prefix);
        let gpg_from_options = || {
            std::sync::Arc::new(super::gpg::GpgModule {
                executable: options.gpg.executable.clone(),
                homedir: options.gpg.homedir.clone(),
                passphrase: passphrase.filter(|p| !p.is_empty()).map(String::from),
                recipients: options.gpg.recipients.clone(),
            })
        };
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
            super::format::check_readable(&header)?;
            let external = match external {
                None if header.encrypted && header.encryption_module == "gpg" => Some(VolumeCipher::external(std::sync::Arc::new(super::gpg::GpgModule { recipients: header.gpg_recipients.clone(), ..(*gpg_from_options()).clone() }))),
                other => other,
            };
            let cipher = header_cipher(&header, passphrase, external)?;
            (header, cipher)
        } else {
            if !create {
                return Err(AppError::file_op(format!("No backup found at {}", backend.describe()), "Check the destination and file-name prefix."));
            }
            let kdf = options.kdf.unwrap_or(kdf);
            kdf.validate().or_else(|e| if kdf == KdfParams::insecure_fast() { Ok(()) } else { Err(e) })?;
            let external = match external {
                None if options.encryption_module == super::EncryptionModuleKind::Gpg => {
                    if options.gpg.recipients.is_empty() && passphrase.is_none_or(|p| p.is_empty()) {
                        return Err(AppError::validation("GPG encryption needs a passphrase or at least one recipient key"));
                    }
                    Some(VolumeCipher::external(gpg_from_options()))
                }
                other => other,
            };
            let (header, cipher) = super::format::create_header(options, passphrase, external, kdf, backend)?;
            if !options.dry_run {
                backend.create_folder()?;
                backend.put(&header_name, &serde_json::to_vec_pretty(&header)?)?;
            }
            (header, cipher)
        };
        let par2 = options.par2.enabled.then(|| Par2::new(options.par2.clone(), options.temp_root()));
        let rollback_pending = AtomicBool::new(header.has_feature(super::format::FEATURE_V1_ROLLBACK));
        Ok(Self {
            backend: LayoutBackend::new(backend, header.layout, &options.prefix),
            db,
            cipher,
            header,
            prefix: options.prefix.clone(),
            par2,
            case_insensitive: options.remote_case_insensitive,
            rollback_pending,
        })
    }

    /// Record that format-2 data is about to be written: an upgraded
    /// repository loses its rollback option (once, before the first write).
    pub(crate) fn note_write(&self) -> Result<(), AppError> {
        if self.rollback_pending.swap(false, Ordering::SeqCst) {
            super::format::clear_rollback(self.backend.inner(), &self.prefix)?;
        }
        Ok(())
    }

    /// The chunker new blocks are cut with.
    pub fn chunker(&self) -> Box<dyn Chunker> {
        self.header.chunker_spec().build()
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
        self.note_write()?;
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
        if let Some(until) = self.locked_until(name)? {
            return Err(AppError::state(format!("{name} is locked until {until}"), "Locked volumes can be deleted once the lock expires."));
        }
        self.backend.delete(name)?;
        if let Some(list) = self.db.get_kv(&Self::par2_key(name))? {
            for n in serde_json::from_str::<Vec<String>>(&list).unwrap_or_default() {
                let _ = self.backend.delete(&n);
            }
        }
        self.db.delete_volume(name)
    }

    /// The lock expiry of a volume, when it is still locked.
    pub fn locked_until(&self, name: &str) -> Result<Option<String>, AppError> {
        Ok(self.db.volume_by_name(name)?.and_then(|v| v.lock_until).filter(|u| DateTime::parse_from_rfc3339(u).is_ok_and(|t| t > Utc::now())))
    }

    /// PAR2 file names recorded for a volume.
    pub(crate) fn recovery_names(&self, name: &str) -> Result<Vec<String>, AppError> {
        Ok(self.db.get_kv(&Self::par2_key(name))?.and_then(|l| serde_json::from_str(&l).ok()).unwrap_or_default())
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
        VolumeFetcher { backend: &self.backend, par2: self.par2.as_ref(), case_insensitive: self.case_insensitive }
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
        meta: VersionMeta,
        dry_run: bool,
    ) -> Result<String, AppError> {
        let name = volume::filelist_volume_name(&self.prefix, ts, &self.cipher);
        let fl = FileListVolume { timestamp: ts.to_rfc3339(), partial, files: files.to_vec(), task_config, control_files, meta };
        let data = volume::seal_json(&self.cipher, &fl)?;
        self.put_volume(VolumeKind::FileList, &name, &data, dry_run)?;
        if !dry_run {
            self.db.add_fileset_with_meta(&fl.timestamp, &name, partial, files, &fl.meta)?;
            // Kept so rewrites (purge, repair) preserve them.
            self.db.set_kv(&format!("dlist_extra:{}", fl.timestamp), &serde_json::to_string(&DlistExtra { task_config: fl.task_config, control_files: fl.control_files })?)?;
        }
        Ok(fl.timestamp)
    }

    /// Re-upload a version's file list (same name), preserving extras and
    /// the version's metadata.
    pub(crate) fn rewrite_filelist(&self, fs: &super::Fileset, files: &[FileEntry]) -> Result<Vec<u8>, AppError> {
        let (timestamp, partial, volume_name) = (fs.timestamp.as_str(), fs.partial, fs.volume_name.as_str());
        let extra: DlistExtra = self.db.get_kv(&format!("dlist_extra:{timestamp}"))?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let fl = FileListVolume { timestamp: timestamp.to_string(), partial, files: files.to_vec(), task_config: extra.task_config, control_files: extra.control_files, meta: fs.meta() };
        let data = volume::seal_json(&self.cipher, &fl)?;
        self.note_write()?;
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

/// The volume cipher for an existing repository header.
pub fn header_cipher(header: &RepoHeader, passphrase: Option<&str>, external: Option<VolumeCipher>) -> Result<VolumeCipher, AppError> {
    super::format::check_readable(header)?;
    let external = match external {
        None if header.encrypted && header.encryption_module == "gpg" => Some(VolumeCipher::external(std::sync::Arc::new(super::gpg::GpgModule {
            passphrase: passphrase.filter(|p| !p.is_empty()).map(String::from),
            recipients: header.gpg_recipients.clone(),
            ..Default::default()
        }))),
        other => other,
    };
    let cipher = match (header.encrypted, external) {
        (true, Some(ext)) => {
            // Key-encrypted GPG backups are written by machines that may not
            // hold the secret key, so only symmetric checks are verified.
            let key_only = header.encryption_module == "gpg" && !header.gpg_recipients.is_empty();
            if let Some(kc) = header.key_check.as_deref().filter(|_| !key_only) {
                ext.verify_key_check(kc)?;
            }
            ext
        }
        (true, None) => {
            let pass = passphrase.filter(|p| !p.is_empty()).ok_or_else(|| AppError::validation("This backup is encrypted; enter its passphrase"))?;
            let salt = hex::decode(header.salt.as_deref().unwrap_or_default()).map_err(|_| AppError::validation("Repository header salt is malformed"))?;
            // v1: the derived key encrypts volumes. v2: it is the KEK that
            // unwraps the master key.
            let kek = VolumeCipher::derive(pass, &salt, header.kdf.unwrap_or_default())?.with_aead(header.cipher);
            kek.verify_key_check(header.key_check.as_deref().unwrap_or_default())?;
            match header.wrapped_master_key.as_deref() {
                Some(w) => kek.unwrap_key(w, header.cipher)?,
                None => kek,
            }
        }
        (false, _) => VolumeCipher::plain(),
    };
    Ok(super::format::configure_cipher(header, cipher))
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
pub(crate) struct DlistExtra {
    pub(crate) task_config: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) control_files: Vec<ControlFile>,
}

/// Remember a file list's embedded configuration and control files so
/// later rewrites (purge, repair) keep them.
pub(crate) fn store_dlist_extra(db: &BackupDb, fl: &FileListVolume) -> Result<(), AppError> {
    if fl.task_config.is_none() && fl.control_files.is_empty() {
        return Ok(());
    }
    db.set_kv(&format!("dlist_extra:{}", fl.timestamp), &serde_json::to_string(&DlistExtra { task_config: fl.task_config.clone(), control_files: fl.control_files.clone() })?)
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
    /// Snapshot method used (`vss`, `lvm`, `apfs`), if any.
    pub snapshot: Option<String>,
    /// Checkpoints (partial versions of the files finished so far) uploaded
    /// during the run; each replaced the previous one.
    #[serde(default)]
    pub checkpoints: u64,
    /// Files re-read by percentage rehash although they looked unchanged.
    #[serde(default)]
    pub files_rehashed: u64,
    /// Of those, files whose content differed from the recorded hash.
    #[serde(default)]
    pub rehash_mismatches: u64,
    /// The run stopped at `max_upload_bytes_per_run` (the version is partial).
    #[serde(default)]
    pub upload_cap_reached: bool,
    /// Informational notes for the run log (e.g. skipped mount points).
    #[serde(default)]
    pub info: Vec<String>,
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
        Self { writer: BlockVolumeWriter::with_format(repo.header.volume_format, opts.zip_method).with_codec(repo.header.block_codec), sizes: Vec::new(), hashes: HashSet::new() }
    }

    pub(crate) fn add_block(&mut self, hash: String, data: &[u8], level: u32) -> Result<(), AppError> {
        let (flag, enc) = volume::encode_block(self.writer.format(), self.writer.codec(), data, level)?;
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
        let (fmt, codec) = (self.writer.format(), self.writer.codec());
        let w = std::mem::replace(&mut self.writer, BlockVolumeWriter::with_format(fmt, Default::default()).with_codec(codec));
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
    repo.note_write()?;
    repo.db.add_volume(&v.block_name, VolumeKind::Block, v.block_data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &v.block_data), "uploading")?;
    repo.backend.put(&v.block_name, &v.block_data)?;
    let par2_names = match &repo.par2 {
        Some(p) => upload_recovery(&repo.backend, p, &v.block_name, &v.block_data)?,
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
    /// Read through this snapshot; entries keep the original paths.
    snap: Option<&'f super::snapshot::Snapshot>,
    #[cfg_attr(not(unix), allow(dead_code))]
    seen_inodes: HashSet<(u64, u64)>,
    out: Vec<(PathBuf, FileEntry)>,
    warnings: Vec<String>,
    /// Folder for generated files (Photos `albums.json`) that the backup
    /// reads; `None` for previews, where nothing is read.
    staging: Option<&'f Path>,
    staged: usize,
    /// `.ufopignore` / `.gitignore` rules, read lazily per folder.
    ignores: super::ignore::IgnoreMatcher<'f>,
    /// Unreadable entries go through the job's error policy.
    errors: &'f ErrorTracker,
    /// Informational notes for the run log (skipped mount points).
    info: Vec<String>,
    roots: Vec<PathBuf>,
    /// Device of each source root (stay on one filesystem).
    root_devs: HashMap<PathBuf, Option<u64>>,
}

impl<'f> Scanner<'f> {
    fn new(filter: &'f SourceFilter, opts: &'f BackupOptions, snap: Option<&'f super::snapshot::Snapshot>, staging: Option<&'f Path>, errors: &'f ErrorTracker) -> Self {
        let ignores = super::ignore::IgnoreMatcher::new(filter.ignore_config(), move |p: &Path| snap.map(|s| s.real(p)).unwrap_or_else(|| p.to_path_buf()));
        Scanner {
            filter,
            opts,
            snap,
            seen_inodes: HashSet::new(),
            out: Vec::new(),
            warnings: Vec::new(),
            staging,
            staged: 0,
            ignores,
            errors,
            info: Vec::new(),
            roots: Vec::new(),
            root_devs: HashMap::new(),
        }
    }

    /// A source root: ignore files are read inside it, and with
    /// `one_file_system` its device bounds the walk.
    fn add_root(&mut self, root: &Path) {
        self.ignores.add_root(root);
        if !self.roots.iter().any(|r| r == root) {
            self.roots.push(root.to_path_buf());
            self.roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
        }
    }

    /// An unreadable or unsupported entry, filed by the error policy.
    fn problem(&mut self, class: ErrorClass, message: String) {
        self.errors.record(class, message);
    }

    /// A folder on another filesystem than its source root.
    fn crosses_device(&mut self, path: &Path, md: &std::fs::Metadata) -> bool {
        if !self.opts.one_file_system {
            return false;
        }
        let Some(root) = self.roots.iter().find(|r| path.starts_with(r)).cloned() else { return false };
        if root == path {
            return false;
        }
        let root_dev = match self.root_devs.get(&root) {
            Some(d) => *d,
            None => {
                let real = self.real(&root);
                let d = std::fs::metadata(&real).ok().and_then(|m| meta::device_id(&real, &m));
                self.root_devs.insert(root.clone(), d);
                d
            }
        };
        let dev = meta::device_id(&self.real(path), md);
        matches!((root_dev, dev), (Some(a), Some(b)) if a != b)
    }

    /// Excluded by an ignore file (explicit filter rules win over them).
    fn ignored(&mut self, path: &Path, is_dir: bool) -> bool {
        self.ignores.is_active() && self.filter.evaluate(&path.to_string_lossy(), is_dir).is_none() && self.ignores.is_ignored(path, is_dir)
    }

    fn real(&self, path: &Path) -> PathBuf {
        self.snap.map(|s| s.real(path)).unwrap_or_else(|| path.to_path_buf())
    }

    fn entry(&self, path: &Path, kind: EntryKind, md: &std::fs::Metadata) -> FileEntry {
        let o = self.opts;
        let meta = (!o.skip_metadata && (o.backup_xattrs || o.backup_acls))
            .then(|| meta::capture(&self.real(path), meta::MetaOptions { xattrs: o.backup_xattrs, acls: o.backup_acls }))
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
        let real = self.real(path);
        let lmd = match std::fs::symlink_metadata(&real) {
            Ok(m) => m,
            Err(e) => {
                self.problem(ErrorClass::File, format!("Cannot read {}: {e}", path.display()));
                return None;
            }
        };
        if !self.opts.ignore_exclusion_xattrs && meta::has_exclusion_attribute(&real) {
            return None;
        }
        let md = if lmd.file_type().is_symlink() {
            match self.opts.symlink_policy {
                SymlinkPolicy::Ignore => return None,
                SymlinkPolicy::Store => {
                    if self.filter.evaluate(&path.to_string_lossy(), false) == Some(super::FilterAction::Exclude) || self.ignored(path, false) {
                        return None;
                    }
                    let mut e = self.entry(path, EntryKind::Symlink, &lmd);
                    e.symlink_target = std::fs::read_link(&real).ok().map(|t| t.to_string_lossy().into_owned());
                    if meta::is_junction(&real) {
                        e.meta.get_or_insert_with(Default::default).junction = true;
                    }
                    self.out.push((path.to_path_buf(), e));
                    return None;
                }
                SymlinkPolicy::Follow => match std::fs::metadata(&real) {
                    Ok(m) => m,
                    Err(e) => {
                        self.problem(ErrorClass::File, format!("Broken link {}: {e}", path.display()));
                        return None;
                    }
                },
            }
        } else {
            lmd
        };
        if self.ignored(path, md.is_dir()) || !self.filter.accepts(path, &md) {
            return None;
        }
        if md.is_dir() {
            self.out.push((path.to_path_buf(), self.entry(path, EntryKind::Dir, &md)));
            if self.crosses_device(path, &md) {
                // Kept as an empty folder, like the mount point it is.
                self.info.push(format!("Skipped {}: another filesystem (stay on one filesystem)", path.display()));
                return None;
            }
            return Some(md);
        }
        if !md.is_file() {
            self.problem(ErrorClass::UnknownEntry, format!("Skipped {}: not a regular file, folder or link", path.display()));
            return None;
        }
        if !self.hardlink_skip(&md) {
            self.out.push((path.to_path_buf(), self.entry(path, EntryKind::File, &md)));
            if self.opts.backup_alternate_streams {
                for (name, size) in meta::alternate_streams(&real) {
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
        match std::fs::read_dir(self.real(path)) {
            Ok(rd) => {
                // Children keep original (logical) paths.
                let mut c: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| path.join(e.file_name()))).collect();
                c.sort();
                c
            }
            Err(e) => {
                self.problem(ErrorClass::Dir, format!("Cannot list {}: {e}", path.display()));
                Vec::new()
            }
        }
    }

    /// Expand a Photos library into its originals (under stable logical
    /// paths), the folders above them and the `albums.json` metadata file.
    fn photos(&mut self, library: &Path) -> Result<(), AppError> {
        if check_sources(std::slice::from_ref(&library.to_path_buf()), self.opts, &mut self.warnings)?.is_empty() {
            return Ok(());
        }
        let export = super::photos::export(library, &self.opts.photos, &self.opts.temp_root())?;
        self.warnings.extend(export.warnings.iter().cloned());
        let root = export.root.clone();
        let library_md = std::fs::metadata(library)?;
        let skip_meta = self.opts.skip_metadata;
        let file_entry = |path: &Path, kind: EntryKind, size: u64, md: Option<&std::fs::Metadata>| FileEntry {
            path: path.to_string_lossy().into_owned(),
            kind,
            size,
            mtime: md.filter(|_| !skip_meta).and_then(mtime_of),
            mode: md.filter(|_| !skip_meta).and_then(mode_of),
            hash: None,
            blocks: Vec::new(),
            symlink_target: None,
            meta: None,
        };
        // Folder filters apply to the logical folders, as in a normal walk.
        let mut dir_ok: HashMap<PathBuf, bool> = HashMap::new();
        let mut files: Vec<(PathBuf, FileEntry)> = Vec::new();
        for item in &export.items {
            let blocked = item
                .logical
                .ancestors()
                .skip(1)
                .take_while(|a| a.starts_with(&root))
                .any(|d| !*dir_ok.entry(d.to_path_buf()).or_insert_with(|| self.filter.accepts(d, &library_md)));
            if blocked {
                continue;
            }
            let md = match std::fs::metadata(&item.original) {
                Ok(m) => m,
                Err(e) => {
                    self.warnings.push(format!("Cannot read {}: {e}", item.original.display()));
                    continue;
                }
            };
            if !self.filter.accepts(&item.logical, &md) {
                continue;
            }
            files.push((item.original.clone(), file_entry(&item.logical, EntryKind::File, md.len(), Some(&md))));
        }
        // Metadata file: regenerated every run, so no timestamp (identical
        // content then means an identical entry and no spurious version).
        let manifest = export.manifest_path();
        let local = match self.staging {
            Some(dir) => {
                self.staged += 1;
                let d = dir.join(self.staged.to_string());
                std::fs::create_dir_all(&d)?;
                let p = d.join(super::photos::MANIFEST_NAME);
                std::fs::write(&p, &export.manifest)?;
                p
            }
            None => manifest.clone(),
        };
        files.push((local, file_entry(&manifest, EntryKind::File, export.manifest.len() as u64, None)));
        let dirs: BTreeSet<PathBuf> = files
            .iter()
            .flat_map(|(_, e)| Path::new(&e.path).ancestors().skip(1).take_while(|a| a.starts_with(&root)).map(Path::to_path_buf).collect::<Vec<_>>())
            .collect();
        for d in dirs {
            let e = file_entry(&d, EntryKind::Dir, 0, None);
            self.out.push((d, e));
        }
        self.out.extend(files);
        Ok(())
    }

    fn walk(&mut self, path: &Path, depth: usize) {
        if self.errors.should_stop() {
            return;
        }
        if depth > 256 {
            self.problem(ErrorClass::Dir, format!("Skipping {}: directory nesting too deep", path.display()));
            return;
        }
        if self.visit(path).is_some() {
            for c in self.children(path) {
                self.walk(&c, depth + 1);
            }
        }
    }

    fn finish(self, entries: Vec<(PathBuf, FileEntry)>) -> ScanOut {
        ScanOut { entries, warnings: self.warnings, info: self.info }
    }
}

/// Scanned entries (local path + entry) and warnings.
pub type ScanResult = (Vec<(PathBuf, FileEntry)>, Vec<String>);

/// What a scan found. Entry problems are in the [`ErrorTracker`].
struct ScanOut {
    entries: Vec<(PathBuf, FileEntry)>,
    warnings: Vec<String>,
    info: Vec<String>,
}

impl ScanOut {
    /// The public shape: problems recorded by the tracker become warnings.
    fn into_result(mut self, errors: &ErrorTracker) -> ScanResult {
        let (w, e) = errors.take();
        self.warnings.extend(w);
        self.warnings.extend(e);
        (self.entries, self.warnings)
    }
}

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

/// Split job sources into ordinary paths and `photos://` library paths.
fn split_sources(sources: &[PathBuf]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut plain = Vec::new();
    let mut libraries = Vec::new();
    for s in sources {
        match super::photos::parse_source(&s.to_string_lossy()) {
            Some(lib) => libraries.push(lib),
            None => plain.push(s.clone()),
        }
    }
    (plain, libraries)
}

/// Temporary folder for generated source files, removed on drop.
struct Staging(Option<PathBuf>);

impl Staging {
    fn new(needed: bool, opts: &BackupOptions) -> Result<Self, AppError> {
        if !needed {
            return Ok(Self(None));
        }
        let d = opts.temp_root().join(format!("ufop-staging-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d)?;
        Ok(Self(Some(d)))
    }
    fn path(&self) -> Option<&Path> {
        self.0.as_deref()
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if let Some(d) = &self.0 {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

fn drop_empty_dirs(out: &mut Vec<(PathBuf, FileEntry)>) {
    let file_paths: Vec<PathBuf> = out.iter().filter(|(_, e)| e.kind != EntryKind::Dir).map(|(p, _)| p.clone()).collect();
    out.retain(|(p, e)| e.kind != EntryKind::Dir || file_paths.iter().any(|f| f.starts_with(p) && f != p));
}

/// Enumerate the selected source entries (also used for dry-run previews).
///
/// Photos library sources are expanded, but their `albums.json` is not
/// written anywhere: its entry's local path is the logical one.
pub fn scan_sources(sources: &[PathBuf], filter: &SourceFilter, opts: &BackupOptions) -> Result<ScanResult, AppError> {
    let errors = ErrorTracker::new(opts.error_policy);
    Ok(scan_sources_in(sources, filter, opts, None, None, &errors)?.into_result(&errors))
}

fn scan_sources_in(
    sources: &[PathBuf],
    filter: &SourceFilter,
    opts: &BackupOptions,
    snap: Option<&super::snapshot::Snapshot>,
    staging: Option<&Path>,
    errors: &ErrorTracker,
) -> Result<ScanOut, AppError> {
    let (plain, libraries) = split_sources(sources);
    let mut s = Scanner::new(filter, opts, snap, staging, errors);
    for src in check_sources(&plain, opts, &mut s.warnings)? {
        s.add_root(src);
        s.walk(src, 0);
    }
    for lib in &libraries {
        s.photos(lib)?;
    }
    let mut out = std::mem::take(&mut s.out);
    if filter.exclude_empty_folders {
        drop_empty_dirs(&mut out);
    }
    Ok(s.finish(out))
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
    let errors = ErrorTracker::new(opts.error_policy);
    let (out, n) = scan_incremental_in(sources, prev, changed, shallow, deleted, filter, opts, None, None, &errors)?;
    Ok((out.into_result(&errors), n))
}

#[allow(clippy::too_many_arguments)]
fn scan_incremental_in(
    sources: &[PathBuf],
    prev: &[FileEntry],
    changed: &[String],
    shallow: &[String],
    deleted: &[String],
    filter: &SourceFilter,
    opts: &BackupOptions,
    snap: Option<&super::snapshot::Snapshot>,
    staging: Option<&Path>,
    errors: &ErrorTracker,
) -> Result<(ScanOut, u64), AppError> {
    let (plain, libraries) = split_sources(sources);
    let mut s = Scanner::new(filter, opts, snap, staging, errors);
    let roots = check_sources(&plain, opts, &mut s.warnings)?;
    for r in &roots {
        s.add_root(r);
    }
    let in_sources = |p: &Path| roots.iter().any(|r| p.starts_with(r));
    let changed: Vec<PathBuf> = changed.iter().map(PathBuf::from).filter(|p| in_sources(p)).collect();
    let shallow: Vec<PathBuf> = shallow.iter().map(PathBuf::from).filter(|p| in_sources(p)).collect();
    let deleted: Vec<PathBuf> = deleted.iter().map(PathBuf::from).collect();
    // Photos libraries have no change list: they are always re-expanded
    // (cheap: unchanged originals are recognised by size and timestamp).
    let photo_roots: Vec<PathBuf> = libraries.iter().map(|l| super::photos::export_root(l)).collect();
    for lib in &libraries {
        s.photos(lib)?;
    }

    // Keep previous entries outside the rescanned areas.
    let mut kept: Vec<(PathBuf, FileEntry)> = prev
        .iter()
        .filter(|e| {
            let p = Path::new(&e.path);
            !changed.iter().chain(deleted.iter()).any(|c| p.starts_with(c))
                && !shallow.iter().any(|d| p.parent() == Some(d.as_path()))
                && !photo_roots.iter().any(|r| p.starts_with(r))
        })
        .map(|e| (PathBuf::from(&e.path), e.clone()))
        .collect();
    let prev_dirs: HashSet<&str> = prev.iter().filter(|e| e.kind == EntryKind::Dir).map(|e| e.path.as_str()).collect();
    let mut rescanned = 0u64;
    for c in &changed {
        if s.real(c).exists() {
            s.walk(c, 0);
            rescanned += 1;
        }
    }
    for d in &shallow {
        if !s.real(d).is_dir() {
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
    Ok((s.finish(kept), rescanned))
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

/// Next version timestamp, strictly after the newest existing one of any
/// source (version file names are made from it).
pub(crate) fn next_timestamp(db: &BackupDb) -> Result<DateTime<Utc>, AppError> {
    use chrono::Timelike;
    let now = Utc::now();
    let now = now.with_nanosecond(0).unwrap_or(now);
    let latest = db.all_filesets()?.first().and_then(|f| DateTime::parse_from_rfc3339(&f.timestamp).ok()).map(|t| t.with_timezone(&Utc));
    Ok(match latest {
        Some(l) if l >= now => l + chrono::Duration::seconds(1),
        _ => now,
    })
}

// ── Worker pipeline ──

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

/// Periodic checkpoints of a long backup: a sealed partial version of the
/// files whose blocks are all uploaded, replacing the run's previous
/// checkpoint. A crash then leaves a restorable partial version, and the
/// next run finds the uploaded blocks again (deduplication).
struct Checkpoints {
    every: Option<std::time::Duration>,
    every_bytes: u64,
    last: Instant,
    last_bytes: u64,
    /// Timestamp of this run's current checkpoint version.
    current: Option<String>,
}

impl Checkpoints {
    fn new(opts: &BackupOptions) -> Self {
        Self {
            every: (opts.checkpoint_interval_secs > 0).then(|| std::time::Duration::from_secs(opts.checkpoint_interval_secs)),
            every_bytes: opts.checkpoint_interval_bytes,
            last: Instant::now(),
            last_bytes: 0,
            current: None,
        }
    }

    fn enabled(&self, opts: &BackupOptions) -> bool {
        !opts.dry_run && (self.every.is_some() || self.every_bytes > 0)
    }

    fn due(&self, bytes_uploaded: u64) -> bool {
        self.every.is_some_and(|d| self.last.elapsed() >= d) || (self.every_bytes > 0 && bytes_uploaded.saturating_sub(self.last_bytes) >= self.every_bytes)
    }

    /// Upload a new checkpoint, then drop the previous one.
    fn write(&mut self, repo: &Repo, files: Vec<FileEntry>, opts: &BackupOptions, report: &mut BackupReport) -> Result<(), AppError> {
        self.last = Instant::now();
        self.last_bytes = report.bytes_uploaded;
        if files.iter().all(|e| e.kind == EntryKind::Dir) {
            return Ok(());
        }
        let ts = next_timestamp(repo.db)?;
        let written = repo.write_filelist(ts, true, &files, opts.task_config.clone(), Vec::new(), version_meta(opts, report), false)?;
        tracing::info!("Checkpoint {written}: {} entries", files.len());
        self.remove(repo)?;
        self.current = Some(written);
        report.checkpoints += 1;
        Ok(())
    }

    /// Remove this run's checkpoint (superseded by a newer version).
    fn remove(&mut self, repo: &Repo) -> Result<(), AppError> {
        let Some(ts) = self.current.take() else { return Ok(()) };
        if let Some(fs) = repo.db.filesets()?.into_iter().find(|f| f.timestamp == ts) {
            repo.delete_volume_files(&fs.volume_name)?;
            repo.db.delete_fileset(fs.id)?;
        }
        Ok(())
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
    run_backup_with_streams(backend, db, sources, filter, opts, retention, passphrase, external, kdf, ctx, Vec::new())
}

/// [`run_backup_with_cipher`] that also stores each [`StreamSource`] (a
/// command's output, standard input) as a virtual file of the version.
/// `sources` may be empty when streams are given.
#[allow(clippy::too_many_arguments)]
pub fn run_backup_with_streams(
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
    streams: Vec<StreamSource>,
) -> Result<BackupReport, AppError> {
    let started = Instant::now();
    let mut report = BackupReport { dry_run: opts.dry_run, ..Default::default() };
    retention.validate()?;
    super::versions::validate_note(opts.default_note.as_deref())?;
    super::versions::validate_tags(&opts.default_tags)?;
    if sources.is_empty() && streams.is_empty() {
        return Err(AppError::validation("The backup job has no source folders"));
    }
    if opts.rehash_percent > 100 {
        return Err(AppError::validation("The rehash percentage must be between 0 and 100"));
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
        let srcs: Vec<String> = split_sources(sources).0.iter().map(|p| p.to_string_lossy().into_owned()).collect();
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
    // ── Snapshot (VSS / LVM / APFS) ──
    let mut snapshot: Option<super::snapshot::Snapshot> = None;
    if opts.snapshot_policy != SnapshotPolicy::Off && !opts.dry_run {
        let live: Vec<PathBuf> = split_sources(sources).0.into_iter().filter(|p| p.exists()).collect();
        match super::snapshot::create(&live, &opts.snapshot) {
            Ok(s) => {
                report.snapshot = Some(s.method.clone());
                snapshot = Some(s);
            }
            Err(e) => match opts.snapshot_policy {
                SnapshotPolicy::Required => return Err(e),
                SnapshotPolicy::On => report.warnings.push(format!("No snapshot ({}); reading live files", e.user_message())),
                _ => tracing::info!("No snapshot: {}", e.user_message()),
            },
        }
    }
    let snap = snapshot.as_ref();
    // Generated source files (Photos albums.json) live here until the run ends.
    let staging = Staging::new(!split_sources(sources).1.is_empty(), opts)?;
    let tracker = ErrorTracker::new(opts.error_policy);
    let scanned = match (&changed, prev_list.is_empty()) {
        (Some(c), false) => {
            let (r, n) = scan_incremental_in(sources, &prev_list, c, &shallow, &opts.deleted_paths, filter, opts, snap, staging.path(), &tracker)?;
            report.rescanned_paths = n;
            r
        }
        _ => scan_sources_in(sources, filter, opts, snap, staging.path(), &tracker)?,
    };
    let mut entries = scanned.entries;
    // Virtual files of streams, after the scanned entries.
    let mut stream_of: HashMap<usize, usize> = HashMap::new();
    let mut stream_paths: HashSet<String> = HashSet::new();
    for (si, st) in streams.iter().enumerate() {
        if !stream_paths.insert(st.path.clone()) {
            return Err(AppError::validation(format!("Two streams are named {}", st.path)));
        }
        for e in super::reader::stream_entries(&st.path, opts.skip_metadata)? {
            if e.kind == EntryKind::Dir && entries.iter().any(|(_, x)| x.path == e.path) {
                continue;
            }
            if e.kind == EntryKind::File {
                stream_of.insert(entries.len(), si);
            }
            entries.push((PathBuf::from(&e.path), e));
        }
    }
    let streams: Vec<Mutex<Option<StreamSource>>> = streams.into_iter().map(|s| Mutex::new(Some(s))).collect();
    report.warnings.extend(scanned.warnings);
    report.info.extend(scanned.info);
    absorb_problems(&mut report, &tracker);
    if tracker.should_stop() {
        // Stopped while scanning: nothing was read, so there is nothing to keep.
        report.errors.extend(tracker.stop_reason());
        report.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(report);
    }
    if entries.iter().all(|(_, e)| e.kind == EntryKind::Dir) && !opts.allow_empty_source {
        return Err(AppError::validation("The source selection contains no files; refusing to create an empty version"));
    }

    let prev_files: HashMap<String, FileEntry> = prev_list.into_iter().map(|f| (f.path.clone(), f)).collect();
    let total = entries.len() as u64;
    let mut results: Vec<Option<FileEntry>> = vec![None; entries.len()];
    let mut work: VecDeque<Work> = VecDeque::new();
    // Unchanged-looking files re-read by percentage rehash → their recorded hash.
    let mut rehash: HashMap<usize, Option<String>> = HashMap::new();
    let rehash_run = rehash_run_counter(db, opts)?;
    for (i, (path, entry)) in entries.iter_mut().enumerate() {
        if entry.kind != EntryKind::File {
            results[i] = Some(entry.clone());
            continue;
        }
        report.files_examined += 1;
        report.bytes_examined += entry.size;
        let input = match stream_of.get(&i) {
            Some(si) => Input::Stream(*si),
            None => Input::Path(snap.map(|s| s.real(path)).unwrap_or_else(|| path.clone())),
        };
        match prev_files.get(&entry.path) {
            Some(prev) if unchanged(prev, entry, opts) && prev.blocks.iter().all(|b| db.has_block(b).unwrap_or(false)) => {
                if rehash_selected(&entry.path, rehash_run, opts.rehash_percent) {
                    rehash.insert(i, prev.hash.clone());
                    report.files_rehashed += 1;
                    work.push_back(Work { idx: i, input, level: compress_level(path, opts), size: entry.size });
                    continue;
                }
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
        work.push_back(Work { idx: i, input, level: compress_level(path, opts), size: entry.size });
    }

    // ── Parallel read/hash/compress → dedup/pack → concurrent upload ──
    let abort = Arc::new(AtomicBool::new(false));
    // Set by the error policy (fail-fast, error budget) or the upload cap:
    // stop reading, keep what is finished as a partial version.
    let halt = Arc::new(AtomicBool::new(false));
    let stop = {
        let (abort, halt) = (abort.clone(), halt.clone());
        let ctx = ctx.clone();
        move || abort.load(Ordering::SeqCst) || halt.load(Ordering::SeqCst) || ctx.is_cancelled()
    };
    let cfg = WorkerCfg {
        chunker: repo.chunker(),
        codec: repo.header.block_codec,
        read_buf: opts.read_buffer_size,
        block_hash: repo.header.block_hash,
        file_hash: repo.header.file_hash,
        format: repo.header.volume_format,
        respect_locks: opts.respect_advisory_locks,
        background: opts.background_io,
        pause: std::time::Duration::from_micros((10 - opts.cpu_intensity.clamp(1, 10) as u64) * 500),
        parallel_threshold: opts.parallel_file_threshold,
        helpers: opts.worker_threads(),
    };
    let mut pending = Pending::for_repo(&repo, opts);
    let mut in_flight: HashSet<String> = HashSet::new();
    let mut file_blocks: HashMap<usize, Vec<String>> = HashMap::new();
    let mut upload_error: Option<String> = None;
    let mut done_count = 0u64;
    let queue = Mutex::new(work);
    let threads = opts.worker_threads();
    let uploaders = opts.upload_concurrency.max(1);
    let mut checkpoints = Checkpoints::new(opts);
    let checkpoints_on = checkpoints.enabled(opts);

    std::thread::scope(|scope| -> Result<(), AppError> {
        let (tx, rx) = mpsc::sync_channel::<Msg>(256);
        for _ in 0..threads {
            let tx = tx.clone();
            let (queue, cfg, stop, streams) = (&queue, &cfg, &stop, &streams);
            scope.spawn(move || {
                if cfg.background {
                    meta::set_background_priority();
                }
                loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                    let Some(w) = next else { break };
                    if stop() {
                        break;
                    }
                    if let Err(error) = read_input(cfg, &w, streams, &tx, stop) {
                        if tx.send(Msg::Failed { idx: w.idx, error }).is_err() {
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
                let (job_rx, res_tx, par2, backend) = (job_rx.clone(), res_tx.clone(), repo.par2.as_ref(), &repo.backend);
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

        // Records a finished upload; true when a volume is now durable.
        let handle_result = |r: UploadResult, in_flight: &mut HashSet<String>, upload_error: &mut Option<String>| -> Result<bool, AppError> {
            match r.error {
                Some(e) => {
                    *upload_error = Some(format!("Upload of {} failed: {e}", r.volume.block_name));
                    abort.store(true, Ordering::SeqCst);
                    Ok(false)
                }
                None => {
                    record_volume(&repo, &r.volume, &r.par2_names)?;
                    for (h, _) in &r.volume.blocks {
                        in_flight.remove(h);
                    }
                    Ok(true)
                }
            }
        };

        let send_volume = |pending: &mut Pending, in_flight: &mut HashSet<String>, report: &mut BackupReport| -> Result<(), AppError> {
            let Some((w, blocks)) = pending.take() else { return Ok(()) };
            let v = seal_pending(&repo, w, blocks)?;
            report.bytes_uploaded += (v.block_data.len() + v.index_data.len()) as u64;
            report.volumes_uploaded += 1;
            if opts.max_upload_bytes_per_run.is_some_and(|cap| report.bytes_uploaded >= cap) && !report.upload_cap_reached {
                // Stop reading; this volume and the files finished so far still go up.
                report.upload_cap_reached = true;
                halt.store(true, Ordering::SeqCst);
            }
            if opts.dry_run {
                return Ok(());
            }
            repo.note_write()?;
            repo.db.add_volume(&v.block_name, VolumeKind::Block, v.block_data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &v.block_data), "uploading")?;
            in_flight.extend(v.blocks.iter().map(|(h, _)| h.clone()));
            job_tx.send(v).map_err(|_| AppError::internal("upload pool stopped"))
        };

        for msg in rx {
            while let Ok(r) = res_rx.try_recv() {
                let durable = handle_result(r, &mut in_flight, &mut upload_error)?;
                if durable && checkpoints_on && upload_error.is_none() && checkpoints.due(report.bytes_uploaded) {
                    // Files whose blocks are all recorded as uploaded.
                    let files: Vec<FileEntry> = results
                        .iter()
                        .flatten()
                        .filter(|e| !e.blocks.iter().any(|b| pending.hashes.contains(b) || in_flight.contains(b)))
                        .cloned()
                        .collect();
                    checkpoints.write(&repo, files, opts, &mut report)?;
                }
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
                    if let Some(expected) = rehash.remove(&idx) {
                        if expected.as_deref() != Some(hash.as_str()) {
                            report.rehash_mismatches += 1;
                            report.files_changed += 1;
                            report.warnings.push(format!("{}: content changed without timestamp change (possible silent corruption); the new content was backed up", e.path));
                        }
                    }
                    e.hash = Some(hash);
                    done_count += 1;
                    progress(ctx, Phase::Uploading, done_count, total, report.bytes_examined, Some(&e.path));
                    results[idx] = Some(e);
                }
                Msg::Failed { idx, error } => {
                    file_blocks.remove(&idx);
                    rehash.remove(&idx);
                    if error != "cancelled" {
                        let path = &entries[idx].1.path;
                        if stream_of.contains_key(&idx) {
                            // A failed stream (its command exited non-zero) fails the run.
                            report.errors.push(format!("Stream {path} failed: {error}"));
                        } else {
                            tracker.record(ErrorClass::File, format!("Skipped {path}: {error}"));
                        }
                        if tracker.should_stop() {
                            halt.store(true, Ordering::SeqCst);
                        }
                        if let Some(prev) = prev_files.get(path) {
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
    // From here on a final (or synthetic partial) version supersedes the checkpoint.
    absorb_problems(&mut report, &tracker);

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
    let halted = halt.load(Ordering::SeqCst);
    if ctx.is_cancelled() || halted {
        // Synthetic partial version: everything finished so far stays restorable.
        if !done.is_empty() {
            let ts = next_timestamp(db)?;
            report.version = Some(repo.write_filelist(ts, true, &done, opts.task_config.clone(), control_files, version_meta(opts, &report), opts.dry_run)?);
            report.partial = true;
            checkpoints.remove(&repo)?;
        }
        match tracker.stop_reason() {
            _ if ctx.is_cancelled() => report.errors.push("The operation was cancelled. Start it again when ready; completed work is kept.".into()),
            Some(r) => report.errors.push(format!("{r}. The files finished so far were kept as a partial version.")),
            None if report.upload_cap_reached => report.warnings.push(format!(
                "Upload limit of {} per run reached; the files finished so far were kept as a partial version and the next run continues",
                human(opts.max_upload_bytes_per_run.unwrap_or_default())
            )),
            None => report.errors.push("The backup stopped early; completed work is kept.".into()),
        }
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
        report.version = Some(repo.write_filelist(ts, false, &done, opts.task_config.clone(), control_files, version_meta(opts, &report), opts.dry_run)?);
    }
    // The complete version (or the unchanged previous one) supersedes the checkpoint.
    checkpoints.remove(&repo)?;
    if let (Some(state), false) = (&usn_next, opts.dry_run) {
        db.set_kv(USN_KEY, &serde_json::to_string(state)?)?;
    }
    // Sources are no longer needed: scripts may release snapshots/locks now.
    // Sources fully read: release the snapshot before uploads finish.
    if let Some(s) = snapshot.take() {
        report.warnings.extend(s.release());
    }
    ctx.source_released();

    if let (Some(l), false) = (opts.object_lock, opts.dry_run) {
        // Lock before retention/compaction can touch anything.
        let r = super::maintenance::apply_locks(&repo, l.days, l.mode, false)?;
        report.errors.extend(r.errors);
    }
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

/// Run number for choosing rehash files (advances once per real run).
fn rehash_run_counter(db: &BackupDb, opts: &BackupOptions) -> Result<u64, AppError> {
    if opts.rehash_percent == 0 {
        return Ok(0);
    }
    let n = db.get_kv("rehash_run")?.and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + 1;
    if !opts.dry_run {
        db.set_kv("rehash_run", &n.to_string())?;
    }
    Ok(n)
}

/// Whether `path` is among the ~`percent`% of files re-read in run `run`.
/// A hash of (path, run) spreads the choice so every file comes up over time.
pub(crate) fn rehash_selected(path: &str, run: u64, percent: u8) -> bool {
    if percent == 0 {
        return false;
    }
    if percent >= 100 {
        return true;
    }
    let h = hash_bytes(HashAlgorithm::Sha256, format!("{run}\0{path}").as_bytes());
    let v = u64::from_str_radix(&h[..12], 16).unwrap_or(0);
    v % 10_000 < percent as u64 * 100
}

/// Move the problems the error policy recorded into the report.
fn absorb_problems(report: &mut BackupReport, tracker: &ErrorTracker) {
    let (w, e) = tracker.take();
    report.warnings.extend(w);
    report.errors.extend(e);
}

/// Metadata recorded with a new version: the job's default note and tags,
/// who wrote it and what this run added so far.
fn version_meta(opts: &BackupOptions, report: &BackupReport) -> VersionMeta {
    VersionMeta {
        note: opts.default_note.clone().filter(|n| !n.trim().is_empty()),
        tags: opts.default_tags.clone(),
        identity: Some(opts.identity.resolve()),
        stats: Some(VersionStats {
            bytes_added: report.bytes_uploaded,
            new_blocks: report.new_blocks,
            files_added: report.files_added,
            files_changed: report.files_changed,
            files_deleted: report.files_deleted,
            upload_cap_reached: report.upload_cap_reached,
        }),
        ..Default::default()
    }
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
    fn gpg_symmetric_backup_restore_and_recreate() {
        if std::process::Command::new("gpg").arg("--version").output().is_err() {
            return;
        }
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        write(src.path(), "secret.txt", b"gpg protected payload");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions {
            encryption_module: crate::backup::EncryptionModuleKind::Gpg,
            gpg: crate::backup::GpgOptions { homedir: home.path().to_string_lossy().into(), ..Default::default() },
            ..opts()
        };
        assert!(run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new()).is_err(), "needs a passphrase or recipients");
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("gpg-pass"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let header: RepoHeader = serde_json::from_slice(&std::fs::read(dst.path().join("ufop-repo.json")).unwrap()).unwrap();
        assert_eq!((header.encrypted, header.encryption_module.as_str(), header.salt.is_none()), (true, "gpg", true));
        // Volumes are OpenPGP messages, not readable as plain zip/ufop data.
        let vol = std::fs::read_dir(dst.path()).unwrap().flatten().find(|e| e.file_name().to_string_lossy().contains("dblock")).unwrap().path();
        assert!(!String::from_utf8_lossy(&std::fs::read(vol).unwrap()).contains("gpg protected payload"));

        // A fresh machine: rebuild the database from the destination, then restore.
        std::env::set_var("GNUPGHOME", home.path());
        let fresh = BackupDb::open_in_memory().unwrap();
        crate::backup::maintenance::recreate_database(&be, &fresh, Some("gpg-pass"), "ufop", &OpContext::new()).unwrap();
        assert!(crate::backup::maintenance::recreate_database(&be, &BackupDb::open_in_memory().unwrap(), Some("wrong"), "ufop", &OpContext::new()).is_err());
        let repo = Repo::open(&be, &fresh, Some("gpg-pass"), &o, false, KdfParams::insecure_fast()).unwrap();
        let rr = crate::backup::run_restore(&repo, &crate::backup::RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        let restored = walk_find(out.path(), "secret.txt").unwrap();
        assert_eq!(std::fs::read(restored).unwrap(), b"gpg protected payload");
    }

    fn walk_find(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == name) {
                return Some(p);
            }
            if p.is_dir() {
                if let Some(f) = walk_find(&p, name) {
                    return Some(f);
                }
            }
        }
        None
    }

    #[test]
    fn kdf_settings_are_validated_and_recorded() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "f", b"x");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let weak = BackupOptions { kdf: Some(KdfParams { memory_kib: 1024, iterations: 1, parallelism: 2 }), ..opts() };
        assert!(run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &weak, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).is_err());
        let custom = KdfParams { memory_kib: 8 * 1024, iterations: 2, parallelism: 1 };
        let o = BackupOptions { kdf: Some(custom), ..opts() };
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        let header: RepoHeader = serde_json::from_slice(&std::fs::read(dst.path().join("ufop-repo.json")).unwrap()).unwrap();
        assert_eq!((header.kdf, header.encryption_module.as_str(), header.format_version), (Some(custom), "aes", crate::backup::REPO_FORMAT_VERSION));
        assert_eq!(KdfParams::preset("interactive").unwrap().memory_kib, 19 * 1024);
        assert!(KdfParams::preset("sensitive").unwrap().validate().is_ok() && KdfParams::preset("x").is_none());
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
        repo.rewrite_filelist(fs, &db.files(fs.id).unwrap()).unwrap();
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
        // Sizes below assume format 1 (deflate, uncompressed metadata).
        let o = BackupOptions { quota_size: Some(1000), quota_warning_percent: 10, format_version: Some(1), ..opts() };
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

    #[test]
    fn backup_reads_through_snapshot_but_keeps_original_paths() {
        let src = tempfile::tempdir().unwrap();
        let frozen = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(src.path(), "doc.txt", b"live, changing");
        write(frozen.path(), "doc.txt", b"frozen snapshot copy");
        crate::backup::snapshot::test_hook::set(vec![(src.path().to_path_buf(), frozen.path().to_path_buf())]);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { snapshot_policy: crate::backup::snapshot::SnapshotPolicy::Required, ..opts() };
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll,
            None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert_eq!(r.snapshot.as_deref(), Some("test"));
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        assert!(files.iter().any(|f| f.path == src.path().join("doc.txt").to_string_lossy()), "original path recorded");
        restore_all(&be, &db, &o, None, out.path());
        assert_eq!(std::fs::read(walkdir_file(out.path(), "doc.txt")).unwrap(), b"frozen snapshot copy");
    }

    fn photos_fixture(dir: &Path) -> (PathBuf, PathBuf) {
        use crate::backup::photos::fixture::{cd, library, Asset};
        let modern = [
            Asset { pk: 1, uuid: "AAAA1111-X", dir: "A", file: "AAAA1111-X.jpeg", original: Some("IMG_0001.JPG"), created: Some(cd(2023, 3, 14)), favorite: true, content: Some(b"first photo"), ..Default::default() },
            Asset { pk: 2, uuid: "BBBB2222-X", dir: "B", file: "BBBB2222-X.jpeg", original: Some("IMG_0001.JPG"), created: Some(cd(2023, 3, 20)), content: Some(b"same name, other photo"), ..Default::default() },
            Asset { pk: 3, uuid: "CCCC3333-X", dir: "C", file: "CCCC3333-X.mov", original: Some("Clip.MOV"), created: Some(cd(2024, 8, 1)), kind: 1, content: Some(b"video bytes"), ..Default::default() },
            Asset { pk: 4, uuid: "DDDD4444-X", dir: "D", file: "DDDD4444-X.jpeg", original: Some("deleted.jpg"), created: Some(cd(2024, 1, 1)), trashed: true, content: Some(b"in the trash"), ..Default::default() },
            Asset { pk: 5, uuid: "EEEE5555-X", dir: "E", file: "EEEE5555-X.jpeg", original: Some("secret.jpg"), created: Some(cd(2024, 1, 2)), hidden: true, content: Some(b"hidden photo"), ..Default::default() },
            Asset { pk: 6, uuid: "FFFF6666-X", dir: "F", file: "FFFF6666-X.jpeg", original: Some("icloud.jpg"), created: Some(cd(2024, 1, 3)), content: None, ..Default::default() },
        ];
        let lib = library(dir, "Photos Library", false, &modern, &[(20, "Trip", 2, false, &[1, 2, 6]), (21, "Family", 2, false, &[3, 5])]);
        let legacy = [Asset { pk: 1, uuid: "LEG-1", dir: "2016/07/04/20160704-101010", file: "IMG_0100.JPG", original: Some("IMG_0100.JPG"), created: Some(cd(2016, 7, 4)), content: Some(b"from Masters"), ..Default::default() }];
        let old = library(dir, "Old Library", true, &legacy, &[(7, "Summer", 2, false, &[1])]);
        (lib, old)
    }

    fn leftovers(tmp: &Path) -> Vec<String> {
        std::fs::read_dir(tmp)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.starts_with("ufop-photos-") || n.starts_with("ufop-staging-"))
            .collect()
    }

    #[test]
    fn photos_library_backup_and_restore() {
        use crate::backup::photos::{source_uri, Manifest};
        let pics = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let (lib, old) = photos_fixture(pics.path());
        let sources: Vec<PathBuf> = vec![source_uri(&lib).into(), source_uri(&old).into()];
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { temp_dir: Some(tmp.path().to_string_lossy().into_owned()), quota_disable: true, ..opts() };
        let run = |o: &BackupOptions| run_backup(&be, &db, &sources, &SourceFilter::default(), o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();

        let r = run(&o);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.files_added, 6, "3 modern + 1 legacy original + 2 albums.json (hidden, trashed and missing left out)");
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        assert!(r.warnings[0].contains("icloud.jpg"), "{}", r.warnings[0]);
        assert!(leftovers(tmp.path()).is_empty(), "temporary database copies and staging removed");

        let root = pics.path().join("Photos Library (Photos export)");
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        let has = |p: PathBuf| files.iter().any(|f| f.path == p.to_string_lossy());
        assert!(has(root.join("Photos/2023/03/IMG_0001.JPG")));
        assert!(has(root.join("Photos/2023/03/IMG_0001 (BBBB2222).JPG")));
        assert!(has(root.join("Photos/2024/08/Clip.MOV")));
        assert!(has(root.join("albums.json")));
        assert!(has(root.join("Photos/2024")) && has(root.clone()), "folders recorded");
        assert!(!files.iter().any(|f| f.path.contains("deleted.jpg") || f.path.contains("secret.jpg") || f.path.contains("icloud.jpg")));
        assert!(!files.iter().any(|f| f.path.contains(".photoslibrary")), "only logical paths are recorded");

        // Restore: a plain folder tree.
        let rr = restore_all(&be, &db, &o, Some("pw"), out.path());
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        let ex = out.path().join("Photos Library (Photos export)");
        assert_eq!(std::fs::read(ex.join("Photos/2023/03/IMG_0001.JPG")).unwrap(), b"first photo");
        assert_eq!(std::fs::read(ex.join("Photos/2023/03/IMG_0001 (BBBB2222).JPG")).unwrap(), b"same name, other photo");
        assert_eq!(std::fs::read(ex.join("Photos/2024/08/Clip.MOV")).unwrap(), b"video bytes");
        assert_eq!(std::fs::read(out.path().join("Old Library (Photos export)/Photos/2016/07/IMG_0100.JPG")).unwrap(), b"from Masters");
        let m: Manifest = serde_json::from_slice(&std::fs::read(ex.join("albums.json")).unwrap()).unwrap();
        let trip = m.albums.iter().find(|a| a.title == "Trip").unwrap();
        assert_eq!(trip.assets, ["Photos/2023/03/IMG_0001.JPG", "Photos/2023/03/IMG_0001 (BBBB2222).JPG"]);
        assert_eq!(m.albums.iter().find(|a| a.title == "Family").unwrap().assets, ["Photos/2024/08/Clip.MOV"]);
        let a = m.assets.iter().find(|a| a.uuid == "AAAA1111-X").unwrap();
        assert!(a.favorite);
        assert_eq!(a.created.as_deref(), Some("2023-03-14T12:00:00+00:00"));
        assert_eq!(a.albums, ["Trip"]);
        for a in &m.assets {
            assert!(ex.join(&a.path).is_file(), "manifest path {} restored", a.path);
        }
        let legacy: Manifest = serde_json::from_slice(&std::fs::read(out.path().join("Old Library (Photos export)/albums.json")).unwrap()).unwrap();
        assert_eq!(legacy.albums[0].assets, ["Photos/2016/07/IMG_0100.JPG"]);

        // Unchanged library: no new version (albums.json is deterministic).
        let r2 = run(&o);
        assert!(r2.skipped_unchanged, "{r2:?}");
        assert_eq!(db.filesets().unwrap().len(), 1);

        // Options: hidden + trashed + album filter.
        let o3 = BackupOptions { photos: crate::backup::photos::PhotosOptions { include_hidden: true, include_trashed: true, albums: vec!["family".into()] }, ..o.clone() };
        let r3 = run(&o3);
        assert!(r3.version.is_some());
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        let photos: Vec<&str> = files.iter().filter(|f| f.kind == EntryKind::File && !f.path.ends_with("albums.json")).map(|f| f.path.as_str()).collect();
        assert_eq!(photos.len(), 2, "{photos:?}");
        assert!(photos.iter().any(|p| p.ends_with("Clip.MOV")) && photos.iter().any(|p| p.ends_with("secret.jpg")));
        assert!(r3.warnings.iter().any(|w| w.contains("no album named 'family'") && w.contains("Old Library")), "{:?}", r3.warnings);

        // Incremental runs (changed paths) still re-expand the library.
        let o4 = BackupOptions { changed_paths: Some(vec![]), ..o.clone() };
        let r4 = run(&o4);
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        assert!(r4.version.is_some());
        assert!(files.iter().any(|f| f.path.ends_with("IMG_0001.JPG")) && !files.iter().any(|f| f.path.ends_with("secret.jpg")));
        assert!(leftovers(tmp.path()).is_empty());
    }

    #[test]
    fn photos_sources_mix_with_folders_and_honour_policies() {
        use crate::backup::photos::source_uri;
        let pics = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        write(src.path(), "notes.txt", b"plain file");
        let (lib, _) = photos_fixture(pics.path());
        let gone = pics.path().join("Gone.photoslibrary");
        let o = opts();
        let (entries, warnings) = scan_sources(&[src.path().into(), source_uri(&lib).into()], &SourceFilter::default(), &o).unwrap();
        assert!(entries.iter().any(|(_, e)| e.path.ends_with("notes.txt")));
        assert!(entries.iter().any(|(_, e)| e.path.ends_with("albums.json")));
        assert_eq!(warnings.len(), 1);

        // Missing library follows the missing-source policy.
        assert!(scan_sources(&[source_uri(&gone).into()], &SourceFilter::default(), &o).is_err());
        let warn = BackupOptions { missing_source: MissingSourcePolicy::Warn, ..opts() };
        let (e, w) = scan_sources(&[src.path().into(), source_uri(&gone).into()], &SourceFilter::default(), &warn).unwrap();
        assert!(e.iter().all(|(_, e)| !e.path.contains("Gone")));
        assert!(w.iter().any(|w| w.contains("Gone.photoslibrary")));

        // Filters see logical paths: excluding a year folder drops it.
        let mut f = SourceFilter::default();
        f.rules = vec![crate::backup::FilterRule { action: crate::backup::FilterAction::Exclude, kind: crate::backup::FilterKind::Wildcard, pattern: "*/Photos/2024/".into() }];
        let f = f.compile().unwrap();
        let (e, _) = scan_sources(&[source_uri(&lib).into()], &f, &o).unwrap();
        assert!(!e.iter().any(|(_, e)| e.path.contains("2024")), "{:?}", e.iter().map(|x| &x.1.path).collect::<Vec<_>>());
        assert!(e.iter().any(|(_, e)| e.path.contains("2023")));
    }

    /// Fails every block upload after the first `ok` ones (a dropped
    /// connection or a crash in the middle of a run).
    struct DiesAfter {
        inner: LocalBackend,
        ok: usize,
        seen: std::sync::atomic::AtomicUsize,
    }
    impl Backend for DiesAfter {
        fn describe(&self) -> String { self.inner.describe() }
        fn list(&self) -> Result<Vec<crate::backup::RemoteFile>, AppError> { self.inner.list() }
        fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> {
            if n.contains(".dblock") && self.seen.fetch_add(1, Ordering::SeqCst) >= self.ok {
                return Err(AppError::state("connection lost", "simulated"));
            }
            self.inner.put(n, d)
        }
        fn get(&self, n: &str) -> Result<Vec<u8>, AppError> { self.inner.get(n) }
        fn delete(&self, n: &str) -> Result<(), AppError> { self.inner.delete(n) }
        fn create_folder(&self) -> Result<(), AppError> { self.inner.create_folder() }
    }

    fn many_files(root: &Path, n: usize) {
        for i in 0..n {
            // Distinct, incompressible-ish content: 3 blocks per file.
            let data: Vec<u8> = (0..3000u32).map(|j| ((j * 7919 + i as u32 * 104_729) % 251) as u8 ^ (i as u8)).collect();
            write(root, &format!("d{}/f{i:03}.bin", i % 4), &data);
        }
    }

    fn dlists(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".dlist")).count()
    }

    #[test]
    fn checkpoints_replace_each_other_and_the_final_version_supersedes_them() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        many_files(src.path(), 24);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { checkpoint_interval_bytes: 1, upload_concurrency: 1, compression_level: 0, verify_samples: 0, ..opts() };
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(r.checkpoints > 0, "{r:?}");
        let sets = db.filesets().unwrap();
        assert_eq!(sets.len(), 1, "only the final version is left");
        assert!(!sets[0].partial);
        assert_eq!(dlists(dst.path()), 1, "checkpoint file lists were deleted remotely");
        // Off by default for short runs; 0 disables.
        let off = BackupOptions { checkpoint_interval_secs: 0, ..o.clone() };
        assert!(!Checkpoints::new(&off).enabled(&off) || off.checkpoint_interval_bytes > 0);
        let none = BackupOptions { checkpoint_interval_secs: 0, checkpoint_interval_bytes: 0, ..o };
        assert!(!Checkpoints::new(&none).enabled(&none));
        assert!(!Checkpoints::new(&BackupOptions { dry_run: true, ..opts() }).enabled(&BackupOptions { dry_run: true, ..opts() }));
    }

    #[test]
    fn a_run_that_dies_leaves_a_restorable_checkpoint_and_the_next_run_resumes() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        many_files(src.path(), 40);
        let dying = DiesAfter { inner: LocalBackend::new(dst.path()), ok: 20, seen: Default::default() };
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { checkpoint_interval_bytes: 1, upload_concurrency: 1, hash_threads: 1, compression_level: 0, verify_samples: 0, ..opts() };
        let r = run_backup(&dying, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(!r.errors.is_empty() && r.version.is_none(), "{r:?}");
        assert!(r.checkpoints > 0, "{r:?}");
        let sets = db.filesets().unwrap();
        assert_eq!(sets.len(), 1);
        assert!(sets[0].partial, "the checkpoint stays as a partial version");
        assert_eq!(dlists(dst.path()), 1);
        let saved = db.files(sets[0].id).unwrap();
        let saved_files = saved.iter().filter(|e| e.kind == EntryKind::File).count();
        assert!(saved_files > 0 && saved_files < 40, "{saved_files}");

        // A new computer (database rebuilt from the destination) restores it.
        let be = LocalBackend::new(dst.path());
        let db2 = BackupDb::open_in_memory().unwrap();
        crate::backup::maintenance::recreate_database(&be, &db2, Some("pw"), "ufop", &OpContext::new()).unwrap();
        assert!(db2.filesets().unwrap()[0].partial);
        let repo = Repo::open(&be, &db2, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
        let rr = crate::backup::run_restore(&repo, &crate::backup::RestoreOptions { target: Some(out.path().into()), preserve_full_paths: false, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert_eq!(rr.files_restored as usize, saved_files);
        for e in saved.iter().filter(|e| e.kind == EntryKind::File) {
            let name = Path::new(&e.path).file_name().unwrap().to_string_lossy().into_owned();
            let found = walk_find(out.path(), &name).expect("restored");
            assert_eq!(std::fs::read(found).unwrap(), std::fs::read(&e.path).unwrap());
        }
        drop(repo);

        // The next run on the healthy destination re-uses every uploaded block.
        let total_blocks = 40 * 3;
        let r2 = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r2.errors.is_empty(), "{:?}", r2.errors);
        assert!(r2.version.is_some());
        assert!(r2.new_blocks < total_blocks as u64 - saved_files as u64 * 3 + 1, "{} new blocks", r2.new_blocks);
        let sets = db.filesets().unwrap();
        assert_eq!((sets.len(), sets[0].partial, sets[1].partial), (2, false, true));
    }
}

#[cfg(test)]
mod source_tests;
