//! Read backed-up files without restoring them: preview, `cat`, archive
//! export and the mounted (WebDAV) view of a version.
//!
//! - [`BlockSource`] finds a block's volume in the local database, downloads
//!   the volume when it is first needed (through the [volume cache](super::cache)
//!   when one is set), decrypts and unpacks it and keeps a few unpacked
//!   volumes in memory, bounded by bytes, for the next reads.
//! - [`FileReader`] is `Read + Seek` over one file: a seek is a binary search
//!   over the file's block offsets, so reading a range downloads only the
//!   volumes holding that range.
//! - [`VersionTree`] is a version's files as a folder tree (paths below the
//!   version's common folder), for listings and the mount.
//!
//! A `BlockSource` either borrows the backend of an open [`Repo`] or owns a
//! shared handle to it (`'static`), so a long-lived server can keep one.

use super::backend::Backend;
use super::cache::VolumeCache;
use super::crypto::VolumeCipher;
use super::engine::{Repo, VolumeFetcher};
use super::layout::{LayoutBackend, RepoLayout};
use super::volume::{self, VolumeKind};
use super::{EntryKind, FileEntry};
use crate::error::AppError;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Unpacked volumes kept in memory by default (bytes).
pub const DEFAULT_MEMORY_BYTES: u64 = 192 * 1024 * 1024;

enum BackendRef<'a> {
    Borrowed(&'a dyn Backend),
    Shared(Arc<dyn Backend>),
}

impl BackendRef<'_> {
    fn get(&self) -> &dyn Backend {
        match self {
            Self::Borrowed(b) => *b,
            Self::Shared(b) => b.as_ref(),
        }
    }
}

type Blocks = HashMap<String, Arc<Vec<u8>>>;

#[derive(Default)]
struct Unpacked {
    order: VecDeque<String>,
    volumes: HashMap<String, (Arc<Blocks>, u64)>,
    bytes: u64,
}

/// Block lookup and lazy volume download for reading files of a repository.
pub struct BlockSource<'a> {
    backend: BackendRef<'a>,
    layout: RepoLayout,
    prefix: String,
    cipher: VolumeCipher,
    case_insensitive: bool,
    /// Block hash → (volume, raw size).
    blocks: HashMap<String, (Arc<str>, u64)>,
    /// Volume → recorded SHA-256.
    hashes: HashMap<String, String>,
    cache: Option<(Arc<VolumeCache>, String)>,
    memory: Mutex<Unpacked>,
    max_memory: u64,
    downloads: AtomicU64,
}

impl<'a> BlockSource<'a> {
    /// Reader over an open repository (borrows its backend).
    pub fn new(repo: &Repo<'a>) -> Result<Self, AppError> {
        Self::build(repo, BackendRef::Borrowed(repo.backend.inner()))
    }

    fn build(repo: &Repo<'_>, backend: BackendRef<'a>) -> Result<Self, AppError> {
        let mut blocks = HashMap::new();
        for (vol, list) in repo.db.blocks_by_volume()? {
            let v: Arc<str> = Arc::from(vol.as_str());
            for (h, size) in list {
                blocks.insert(h, (v.clone(), size));
            }
        }
        let hashes = repo.db.volumes(Some(VolumeKind::Block))?.into_iter().filter(|v| !v.hash.is_empty()).map(|v| (v.name, v.hash)).collect();
        Ok(Self {
            backend,
            layout: repo.header.layout,
            prefix: repo.prefix.clone(),
            cipher: repo.cipher.clone(),
            case_insensitive: repo.case_insensitive,
            blocks,
            hashes,
            cache: repo.cache_entry(),
            memory: Mutex::default(),
            max_memory: DEFAULT_MEMORY_BYTES,
            downloads: AtomicU64::new(0),
        })
    }

    /// Neither read nor fill the volume cache (large one-pass exports).
    pub fn without_cache(mut self) -> Self {
        self.cache = None;
        self
    }

    /// Limit the unpacked volumes kept in memory (at least one is kept).
    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.max_memory = bytes;
        self
    }

    /// Volumes downloaded (or taken from the cache) so far.
    pub fn volumes_loaded(&self) -> u64 {
        self.downloads.load(Ordering::Relaxed)
    }

    /// Raw size of a block, when the database knows it.
    pub fn block_size(&self, hash: &str) -> Option<u64> {
        self.blocks.get(hash).map(|b| b.1)
    }

    pub fn has_block(&self, hash: &str) -> bool {
        self.blocks.contains_key(hash)
    }

    fn load_volume(&self, name: &str) -> Result<Arc<Blocks>, AppError> {
        if let Some(v) = self.memory.lock().unwrap_or_else(|e| e.into_inner()).volumes.get(name) {
            return Ok(v.0.clone());
        }
        let backend = LayoutBackend::new(self.backend.get(), self.layout, &self.prefix);
        let fetcher = VolumeFetcher::new(&backend, self.case_insensitive, self.cache.as_ref().map(|(c, ns)| (c.as_ref(), ns.as_str())), true);
        let sealed = fetcher.fetch(name, self.hashes.get(name).map(String::as_str), &[], false)?;
        self.downloads.fetch_add(1, Ordering::Relaxed);
        let map: Blocks = volume::read_block_volume(&self.cipher.open(&sealed)?)?.into_iter().map(|(h, d)| (h, Arc::new(d))).collect();
        let size: u64 = map.values().map(|d| d.len() as u64).sum();
        let map = Arc::new(map);
        let mut m = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        if !m.volumes.contains_key(name) {
            m.order.push_back(name.to_string());
            m.volumes.insert(name.to_string(), (map.clone(), size));
            m.bytes += size;
            while m.bytes > self.max_memory && m.order.len() > 1 {
                if let Some(old) = m.order.pop_front() {
                    if let Some((_, s)) = m.volumes.remove(&old) {
                        m.bytes -= s;
                    }
                }
            }
        }
        Ok(map)
    }

    /// One block's raw (decompressed) data.
    pub fn block(&self, hash: &str) -> Result<Arc<Vec<u8>>, AppError> {
        let short = &hash[..12.min(hash.len())];
        let (vol, _) = self
            .blocks
            .get(hash)
            .ok_or_else(|| AppError::transfer(format!("data block {short} is not in any known volume"), "Run Verify or list-broken-files on this backup."))?;
        self.load_volume(vol)?
            .get(hash)
            .cloned()
            .ok_or_else(|| AppError::transfer(format!("data block {short} is missing from volume {vol}"), "Run Repair on this backup."))
    }
}

impl BlockSource<'static> {
    /// Reader that owns a handle to the destination, for servers that outlive the [`Repo`].
    pub fn shared(repo: &Repo<'_>, backend: Arc<dyn Backend>) -> Result<Self, AppError> {
        Self::build(repo, BackendRef::Shared(backend))
    }
}

/// `Read + Seek` over one backed-up file.
pub struct FileReader<'s> {
    src: &'s BlockSource<'s>,
    /// (block hash, start offset, length), in file order.
    blocks: Vec<(String, u64, u64)>,
    size: u64,
    pos: u64,
    current: Option<(usize, Arc<Vec<u8>>)>,
}

/// Open a file entry of a version for reading.
pub fn open_file<'s>(src: &'s BlockSource<'s>, entry: &FileEntry) -> Result<FileReader<'s>, AppError> {
    if entry.kind != EntryKind::File {
        return Err(AppError::validation(format!("{} is not a file", entry.path)));
    }
    let mut blocks = Vec::with_capacity(entry.blocks.len());
    let mut off = 0u64;
    for h in &entry.blocks {
        let len = src
            .block_size(h)
            .ok_or_else(|| AppError::transfer(format!("{}: data block {} is not in any known volume", entry.path, &h[..12.min(h.len())]), "Run Verify or list-broken-files on this backup."))?;
        blocks.push((h.clone(), off, len));
        off += len;
    }
    if off != entry.size {
        return Err(AppError::transfer(format!("{}: its blocks add up to {off} bytes, the file list says {}", entry.path, entry.size), "Run Verify on this backup."));
    }
    Ok(FileReader { src, blocks, size: off, pos: 0, current: None })
}

impl FileReader<'_> {
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Up to `len` bytes from `offset` (fewer at the end of the file).
    pub fn read_range(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, AppError> {
        let end = offset.saturating_add(len).min(self.size);
        let mut out = Vec::with_capacity(end.saturating_sub(offset) as usize);
        self.seek(SeekFrom::Start(offset.min(self.size)))?;
        (&mut *self).take(end.saturating_sub(offset)).read_to_end(&mut out)?;
        Ok(out)
    }

    /// Copy the whole file to `w`; returns the bytes written.
    pub fn copy_to(&mut self, w: &mut dyn std::io::Write) -> Result<u64, AppError> {
        self.seek(SeekFrom::Start(0))?;
        Ok(std::io::copy(self, w)?)
    }
}

impl Read for FileReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.size || buf.is_empty() {
            return Ok(0);
        }
        // The block holding `pos`: the last one starting at or before it.
        let idx = match self.blocks.binary_search_by(|b| b.1.cmp(&self.pos)) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let data = match &self.current {
            Some((i, d)) if *i == idx => d.clone(),
            _ => {
                let d = self.src.block(&self.blocks[idx].0).map_err(|e| std::io::Error::other(e.user_message()))?;
                if d.len() as u64 != self.blocks[idx].2 {
                    return Err(std::io::Error::other(format!("data block {} has an unexpected size", &self.blocks[idx].0[..12.min(self.blocks[idx].0.len())])));
                }
                self.current = Some((idx, d.clone()));
                d
            }
        };
        let within = (self.pos - self.blocks[idx].1) as usize;
        let n = buf.len().min(data.len() - within);
        buf[..n].copy_from_slice(&data[within..within + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for FileReader<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.size.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let p = p.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start of the file"))?;
        self.pos = p;
        Ok(p)
    }
}

/// Find one entry of a version by its backed-up path (`/` and `\` match each other).
pub fn find_entry<'e>(entries: &'e [FileEntry], path: &str) -> Option<&'e FileEntry> {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_string();
    let want = norm(path);
    entries.iter().find(|e| norm(&e.path) == want)
}

/// Path components of a backed-up path: `/a/b` → `[a, b]`, `C:\a` → `[C, a]`.
pub fn components(path: &str) -> Vec<String> {
    path.split(['/', '\\']).filter(|s| !s.is_empty()).map(|s| s.strip_suffix(':').unwrap_or(s).to_string()).filter(|s| !s.is_empty()).collect()
}

/// A node of a [`VersionTree`].
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: Option<i64>,
    /// Index into [`VersionTree::entries`]; `None` for folders that exist
    /// only as parents of backed-up entries.
    pub entry: Option<usize>,
    /// Child names → virtual paths.
    pub children: BTreeMap<String, String>,
}

/// A version's entries as a folder tree. Paths are `/`-separated and
/// relative to the deepest folder that holds every entry (`""` = root).
#[derive(Debug, Clone)]
pub struct VersionTree {
    pub entries: Vec<FileEntry>,
    nodes: HashMap<String, TreeNode>,
}

impl VersionTree {
    pub fn new(entries: Vec<FileEntry>) -> Self {
        let comps: Vec<Vec<String>> = entries.iter().map(|e| components(&e.path)).collect();
        // Common parent of every entry (an entry never collapses into itself).
        let mut common: Option<Vec<String>> = None;
        for c in &comps {
            let parent = &c[..c.len().saturating_sub(1)];
            common = Some(match common {
                None => parent.to_vec(),
                Some(cur) => cur.iter().zip(parent).take_while(|(a, b)| a == b).map(|(a, _)| a.clone()).collect(),
            });
        }
        let skip = common.map(|c| c.len()).unwrap_or(0);
        let mut nodes: HashMap<String, TreeNode> = HashMap::new();
        let dir = |name: &str| TreeNode { name: name.to_string(), kind: EntryKind::Dir, size: 0, mtime: None, entry: None, children: BTreeMap::new() };
        nodes.insert(String::new(), dir(""));
        for (i, c) in comps.iter().enumerate() {
            let rel = &c[skip.min(c.len())..];
            if rel.is_empty() {
                continue;
            }
            let mut parent = String::new();
            for (depth, name) in rel.iter().enumerate() {
                let path = if parent.is_empty() { name.clone() } else { format!("{parent}/{name}") };
                nodes.entry(parent.clone()).or_insert_with(|| dir("")).children.insert(name.clone(), path.clone());
                let n = nodes.entry(path.clone()).or_insert_with(|| dir(name));
                if depth == rel.len() - 1 {
                    let e = &entries[i];
                    n.kind = e.kind;
                    n.size = e.size;
                    n.mtime = e.mtime;
                    n.entry = Some(i);
                }
                parent = path;
            }
        }
        Self { entries, nodes }
    }

    /// The node at a virtual path (`""` or `/` = root).
    pub fn node(&self, path: &str) -> Option<&TreeNode> {
        self.nodes.get(components(path).join("/").as_str())
    }

    pub fn entry(&self, node: &TreeNode) -> Option<&FileEntry> {
        node.entry.and_then(|i| self.entries.get(i))
    }

    /// Children of a folder: (virtual path, node), sorted by name.
    pub fn children(&self, path: &str) -> Vec<(&str, &TreeNode)> {
        let Some(n) = self.node(path) else { return Vec::new() };
        n.children.values().filter_map(|p| self.nodes.get(p).map(|c| (p.as_str(), c))).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::db::BackupDb;
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, BackupOptions, OpContext, RetentionPolicy, SourceFilter};

    pub(crate) fn sample_data(n: usize) -> Vec<u8> {
        (0..n as u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect()
    }

    /// Back up `files` (relative path → content) into a fresh repository.
    pub(crate) fn repo_with(files: &[(&str, &[u8])]) -> (tempfile::TempDir, tempfile::TempDir, BackupDb) {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for (p, d) in files {
            write(src.path(), p, d);
        }
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { block_size: 1024, volume_size: 4096, ..Default::default() };
        let r = run_backup(&LocalBackend::new(dst.path()), &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        (src, dst, db)
    }

    #[test]
    fn read_and_seek_across_blocks_and_volumes() {
        let big = sample_data(20_000);
        let (src, dst, db) = repo_with(&[("docs/big.bin", &big), ("docs/small.txt", b"hello")]);
        let be = LocalBackend::new(dst.path());
        let repo = Repo::open(&be, &db, Some("pw"), &BackupOptions::default(), false, KdfParams::insecure_fast()).unwrap();
        let bs = BlockSource::new(&repo).unwrap().with_memory_limit(1);
        let entries = db.files(db.fileset_by_version(0).unwrap().id).unwrap();
        let e = find_entry(&entries, &format!("{}/docs/big.bin", src.path().display())).unwrap();
        let mut r = open_file(&bs, e).unwrap();
        assert_eq!(r.size(), 20_000);
        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        assert_eq!(all, big);
        // Ranges anywhere, including across block boundaries and past the end.
        for (off, len) in [(0u64, 10u64), (1000, 100), (1023, 2), (5000, 7000), (19_990, 100), (25_000, 5)] {
            let got = r.read_range(off, len).unwrap();
            let s = (off as usize).min(big.len());
            let e = ((off + len) as usize).min(big.len());
            assert_eq!(got, &big[s..e], "range {off}+{len}");
        }
        r.seek(SeekFrom::End(-5)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &big[big.len() - 5..]);
        assert!(r.seek(SeekFrom::Current(-100_000)).is_err());
        assert!(bs.volumes_loaded() >= 2, "the data spans several volumes");
        let small = find_entry(&entries, &format!("{}\\docs\\small.txt", src.path().display()).replace('/', "\\")).unwrap();
        let mut out = Vec::new();
        open_file(&bs, small).unwrap().copy_to(&mut out).unwrap();
        assert_eq!(out, b"hello");
        let dir = find_entry(&entries, &format!("{}/docs/", src.path().display())).unwrap();
        assert!(open_file(&bs, dir).is_err(), "folders are not files");
    }

    #[test]
    fn reading_uses_the_volume_cache() {
        let big = sample_data(9_000);
        let (src, dst, db) = repo_with(&[("f.bin", &big)]);
        let be = LocalBackend::new(dst.path());
        let cdir = tempfile::tempdir().unwrap();
        let cache = Arc::new(VolumeCache::new(cdir.path(), 1 << 30).unwrap());
        let read = || {
            let repo = Repo::open(&be, &db, Some("pw"), &BackupOptions::default(), false, KdfParams::insecure_fast()).unwrap().with_cache(Some(cache.clone()));
            let bs = BlockSource::new(&repo).unwrap();
            let entries = db.files(db.fileset_by_version(0).unwrap().id).unwrap();
            let e = find_entry(&entries, &format!("{}/f.bin", src.path().display())).unwrap().clone();
            let mut v = Vec::new();
            open_file(&bs, &e).unwrap().read_to_end(&mut v).unwrap();
            v
        };
        assert_eq!(read(), big);
        let after_first = cache.stats();
        assert!(after_first.entries >= 1 && after_first.hits == 0);
        // Volumes gone from the destination: the cache alone serves them.
        for f in std::fs::read_dir(dst.path()).unwrap().flatten() {
            if f.file_name().to_string_lossy().contains(".dblock") {
                std::fs::remove_file(f.path()).unwrap();
            }
        }
        assert_eq!(read(), big);
        assert!(cache.stats().hits >= after_first.entries);
    }

    #[test]
    fn tree_collapses_to_the_common_folder() {
        let f = |p: &str, k: EntryKind| FileEntry { path: p.into(), kind: k, size: 3, mtime: Some(5), mode: None, hash: None, blocks: vec![], symlink_target: None, meta: None };
        let t = VersionTree::new(vec![
            f("/home/u/docs", EntryKind::Dir),
            f("/home/u/docs/a.txt", EntryKind::File),
            f("/home/u/docs/sub/b.txt", EntryKind::File),
            f("/home/u/pics", EntryKind::Dir),
        ]);
        let root: Vec<&str> = t.children("/").iter().map(|c| c.0).collect();
        assert_eq!(root, vec!["docs", "pics"]);
        assert_eq!(t.children("docs").iter().map(|c| c.1.name.as_str()).collect::<Vec<_>>(), vec!["a.txt", "sub"]);
        let a = t.node("/docs/a.txt").unwrap();
        assert_eq!((a.kind, a.size), (EntryKind::File, 3));
        assert_eq!(t.entry(a).unwrap().path, "/home/u/docs/a.txt");
        // `sub` was not backed up as an entry of its own but appears as a folder.
        assert_eq!(t.node("docs/sub").unwrap().kind, EntryKind::Dir);
        assert!(t.node("docs/nope").is_none());
        let w = VersionTree::new(vec![f(r"C:\data", EntryKind::Dir), f(r"C:\data\x.txt", EntryKind::File)]);
        assert_eq!(w.children("").iter().map(|c| c.0).collect::<Vec<_>>(), vec!["data"]);
        assert!(w.node("data/x.txt").is_some());
        assert_eq!(components(r"C:\a\b"), vec!["C", "a", "b"]);
    }
}
