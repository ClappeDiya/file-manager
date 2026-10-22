//! File search index: names and (optionally) text content of the files
//! under chosen folders, in SQLite FTS5.
//!
//! The index lives in its own database file so a long indexing run never
//! holds the app's main connection. Indexing is incremental: unchanged
//! files (same size and mtime) are skipped and vanished ones removed, and
//! work is committed in batches so searches interleave with a running index.

use crate::core::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Text read per file for content search.
pub const DEFAULT_MAX_CONTENT_BYTES: u64 = 1024 * 1024;
const BATCH: usize = 500;
/// Folders never worth indexing.
const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", ".git", ".hg", ".svn", ".sync-rollback", ".sync-quarantine"];

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS roots (
    path TEXT PRIMARY KEY,
    content INTEGER NOT NULL,
    indexed_at TEXT,
    files INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    root TEXT NOT NULL,
    parent TEXT NOT NULL,
    name TEXT NOT NULL,
    is_dir INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL -- milliseconds
);
CREATE INDEX IF NOT EXISTS files_root ON files(root);
CREATE INDEX IF NOT EXISTS files_parent ON files(root, parent);
CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(name, content, tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3');
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexRoot {
    pub path: String,
    /// Text content is indexed (not just names).
    pub content: bool,
    pub indexed_at: Option<String>,
    pub files: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IndexStats {
    pub scanned: u64,
    pub added: u64,
    pub updated: u64,
    pub removed: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: i64,
    /// Matching text around the hit, with `[` `]` around matched terms.
    pub snippet: Option<String>,
    /// Protocol for an entry seen while browsing a server (`sftp`, `s3`…);
    /// `None` for local files.
    pub source: Option<String>,
}

/// An entry of a remote folder listing, recorded as the user browses.
#[derive(Debug, Clone)]
pub struct ListingEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_ms: i64,
}

/// Roots holding remote listings are named `remote:<protocol>`.
pub const REMOTE_ROOT_PREFIX: &str = "remote:";

fn remote_key(protocol: &str, path: &str) -> String {
    format!("{protocol}:{path}")
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    /// Only results under this folder.
    pub root: Option<String>,
    /// Search file contents (and names) instead of names only.
    #[serde(default)]
    pub content: bool,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn db_err(e: rusqlite::Error) -> AppError {
    AppError::internal(format!("Search index error: {e}"))
}

fn norm_root(root: &str) -> String {
    let t = root.trim_end_matches(['/', '\\']);
    if t.is_empty() { root.to_string() } else { t.to_string() }
}

/// `[lo, hi)` bounds selecting every path strictly under `root`.
fn under_bounds(root: &str) -> (String, String) {
    let lo = format!("{}{}", norm_root(root), MAIN_SEPARATOR);
    let mut hi = lo.clone();
    hi.pop();
    hi.push((MAIN_SEPARATOR as u8 + 1) as char);
    (lo, hi)
}

/// Each word of the user's text as an FTS5 prefix term, quoted so
/// operators and punctuation in it are taken literally.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Plain text worth indexing: no NUL bytes in the first 8 KiB.
/// Text for the content index: office documents and PDFs through
/// `doctext`, other files when they look like text.
fn read_text(path: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    if ufop_core::doctext::has_text(&ext) {
        return ufop_core::doctext::extract_text(path, max as usize).ok().flatten().filter(|t| !t.is_empty());
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.by_ref().take(max).read_to_end(&mut buf).ok()?;
    if buf[..buf.len().min(8192)].contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Milliseconds, so a same-size edit within the same second is noticed.
fn mtime_ms(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct SearchIndex {
    conn: Mutex<Connection>,
}

/// A changed entry waiting to be written.
struct Pending {
    path: String,
    parent: String,
    name: String,
    is_dir: bool,
    size: u64,
    mtime: i64,
    content: Option<String>,
    old_id: Option<i64>,
}

impl SearchIndex {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::internal(format!("Cannot create index folder: {e}")))?;
        }
        Self::init(Connection::open(path).map_err(db_err)?)
    }

    pub fn open_in_memory() -> Result<Self, AppError> {
        Self::init(Connection::open_in_memory().map_err(db_err)?)
    }

    fn init(conn: Connection) -> Result<Self, AppError> {
        conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        let index = Self { conn: Mutex::new(conn) };
        index.upgrade()?;
        Ok(index)
    }

    /// Index format 1 reads document text (PDF, office files): documents
    /// indexed before it are marked changed so the next run reads them.
    fn upgrade(&self) -> Result<(), AppError> {
        self.with(|c| {
            let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if version >= 1 {
                return Ok(());
            }
            let tx = c.transaction()?;
            let docs: Vec<i64> = {
                let mut st = tx.prepare("SELECT id, name FROM files WHERE is_dir = 0")?;
                let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
                rows.filter_map(Result::ok).filter(|(_, n)| Path::new(n).extension().is_some_and(|e| ufop_core::doctext::has_text(&e.to_string_lossy()))).map(|(id, _)| id).collect()
            };
            for id in docs {
                tx.execute("UPDATE files SET mtime = -1 WHERE id = ?1", [id])?;
            }
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, AppError> {
        let mut c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut c).map_err(db_err)
    }

    pub fn roots(&self) -> Result<Vec<IndexRoot>, AppError> {
        self.with(|c| {
            let mut st = c.prepare("SELECT path, content, indexed_at, files FROM roots ORDER BY path")?;
            let rows = st.query_map([], |r| {
                Ok(IndexRoot { path: r.get(0)?, content: r.get::<_, i64>(1)? != 0, indexed_at: r.get(2)?, files: r.get::<_, i64>(3)? as u64 })
            })?;
            rows.collect()
        })
    }

    /// The indexed root covering `path`, if any (the deepest one).
    pub fn root_covering(&self, path: &str) -> Result<Option<IndexRoot>, AppError> {
        let p = norm_root(path);
        Ok(self
            .roots()?
            .into_iter()
            .filter(|r| p == r.path || p.starts_with(&under_bounds(&r.path).0))
            .max_by_key(|r| r.path.len()))
    }

    pub fn add_root(&self, root: &str, content: bool) -> Result<IndexRoot, AppError> {
        let root = norm_root(root);
        self.with(|c| {
            c.execute(
                "INSERT INTO roots (path, content) VALUES (?1, ?2) ON CONFLICT(path) DO UPDATE SET content = excluded.content",
                params![root, content as i64],
            )
        })?;
        self.roots()?.into_iter().find(|r| r.path == root).ok_or_else(|| AppError::internal("Root not stored"))
    }

    pub fn remove_root(&self, root: &str) -> Result<(), AppError> {
        let root = norm_root(root);
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM fts WHERE rowid IN (SELECT id FROM files WHERE root = ?1)", [&root])?;
            tx.execute("DELETE FROM files WHERE root = ?1", [&root])?;
            tx.execute("DELETE FROM roots WHERE path = ?1", [&root])?;
            tx.commit()
        })
    }

    /// Bring the index of `root` up to date. `progress` gets the number of
    /// entries scanned so far; `cancel` stops after the current batch.
    pub fn index_root(&self, root: &str, cancel: &AtomicBool, progress: &AtomicU64) -> Result<IndexStats, AppError> {
        let root = norm_root(root);
        if root.starts_with(REMOTE_ROOT_PREFIX) {
            return Err(AppError::validation("Server listings are recorded as you browse; they can't be re-scanned"));
        }
        let Some(meta) = self.roots()?.into_iter().find(|r| r.path == root) else {
            return Err(AppError::validation(format!("{root} is not an indexed folder")));
        };
        if !Path::new(&root).is_dir() {
            return Err(AppError::file_op(format!("Folder not found: {root}"), "Remove it from the search index, or reconnect the drive."));
        }
        let known: HashMap<String, (i64, u64, i64)> = self.with(|c| {
            let mut st = c.prepare("SELECT path, id, size, mtime FROM files WHERE root = ?1")?;
            let rows = st.query_map([&root], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get::<_, i64>(2)? as u64, r.get(3)?))))?;
            rows.collect()
        })?;
        let mut seen = std::collections::HashSet::with_capacity(known.len());
        let mut stats = IndexStats::default();
        let mut pending: Vec<Pending> = Vec::new();

        let flush = |pending: &mut Vec<Pending>| -> Result<(), AppError> {
            if pending.is_empty() {
                return Ok(());
            }
            let batch = std::mem::take(pending);
            self.with(|c| {
                let tx = c.transaction()?;
                for Pending { path, parent, name, is_dir, size, mtime, content, old_id } in batch {
                    if let Some(id) = old_id {
                        tx.execute("DELETE FROM fts WHERE rowid = ?1", [id])?;
                        tx.execute("UPDATE files SET size = ?2, mtime = ?3, is_dir = ?4 WHERE id = ?1", params![id, size as i64, mtime, is_dir as i64])?;
                        tx.execute("INSERT INTO fts (rowid, name, content) VALUES (?1, ?2, ?3)", params![id, name, content])?;
                    } else {
                        tx.execute(
                            "INSERT INTO files (path, root, parent, name, is_dir, size, mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![path, root, parent, name, is_dir as i64, size as i64, mtime],
                        )?;
                        let id = tx.last_insert_rowid();
                        tx.execute("INSERT INTO fts (rowid, name, content) VALUES (?1, ?2, ?3)", params![id, name, content])?;
                    }
                }
                tx.commit()
            })
        };

        let mut stack = vec![PathBuf::from(&root)];
        'walk: while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                if cancel.load(Ordering::Relaxed) {
                    stats.cancelled = true;
                    break 'walk;
                }
                let name = e.file_name().to_string_lossy().to_string();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                let is_dir = ft.is_dir();
                if is_dir && (SKIP_DIRS.contains(&name.as_str()) || name.starts_with(".ufop")) {
                    continue;
                }
                let Ok(m) = e.metadata() else { continue };
                let path = e.path().to_string_lossy().to_string();
                let (size, mtime) = (if is_dir { 0 } else { m.len() }, mtime_ms(&m));
                stats.scanned += 1;
                progress.store(stats.scanned, Ordering::Relaxed);
                if is_dir {
                    stack.push(e.path());
                }
                let old = known.get(&path);
                seen.insert(path.clone());
                if old.is_some_and(|&(_, s, t)| s == size && t == mtime) {
                    continue;
                }
                let content = (meta.content && !is_dir).then(|| read_text(&e.path(), DEFAULT_MAX_CONTENT_BYTES)).flatten();
                match old {
                    Some(_) => stats.updated += 1,
                    None => stats.added += 1,
                }
                pending.push(Pending { path, parent: dir.to_string_lossy().to_string(), name, is_dir, size, mtime, content, old_id: old.map(|o| o.0) });
                if pending.len() >= BATCH {
                    flush(&mut pending)?;
                }
            }
        }
        flush(&mut pending)?;

        if !stats.cancelled {
            let gone: Vec<i64> = known.iter().filter(|(p, _)| !seen.contains(*p)).map(|(_, v)| v.0).collect();
            stats.removed = gone.len() as u64;
            for chunk in gone.chunks(BATCH) {
                self.with(|c| {
                    let tx = c.transaction()?;
                    for id in chunk {
                        tx.execute("DELETE FROM fts WHERE rowid = ?1", [id])?;
                        tx.execute("DELETE FROM files WHERE id = ?1", [id])?;
                    }
                    tx.commit()
                })?;
            }
            let now = chrono::Utc::now().to_rfc3339();
            self.with(|c| {
                c.execute(
                    "UPDATE roots SET indexed_at = ?2, files = (SELECT COUNT(*) FROM files WHERE root = ?1) WHERE path = ?1",
                    params![root, now],
                )
            })?;
        }
        Ok(stats)
    }

    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>, AppError> {
        let limit = q.limit.unwrap_or(200).clamp(1, 5000) as i64;
        let (lo, hi) = match q.root.as_deref().filter(|r| !r.is_empty()) {
            Some(r) => under_bounds(r),
            None => (String::new(), "\u{10FFFF}".to_string()),
        };
        // Columns: path, name, is_dir, size, mtime, root[, snippet].
        let hit = |r: &rusqlite::Row, snippet: bool| -> rusqlite::Result<SearchHit> {
            let root: String = r.get(5)?;
            let source = root.strip_prefix(REMOTE_ROOT_PREFIX).map(str::to_string);
            let path: String = r.get(0)?;
            Ok(SearchHit {
                // Remote rows are keyed `<protocol>:<path>`; show the path.
                path: match &source {
                    Some(p) => path.strip_prefix(&format!("{p}:")).unwrap_or(&path).to_string(),
                    None => path,
                },
                name: r.get(1)?,
                is_dir: r.get::<_, i64>(2)? != 0,
                size: r.get::<_, i64>(3)? as u64,
                modified: r.get::<_, i64>(4)? / 1000,
                snippet: if snippet { r.get::<_, Option<String>>(6)?.filter(|s| !s.is_empty()) } else { None },
                source,
            })
        };
        if q.content {
            let Some(m) = fts_query(&q.text) else { return Ok(Vec::new()) };
            self.with(|c| {
                let mut st = c.prepare(
                    "SELECT f.path, f.name, f.is_dir, f.size, f.mtime, f.root, snippet(fts, 1, '[', ']', '…', 12) \
                     FROM fts JOIN files f ON f.id = fts.rowid \
                     WHERE fts MATCH ?1 AND f.path >= ?2 AND f.path < ?3 ORDER BY rank LIMIT ?4",
                )?;
                let rows = st.query_map(params![m, lo, hi, limit], |r| hit(r, true))?;
                rows.collect()
            })
        } else {
            let text = q.text.trim();
            if text.is_empty() {
                return Ok(Vec::new());
            }
            let pattern = format!("%{}%", like_escape(text));
            self.with(|c| {
                let mut st = c.prepare(
                    "SELECT path, name, is_dir, size, mtime, root FROM files \
                     WHERE name LIKE ?1 ESCAPE '\\' AND path >= ?2 AND path < ?3 \
                     ORDER BY (lower(name) = lower(?4)) DESC, length(name), name LIMIT ?5",
                )?;
                let rows = st.query_map(params![pattern, lo, hi, text, limit], |r| hit(r, false))?;
                rows.collect()
            })
        }
    }

    /// Replace what is known about one remote folder with a fresh listing
    /// (names only), so global search finds files seen on servers.
    pub fn record_listing(&self, protocol: &str, dir: &str, entries: &[ListingEntry]) -> Result<(), AppError> {
        let root = format!("{REMOTE_ROOT_PREFIX}{protocol}");
        let parent = remote_key(protocol, dir.trim_end_matches('/'));
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("INSERT OR IGNORE INTO roots (path, content) VALUES (?1, 0)", [&root])?;
            tx.execute("DELETE FROM fts WHERE rowid IN (SELECT id FROM files WHERE root = ?1 AND parent = ?2)", params![root, parent])?;
            tx.execute("DELETE FROM files WHERE root = ?1 AND parent = ?2", params![root, parent])?;
            for e in entries {
                tx.execute(
                    "INSERT INTO files (path, root, parent, name, is_dir, size, mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(path) DO UPDATE SET parent = excluded.parent, name = excluded.name, is_dir = excluded.is_dir, size = excluded.size, mtime = excluded.mtime",
                    params![remote_key(protocol, &e.path), root, parent, e.name, e.is_dir as i64, e.size as i64, e.modified_ms],
                )?;
                let id: i64 = tx.query_row("SELECT id FROM files WHERE path = ?1", [remote_key(protocol, &e.path)], |r| r.get(0))?;
                tx.execute("DELETE FROM fts WHERE rowid = ?1", [id])?;
                tx.execute("INSERT INTO fts (rowid, name, content) VALUES (?1, ?2, NULL)", params![id, e.name])?;
            }
            tx.execute(
                "UPDATE roots SET indexed_at = ?2, files = (SELECT COUNT(*) FROM files WHERE root = ?1) WHERE path = ?1",
                params![root, chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()
        })
    }

    pub fn file_count(&self) -> Result<u64, AppError> {
        self.with(|c| c.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0)).optional())
            .map(|n| n.unwrap_or(0) as u64)
    }
}

/// A background indexing run, as the UI sees it.
#[derive(Debug, Clone, Serialize)]
pub struct IndexJob {
    pub root: String,
    pub scanned: u64,
    pub running: bool,
    pub last: Option<IndexStats>,
    pub error: Option<String>,
}

struct JobState {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    last: Arc<Mutex<(Option<IndexStats>, Option<String>)>>,
}

/// The index plus its background jobs (managed Tauri state).
pub struct SearchManager {
    index: Arc<SearchIndex>,
    jobs: Mutex<HashMap<String, JobState>>,
}

/// Re-index a searched root in the background once it is this old.
pub const STALE_AFTER_SECS: i64 = 10 * 60;

impl SearchManager {
    pub fn new(index: SearchIndex) -> Self {
        Self { index: Arc::new(index), jobs: Mutex::new(HashMap::new()) }
    }

    /// `<data dir>/search-index.sqlite`, or in memory if that fails.
    pub fn open_default() -> Self {
        let path = directories::ProjectDirs::from("com", "ufop", "unified-file-ops").map(|d| d.data_dir().join("search-index.sqlite"));
        let index = path
            .and_then(|p| SearchIndex::open(&p).map_err(|e| tracing::warn!("Search index unavailable on disk: {e}")).ok())
            .unwrap_or_else(|| SearchIndex::open_in_memory().expect("in-memory SQLite"));
        Self::new(index)
    }

    pub fn index(&self) -> &SearchIndex {
        &self.index
    }

    /// The index, for queries run off the async runtime.
    pub fn index_arc(&self) -> Arc<SearchIndex> {
        self.index.clone()
    }

    /// Start (or keep) a background index run for `root`.
    pub fn start(&self, root: &str) {
        let root = norm_root(root);
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let job = jobs.entry(root.clone()).or_insert_with(|| JobState {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            last: Arc::new(Mutex::new((None, None))),
        });
        if job.running.swap(true, Ordering::SeqCst) {
            return;
        }
        job.cancel.store(false, Ordering::SeqCst);
        job.progress.store(0, Ordering::SeqCst);
        let (index, cancel, progress, running, last) = (self.index.clone(), job.cancel.clone(), job.progress.clone(), job.running.clone(), job.last.clone());
        std::thread::spawn(move || {
            let out = index.index_root(&root, &cancel, &progress);
            let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
            *l = match out {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(e.user_message())),
            };
            running.store(false, Ordering::SeqCst);
        });
    }

    pub fn cancel(&self, root: &str) {
        if let Some(j) = self.jobs.lock().unwrap_or_else(|e| e.into_inner()).get(&norm_root(root)) {
            j.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn jobs(&self) -> Vec<IndexJob> {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(root, j)| {
                let (last, error) = j.last.lock().unwrap_or_else(|e| e.into_inner()).clone();
                IndexJob { root: root.clone(), scanned: j.progress.load(Ordering::Relaxed), running: j.running.load(Ordering::Relaxed), last, error }
            })
            .collect()
    }

    pub fn is_running(&self, root: &str) -> bool {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).get(&norm_root(root)).is_some_and(|j| j.running.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests;
