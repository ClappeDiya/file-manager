//! Verification with a scope and sampling (Kopia-style `snapshot verify`).
//!
//! Three levels, each including the one before:
//!
//! 1. **Structure** (always): every block referenced by the scoped versions
//!    is in the index, and every volume those blocks (and the versions' file
//!    lists) live in exists at the destination. Needs one listing, no downloads.
//! 2. **Volumes** ([`VerifyMode::SampleVolumes`], [`VerifyMode::SamplePercent`],
//!    [`VerifyMode::Full`]): download volumes, check their recorded SHA-256,
//!    decrypt and unpack them; `Full` takes every volume and also hashes each
//!    block.
//! 3. **Files** (`files_percent`): reconstruct that share of the scoped
//!    files from their blocks (download, decrypt, decompress) and compare
//!    each with its recorded file hash.
//!
//! The scope narrows everything to versions (selectors such as `latest`,
//! `0`, a timestamp) and paths (folders or wildcards); empty = everything.
//! Problems name the affected versions and paths. `max_errors` stops early.
//! Downloads never read the volume cache unless `use_cache` is set, so a
//! verification checks the destination itself.

use super::access::{open_file, BlockSource};
use super::backend::Backend;
use super::engine::Repo;
use super::restore::path_selected;
use super::select::VersionSelector;
use super::volume::{self, FileListVolume, IndexVolume, VolumeKind};
use super::{hash_bytes, EntryKind, FileEntry, FileHasher, OpContext};
use crate::error::AppError;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Which versions and paths to verify (both empty = the whole repository).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyScope {
    /// Version selectors: a number (0 = newest), `latest`, `oldest`, or a
    /// version's timestamp / file-list name.
    pub versions: Vec<String>,
    /// Paths (a folder includes everything below it) or wildcards.
    pub paths: Vec<String>,
}

impl VerifyScope {
    pub fn is_all(&self) -> bool {
        self.versions.is_empty() && self.paths.is_empty()
    }
}

/// How much data to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VerifyMode {
    /// Index and listing only: nothing is downloaded.
    Structure,
    /// Download this many volumes (the newest file list always among them).
    SampleVolumes { count: usize },
    /// Download this share of the volumes (1–100).
    SamplePercent { percent: u8 },
    /// Every volume, with every block's hash checked.
    Full,
}

impl Default for VerifyMode {
    fn default() -> Self {
        Self::SampleVolumes { count: 1 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyOptions {
    pub scope: VerifyScope,
    pub mode: VerifyMode,
    /// Reconstruct this share of the scoped files and check their hashes
    /// (`None`/0 = none, 100 = every file).
    pub files_percent: Option<u8>,
    /// Downloads in flight at once (0 = 4).
    pub parallelism: usize,
    /// Stop after this many problems (0 = check everything).
    pub max_errors: usize,
    /// Read volumes from the volume cache when they are there.
    pub use_cache: bool,
}


impl VerifyOptions {
    /// The older `verify(samples, full)` call: `full` checks every block,
    /// `samples = None` downloads every volume.
    pub fn legacy(samples: Option<usize>, full: bool) -> Self {
        let mode = match (full, samples) {
            (true, _) => VerifyMode::Full,
            (false, None) => VerifyMode::SamplePercent { percent: 100 },
            (false, Some(n)) => VerifyMode::SampleVolumes { count: n },
        };
        Self { mode, ..Default::default() }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if let VerifyMode::SamplePercent { percent } = self.mode {
            if !(1..=100).contains(&percent) {
                return Err(AppError::validation("The volume sample must be 1 to 100 percent"));
            }
        }
        if self.files_percent.is_some_and(|p| p > 100) {
            return Err(AppError::validation("The file sample must be 0 to 100 percent"));
        }
        for v in &self.scope.versions {
            VersionSelector::parse(v)?;
        }
        for p in self.scope.paths.iter().filter(|p| p.contains('*') || p.contains('?')) {
            glob::Pattern::new(&p.replace('\\', "/")).map_err(|e| AppError::validation(format!("Invalid wildcard '{p}': {e}")))?;
        }
        Ok(())
    }
}

/// One problem found, with what it affects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyProblem {
    /// `missing_block`, `missing_volume`, `volume`, `block_hash` or `file`.
    pub kind: String,
    pub volume: Option<String>,
    pub message: String,
    /// Affected versions (timestamps) and paths (at most [`MAX_AFFECTED`] listed).
    pub versions: Vec<String>,
    pub paths: Vec<String>,
}

/// Paths listed per problem.
pub const MAX_AFFECTED: usize = 20;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyReport {
    /// Volumes downloaded and checked.
    pub checked: usize,
    /// One line per problem (as before; details in `problems`).
    pub errors: Vec<String>,
    pub problems: Vec<VerifyProblem>,
    pub versions_checked: usize,
    /// Files in scope, and those reconstructed and hash-checked.
    pub files_in_scope: u64,
    pub files_checked: u64,
    /// Distinct blocks the scoped versions reference.
    pub blocks_referenced: u64,
    /// Volumes the scope depends on.
    pub volumes_in_scope: usize,
    /// Why the check stopped before the end (`max_errors`, cancel).
    pub stop_reason: Option<String>,
}

/// Where each block is used: versions and a few paths.
#[derive(Default)]
struct Usage {
    versions: BTreeSet<String>,
    paths: BTreeSet<String>,
}

impl Usage {
    fn add(&mut self, version: &str, path: &str) {
        self.versions.insert(version.to_string());
        if self.paths.len() < MAX_AFFECTED {
            self.paths.insert(path.to_string());
        }
    }
    fn merge(&mut self, o: &Usage) {
        self.versions.extend(o.versions.iter().cloned());
        for p in &o.paths {
            if self.paths.len() >= MAX_AFFECTED {
                break;
            }
            self.paths.insert(p.clone());
        }
    }
}

/// Collects problems and enforces `max_errors` across worker threads.
struct Problems {
    list: Mutex<Vec<VerifyProblem>>,
    max: usize,
    full: AtomicBool,
}

impl Problems {
    fn push(&self, p: VerifyProblem) {
        let mut l = self.list.lock().unwrap_or_else(|e| e.into_inner());
        if self.max > 0 && l.len() >= self.max {
            self.full.store(true, Ordering::SeqCst);
            return;
        }
        l.push(p);
        if self.max > 0 && l.len() >= self.max {
            self.full.store(true, Ordering::SeqCst);
        }
    }
    fn stop(&self) -> bool {
        self.full.load(Ordering::SeqCst)
    }
}

fn problem(kind: &str, volume: Option<&str>, message: String, usage: Option<&Usage>) -> VerifyProblem {
    VerifyProblem {
        kind: kind.into(),
        volume: volume.map(String::from),
        message,
        versions: usage.map(|u| u.versions.iter().cloned().collect()).unwrap_or_default(),
        paths: usage.map(|u| u.paths.iter().cloned().collect()).unwrap_or_default(),
    }
}

/// Verify `repo` as `opts` say. `ctx` cancels between items.
pub fn verify(repo: &Repo, opts: &VerifyOptions, ctx: &OpContext) -> Result<VerifyReport, AppError> {
    opts.validate()?;
    let mut r = VerifyReport::default();
    let problems = Problems { list: Mutex::default(), max: opts.max_errors, full: AtomicBool::new(false) };

    // ── Scope: versions and their files ──
    let all = repo.db.filesets()?;
    let mut chosen: Vec<usize> = if opts.scope.versions.is_empty() {
        (0..all.len()).collect()
    } else {
        opts.scope.versions.iter().map(|s| VersionSelector::parse(s)?.resolve(&all)).collect::<Result<_, _>>()?
    };
    chosen.sort_unstable();
    chosen.dedup();
    r.versions_checked = chosen.len();
    let mut usage: HashMap<String, Usage> = HashMap::new();
    let mut files: Vec<(String, FileEntry)> = Vec::new();
    let mut lists: Vec<(String, String)> = Vec::new();
    for &i in &chosen {
        let fs = &all[i];
        lists.push((fs.volume_name.clone(), fs.timestamp.clone()));
        for f in repo.db.files(fs.id)? {
            if f.kind != EntryKind::File || !path_selected(&f.path, &opts.scope.paths) {
                continue;
            }
            for b in &f.blocks {
                usage.entry(b.clone()).or_default().add(&fs.timestamp, &f.path);
            }
            files.push((fs.timestamp.clone(), f));
        }
    }
    r.files_in_scope = files.len() as u64;
    r.blocks_referenced = usage.len() as u64;

    // ── Structure: index and destination listing ──
    let locations = repo.db.block_locations()?;
    let norm = |n: &str| if repo.case_insensitive { n.to_lowercase() } else { n.to_string() };
    let remote: HashSet<String> = repo.backend.list()?.into_iter().map(|f| norm(&f.name)).collect();
    let mut by_volume: HashMap<String, Usage> = HashMap::new();
    let mut unindexed: Vec<&String> = Vec::new();
    for (b, u) in &usage {
        match locations.get(b) {
            Some(v) => by_volume.entry(v.clone()).or_default().merge(u),
            None => unindexed.push(b),
        }
    }
    unindexed.sort();
    for b in unindexed {
        problems.push(problem("missing_block", None, format!("data block {} is not in the index", &b[..12.min(b.len())]), usage.get(b)));
    }
    let mut needed: Vec<(String, VolumeKind)> = by_volume.keys().map(|v| (v.clone(), VolumeKind::Block)).collect();
    for (name, ts) in &lists {
        let mut u = Usage::default();
        u.versions.insert(ts.clone());
        by_volume.entry(name.clone()).or_default().merge(&u);
        needed.push((name.clone(), VolumeKind::FileList));
    }
    if opts.scope.is_all() {
        // The whole repository: index volumes (and unreferenced ones) too.
        let known: HashSet<String> = needed.iter().map(|(n, _)| n.clone()).collect();
        for v in repo.db.volumes(None)? {
            if !known.contains(&v.name) && (v.state == "uploaded" || v.state == "verified") {
                needed.push((v.name, v.kind));
            }
        }
    }
    needed.sort_by(|a, b| a.0.cmp(&b.0));
    r.volumes_in_scope = needed.len();
    let mut present: Vec<(String, VolumeKind)> = Vec::new();
    for (name, kind) in needed {
        if remote.contains(&norm(&name)) {
            present.push((name, kind));
        } else {
            problems.push(problem("missing_volume", Some(&name), format!("{name} is missing from the destination"), by_volume.get(&name)));
        }
    }

    // ── Volumes: download a sample (or all) ──
    let mut sample = present;
    match opts.mode {
        VerifyMode::Structure => sample.clear(),
        VerifyMode::Full => {}
        VerifyMode::SampleVolumes { count } => pick(&mut sample, &lists, count),
        VerifyMode::SamplePercent { percent } => {
            let n = (sample.len() * percent as usize).div_ceil(100);
            pick(&mut sample, &lists, n);
        }
    }
    let full = opts.mode == VerifyMode::Full;
    let threads = if opts.parallelism == 0 { 4 } else { opts.parallelism.min(64) };
    let mut infos = Vec::with_capacity(sample.len());
    for (name, kind) in sample {
        let (hash, rec) = repo.volume_check_info(&name)?;
        infos.push((name, kind, hash, rec));
    }
    let fetcher = repo.fetcher_with(opts.use_cache);
    let (cipher, block_hash) = (&repo.cipher, repo.header.block_hash);
    let next = AtomicUsize::new(0);
    let verified: Mutex<Vec<String>> = Mutex::default();
    std::thread::scope(|s| {
        for _ in 0..threads.min(infos.len().max(1)) {
            s.spawn(|| loop {
                if problems.stop() || ctx.is_cancelled() {
                    break;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((name, kind, hash, rec)) = infos.get(i) else { break };
                let res = (|| -> Result<Option<(String, String)>, AppError> {
                    let payload = cipher.open(&fetcher.fetch(name, hash.as_deref(), rec, false)?)?;
                    match kind {
                        VolumeKind::Block => {
                            for (h, d) in volume::read_block_volume(&payload)? {
                                if full && hash_bytes(block_hash, &d) != h {
                                    let msg = format!("block {} in {name} fails its hash check", &h[..12.min(h.len())]);
                                    return Ok(Some((h, msg)));
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
                    Ok(None)
                })();
                match res {
                    Ok(None) => verified.lock().unwrap_or_else(|e| e.into_inner()).push(name.clone()),
                    Ok(Some((block, msg))) => {
                        let u = usage.get(&block).or_else(|| by_volume.get(name));
                        problems.push(problem("block_hash", Some(name), msg, u));
                    }
                    Err(e) => problems.push(problem("volume", Some(name), format!("{name}: {}", e.user_message()), by_volume.get(name))),
                }
            });
        }
    });
    let verified = verified.into_inner().unwrap_or_else(|e| e.into_inner());
    r.checked = infos.len().min(next.load(Ordering::SeqCst));
    for v in &verified {
        repo.db.set_volume_state(v, "verified")?;
    }

    // ── Files: reconstruct a share and check their hashes ──
    let pct = opts.files_percent.unwrap_or(0) as usize;
    if pct > 0 && !problems.stop() && !ctx.is_cancelled() {
        files.retain(|(_, f)| f.hash.is_some());
        files.shuffle(&mut rand::thread_rng());
        files.truncate((files.len() * pct).div_ceil(100));
        let src = BlockSource::new(repo)?;
        let src = if opts.use_cache { src } else { src.without_cache() };
        let alg = repo.header.file_hash;
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..threads.min(files.len().max(1)) {
                s.spawn(|| loop {
                    if problems.stop() || ctx.is_cancelled() {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some((ver, f)) = files.get(i) else { break };
                    let res = (|| -> Result<bool, AppError> {
                        let mut rd = open_file(&src, f)?;
                        let mut h = FileHasher::new(alg);
                        let mut buf = vec![0u8; 256 * 1024];
                        loop {
                            let n = rd.read(&mut buf)?;
                            if n == 0 {
                                break;
                            }
                            h.update(&buf[..n]);
                        }
                        Ok(f.hash.as_deref() == Some(h.finish().as_str()))
                    })();
                    done.fetch_add(1, Ordering::SeqCst);
                    let mut u = Usage::default();
                    u.add(ver, &f.path);
                    match res {
                        Ok(true) => {}
                        Ok(false) => problems.push(problem("file", None, format!("{}: content does not match its recorded hash", f.path), Some(&u))),
                        Err(e) => problems.push(problem("file", None, format!("{}: {}", f.path, e.user_message()), Some(&u))),
                    }
                });
            }
        });
        r.files_checked = done.load(Ordering::SeqCst) as u64;
    }

    r.problems = problems.list.into_inner().unwrap_or_else(|e| e.into_inner());
    r.errors = r.problems.iter().map(|p| p.message.clone()).collect();
    if problems.full.load(Ordering::SeqCst) {
        r.stop_reason = Some(format!("Stopped after {} problem(s) (max errors)", opts.max_errors));
    } else if ctx.is_cancelled() {
        r.stop_reason = Some("Cancelled".into());
    }
    Ok(r)
}

/// Shuffle and keep `n`, the newest scoped file list first.
fn pick(v: &mut Vec<(String, VolumeKind)>, lists: &[(String, String)], n: usize) {
    v.shuffle(&mut rand::thread_rng());
    let newest = lists.first().map(|(n, _)| n.clone());
    v.sort_by_key(|(name, _)| Some(name) != newest.as_ref());
    v.truncate(n);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::db::BackupDb;
    use crate::backup::retention::RetentionPolicy;
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, BackupOptions, SourceFilter};
    use std::path::Path;

    fn o() -> BackupOptions {
        BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, quota_disable: true, ..Default::default() }
    }

    fn run(src: &Path, be: &LocalBackend, db: &BackupDb) {
        let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), &o(), &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    fn open<'a>(be: &'a LocalBackend, db: &'a BackupDb) -> Repo<'a> {
        Repo::open(be, db, Some("pw"), &o(), false, KdfParams::insecure_fast()).unwrap()
    }

    /// Two versions: `docs/a.txt` in both, `photos/p.bin` added in the second.
    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, BackupDb) {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let db = BackupDb::open_in_memory().unwrap();
        write(src.path(), "docs/a.txt", &(0..3000u32).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let be = LocalBackend::new(dst.path());
        run(src.path(), &be, &db);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(src.path(), "photos/p.bin", &(0..9000u32).map(|i| (i * 13 % 239) as u8).collect::<Vec<_>>());
        run(src.path(), &be, &db);
        (src, dst, db)
    }

    /// Volume holding the first block of the file at `path` (newest version).
    fn volume_of(db: &BackupDb, path_end: &str) -> String {
        let fs = &db.filesets().unwrap()[0];
        let f = db.files(fs.id).unwrap().into_iter().find(|f| f.path.ends_with(path_end)).unwrap();
        db.block_locations().unwrap()[&f.blocks[0]].clone()
    }

    fn corrupt(dst: &Path, name: &str) {
        let p = dst.join(name);
        let mut d = std::fs::read(&p).unwrap();
        let n = d.len();
        for b in d.iter_mut().skip(n / 2).take(16) {
            *b ^= 0x5a;
        }
        std::fs::write(&p, &d).unwrap();
    }

    #[test]
    fn clean_repository_passes_every_level() {
        let (_s, dst, db) = fixture();
        let be = LocalBackend::new(dst.path());
        let repo = open(&be, &db);
        let opts = VerifyOptions { mode: VerifyMode::Full, files_percent: Some(100), parallelism: 3, ..Default::default() };
        let r = verify(&repo, &opts, &OpContext::new()).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(r.versions_checked, 2);
        assert_eq!(r.files_in_scope, 3);
        assert_eq!(r.files_checked, 3);
        assert!(r.checked >= 3 && r.checked == r.volumes_in_scope);
        // Structure only downloads nothing.
        let r = verify(&repo, &VerifyOptions { mode: VerifyMode::Structure, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.checked, 0);
        assert!(r.problems.is_empty());
    }

    #[test]
    fn corrupted_block_found_only_when_sampled() {
        let (_s, dst, db) = fixture();
        let vol = volume_of(&db, "photos/p.bin");
        corrupt(dst.path(), &vol);
        let be = LocalBackend::new(dst.path());
        let repo = open(&be, &db);
        // Structure is fine: the volume exists.
        let s = verify(&repo, &VerifyOptions { mode: VerifyMode::Structure, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(s.problems.is_empty());
        // Scoped to docs: the damaged volume is not sampled.
        let docs = VerifyOptions { mode: VerifyMode::Full, scope: VerifyScope { paths: vec!["*/docs/*".into()], ..Default::default() }, files_percent: Some(100), ..Default::default() };
        let r = verify(&repo, &docs, &OpContext::new()).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(r.files_in_scope, 2, "a.txt in both versions");
        // File sampling of photos reconstructs the file and finds it.
        let photos = VerifyOptions { mode: VerifyMode::Structure, scope: VerifyScope { paths: vec!["*/photos".into(), "*photos/p.bin".into()], ..Default::default() }, files_percent: Some(100), ..Default::default() };
        let r = verify(&repo, &photos, &OpContext::new()).unwrap();
        assert_eq!(r.files_checked, 1);
        assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
        assert_eq!(r.problems[0].kind, "file");
        assert!(r.problems[0].paths[0].ends_with("photos/p.bin"));
        // Volume sampling of everything finds it, naming the version and path.
        let r = verify(&repo, &VerifyOptions { mode: VerifyMode::SamplePercent { percent: 100 }, ..Default::default() }, &OpContext::new()).unwrap();
        let p = r.problems.iter().find(|p| p.volume.as_deref() == Some(vol.as_str())).expect("damaged volume reported");
        assert_eq!(p.kind, "volume");
        assert_eq!(p.versions, vec![db.filesets().unwrap()[0].timestamp.clone()]);
        assert!(p.paths.iter().any(|x| x.ends_with("photos/p.bin")));
    }

    #[test]
    fn version_scope_missing_volume_and_max_errors() {
        let (_s, dst, db) = fixture();
        let be = LocalBackend::new(dst.path());
        // The oldest version only: fewer files and volumes.
        let repo = open(&be, &db);
        let r = verify(&repo, &VerifyOptions { scope: VerifyScope { versions: vec!["oldest".into()], ..Default::default() }, mode: VerifyMode::Structure, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!((r.versions_checked, r.files_in_scope), (1, 1));
        assert!(verify(&repo, &VerifyOptions { scope: VerifyScope { versions: vec!["7".into()], ..Default::default() }, ..Default::default() }, &OpContext::new()).is_err());
        // Delete the photo's block volumes and the newest file list: structure reports them missing.
        let fs = &db.filesets().unwrap()[0];
        let f = db.files(fs.id).unwrap().into_iter().find(|f| f.path.ends_with("photos/p.bin")).unwrap();
        let loc = db.block_locations().unwrap();
        let mut vols: BTreeSet<String> = f.blocks.iter().map(|b| loc[b].clone()).collect();
        // And the newest version's file list.
        vols.insert(fs.volume_name.clone());
        assert!(vols.len() >= 2, "{vols:?}");
        for v in &vols {
            be.delete(v).unwrap();
        }
        let r = verify(&repo, &VerifyOptions { mode: VerifyMode::Structure, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.problems.iter().filter(|p| p.kind == "missing_volume").count(), vols.len());
        assert!(r.stop_reason.is_none());
        // max_errors stops after the first.
        let r = verify(&repo, &VerifyOptions { mode: VerifyMode::Structure, max_errors: 1, ..Default::default() }, &OpContext::new()).unwrap();
        assert_eq!(r.problems.len(), 1);
        assert!(r.stop_reason.unwrap().contains("max errors"));
        // The older entry point still works (and reports).
        assert!(!crate::backup::maintenance::verify(&repo, None, false).unwrap().errors.is_empty());
    }

    #[test]
    fn options_validate_and_map_the_older_call() {
        assert_eq!(VerifyOptions::legacy(None, true).mode, VerifyMode::Full);
        assert_eq!(VerifyOptions::legacy(None, false).mode, VerifyMode::SamplePercent { percent: 100 });
        assert_eq!(VerifyOptions::legacy(Some(3), false).mode, VerifyMode::SampleVolumes { count: 3 });
        assert!(VerifyOptions { mode: VerifyMode::SamplePercent { percent: 0 }, ..Default::default() }.validate().is_err());
        assert!(VerifyOptions { files_percent: Some(101), ..Default::default() }.validate().is_err());
        assert!(VerifyOptions { scope: VerifyScope { paths: vec!["[".into()], ..Default::default() }, ..Default::default() }.validate().is_ok(), "no wildcard chars: a plain path");
        assert!(VerifyOptions { scope: VerifyScope { paths: vec!["a[*".into()], ..Default::default() }, ..Default::default() }.validate().is_err());
        let j: VerifyOptions = serde_json::from_str(r#"{"mode":{"type":"sample_percent","percent":10},"files_percent":5}"#).unwrap();
        assert_eq!(j.mode, VerifyMode::SamplePercent { percent: 10 });
        assert_eq!(j.files_percent, Some(5));
    }
}
