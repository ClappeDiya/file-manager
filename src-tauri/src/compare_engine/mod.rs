//! Standalone folder compare and reconcile (local folders and saved
//! connections, in any combination).
//!
//! The comparison itself is `ufop_core::sync::compare` (the sync engine's
//! filter engine and comparer). This module lists the two sides — a local
//! folder with `read_dir`, a connection with its connector's `list_remote`
//! — with a bounded number of listings in flight ([`crate::connectors::walk`]),
//! as a cancellable task that reports progress, keeps the latest results so
//! the user can act on them ([`CompareManager`]), and runs those actions
//! ([`apply`]).

pub mod apply;

use crate::connectors::{ConnectionManager, Connector, ConnectorRegistry};
use crate::core::error::AppError;
use crate::sync_engine::compare::{self, CompareEntry, CompareOptions, CompareOutput, ListingError, LocalSidesHasher, SideListing};
use crate::sync_engine::planner::FileMeta;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use ufop_core::sync::versions::is_safe_relative;

/// Event carrying [`CompareProgress`] while a comparison or action runs.
pub const COMPARE_PROGRESS_EVENT: &str = "compare-progress";

/// Folder listings in flight per side.
pub const LIST_CONCURRENCY: usize = 8;

/// Comparisons whose results are kept for acting on.
const KEEP_RESULTS: usize = 8;

/// One side of a comparison, as the UI names it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompareSide {
    /// A folder on this computer.
    Local { path: String },
    /// A folder on a saved connection (`connection_id`), or on a connected
    /// protocol without a saved connection (`protocol` only).
    Remote {
        #[serde(default)]
        connection_id: Option<String>,
        #[serde(default)]
        protocol: Option<String>,
        path: String,
    },
}

/// Where a connection listed each entry of a comparison (its
/// `FileEntry.path`), by relative path. Connectors address items by these
/// paths, which are not always folder names joined up: Google Drive's are
/// chains of item IDs.
pub type ListedPaths = std::collections::HashMap<String, String>;

/// A side ready to list and change.
#[derive(Clone)]
pub enum ResolvedSide {
    Local { root: PathBuf },
    Remote {
        connector: Arc<dyn Connector>,
        protocol: String,
        name: String,
        root: String,
        connection_id: Option<uuid::Uuid>,
        /// What the connection listed so far (see [`ListedPaths`]).
        listed: Arc<std::sync::Mutex<ListedPaths>>,
    },
}

impl ResolvedSide {
    /// Connect a side named by the UI.
    pub async fn resolve(side: &CompareSide, registry: &ConnectorRegistry, connections: &ConnectionManager) -> Result<Self, AppError> {
        match side {
            CompareSide::Local { path } => {
                let root = PathBuf::from(path);
                if !root.is_dir() {
                    return Err(AppError::file_op(format!("Folder not found: {path}"), "Check that the folder exists, then compare again."));
                }
                Ok(Self::Local { root })
            }
            CompareSide::Remote { connection_id, protocol, path } => {
                let cid = connection_id.as_deref().filter(|c| !c.is_empty());
                let (protocol, name, id) = match cid {
                    Some(cid) => {
                        let id = uuid::Uuid::parse_str(cid).map_err(|_| AppError::validation(format!("Invalid connection id '{cid}'")))?;
                        let profile = connections.get_connection(id).await?.ok_or_else(|| AppError::not_found(format!("Saved connection {cid} not found")))?;
                        (profile.protocol.as_str().to_string(), profile.name, Some(id))
                    }
                    None => {
                        let protocol = protocol.clone().filter(|p| !p.is_empty()).ok_or_else(|| AppError::validation("A remote side needs a connection."))?;
                        (protocol.clone(), protocol, None)
                    }
                };
                let connector = crate::commands::connector_commands::user_instance(registry, connections, &protocol, cid).await?;
                Ok(Self::remote(connector, &protocol, &name, path, id))
            }
        }
    }

    /// A remote side over `connector` (connected).
    pub fn remote(connector: Arc<dyn Connector>, protocol: &str, name: &str, root: &str, connection_id: Option<uuid::Uuid>) -> Self {
        let root = match root.trim().trim_end_matches('/') {
            "" => "/".to_string(),
            r if r.starts_with('/') => r.to_string(),
            r => format!("/{r}"),
        };
        Self::Remote { connector, protocol: protocol.to_string(), name: name.to_string(), root, connection_id, listed: Default::default() }
    }

    /// This side with the connection's listed paths of an earlier
    /// comparison (no change for a local folder).
    pub fn with_listed(self, paths: ListedPaths) -> Self {
        if let Self::Remote { listed, .. } = &self {
            *listed.lock().unwrap_or_else(|e| e.into_inner()) = paths;
        }
        self
    }

    /// The connection's listed paths (empty for a local folder).
    pub fn listed_paths(&self) -> ListedPaths {
        match self {
            Self::Remote { listed, .. } => listed.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            Self::Local { .. } => ListedPaths::new(),
        }
    }

    /// The connection listed `rel` at `path`.
    pub(crate) fn remember(&self, rel: &str, path: &str) {
        if let Self::Remote { listed, .. } = self {
            if !rel.is_empty() && !path.is_empty() {
                listed.lock().unwrap_or_else(|e| e.into_inner()).insert(rel.to_string(), path.to_string());
            }
        }
    }

    /// `rel` (and everything below it) is no longer on the connection.
    pub(crate) fn forget(&self, rel: &str) {
        if let Self::Remote { listed, .. } = self {
            let below = format!("{rel}/");
            listed.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| k != rel && !k.starts_with(&below));
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    /// `rel` when it is a plain path inside the root (`""` is the root): no
    /// `..`, no absolute or drive part, no backslash tricks.
    pub fn check_relative(rel: &str) -> Result<&str, String> {
        if rel.is_empty() || is_safe_relative(rel) {
            Ok(rel)
        } else {
            Err(format!("\"{rel}\" is not a path inside the compared folder, so it was left alone."))
        }
    }

    /// Where `rel` (relative, `/`-separated) is on this side: a local path,
    /// or the path the connection listed it at (what its download, remove
    /// and listing take). Refuses a path that would leave the root.
    pub fn abs(&self, rel: &str) -> Result<String, String> {
        let rel = Self::check_relative(rel)?;
        Ok(self.located(rel))
    }

    /// Where a new item `rel` is created on this side (what an upload or a
    /// new folder takes): on a connection, the listed path of the folder it
    /// goes in, then its name. Refuses a path that would leave the root.
    pub fn abs_new(&self, rel: &str) -> Result<String, String> {
        let rel = Self::check_relative(rel)?;
        Ok(match (self, rel.rsplit_once('/')) {
            (Self::Local { .. }, _) => self.named(rel),
            (Self::Remote { root, .. }, _) if rel.is_empty() => root.clone(),
            (Self::Remote { .. }, Some((parent, name))) => child_path(&self.located(parent), name),
            (Self::Remote { root, .. }, None) => child_path(root, rel),
        })
    }

    /// `rel` as this side addresses it (callers check `rel` first): a
    /// connection's listed path when it has one, else the listed path of
    /// the nearest folder above it with the rest of the names joined on.
    fn located(&self, rel: &str) -> String {
        let Self::Remote { root, listed, .. } = self else { return self.named(rel) };
        if rel.is_empty() {
            return root.clone();
        }
        if let Some(p) = listed.lock().unwrap_or_else(|e| e.into_inner()).get(rel) {
            return p.clone();
        }
        match rel.rsplit_once('/') {
            Some((parent, name)) => child_path(&self.located(parent), name),
            None => child_path(root, rel),
        }
    }

    /// `rel`'s names joined onto the root, as text (callers check `rel`
    /// first). How the UI names it; a connection may address it otherwise.
    fn named(&self, rel: &str) -> String {
        match self {
            Self::Local { root } if rel.is_empty() => root.to_string_lossy().to_string(),
            Self::Local { root } => rel.split('/').fold(root.clone(), |p, part| p.join(part)).to_string_lossy().to_string(),
            Self::Remote { root, .. } if rel.is_empty() => root.clone(),
            Self::Remote { root, .. } if root == "/" => format!("/{rel}"),
            Self::Remote { root, .. } => format!("{root}/{rel}"),
        }
    }

    /// The local path of `rel` for a local side (`None` for a connection).
    /// Refuses a path that would leave the root.
    pub fn local_path(&self, rel: &str) -> Result<Option<PathBuf>, String> {
        let rel = Self::check_relative(rel)?;
        Ok(match self {
            Self::Local { root } => Some(rel.split('/').filter(|p| !p.is_empty()).fold(root.clone(), |p, part| p.join(part))),
            Self::Remote { .. } => None,
        })
    }

    /// How the UI, the queue and the ledger name `rel` on this side
    /// (`sftp://My server/docs/a.txt` for a connection). Display only.
    pub fn label(&self, rel: &str) -> String {
        match self {
            Self::Local { .. } => self.named(rel),
            Self::Remote { protocol, name, .. } => format!("{protocol}://{name}{}", self.named(rel)),
        }
    }

    fn connection_id(&self) -> Option<uuid::Uuid> {
        match self {
            Self::Remote { connection_id, .. } => *connection_id,
            Self::Local { .. } => None,
        }
    }
}

/// `name` inside the remote folder at `dir`.
fn child_path(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

/// What a running comparison or action is doing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComparePhase {
    Listing,
    Comparing,
    Applying,
    Refreshing,
}

/// Progress of a comparison (`folders_listed`, `entries_found`) or of an
/// action (`done` of `total` steps, `current` path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompareProgress {
    pub task_id: String,
    pub phase: ComparePhase,
    pub folders_listed: u64,
    pub entries_found: u64,
    pub done: u64,
    pub total: u64,
    pub current: Option<String>,
}

impl CompareProgress {
    fn new(task_id: &str, phase: ComparePhase) -> Self {
        Self { task_id: task_id.to_string(), phase, folders_listed: 0, entries_found: 0, done: 0, total: 0, current: None }
    }
}

/// Where progress goes (the app emits it as [`COMPARE_PROGRESS_EVENT`]).
pub type ProgressFn = Arc<dyn Fn(CompareProgress) + Send + Sync>;

/// A comparison's results, as returned to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResult {
    pub task_id: String,
    pub left: CompareSide,
    pub right: CompareSide,
    pub left_label: String,
    pub right_label: String,
    pub options: CompareOptions,
    /// Every entry in tree order (a folder, then what is in it).
    pub entries: Vec<CompareEntry>,
    pub summary: compare::CompareSummary,
    pub notes: Vec<String>,
    pub computed_at: DateTime<Utc>,
}

/// A local folder's entries, as the comparison walk sees them.
struct Listed {
    name: String,
    is_dir: bool,
    size: u64,
    modified: Option<DateTime<Utc>>,
    /// Where to list it, for a folder.
    abs: String,
    /// List what is inside (a folder that is not a link).
    descend: bool,
    /// Could not be read.
    error: Option<String>,
}

/// List one local folder. Links are listed as what they point to; a linked
/// folder is not walked into (it could loop back up the tree).
fn list_local_dir(dir: &Path) -> Result<Vec<Listed>, AppError> {
    let read = std::fs::read_dir(dir).map_err(|e| AppError::file_op(format!("Cannot read folder {}: {e}", dir.display()), "Check that it exists and that you may read it."))?;
    let mut out = Vec::new();
    for entry in read {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(AppError::file_op(format!("Cannot read folder {}: {e}", dir.display()), "Check that you may read it.")),
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let abs = path.to_string_lossy().to_string();
        let link = std::fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        // A dangling link is listed as the link itself.
        let meta = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path));
        out.push(match meta {
            Ok(m) => Listed {
                name,
                is_dir: m.is_dir(),
                size: if m.is_dir() { 0 } else { m.len() },
                modified: m.modified().ok().map(DateTime::<Utc>::from),
                abs,
                descend: m.is_dir() && !link,
                error: None,
            },
            Err(e) => Listed { name, is_dir: false, size: 0, modified: None, abs, descend: false, error: Some(format!("Cannot read {}: {e}", path.display())) },
        });
    }
    Ok(out)
}

/// Counters a walk updates as it goes.
#[derive(Default)]
struct Counters {
    folders: AtomicU64,
    entries: AtomicU64,
}

/// Walk one side to `max_depth` folder levels (0 = all). Folders that
/// cannot be listed become error entries; the root failing fails the walk.
async fn list_side(side: &ResolvedSide, max_depth: u32, cancel: &CancellationToken, counters: &Counters) -> Result<SideListing, AppError> {
    match side {
        ResolvedSide::Local { root } => {
            let lister = |(abs, _rel, _depth): (String, String, u32)| async move {
                tokio::task::spawn_blocking(move || list_local_dir(Path::new(&abs)))
                    .await
                    .map_err(|e| AppError::internal(format!("Listing task failed: {e}")))?
            };
            walk_side(lister, root.to_string_lossy().to_string(), max_depth, cancel, counters).await
        }
        ResolvedSide::Remote { connector, root, listed, .. } => {
            let connector = connector.clone();
            let lister = move |(abs, _rel, _depth): (String, String, u32)| {
                let c = connector.clone();
                async move {
                    let entries = c.list_remote(&abs).await?;
                    Ok(entries
                        .into_iter()
                        .map(|e| {
                            let child = if e.path.is_empty() { child_path(&abs, &e.name) } else { e.path.clone() };
                            Listed { is_dir: e.is_dir, size: if e.is_dir { 0 } else { e.size }, modified: e.modified, descend: e.is_dir, abs: child, error: None, name: e.name.trim_end_matches('/').to_string() }
                        })
                        .collect::<Vec<_>>())
                }
            };
            let listing = walk_side(lister, root.clone(), max_depth, cancel, counters).await?;
            // Every action on these entries addresses them by these paths.
            let paths = listing.entries.iter().map(|m| (m.relative_path.clone(), m.absolute_path.to_string_lossy().to_string())).collect();
            *listed.lock().unwrap_or_else(|e| e.into_inner()) = paths;
            Ok(listing)
        }
    }
}

async fn walk_side<L, F>(lister: L, root: String, max_depth: u32, cancel: &CancellationToken, counters: &Counters) -> Result<SideListing, AppError>
where
    L: Fn((String, String, u32)) -> F,
    F: std::future::Future<Output = Result<Vec<Listed>, AppError>> + Send + 'static,
{
    let mut listing = SideListing::default();
    let complete = crate::connectors::walk::walk_tree(lister, (root, String::new(), 0u32), LIST_CONCURRENCY, cancel, |(_abs, rel, depth), result| {
        counters.folders.fetch_add(1, Ordering::Relaxed);
        let items = match result {
            Ok(items) => items,
            Err(e) if rel.is_empty() => return Err(e),
            Err(e) => {
                listing.errors.push(ListingError { path: rel, is_dir: true, message: e.to_string() });
                return Ok(Vec::new());
            }
        };
        let mut next = Vec::new();
        for it in items {
            if it.name.is_empty() || it.name == "." || it.name == ".." {
                continue;
            }
            counters.entries.fetch_add(1, Ordering::Relaxed);
            // A name with a path separator or `..` in it (legal on many
            // servers) must never become a path that climbs out of the
            // root: it is shown, with its separators replaced, as an entry
            // that cannot be compared or acted on.
            let plain = !it.name.contains(['/', '\\']) && is_safe_relative(&it.name);
            let shown = if plain { it.name.clone() } else { it.name.replace('/', "\u{2215}").replace('\\', "\u{29F5}") };
            let child = if rel.is_empty() { shown } else { format!("{rel}/{shown}") };
            if !plain {
                let message = format!("\"{}\" is not a plain name (it holds a path separator or \"..\"), so it is left alone.", it.name);
                listing.errors.push(ListingError { path: child, is_dir: it.is_dir, message });
                continue;
            }
            if let Some(message) = it.error {
                listing.errors.push(ListingError { path: child, is_dir: it.is_dir, message });
                continue;
            }
            listing.entries.push(FileMeta {
                relative_path: child.clone(),
                absolute_path: PathBuf::from(&it.abs),
                size: it.size,
                modified: it.modified,
                is_dir: it.is_dir,
                is_symlink: false,
                hidden: it.name.starts_with('.'),
                readonly: false,
                attributes: 0,
            });
            if it.is_dir {
                if it.descend && (max_depth == 0 || depth + 1 < max_depth) {
                    next.push((it.abs, child, depth + 1));
                } else {
                    listing.truncated.insert(child);
                }
            }
        }
        Ok(next)
    })
    .await?;
    if !complete {
        return Err(compare::cancelled());
    }
    Ok(listing)
}

/// List both sides (at the same time) and compare them, reporting progress
/// every quarter second. Stops with "Comparison cancelled." when `cancel` fires.
pub async fn run_compare(
    task_id: &str,
    left: &ResolvedSide,
    right: &ResolvedSide,
    opts: &CompareOptions,
    cancel: &CancellationToken,
    progress: &ProgressFn,
) -> Result<CompareOutput, AppError> {
    let start = std::time::Instant::now();
    let counters = Arc::new(Counters::default());
    let ticker = {
        let (counters, progress, id, stop) = (counters.clone(), progress.clone(), task_id.to_string(), cancel.child_token());
        let handle = tokio::spawn({
            let stop = stop.clone();
            async move {
                loop {
                    let mut p = CompareProgress::new(&id, ComparePhase::Listing);
                    p.folders_listed = counters.folders.load(Ordering::Relaxed);
                    p.entries_found = counters.entries.load(Ordering::Relaxed);
                    progress(p);
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                    }
                }
            }
        });
        (handle, stop)
    };
    let (l, r) = tokio::join!(list_side(left, opts.max_depth, cancel, &counters), list_side(right, opts.max_depth, cancel, &counters));
    ticker.1.cancel();
    let _ = ticker.0.await;
    let (l, r) = (l?, r?);

    let mut p = CompareProgress::new(task_id, ComparePhase::Comparing);
    p.folders_listed = counters.folders.load(Ordering::Relaxed);
    p.entries_found = counters.entries.load(Ordering::Relaxed);
    progress(p);

    let flag = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (flag, cancel) = (flag.clone(), cancel.clone());
        tokio::spawn(async move {
            cancel.cancelled().await;
            flag.store(true, Ordering::Relaxed);
        })
    };
    let hasher = LocalSidesHasher { left_local: left.is_local(), right_local: right.is_local() };
    let opts_owned = opts.clone();
    let flag2 = flag.clone();
    let result = tokio::task::spawn_blocking(move || compare::compare_listings(&l, &r, &opts_owned, Some(&hasher), Some(&flag2)))
        .await
        .map_err(|e| AppError::internal(format!("Comparison task failed: {e}")))?;
    watcher.abort();
    let mut out = result?;
    if opts.compare_mode == ufop_core::sync_types::CompareMode::Checksum && !(left.is_local() && right.is_local()) {
        out.notes.insert(0, "Checksums are computed for local folders only; files on a connection were compared by size and modification time.".to_string());
    }
    out.summary.duration_ms = start.elapsed().as_millis() as u64;
    Ok(out)
}

/// A comparison kept for acting on.
#[derive(Debug, Clone)]
pub struct StoredCompare {
    pub task_id: String,
    pub left: CompareSide,
    pub right: CompareSide,
    pub options: CompareOptions,
    pub entries: Vec<CompareEntry>,
    /// Where each connection listed the entries (kept server side; the
    /// actions address remote entries by these).
    pub left_paths: ListedPaths,
    pub right_paths: ListedPaths,
}

/// Running comparisons and actions (cancellable by id) and the latest results.
#[derive(Clone, Default)]
pub struct CompareManager {
    /// Each run's token, with the run number it was started as.
    running: Arc<std::sync::Mutex<std::collections::HashMap<String, (u64, CancellationToken)>>>,
    runs: Arc<AtomicU64>,
    results: Arc<tokio::sync::Mutex<Vec<StoredCompare>>>,
}

/// A run started with [`CompareManager::begin`]: its cancellation token
/// (dereferences to it) and which run it is, for [`CompareManager::end`].
#[derive(Clone)]
pub struct CompareRun {
    token: CancellationToken,
    number: u64,
}

impl std::ops::Deref for CompareRun {
    type Target = CancellationToken;
    fn deref(&self) -> &CancellationToken {
        &self.token
    }
}

impl CompareManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn running(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, (u64, CancellationToken)>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start task `id`: the token [`CompareManager::cancel`] fires. A run
    /// still going under the same id is cancelled and replaced.
    pub fn begin(&self, id: &str) -> CompareRun {
        let run = CompareRun { token: CancellationToken::new(), number: self.runs.fetch_add(1, Ordering::Relaxed) };
        if let Some((_, old)) = self.running().insert(id.to_string(), (run.number, run.token.clone())) {
            old.cancel();
        }
        run
    }

    /// `run` of task `id` ended (a newer run under the same id is kept).
    pub fn end(&self, id: &str, run: &CompareRun) {
        let mut running = self.running();
        if running.get(id).is_some_and(|(n, _)| *n == run.number) {
            running.remove(id);
        }
    }

    /// Stop task `id`; `false` when it is not running.
    pub fn cancel(&self, id: &str) -> bool {
        match self.running().remove(id) {
            Some((_, t)) => {
                t.cancel();
                true
            }
            None => false,
        }
    }

    /// Keep `stored` (replacing results with its id; oldest dropped first).
    pub async fn store(&self, stored: StoredCompare) {
        let mut results = self.results.lock().await;
        results.retain(|r| r.task_id != stored.task_id);
        results.push(stored);
        let excess = results.len().saturating_sub(KEEP_RESULTS);
        results.drain(..excess);
    }

    /// Run `f` on the results of `id`.
    pub async fn with<R>(&self, id: &str, f: impl FnOnce(&mut StoredCompare) -> R) -> Result<R, AppError> {
        let mut results = self.results.lock().await;
        let stored = results.iter_mut().find(|r| r.task_id == id).ok_or_else(|| {
            AppError::not_found("These comparison results are no longer available. Compare the folders again.")
        })?;
        Ok(f(stored))
    }

    /// Forget the results of `id`; `false` when there were none.
    pub async fn release(&self, id: &str) -> bool {
        let mut results = self.results.lock().await;
        let before = results.len();
        results.retain(|r| r.task_id != id);
        results.len() != before
    }
}

#[cfg(test)]
pub(crate) mod tests;
