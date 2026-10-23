//! Shared multi-client repositories: several jobs, on one computer or many,
//! write to the same destination prefix and deduplicate against each other.
//!
//! - The header carries the format 2 feature `shared_repository` (set when
//!   the repository is created with `shared_repository`, or later with
//!   [`enable_sharing`]). Versions that do not know it refuse the
//!   repository instead of misreading it.
//! - Every version records the [`ClientIdentity`] that wrote it; its source
//!   id groups versions into "sources". The local database is scoped to the
//!   job's own source ([`BackupDb::set_scope`]), so listing, restore,
//!   retention, pins and comparisons default to it. Other sources are
//!   visible through [`list_sources`] and can be targeted explicitly.
//! - Before every operation the local database is brought up to date from
//!   the destination ([`sync_from_remote`]): index volumes it has not seen
//!   (blocks uploaded by other clients, so they are not uploaded again),
//!   file lists it has not seen or that changed (other sources' versions),
//!   and volumes that disappeared (removed by another client's compaction).
//!   Known volume names live in `remote_volumes`, so this costs one listing
//!   when nothing changed.
//! - Writers are serialised by the exclusive repository lock (see
//!   [`super::lock`]). Compaction counts the blocks of every source's
//!   versions as used and leaves volumes younger than
//!   `shared_gc_delay_hours` alone, so a writer that uploaded blocks but
//!   not yet its file list (or crashed in between) is safe.

use super::backend::Backend;
use super::db::BackupDb;
use super::engine::Repo;
use super::versions::{rewrite_version_meta, ClientIdentity};
use super::volume::{self, FileListVolume, IndexVolume, RepoHeader, VolumeKind};
use super::{hash_bytes, format, HashAlgorithm, OpContext};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Default safety delay before compaction may delete a young volume.
pub const DEFAULT_GC_DELAY_HOURS: u32 = 4;
/// Database marker: the repository is shared (scope versions to a source).
pub const SHARED_KEY: &str = "shared_repository";

pub fn is_shared(h: &RepoHeader) -> bool {
    h.has_feature(format::FEATURE_SHARED)
}

/// Scope `db` to `identity`'s source when the database belongs to a shared
/// repository (no-op otherwise). Used where the database is opened without
/// the repository (listings, search).
pub fn scope_db(db: &BackupDb, identity: &ClientIdentity) -> Result<(), AppError> {
    if db.get_kv(SHARED_KEY)?.as_deref() == Some("1") {
        db.set_scope(Some(identity.resolve().source_id()));
    }
    Ok(())
}

/// What [`sync_from_remote`] changed in the local database.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub index_volumes_added: usize,
    pub blocks_added: usize,
    pub versions_added: usize,
    pub versions_updated: usize,
    pub versions_removed: usize,
    pub volumes_removed: usize,
    pub warnings: Vec<String>,
}

impl SyncReport {
    pub fn changed(&self) -> bool {
        self.index_volumes_added + self.versions_added + self.versions_updated + self.versions_removed + self.volumes_removed > 0
    }
}

/// Bring the local database up to date with the destination (incremental:
/// only volumes it does not know, or file lists whose size changed, are
/// downloaded).
pub fn sync_from_remote(repo: &Repo, ctx: &OpContext) -> Result<SyncReport, AppError> {
    let db = repo.db;
    let prefix = repo.prefix.as_str();
    let norm = |n: &str| if repo.case_insensitive { n.to_lowercase() } else { n.to_string() };
    let listing = repo.backend.list()?;
    let remote: HashMap<String, (&str, u64)> = listing
        .iter()
        .filter(|f| volume::classify(prefix, &f.name).is_some())
        .map(|f| (norm(&f.name), (f.name.as_str(), f.size)))
        .collect();
    let known = db.volumes(None)?;
    let known_names: HashMap<String, (VolumeKind, u64)> = known.iter().map(|v| (norm(&v.name), (v.kind, v.size))).collect();
    let mut r = SyncReport::default();
    db.begin()?;
    let res = (|| -> Result<(), AppError> {
        // 1. Volumes removed by another client (compaction, retention).
        for v in known.iter().filter(|v| v.state == "uploaded" || v.state == "verified") {
            if remote.contains_key(&norm(&v.name)) {
                continue;
            }
            match v.kind {
                VolumeKind::FileList => {
                    for fs in db.all_filesets()?.into_iter().filter(|f| f.volume_name == v.name) {
                        db.delete_fileset(fs.id)?;
                        r.versions_removed += 1;
                    }
                }
                VolumeKind::Block => {
                    db.delete_kv(&format!("index:{}", v.name))?;
                    r.volumes_removed += 1;
                }
                VolumeKind::Index => r.volumes_removed += 1,
            }
            db.delete_volume(&v.name)?;
        }
        // 2. New index volumes: blocks other clients uploaded.
        for (key, (name, size)) in &remote {
            if volume::classify(prefix, name) != Some(VolumeKind::Index) || known_names.contains_key(key) {
                continue;
            }
            ctx.check()?;
            let data = repo.backend.get(name)?;
            let idx: IndexVolume = match volume::open_json(&repo.cipher, &data) {
                Ok(i) => i,
                Err(e) => {
                    r.warnings.push(format!("{name}: {}", e.user_message()));
                    continue;
                }
            };
            let Some((bname, bsize)) = remote.get(&norm(&idx.block_volume)) else {
                // Its block volume is gone (or not uploaded yet): nothing to use.
                continue;
            };
            db.add_volume(name, VolumeKind::Index, *size, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
            let vid = match db.volume_by_name(bname)? {
                Some(v) => v.id,
                None => db.add_volume(bname, VolumeKind::Block, *bsize, "", "uploaded")?,
            };
            db.set_kv(&format!("index:{bname}"), name)?;
            for b in &idx.blocks {
                db.add_block(&b.hash, b.size, vid)?;
            }
            r.blocks_added += idx.blocks.len();
            r.index_volumes_added += 1;
        }
        // 3. File lists not seen yet, or rewritten (note, tags, pin).
        for (key, (name, size)) in &remote {
            if volume::classify(prefix, name) != Some(VolumeKind::FileList) {
                continue;
            }
            let existing = known_names.get(key);
            if existing.is_some_and(|(_, s)| s == size) {
                continue;
            }
            ctx.check()?;
            let data = repo.backend.get(name)?;
            let fl: FileListVolume = match volume::open_json(&repo.cipher, &data) {
                Ok(f) => f,
                Err(e) => {
                    r.warnings.push(format!("{name}: {}", e.user_message()));
                    continue;
                }
            };
            db.add_volume(name, VolumeKind::FileList, *size, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
            match db.all_filesets()?.into_iter().find(|f| f.volume_name == *name || f.timestamp == fl.timestamp) {
                Some(fs) => {
                    db.replace_fileset_files(fs.id, name, &fl.files)?;
                    db.set_fileset_meta(fs.id, &fl.meta)?;
                    r.versions_updated += 1;
                }
                None => {
                    db.add_fileset_with_meta(&fl.timestamp, name, fl.partial, &fl.files, &fl.meta)?;
                    r.versions_added += 1;
                }
            }
            super::engine::store_dlist_extra(db, &fl)?;
        }
        db.set_kv(SHARED_KEY, "1")
    })();
    match res {
        Ok(()) => db.commit()?,
        Err(e) => {
            let _ = db.rollback();
            return Err(e);
        }
    }
    if r.changed() {
        tracing::info!(
            "Shared repository sync: {} index volume(s), {} new / {} updated / {} removed version(s), {} volume(s) gone",
            r.index_volumes_added,
            r.versions_added,
            r.versions_updated,
            r.versions_removed,
            r.volumes_removed
        );
    }
    Ok(r)
}

/// Whether compaction may delete (or repack) volume `name` now: always in
/// a single-client repository, after the safety delay in a shared one.
/// The age counts from the later of when this database first saw the
/// volume and the destination's modification time.
pub fn old_enough(repo: &Repo, name: &str, delay_hours: u32, now: DateTime<Utc>) -> Result<bool, AppError> {
    if !is_shared(&repo.header) || delay_hours == 0 {
        return Ok(true);
    }
    let seen = repo.db.volume_created_at(name)?;
    let modified = repo.backend.modified(name).ok().flatten();
    let newest = match (seen, modified) {
        (Some(a), Some(b)) => a.max(b),
        (a, b) => a.or(b).unwrap_or(now),
    };
    Ok(now - newest >= chrono::Duration::hours(delay_hours as i64))
}

/// Result of [`enable_sharing`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharingReport {
    pub already_shared: bool,
    /// Existing versions assigned to the enabling job's source.
    pub versions_assigned: usize,
    /// The enabling job's source id.
    pub source: String,
}

/// Turn a single-job format 2 repository into a shared one. Existing
/// versions belong to the job that enables sharing: those without an
/// identity get its identity, and with `claim_all` every version does (a
/// single-job repository's history is that job's, even when it was written
/// on another computer). The file lists are rewritten first, the header
/// last.
pub fn enable_sharing(repo: &Repo, identity: &ClientIdentity, claim_all: bool) -> Result<SharingReport, AppError> {
    let identity = identity.resolve();
    let mut r = SharingReport { source: identity.source_id(), ..Default::default() };
    if is_shared(&repo.header) {
        r.already_shared = true;
        return Ok(r);
    }
    if repo.header.format_version < 2 {
        return Err(AppError::state("Sharing a repository needs format 2", "Upgrade the repository to format 2 first (Maintenance → Repository format)."));
    }
    let _all = repo.db.scoped(None);
    for fs in repo.db.all_filesets()? {
        let mine = fs.identity.as_ref().is_some_and(|i| i.source_id() == r.source);
        if mine || (fs.identity.is_some() && !claim_all) {
            continue;
        }
        let mut meta = fs.meta();
        meta.identity = Some(identity.clone());
        rewrite_version_meta(repo, &fs, &meta)?;
        r.versions_assigned += 1;
    }
    let inner = repo.backend.inner();
    let (mut h, _) = format::read_header(inner, &repo.prefix)?;
    h.features.push(format::FEATURE_SHARED.to_string());
    format::sync_features(&mut h);
    format::write_header(inner, &repo.prefix, &h)?;
    repo.db.set_kv(SHARED_KEY, "1")?;
    Ok(r)
}

/// One source of a repository (a job on a computer).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Source id ("" = versions without an identity).
    pub id: String,
    /// Identity of its newest version.
    pub identity: Option<ClientIdentity>,
    /// `user@host (label)`, or "Unknown source".
    pub label: String,
    pub versions: u64,
    pub newest: Option<String>,
    pub oldest: Option<String>,
    /// Size of the newest version.
    pub newest_size: u64,
    /// The asking job's own source.
    pub own: bool,
}

/// Every source with versions in the database, own first, then by label.
pub fn list_sources(db: &BackupDb, own: Option<&str>) -> Result<Vec<SourceInfo>, AppError> {
    let mut by: BTreeMap<String, SourceInfo> = BTreeMap::new();
    for fs in db.all_filesets()? {
        // Newest first: the first version seen sets identity and newest.
        let e = by.entry(fs.source.clone()).or_insert_with(|| SourceInfo {
            id: fs.source.clone(),
            label: fs.identity.as_ref().map(ClientIdentity::display).unwrap_or_else(|| "Unknown source".into()),
            identity: fs.identity.clone(),
            newest: Some(fs.timestamp.clone()),
            newest_size: fs.total_size,
            own: own == Some(fs.source.as_str()),
            ..Default::default()
        });
        e.versions += 1;
        e.oldest = Some(fs.timestamp.clone());
    }
    let mut v: Vec<SourceInfo> = by.into_values().collect();
    v.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| a.label.cmp(&b.label)));
    Ok(v)
}

/// Resolve a source selector: `None` = the scope already set, `"*"` or
/// `"all"` = every source, otherwise that source id.
pub fn selector_scope(selector: Option<&str>) -> Option<Option<String>> {
    match selector.map(str::trim) {
        None | Some("") => None,
        Some("*") | Some("all") => Some(None),
        Some(id) => Some(Some(id.to_string())),
    }
}

/// Is a backend's repository shared? (Reads only the header.)
pub fn repository_is_shared(backend: &dyn Backend, prefix: &str) -> Result<bool, AppError> {
    Ok(is_shared(&format::read_header(backend, prefix)?.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::lock::{with_lock, LockKind, LockOptions};
    use crate::backup::test_util::write;
    use crate::backup::{maintenance, run_backup, run_restore, BackupOptions, BackupReport, RestoreOptions, RetentionPolicy, SourceFilter};
    use std::path::Path;

    fn ident(host: &str, job: &str) -> ClientIdentity {
        ClientIdentity { hostname: host.into(), username: "u".into(), job_id: job.into(), source_label: job.into() }
    }

    fn o(id: &ClientIdentity, delay: u32) -> BackupOptions {
        BackupOptions {
            block_size: 1024,
            volume_size: 4096,
            verify_samples: 0,
            compact_threshold_percent: 10,
            shared_repository: true,
            shared_gc_delay_hours: delay,
            identity: id.clone(),
            ..Default::default()
        }
    }

    fn run(src: &Path, be: &LocalBackend, db: &BackupDb, o: &BackupOptions, ret: &RetentionPolicy) -> BackupReport {
        let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), o, ret, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        r
    }

    fn payload(seed: u32) -> Vec<u8> {
        (0..20_000u32).map(|i| (i.wrapping_add(seed.wrapping_mul(7919)).wrapping_mul(2654435761) >> 13) as u8).collect()
    }

    fn tick() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    #[test]
    fn two_jobs_share_one_repository() {
        let (sa, sb, dst, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(sa.path(), "same.bin", &payload(1));
        write(sb.path(), "same.bin", &payload(1));
        let be = LocalBackend::new(dst.path());
        let (ia, ib) = (ident("pc-a", "job-a"), ident("pc-b", "job-b"));
        let (oa, ob) = (o(&ia, 0), o(&ib, 0));
        let (da, dbb) = (BackupDb::open_in_memory().unwrap(), BackupDb::open_in_memory().unwrap());
        let ra = run(sa.path(), &be, &da, &oa, &RetentionPolicy::KeepAll);
        assert!(ra.new_blocks > 10);
        let (h, _) = format::read_header(&be, "ufop").unwrap();
        assert!(is_shared(&h), "created shared");

        // B finds A's blocks: nothing new but its index-free file list.
        let rb = run(sb.path(), &be, &dbb, &ob, &RetentionPolicy::KeepAll);
        assert_eq!((rb.new_blocks, rb.bytes_uploaded, rb.volumes_uploaded), (0, 0, 0), "identical data is not uploaded again");
        assert_eq!(dbb.filesets().unwrap().len(), 1, "B sees its own version only");
        assert_eq!(dbb.all_filesets().unwrap().len(), 2);
        let srcs = list_sources(&dbb, Some(&ib.source_id())).unwrap();
        assert_eq!(srcs.len(), 2);
        assert!(srcs[0].own && srcs[0].label == "u@pc-b (job-b)");

        // A writes two more versions, then keeps only one: B's version stays.
        for seed in [2, 3] {
            tick();
            write(sa.path(), "same.bin", &payload(seed));
            run(sa.path(), &be, &da, &oa, &RetentionPolicy::KeepAll);
        }
        tick();
        write(sa.path(), "extra.txt", b"x");
        let r = run(sa.path(), &be, &da, &oa, &RetentionPolicy::KeepCount { count: 1 });
        assert_eq!(r.versions_deleted, 3);
        assert!(r.volumes_compacted > 0, "A's old blocks were compacted away");
        assert_eq!(da.filesets().unwrap().len(), 1);
        assert_eq!(da.all_filesets().unwrap().len(), 2, "B's version survives A's retention");
        // A's compaction (inside that backup) kept the blocks of B's version.
        let repo_b = Repo::open(&be, &dbb, Some("pw"), &ob, false, KdfParams::insecure_fast()).unwrap();
        let rr = run_restore(&repo_b, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert_eq!(std::fs::read(out.path().join(sb.path().file_name().unwrap()).join("same.bin")).unwrap(), payload(1));
        assert!(maintenance::verify(&repo_b, None, true).unwrap().errors.is_empty());
        assert!(repo_b.consistency_check().unwrap().is_clean(), "B's database followed A's compaction");

        // B can restore A's newest version by naming the source.
        let out2 = tempfile::tempdir().unwrap();
        let ro = RestoreOptions { target: Some(out2.path().into()), source: Some(ia.source_id()), ..Default::default() };
        run_restore(&repo_b, &ro, &OpContext::new()).unwrap();
        assert_eq!(std::fs::read(out2.path().join(sa.path().file_name().unwrap()).join("extra.txt")).unwrap(), b"x");
        assert_eq!(dbb.filesets().unwrap().len(), 1, "scope restored after the restore");
    }

    #[test]
    fn safety_delay_keeps_young_volumes() {
        let (sa, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let be = LocalBackend::new(dst.path());
        let ia = ident("pc-a", "job-a");
        let db = BackupDb::open_in_memory().unwrap();
        write(sa.path(), "f.bin", &payload(7));
        run(sa.path(), &be, &db, &o(&ia, 4), &RetentionPolicy::KeepAll);
        tick();
        write(sa.path(), "f.bin", &payload(8));
        run(sa.path(), &be, &db, &o(&ia, 4), &RetentionPolicy::KeepCount { count: 1 });
        let repo = Repo::open(&be, &db, Some("pw"), &o(&ia, 4), false, KdfParams::insecure_fast()).unwrap();
        let before = be.list().unwrap().len();
        let c = maintenance::compact(&repo, &o(&ia, 4), &OpContext::new()).unwrap();
        assert_eq!((c.volumes_removed, c.volumes_rewritten), (0, 0));
        assert!(c.volumes_too_young > 0);
        assert_eq!(be.list().unwrap().len(), before);
        // Without the delay the unused volumes go.
        let c = maintenance::compact(&repo, &o(&ia, 0), &OpContext::new()).unwrap();
        assert!(c.volumes_removed + c.volumes_rewritten > 0);
        let now = Utc::now();
        assert!(old_enough(&repo, "anything", 0, now).unwrap());
    }

    #[test]
    fn writers_are_serialised_by_the_lock() {
        let dst = tempfile::tempdir().unwrap();
        let be = LocalBackend::new(dst.path());
        let lo = LockOptions { settle: std::time::Duration::from_millis(10), ..Default::default() };
        let (tx, rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|s| {
            let (be, lo) = (&be, &lo);
            s.spawn(move || {
                with_lock(be, "ufop", LockKind::Exclusive, "backup", lo, &OpContext::new(), || {
                    tx.send(()).unwrap();
                    go_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
            });
            rx.recv().unwrap();
            let e = with_lock(be, "ufop", LockKind::Exclusive, "backup", lo, &OpContext::new(), || Ok(())).unwrap_err();
            assert!(e.user_message().contains("in use"), "{e}");
            go_tx.send(()).unwrap();
        });
        assert!(with_lock(&be, "ufop", LockKind::Exclusive, "backup", &lo, &OpContext::new(), || Ok(())).is_ok());
    }

    #[test]
    fn enable_sharing_assigns_history_and_v1_is_refused() {
        let (src, dst, d1) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a", b"a");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let old = ident("old-pc", "job-x");
        let single = BackupOptions { shared_repository: false, ..o(&old, 0) };
        run(src.path(), &be, &db, &single, &RetentionPolicy::KeepAll);
        // Remove the identity of that version as if written before identities existed.
        let repo = Repo::open(&be, &db, Some("pw"), &single, false, KdfParams::insecure_fast()).unwrap();
        let fs = db.filesets().unwrap().remove(0);
        rewrite_version_meta(&repo, &fs, &crate::backup::versions::VersionMeta::default()).unwrap();
        tick();
        write(src.path(), "b", b"b");
        run(src.path(), &be, &db, &single, &RetentionPolicy::KeepAll);
        let me = ident("new-pc", "job-y");
        let repo = Repo::open(&be, &db, Some("pw"), &single, false, KdfParams::insecure_fast()).unwrap();
        let r = enable_sharing(&repo, &me, false).unwrap();
        assert_eq!(r.versions_assigned, 1, "only the version without an identity");
        assert!(repository_is_shared(&be, "ufop").unwrap());
        assert!(enable_sharing(&Repo::open(&be, &db, Some("pw"), &o(&me, 0), false, KdfParams::insecure_fast()).unwrap(), &me, true).unwrap().already_shared);
        let mine = o(&me, 0);
        let fresh = BackupDb::open_in_memory().unwrap();
        let _repo = Repo::open(&be, &fresh, Some("pw"), &mine, false, KdfParams::insecure_fast()).unwrap();
        assert_eq!(fresh.filesets().unwrap().len(), 1, "a fresh database syncs and scopes to the source");
        assert_eq!(list_sources(&fresh, Some(&me.source_id())).unwrap().len(), 2);

        // claim_all on another repository takes over every version.
        let be2 = LocalBackend::new(d1.path());
        let db2 = BackupDb::open_in_memory().unwrap();
        run(src.path(), &be2, &db2, &single, &RetentionPolicy::KeepAll);
        let repo2 = Repo::open(&be2, &db2, Some("pw"), &single, false, KdfParams::insecure_fast()).unwrap();
        assert_eq!(enable_sharing(&repo2, &me, true).unwrap().versions_assigned, 1);

        // Format 1 cannot be shared.
        let (d3, db3) = (tempfile::tempdir().unwrap(), BackupDb::open_in_memory().unwrap());
        let be3 = LocalBackend::new(d3.path());
        let v1 = BackupOptions { format_version: Some(1), ..single.clone() };
        run(src.path(), &be3, &db3, &v1, &RetentionPolicy::KeepAll);
        let repo3 = Repo::open(&be3, &db3, Some("pw"), &v1, false, KdfParams::insecure_fast()).unwrap();
        assert!(enable_sharing(&repo3, &me, true).is_err());
        let bad = BackupOptions { format_version: Some(1), ..o(&me, 0) };
        let d4 = tempfile::tempdir().unwrap();
        assert!(run_backup(&LocalBackend::new(d4.path()), &BackupDb::open_in_memory().unwrap(), &[src.path().into()], &SourceFilter::default(), &bad, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).is_err());
    }
}
