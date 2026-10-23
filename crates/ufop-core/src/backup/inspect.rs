//! Read-only expert views of a repository (`ufop backup debug …`).
//!
//! Everything here decodes what the engine itself writes, through the
//! engine's own readers ([`volume::read_block_volume`],
//! [`volume::list_block_volume`], [`volume::open_json`], the repository
//! header and lock files), and never changes the destination or the local
//! database. Callers hold a shared repository lock.

use super::db::BackupDb;
use super::engine::Repo;
use super::format::{self, FormatInfo};
use super::lock;
use super::volume::{self, FileListVolume, IndexVolume, RepoHeader, VolumeKind};
use super::{hash_bytes, Backend};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The decoded repository header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderView {
    /// File name on the destination.
    pub file: String,
    pub header: RepoHeader,
    pub format: FormatInfo,
    /// Feature flags this build understands / does not understand.
    pub known_features: Vec<String>,
    pub unknown_features: Vec<String>,
    /// Size of the header file in bytes.
    pub size: u64,
}

/// Header of `prefix` (no passphrase needed). Key material in the header
/// (salt, key check, wrapped master key) is public by design: it is only
/// usable with the passphrase.
pub fn header(backend: &dyn Backend, prefix: &str) -> Result<HeaderView, AppError> {
    let (h, raw) = format::read_header(backend, prefix)?;
    let (known_features, unknown_features) = h.features.iter().cloned().partition(|f| format::KNOWN_FEATURES.contains(&f.as_str()));
    Ok(HeaderView { file: volume::repo_header_name(prefix), format: FormatInfo::from(&h), header: h, known_features, unknown_features, size: raw.len() as u64 })
}

/// One file on the destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeView {
    pub name: String,
    /// `dblock`, `dindex`, `dlist`, `header`, `lock` or `other`.
    pub kind: String,
    pub size: u64,
    /// Shard folder with the sharded layout.
    pub shard: Option<String>,
    /// State recorded in the local database (`uploaded`, …), if known.
    pub state: Option<String>,
}

/// Filters for [`volumes`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeFilter {
    /// `dblock`, `dindex`, `dlist`, `header`, `lock`, `other` (`block`, `index`, `list` too).
    pub kind: Option<String>,
    /// Name prefix (after the repository prefix and dash, e.g. `b12`).
    pub name_prefix: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
}

fn kind_label(prefix: &str, name: &str) -> &'static str {
    if let Some(k) = volume::classify(prefix, name) {
        return k.as_str();
    }
    if name == volume::repo_header_name(prefix) || super::layout::header_copies(prefix).iter().any(|c| c == name) {
        "header"
    } else if lock::is_lock_file(prefix, name) {
        "lock"
    } else {
        "other"
    }
}

fn normalize_kind(k: &str) -> String {
    match k.trim().to_ascii_lowercase().as_str() {
        "block" | "blocks" | "b" => "dblock".into(),
        "index" | "i" => "dindex".into(),
        "list" | "filelist" | "fileset" | "l" => "dlist".into(),
        other => other.to_string(),
    }
}

/// Files of the repository on the destination, by name.
pub fn volumes(repo: &Repo, filter: &VolumeFilter) -> Result<Vec<VolumeView>, AppError> {
    let states: HashMap<String, String> = repo.db.volumes(None)?.into_iter().map(|v| (v.name, v.state)).collect();
    let want_kind = filter.kind.as_deref().map(normalize_kind);
    let mut out = Vec::new();
    for f in repo.backend.list()? {
        let belongs = f.name.starts_with(&format!("{}-", repo.prefix));
        if !belongs {
            continue;
        }
        let kind = kind_label(&repo.prefix, &f.name);
        if want_kind.as_deref().is_some_and(|k| k != kind) {
            continue;
        }
        let rest = &f.name[repo.prefix.len() + 1..];
        if filter.name_prefix.as_deref().is_some_and(|p| !rest.starts_with(p) && !f.name.starts_with(p)) {
            continue;
        }
        if filter.min_size.is_some_and(|m| f.size < m) || filter.max_size.is_some_and(|m| f.size > m) {
            continue;
        }
        let physical = repo.header.layout.physical(&repo.prefix, &f.name);
        let shard = physical.rsplit_once('/').map(|(d, _)| d.to_string());
        out.push(VolumeView { state: states.get(&f.name).cloned(), name: f.name, kind: kind.into(), size: f.size, shard });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KindStats {
    pub kind: String,
    pub count: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionStats {
    /// 0 = newest.
    pub version: usize,
    pub timestamp: String,
    pub files: u64,
    /// Sum of file sizes in the version.
    pub logical_bytes: u64,
    /// Distinct blocks the version references, and their raw size.
    pub blocks: u64,
    pub referenced_bytes: u64,
    /// Referenced bytes no other version references.
    pub unique_bytes: u64,
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoStats {
    pub by_kind: Vec<KindStats>,
    pub total_bytes: u64,
    /// Blocks known to the local database and their raw (uncompressed) size.
    pub blocks: u64,
    pub block_bytes: u64,
    pub avg_block_size: f64,
    /// Bytes of all dblock volumes (compressed and sealed).
    pub stored_block_bytes: u64,
    /// stored_block_bytes / block_bytes.
    pub compression_ratio: f64,
    /// Sum of every version's logical size / raw block bytes (higher = more saved).
    pub dedup_ratio: f64,
    pub versions: Vec<VersionStats>,
}

fn block_sizes(db: &BackupDb) -> Result<HashMap<String, u64>, AppError> {
    Ok(db.blocks_by_volume()?.into_values().flatten().collect())
}

/// Counts and bytes per kind, blocks, deduplication and per-version use.
pub fn stats(repo: &Repo) -> Result<RepoStats, AppError> {
    let mut by_kind: BTreeMap<String, KindStats> = BTreeMap::new();
    for v in volumes(repo, &VolumeFilter::default())? {
        let k = by_kind.entry(v.kind.clone()).or_insert(KindStats { kind: v.kind.clone(), count: 0, bytes: 0 });
        k.count += 1;
        k.bytes += v.size;
    }
    let sizes = block_sizes(repo.db)?;
    let block_bytes: u64 = sizes.values().sum();
    let stored_block_bytes = by_kind.get("dblock").map_or(0, |k| k.bytes);
    let filesets = repo.db.filesets()?;
    let mut per_version: Vec<(HashSet<String>, &super::Fileset)> = Vec::new();
    for fs in &filesets {
        let blocks: HashSet<String> = repo.db.files(fs.id)?.into_iter().flat_map(|f| f.blocks).collect();
        per_version.push((blocks, fs));
    }
    let mut refs: HashMap<&str, u32> = HashMap::new();
    for (b, _) in &per_version {
        for h in b {
            *refs.entry(h.as_str()).or_default() += 1;
        }
    }
    let size_of = |h: &str| sizes.get(h).copied().unwrap_or(0);
    let versions: Vec<VersionStats> = per_version
        .iter()
        .enumerate()
        .map(|(i, (b, fs))| VersionStats {
            version: i,
            timestamp: fs.timestamp.clone(),
            files: fs.file_count,
            logical_bytes: fs.total_size,
            blocks: b.len() as u64,
            referenced_bytes: b.iter().map(|h| size_of(h)).sum(),
            unique_bytes: b.iter().filter(|h| refs.get(h.as_str()) == Some(&1)).map(|h| size_of(h)).sum(),
            partial: fs.partial,
        })
        .collect();
    let logical: u64 = filesets.iter().map(|f| f.total_size).sum();
    let ratio = |a: u64, b: u64| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let by_kind: Vec<KindStats> = by_kind.into_values().collect();
    Ok(RepoStats {
        total_bytes: by_kind.iter().map(|k| k.bytes).sum(),
        by_kind,
        blocks: sizes.len() as u64,
        block_bytes,
        avg_block_size: if sizes.is_empty() { 0.0 } else { block_bytes as f64 / sizes.len() as f64 },
        stored_block_bytes,
        compression_ratio: ratio(stored_block_bytes, block_bytes),
        dedup_ratio: ratio(logical, block_bytes),
        versions,
    })
}

/// One block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockView {
    pub hash: String,
    /// Raw size.
    pub size: u64,
    pub volume: String,
    /// Filled in when the volume was downloaded (`details`).
    pub stored_size: Option<u64>,
    pub codec: Option<String>,
}

/// Filters for [`blocks`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BlockFilter {
    /// Only blocks referenced by this version (0 = newest).
    pub version: Option<usize>,
    pub volume: Option<String>,
    /// Block hash prefix.
    pub hash_prefix: Option<String>,
    /// Download the volumes involved for stored size and codec (always
    /// done with `volume`).
    pub details: bool,
    /// Stop after this many blocks (0 = all).
    pub limit: usize,
}

/// Blocks known to the local database, filtered.
pub fn blocks(repo: &Repo, filter: &BlockFilter) -> Result<Vec<BlockView>, AppError> {
    let wanted: Option<HashSet<String>> = match filter.version {
        Some(v) => {
            let fs = repo.db.fileset_by_version(v)?;
            Some(repo.db.files(fs.id)?.into_iter().flat_map(|f| f.blocks).collect())
        }
        None => None,
    };
    let prefix = filter.hash_prefix.as_deref().map(|p| p.to_ascii_lowercase());
    let mut out = Vec::new();
    let mut by_volume: Vec<(String, Vec<(String, u64)>)> = repo.db.blocks_by_volume()?.into_iter().collect();
    by_volume.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(v) = &filter.volume {
        if !by_volume.iter().any(|(n, _)| n == v) {
            return Err(AppError::not_found(format!("No block volume named {v} in the local database")));
        }
    }
    for (vol, mut list) in by_volume {
        if filter.volume.as_ref().is_some_and(|v| *v != vol) {
            continue;
        }
        list.sort();
        let start = out.len();
        for (hash, size) in list {
            if wanted.as_ref().is_some_and(|w| !w.contains(&hash)) || prefix.as_deref().is_some_and(|p| !hash.starts_with(p)) {
                continue;
            }
            out.push(BlockView { hash, size, volume: vol.clone(), stored_size: None, codec: None });
        }
        if (filter.details || filter.volume.is_some()) && out.len() > start {
            let entries: HashMap<String, volume::StoredBlock> = volume::list_block_volume(&repo.cipher.open(&repo.get_volume(&vol)?)?)?.into_iter().map(|e| (e.hash.clone(), e)).collect();
            for b in &mut out[start..] {
                if let Some(e) = entries.get(&b.hash) {
                    b.stored_size = Some(e.stored_size);
                    b.codec = Some(e.codec.clone());
                }
            }
        }
        if filter.limit > 0 && out.len() >= filter.limit {
            out.truncate(filter.limit);
            break;
        }
    }
    Ok(out)
}

/// One block fetched from its volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockData {
    pub hash: String,
    pub volume: String,
    pub codec: String,
    pub stored_size: u64,
    pub size: u64,
    /// The decompressed data hashes to the block's name.
    pub hash_ok: bool,
    /// Decompressed bytes, or the stored bytes with `raw` (UFOP packs).
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// Download the volume holding `hash` and decode that block. With `raw`
/// the stored (compressed) bytes of a UFOP pack entry are returned.
pub fn block(repo: &Repo, hash: &str, raw: bool) -> Result<BlockData, AppError> {
    let hash = hash.trim().to_ascii_lowercase();
    let vol = repo.db.block_locations()?.remove(&hash).ok_or_else(|| AppError::not_found(format!("Block {hash} is not in the local database")))?;
    let payload = repo.cipher.open(&repo.get_volume(&vol)?)?;
    let info = volume::list_block_volume(&payload)?.into_iter().find(|e| e.hash == hash).ok_or_else(|| AppError::not_found(format!("Block {hash} is missing from volume {vol}")))?;
    let decoded = volume::read_block_volume(&payload)?.remove(&hash).ok_or_else(|| AppError::not_found(format!("Block {hash} is missing from volume {vol}")))?;
    let hash_ok = hash_bytes(repo.header.block_hash, &decoded) == hash;
    let data = if raw { volume::raw_block_entry(&payload, &hash).map(|(_, d)| d).unwrap_or(decoded) } else { decoded };
    Ok(BlockData { hash, volume: vol, codec: info.codec, stored_size: info.stored_size, size: info.size, hash_ok, data })
}

/// A decoded dindex volume.
pub fn index(repo: &Repo, name: &str) -> Result<IndexVolume, AppError> {
    if volume::classify(&repo.prefix, name) != Some(VolumeKind::Index) {
        return Err(AppError::validation(format!("{name} is not an index (dindex) volume of this backup")));
    }
    volume::open_json(&repo.cipher, &repo.get_volume(name)?)
}

/// The decoded dlist of a version (0 = newest), with its metadata.
pub fn fileset(repo: &Repo, version: usize) -> Result<(String, FileListVolume), AppError> {
    let fs = repo.db.fileset_by_version(version)?;
    let fl = volume::open_json(&repo.cipher, &repo.get_volume(&fs.volume_name)?)?;
    Ok((fs.volume_name, fl))
}

/// Current lock holders: the writer (exclusive lock) and readers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sessions {
    pub writer: Option<lock::LockEntry>,
    pub readers: Vec<lock::LockEntry>,
    /// Expired or damaged lock files the next operation will take over.
    pub stale: Vec<lock::LockEntry>,
    pub summary: Option<String>,
}

pub fn sessions(backend: &dyn Backend, prefix: &str) -> Result<Sessions, AppError> {
    let st = lock::status(backend, prefix)?;
    let summary = st.summary();
    let (live, stale): (Vec<_>, Vec<_>) = st.locks.into_iter().partition(|l| !l.stale);
    let (writers, readers): (Vec<_>, Vec<_>) = live.into_iter().partition(|l| l.info.kind == lock::LockKind::Exclusive);
    Ok(Sessions { writer: writers.into_iter().next(), readers, stale, summary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, BackupOptions, OpContext, RetentionPolicy, SourceFilter};

    fn setup(pass: Option<&str>, layout_sharded: bool) -> (tempfile::TempDir, tempfile::TempDir, BackupDb, BackupOptions) {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "a.txt", &b"hello world ".repeat(400));
        write(src.path(), "dir/b.bin", &(0..5000u32).map(|i| (i * 7 % 251) as u8).collect::<Vec<_>>());
        let mut o = BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, ..Default::default() };
        if layout_sharded {
            o.layout = crate::backup::layout::RepoLayout::Sharded { prefix_len: 1 };
        }
        let be = LocalBackend::new(dst.path().to_str().unwrap());
        let db = BackupDb::open_in_memory().unwrap();
        let run = |db: &BackupDb| run_backup(&be, db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, pass, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        run(&db);
        write(src.path(), "c.txt", b"a new file in the second version");
        run(&db);
        (src, dst, db, o)
    }

    #[test]
    fn header_volumes_stats_and_blocks() {
        let (_s, dst, db, o) = setup(Some("pw"), true);
        let be = LocalBackend::new(dst.path().to_str().unwrap());
        let h = header(&be, "ufop").unwrap();
        assert!(h.header.encrypted && h.unknown_features.is_empty() && h.size > 0);
        let repo = Repo::open(&be, &db, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();

        let all = volumes(&repo, &VolumeFilter::default()).unwrap();
        assert!(all.iter().any(|v| v.kind == "header"));
        let blocks_v = volumes(&repo, &VolumeFilter { kind: Some("block".into()), ..Default::default() }).unwrap();
        assert!(!blocks_v.is_empty() && blocks_v.iter().all(|v| v.kind == "dblock" && v.shard.as_deref().is_some_and(|s| s.len() == 1) && v.state.is_some()));
        assert!(volumes(&repo, &VolumeFilter { kind: Some("dlist".into()), ..Default::default() }).unwrap().iter().all(|v| v.shard.is_none()));
        assert_eq!(volumes(&repo, &VolumeFilter { kind: Some("dlist".into()), ..Default::default() }).unwrap().len(), 2);
        assert!(volumes(&repo, &VolumeFilter { min_size: Some(u64::MAX), ..Default::default() }).unwrap().is_empty());
        let one = &blocks_v[0].name;
        let rest = &one["ufop-".len()..];
        assert_eq!(volumes(&repo, &VolumeFilter { name_prefix: Some(rest[..12].into()), ..Default::default() }).unwrap().len(), 1);

        let s = stats(&repo).unwrap();
        assert_eq!(s.versions.len(), 2);
        assert!(s.blocks > 0 && s.block_bytes > 0 && s.avg_block_size > 0.0);
        assert!(s.dedup_ratio > 1.5, "two nearly equal versions dedup ~2x: {}", s.dedup_ratio);
        assert_eq!(s.by_kind.iter().find(|k| k.kind == "dlist").unwrap().count, 2);
        // The newer version alone references the new file's block.
        assert!(s.versions[0].unique_bytes > 0 && s.versions[1].unique_bytes == 0);

        let all_blocks = blocks(&repo, &BlockFilter::default()).unwrap();
        assert_eq!(all_blocks.len() as u64, s.blocks);
        assert!(all_blocks.iter().all(|b| b.codec.is_none()));
        let v1 = blocks(&repo, &BlockFilter { version: Some(1), ..Default::default() }).unwrap();
        assert!(v1.len() < all_blocks.len());
        let detailed = blocks(&repo, &BlockFilter { volume: Some(all_blocks[0].volume.clone()), ..Default::default() }).unwrap();
        assert!(detailed.iter().all(|b| b.codec.is_some() && b.stored_size.is_some()));
        let pfx = blocks(&repo, &BlockFilter { hash_prefix: Some(all_blocks[0].hash[..10].to_uppercase()), ..Default::default() }).unwrap();
        assert_eq!(pfx.len(), 1);
        assert_eq!(blocks(&repo, &BlockFilter { limit: 2, ..Default::default() }).unwrap().len(), 2);
        assert!(blocks(&repo, &BlockFilter { volume: Some("nope".into()), ..Default::default() }).is_err());
    }

    #[test]
    fn block_index_fileset_and_sessions() {
        let (_s, dst, db, o) = setup(None, false);
        let be = LocalBackend::new(dst.path().to_str().unwrap());
        let repo = Repo::open(&be, &db, None, &o, false, KdfParams::insecure_fast()).unwrap();
        let first = blocks(&repo, &BlockFilter { details: true, ..Default::default() }).unwrap();
        let compressed = first.iter().find(|b| b.codec.as_deref() != Some("stored")).unwrap();
        let b = block(&repo, &compressed.hash, false).unwrap();
        assert!(b.hash_ok);
        assert_eq!(b.data.len() as u64, b.size);
        let raw = block(&repo, &compressed.hash, true).unwrap();
        assert_eq!(raw.data.len() as u64, raw.stored_size);
        assert!(raw.stored_size < raw.size);
        assert!(block(&repo, "00", false).is_err());

        let idx_name = volumes(&repo, &VolumeFilter { kind: Some("dindex".into()), ..Default::default() }).unwrap()[0].name.clone();
        let idx = index(&repo, &idx_name).unwrap();
        assert!(!idx.blocks.is_empty() && idx.block_volume.contains(".dblock"));
        assert!(index(&repo, &idx.block_volume).is_err());

        let (name, fl) = fileset(&repo, 0).unwrap();
        assert!(name.ends_with(".dlist"));
        assert!(fl.files.iter().any(|f| f.path.ends_with("c.txt")));
        assert!(!fileset(&repo, 1).unwrap().1.files.iter().any(|f| f.path.ends_with("c.txt")));
        assert!(fileset(&repo, 9).is_err());

        let none = sessions(&be, "ufop").unwrap();
        assert!(none.writer.is_none() && none.readers.is_empty() && none.summary.is_none());
        let opts = lock::LockOptions::from_backup_options(&o, "test");
        let r1 = lock::RepoLock::acquire(&be, "ufop", lock::LockKind::Shared, "restore", &opts).unwrap();
        let r2 = lock::RepoLock::acquire(&be, "ufop", lock::LockKind::Shared, "verify", &opts).unwrap();
        let s = sessions(&be, "ufop").unwrap();
        assert_eq!(s.readers.len(), 2);
        assert!(s.writer.is_none() && s.summary.is_some());
        r1.release().unwrap();
        r2.release().unwrap();
        let w = lock::RepoLock::acquire(&be, "ufop", lock::LockKind::Exclusive, "backup", &opts).unwrap();
        assert_eq!(sessions(&be, "ufop").unwrap().writer.unwrap().info.operation, "backup");
        w.release().unwrap();
    }

    #[test]
    fn volume_listing_helpers() {
        let mut w = volume::BlockVolumeWriter::new();
        w.add("aa".into(), &[7u8; 1000], 6).unwrap();
        w.add("bb".into(), b"xyz", 6).unwrap();
        let p = w.finish();
        let l = volume::list_block_volume(&p).unwrap();
        assert_eq!(l[0].codec, "deflate");
        assert_eq!((l[0].size, l[1].codec.as_str(), l[1].size, l[1].stored_size), (1000, "stored", 3, 3));
        assert_eq!(volume::raw_block_entry(&p, "bb"), Some((0, b"xyz".to_vec())));
        assert_eq!(volume::raw_block_entry(&p, "cc"), None);
        assert!(volume::list_block_volume(&[1, 0, 0, 0, 2]).is_err());
        let mut z = volume::BlockVolumeWriter::with_format(crate::backup::VolumeFormat::Zip, crate::backup::ZipMethod::Deflate);
        z.add("aa".into(), &[7u8; 1000], 6).unwrap();
        let zl = volume::list_block_volume(&z.finish()).unwrap();
        assert_eq!((zl[0].hash.as_str(), zl[0].size), ("aa", 1000));
        assert!(zl[0].stored_size < 1000);
    }
}
