//! Restore: pick a version and paths, download only the dblocks that hold
//! the needed blocks, rebuild files to the original or an alternate
//! location, verify their hashes and carry on past individual failures.

use super::archive::ArchiveTarget;
use super::backend::Backend;
use super::engine::Repo;
use super::errors::{ErrorClass, ErrorTracker, RestoreErrorPolicy};
use super::owner::{OwnerPolicy, OwnerRestore};
use super::select::VersionSelector;
use super::sparse::{SparseMode, SparseWriter};
use super::volume;
use super::{hash_bytes, hash_file, meta, HashAlgorithm, EntryKind, FileEntry, OpContext, Phase, Progress};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// Shared repositories: restore from this source (id, or `*` for every
    /// source's versions numbered together) instead of the job's own.
    #[serde(default)]
    pub source: Option<String>,
    /// Pick the version by time, newest/oldest or id instead of `version`.
    #[serde(default)]
    pub selector: Option<VersionSelector>,
    /// Write a ZIP / TAR archive instead of files (`target` is ignored).
    #[serde(default)]
    pub archive: Option<ArchiveTarget>,
    /// Mirror: delete what the restored folders in `target` hold but the
    /// version does not. Needs `target`; run a dry run first to see the list.
    #[serde(default)]
    pub delete_extraneous: bool,
    #[serde(default)]
    pub restore_owner: OwnerPolicy,
    #[serde(default)]
    pub error_policy: RestoreErrorPolicy,
    #[serde(default)]
    pub sparse: SparseMode,
    /// Read volumes from (and keep them in) the local volume cache. Off for
    /// ordinary restores, which read each volume once.
    #[serde(default)]
    pub use_cache: bool,
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
            source: None,
            selector: None,
            archive: None,
            delete_extraneous: false,
            restore_owner: OwnerPolicy::Auto,
            error_policy: RestoreErrorPolicy::default(),
            sparse: SparseMode::Auto,
            use_cache: false,
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
#[serde(default)]
pub struct RestoreReport {
    pub version: String,
    /// The restored version's number (0 = newest) when one version was restored.
    pub version_index: Option<usize>,
    /// The archive written (archive restores).
    pub archive: Option<String>,
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
    /// Mirror restore: paths that are (dry run: would be) deleted; folders end in `/`.
    pub planned_deletions: Vec<String>,
    pub files_deleted: u64,
    pub folders_deleted: u64,
    pub owners_restored: u64,
    /// Files written with holes for their zero runs.
    pub sparse_files: u64,
    /// Why the restore stopped early (fail-fast, error budget).
    pub stop_reason: Option<String>,
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

/// Shared state of one restore's writers: error policy, owners, sparse files.
struct Writers<'o> {
    errors: ErrorTracker,
    owner: OwnerRestore<'o>,
    sparse_files: AtomicU64,
}

impl Writers<'_> {
    fn stop(&self, ctx: &OpContext) -> bool {
        ctx.is_cancelled() || self.errors.should_stop()
    }
    /// Metadata problems (attributes, owner) go through the policy.
    fn metadata(&self, warns: Vec<String>) {
        for w in warns {
            self.errors.record(ErrorClass::Metadata, w);
        }
    }
}

fn apply_metadata(path: &Path, e: &FileEntry, opts: &RestoreOptions, w: &Writers) {
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
    // Owner before mode: changing the owner clears set-user-id bits.
    if let Err(msg) = w.owner.apply(path, e.meta.as_ref()) {
        warn.push(msg);
    }
    #[cfg(unix)]
    if opts.restore_permissions {
        if let Some(mode) = e.mode {
            use std::os::unix::fs::PermissionsExt;
            if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777)) {
                warn.push(format!("{}: permissions not restored ({err})", path.display()));
            }
        }
    }
    w.metadata(warn);
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

/// The version number a restore uses: the selector when given, else `version`.
pub fn resolve_version(repo: &Repo, opts: &RestoreOptions) -> Result<usize, AppError> {
    match &opts.selector {
        Some(s) => s.resolve(&repo.db.filesets()?),
        None => Ok(opts.version),
    }
}

/// Restore from an opened repository.
pub fn run_restore(repo: &Repo, opts: &RestoreOptions, ctx: &OpContext) -> Result<RestoreReport, AppError> {
    let owner = OwnerRestore::system(if opts.dry_run && opts.restore_owner == OwnerPolicy::Always { OwnerPolicy::Auto } else { opts.restore_owner })?;
    run_restore_with(repo, opts, ctx, owner)
}

/// [`run_restore`] with an owner restorer (tests inject the privilege check and chown).
pub(crate) fn run_restore_with(repo: &Repo, opts: &RestoreOptions, ctx: &OpContext, owner: OwnerRestore<'_>) -> Result<RestoreReport, AppError> {
    let mut report = RestoreReport::default();
    let _scope = super::shared::selector_scope(opts.source.as_deref()).map(|s| repo.db.scoped(s));
    // Versions to restore and where each one goes.
    let multi = !opts.versions.is_empty();
    if multi && opts.target.is_none() && opts.archive.is_none() {
        return Err(AppError::validation("Restoring several versions needs a target folder"));
    }
    if multi && (opts.archive.is_some() || opts.delete_extraneous) {
        return Err(AppError::validation("Archive and mirror restores take one version"));
    }
    if opts.delete_extraneous && (opts.archive.is_some() || opts.target.is_none()) {
        return Err(AppError::file_op("A mirror restore needs a target folder", "Choose a folder to restore into; mirroring deletes files there that the backup does not have."));
    }
    let mut picks = opts.versions.clone();
    if !multi {
        let v = resolve_version(repo, opts)?;
        report.version_index = Some(v);
        picks.push(v);
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
    let w = Writers { errors: ErrorTracker::for_restore(opts.error_policy), owner, sparse_files: AtomicU64::new(0) };
    if let Some(target) = &opts.archive {
        restore_archive(repo, &sets[0].1, opts, target, ctx, &w, &mut report)?;
        return Ok(finish(report, w));
    }

    // Plan: decide per entry what happens.
    let mut jobs: Vec<Job> = Vec::new();
    let mut expected: Vec<PathBuf> = Vec::new();
    let mut seen_content: HashMap<&str, &str> = HashMap::new();
    for (_, entries, o) in &sets {
        for (e, d) in entries.iter().zip(destinations(entries, o)) {
            if opts.delete_extraneous {
                expected.push(d.clone());
            }
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
                        let r = rename_target(&d);
                        expected.push(r.clone());
                        jobs.push(Job { entry: e, dest: r });
                        continue;
                    }
                    ConflictPolicy::Overwrite => {}
                }
            }
            jobs.push(Job { entry: e, dest: d });
        }
    }
    let mirror_plan = match (opts.delete_extraneous, opts.target.as_deref()) {
        (true, Some(t)) => {
            let scopes: Vec<PathBuf> = sets[0].1.iter().zip(destinations(&sets[0].1, &sets[0].2)).filter(|(e, _)| e.kind == EntryKind::Dir).map(|(_, d)| d).collect();
            let plan = super::restore_mirror::plan(t, &scopes, &expected);
            report.planned_deletions = plan.iter().map(|x| format!("{}{}", x.path.display(), if x.is_dir { "/" } else { "" })).collect();
            Some((t.to_path_buf(), plan))
        }
        _ => None,
    };
    if opts.dry_run {
        report.planned = jobs.iter().map(|j| j.dest.to_string_lossy().into_owned()).collect();
        return Ok(finish(report, w));
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
    let fetcher = repo.fetcher_with(opts.use_cache);
    let (cipher, fetcher) = (&repo.cipher, &fetcher);
    let queue = Mutex::new(work);
    let total_vol = queue.lock().map(|q| q.len()).unwrap_or(0) as u64;
    let done = AtomicU64::new(0);
    let failed_vols = AtomicU64::new(0);
    let missing = Mutex::new(missing_blocks);
    let wr = &w;
    std::thread::scope(|sc| {
        for _ in 0..opts.download_workers() {
            sc.spawn(|| loop {
                if wr.stop(ctx) {
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
                let mut g = missing.lock().unwrap_or_else(|p| p.into_inner());
                match res {
                    Ok(absent) => g.extend(absent),
                    Err(e) => {
                        failed_vols.fetch_add(1, Ordering::SeqCst);
                        wr.errors.record(ErrorClass::MissingData, format!("Volume {vol}: {}", e.user_message()));
                        g.extend(hashes.iter().map(|h| h.to_string()));
                    }
                }
            });
        }
    });
    ctx.check()?;
    let missing_blocks = missing.into_inner().unwrap_or_else(|p| p.into_inner());
    report.volumes_downloaded = done.load(Ordering::SeqCst).min(total_vol) - failed_vols.load(Ordering::SeqCst);

    // Folders first (sequential), then files and links in parallel, then
    // alternate streams (their base file must exist), then folder times.
    for j in jobs.iter().filter(|j| j.entry.kind == EntryKind::Dir) {
        if w.stop(ctx) {
            break;
        }
        match std::fs::create_dir_all(&j.dest) {
            Ok(()) => report.folders_created += 1,
            Err(e) => {
                w.errors.record(ErrorClass::Dir, format!("{}: {}", j.entry.path, AppError::from(e).user_message()));
            }
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
                    if wr.stop(ctx) {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst) as usize;
                    let Some(j) = list.get(i) else { return };
                    let n = processed.fetch_add(1, Ordering::SeqCst);
                    ctx.report(&Progress { phase: Phase::Restoring, files_processed: n, files_total: total, bytes_processed: 0, current_path: Some(j.entry.path.clone()) });
                    let mut warns = Vec::new();
                    let r = match j.entry.kind {
                        EntryKind::Symlink => restore_symlink(j.entry, &j.dest, opts, wr, &mut warns).map(|_| None).map_err(|e| (ErrorClass::File, e)),
                        _ => match j.entry.blocks.iter().find(|b| missing_blocks.contains(b.as_str())) {
                            Some(b) => Err((ErrorClass::MissingData, AppError::transfer(format!("data block {} is unavailable", &b[..12.min(b.len())]), "Run list-broken-files."))),
                            None => write_file(file_hash, j.entry, &j.dest, spill.path(), opts, is_stream_entry(&j.entry.path), wr, &mut warns).map(Some).map_err(|e| {
                                let class = if matches!(e, AppError::Transfer { .. }) { ErrorClass::MissingData } else { ErrorClass::File };
                                (class, e)
                            }),
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
                        Err((class, e)) => {
                            wr.errors.record(class, format!("{}: {}", j.entry.path, e.user_message()));
                        }
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
        apply_metadata(&j.dest, j.entry, opts, &w);
    }
    // Mirror: only after a restore that ran to the end, so a stopped
    // restore never deletes what it did not replace.
    if let Some((target, plan)) = mirror_plan {
        if w.errors.should_stop() {
            report.warnings.push("Mirror deletions skipped because the restore stopped early".into());
        } else {
            let (files, folders, errors) = super::restore_mirror::delete(&target, &plan);
            report.files_deleted = files;
            report.folders_deleted = folders;
            for e in errors {
                w.errors.record(ErrorClass::File, e);
            }
        }
    }
    ctx.report(&Progress { phase: Phase::Completed, files_processed: total, files_total: total, bytes_processed: report.bytes_restored, current_path: None });
    Ok(finish(report, w))
}

/// Move what the writers recorded into the report.
fn finish(mut report: RestoreReport, w: Writers) -> RestoreReport {
    let (warnings, errors) = w.errors.take();
    report.warnings.extend(warnings);
    report.errors.extend(errors);
    report.stop_reason = w.errors.stop_reason();
    report.owners_restored = w.owner.restored();
    report.warnings.extend(w.owner.summary());
    report.sparse_files = w.sparse_files.load(Ordering::Relaxed);
    report
}

/// Name of an entry inside an archive: its restore destination relative to
/// the archive root, `/`-separated.
fn archive_names(entries: &[FileEntry], opts: &RestoreOptions) -> Vec<String> {
    let o = RestoreOptions { target: Some(PathBuf::new()), preserve_full_paths: opts.preserve_full_paths, ..Default::default() };
    destinations(entries, &o).iter().map(|p| super::access::components(&p.to_string_lossy()).join("/")).collect()
}

fn restore_archive(repo: &Repo, entries: &[FileEntry], opts: &RestoreOptions, target: &ArchiveTarget, ctx: &OpContext, w: &Writers, report: &mut RestoreReport) -> Result<(), AppError> {
    target.format()?;
    let names = archive_names(entries, opts);
    let mut items: Vec<(&FileEntry, String)> = Vec::new();
    let mut streams = 0;
    for (e, n) in entries.iter().zip(names) {
        if is_stream_entry(&e.path) {
            streams += 1;
        } else if !n.is_empty() && (e.kind != EntryKind::Symlink || opts.restore_symlinks) {
            items.push((e, n));
        }
    }
    if streams > 0 {
        report.warnings.push(format!("{streams} alternate data stream(s) left out: archives have no place for them"));
    }
    report.archive = Some(target.path.to_string_lossy().into_owned());
    if opts.dry_run {
        report.planned = items.iter().map(|(_, n)| n.clone()).collect();
        return Ok(());
    }
    let src = super::access::BlockSource::new(repo)?;
    let src = if opts.use_cache { src } else { src.without_cache() };
    let total = items.len() as u64;
    let out = super::archive::write_archive(
        &src,
        repo.header.file_hash,
        &items,
        target,
        !opts.skip_verify,
        &|| w.stop(ctx),
        &|n, p| ctx.report(&Progress { phase: Phase::Restoring, files_processed: n, files_total: total, bytes_processed: 0, current_path: Some(p.to_string()) }),
    );
    let out = match out {
        Ok(o) => o,
        Err(e) if ctx.is_cancelled() => return Err(ctx.check().err().unwrap_or(e)),
        Err(e) => return Err(e),
    };
    report.files_restored = out.files + out.links;
    report.folders_created = out.folders;
    report.bytes_restored = out.bytes;
    report.volumes_downloaded = src.volumes_loaded();
    for (path, problem, _) in out.skipped {
        w.errors.record(ErrorClass::MissingData, format!("{path}: {problem}"));
    }
    ctx.report(&Progress { phase: Phase::Completed, files_processed: total, files_total: total, bytes_processed: report.bytes_restored, current_path: None });
    Ok(())
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

#[allow(clippy::too_many_arguments)]
fn write_file(file_hash: HashAlgorithm, e: &FileEntry, dest: &Path, spill: &Path, opts: &RestoreOptions, stream: bool, w: &Writers, warns: &mut Vec<String>) -> Result<u64, AppError> {
    if stream && !cfg!(windows) {
        warns.push(format!("Alternate data stream {} skipped (NTFS only)", e.path));
        return Ok(0);
    }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    // Streams are written in place: a temp name + rename would detach them.
    let tmp = if stream { dest.to_path_buf() } else { dest.with_file_name(format!(".{}.ufop-restore", dest.file_name().unwrap_or_default().to_string_lossy())) };
    let n;
    {
        let mut f = std::fs::File::create(&tmp)?;
        // Holes for zero runs (sparse), else preallocate to the final size.
        let sparse = opts.sparse == SparseMode::Auto && !stream && e.size >= 2 * super::sparse::CHUNK as u64 && super::sparse::prepare(&f);
        if !sparse {
            f.set_len(e.size)?;
        }
        let mut out = SparseWriter::new(&mut f, sparse);
        for b in &e.blocks {
            out.write(&std::fs::read(spill.join(b))?)?;
        }
        n = out.position();
        if out.holes {
            w.sparse_files.fetch_add(1, Ordering::Relaxed);
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
    apply_metadata(dest, e, opts, w);
    Ok(n)
}

#[allow(unused_variables)]
fn restore_symlink(e: &FileEntry, dest: &Path, opts: &RestoreOptions, w: &Writers, warns: &mut Vec<String>) -> Result<(), AppError> {
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
        if let Err(msg) = w.owner.apply(dest, e.meta.as_ref()) {
            w.metadata(vec![msg]);
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

    fn two_versions() -> (tempfile::TempDir, tempfile::TempDir, BackupDb, chrono::DateTime<chrono::Utc>) {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "docs/a.txt", b"first");
        write(src.path(), "docs/old.txt", b"only in v1");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let between = chrono::Utc::now();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(src.path(), "docs/a.txt", b"second");
        std::fs::remove_file(src.path().join("docs/old.txt")).unwrap();
        write(src.path(), "docs/new.txt", b"only in v2");
        backup(src.path(), &be, &db);
        (src, dst, db, between)
    }

    #[test]
    fn restore_as_of_a_time_and_by_selector() {
        let (src, dst, db, between) = two_versions();
        let be = LocalBackend::new(dst.path());
        let repo = open(&be, &db);
        let out = tempfile::tempdir().unwrap();
        let root = out.path().join(src.path().file_name().unwrap());
        let o = RestoreOptions { selector: Some(VersionSelector::AsOf { time: between, allow_partial: false }), target: Some(out.path().into()), ..Default::default() };
        let r = run_restore(&repo, &o, &OpContext::new()).unwrap();
        assert_eq!(r.version_index, Some(1), "as of the time between the runs = the older version");
        assert_eq!(std::fs::read(root.join("docs/a.txt")).unwrap(), b"first");
        let r = run_restore(&repo, &RestoreOptions { selector: Some(VersionSelector::Latest { allow_partial: false }), dry_run: true, target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.version_index, Some(0));
        let r = run_restore(&repo, &RestoreOptions { selector: Some(VersionSelector::Oldest { allow_partial: false }), dry_run: true, target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.version_index, Some(1));
        // Before the first version: refused, nothing written.
        let early = RestoreOptions { selector: Some(VersionSelector::AsOf { time: between - chrono::Duration::days(1), allow_partial: false }), target: Some(out.path().join("x")), ..Default::default() };
        assert!(run_restore(&repo, &early, &OpContext::new()).is_err());
        assert!(!out.path().join("x").exists());
        // A partial newest version is skipped by "as of now" unless allowed.
        db.add_fileset("2099-01-01T00:00:00+00:00", "p-partial.dlist", true, &[]).unwrap();
        let now = RestoreOptions { selector: Some(VersionSelector::AsOf { time: chrono::Utc::now() + chrono::Duration::days(36500), allow_partial: false }), dry_run: true, target: Some(out.path().into()), ..Default::default() };
        assert_eq!(run_restore(&repo, &now, &OpContext::new()).unwrap().version_index, Some(1), "the complete newest is now number 1");
    }

    fn archive_names_and_bytes(path: &Path) -> Vec<(String, Vec<u8>)> {
        let n = path.to_string_lossy().to_string();
        let mut out = Vec::new();
        if n.ends_with(".zip") {
            let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
            for i in 0..z.len() {
                let mut f = z.by_index(i).unwrap();
                let mut v = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut v).unwrap();
                out.push((f.name().to_string(), v));
            }
        } else {
            let r: Box<dyn std::io::Read> = if n.ends_with(".tar") { Box::new(std::fs::File::open(path).unwrap()) } else { Box::new(flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())) };
            let mut a = tar::Archive::new(r);
            for e in a.entries().unwrap() {
                let mut e = e.unwrap();
                let name = e.path().unwrap().to_string_lossy().into_owned();
                let mut v = Vec::new();
                std::io::Read::read_to_end(&mut e, &mut v).unwrap();
                out.push((name, v));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn restore_to_zip_tar_and_tgz() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..12_000u32).map(|i| (i * 31 % 253) as u8).collect();
        write(src.path(), "docs/a.txt", b"alpha");
        write(src.path(), "docs/sub/big.bin", &big);
        write(src.path(), "other/c.txt", b"gamma");
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", src.path().join("docs/link")).unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        let repo = open(&be, &db);
        let out = tempfile::tempdir().unwrap();
        let root = src.path().file_name().unwrap().to_string_lossy().into_owned();
        for name in ["all.zip", "all.tar", "all.tgz"] {
            let target = out.path().join(name);
            let o = RestoreOptions { archive: Some(ArchiveTarget { path: target.clone(), format: None }), ..Default::default() };
            let r = run_restore(&repo, &o, &OpContext::new()).unwrap();
            assert!(r.errors.is_empty(), "{name}: {:?}", r.errors);
            assert_eq!(r.archive.as_deref(), Some(target.to_string_lossy().as_ref()));
            let got = archive_names_and_bytes(&target);
            let file = |p: &str| got.iter().find(|(n, _)| n == &format!("{root}/{p}")).map(|(_, d)| d.clone());
            assert_eq!(file("docs/a.txt").unwrap(), b"alpha", "{name}");
            assert_eq!(file("docs/sub/big.bin").unwrap(), big, "{name}");
            assert_eq!(file("other/c.txt").unwrap(), b"gamma");
            assert!(got.iter().any(|(n, _)| n == &format!("{root}/docs/sub/")), "{name}: folders are kept: {:?}", got.iter().map(|g| &g.0).collect::<Vec<_>>());
            assert!(!out.path().join(format!("{name}.partial")).exists());
        }
        // Selection: only docs/, names relative to the selection's parent.
        let sel = format!("{}/docs", src.path().display());
        let target = out.path().join("docs.zip");
        let r = run_restore(&repo, &RestoreOptions { paths: vec![sel], archive: Some(ArchiveTarget { path: target.clone(), format: Some(super::super::archive::ArchiveFormat::ZipStored) }), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty());
        let names: Vec<String> = archive_names_and_bytes(&target).into_iter().map(|g| g.0).collect();
        assert!(names.contains(&"docs/a.txt".to_string()) && names.contains(&"docs/sub/big.bin".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("other")));
        #[cfg(unix)]
        {
            let mut z = zip::ZipArchive::new(std::fs::File::open(&target).unwrap()).unwrap();
            let l = z.by_name("docs/link").unwrap();
            assert!(l.is_symlink(), "links are Unix link entries");
            let t = out.path().join("all.tar");
            let mut a = tar::Archive::new(std::fs::File::open(&t).unwrap());
            let link = a.entries().unwrap().map(|e| e.unwrap()).find(|e| e.path().unwrap().ends_with("docs/link")).unwrap();
            assert_eq!(link.header().entry_type(), tar::EntryType::Symlink);
            assert_eq!(link.link_name().unwrap().unwrap().to_string_lossy(), "a.txt");
        }
        // Dry run lists names and writes nothing; unknown extensions are refused.
        let r = run_restore(&repo, &RestoreOptions { dry_run: true, archive: Some(ArchiveTarget { path: out.path().join("dry.tar"), format: None }), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.planned.contains(&format!("{root}/docs/a.txt")));
        assert!(!out.path().join("dry.tar").exists());
        assert!(run_restore(&repo, &RestoreOptions { archive: Some(ArchiveTarget { path: out.path().join("x.rar"), format: None }), ..Default::default() }, &OpContext::new()).is_err());
    }

    #[test]
    fn mirror_restore_dry_run_then_delete() {
        let (src, dst, db, _) = two_versions();
        let be = LocalBackend::new(dst.path());
        let repo = open(&be, &db);
        let out = tempfile::tempdir().unwrap();
        let root = out.path().join(src.path().file_name().unwrap());
        // Restore the newest version, then add junk and an outsider.
        run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        write(&root, "docs/junk.tmp", b"j");
        write(&root, "docs/cache/x.bin", b"x");
        write(out.path(), "outside.txt", b"keep me");
        // Mirror needs a target.
        assert!(run_restore(&repo, &RestoreOptions { delete_extraneous: true, ..Default::default() }, &OpContext::new()).is_err());
        let o = RestoreOptions { version: 1, target: Some(out.path().into()), delete_extraneous: true, ..Default::default() };
        let dry = run_restore(&repo, &RestoreOptions { dry_run: true, ..o.clone() }, &OpContext::new()).unwrap();
        let mut planned: Vec<String> = dry.planned_deletions.iter().map(|p| p.strip_prefix(&root.to_string_lossy().to_string()).unwrap_or(p).replace('\\', "/")).collect();
        planned.sort();
        assert_eq!(planned, vec!["/docs/cache/", "/docs/cache/x.bin", "/docs/junk.tmp", "/docs/new.txt"], "new.txt is not in version 1");
        assert!(root.join("docs/junk.tmp").exists(), "a dry run deletes nothing");
        let r = run_restore(&repo, &o, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!((r.files_deleted, r.folders_deleted), (3, 1));
        assert_eq!(std::fs::read(root.join("docs/a.txt")).unwrap(), b"first");
        assert_eq!(std::fs::read(root.join("docs/old.txt")).unwrap(), b"only in v1");
        assert!(!root.join("docs/new.txt").exists() && !root.join("docs/cache").exists());
        assert!(out.path().join("outside.txt").exists(), "never outside the restored folders");
        // Restoring only one file never deletes its siblings.
        write(&root, "docs/junk2.tmp", b"j");
        let one = format!("{}/docs/a.txt", src.path().display());
        let r = run_restore(&repo, &RestoreOptions { paths: vec![one], dry_run: true, ..o.clone() }, &OpContext::new()).unwrap();
        assert!(r.planned_deletions.is_empty(), "{:?}", r.planned_deletions);
    }

    #[test]
    fn owner_restore_uses_the_injected_chown() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "a.txt", b"a");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        let entries = db.files(db.fileset_by_version(0).unwrap().id).unwrap();
        #[cfg(unix)]
        assert!(entries.iter().all(|e| e.meta.as_ref().is_some_and(|m| m.uid.is_some() && m.gid.is_some())), "owners are recorded at backup time");
        let repo = open(&be, &db);
        let calls = Mutex::new(Vec::new());
        let chown = |p: &Path, m: &meta::ExtMeta| -> std::io::Result<bool> {
            calls.lock().unwrap().push((p.file_name().map(|n| n.to_string_lossy().into_owned()), m.uid));
            Ok(true)
        };
        let out = tempfile::tempdir().unwrap();
        let o = RestoreOptions { target: Some(out.path().into()), ..Default::default() };
        let r = run_restore_with(&repo, &o, &OpContext::new(), OwnerRestore::new(OwnerPolicy::Auto, true, &chown).unwrap()).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(r.owners_restored, 2, "the file and its folder");
            assert!(calls.lock().unwrap().iter().any(|c| c.0.as_deref() == Some("a.txt")));
        }
        // Unprivileged: one summary warning, no chown.
        let out2 = tempfile::tempdir().unwrap();
        let no = |_: &Path, _: &meta::ExtMeta| -> std::io::Result<bool> { panic!("not privileged") };
        let r = run_restore_with(&repo, &RestoreOptions { target: Some(out2.path().into()), ..Default::default() }, &OpContext::new(), OwnerRestore::new(OwnerPolicy::Auto, false, &no).unwrap()).unwrap();
        #[cfg(unix)]
        assert_eq!(r.warnings.iter().filter(|w| w.contains("owners were not restored")).count(), 1, "{:?}", r.warnings);
        let _ = r;
    }

    #[test]
    fn restore_error_policy_fail_fast_and_continue() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for i in 0..6 {
            write(src.path(), &format!("f{i}"), &vec![i as u8 + 1; 4000]);
        }
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db);
        for f in std::fs::read_dir(dst.path()).unwrap().flatten() {
            if f.file_name().to_string_lossy().contains(".dblock") {
                std::fs::write(f.path(), b"garbage").unwrap();
            }
        }
        let repo = open(&be, &db);
        let out = tempfile::tempdir().unwrap();
        // Default: every problem is an error, the restore runs to the end.
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), legacy: true, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.len() > 1 && r.stop_reason.is_none());
        // Fail fast stops at the first one.
        let p = RestoreErrorPolicy { fail_fast: true, ..Default::default() };
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), legacy: true, error_policy: p, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert!(r.stop_reason.as_deref().unwrap_or_default().contains("fail-fast"));
        // Missing data as warnings only.
        let p = RestoreErrorPolicy { on_missing_data: super::super::errors::ErrorAction::Continue, ..Default::default() };
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), error_policy: p, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty() && !r.warnings.is_empty());
        // Error budget.
        let p = RestoreErrorPolicy { max_errors: Some(2), ..Default::default() };
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), legacy: true, error_policy: p, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.stop_reason.as_deref().unwrap_or_default().contains("more than 2"));
    }

    #[cfg(unix)]
    #[test]
    fn sparse_restore_keeps_holes_and_content() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let mut data = b"head".to_vec();
        data.extend(vec![0u8; 8 << 20]);
        data.extend(b"tail");
        write(src.path(), "disk.img", &data);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { block_size: 64 * 1024, volume_size: 1 << 20, ..Default::default() };
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        let repo = open(&be, &db);
        let out = tempfile::tempdir().unwrap();
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().join("s")), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.sparse_files, 1);
        let p = out.path().join("s").join(src.path().file_name().unwrap()).join("disk.img");
        assert_eq!(std::fs::read(&p).unwrap(), data, "byte-identical");
        if crate::backup::sparse::tests::supports_holes(out.path()) {
            let alloc = crate::backup::sparse::allocated(&p).unwrap();
            assert!(alloc < data.len() as u64 / 2, "allocated {alloc} of {}", data.len());
        }
        // Off writes every byte, same content.
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.path().join("d")), sparse: SparseMode::Off, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.sparse_files, 0);
        assert_eq!(std::fs::read(out.path().join("d").join(src.path().file_name().unwrap()).join("disk.img")).unwrap(), data);
    }

    #[test]
    fn restore_report_reads_old_json() {
        let r: RestoreReport = serde_json::from_str(r#"{"version":"v","files_restored":1,"folders_created":0,"files_skipped_matching":0,"files_skipped_conflict":0,"files_skipped_duplicate":0,"bytes_restored":0,"volumes_downloaded":0,"blocks_from_local":0,"planned":[],"warnings":[],"errors":[]}"#).unwrap();
        assert_eq!(r.files_restored, 1);
        let o: RestoreOptions = serde_json::from_str(r#"{"version":2}"#).unwrap();
        assert_eq!((o.version, o.restore_owner, o.sparse, o.delete_extraneous), (2, OwnerPolicy::Auto, SparseMode::Auto, false));
    }
}
