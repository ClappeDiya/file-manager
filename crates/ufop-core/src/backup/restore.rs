//! Restore: pick a version and paths, download only the dblocks that hold
//! the needed blocks, rebuild files to the original or an alternate
//! location, verify their hashes and carry on past individual failures.

use super::engine::Repo;
use super::volume;
use super::{hash_file, EntryKind, FileEntry, OpContext, Phase, Progress};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

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
    /// Skip post-restore hash checks (recovery situations only).
    pub skip_verify: bool,
    pub dry_run: bool,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            version: 0,
            paths: Vec::new(),
            target: None,
            preserve_full_paths: false,
            conflict: ConflictPolicy::Overwrite,
            restore_permissions: false,
            restore_symlinks: true,
            skip_verify: false,
            dry_run: false,
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
    pub bytes_restored: u64,
    pub volumes_downloaded: u64,
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

fn apply_metadata(path: &Path, e: &FileEntry, opts: &RestoreOptions) {
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
    #[cfg(not(unix))]
    let _ = opts;
}

/// Restore from an opened repository.
pub fn run_restore(repo: &Repo, opts: &RestoreOptions, ctx: &OpContext) -> Result<RestoreReport, AppError> {
    let fs = repo.db.fileset_by_version(opts.version)?;
    let mut report = RestoreReport { version: fs.timestamp.clone(), ..Default::default() };
    let entries: Vec<FileEntry> = repo.db.files(fs.id)?.into_iter().filter(|e| path_selected(&e.path, &opts.paths)).collect();
    if entries.is_empty() {
        return Err(AppError::validation("No backed-up files match the selected paths in this version"));
    }
    let dests = destinations(&entries, opts);

    // Plan: decide per entry what happens, collect needed blocks.
    struct Job<'e> {
        entry: &'e FileEntry,
        dest: PathBuf,
    }
    let mut jobs = Vec::new();
    for (e, d) in entries.iter().zip(dests) {
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
    if opts.dry_run {
        report.planned = jobs.iter().map(|j| j.dest.to_string_lossy().into_owned()).collect();
        return Ok(report);
    }

    let locations = repo.db.block_locations()?;
    let needed: HashSet<&str> = jobs
        .iter()
        .filter(|j| j.entry.kind == EntryKind::File)
        .flat_map(|j| j.entry.blocks.iter().map(String::as_str))
        .collect();
    let mut by_volume: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut missing_blocks: HashSet<&str> = HashSet::new();
    for h in &needed {
        match locations.get(*h) {
            Some(v) => by_volume.entry(v.as_str()).or_default().push(h),
            None => {
                missing_blocks.insert(h);
            }
        }
    }

    // Download and spill needed blocks to a temp dir, one volume at a time.
    let spill = tempfile_dir()?;
    let total_vol = by_volume.len() as u64;
    for (i, (vol, hashes)) in by_volume.iter().enumerate() {
        ctx.check()?;
        ctx.report(&Progress { phase: Phase::Downloading, files_processed: i as u64, files_total: total_vol, bytes_processed: report.bytes_restored, current_path: Some(vol.to_string()) });
        let blocks = repo.get_volume(vol).and_then(|d| repo.cipher.open(&d)).and_then(|p| volume::read_block_volume(&p));
        match blocks {
            Ok(mut map) => {
                report.volumes_downloaded += 1;
                for h in hashes {
                    match map.remove(*h) {
                        Some(data) => std::fs::write(spill.path().join(h), data)?,
                        None => {
                            missing_blocks.insert(h);
                        }
                    }
                }
            }
            Err(e) => {
                report.errors.push(format!("Volume {vol}: {}", e.user_message()));
                missing_blocks.extend(hashes.iter().copied());
            }
        }
    }

    // Directories first, then files and links.
    let total = jobs.len() as u64;
    for (i, j) in jobs.iter().enumerate() {
        ctx.check()?;
        ctx.report(&Progress { phase: Phase::Restoring, files_processed: i as u64, files_total: total, bytes_processed: report.bytes_restored, current_path: Some(j.entry.path.clone()) });
        let r = match j.entry.kind {
            EntryKind::Dir => std::fs::create_dir_all(&j.dest).map(|_| {
                report.folders_created += 1;
            }).map_err(AppError::from),
            EntryKind::Symlink => restore_symlink(j.entry, &j.dest, opts, &mut report),
            EntryKind::File => {
                if let Some(b) = j.entry.blocks.iter().find(|b| missing_blocks.contains(b.as_str())) {
                    Err(AppError::transfer(format!("data block {} is unavailable", &b[..12.min(b.len())]), "Run list-broken-files."))
                } else {
                    write_file(repo, j.entry, &j.dest, spill.path(), opts).map(|n| {
                        report.files_restored += 1;
                        report.bytes_restored += n;
                    })
                }
            }
        };
        if let Err(e) = r {
            report.errors.push(format!("{}: {}", j.entry.path, e.user_message()));
        }
    }
    // Directory mtimes last, since writing children changes them.
    for j in jobs.iter().filter(|j| j.entry.kind == EntryKind::Dir) {
        apply_metadata(&j.dest, j.entry, opts);
    }
    ctx.report(&Progress { phase: Phase::Completed, files_processed: total, files_total: total, bytes_processed: report.bytes_restored, current_path: None });
    Ok(report)
}

fn tempfile_dir() -> Result<TempDir, AppError> {
    let p = std::env::temp_dir().join(format!("ufop-restore-{}", uuid::Uuid::new_v4().simple()));
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

fn write_file(repo: &Repo, e: &FileEntry, dest: &Path, spill: &Path, opts: &RestoreOptions) -> Result<u64, AppError> {
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = dest.with_file_name(format!(".{}.ufop-restore", dest.file_name().unwrap_or_default().to_string_lossy()));
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
            if &hash_file(repo.header.file_hash, &tmp)? != h {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::transfer("restored content failed hash verification", "The backup data may be damaged; run Verify."));
            }
        }
    }
    std::fs::rename(&tmp, dest)?;
    apply_metadata(dest, e, opts);
    Ok(n)
}

#[allow(unused_variables)]
fn restore_symlink(e: &FileEntry, dest: &Path, opts: &RestoreOptions, report: &mut RestoreReport) -> Result<(), AppError> {
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
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        report.warnings.push(format!("Symbolic link {} → {target} not recreated on this platform", dest.display()));
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
}
