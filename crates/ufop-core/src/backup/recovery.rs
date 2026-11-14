//! Recovery tool: salvage data from a damaged or partially lost backup
//! without its local database, the way Duplicati's RecoveryTool does.
//!
//! 1. [`download`] copies every remote volume into a local folder,
//!    decrypted (parallel, resumable, optionally rate-limited). Damaged
//!    volumes are reported and skipped.
//! 2. [`build_index`] scans the downloaded block volumes directly, so the
//!    tool works even when index volumes or the database are lost.
//! 3. [`list_versions`] / [`restore`] rebuild files from the local copy. A
//!    file with missing blocks is still written (gaps zero-filled) and
//!    reported as partial, so as much data as possible is recovered.
//! 4. [`convert`] rewrites the copy into a new repository with another
//!    container format, compression or passphrase.
//!
//! The local folder holds decrypted data: keep it somewhere private.

use super::backend::Backend;
use super::crypto::KdfParams;
use super::engine::header_cipher;
use super::layout::LayoutBackend;
use super::restore::{destinations, path_selected, RestoreOptions};
use super::volume::{self, BlockVolumeWriter, FileListVolume, IndexVolume, IndexedBlock, RepoHeader, VolumeKind};
use super::{hash_bytes, EntryKind, FileEntry, OpContext, Phase, Progress, VolumeFormat, ZipMethod};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const HEADER_FILE: &str = "repo.json";
const INDEX_FILE: &str = "recovery-index.json";
const PREFIX_FILE: &str = "prefix.txt";
/// Block hash → size, for placing variable-size (content-defined) blocks.
const SIZES_FILE: &str = "recovery-sizes.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadOptions {
    /// Parallel downloads (0 = 4).
    pub threads: usize,
    /// Skip volumes already in the folder (continue an interrupted run).
    pub resume: bool,
    /// Download limit in bytes per second (0 = unlimited).
    pub max_bytes_per_sec: u64,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self { threads: 0, resume: true, max_bytes_per_sec: 0 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadReport {
    pub downloaded: usize,
    pub skipped_existing: usize,
    pub bytes: u64,
    /// `name: reason` for volumes that could not be downloaded or decrypted.
    pub failed: Vec<String>,
}

fn header_of(dir: &Path) -> Result<RepoHeader, AppError> {
    let p = dir.join(HEADER_FILE);
    let d = std::fs::read(&p).map_err(|_| AppError::file_op(format!("{} has no {HEADER_FILE}", dir.display()), "Run the download step first."))?;
    Ok(serde_json::from_slice(&d)?)
}

/// Download and decrypt every volume of `prefix` into `dir`.
pub fn download(backend: &dyn Backend, passphrase: Option<&str>, prefix: &str, dir: &Path, opts: &DownloadOptions, ctx: &OpContext) -> Result<DownloadReport, AppError> {
    std::fs::create_dir_all(dir)?;
    let listing = backend.list()?;
    let hname = volume::repo_header_name(prefix);
    if !listing.iter().any(|f| f.name == hname) {
        return Err(AppError::file_op(format!("No backup with prefix '{prefix}' at {}", backend.describe()), "Check the destination and prefix."));
    }
    let raw = backend.get(&hname)?;
    let header: RepoHeader = serde_json::from_slice(&raw)?;
    let cipher = header_cipher(&header, passphrase, None)?;
    // Volumes under their logical names, wherever the layout stores them.
    let layout = LayoutBackend::new(backend, header.layout, prefix);
    let backend: &dyn Backend = &layout;
    let listing = if header.layout.is_sharded() { backend.list()? } else { listing };
    std::fs::write(dir.join(HEADER_FILE), &raw)?;
    std::fs::write(dir.join(PREFIX_FILE), prefix)?;

    let mut todo: Vec<(String, u64)> = Vec::new();
    let mut r = DownloadReport::default();
    for f in listing.into_iter().filter(|f| volume::classify(prefix, &f.name).is_some()) {
        if opts.resume && dir.join(&f.name).is_file() {
            r.skipped_existing += 1;
        } else {
            todo.push((f.name, f.size));
        }
    }
    let total = todo.len() as u64;
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let bytes = AtomicU64::new(0);
    let failed = Mutex::new(Vec::new());
    let started = Instant::now();
    let threads = if opts.threads == 0 { 4 } else { opts.threads.min(32) };
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                if ctx.is_cancelled() {
                    return;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((name, _)) = todo.get(i) else { return };
                let res = backend.get(name).and_then(|d| {
                    bytes.fetch_add(d.len() as u64, Ordering::SeqCst);
                    cipher.open(&d)
                }).and_then(|p| match volume::classify(prefix, name) {
                    // Metadata is stored as plain JSON locally (v2 compresses it remotely).
                    Some(VolumeKind::Index | VolumeKind::FileList) => volume::decode_meta(p),
                    _ => Ok(p),
                });
                match res {
                    Ok(plain) => {
                        // Write under a temp name so an interrupted run never leaves a half file.
                        let tmp = dir.join(format!(".{name}.part"));
                        let ok = std::fs::write(&tmp, &plain).and_then(|_| std::fs::rename(&tmp, dir.join(name)));
                        if let Err(e) = ok {
                            failed.lock().map(|mut f| f.push(format!("{name}: {e}"))).ok();
                        }
                    }
                    Err(e) => {
                        failed.lock().map(|mut f| f.push(format!("{name}: {}", e.user_message()))).ok();
                    }
                }
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                ctx.report(&Progress { phase: Phase::Downloading, files_processed: n, files_total: total, bytes_processed: bytes.load(Ordering::SeqCst), current_path: Some(name.clone()) });
                if opts.max_bytes_per_sec > 0 {
                    // Pace the whole run to the limit.
                    let want = Duration::from_secs_f64(bytes.load(Ordering::SeqCst) as f64 / opts.max_bytes_per_sec as f64);
                    if let Some(wait) = want.checked_sub(started.elapsed()) {
                        std::thread::sleep(wait);
                    }
                }
            });
        }
    });
    ctx.check()?;
    r.failed = failed.into_inner().unwrap_or_default();
    r.downloaded = total as usize - r.failed.len();
    r.bytes = bytes.into_inner();
    Ok(r)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexReport {
    pub block_volumes: usize,
    pub blocks: usize,
    /// Block volumes that could not be read.
    pub damaged: Vec<String>,
}

fn local_volumes(dir: &Path, kind: VolumeKind) -> Result<Vec<String>, AppError> {
    let prefix = std::fs::read_to_string(dir.join(PREFIX_FILE)).map_err(|_| AppError::file_op(format!("{} is not a recovery folder", dir.display()), "Run the download step first."))?;
    let mut v: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| !n.starts_with('.') && volume::classify(prefix.trim(), n) == Some(kind))
        .collect();
    v.sort();
    Ok(v)
}

/// Map every block hash to the local block volume that holds it.
pub fn build_index(dir: &Path, ctx: &OpContext) -> Result<IndexReport, AppError> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    let mut r = IndexReport::default();
    let vols = local_volumes(dir, VolumeKind::Block)?;
    for (i, v) in vols.iter().enumerate() {
        ctx.check()?;
        ctx.report(&Progress { phase: Phase::Verifying, files_processed: i as u64, files_total: vols.len() as u64, bytes_processed: 0, current_path: Some(v.clone()) });
        match std::fs::read(dir.join(v)).map_err(AppError::from).and_then(|d| volume::read_block_volume(&d)) {
            Ok(blocks) => {
                r.block_volumes += 1;
                for (h, d) in blocks {
                    sizes.insert(h.clone(), d.len() as u64);
                    map.insert(h, v.clone());
                }
            }
            Err(e) => r.damaged.push(format!("{v}: {}", e.user_message())),
        }
    }
    r.blocks = map.len();
    std::fs::write(dir.join(INDEX_FILE), serde_json::to_vec(&map)?)?;
    std::fs::write(dir.join(SIZES_FILE), serde_json::to_vec(&sizes)?)?;
    Ok(r)
}

fn load_index(dir: &Path) -> Result<HashMap<String, String>, AppError> {
    let d = std::fs::read(dir.join(INDEX_FILE)).map_err(|_| AppError::file_op("No recovery index yet", "Run the index step first."))?;
    Ok(serde_json::from_slice(&d)?)
}

/// A version found in the local copy (0 = newest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryVersion {
    pub version: usize,
    pub timestamp: String,
    pub partial: bool,
    pub files: usize,
    pub volume: String,
}

fn filelists(dir: &Path) -> Result<Vec<(String, FileListVolume)>, AppError> {
    let mut out = Vec::new();
    for v in local_volumes(dir, VolumeKind::FileList)? {
        if let Ok(fl) = std::fs::read(dir.join(&v)).map_err(AppError::from).and_then(|d| Ok(serde_json::from_slice::<FileListVolume>(&d)?)) {
            out.push((v, fl));
        }
    }
    out.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
    Ok(out)
}

pub fn list_versions(dir: &Path) -> Result<Vec<RecoveryVersion>, AppError> {
    Ok(filelists(dir)?
        .into_iter()
        .enumerate()
        .map(|(i, (v, fl))| RecoveryVersion { version: i, timestamp: fl.timestamp, partial: fl.partial, files: fl.files.len(), volume: v })
        .collect())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryRestoreReport {
    pub version: String,
    pub files_restored: u64,
    /// Written with zero-filled gaps where blocks are missing.
    pub files_partial: Vec<String>,
    pub folders_created: u64,
    pub bytes_written: u64,
    pub errors: Vec<String>,
}

/// Rebuild `paths` (empty = all) of `version` below `target`.
pub fn restore(dir: &Path, version: usize, paths: &[String], target: &Path, ctx: &OpContext) -> Result<RecoveryRestoreReport, AppError> {
    let header = header_of(dir)?;
    let index = load_index(dir)?;
    let (_, fl) = filelists(dir)?.into_iter().nth(version).ok_or_else(|| AppError::not_found(format!("Version {version} is not in the recovered data")))?;
    let mut r = RecoveryRestoreReport { version: fl.timestamp.clone(), ..Default::default() };
    let entries: Vec<FileEntry> = fl.files.into_iter().filter(|e| path_selected(&e.path, paths)).collect();
    let ro = RestoreOptions { target: Some(target.to_path_buf()), ..Default::default() };
    let dests = destinations(&entries, &ro);
    let bs = header.block_size as u64;
    // Content-defined blocks vary in size: offsets come from the block sizes.
    let sizes: Option<HashMap<String, u64>> = if header.chunker_spec().is_content_defined() {
        let d = std::fs::read(dir.join(SIZES_FILE)).map_err(|_| AppError::file_op("The recovery index has no block sizes", "Run the index step again."))?;
        Some(serde_json::from_slice(&d)?)
    } else {
        None
    };

    // Create folders and empty files; collect writes per volume.
    let mut writes: HashMap<&str, Vec<(usize, u64, &str)>> = HashMap::new();
    let mut missing: HashMap<usize, usize> = HashMap::new();
    for (i, (e, d)) in entries.iter().zip(&dests).enumerate() {
        match e.kind {
            EntryKind::Dir => match std::fs::create_dir_all(d) {
                Ok(()) => r.folders_created += 1,
                Err(err) => r.errors.push(format!("{}: {err}", e.path)),
            },
            EntryKind::Symlink => {
                #[cfg(unix)]
                if let Some(t) = &e.symlink_target {
                    if let Some(p) = d.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    let _ = std::fs::remove_file(d);
                    if let Err(err) = std::os::unix::fs::symlink(t, d) {
                        r.errors.push(format!("{}: {err}", e.path));
                    }
                }
            }
            EntryKind::File => {
                let made = d.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::File::create(d).and_then(|f| f.set_len(e.size)));
                if let Err(err) = made {
                    r.errors.push(format!("{}: {err}", e.path));
                    continue;
                }
                let mut off = Some(0u64);
                for (bi, h) in e.blocks.iter().enumerate() {
                    let at = match &sizes {
                        None => Some(bi as u64 * bs),
                        Some(_) => off,
                    };
                    if let (Some(m), Some(o)) = (&sizes, off) {
                        // A block of unknown size hides where the next ones start.
                        off = m.get(h).map(|s| o + s);
                    }
                    match (index.get(h), at) {
                        (Some(v), Some(at)) => writes.entry(v.as_str()).or_default().push((i, at, h.as_str())),
                        _ => *missing.entry(i).or_default() += 1,
                    }
                }
            }
        }
    }
    // One pass per volume: read it once, write its blocks into every file.
    let total = writes.len() as u64;
    let mut handles: HashMap<usize, std::fs::File> = HashMap::new();
    for (n, (v, list)) in writes.into_iter().enumerate() {
        ctx.check()?;
        ctx.report(&Progress { phase: Phase::Restoring, files_processed: n as u64, files_total: total, bytes_processed: r.bytes_written, current_path: Some(v.to_string()) });
        let blocks = match std::fs::read(dir.join(v)).map_err(AppError::from).and_then(|d| volume::read_block_volume(&d)) {
            Ok(b) => b,
            Err(e) => {
                r.errors.push(format!("{v}: {}", e.user_message()));
                for (i, _, _) in list {
                    *missing.entry(i).or_default() += 1;
                }
                continue;
            }
        };
        for (i, off, h) in list {
            let Some(data) = blocks.get(h) else {
                *missing.entry(i).or_default() += 1;
                continue;
            };
            let f = match handles.entry(i) {
                std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => match std::fs::OpenOptions::new().write(true).open(&dests[i]) {
                    Ok(f) => slot.insert(f),
                    Err(e) => {
                        r.errors.push(format!("{}: {e}", entries[i].path));
                        continue;
                    }
                },
            };
            if let Err(e) = f.seek(SeekFrom::Start(off)).and_then(|_| f.write_all(data)) {
                r.errors.push(format!("{}: {e}", entries[i].path));
            } else {
                r.bytes_written += data.len() as u64;
            }
        }
        // Bound open handles on large restores.
        if handles.len() > 256 {
            handles.clear();
        }
    }
    drop(handles);
    for (i, e) in entries.iter().enumerate() {
        if e.kind != EntryKind::File {
            continue;
        }
        if missing.get(&i).copied().unwrap_or(0) > 0 {
            r.files_partial.push(e.path.clone());
        } else {
            r.files_restored += 1;
        }
        if let Some(m) = e.mtime {
            if let Ok(f) = std::fs::File::options().write(true).open(&dests[i]) {
                let _ = f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(m.max(0) as u64));
            }
        }
    }
    Ok(r)
}

/// Target settings for [`convert`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertOptions {
    pub prefix: String,
    pub volume_format: VolumeFormat,
    pub zip_method: ZipMethod,
    pub compression_level: u32,
    /// New passphrase; `None` writes an unencrypted repository.
    pub passphrase: Option<String>,
    #[serde(skip)]
    pub kdf: KdfParams,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self { prefix: "ufop".into(), volume_format: VolumeFormat::Ufop, zip_method: ZipMethod::Deflate, compression_level: 6, passphrase: None, kdf: KdfParams::default() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvertReport {
    pub block_volumes: usize,
    pub file_lists: usize,
    pub bytes_uploaded: u64,
    pub skipped: Vec<String>,
}

/// Re-encode the recovered copy into a new repository at `out`.
pub fn convert(dir: &Path, out: &dyn Backend, opts: &ConvertOptions, ctx: &OpContext) -> Result<ConvertReport, AppError> {
    let old = header_of(dir)?;
    out.create_folder()?;
    let hname = volume::repo_header_name(&opts.prefix);
    if out.list()?.iter().any(|f| f.name == hname) {
        return Err(AppError::validation("The target already contains a backup with this prefix"));
    }
    let v2 = old.format_version >= 2;
    let o = super::BackupOptions {
        prefix: opts.prefix.clone(),
        block_size: old.block_size,
        block_hash: old.block_hash,
        file_hash: old.file_hash,
        volume_format: opts.volume_format,
        format_version: Some(old.format_version),
        chunker: v2.then_some(super::ChunkerKind::Fixed),
        block_codec: v2.then_some(old.block_codec),
        cipher: if v2 { old.cipher } else { Default::default() },
        ..Default::default()
    };
    // Same format, blocks and chunker; new keys (a fresh v2 master key).
    let (mut header, cipher) = super::format::create_header(&o, opts.passphrase.as_deref(), None, opts.kdf, out)?;
    header.block_size = old.block_size;
    header.chunker = old.chunker;
    super::format::sync_features(&mut header);
    let mut r = ConvertReport::default();
    let mut put = |name: &str, data: &[u8]| -> Result<(), AppError> {
        out.put(name, data)?;
        r.bytes_uploaded += data.len() as u64;
        Ok(())
    };
    for v in local_volumes(dir, VolumeKind::Block)? {
        ctx.check()?;
        let blocks = match std::fs::read(dir.join(&v)).map_err(AppError::from).and_then(|d| volume::read_block_volume(&d)) {
            Ok(b) => b,
            Err(e) => {
                r.skipped.push(format!("{v}: {}", e.user_message()));
                continue;
            }
        };
        let mut w = BlockVolumeWriter::with_format(opts.volume_format, opts.zip_method).with_codec(header.block_codec);
        let mut idx = Vec::new();
        for (h, d) in blocks {
            if hash_bytes(header.block_hash, &d) != h {
                r.skipped.push(format!("{v}: block {} is corrupt", &h[..12.min(h.len())]));
                continue;
            }
            idx.push(IndexedBlock { hash: h.clone(), size: d.len() as u64 });
            w.add(h, &d, opts.compression_level)?;
        }
        let bname = volume::block_volume_name(&opts.prefix, &cipher);
        put(&bname, &cipher.seal(&w.finish())?)?;
        put(&volume::index_volume_name(&opts.prefix, &cipher), &volume::seal_json(&cipher, &IndexVolume { block_volume: bname, blocks: idx })?)?;
        r.block_volumes += 1;
    }
    for (v, fl) in filelists(dir)? {
        let ts = chrono::DateTime::parse_from_rfc3339(&fl.timestamp).map(|t| t.with_timezone(&chrono::Utc)).map_err(|_| AppError::validation(format!("{v}: bad timestamp")))?;
        put(&volume::filelist_volume_name(&opts.prefix, ts, &cipher), &volume::seal_json(&cipher, &fl)?)?;
        r.file_lists += 1;
    }
    // Header last: the repository becomes visible only when complete.
    put(&hname, &serde_json::to_vec_pretty(&header)?)?;
    Ok(r)
}

/// Blocks referenced by `version` that are missing from the local copy.
pub fn missing_blocks(dir: &Path, version: usize) -> Result<usize, AppError> {
    let index = load_index(dir)?;
    let (_, fl) = filelists(dir)?.into_iter().nth(version).ok_or_else(|| AppError::not_found("No such version"))?;
    let need: HashSet<&String> = fl.files.iter().flat_map(|f| f.blocks.iter()).collect();
    Ok(need.into_iter().filter(|h| !index.contains_key(*h)).count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::db::BackupDb;
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, BackupOptions, RetentionPolicy, SourceFilter};

    fn backup(src: &Path, dst: &Path) {
        // Fixed 1 KiB blocks: the block counts below depend on them.
        let o = BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, chunker: Some(crate::backup::ChunkerKind::Fixed), ..Default::default() };
        let db = BackupDb::open_in_memory().unwrap();
        let r = run_backup(&LocalBackend::new(dst), &db, &[src.into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn salvage_download_index_restore_and_resume() {
        let (src, dst, rec, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let big: Vec<u8> = (0..9000u32).map(|i| (i * 31 % 251) as u8).collect();
        write(src.path(), "a/big.bin", &big);
        write(src.path(), "a/small.txt", b"small");
        backup(src.path(), dst.path());
        let be = LocalBackend::new(dst.path());
        assert!(download(&be, Some("wrong"), "ufop", rec.path(), &DownloadOptions::default(), &OpContext::new()).is_err());
        let d = download(&be, Some("pw"), "ufop", rec.path(), &DownloadOptions { threads: 3, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(d.failed.is_empty() && d.downloaded >= 3, "{d:?}");
        // Resume: nothing left to fetch.
        let d2 = download(&be, Some("pw"), "ufop", rec.path(), &DownloadOptions::default(), &OpContext::new()).unwrap();
        assert_eq!(d2.downloaded, 0);
        assert_eq!(d2.skipped_existing, d.downloaded);

        let ix = build_index(rec.path(), &OpContext::new()).unwrap();
        assert!(ix.blocks >= 10 && ix.damaged.is_empty());
        let v = list_versions(rec.path()).unwrap();
        assert_eq!(v.len(), 1);
        let r = restore(rec.path(), 0, &[], out.path(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty() && r.files_partial.is_empty(), "{r:?}");
        assert_eq!(r.files_restored, 2);
        let root = out.path().join(src.path().file_name().unwrap());
        assert_eq!(std::fs::read(root.join("a/big.bin")).unwrap(), big);

        // Lose one block volume: files are still written, gaps reported.
        let gone = local_volumes(rec.path(), VolumeKind::Block).unwrap().into_iter().next().unwrap();
        std::fs::write(rec.path().join(&gone), b"corrupt").unwrap();
        let ix = build_index(rec.path(), &OpContext::new()).unwrap();
        assert_eq!(ix.damaged.len(), 1);
        assert!(missing_blocks(rec.path(), 0).unwrap() > 0);
        let out2 = tempfile::tempdir().unwrap();
        let r = restore(rec.path(), 0, &[], out2.path(), &OpContext::new()).unwrap();
        assert!(!r.files_partial.is_empty());
        let p = out2.path().join(src.path().file_name().unwrap()).join("a/big.bin");
        assert_eq!(std::fs::metadata(p).unwrap().len(), big.len() as u64, "full size, gaps zero-filled");
    }

    #[test]
    fn rate_limit_paces_downloads() {
        let (src, dst, rec) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "f", &vec![1u8; 6000]);
        backup(src.path(), dst.path());
        let t = Instant::now();
        let d = download(&LocalBackend::new(dst.path()), Some("pw"), "ufop", rec.path(), &DownloadOptions { threads: 1, resume: false, max_bytes_per_sec: 8000 }, &OpContext::new()).unwrap();
        let expect = d.bytes as f64 / 8000.0;
        assert!(t.elapsed().as_secs_f64() >= expect * 0.8, "{} bytes in {:?}", d.bytes, t.elapsed());
    }

    #[test]
    fn convert_to_unencrypted_zip_and_restore_normally() {
        let (src, dst, rec, out2, outr) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "x/data.bin", &(0..7000u32).map(|i| (i % 199) as u8).collect::<Vec<_>>());
        backup(src.path(), dst.path());
        download(&LocalBackend::new(dst.path()), Some("pw"), "ufop", rec.path(), &DownloadOptions::default(), &OpContext::new()).unwrap();
        let target = LocalBackend::new(out2.path());
        let c = convert(rec.path(), &target, &ConvertOptions { prefix: "conv".into(), volume_format: VolumeFormat::Zip, zip_method: ZipMethod::Zstd, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(c.block_volumes > 0 && c.file_lists == 1 && c.skipped.is_empty(), "{c:?}");
        assert!(convert(rec.path(), &target, &ConvertOptions { prefix: "conv".into(), ..Default::default() }, &OpContext::new()).is_err(), "won't overwrite");
        // The converted repository works with the normal tools.
        let db = BackupDb::open_in_memory().unwrap();
        crate::backup::maintenance::recreate_database(&target, &db, None, "conv", &OpContext::new()).unwrap();
        let o = BackupOptions { prefix: "conv".into(), ..Default::default() };
        let repo = crate::backup::engine::Repo::open(&target, &db, None, &o, false, KdfParams::insecure_fast()).unwrap();
        assert_eq!(repo.header.volume_format, VolumeFormat::Zip);
        let rr = crate::backup::run_restore(&repo, &RestoreOptions { target: Some(outr.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert_eq!(
            std::fs::read(outr.path().join(src.path().file_name().unwrap()).join("x/data.bin")).unwrap(),
            std::fs::read(src.path().join("x/data.bin")).unwrap()
        );
    }

    /// The independent Python tool must read what the engine writes.
    #[test]
    fn python_recovery_tool_reads_engine_output() {
        let ok = std::process::Command::new("python3").args(["-c", "import cryptography.hazmat.primitives.kdf.argon2"]).status().is_ok_and(|s| s.success());
        if !ok {
            eprintln!("skipped: python3 with cryptography >= 44 not available");
            return;
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/ufop-recover.py");
        // Python can read zstd only with 3.14+ or the zstandard package.
        let py_zstd = std::process::Command::new("python3")
            .args(["-c", "import importlib.util as u, sys; sys.exit(0 if u.find_spec('zstandard') or u.find_spec('compression.zstd') else 1)"])
            .status()
            .is_ok_and(|s| s.success());
        let base = BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, ..Default::default() };
        let mut variants = vec![
            ("v1 ufop", BackupOptions { format_version: Some(1), ..base.clone() }),
            ("v1 zip", BackupOptions { format_version: Some(1), volume_format: VolumeFormat::Zip, ..base.clone() }),
            // Format 2 without zstd: envelope key, ChaCha20-Poly1305, CDC, sharded layout.
            (
                "v2 chacha cdc sharded",
                BackupOptions {
                    block_codec: Some(crate::backup::BlockCodec::Deflate),
                    metadata_compression: Some(crate::backup::MetadataCompression::None),
                    cipher: crate::backup::crypto::CipherKind::ChaCha20Poly1305,
                    layout: crate::backup::layout::RepoLayout::Sharded { prefix_len: 1 },
                    ..base.clone()
                },
            ),
        ];
        if py_zstd {
            variants.push(("v2 default", base.clone()));
        }
        for (label, o) in variants {
            let fmt = label;
            let (src, dst, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            // Non-periodic content so content-defined blocks vary in size.
            let data: Vec<u8> = (0..12_000u64).map(|i| ((i.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)) >> 37) as u8).collect();
            write(src.path(), "d/file.bin", &data);
            let db = BackupDb::open_in_memory().unwrap();
            run_backup(&LocalBackend::new(dst.path()), &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
            let st = std::process::Command::new("python3")
                .arg("-I")
                .arg(&script)
                .args(["restore", &dst.path().to_string_lossy(), &out.path().to_string_lossy()])
                .env("UFOP_PASSPHRASE", "pw")
                .status()
                .unwrap();
            assert!(st.success(), "{fmt}");
            assert_eq!(std::fs::read(out.path().join(src.path().file_name().unwrap()).join("d/file.bin")).unwrap(), data, "{fmt}");
        }
    }
}
