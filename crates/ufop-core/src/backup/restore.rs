//! Restore: pick a version and paths, download only the dblocks that hold
//! the needed blocks, rebuild files to the original or an alternate
//! location, verify their hashes and carry on past individual failures.

use super::backend::Backend;
use super::engine::Repo;
use super::volume;
use super::{hash_bytes, hash_file, meta, HashAlgorithm, EntryKind, FileEntry, OpContext, Phase, Progress};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Replace the existing file.
    #[default]
    Overwrite,
    /// Keep the existing file and write `name (restored YYYYMMDD-HHMMSS).ext`.
    Rename,
    /// Leave the existing file alone.
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreOptions {
    /// 0 = newest version.
    pub version: usize,
    /// Restore several versions side by side (`target/<timestamp>/…`);
    /// overrides `version` when non-empty. Requires `target`.
    pub versions: Vec<usize>,
    /// With `versions`: skip a file whose content equals what an earlier
    /// (newer) selected version already restored for the same path.
    pub unique_versions: bool,
    /// Paths to restore: exact files, folders (everything below) or
    /// wildcards. Empty = everything.
    pub paths: Vec<String>,
    /// `None` restores to the original locations.
    pub target: Option<PathBuf>,
    /// Keep the full original folder structure below `target` instead of
    /// collapsing to the shortest common parent.
    pub preserve_full_paths: bool,
    pub conflict: ConflictPolicy,
    pub restore_permissions: bool,
    pub restore_symlinks: bool,
    /// Re-apply extended attributes / Windows attributes captured at backup.
    pub restore_metadata: bool,
    /// Skip post-restore hash checks (recovery situations only).
    pub skip_verify: bool,
    pub dry_run: bool,
    /// Read blocks from files already on this machine (original paths)
    /// before downloading, so only missing data is fetched.
    pub use_local_blocks: bool,
    /// Parallel volume downloads (0 = 2).
    pub download_threads: usize,
    /// Parallel file writers (0 = number of CPUs, max 8).
    pub file_threads: usize,
    /// Sequential single-threaded restore (no local-block reuse).
    pub legacy: bool,
    /// Where downloaded blocks are staged (default: system temp).
    pub temp_dir: Option<PathBuf>,
    /// Do not refuse to start when the target or temp folder looks too small.
    pub skip_free_space_check: bool,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            version: 0,
            versions: Vec::new(),
            unique_versions: false,
            paths: Vec::new(),
            target: None,
            preserve_full_paths: false,
            conflict: ConflictPolicy::Overwrite,
            restore_permissions: false,
            restore_symlinks: true,
            restore_metadata: true,
            skip_verify: false,
            dry_run: false,
            use_local_blocks: false,
            download_threads: 0,
            file_threads: 0,
            legacy: false,
            temp_dir: None,
            skip_free_space_check: false,
        }
    }
}

impl RestoreOptions {
    fn download_workers(&self) -> usize {
        if self.legacy { 1 } else if self.download_threads == 0 { 2 } else { self.download_threads.min(16) }
    }
    fn file_workers(&self) -> usize {
        if self.legacy {
            1
        } else if self.file_threads == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(8)
        } else {
            self.file_threads.min(32)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreReport {
    pub version: String,
    pub files_restored: u64,
    pub folders_created: u64,
    pub files_skipped_matching: u64,
    pub files_skipped_conflict: u64,
    /// Identical to an already restored newer version (`unique_versions`).
    pub files_skipped_duplicate: u64,
    pub bytes_restored: u64,
    pub volumes_downloaded: u64,
    /// Blocks taken from local files instead of the destination.
    pub blocks_from_local: u64,
    /// Planned destination paths (dry run).
    pub planned: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub(crate) fn path_selected(path: &str, selectors: &[String]) -> bool {
    if selectors.is_empty() {
        return true;
    }
    let norm = |s: &str| s.replace('\\', "/");
    let p = norm(path);
    selectors.iter().any(|s| {
        let s = norm(s);
        if s.contains('*') || s.contains('?') {
            glob::Pattern::new(&s).map(|g| g.matches(&p)).unwrap_or(false)
        } else {
            let t = s.trim_end_matches('/');
            p == t || p.starts_with(&format!("{t}/"))
        }
    })
}

/// Path without its root/prefix (`/a/b` → `a/b`, `C:\a` → `a`).
fn relative_of(p: &Path) -> PathBuf {
    p.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect()
}

fn common_parent(paths: &[&Path]) -> PathBuf {
    let mut it = paths.iter();
    let Some(first) = it.next() else { return PathBuf::new() };
    let mut common: Vec<Component> = first.parent().map(|p| p.components().collect()).unwrap_or_default();
    for p in it {
        let comps: Vec<Component> = p.parent().map(|p| p.components().collect()).unwrap_or_default();
        let n = common.iter().zip(&comps).take_while(|(a, b)| a == b).count();
        common.truncate(n);
    }
    common.iter().collect()
}

fn rename_target(dest: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match dest.extension() {
        Some(e) => format!("{stem} (restored {stamp}).{}", e.to_string_lossy()),
        None => format!("{stem} (restored {stamp})"),
    };
    dest.with_file_name(name)
}

/// Compute destination paths for the selected entries.
pub(crate) fn destinations(entries: &[FileEntry], opts: &RestoreOptions) -> Vec<PathBuf> {
    let paths: Vec<&Path> = entries.iter().map(|e| Path::new(&e.path)).collect();
    match &opts.target {
        None => paths.iter().map(|p| p.to_path_buf()).collect(),
        Some(t) if opts.preserve_full_paths => paths.iter().map(|p| t.join(relative_of(p))).collect(),
        Some(t) => {
            let base = common_parent(&paths.iter().copied().filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>());
            paths
                .iter()
                .map(|p| match p.strip_prefix(&base) {
                    Ok(r) if !r.as_os_str().is_empty() => t.join(r),
                    _ => t.join(p.file_name().unwrap_or_default()),
                })
                .collect()
        }
    }
}

fn apply_metadata(path: &Path, e: &FileEntry, opts: &RestoreOptions) -> Vec<String> {
    let mut warn = Vec::new();
    if opts.restore_metadata {
        if let Some(m) = &e.meta {
            warn = meta::apply(path, m, opts.restore_permissions);
        }
    }
    if let Some(m) = e.mtime {
        if let Ok(f) = std::fs::File::options().write(true).open(path) {
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(m.max(0) as u64);
            let _ = f.set_modified(t);
        }
    }
    #[cfg(unix)]
    if opts.restore_permissions {
        if let Some(mode) = e.mode {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777));
        }
    }
    warn
}

/// One planned restore action.
struct Job<'e> {
    entry: &'e FileEntry,
    dest: PathBuf,
}

/// Windows alternate data stream entry (`C:\dir\file.txt:stream`).
fn is_stream_entry(path: &str) -> bool {
    super::usn::volume_root(path).is_some() && path.rsplit('\\').next().is_some_and(|last| last.contains(':'))
}

/// Restore from an opened repository.
pub fn run_restore(repo: &Repo, opts: &RestoreOptions, ctx: &OpContext) -> Result<RestoreReport, AppError> {
    let mut report = RestoreReport::default();
    // Versions to restore and where each one goes.
    let multi = !opts.versions.is_empty();
    if multi && opts.target.is_none() {
        return Err(AppError::validation("Restoring several versions needs a target folder"));
    }
    let mut picks = opts.versions.clone();
    if !multi {
        picks.push(opts.version);
    }
    picks.sort_unstable();
    picks.dedup();
    let mut sets: Vec<(String, Vec<FileEntry>, RestoreOptions)> = Vec::new();
    for v in picks {
        let fs = repo.db.fileset_by_version(v)?;
        let entries: Vec<FileEntry> = repo.db.files(fs.id)?.into_iter().filter(|e| path_selected(&e.path, &opts.paths)).collect();
        let mut o = opts.clone();
        if multi {
            let stamp = fs.timestamp.replace([':', '+'], "-");
            o.target = opts.target.as_ref().map(|t| t.join(stamp));
        }
        sets.push((fs.timestamp, entries, o));
    }
    report.version = sets.iter().map(|s| s.0.clone()).collect::<Vec<_>>().join(", ");
    if sets.iter().all(|s| s.1.is_empty()) {
        return Err(AppError::validation("No backed-up files match the selected paths in this version"));
    }

    // Plan: decide per entry what happens.
    let mut jobs: Vec<Job> = Vec::new();
    let mut seen_content: HashMap<&str, &str> = HashMap::new();
    for (_, entries, o) in &sets {
        for (e, d) in entries.iter().zip(destinations(entries, o)) {
            if multi && opts.unique_versions && e.kind == EntryKind::File {
                if let Some(h) = e.hash.as_deref() {
                    if seen_content.get(e.path.as_str()) == Some(&h) {
                        report.files_skipped_duplicate += 1;
                        continue;
                    }
                    seen_content.insert(&e.path, h);
                }
            }
            if e.kind == EntryKind::File && d.exists() {
                let matches = e.hash.as_deref().is_some_and(|h| hash_file(repo.header.file_hash, &d).ok().as_deref() == Some(h));
                if matches {
                    report.files_skipped_matching += 1;
                    continue;
                }
                match opts.conflict {
                    ConflictPolicy::Skip => {
                        report.files_skipped_conflict += 1;
                        continue;
                    }
                    ConflictPolicy::Rename => {
                        jobs.push(Job { entry: e, dest: rename_target(&d) });
                        continue;
                    }
                    ConflictPolicy::Overwrite => {}
                }
            }
            jobs.push(Job { entry: e, dest: d });
        }
    }
    if opts.dry_run {
        report.planned = jobs.iter().map(|j| j.dest.to_string_lossy().into_owned()).collect();
        return Ok(report);
    }

    let restore_bytes: u64 = jobs.iter().filter(|j| j.entry.kind == EntryKind::File).map(|j| j.entry.size).sum();
    let spill = tempfile_dir(opts.temp_dir.as_deref())?;
    if !opts.skip_free_space_check {
        check_space(jobs.first().map(|j| j.dest.as_path()), restore_bytes, "restore target")?;
        check_space(Some(spill.path()), restore_bytes, "temporary folder")?;
    }

    let locations = repo.db.block_locations()?;
    let needed: HashSet<&str> = jobs
        .iter()
        .filter(|j| j.entry.kind == EntryKind::File)
        .flat_map(|j| j.entry.blocks.iter().map(String::as_str))
        .collect();
    let mut have: HashSet<String> = HashSet::new();
    if opts.use_local_blocks && !opts.legacy {
        report.blocks_from_local = harvest_local_blocks(repo, &jobs, &needed, spill.path(), &mut have, ctx)?;
    }
    let mut by_volume: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut missing_blocks: HashSet<String> = HashSet::new();
    for h in needed.iter().filter(|h| !have.contains(**h)) {
        match locations.get(*h) {
            Some(v) => by_volume.entry(v.as_str()).or_default().push(h),
            None => {
                missing_blocks.insert(h.to_string());
            }
        }
    }

    // Download needed volumes in parallel and spill their blocks.
    let mut work: Vec<(&str, Vec<&str>, Option<String>, Vec<String>)> = Vec::new();
    for (vol, hashes) in by_volume {
        let (hash, rec) = repo.volume_check_info(vol)?;
        work.push((vol, hashes, hash, rec));
    }
    let fetcher = repo.fetcher();
    let (cipher, fetcher) = (&repo.cipher, &fetcher);
    let queue = Mutex::new(work);
    let total_vol = queue.lock().map(|q| q.len()).unwrap_or(0) as u64;
    let done = AtomicU64::new(0);
    let shared = Mutex::new((missing_blocks, Vec::<String>::new()));
    std::thread::scope(|sc| {
        for _ in 0..opts.download_workers() {
            sc.spawn(|| loop {
                if ctx.is_cancelled() {
                    return;
                }
                let Some((vol, hashes, expected, rec)) = queue.lock().ok().and_then(|mut q| q.pop()) else { return };
                let n = done.fetch_add(1, Ordering::SeqCst);
                ctx.report(&Progress { phase: Phase::Downloading, files_processed: n, files_total: total_vol, bytes_processed: 0, current_path: Some(vol.to_string()) });
                let res = fetcher.fetch(vol, expected.as_deref(), &rec, true).and_then(|d| cipher.open(&d)).and_then(|p| volume::read_block_volume(&p)).and_then(|mut map| {
                    let mut absent = Vec::new();
                    for h in &hashes {
                        match map.remove(*h) {
                            Some(data) => std::fs::write(spill.path().join(h), data)?,
                            None => absent.push(h.to_string()),
                        }
                    }
                    Ok(absent)
                });
                let mut g = match shared.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                match res {
                    Ok(absent) => g.0.extend(absent),
                    Err(e) => {
                        g.1.push(format!("Volume {vol}: {}", e.user_message()));
                        g.0.extend(hashes.iter().map(|h| h.to_string()));
                    }
                }
            });
        }
    });
    ctx.check()?;
    let (missing_blocks, vol_errors) = shared.into_inner().unwrap_or_else(|p| p.into_inner());
    report.volumes_downloaded = total_vol - vol_errors.len() as u64;
    report.errors.extend(vol_errors);

    // Folders first (sequential), then files and links in parallel, then
    // alternate streams (their base file must exist), then folder times.
    for j in jobs.iter().filter(|j| j.entry.kind == EntryKind::Dir) {
        match std::fs::create_dir_all(&j.dest) {
            Ok(()) => report.folders_created += 1,
            Err(e) => report.errors.push(format!("{}: {}", j.entry.path, AppError::from(e).user_message())),
        }
    }
    let (streams, plain): (Vec<&Job>, Vec<&Job>) = jobs.iter().filter(|j| j.entry.kind != EntryKind::Dir).partition(|j| is_stream_entry(&j.entry.path));
    let total = jobs.len() as u64;
    let processed = AtomicU64::new(report.folders_created);
    let out = Mutex::new(&mut report);
    let file_hash = repo.header.file_hash;
    let run = |list: &[&Job], workers: usize| {
        let next = AtomicU64::new(0);
        std::thread::scope(|sc| {
            for _ in 0..workers.max(1) {
                sc.spawn(|| loop {
                    if ctx.is_cancelled() {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst) as usize;
                    let Some(j) = list.get(i) else { return };
                    let n = processed.fetch_add(1, Ordering::SeqCst);
                    ctx.report(&Progress { phase: Phase::Restoring, files_processed: n, files_total: total, bytes_processed: 0, current_path: Some(j.entry.path.clone()) });
                    let mut warns = Vec::new();
                    let r = match j.entry.kind {
                        EntryKind::Symlink => restore_symlink(j.entry, &j.dest, opts, &mut warns).map(|_| None),
                        _ => match j.entry.blocks.iter().find(|b| missing_blocks.contains(b.as_str())) {
                            Some(b) => Err(AppError::transfer(format!("data block {} is unavailable", &b[..12.min(b.len())]), "Run list-broken-files.")),
                            None => write_file(file_hash, j.entry, &j.dest, spill.path(), opts, is_stream_entry(&j.entry.path), &mut warns).map(Some),
                        },
                    };
                    let mut g = match out.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    g.warnings.extend(warns);
                    match r {
                        Ok(Some(bytes)) => {
                            g.files_restored += 1;
                            g.bytes_restored += bytes;
                        }
                        Ok(None) => {}
                        Err(e) => g.errors.push(format!("{}: {}", j.entry.path, e.user_message())),
                    }
                });
            }
        });
    };
    run(&plain, opts.file_workers());
    run(&streams, 1);
    ctx.check()?;
    // Directory times/metadata last, deepest first, since writing children changes them.
    let mut dirs: Vec<&Job> = jobs.iter().filter(|j| j.entry.kind == EntryKind::Dir).collect();
    dirs.sort_by_key(|j| std::cmp::Reverse(j.dest.components().count()));
    for j in dirs {
        report.warnings.extend(apply_metadata(&j.dest, j.entry, opts));
    }
    ctx.report(&Progress { phase: Phase::Completed, files_processed: total, files_total: total, bytes_processed: report.bytes_restored, current_path: None });
    Ok(report)
}

/// Result of [`autotune`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoTune {
    pub download_threads: usize,
    pub file_threads: usize,
    /// `(threads, MB/s)` per measured setting.
    pub measurements: Vec<(usize, f64)>,
    pub sample_bytes: u64,
}

/// Benchmark volume downloads with 1, 2, 4 and 8 parallel readers on a
/// sample of block volumes and pick the smallest setting within 10% of
/// the fastest. Nothing is written.
pub fn autotune(repo: &Repo, sample_volumes: usize, ctx: &OpContext) -> Result<AutoTune, AppError> {
    // Only volumes that are actually at the destination (missing ones are a repair matter).
    let present: HashSet<String> = repo.backend.list()?.into_iter().map(|f| f.name).collect();
    let mut vols: Vec<String> = repo.db.volumes(Some(volume::VolumeKind::Block))?.into_iter().map(|v| v.name).filter(|n| present.contains(n)).collect();
    if vols.is_empty() {
        return Err(AppError::validation("Run a backup first: there are no volumes to measure"));
    }
    vols.truncate(sample_volumes.max(1));
    let mut info = Vec::new();
    for v in &vols {
        info.push((v.clone(), repo.volume_check_info(v)?));
    }
    let fetcher = repo.fetcher();
    let cipher = &repo.cipher;
    let mut t = AutoTune { file_threads: RestoreOptions::default().file_workers(), ..Default::default() };
    for threads in [1usize, 2, 4, 8] {
        ctx.check()?;
        let next = AtomicU64::new(0);
        let bytes = AtomicU64::new(0);
        let failed = Mutex::new(None::<AppError>);
        let start = std::time::Instant::now();
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::SeqCst) as usize;
                    let Some((name, (hash, rec))) = info.get(i) else { return };
                    match fetcher.fetch(name, hash.as_deref(), rec, false).and_then(|d| cipher.open(&d).map(|p| (d.len(), p))).and_then(|(n, p)| volume::read_block_volume(&p).map(|_| n)) {
                        Ok(n) => {
                            bytes.fetch_add(n as u64, Ordering::SeqCst);
                        }
                        Err(e) => {
                            if let Ok(mut g) = failed.lock() {
                                *g = Some(e);
                            }
                            return;
                        }
                    }
                });
            }
        });
        if let Some(e) = failed.into_inner().unwrap_or_else(|p| p.into_inner()) {
            return Err(e);
        }
        let b = bytes.load(Ordering::SeqCst);
        t.sample_bytes = b;
        let secs = start.elapsed().as_secs_f64().max(1e-6);
        t.measurements.push((threads, b as f64 / 1_048_576.0 / secs));
    }
    let best = t.measurements.iter().map(|m| m.1).fold(0.0, f64::max);
    t.download_threads = t.measurements.iter().find(|m| m.1 >= best * 0.9).map(|m| m.0).unwrap_or(2);
    Ok(t)
}

/// Refuse to start when `path`'s volume has less than `need` bytes free.
fn check_space(path: Option<&Path>, need: u64, what: &str) -> Result<(), AppError> {
    let Some(free) = path.and_then(meta::free_space) else { return Ok(()) };
    if free < need {
        return Err(AppError::file_op(
            format!("Not enough free space in the {what}: {need} bytes needed, {free} available"),
            "Free some space, pick another folder, or enable 'skip free space check'.",
        ));
    }
    Ok(())
}

/// Hash files already present at the original paths (and overwrite
/// targets) and stage any block the restore needs. Returns blocks found.
fn harvest_local_blocks(repo: &Repo, jobs: &[Job], needed: &HashSet<&str>, spill: &Path, have: &mut HashSet<String>, ctx: &OpContext) -> Result<u64, AppError> {
    let chunker = repo.chunker();
    let mut candidates: Vec<&Path> = Vec::new();
    for j in jobs.iter().filter(|j| j.entry.kind == EntryKind::File) {
        candidates.push(Path::new(&j.entry.path));
        candidates.push(&j.dest);
    }
    candidates.sort();
    candidates.dedup();
    let mut found = 0u64;
    for p in candidates {
        ctx.check()?;
        if have.len() >= needed.len() {
            break;
        }
        let Ok(f) = std::fs::File::open(p) else { continue };
        let mut f = std::io::BufReader::new(f);
        // Cut with the repository's chunker (fixed or content-defined) so
        // blocks line up with the backed-up ones.
        let mut write_err = None;
        let _ = super::chunker::for_each_chunk(&mut f, chunker.as_ref(), |chunk| {
            let h = hash_bytes(repo.header.block_hash, chunk);
            if needed.contains(h.as_str()) && !have.contains(&h) {
                if let Err(e) = std::fs::write(spill.join(&h), chunk) {
                    write_err = Some(e);
                    return Ok(false);
                }
                have.insert(h);
                found += 1;
            }
            Ok(true)
        });
        if let Some(e) = write_err {
            return Err(e.into());
        }
    }
    Ok(found)
}

fn tempfile_dir(base: Option<&Path>) -> Result<TempDir, AppError> {
    let p = base.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join(format!("ufop-restore-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&p)?;
    Ok(TempDir(p))
}

struct TempDir(PathBuf);
impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_file(file_hash: HashAlgorithm, e: &FileEntry, dest: &Path, spill: &Path, opts: &RestoreOptions, stream: bool, warns: &mut Vec<String>) -> Result<u64, AppError> {
    if stream && !cfg!(windows) {
        warns.push(format!("Alternate data stream {} skipped (NTFS only)", e.path));
        return Ok(0);
    }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    // Streams are written in place: a temp name + rename would detach them.
    let tmp = if stream { dest.to_path_buf() } else { dest.with_file_name(format!(".{}.ufop-restore", dest.file_name().unwrap_or_default().to_string_lossy())) };
    let mut n = 0u64;
    {
        let mut f = std::fs::File::create(&tmp)?;
        // Preallocate to the final size.
        f.set_len(e.size)?;
        for b in &e.blocks {
            let d = std::fs::read(spill.join(b))?;
            n += d.len() as u64;
            f.write_all(&d)?;
        }
        f.set_len(n)?;
        f.sync_all()?;
    }
    if !opts.skip_verify {
        if let Some(h) = &e.hash {
            if &hash_file(file_hash, &tmp)? != h {
                if !stream {
                    let _ = std::fs::remove_file(&tmp);
                }
                return Err(AppError::transfer("restored content failed hash verification", "The backup data may be damaged; run Verify."));
            }
        }
    }
    if !stream {
        std::fs::rename(&tmp, dest)?;
    }
    warns.extend(apply_metadata(dest, e, opts));
    Ok(n)
}

#[allow(unused_variables)]
fn restore_symlink(e: &FileEntry, dest: &Path, opts: &RestoreOptions, warns: &mut Vec<String>) -> Result<(), AppError> {
    if !opts.restore_symlinks {
        return Ok(());
    }
    let target = e.symlink_target.as_deref().unwrap_or_default();
    #[cfg(unix)]
    {
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        if std::fs::symlink_metadata(dest).is_ok() {
            std::fs::remove_file(dest)?;
        }
        std::os::unix::fs::symlink(target, dest)?;
        // The link's own timestamp (not its target's).
        if let Some(m) = e.mtime {
            if let Err(err) = meta::set_symlink_mtime(dest, m) {
                warns.push(format!("{}: link time not restored ({err})", dest.display()));
            }
        }
        return Ok(());
    }
    #[cfg(windows)]
    {
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        if std::fs::symlink_metadata(dest).is_ok() {
            let _ = std::fs::remove_dir(dest).or_else(|_| std::fs::remove_file(dest));
        }
        if e.meta.as_ref().is_some_and(|m| m.junction) {
            // Junctions need no special privilege.
            return meta::create_junction(dest, target).map_err(|err| AppError::file_op(format!("Junction {}: {err}", dest.display()), "Check the target folder."));
        }
        let looks_dir = target.ends_with('\\') || Path::new(target).is_dir();
        let r = if looks_dir { std::os::windows::fs::symlink_dir(target, dest) } else { std::os::windows::fs::symlink_file(target, dest) };
        if let Err(err) = r {
            warns.push(format!("Symbolic link {} → {target} not recreated: {err} (needs Developer Mode or administrator rights)", dest.display()));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        if e.meta.as_ref().is_some_and(|m| m.junction) {
            warns.push(format!("Junction {} → {target} not recreated (NTFS only)", dest.display()));
        } else {
            warns.push(format!("Symbolic link {} → {target} not recreated on this platform", dest.display()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::db::BackupDb;
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, BackupOptions, RetentionPolicy, SourceFilter};

    fn backup(src: &Path, be: &LocalBackend, db: &BackupDb) {
        let o = BackupOptions { block_size: 1024, volume_size: 3000, ..Default::default() };
        let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"),
            KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    fn open<'a>(be: &'a LocalBackend, db: &'a BackupDb) -> Repo<'a> {
        Repo::open(be, db, Some("pw"), &BackupOptions::default(), false, KdfParams::insecure_fast()).unwrap()
    }

    #[test]
    fn restore_versions_to_alternate_location() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..10_000u32).map(|i| (i * 7 % 251) as u8).collect();
        write(src.path(), "docs/a.txt", b"version one");
        write(src.path(), "docs/big.bin", &big);
        std::fs::create_dir_all(src.path().join("empty")).unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(src.path(), "docs/a.txt", b"version two!");
        backup(src.path(), &be, &db);
        assert_eq!(db.filesets().unwrap().len(), 2);

        let repo = open(&be, &db);
        // Older version, collapsed paths.
        let r = run_restore(&repo, &RestoreOptions { version: 1, target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let root = out.path().join(src.path().file_name().unwrap());
        assert_eq!(std::fs::read(root.join("docs/a.txt")).unwrap(), b"version one");
        assert_eq!(std::fs::read(root.join("docs/big.bin")).unwrap(), big);
        assert!(root.join("empty").is_dir());

        // Same again: identical files are skipped.
        let r = run_restore(&repo, &RestoreOptions { version: 1, target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.files_restored, 0);
        assert_eq!(r.files_skipped_matching, 2);

        // Newest version with rename conflict policy keeps the old file.
        let sel = format!("{}/docs/a.txt", src.path().display());
        let r = run_restore(&repo, &RestoreOptions { paths: vec![sel], target: Some(out.path().join("x")), conflict: ConflictPolicy::Rename, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.files_restored, 1);
        assert_eq!(std::fs::read(out.path().join("x/a.txt")).unwrap(), b"version two!");

        // Full-path preservation.
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().join("full")), preserve_full_paths: true, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty());
        assert!(out.path().join("full").join(relative_of(&src.path().join("docs/a.txt"))).is_file());
    }

    #[test]
    fn best_effort_when_volume_damaged() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(src.path(), "a", &vec![1u8; 5000]);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        // Damage every block volume.
        for f in std::fs::read_dir(dst.path()).unwrap() {
            let p = f.unwrap().path();
            if p.to_string_lossy().contains(".dblock") {
                std::fs::write(&p, b"garbage").unwrap();
            }
        }
        write(src.path(), "b", b"x"); // unrelated
        let repo = open(&be, &db);
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(!r.errors.is_empty());
        assert_eq!(r.files_restored, 0);
    }

    #[test]
    fn selection_and_dry_run() {
        assert!(path_selected("/a/b/c.txt", &["/a/b".into()]));
        assert!(!path_selected("/a/bc", &["/a/b".into()]));
        assert!(path_selected("/a/b/c.txt", &["*.txt".into()]));
        assert!(path_selected("x", &[]));
    }

    #[test]
    fn multi_version_unique_restore() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(src.path(), "a.txt", b"v1");
        write(src.path(), "same.txt", b"constant");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(src.path(), "a.txt", b"v2");
        backup(src.path(), &be, &db);
        let repo = open(&be, &db);
        assert!(run_restore(&repo, &RestoreOptions { versions: vec![0, 1], ..Default::default() }, &OpContext::new()).is_err(), "needs target");
        let r = run_restore(&repo, &RestoreOptions { versions: vec![0, 1], unique_versions: true, target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.files_restored, 3, "a.txt twice, same.txt once");
        assert_eq!(r.files_skipped_duplicate, 1);
        let dirs: Vec<PathBuf> = std::fs::read_dir(out.path()).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(dirs.len(), 2);
        let mut contents: Vec<Vec<u8>> = dirs.iter().map(|d| std::fs::read(d.join(src.path().file_name().unwrap()).join("a.txt")).unwrap()).collect();
        contents.sort();
        assert_eq!(contents, vec![b"v1".to_vec(), b"v2".to_vec()]);
    }

    #[test]
    fn local_blocks_avoid_downloads() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..6000u32).map(|i| (i % 97) as u8).collect();
        write(src.path(), "big.bin", &big);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        // Destination volumes gone: only the local original can supply data.
        for f in std::fs::read_dir(dst.path()).unwrap().flatten() {
            if f.file_name().to_string_lossy().contains(".dblock") {
                std::fs::remove_file(f.path()).unwrap();
            }
        }
        let repo = open(&be, &db);
        let o = RestoreOptions { target: Some(out.path().into()), use_local_blocks: true, temp_dir: Some(out.path().join("tmp")), ..Default::default() };
        std::fs::create_dir_all(out.path().join("tmp")).unwrap();
        let r = run_restore(&repo, &o, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.volumes_downloaded, 0);
        // Every block came from the local file (count depends on the chunker).
        assert!(r.blocks_from_local >= 1);
        assert_eq!(std::fs::read(out.path().join(src.path().file_name().unwrap()).join("big.bin")).unwrap(), big);
        // The staging folder is cleaned up.
        assert_eq!(std::fs::read_dir(out.path().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn legacy_and_parallel_restore_agree() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for i in 0..30 {
            write(src.path(), &format!("d{}/f{i}", i % 4), &vec![i as u8; 900 + i * 50]);
        }
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        let repo = open(&be, &db);
        for o in [
            RestoreOptions { legacy: true, ..Default::default() },
            RestoreOptions { download_threads: 4, file_threads: 6, ..Default::default() },
        ] {
            let out = tempfile::tempdir().unwrap();
            let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..o }, &OpContext::new()).unwrap();
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            assert_eq!(r.files_restored, 30);
            for i in 0..30 {
                assert_eq!(std::fs::read(out.path().join(src.path().file_name().unwrap()).join(format!("d{}/f{i}", i % 4))).unwrap(), vec![i as u8; 900 + i * 50]);
            }
        }
    }

    #[test]
    fn stream_entries_detected() {
        assert!(is_stream_entry(r"C:\d\file.txt:Zone.Identifier"));
        assert!(!is_stream_entry(r"C:\d\file.txt"));
        assert!(!is_stream_entry("/home/a:b"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_time_restored() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(src.path(), "t.txt", b"x");
        std::os::unix::fs::symlink("t.txt", src.path().join("link")).unwrap();
        crate::backup::meta::set_symlink_mtime(&src.path().join("link"), 1_000_000_000).unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        let r = run_restore(&open(&be, &db), &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let md = std::fs::symlink_metadata(out.path().join(src.path().file_name().unwrap()).join("link")).unwrap();
        assert!(md.file_type().is_symlink());
        use std::os::unix::fs::MetadataExt;
        assert_eq!(md.mtime(), 1_000_000_000);
    }

    #[test]
    fn autotune_measures_and_picks() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for i in 0..6 {
            write(src.path(), &format!("f{i}"), &vec![i as u8 + 1; 2500]);
        }
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        let t = autotune(&open(&be, &db), 4, &OpContext::new()).unwrap();
        assert_eq!(t.measurements.len(), 4);
        assert!([1, 2, 4, 8].contains(&t.download_threads));
        assert!(t.sample_bytes > 0 && t.file_threads >= 1);
        let empty = BackupDb::open_in_memory().unwrap();
        let d2 = tempfile::tempdir().unwrap();
        let be2 = LocalBackend::new(d2.path());
        let r = Repo::open(&be2, &empty, None, &BackupOptions::default(), true, KdfParams::insecure_fast()).unwrap();
        assert!(autotune(&r, 2, &OpContext::new()).is_err());
    }
}
