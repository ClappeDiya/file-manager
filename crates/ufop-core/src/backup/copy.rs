//! Copy selected versions from one repository to another.
//!
//! Only the blocks the selected versions reference and the destination
//! does not have are transferred. Blocks are always decoded from the source
//! and re-encoded for the destination (its codec, cipher and volume
//! format). When both repositories cut and hash blocks the same way, block
//! identities are kept; when the chunker or a hash algorithm differs, the
//! file contents are re-assembled from the source blocks and cut again with
//! the destination's chunker (so a format 1 fixed-size repository can be
//! copied into a format 2 content-defined one). Each version gets a new
//! file list in the destination's format with its note, tags, pin and
//! identity; its statistics describe what the copy added there.
//!
//! The caller takes the locks: shared on the source, exclusive on the
//! destination.

use super::chunker::for_each_chunk;
use super::engine::{flush, BackupReport, Pending, Repo};
use super::volume::{self, FileListVolume};
use super::{hash_bytes, BackupOptions, EntryKind, FileEntry, FileHasher, OpContext};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;

/// What to copy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CopyOptions {
    /// Source version numbers (0 = newest).
    pub versions: Vec<usize>,
    /// Shared source repositories: copy from this source (`*` = number
    /// every source's versions together) instead of the job's own.
    pub source: Option<String>,
    /// Count what would be copied without writing anything.
    pub dry_run: bool,
}

/// Result of [`copy_versions`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CopyReport {
    pub dry_run: bool,
    pub versions_copied: usize,
    /// Already in the destination (same time and the same files).
    pub versions_skipped: usize,
    pub files: u64,
    /// Blocks transferred (missing in the destination).
    pub blocks_copied: u64,
    /// Blocks the destination already had.
    pub blocks_reused: u64,
    /// Raw size of the blocks transferred.
    pub bytes_copied: u64,
    /// Sealed bytes uploaded (block volumes and indexes).
    pub bytes_uploaded: u64,
    /// File contents were cut again with the destination's chunker.
    pub rechunked: bool,
    /// Timestamps of the versions written to the destination.
    pub copied: Vec<String>,
    pub warnings: Vec<String>,
}

/// Decoded source block volumes, a few kept in memory.
struct BlockCache<'r, 'a> {
    repo: &'r Repo<'a>,
    loc: HashMap<String, String>,
    vols: VecDeque<(String, HashMap<String, Vec<u8>>)>,
}

const CACHED_VOLUMES: usize = 4;

impl<'r, 'a> BlockCache<'r, 'a> {
    fn new(repo: &'r Repo<'a>) -> Result<Self, AppError> {
        Ok(Self { repo, loc: repo.db.block_locations()?, vols: VecDeque::new() })
    }

    fn volume_of(&self, hash: &str) -> Result<&str, AppError> {
        self.loc.get(hash).map(String::as_str).ok_or_else(|| {
            AppError::transfer(format!("Block {hash} is not in the source repository"), "Run Verify and Repair on the source backup, then copy again.")
        })
    }

    fn load(&mut self, vol: &str) -> Result<&HashMap<String, Vec<u8>>, AppError> {
        if let Some(i) = self.vols.iter().position(|(n, _)| n == vol) {
            return Ok(&self.vols[i].1);
        }
        let data = self.repo.cipher.open(&self.repo.get_volume(vol)?)?;
        let map = volume::read_block_volume(&data)?;
        if self.vols.len() >= CACHED_VOLUMES {
            self.vols.pop_front();
        }
        self.vols.push_back((vol.to_string(), map));
        Ok(&self.vols[self.vols.len() - 1].1)
    }

    fn get(&mut self, hash: &str) -> Result<Vec<u8>, AppError> {
        let vol = self.volume_of(hash)?.to_string();
        self.load(&vol)?.get(hash).cloned().ok_or_else(|| AppError::transfer(format!("Block {hash} is missing from {vol}"), "Run Verify and Repair on the source backup."))
    }
}

/// Reads a file's content back from its source blocks.
struct BlockReader<'c, 'r, 'a> {
    cache: &'c mut BlockCache<'r, 'a>,
    blocks: std::slice::Iter<'c, String>,
    cur: Vec<u8>,
    pos: usize,
}

impl Read for BlockReader<'_, '_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.cur.len() {
            let Some(h) = self.blocks.next() else { return Ok(0) };
            self.cur = self.cache.get(h).map_err(|e| std::io::Error::other(e.user_message()))?;
            self.pos = 0;
        }
        let n = buf.len().min(self.cur.len() - self.pos);
        buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Adds blocks the destination lacks to its pending volume.
struct Sink<'r, 'a> {
    dst: &'r Repo<'a>,
    opts: BackupOptions,
    pending: Pending,
    seen: HashSet<String>,
    scratch: BackupReport,
}

impl Sink<'_, '_> {
    fn has(&self, hash: &str) -> Result<bool, AppError> {
        Ok(self.seen.contains(hash) || self.dst.db.has_block(hash)?)
    }

    fn add(&mut self, hash: &str, data: &[u8], r: &mut CopyReport) -> Result<(), AppError> {
        if self.has(hash)? {
            r.blocks_reused += 1;
            return Ok(());
        }
        self.pending.add_block(hash.to_string(), data, self.opts.compression_level)?;
        self.seen.insert(hash.to_string());
        r.blocks_copied += 1;
        r.bytes_copied += data.len() as u64;
        if self.pending.byte_size() >= self.opts.volume_size {
            flush(self.dst, &mut self.pending, &self.opts, &mut self.scratch)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), AppError> {
        flush(self.dst, &mut self.pending, &self.opts, &mut self.scratch)
    }
}

fn same_files(a: &[FileEntry], b: &[FileEntry]) -> bool {
    let key = |v: &[FileEntry]| {
        let mut k: Vec<(String, Option<String>, u64)> = v.iter().map(|f| (f.path.clone(), f.hash.clone(), f.size)).collect();
        k.sort();
        k
    };
    key(a) == key(b)
}

/// Copy `opts.versions` of `src` into `dst`. `dst_opts` supplies the
/// destination's volume size and compression level.
pub fn copy_versions(src: &Repo, dst: &Repo, opts: &CopyOptions, dst_opts: &BackupOptions, ctx: &OpContext) -> Result<CopyReport, AppError> {
    let mut r = CopyReport { dry_run: opts.dry_run, ..Default::default() };
    if opts.versions.is_empty() {
        return Err(AppError::validation("Select at least one version to copy"));
    }
    let picks = {
        let _scope = super::shared::selector_scope(opts.source.as_deref()).map(|s| src.db.scoped(s));
        let all = src.db.filesets()?;
        let mut p = Vec::new();
        for v in &opts.versions {
            p.push(all.get(*v).cloned().ok_or_else(|| AppError::validation(format!("Backup version {v} does not exist")))?);
        }
        p.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        p.dedup_by(|a, b| a.id == b.id);
        p
    };
    let (sh, dh) = (&src.header, &dst.header);
    let rechunk = sh.chunker_spec() != dh.chunker_spec() || sh.block_hash != dh.block_hash || sh.file_hash != dh.file_hash;
    r.rechunked = rechunk;
    let mut cache = BlockCache::new(src)?;
    let mut sink = Sink { dst, opts: BackupOptions { dry_run: opts.dry_run, ..dst_opts.clone() }, pending: Pending::for_repo(dst, dst_opts), seen: HashSet::new(), scratch: BackupReport::default() };
    // Re-cut contents keyed by the source block list (same content, same result).
    let mut recut: HashMap<String, (Vec<String>, String)> = HashMap::new();
    let chunker = dst.chunker();

    for fs in picks {
        ctx.check()?;
        let fl: FileListVolume = volume::open_json(&src.cipher, &src.get_volume(&fs.volume_name)?)?;
        let (blocks_before, uploaded_before) = (r.blocks_copied, sink.scratch.bytes_uploaded);
        let mut files = fl.files.clone();
        if !rechunk {
            // Missing blocks grouped by source volume: each is downloaded once.
            let mut by_vol: HashMap<String, Vec<String>> = HashMap::new();
            let mut wanted = HashSet::new();
            for h in files.iter().filter(|f| f.kind == EntryKind::File).flat_map(|f| f.blocks.iter()) {
                if !wanted.insert(h.clone()) {
                    continue;
                }
                if sink.has(h)? {
                    r.blocks_reused += 1;
                    continue;
                }
                by_vol.entry(cache.volume_of(h)?.to_string()).or_default().push(h.clone());
            }
            let mut vols: Vec<_> = by_vol.into_iter().collect();
            vols.sort();
            for (vol, hashes) in vols {
                ctx.check()?;
                let map = cache.load(&vol)?.clone();
                for h in hashes {
                    let d = map.get(&h).ok_or_else(|| AppError::transfer(format!("Block {h} is missing from {vol}"), "Run Verify and Repair on the source backup."))?;
                    sink.add(&h, d, &mut r)?;
                }
            }
        } else {
            for f in files.iter_mut().filter(|f| f.kind == EntryKind::File) {
                ctx.check()?;
                let key = format!("{}:{}", f.size, f.blocks.join(","));
                if let Some((blocks, hash)) = recut.get(&key) {
                    for b in blocks {
                        if !sink.has(b)? {
                            return Err(AppError::internal("re-cut block vanished during copy"));
                        }
                        r.blocks_reused += 1;
                    }
                    f.blocks = blocks.clone();
                    f.hash = Some(hash.clone());
                    continue;
                }
                let src_blocks = std::mem::take(&mut f.blocks);
                let mut reader = BlockReader { cache: &mut cache, blocks: src_blocks.iter(), cur: Vec::new(), pos: 0 };
                let mut fh = FileHasher::new(dh.file_hash);
                let mut out = Vec::new();
                let mut err: Option<AppError> = None;
                let size = for_each_chunk(&mut reader, chunker.as_ref(), |chunk| {
                    fh.update(chunk);
                    let h = hash_bytes(dh.block_hash, chunk);
                    if let Err(e) = sink.add(&h, chunk, &mut r) {
                        err = Some(e);
                        return Ok(false);
                    }
                    out.push(h);
                    Ok(true)
                })
                .map_err(|e| AppError::transfer(format!("Cannot read {} from the source: {e}", f.path), "Run Verify on the source backup."))?;
                if let Some(e) = err {
                    return Err(e);
                }
                if size != f.size {
                    r.warnings.push(format!("{}: {size} bytes read, {} recorded", f.path, f.size));
                }
                let hash = fh.finish();
                recut.insert(key, (out.clone(), hash.clone()));
                f.blocks = out;
                f.hash = Some(hash);
            }
        }
        // Blocks are durable before a version refers to them.
        sink.flush()?;
        r.files += files.iter().filter(|f| f.kind == EntryKind::File).count() as u64;

        // Keep the version's time unless the destination has another version then.
        let mut ts: DateTime<Utc> = DateTime::parse_from_rfc3339(&fl.timestamp).map(|t| t.with_timezone(&Utc)).map_err(|_| AppError::validation(format!("{}: bad timestamp", fs.volume_name)))?;
        let existing = dst.db.all_filesets()?;
        let mut skip = false;
        while let Some(e) = existing.iter().find(|e| e.timestamp == ts.to_rfc3339() || e.volume_name == volume::filelist_volume_name(&dst.prefix, ts, &dst.cipher)) {
            if same_files(&dst.db.files(e.id)?, &files) {
                skip = true;
                break;
            }
            ts += chrono::Duration::seconds(1);
        }
        if skip {
            r.versions_skipped += 1;
            continue;
        }
        if ts.to_rfc3339() != fl.timestamp {
            r.warnings.push(format!("Version {} was copied as {} (the destination has another version at that time)", fl.timestamp, ts.to_rfc3339()));
        }
        let mut meta = fl.meta.clone();
        let mut st = meta.stats.unwrap_or_default();
        st.bytes_added = sink.scratch.bytes_uploaded - uploaded_before;
        st.new_blocks = r.blocks_copied - blocks_before;
        meta.stats = Some(st);
        let written = dst.write_filelist(ts, fl.partial, &files, fl.task_config.clone(), fl.control_files.clone(), meta, opts.dry_run)?;
        r.copied.push(written);
        r.versions_copied += 1;
    }
    r.bytes_uploaded = sink.scratch.bytes_uploaded;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::{Backend, LocalBackend};
    use crate::backup::crypto::KdfParams;
    use crate::backup::db::BackupDb;
    use crate::backup::test_util::write;
    use crate::backup::versions::{set_version_metadata, MetadataPatch};
    use crate::backup::{maintenance, run_backup, run_restore, ChunkerKind, RestoreOptions, RetentionPolicy, SourceFilter};
    use std::path::Path;

    fn data(len: usize, seed: u64) -> Vec<u8> {
        (0..len as u64).map(|i| ((i ^ seed).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 37) as u8).collect()
    }

    fn backup(src: &Path, be: &LocalBackend, db: &BackupDb, o: &BackupOptions, pass: Option<&str>) {
        let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), o, &RetentionPolicy::KeepAll, pass, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    fn tick() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    fn restore_newest(be: &LocalBackend, db: &BackupDb, o: &BackupOptions, pass: Option<&str>, out: &Path) {
        let repo = Repo::open(be, db, pass, o, false, KdfParams::insecure_fast()).unwrap();
        let rr = run_restore(&repo, &RestoreOptions { target: Some(out.into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty(), "{:?}", rr.errors);
        assert!(maintenance::verify(&repo, None, true).unwrap().errors.is_empty());
    }

    #[test]
    fn copies_selected_versions_and_only_missing_blocks() {
        let (src, d1, d2, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let o = BackupOptions { block_size: 1024, volume_size: 8192, verify_samples: 0, ..Default::default() };
        let (b1, b2) = (LocalBackend::new(d1.path()), LocalBackend::new(d2.path()));
        let (db1, db2) = (BackupDb::open_in_memory().unwrap(), BackupDb::open_in_memory().unwrap());
        write(src.path(), "a.bin", &data(9000, 1));
        backup(src.path(), &b1, &db1, &o, Some("pw"));
        tick();
        write(src.path(), "b.bin", &data(5000, 2));
        backup(src.path(), &b1, &db1, &o, Some("pw"));
        let r1 = Repo::open(&b1, &db1, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
        set_version_metadata(&r1, 0, &MetadataPatch { note: Some("release".into()), pinned: Some(true), ..Default::default() }).unwrap();
        // Destination: another passphrase (another key), unencrypted ZIP volumes would also work.
        let o2 = BackupOptions { volume_format: crate::backup::VolumeFormat::Zip, ..o.clone() };
        let r2 = Repo::open(&b2, &db2, Some("other"), &o2, true, KdfParams::insecure_fast()).unwrap();

        let dry = copy_versions(&r1, &r2, &CopyOptions { versions: vec![1], dry_run: true, ..Default::default() }, &o2, &OpContext::new()).unwrap();
        assert!(dry.dry_run && dry.versions_copied == 1 && dry.blocks_copied > 0 && dry.bytes_uploaded > 0);
        assert!(db2.filesets().unwrap().is_empty() && b2.list().unwrap().len() == 1, "dry run writes nothing (header only)");

        let a = copy_versions(&r1, &r2, &CopyOptions { versions: vec![1], ..Default::default() }, &o2, &OpContext::new()).unwrap();
        assert_eq!((a.versions_copied, a.blocks_copied, a.blocks_reused, a.rechunked), (1, dry.blocks_copied, 0, false));
        let b = copy_versions(&r1, &r2, &CopyOptions { versions: vec![0, 1], ..Default::default() }, &o2, &OpContext::new()).unwrap();
        assert_eq!((b.versions_copied, b.versions_skipped), (1, 1), "the version copied before is skipped");
        assert!(b.blocks_reused > 0 && b.blocks_copied > 0 && b.blocks_copied < a.blocks_copied + 10);
        let v = db2.filesets().unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].timestamp, db1.filesets().unwrap()[0].timestamp, "time kept");
        assert!(v[0].pinned && v[0].note.as_deref() == Some("release"), "metadata kept");
        assert_eq!(v[0].identity, db1.filesets().unwrap()[0].identity);
        assert!(v[0].stats.unwrap().bytes_added > 0);
        restore_newest(&b2, &db2, &o2, Some("other"), out.path());
        let root = out.path().join(src.path().file_name().unwrap());
        assert_eq!(std::fs::read(root.join("a.bin")).unwrap(), data(9000, 1));
        assert_eq!(std::fs::read(root.join("b.bin")).unwrap(), data(5000, 2));
        assert!(copy_versions(&r1, &r2, &CopyOptions { versions: vec![7], ..Default::default() }, &o2, &OpContext::new()).is_err());
    }

    #[test]
    fn copies_from_format_1_into_format_2_with_content_defined_chunking() {
        let (src, d1, d2, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let v1 = BackupOptions { block_size: 1024, volume_size: 8192, verify_samples: 0, format_version: Some(1), ..Default::default() };
        let v2 = BackupOptions { format_version: Some(2), chunker: Some(ChunkerKind::FastCdc), ..v1.clone() };
        let (b1, b2) = (LocalBackend::new(d1.path()), LocalBackend::new(d2.path()));
        let (db1, db2) = (BackupDb::open_in_memory().unwrap(), BackupDb::open_in_memory().unwrap());
        write(src.path(), "big.bin", &data(40_000, 3));
        write(src.path(), "copy.bin", &data(40_000, 3)); // same content: re-cut once
        write(src.path(), "empty.txt", b"");
        backup(src.path(), &b1, &db1, &v1, None);
        let r1 = Repo::open(&b1, &db1, None, &v1, false, KdfParams::insecure_fast()).unwrap();
        let r2 = Repo::open(&b2, &db2, Some("pw"), &v2, true, KdfParams::insecure_fast()).unwrap();
        assert!(r2.header.chunker_spec().is_content_defined());
        let rep = copy_versions(&r1, &r2, &CopyOptions { versions: vec![0], ..Default::default() }, &v2, &OpContext::new()).unwrap();
        assert!(rep.rechunked && rep.warnings.is_empty(), "{rep:?}");
        assert!(rep.blocks_reused > 0, "second identical file reuses the re-cut blocks");
        let files = db2.files(db2.filesets().unwrap()[0].id).unwrap();
        let big = files.iter().find(|f| f.path.ends_with("big.bin")).unwrap();
        let src_big = db1.files(db1.filesets().unwrap()[0].id).unwrap().into_iter().find(|f| f.path.ends_with("big.bin")).unwrap();
        assert_ne!(big.blocks, src_big.blocks, "blocks were cut again");
        restore_newest(&b2, &db2, &v2, Some("pw"), out.path());
        let root = out.path().join(src.path().file_name().unwrap());
        assert_eq!(std::fs::read(root.join("big.bin")).unwrap(), data(40_000, 3));
        assert_eq!(std::fs::read(root.join("empty.txt")).unwrap(), b"");
    }
}
