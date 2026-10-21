//! Repository maintenance: retention/deletion, compaction, verification,
//! consistency repair, database recreation from remote, broken-file
//! inspection, purging, search and version comparison.

use super::backend::Backend;
use super::crypto::KdfParams;
use super::db::BackupDb;
use super::engine::{flush, Pending, Repo};
use super::restore::path_selected;
use super::retention::RetentionPolicy;
use super::volume::{self, FileListVolume, IndexVolume, IndexedBlock, VolumeKind};
use super::{hash_bytes, BackupOptions, EntryKind, FileEntry, HashAlgorithm, OpContext};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

/// Delete versions by index (0 = newest). Refuses to remove every version
/// unless `allow_full_removal` is set.
pub fn delete_versions(repo: &Repo, versions: &[usize], allow_full_removal: bool) -> Result<usize, AppError> {
    let all = repo.db.filesets()?;
    let ids: Vec<i64> = versions.iter().filter_map(|v| all.get(*v).map(|f| f.id)).collect();
    delete_fileset_ids(repo, &ids, allow_full_removal)
}

fn delete_fileset_ids(repo: &Repo, ids: &[i64], allow_full_removal: bool) -> Result<usize, AppError> {
    let all = repo.db.filesets()?;
    let doomed: Vec<_> = all.iter().filter(|f| ids.contains(&f.id)).collect();
    if !allow_full_removal && !all.is_empty() && doomed.len() >= all.len() {
        return Err(AppError::validation(
            "Refusing to delete the last remaining backup version (enable 'allow full removal' to override)",
        ));
    }
    for f in &doomed {
        repo.delete_volume_files(&f.volume_name)?;
        repo.db.delete_fileset(f.id)?;
    }
    Ok(doomed.len())
}

/// Apply a retention policy; returns how many versions were removed.
pub fn apply_retention(repo: &Repo, policy: &RetentionPolicy, allow_full_removal: bool) -> Result<usize, AppError> {
    let versions: Vec<(i64, DateTime<Utc>)> =
        repo.db.filesets()?.iter().filter_map(|f| parse_ts(&f.timestamp).map(|t| (f.id, t))).collect();
    let del = policy.select_deletions(&versions, Utc::now())?;
    if del.is_empty() {
        return Ok(0);
    }
    delete_fileset_ids(repo, &del, allow_full_removal)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactReport {
    pub volumes_removed: usize,
    pub volumes_rewritten: usize,
    pub bytes_reclaimed: u64,
}

fn delete_block_volume(repo: &Repo, name: &str) -> Result<(), AppError> {
    repo.delete_volume_files(name)?;
    let key = format!("index:{name}");
    if let Some(idx) = repo.db.get_kv(&key)? {
        repo.delete_volume_files(&idx)?;
    }
    Ok(())
}

/// Remove fully unused block volumes and repack mostly-unused ones.
pub fn compact(repo: &Repo, opts: &BackupOptions, ctx: &OpContext) -> Result<CompactReport, AppError> {
    let referenced = repo.db.referenced_blocks()?;
    let by_volume = repo.db.blocks_by_volume()?;
    let volumes: HashMap<String, u64> =
        repo.db.volumes(Some(VolumeKind::Block))?.into_iter().map(|v| (v.name, v.size)).collect();
    let mut r = CompactReport::default();
    let mut pending = Pending::for_repo(repo, opts);
    let mut scratch = super::engine::BackupReport::default();
    let mut rewritten = Vec::new();
    for (name, size) in &volumes {
        ctx.check()?;
        let blocks = by_volume.get(name).cloned().unwrap_or_default();
        let total: u64 = blocks.iter().map(|(_, s)| *s).sum();
        let used: Vec<&(String, u64)> = blocks.iter().filter(|(h, _)| referenced.contains(h)).collect();
        let used_bytes: u64 = used.iter().map(|(_, s)| *s).sum();
        if used.is_empty() {
            delete_block_volume(repo, name)?;
            r.volumes_removed += 1;
            r.bytes_reclaimed += size;
            continue;
        }
        let wasted_pct = ((total - used_bytes) * 100).checked_div(total).unwrap_or(0);
        if wasted_pct >= opts.compact_threshold_percent as u64 {
            let data = repo.cipher.open(&repo.get_volume(name)?)?;
            let map = volume::read_block_volume(&data)?;
            for (h, _) in used {
                if let Some(d) = map.get(h) {
                    pending.add_block(h.clone(), d, opts.compression_level)?;
                    if pending.byte_size() >= opts.volume_size {
                        flush(repo, &mut pending, opts, &mut scratch)?;
                    }
                }
            }
            rewritten.push((name.clone(), *size));
        }
    }
    flush(repo, &mut pending, opts, &mut scratch)?;
    // Old volumes are deleted only after their live blocks are safely re-uploaded.
    for (name, size) in rewritten {
        delete_block_volume(repo, &name)?;
        r.volumes_rewritten += 1;
        r.bytes_reclaimed += size;
    }
    if r.volumes_removed + r.volumes_rewritten > 0 {
        repo.db.vacuum()?;
    }
    Ok(r)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyReport {
    pub checked: usize,
    pub errors: Vec<String>,
}

/// Download volumes and check them. `samples = None` checks every volume;
/// `full` also decodes each block and checks its hash.
pub fn verify(repo: &Repo, samples: Option<usize>, full: bool) -> Result<VerifyReport, AppError> {
    let mut vols = repo.db.volumes(None)?;
    if let Some(n) = samples {
        vols.shuffle(&mut rand::thread_rng());
        // Always include the newest file list in a sample.
        vols.sort_by_key(|v| v.kind != VolumeKind::FileList);
        vols.truncate(n);
    }
    let mut r = VerifyReport::default();
    for v in vols {
        r.checked += 1;
        let res = (|| -> Result<(), AppError> {
            let payload = repo.cipher.open(&repo.get_volume_ex(&v.name, false)?)?;
            match v.kind {
                VolumeKind::Block => {
                    let m = volume::read_block_volume(&payload)?;
                    if full {
                        for (h, d) in &m {
                            if hash_bytes(repo.header.block_hash, d) != *h {
                                return Err(AppError::transfer(format!("block {h} hash mismatch"), "Run Repair."));
                            }
                        }
                    }
                }
                VolumeKind::Index => {
                    serde_json::from_slice::<IndexVolume>(&payload)?;
                }
                VolumeKind::FileList => {
                    serde_json::from_slice::<FileListVolume>(&payload)?;
                }
            }
            Ok(())
        })();
        match res {
            Ok(()) => repo.db.set_volume_state(&v.name, "verified")?,
            Err(e) => r.errors.push(format!("{}: {}", v.name, e.user_message())),
        }
    }
    Ok(r)
}

/// Upload `{prefix}-verification.json`: an unencrypted inventory of every
/// remote volume with size and SHA-256, for independent checking.
pub fn upload_verification_file(repo: &Repo) -> Result<(), AppError> {
    #[derive(Serialize)]
    struct Item {
        name: String,
        size: u64,
        sha256: String,
    }
    let items: Vec<Item> = repo
        .db
        .volumes(None)?
        .into_iter()
        .filter(|v| !v.hash.is_empty())
        .map(|v| Item { name: v.name, size: v.size, sha256: v.hash })
        .collect();
    repo.backend.put(&volume::verification_file_name(&repo.prefix), &serde_json::to_vec_pretty(&items)?)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecreateReport {
    pub versions: usize,
    pub block_volumes: usize,
    pub blocks: usize,
    pub warnings: Vec<String>,
}

/// Rebuild the local database from the remote files alone (lost or damaged
/// local state, or restoring on a new machine).
pub fn recreate_database(
    backend: &dyn Backend,
    db: &BackupDb,
    passphrase: Option<&str>,
    prefix: &str,
    ctx: &OpContext,
) -> Result<RecreateReport, AppError> {
    let opts = BackupOptions { prefix: prefix.to_string(), ..Default::default() };
    let repo = Repo::open(backend, db, passphrase, &opts, false, KdfParams::default())?;
    let listing = backend.list()?;
    let sizes: HashMap<&str, u64> = listing.iter().map(|f| (f.name.as_str(), f.size)).collect();
    db.clear()?;
    let mut r = RecreateReport::default();
    let mut indexed: HashSet<String> = HashSet::new();
    db.begin()?;
    for f in listing.iter().filter(|f| volume::classify(prefix, &f.name) == Some(VolumeKind::Index)) {
        ctx.check()?;
        let data = backend.get(&f.name)?;
        let idx: IndexVolume = match volume::open_json(&repo.cipher, &data) {
            Ok(i) => i,
            Err(e) => {
                r.warnings.push(format!("{}: {}", f.name, e.user_message()));
                continue;
            }
        };
        let Some(bsize) = sizes.get(idx.block_volume.as_str()) else {
            r.warnings.push(format!("{} refers to missing {}", f.name, idx.block_volume));
            continue;
        };
        db.add_volume(&f.name, VolumeKind::Index, f.size, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
        let vid = db.add_volume(&idx.block_volume, VolumeKind::Block, *bsize, "", "uploaded")?;
        db.set_kv(&format!("index:{}", idx.block_volume), &f.name)?;
        for b in &idx.blocks {
            db.add_block(&b.hash, b.size, vid)?;
        }
        r.blocks += idx.blocks.len();
        r.block_volumes += 1;
        indexed.insert(idx.block_volume);
    }
    // Block volumes with no index: read the volume itself.
    for f in listing.iter().filter(|f| volume::classify(prefix, &f.name) == Some(VolumeKind::Block) && !indexed.contains(&f.name)) {
        ctx.check()?;
        let data = backend.get(&f.name)?;
        match repo.cipher.open(&data).and_then(|p| volume::read_block_volume(&p)) {
            Ok(m) => {
                let vid = db.add_volume(&f.name, VolumeKind::Block, f.size, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
                for (h, d) in &m {
                    db.add_block(h, d.len() as u64, vid)?;
                }
                r.blocks += m.len();
                r.block_volumes += 1;
                r.warnings.push(format!("{} had no index volume; read it directly", f.name));
            }
            Err(e) => r.warnings.push(format!("{}: {}", f.name, e.user_message())),
        }
    }
    for f in listing.iter().filter(|f| volume::classify(prefix, &f.name) == Some(VolumeKind::FileList)) {
        ctx.check()?;
        let data = backend.get(&f.name)?;
        match volume::open_json::<FileListVolume>(&repo.cipher, &data) {
            Ok(fl) => {
                db.add_volume(&f.name, VolumeKind::FileList, f.size, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
                db.add_fileset(&fl.timestamp, &f.name, fl.partial, &fl.files)?;
                r.versions += 1;
            }
            Err(e) => r.warnings.push(format!("{}: {}", f.name, e.user_message())),
        }
    }
    db.commit()?;
    Ok(r)
}

/// Read the job configuration embedded in the newest file list, if any.
pub fn read_remote_task_config(
    backend: &dyn Backend,
    passphrase: Option<&str>,
    prefix: &str,
) -> Result<Option<serde_json::Value>, AppError> {
    let db = BackupDb::open_in_memory()?;
    let opts = BackupOptions { prefix: prefix.to_string(), ..Default::default() };
    let repo = Repo::open(backend, &db, passphrase, &opts, false, KdfParams::default())?;
    let mut lists: Vec<String> = backend
        .list()?
        .into_iter()
        .filter(|f| volume::classify(prefix, &f.name) == Some(VolumeKind::FileList))
        .map(|f| f.name)
        .collect();
    lists.sort();
    for name in lists.iter().rev() {
        let fl: FileListVolume = volume::open_json(&repo.cipher, &backend.get(name)?)?;
        if fl.task_config.is_some() {
            return Ok(fl.task_config);
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairReport {
    pub reuploaded: Vec<String>,
    pub unrecoverable: Vec<String>,
    pub removed_incomplete: Vec<String>,
    pub broken_files: usize,
}

/// Re-upload file lists and indexes that are missing remotely, using the
/// local database; report block volumes that cannot be regenerated.
pub fn repair(repo: &Repo) -> Result<RepairReport, AppError> {
    let cons = repo.consistency_check()?;
    let mut r = RepairReport::default();
    for name in &cons.incomplete {
        repo.db.delete_volume(name)?;
        r.removed_incomplete.push(name.clone());
    }
    let filesets = repo.db.filesets()?;
    let by_volume = repo.db.blocks_by_volume()?;
    for name in cons.missing.iter().chain(cons.size_mismatch.iter()) {
        match volume::classify(&repo.prefix, name) {
            Some(VolumeKind::FileList) => {
                if let Some(fs) = filesets.iter().find(|f| &f.volume_name == name) {
                    repo.rewrite_filelist(&fs.timestamp, fs.partial, name, &repo.db.files(fs.id)?)?;
                    r.reuploaded.push(name.clone());
                }
            }
            Some(VolumeKind::Index) => {
                let block = repo.db.volumes(Some(VolumeKind::Block))?.into_iter().find(|b| {
                    repo.db.get_kv(&format!("index:{}", b.name)).ok().flatten().as_deref() == Some(name.as_str())
                });
                if let Some(b) = block {
                    let idx = IndexVolume {
                        block_volume: b.name.clone(),
                        blocks: by_volume.get(&b.name).cloned().unwrap_or_default().into_iter().map(|(hash, size)| IndexedBlock { hash, size }).collect(),
                    };
                    let data = volume::seal_json(&repo.cipher, &idx)?;
                    repo.backend.put(name, &data)?;
                    repo.db.add_volume(name, VolumeKind::Index, data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
                    r.reuploaded.push(name.clone());
                }
            }
            _ => r.unrecoverable.push(name.clone()),
        }
    }
    r.broken_files = list_broken_files(repo)?.len();
    Ok(r)
}

/// A file that cannot be fully restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokenFile {
    pub version: String,
    pub path: String,
    pub missing_blocks: usize,
}

/// Files whose blocks live in missing volumes (or nowhere).
pub fn list_broken_files(repo: &Repo) -> Result<Vec<BrokenFile>, AppError> {
    let cons = repo.consistency_check()?;
    let bad: HashSet<&String> = cons.missing.iter().chain(cons.size_mismatch.iter()).collect();
    affected_by(repo, |vol| vol.map(|v| bad.contains(&v.to_string())).unwrap_or(true))
}

/// Files that depend on any of the named remote volumes.
pub fn affected(repo: &Repo, volumes: &[String]) -> Result<Vec<BrokenFile>, AppError> {
    let set: HashSet<&String> = volumes.iter().collect();
    affected_by(repo, |vol| vol.map(|v| set.contains(&v.to_string())).unwrap_or(false))
}

fn affected_by(repo: &Repo, is_bad: impl Fn(Option<&str>) -> bool) -> Result<Vec<BrokenFile>, AppError> {
    let loc = repo.db.block_locations()?;
    let mut out = Vec::new();
    for fs in repo.db.filesets()? {
        for f in repo.db.files(fs.id)? {
            if f.kind != EntryKind::File {
                continue;
            }
            let n = f.blocks.iter().filter(|b| is_bad(loc.get(*b).map(String::as_str))).count();
            if n > 0 {
                out.push(BrokenFile { version: fs.timestamp.clone(), path: f.path, missing_blocks: n });
            }
        }
    }
    Ok(out)
}

fn rewrite_fileset(repo: &Repo, fs: &super::Fileset, files: &[FileEntry]) -> Result<(), AppError> {
    repo.rewrite_filelist(&fs.timestamp, fs.partial, &fs.volume_name, files)?;
    repo.db.replace_fileset_files(fs.id, &fs.volume_name, files)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PurgeReport {
    pub versions_rewritten: usize,
    pub entries_removed: usize,
    pub dry_run: bool,
}

/// Remove matching paths from the selected versions (all when `None`).
pub fn purge(repo: &Repo, selectors: &[String], versions: Option<&[usize]>, dry_run: bool) -> Result<PurgeReport, AppError> {
    if selectors.is_empty() {
        return Err(AppError::validation("Give at least one path or pattern to purge"));
    }
    let mut r = PurgeReport { dry_run, ..Default::default() };
    for (i, fs) in repo.db.filesets()?.iter().enumerate() {
        if versions.is_some_and(|v| !v.contains(&i)) {
            continue;
        }
        let files = repo.db.files(fs.id)?;
        let kept: Vec<FileEntry> = files.iter().filter(|f| !path_selected(&f.path, selectors)).cloned().collect();
        let removed = files.len() - kept.len();
        if removed > 0 {
            r.entries_removed += removed;
            r.versions_rewritten += 1;
            if !dry_run {
                rewrite_fileset(repo, fs, &kept)?;
            }
        }
    }
    Ok(r)
}

/// Rewrite history without unrecoverable files so backups can continue.
pub fn purge_broken_files(repo: &Repo, dry_run: bool) -> Result<PurgeReport, AppError> {
    let broken = list_broken_files(repo)?;
    let mut by_version: HashMap<String, HashSet<String>> = HashMap::new();
    for b in &broken {
        by_version.entry(b.version.clone()).or_default().insert(b.path.clone());
    }
    let mut r = PurgeReport { dry_run, ..Default::default() };
    for fs in repo.db.filesets()? {
        let Some(bad) = by_version.get(&fs.timestamp) else { continue };
        let kept: Vec<FileEntry> = repo.db.files(fs.id)?.into_iter().filter(|f| !bad.contains(&f.path)).collect();
        r.entries_removed += bad.len();
        r.versions_rewritten += 1;
        if !dry_run {
            rewrite_fileset(repo, &fs, &kept)?;
        }
    }
    Ok(r)
}

/// One search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundFile {
    pub version: usize,
    pub timestamp: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: Option<i64>,
    pub hash: Option<String>,
}

/// Search paths (substring or wildcard, case-insensitive) in one version or
/// across all versions.
pub fn find(db: &BackupDb, pattern: &str, version: Option<usize>, limit: usize) -> Result<Vec<FoundFile>, AppError> {
    let pat = pattern.to_lowercase();
    let glob = if pat.contains('*') || pat.contains('?') { glob::Pattern::new(&pat).ok() } else { None };
    let mut out = Vec::new();
    for (i, fs) in db.filesets()?.iter().enumerate() {
        if version.is_some_and(|v| v != i) {
            continue;
        }
        for f in db.files(fs.id)? {
            let p = f.path.to_lowercase();
            let hit = match &glob {
                Some(g) => g.matches(&p),
                None => p.contains(&pat),
            };
            if hit {
                out.push(FoundFile { version: i, timestamp: fs.timestamp.clone(), path: f.path, kind: f.kind, size: f.size, mtime: f.mtime, hash: f.hash });
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionComparison {
    pub base: String,
    pub compare: String,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub modified: Vec<String>,
}

/// Differences going from version `a` to version `b`.
pub fn compare_versions(db: &BackupDb, a: usize, b: usize) -> Result<VersionComparison, AppError> {
    let fa = db.fileset_by_version(a)?;
    let fb = db.fileset_by_version(b)?;
    let ma: HashMap<String, FileEntry> = db.files(fa.id)?.into_iter().map(|f| (f.path.clone(), f)).collect();
    let mb: HashMap<String, FileEntry> = db.files(fb.id)?.into_iter().map(|f| (f.path.clone(), f)).collect();
    let mut c = VersionComparison { base: fa.timestamp, compare: fb.timestamp, ..Default::default() };
    for (p, e) in &mb {
        match ma.get(p) {
            None => c.added.push(p.clone()),
            Some(o) if o.hash != e.hash || o.size != e.size || o.symlink_target != e.symlink_target => c.modified.push(p.clone()),
            _ => {}
        }
    }
    c.deleted = ma.keys().filter(|p| !mb.contains_key(*p)).cloned().collect();
    c.added.sort();
    c.deleted.sort();
    c.modified.sort();
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::restore::{run_restore, RestoreOptions};
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, SourceFilter};
    use std::path::Path;

    fn o() -> BackupOptions {
        BackupOptions { block_size: 1024, volume_size: 2048, compact_threshold_percent: 10, verify_samples: 0, ..Default::default() }
    }

    fn run(src: &Path, be: &LocalBackend, db: &BackupDb, ret: &RetentionPolicy) {
        let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), &o(), ret, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    fn open<'a>(be: &'a LocalBackend, db: &'a BackupDb) -> Repo<'a> {
        Repo::open(be, db, Some("pw"), &o(), false, KdfParams::insecure_fast()).unwrap()
    }

    fn tick() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    #[test]
    fn retention_compaction_compare_and_find() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        write(src.path(), "keep.txt", b"keep");
        write(src.path(), "churn.bin", &(0..6000u32).map(|i| (i % 253) as u8).collect::<Vec<_>>());
        run(src.path(), &be, &db, &RetentionPolicy::KeepAll);
        tick();
        write(src.path(), "churn.bin", &(0..6000u32).map(|i| (i % 241) as u8).collect::<Vec<_>>());
        write(src.path(), "new.txt", b"n");
        run(src.path(), &be, &db, &RetentionPolicy::KeepAll);

        let c = compare_versions(&db, 1, 0).unwrap();
        assert_eq!(c.added.len(), 1);
        assert_eq!(c.modified.len(), 1);
        assert!(c.deleted.is_empty());
        assert_eq!(find(&db, "*.TXT", None, 100).unwrap().len(), 3);
        assert_eq!(find(&db, "new", Some(1), 100).unwrap().len(), 0);

        let before = be.list().unwrap().len();
        tick();
        write(src.path(), "x", b"x");
        run(src.path(), &be, &db, &RetentionPolicy::KeepCount { count: 1 });
        assert_eq!(db.filesets().unwrap().len(), 1);
        // Compaction removed the old churn.bin blocks.
        assert!(be.list().unwrap().len() < before + 3);
        assert!(db.integrity_problems().unwrap().is_empty());
        let repo = open(&be, &db);
        assert!(verify(&repo, None, true).unwrap().errors.is_empty());
        assert!(delete_versions(&repo, &[0], false).is_err());
    }

    #[test]
    fn recreate_database_and_restore_without_original_db() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        write(src.path(), "d/f.txt", b"payload");
        let opts = BackupOptions { task_config: Some(serde_json::json!({"name": "job"})), ..o() };
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &opts, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();

        let fresh = BackupDb::open_in_memory().unwrap();
        assert!(recreate_database(&be, &fresh, Some("bad"), "ufop", &OpContext::new()).is_err());
        let r = recreate_database(&be, &fresh, Some("pw"), "ufop", &OpContext::new()).unwrap();
        assert_eq!(r.versions, 1);
        let repo = open(&be, &fresh);
        let rr = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert_eq!(rr.files_restored, 1);
        let cfg = read_remote_task_config(&be, Some("pw"), "ufop").unwrap().unwrap();
        assert_eq!(cfg["name"], "job");
    }

    #[test]
    fn repair_broken_and_purge() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        write(src.path(), "a.txt", b"aaa");
        write(src.path(), "b.log", b"bbb");
        run(src.path(), &be, &db, &RetentionPolicy::KeepAll);
        let repo = open(&be, &db);

        // Lose the file list and an index: repair re-uploads both.
        let names: Vec<String> = be.list().unwrap().into_iter().map(|f| f.name).collect();
        let dlist = names.iter().find(|n| n.contains(".dlist")).unwrap().clone();
        let dindex = names.iter().find(|n| n.contains(".dindex")).unwrap().clone();
        be.delete(&dlist).unwrap();
        be.delete(&dindex).unwrap();
        let r = repair(&repo).unwrap();
        assert_eq!(r.reuploaded.len(), 2);
        assert!(repo.consistency_check().unwrap().is_clean());

        // Purge a pattern.
        let p = purge(&repo, &["*.log".into()], None, false).unwrap();
        assert_eq!(p.entries_removed, 1);
        assert!(find(&db, ".log", None, 10).unwrap().is_empty());

        // Lose the block volume: files become broken, purge-broken cleans them.
        let dblock = be.list().unwrap().into_iter().find(|f| f.name.contains(".dblock")).unwrap().name;
        assert_eq!(affected(&repo, std::slice::from_ref(&dblock)).unwrap().len(), 1);
        be.delete(&dblock).unwrap();
        assert_eq!(list_broken_files(&repo).unwrap().len(), 1);
        let r = repair(&repo).unwrap();
        assert_eq!(r.unrecoverable, vec![dblock]);
        let pb = purge_broken_files(&repo, false).unwrap();
        assert_eq!(pb.entries_removed, 1);
        assert!(list_broken_files(&repo).unwrap().is_empty());
    }
}
