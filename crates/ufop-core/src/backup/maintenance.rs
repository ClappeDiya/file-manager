//! Repository maintenance: retention/deletion, compaction, verification,
//! consistency repair, database recreation from remote, broken-file
//! inspection, purging, search and version comparison.

use super::backend::Backend;
use super::crypto::KdfParams;
use super::db::BackupDb;
use super::engine::{flush, Pending, Repo};
use super::restore::path_selected;
use super::retention::{RetentionCandidate, RetentionPolicy, RetentionVerdict};
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
/// unless `allow_full_removal` is set, and refuses pinned versions.
pub fn delete_versions(repo: &Repo, versions: &[usize], allow_full_removal: bool) -> Result<usize, AppError> {
    delete_versions_ex(repo, versions, allow_full_removal, false)
}

/// [`delete_versions`]; with `force` pinned versions are deleted too.
pub fn delete_versions_ex(repo: &Repo, versions: &[usize], allow_full_removal: bool, force: bool) -> Result<usize, AppError> {
    let all = repo.db.filesets()?;
    let picked: Vec<&super::Fileset> = versions.iter().filter_map(|v| all.get(*v)).collect();
    if !force {
        if let Some(p) = picked.iter().find(|f| f.pinned) {
            return Err(AppError::state(
                format!("Version {} is pinned (Keep forever)", p.timestamp),
                "Unpin it first, or delete with force.",
            ));
        }
    }
    let ids: Vec<i64> = picked.iter().map(|f| f.id).collect();
    delete_fileset_ids(repo, &ids, allow_full_removal, force)
}

fn delete_fileset_ids(repo: &Repo, ids: &[i64], allow_full_removal: bool, force: bool) -> Result<usize, AppError> {
    let all = repo.db.filesets()?;
    let doomed: Vec<_> = all.iter().filter(|f| ids.contains(&f.id)).collect();
    if !allow_full_removal && !all.is_empty() && doomed.len() >= all.len() {
        return Err(AppError::validation(
            "Refusing to delete the last remaining backup version (enable 'allow full removal' to override)",
        ));
    }
    let mut n = 0;
    for f in &doomed {
        if f.pinned && !force {
            tracing::info!("Keeping version {} (pinned)", f.timestamp);
            continue;
        }
        // Locked versions stay until their lock expires.
        if let Some(until) = repo.locked_until(&f.volume_name)? {
            tracing::info!("Keeping version {} (locked until {until})", f.timestamp);
            continue;
        }
        repo.delete_volume_files(&f.volume_name)?;
        repo.db.delete_fileset(f.id)?;
        n += 1;
    }
    Ok(n)
}

/// The current source's versions as retention sees them (pins, locks).
pub fn retention_candidates(db: &BackupDb) -> Result<Vec<RetentionCandidate>, AppError> {
    let now = Utc::now();
    let locks: HashMap<String, String> = db
        .volumes(Some(VolumeKind::FileList))?
        .into_iter()
        .filter_map(|v| v.lock_until.filter(|u| parse_ts(u).is_some_and(|t| t > now)).map(|u| (v.name, u)))
        .collect();
    Ok(db
        .filesets()?
        .into_iter()
        .filter_map(|f| {
            parse_ts(&f.timestamp).map(|t| RetentionCandidate { id: f.id, time: t, pinned: f.pinned, locked_until: locks.get(&f.volume_name).cloned() })
        })
        .collect())
}

/// What a retention policy would keep and delete, version by version.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionPreview {
    pub policy: RetentionPolicy,
    pub legal_hold: bool,
    pub keep: usize,
    pub delete: usize,
    /// Newest first.
    pub versions: Vec<RetentionVerdict>,
}

/// Dry run of retention on the current source (nothing is changed).
pub fn preview_retention(db: &BackupDb, policy: &RetentionPolicy, legal_hold: bool) -> Result<RetentionPreview, AppError> {
    let versions = policy.plan(&retention_candidates(db)?, Utc::now(), legal_hold)?;
    let keep = versions.iter().filter(|v| v.keep).count();
    Ok(RetentionPreview { policy: policy.clone(), legal_hold, keep, delete: versions.len() - keep, versions })
}

/// Apply a retention policy to the current source; returns how many
/// versions were removed. Pinned and locked versions are kept.
pub fn apply_retention(repo: &Repo, policy: &RetentionPolicy, allow_full_removal: bool) -> Result<usize, AppError> {
    let del: Vec<i64> = policy.plan(&retention_candidates(repo.db)?, Utc::now(), false)?.into_iter().filter(|v| !v.keep).map(|v| v.id).collect();
    if del.is_empty() {
        return Ok(0);
    }
    delete_fileset_ids(repo, &del, allow_full_removal, false)
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
        if repo.locked_until(name)?.is_some() {
            continue;
        }
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
                    serde_json::from_slice::<IndexVolume>(&volume::decode_meta(payload)?)?;
                }
                VolumeKind::FileList => {
                    serde_json::from_slice::<FileListVolume>(&volume::decode_meta(payload)?)?;
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
    let listing = repo.backend.list()?;
    let sizes: HashMap<&str, u64> = listing.iter().map(|f| (f.name.as_str(), f.size)).collect();
    db.clear()?;
    let mut r = RecreateReport::default();
    let mut indexed: HashSet<String> = HashSet::new();
    db.begin()?;
    for f in listing.iter().filter(|f| volume::classify(prefix, &f.name) == Some(VolumeKind::Index)) {
        ctx.check()?;
        let data = repo.backend.get(&f.name)?;
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
        let data = repo.backend.get(&f.name)?;
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
        let data = repo.backend.get(&f.name)?;
        match volume::open_json::<FileListVolume>(&repo.cipher, &data) {
            Ok(fl) => {
                db.add_volume(&f.name, VolumeKind::FileList, f.size, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
                db.add_fileset_with_meta(&fl.timestamp, &f.name, fl.partial, &fl.files, &fl.meta)?;
                super::engine::store_dlist_extra(db, &fl)?;
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
        let fl: FileListVolume = volume::open_json(&repo.cipher, &repo.backend.get(name)?)?;
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
                    repo.rewrite_filelist(fs, &repo.db.files(fs.id)?)?;
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
                    repo.note_write()?;
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

/// Result of [`rebuild_missing_block_volumes`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RebuildReport {
    pub volumes_rebuilt: usize,
    pub blocks_recovered: u64,
    pub blocks_unrecoverable: u64,
    /// Missing volumes whose blocks could not all be found locally.
    pub incomplete_volumes: Vec<String>,
}

/// Rebuild block volumes that are missing from the destination by reading
/// their blocks back from the source files still on this machine. Blocks
/// found are re-uploaded in new volumes; a missing volume is forgotten once
/// every one of its blocks is back.
pub fn rebuild_missing_block_volumes(repo: &Repo, opts: &BackupOptions, ctx: &OpContext) -> Result<RebuildReport, AppError> {
    use std::io::{Read, Seek, SeekFrom};
    let cons = repo.consistency_check()?;
    let by_volume = repo.db.blocks_by_volume()?;
    let missing: Vec<&String> = cons
        .missing
        .iter()
        .chain(cons.size_mismatch.iter())
        .filter(|n| volume::classify(&repo.prefix, n) == Some(VolumeKind::Block))
        .collect();
    let mut r = RebuildReport::default();
    if missing.is_empty() {
        return Ok(r);
    }
    let mut wanted: HashMap<String, &String> = HashMap::new();
    for v in &missing {
        for (h, _) in by_volume.get(*v).map(Vec::as_slice).unwrap_or_default() {
            wanted.insert(h.clone(), *v);
        }
    }
    let bs = repo.header.block_size.max(1);
    let mut found: HashSet<String> = HashSet::new();
    let mut pending = Pending::for_repo(repo, opts);
    let mut scratch = super::engine::BackupReport::default();
    let mut buf = vec![0u8; bs];
    // Newest versions first: their files most likely still match.
    'outer: for fs in repo.db.filesets()? {
        for f in repo.db.files(fs.id)? {
            ctx.check()?;
            if f.kind != EntryKind::File || !f.blocks.iter().any(|b| wanted.contains_key(b) && !found.contains(b)) {
                continue;
            }
            let Ok(mut file) = std::fs::File::open(&f.path) else { continue };
            if repo.header.chunker_spec().is_content_defined() {
                // Variable-size blocks: re-cut the whole file and keep matches.
                let chunker = repo.chunker();
                let mut res: Result<(), AppError> = Ok(());
                let _ = super::chunker::for_each_chunk(&mut std::io::BufReader::new(&mut file), chunker.as_ref(), |chunk| {
                    let h = hash_bytes(repo.header.block_hash, chunk);
                    if wanted.contains_key(&h) && !found.contains(&h) {
                        res = pending.add_block(h.clone(), chunk, opts.compression_level);
                        if res.is_ok() && pending.byte_size() >= opts.volume_size {
                            res = flush(repo, &mut pending, opts, &mut scratch);
                        }
                        found.insert(h);
                    }
                    Ok(res.is_ok())
                });
                res?;
                if found.len() == wanted.len() {
                    break 'outer;
                }
                continue;
            }
            for (i, b) in f.blocks.iter().enumerate() {
                if !wanted.contains_key(b) || found.contains(b) {
                    continue;
                }
                if file.seek(SeekFrom::Start((i * bs) as u64)).is_err() {
                    break;
                }
                let mut n = 0;
                while n < bs {
                    match file.read(&mut buf[n..]) {
                        Ok(0) | Err(_) => break,
                        Ok(k) => n += k,
                    }
                }
                if n > 0 && hash_bytes(repo.header.block_hash, &buf[..n]) == *b {
                    pending.add_block(b.clone(), &buf[..n], opts.compression_level)?;
                    found.insert(b.clone());
                    if pending.byte_size() >= opts.volume_size {
                        flush(repo, &mut pending, opts, &mut scratch)?;
                    }
                }
            }
            if found.len() == wanted.len() {
                break 'outer;
            }
        }
    }
    flush(repo, &mut pending, opts, &mut scratch)?;
    r.blocks_recovered = found.len() as u64;
    r.blocks_unrecoverable = (wanted.len() - found.len()) as u64;
    for v in missing {
        let complete = by_volume.get(v).map(Vec::as_slice).unwrap_or_default().iter().all(|(h, _)| found.contains(h));
        if complete {
            // Its blocks now point at the new volumes.
            delete_block_volume(repo, v)?;
            r.volumes_rebuilt += 1;
        } else {
            r.incomplete_volumes.push(v.clone());
        }
    }
    Ok(r)
}

/// Control files stored with a version (read from the remote file list).
pub fn control_files(repo: &Repo, version: usize) -> Result<Vec<volume::ControlFile>, AppError> {
    let fs = repo.db.fileset_by_version(version)?;
    let fl: FileListVolume = volume::open_json(&repo.cipher, &repo.get_volume(&fs.volume_name)?)?;
    Ok(fl.control_files)
}

/// Write a version's control files into `target`; returns the paths written.
pub fn extract_control_files(repo: &Repo, version: usize, target: &std::path::Path) -> Result<Vec<std::path::PathBuf>, AppError> {
    use base64::Engine;
    std::fs::create_dir_all(target)?;
    let mut out = Vec::new();
    for c in control_files(repo, version)? {
        // Names are plain file names; never let one escape the target.
        let name = std::path::Path::new(&c.name).file_name().map(|n| n.to_owned()).ok_or_else(|| AppError::validation(format!("Invalid control file name '{}'", c.name)))?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(&c.data)
            .map_err(|e| AppError::transfer(format!("Control file {} is damaged: {e}", c.name), "Run Verify."))?;
        let p = target.join(name);
        std::fs::write(&p, data)?;
        out.push(p);
    }
    Ok(out)
}

/// Result of [`apply_locks`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockReport {
    pub locked: usize,
    pub already_locked: usize,
    pub until: String,
    pub errors: Vec<String>,
}

/// Lock volumes (and their PAR2 files) for `days`. With `all = false` only
/// volumes without an active lock are touched (new uploads); `all = true`
/// extends every lock (existing-volume administration).
pub fn apply_locks(repo: &Repo, days: u32, mode: super::backend::LockMode, all: bool) -> Result<LockReport, AppError> {
    if !repo.backend.supports_lock() {
        return Err(AppError::validation(format!("{} does not support object locks", repo.backend.describe())));
    }
    let until = Utc::now() + chrono::Duration::days(days.max(1) as i64);
    let mut r = LockReport { until: until.to_rfc3339(), ..Default::default() };
    for v in repo.db.volumes(None)? {
        if !all && repo.locked_until(&v.name)?.is_some() {
            r.already_locked += 1;
            continue;
        }
        let mut names = vec![v.name.clone()];
        names.extend(repo.recovery_names(&v.name)?);
        match names.iter().try_for_each(|n| repo.backend.lock(n, until, mode)) {
            Ok(()) => {
                repo.db.set_volume_lock(&v.name, Some(&r.until))?;
                r.locked += 1;
            }
            Err(e) => r.errors.push(format!("{}: {}", v.name, e.user_message())),
        }
    }
    Ok(r)
}

/// Volumes with an active lock: (name, until).
pub fn list_locks(repo: &Repo) -> Result<Vec<(String, String)>, AppError> {
    let mut out = Vec::new();
    for v in repo.db.volumes(None)? {
        if let Some(u) = repo.locked_until(&v.name)? {
            out.push((v.name, u));
        }
    }
    Ok(out)
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
    repo.rewrite_filelist(fs, files)?;
    repo.db.replace_fileset_files(fs.id, &fs.volume_name, files)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PurgeReport {
    pub versions_rewritten: usize,
    pub entries_removed: usize,
    pub dry_run: bool,
    /// Pinned versions left unchanged (a purge never rewrites them).
    #[serde(default)]
    pub pinned_skipped: usize,
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
        if removed > 0 && fs.pinned {
            r.pinned_skipped += 1;
            continue;
        }
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

/// How the local database should be kept (see [`open_job_db`]).
#[derive(Debug, Clone, Default)]
pub struct JobDbOptions<'k> {
    /// The job's database key, when one is stored.
    pub key: Option<&'k super::db::DbKey>,
    /// Encrypt the database (`key` must be set) or keep it plain.
    pub encrypt: bool,
}

/// Open a job's local database the way the job wants it:
///
/// - encrypted with `key` when `encrypt` is set (a plain database is
///   migrated in place), plain otherwise (an encrypted one is decrypted);
/// - when the database is encrypted but the key is missing or wrong, the
///   file is set aside (`<name>.undecryptable-<time>`), a new database is
///   started and, when `backend` holds the repository, rebuilt from it.
///
/// Returns the database and notices for the run log.
pub fn open_job_db(
    path: &std::path::Path,
    opts: &JobDbOptions,
    backend: Option<&dyn Backend>,
    passphrase: Option<&str>,
    prefix: &str,
    ctx: &OpContext,
) -> Result<(BackupDb, Vec<String>), AppError> {
    use super::db::KeyCheck;
    if opts.encrypt && opts.key.is_none() {
        return Err(AppError::validation("Encrypting the local database needs a key"));
    }
    let mut notices = Vec::new();
    let check = BackupDb::key_check(path, opts.key)?;
    if matches!(check, KeyCheck::KeyMissing | KeyCheck::WrongKey) {
        let aside = std::path::PathBuf::from(format!("{}.undecryptable-{}", path.display(), Utc::now().format("%Y%m%dT%H%M%SZ")));
        std::fs::rename(path, &aside)?;
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        let why = if check == KeyCheck::KeyMissing { "its key is not in the keychain" } else { "the keychain holds a different key" };
        notices.push(format!("The local database is encrypted but {why}; it was set aside as {} and rebuilt from the destination", aside.display()));
        let mut db = BackupDb::open(path)?;
        if opts.encrypt {
            db.rekey(opts.key)?;
        }
        if let Some(b) = backend {
            let header = volume::repo_header_name(prefix);
            if b.list()?.iter().any(|f| f.name == header) {
                let r = recreate_database(b, &db, passphrase, prefix, ctx)?;
                notices.push(format!("Database rebuilt: {} version(s), {} block volume(s)", r.versions, r.block_volumes));
            }
        }
        return Ok((db, notices));
    }
    let mut db = BackupDb::open_with_key(path, opts.key)?;
    if opts.encrypt && !db.is_encrypted() {
        let had_data = !db.is_empty()?;
        db.rekey(opts.key)?;
        if had_data {
            notices.push("Encrypted the local database".into());
        }
    } else if !opts.encrypt && db.is_encrypted() {
        db.rekey(None)?;
        notices.push("Decrypted the local database (encryption was turned off for this job)".into());
    }
    Ok((db, notices))
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

    #[test]
    fn rebuild_missing_block_volume_from_sources() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 13 % 251) as u8).collect();
        write(src.path(), "a.bin", &data);
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        run(src.path(), &be, &db, &RetentionPolicy::KeepAll);
        let gone: Vec<_> = std::fs::read_dir(dst.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".dblock")).collect();
        assert!(!gone.is_empty());
        for g in &gone {
            std::fs::remove_file(g.path()).unwrap();
        }
        let repo = open(&be, &db);
        assert!(!list_broken_files(&repo).unwrap().is_empty());
        let r = rebuild_missing_block_volumes(&repo, &o(), &OpContext::new()).unwrap();
        assert_eq!(r.volumes_rebuilt, gone.len());
        assert_eq!(r.blocks_unrecoverable, 0);
        assert!(repo.consistency_check().unwrap().is_clean());
        assert!(list_broken_files(&repo).unwrap().is_empty());
        let rr = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert_eq!(std::fs::read(out.path().join(src.path().file_name().unwrap()).join("a.bin")).unwrap(), data);

        // Source changed and volume lost again: reported, not faked.
        write(src.path(), "a.bin", b"different");
        for e in std::fs::read_dir(dst.path()).unwrap().flatten() {
            if e.file_name().to_string_lossy().contains(".dblock") {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        let r = rebuild_missing_block_volumes(&repo, &o(), &OpContext::new()).unwrap();
        assert!(r.blocks_unrecoverable > 0 && !r.incomplete_volumes.is_empty());
    }

    #[test]
    fn control_files_extracted() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(src.path(), "f", b"1");
        write(out.path(), "in/notes.txt", b"hello control");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let opts = BackupOptions { control_files: vec![out.path().join("in/notes.txt").to_string_lossy().into_owned()], ..o() };
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &opts, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        let repo = open(&be, &db);
        let written = extract_control_files(&repo, 0, &out.path().join("x")).unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(std::fs::read(&written[0]).unwrap(), b"hello control");
    }

    /// Write-once test destination: refuses deleting/overwriting locked files.
    struct Worm {
        inner: LocalBackend,
        locks: std::sync::Mutex<HashMap<String, (DateTime<Utc>, crate::backup::backend::LockMode)>>,
    }
    impl crate::backup::Backend for Worm {
        fn describe(&self) -> String {
            "worm".into()
        }
        fn list(&self) -> Result<Vec<crate::backup::RemoteFile>, AppError> {
            self.inner.list()
        }
        fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> {
            if self.locks.lock().unwrap().contains_key(n) {
                return Err(AppError::state("locked", "x"));
            }
            self.inner.put(n, d)
        }
        fn get(&self, n: &str) -> Result<Vec<u8>, AppError> {
            self.inner.get(n)
        }
        fn delete(&self, n: &str) -> Result<(), AppError> {
            if self.locks.lock().unwrap().get(n).is_some_and(|(u, _)| *u > Utc::now()) {
                return Err(AppError::state(format!("{n} is locked"), "x"));
            }
            self.inner.delete(n)
        }
        fn create_folder(&self) -> Result<(), AppError> {
            self.inner.create_folder()
        }
        fn supports_lock(&self) -> bool {
            true
        }
        fn lock(&self, n: &str, u: DateTime<Utc>, m: crate::backup::backend::LockMode) -> Result<(), AppError> {
            self.locks.lock().unwrap().insert(n.to_string(), (u, m));
            Ok(())
        }
    }

    #[test]
    fn object_lock_protects_versions_from_retention_and_compaction() {
        use crate::backup::backend::LockMode;
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let be = Worm { inner: LocalBackend::new(dst.path()), locks: Default::default() };
        let db = BackupDb::open_in_memory().unwrap();
        let opts = BackupOptions { object_lock: Some(crate::backup::ObjectLockOptions { days: 30, mode: LockMode::Compliance }), ..o() };
        write(src.path(), "a", &vec![1u8; 3000]);
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &opts, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let n_vols = db.volumes(None).unwrap().len();
        assert_eq!(be.locks.lock().unwrap().len(), n_vols, "every volume locked");
        assert!(be.locks.lock().unwrap().values().all(|(_, m)| *m == LockMode::Compliance));
        tick();
        write(src.path(), "a", &vec![2u8; 3000]);
        // Keep only 1 version: the old one is locked and must survive.
        let r = run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &opts, &RetentionPolicy::KeepCount { count: 1 }, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.versions_deleted, 0);
        assert_eq!(db.filesets().unwrap().len(), 2);
        let repo = Repo::open(&be, &db, Some("pw"), &opts, false, KdfParams::insecure_fast()).unwrap();
        assert!(delete_versions(&repo, &[1], false).unwrap() == 0);
        assert_eq!(list_locks(&repo).unwrap().len(), db.volumes(None).unwrap().len());
        // Existing-volume administration: extend all locks.
        let ext = apply_locks(&repo, 60, LockMode::Governance, true).unwrap();
        assert_eq!(ext.locked, db.volumes(None).unwrap().len());
        // A destination without lock support is refused clearly.
        let plain = LocalBackend::new(dst.path());
        let repo2 = Repo::open(&plain, &db, Some("pw"), &opts, false, KdfParams::insecure_fast()).unwrap();
        assert!(apply_locks(&repo2, 1, LockMode::Governance, false).is_err());
    }

    #[test]
    fn job_database_follows_the_job_and_rebuilds_without_its_key() {
        use crate::backup::db::{DbKey, KeyCheck};
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        crate::backup::test_util::write(src.path(), "zzSecretFolder/a.txt", b"hello");
        let be = crate::backup::LocalBackend::new(dst.path());
        let path = data.path().join("job.sqlite");
        let key = DbKey::generate();
        let enc = JobDbOptions { key: Some(&key), encrypt: true };
        let o = BackupOptions { verify_samples: 0, ..Default::default() };
        // New job: an encrypted database from the start.
        let (db, notes) = open_job_db(&path, &enc, Some(&be), Some("pw"), "ufop", &OpContext::new()).unwrap();
        assert!(db.is_encrypted() && notes.is_empty());
        crate::backup::run_backup(&be, &db, &[src.path().into()], &Default::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        drop(db);
        assert_eq!(BackupDb::key_check(&path, Some(&key)).unwrap(), KeyCheck::Encrypted);

        // Key lost: set aside, rebuilt from the destination, encrypted with the new key.
        let new_key = DbKey::generate();
        let enc2 = JobDbOptions { key: Some(&new_key), encrypt: true };
        let (db, notes) = open_job_db(&path, &enc2, Some(&be), Some("pw"), "ufop", &OpContext::new()).unwrap();
        assert!(notes[0].contains("different key") && notes[1].contains("1 version"), "{notes:?}");
        assert!(db.is_encrypted());
        let files = db.files(db.filesets().unwrap()[0].id).unwrap();
        assert!(files.iter().any(|f| f.path.ends_with("a.txt")));
        drop(db);
        assert!(std::fs::read_dir(data.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(".undecryptable-")));
        // No key at all (keychain wiped) and encryption off: rebuilt plain.
        let (db, notes) = open_job_db(&path, &JobDbOptions::default(), Some(&be), Some("pw"), "ufop", &OpContext::new()).unwrap();
        assert!(notes[0].contains("not in the keychain") && !db.is_encrypted());
        assert_eq!(db.filesets().unwrap().len(), 1);
        drop(db);
        // Turning encryption on migrates the existing plain database.
        let (db, notes) = open_job_db(&path, &enc, None, None, "ufop", &OpContext::new()).unwrap();
        assert_eq!(notes, ["Encrypted the local database"]);
        assert_eq!(db.filesets().unwrap().len(), 1);
        drop(db);
        // And off again decrypts it.
        let (db, notes) = open_job_db(&path, &JobDbOptions { key: Some(&key), encrypt: false }, None, None, "ufop", &OpContext::new()).unwrap();
        assert!(notes[0].starts_with("Decrypted") && !db.is_encrypted());
        assert!(open_job_db(&path, &JobDbOptions { key: None, encrypt: true }, None, None, "ufop", &OpContext::new()).is_err());
    }
}
