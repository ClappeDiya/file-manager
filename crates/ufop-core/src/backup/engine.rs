//! Backup run: scan → deduplicate → pack/compress/encrypt → upload → write
//! the version's file list → retention → compaction → sampled verification.

use super::backend::Backend;
use super::crypto::{KdfParams, VolumeCipher};
use super::db::BackupDb;
use super::filters::SourceFilter;
use super::retention::RetentionPolicy;
use super::volume::{self, BlockVolumeWriter, FileListVolume, IndexVolume, IndexedBlock, RepoHeader, VolumeKind};
use super::{
    hash_bytes, BackupOptions, EntryKind, FileEntry, FileHasher, HardlinkPolicy, HashAlgorithm, MissingSourcePolicy, OpContext,
    Phase, Progress, SymlinkPolicy, REPO_FORMAT_VERSION,
};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// An opened repository: backend + local database + key material.
pub struct Repo<'a> {
    pub backend: &'a dyn Backend,
    pub db: &'a BackupDb,
    pub cipher: VolumeCipher,
    pub header: RepoHeader,
    pub prefix: String,
}

impl<'a> Repo<'a> {
    /// Open (or, with `create`, initialise) the repository at `backend`.
    /// The block size and hash algorithms of an existing repository win over
    /// `options`, because they cannot change after the first backup.
    pub fn open(
        backend: &'a dyn Backend,
        db: &'a BackupDb,
        passphrase: Option<&str>,
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
        let exists = listing.iter().any(|f| f.name == header_name);
        let (header, cipher) = if exists {
            let header: RepoHeader = serde_json::from_slice(&backend.get(&header_name)?)?;
            if header.format_version > REPO_FORMAT_VERSION {
                return Err(AppError::validation(format!(
                    "This backup was written by a newer version (format {}); update the app",
                    header.format_version
                )));
            }
            let cipher = if header.encrypted {
                let pass = passphrase.filter(|p| !p.is_empty()).ok_or_else(|| {
                    AppError::validation("This backup is encrypted; enter its passphrase")
                })?;
                let salt = hex::decode(header.salt.as_deref().unwrap_or_default())
                    .map_err(|_| AppError::validation("Repository header salt is malformed"))?;
                let c = VolumeCipher::derive(pass, &salt, header.kdf.unwrap_or_default())?;
                c.verify_key_check(header.key_check.as_deref().unwrap_or_default())?;
                c
            } else {
                VolumeCipher::plain()
            };
            (header, cipher)
        } else {
            if !create {
                return Err(AppError::file_op(
                    format!("No backup found at {}", backend.describe()),
                    "Check the destination and file-name prefix.",
                ));
            }
            let (cipher, salt) = match passphrase.filter(|p| !p.is_empty()) {
                Some(p) => {
                    let salt = VolumeCipher::random_salt();
                    (VolumeCipher::derive(p, &salt, kdf)?, Some(hex::encode(&salt)))
                }
                None => (VolumeCipher::plain(), None),
            };
            let header = RepoHeader {
                format_version: REPO_FORMAT_VERSION,
                encrypted: cipher.is_encrypted(),
                salt,
                kdf: cipher.is_encrypted().then_some(kdf),
                key_check: cipher.key_check()?,
                block_size: options.block_size.max(1024),
                block_hash: options.block_hash,
                file_hash: options.file_hash,
            };
            if !options.dry_run {
                backend.create_folder()?;
                backend.put(&header_name, &serde_json::to_vec_pretty(&header)?)?;
            }
            (header, cipher)
        };
        Ok(Self { backend, db, cipher, header, prefix: options.prefix.clone() })
    }

    pub(crate) fn put_volume(&self, kind: VolumeKind, name: &str, data: &[u8], dry_run: bool) -> Result<i64, AppError> {
        let h = hash_bytes(HashAlgorithm::Sha256, data);
        if dry_run {
            return Ok(-1);
        }
        self.db.add_volume(name, kind, data.len() as u64, &h, "uploading")?;
        self.backend.put(name, data)?;
        self.db.set_volume_state(name, "uploaded")?;
        Ok(self.db.volume_by_name(name)?.map(|v| v.id).unwrap_or(-1))
    }

    /// Download a volume and check its recorded SHA-256 (when known).
    pub(crate) fn get_volume(&self, name: &str) -> Result<Vec<u8>, AppError> {
        let data = self.backend.get(name)?;
        if let Some(v) = self.db.volume_by_name(name)?.filter(|v| !v.hash.is_empty()) {
            if v.hash != hash_bytes(HashAlgorithm::Sha256, &data) {
                return Err(AppError::transfer(
                    format!("Remote volume {name} does not match its recorded hash"),
                    "The file was modified or damaged on the destination. Run Repair or list-broken-files.",
                ));
            }
        }
        Ok(data)
    }

    /// Write a sealed file list for `files` and record the version.
    pub(crate) fn write_filelist(
        &self,
        ts: DateTime<Utc>,
        partial: bool,
        files: &[FileEntry],
        task_config: Option<serde_json::Value>,
        dry_run: bool,
    ) -> Result<String, AppError> {
        let name = volume::filelist_volume_name(&self.prefix, ts, &self.cipher);
        let fl = FileListVolume { timestamp: ts.to_rfc3339(), partial, files: files.to_vec(), task_config };
        let data = volume::seal_json(&self.cipher, &fl)?;
        self.put_volume(VolumeKind::FileList, &name, &data, dry_run)?;
        if !dry_run {
            self.db.add_fileset(&fl.timestamp, &name, partial, files)?;
        }
        Ok(fl.timestamp)
    }

    /// Compare the local volume inventory with the remote listing.
    pub fn consistency_check(&self) -> Result<ConsistencyReport, AppError> {
        let remote: HashMap<String, u64> = self.backend.list()?.into_iter().map(|f| (f.name, f.size)).collect();
        let mut r = ConsistencyReport::default();
        for v in self.db.volumes(None)? {
            match remote.get(&v.name) {
                None if v.state == "uploaded" || v.state == "verified" => r.missing.push(v.name),
                None => r.incomplete.push(v.name),
                Some(sz) if *sz != v.size => r.size_mismatch.push(v.name),
                _ => {}
            }
        }
        let known: HashSet<String> = self.db.volumes(None)?.into_iter().map(|v| v.name).collect();
        for name in remote.keys() {
            if volume::classify(&self.prefix, name).is_some() && !known.contains(name) {
                r.extra.push(name.clone());
            }
        }
        r.db_problems = self.db.integrity_problems()?;
        Ok(r)
    }
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
    pub(crate) fn new() -> Self {
        Self { writer: BlockVolumeWriter::new(), sizes: Vec::new(), hashes: HashSet::new() }
    }

    pub(crate) fn add_block(&mut self, hash: String, data: &[u8], level: u32) -> Result<(), AppError> {
        self.writer.add(hash.clone(), data, level)?;
        self.sizes.push((hash.clone(), data.len() as u64));
        self.hashes.insert(hash);
        Ok(())
    }

    pub(crate) fn byte_size(&self) -> usize {
        self.writer.byte_size()
    }
}

pub(crate) fn flush(repo: &Repo, p: &mut Pending, opts: &BackupOptions, report: &mut BackupReport) -> Result<(), AppError> {
    if p.writer.is_empty() {
        return Ok(());
    }
    let old = std::mem::replace(p, Pending::new());
    let name = volume::block_volume_name(&repo.prefix, &repo.cipher);
    let sealed = repo.cipher.seal(&old.writer.finish())?;
    let vid = repo.put_volume(VolumeKind::Block, &name, &sealed, opts.dry_run)?;
    report.bytes_uploaded += sealed.len() as u64;
    report.volumes_uploaded += 1;
    let idx = IndexVolume {
        block_volume: name.clone(),
        blocks: old.sizes.iter().map(|(h, s)| IndexedBlock { hash: h.clone(), size: *s }).collect(),
    };
    let iname = volume::index_volume_name(&repo.prefix, &repo.cipher);
    let idata = volume::seal_json(&repo.cipher, &idx)?;
    repo.put_volume(VolumeKind::Index, &iname, &idata, opts.dry_run)?;
    report.bytes_uploaded += idata.len() as u64;
    if !opts.dry_run {
        repo.db.set_kv(&format!("index:{name}"), &iname)?;
        repo.db.begin()?;
        for (h, s) in &old.sizes {
            repo.db.add_block(h, *s, vid)?;
        }
        repo.db.commit()?;
    }
    Ok(())
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
    seen_inodes: HashSet<(u64, u64)>,
    out: Vec<(PathBuf, FileEntry)>,
    warnings: Vec<String>,
}

impl Scanner<'_> {
    fn entry(&self, path: &Path, kind: EntryKind, md: &std::fs::Metadata) -> FileEntry {
        FileEntry {
            path: path.to_string_lossy().into_owned(),
            kind,
            size: if kind == EntryKind::File { md.len() } else { 0 },
            mtime: if self.opts.skip_metadata { None } else { mtime_of(md) },
            mode: if self.opts.skip_metadata { None } else { mode_of(md) },
            hash: None,
            blocks: Vec::new(),
            symlink_target: None,
        }
    }

    #[allow(unused_variables)]
    fn hardlink_skip(&mut self, md: &std::fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if md.nlink() > 1 {
                match self.opts.hardlink_policy {
                    HardlinkPolicy::All => {}
                    HardlinkPolicy::None => return true,
                    HardlinkPolicy::First => return !self.seen_inodes.insert((md.dev(), md.ino())),
                }
            }
        }
        false
    }

    fn walk(&mut self, path: &Path, depth: usize) {
        if depth > 256 {
            self.warnings.push(format!("Skipping {}: directory nesting too deep", path.display()));
            return;
        }
        let lmd = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                self.warnings.push(format!("Cannot read {}: {e}", path.display()));
                return;
            }
        };
        let md = if lmd.file_type().is_symlink() {
            match self.opts.symlink_policy {
                SymlinkPolicy::Ignore => return,
                SymlinkPolicy::Store => {
                    if self.filter.evaluate(&path.to_string_lossy(), false) == Some(super::FilterAction::Exclude) {
                        return;
                    }
                    let mut e = self.entry(path, EntryKind::Symlink, &lmd);
                    e.symlink_target = std::fs::read_link(path).ok().map(|t| t.to_string_lossy().into_owned());
                    self.out.push((path.to_path_buf(), e));
                    return;
                }
                SymlinkPolicy::Follow => match std::fs::metadata(path) {
                    Ok(m) => m,
                    Err(e) => {
                        self.warnings.push(format!("Broken link {}: {e}", path.display()));
                        return;
                    }
                },
            }
        } else {
            lmd
        };
        if !self.filter.accepts(path, &md) {
            return;
        }
        if md.is_dir() {
            self.out.push((path.to_path_buf(), self.entry(path, EntryKind::Dir, &md)));
            let mut children: Vec<PathBuf> = match std::fs::read_dir(path) {
                Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
                Err(e) => {
                    self.warnings.push(format!("Cannot list {}: {e}", path.display()));
                    return;
                }
            };
            children.sort();
            for c in children {
                self.walk(&c, depth + 1);
            }
        } else if md.is_file() {
            if self.hardlink_skip(&md) {
                return;
            }
            self.out.push((path.to_path_buf(), self.entry(path, EntryKind::File, &md)));
        }
    }
}

/// Scanned entries (local path + entry) and warnings.
pub type ScanResult = (Vec<(PathBuf, FileEntry)>, Vec<String>);

/// Enumerate the selected source entries (also used for dry-run previews).
pub fn scan_sources(sources: &[PathBuf], filter: &SourceFilter, opts: &BackupOptions) -> Result<ScanResult, AppError> {
    let mut s = Scanner { filter, opts, seen_inodes: HashSet::new(), out: Vec::new(), warnings: Vec::new() };
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
                    s.warnings.push(format!("Source {} is missing; skipped", src.display()));
                    continue;
                }
            }
        }
        s.walk(src, 0);
    }
    let mut out = s.out;
    if filter.exclude_empty_folders {
        let file_paths: Vec<String> =
            out.iter().filter(|(_, e)| e.kind != EntryKind::Dir).map(|(_, e)| e.path.clone()).collect();
        out.retain(|(p, e)| {
            if e.kind != EntryKind::Dir {
                return true;
            }
            let prefix = format!("{}{}", p.to_string_lossy(), std::path::MAIN_SEPARATOR);
            file_paths.iter().any(|f| f.starts_with(&prefix))
        });
    }
    Ok((out, s.warnings))
}

fn is_no_compress(path: &Path, opts: &BackupOptions) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .map(|e| opts.no_compress_extensions.iter().any(|x| x.eq_ignore_ascii_case(&e)))
        .unwrap_or(false)
}

/// Read, chunk and deduplicate one file into `pending`.
fn process_file(
    repo: &Repo,
    path: &Path,
    entry: &mut FileEntry,
    pending: &mut Pending,
    opts: &BackupOptions,
    report: &mut BackupReport,
    ctx: &OpContext,
) -> Result<(), AppError> {
    let mut f = std::fs::File::open(path)?;
    let bs = repo.header.block_size;
    let mut buf = vec![0u8; bs];
    let mut fh = FileHasher::new(repo.header.file_hash);
    let level = if is_no_compress(path, opts) { 0 } else { opts.compression_level };
    let mut size = 0u64;
    entry.blocks.clear();
    loop {
        ctx.check()?;
        let mut filled = 0;
        while filled < bs {
            let n = f.read(&mut buf[filled..])?;
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
        let h = hash_bytes(repo.header.block_hash, chunk);
        if !pending.hashes.contains(&h) && !repo.db.has_block(&h)? {
            pending.add_block(h.clone(), chunk, level)?;
            report.new_blocks += 1;
            if pending.byte_size() >= opts.volume_size {
                flush(repo, pending, opts, report)?;
            }
        }
        entry.blocks.push(h);
        if filled < bs {
            break;
        }
    }
    entry.size = size;
    entry.hash = Some(fh.finish());
    Ok(())
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
            x.path == y.path && x.kind == y.kind && x.size == y.size && x.mtime == y.mtime && x.hash == y.hash
                && x.symlink_target == y.symlink_target && x.mode == y.mode
        })
}

fn progress(ctx: &OpContext, phase: Phase, done: u64, total: u64, bytes: u64, cur: Option<&str>) {
    ctx.report(&Progress { phase, files_processed: done, files_total: total, bytes_processed: bytes, current_path: cur.map(String::from) });
}

/// Next version timestamp, strictly after the newest existing one.
fn next_timestamp(db: &BackupDb) -> Result<DateTime<Utc>, AppError> {
    let now = Utc::now().with_nanosecond_zero();
    let latest = db
        .filesets()?
        .first()
        .and_then(|f| DateTime::parse_from_rfc3339(&f.timestamp).ok())
        .map(|t| t.with_timezone(&Utc));
    Ok(match latest {
        Some(l) if l >= now => l + chrono::Duration::seconds(1),
        _ => now,
    })
}

trait NoNanos {
    fn with_nanosecond_zero(self) -> Self;
}
impl NoNanos for DateTime<Utc> {
    fn with_nanosecond_zero(self) -> Self {
        use chrono::Timelike;
        self.with_nanosecond(0).unwrap_or(self)
    }
}

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
    let started = Instant::now();
    let mut report = BackupReport { dry_run: opts.dry_run, ..Default::default() };
    retention.validate()?;
    if sources.is_empty() {
        return Err(AppError::validation("The backup job has no source folders"));
    }
    progress(ctx, Phase::Preparing, 0, 0, 0, None);
    let repo = Repo::open(backend, db, passphrase, opts, true, kdf)?;

    let cons = if opts.dry_run && db.volumes(None)?.is_empty() { Default::default() } else { repo.consistency_check()? };
    if !cons.missing.is_empty() {
        return Err(AppError::transfer(
            format!("{} remote volume(s) are missing from the destination", cons.missing.len()),
            "Run Repair (or Recreate database) before the next backup.",
        ));
    }
    for name in &cons.incomplete {
        // Interrupted uploads from an earlier run: forget them, data is re-uploaded.
        if !opts.dry_run {
            let _ = backend.delete(name);
            db.delete_volume(name)?;
        }
    }
    if !cons.extra.is_empty() {
        report.warnings.push(format!(
            "{} remote file(s) are unknown to the local database; run Repair if they belong to this backup",
            cons.extra.len()
        ));
    }

    progress(ctx, Phase::Scanning, 0, 0, 0, None);
    let (mut entries, warnings) = scan_sources(sources, filter, opts)?;
    report.warnings.extend(warnings);
    if entries.iter().all(|(_, e)| e.kind == EntryKind::Dir) && !opts.allow_empty_source {
        return Err(AppError::validation(
            "The source selection contains no files; refusing to create an empty version",
        ));
    }

    let prev_files: HashMap<String, FileEntry> = match db.filesets()?.into_iter().find(|f| !f.partial) {
        Some(fs) => db.files(fs.id)?.into_iter().map(|f| (f.path.clone(), f)).collect(),
        None => HashMap::new(),
    };
    let total = entries.len() as u64;
    let mut pending = Pending::new();
    let mut done: Vec<FileEntry> = Vec::with_capacity(entries.len());
    let mut interrupted: Option<AppError> = None;

    for (i, (path, entry)) in entries.iter_mut().enumerate() {
        if let Err(e) = ctx.check() {
            interrupted = Some(e);
            break;
        }
        progress(ctx, Phase::Uploading, i as u64, total, report.bytes_examined, Some(&entry.path));
        if entry.kind != EntryKind::File {
            done.push(entry.clone());
            continue;
        }
        report.files_examined += 1;
        report.bytes_examined += entry.size;
        match prev_files.get(&entry.path) {
            Some(prev) if unchanged(prev, entry, opts) && prev.blocks.iter().all(|b| db.has_block(b).unwrap_or(false)) => {
                let mut e = prev.clone();
                e.mode = entry.mode;
                done.push(e);
                continue;
            }
            Some(_) => report.files_changed += 1,
            None => report.files_added += 1,
        }
        match process_file(&repo, path, entry, &mut pending, opts, &mut report, ctx) {
            Ok(()) => done.push(entry.clone()),
            Err(e) if ctx.is_cancelled() => {
                interrupted = Some(e);
                break;
            }
            Err(e) => {
                report.warnings.push(format!("Skipped {}: {}", entry.path, e.user_message()));
                if let Some(prev) = prev_files.get(&entry.path) {
                    // Keep the previous good copy in this version.
                    done.push(prev.clone());
                }
            }
        }
    }
    let present: HashSet<&str> = done.iter().map(|e| e.path.as_str()).collect();
    report.files_deleted = prev_files.keys().filter(|p| !present.contains(p.as_str())).count() as u64;

    progress(ctx, Phase::Finalizing, total, total, report.bytes_examined, None);
    flush(&repo, &mut pending, opts, &mut report)?;

    if let Some(err) = interrupted {
        // Synthetic partial version: everything finished so far stays restorable.
        if !done.is_empty() {
            let ts = next_timestamp(db)?;
            report.version = Some(repo.write_filelist(ts, true, &done, opts.task_config.clone(), opts.dry_run)?);
            report.partial = true;
        }
        report.errors.push(err.user_message());
        report.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(report);
    }

    let mut prev_sorted: Vec<FileEntry> = prev_files.into_values().collect();
    prev_sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut cur_sorted = done.clone();
    cur_sorted.sort_by(|a, b| a.path.cmp(&b.path));
    if !prev_sorted.is_empty() && same_version(&prev_sorted, &cur_sorted) && !opts.upload_unchanged_backups {
        report.skipped_unchanged = true;
    } else {
        let ts = next_timestamp(db)?;
        report.version = Some(repo.write_filelist(ts, false, &done, opts.task_config.clone(), opts.dry_run)?);
    }

    if !opts.dry_run {
        progress(ctx, Phase::Retention, 0, 0, 0, None);
        report.versions_deleted = super::maintenance::apply_retention(&repo, retention, opts.allow_full_removal)? as u64;
        if opts.auto_compact {
            progress(ctx, Phase::Compacting, 0, 0, 0, None);
            report.volumes_compacted = super::maintenance::compact(&repo, opts, ctx)?.volumes_removed as u64;
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
    }
    progress(ctx, Phase::Completed, total, total, report.bytes_examined, None);
    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::test_util::write;

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
}
