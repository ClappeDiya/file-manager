//! Standalone folder comparison and reconciliation planning.
//!
//! Two listings (one per side, as [`FileMeta`]s with relative paths) become
//! one entry per path with a status — only on the left, only on the right,
//! the same, different, or not readable — using the sync engine's own
//! decision logic: the pair filter engine ([`CompiledSyncFilter`]) and the
//! planner's comparer (size + time, size only, or checksum). Folders take
//! their status from what is inside them.
//!
//! [`plan_action`] turns a selection of entries into the steps of "copy to
//! the right / left" or "delete on the left / right"; it never copies an
//! entry that is already the same. The listing, the copying and the deleting
//! themselves live with the callers (the desktop app walks local folders
//! and remote connections), so everything here is pure and testable.

use super::filter::CompiledSyncFilter;
use super::planner::{Comparer, ContentHasher, FileMeta};
use crate::error::AppError;
use crate::sync_types::{CompareMode, CopyOptions, SyncFilter, SyncSide};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::AtomicBool;

/// What a comparison found for one path.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CompareStatus {
    LeftOnly,
    RightOnly,
    Same,
    Different,
    /// The entry (or the folder it is in) could not be read on a side, or
    /// a file on one side has the name of a folder on the other.
    Error,
}

/// How a folder comparison decides, and what it looks at.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CompareOptions {
    /// The sync filter engine: files it rejects on both sides are left out.
    pub filter: SyncFilter,
    /// Same size + time (default), same size, or same content hash.
    pub compare_mode: CompareMode,
    /// Folder levels to compare below each root; 0 = all.
    pub max_depth: u32,
    /// Treat modification times up to 2 s apart as equal (FAT drives and
    /// many servers store times coarsely).
    pub time_tolerance: bool,
}

impl CompareOptions {
    /// The copy options the planner's comparer reads.
    fn copy_options(&self) -> CopyOptions {
        CopyOptions { compare_mode: self.compare_mode, fat_time_tolerance: self.time_tolerance, ..CopyOptions::default() }
    }
}

/// An entry of one side that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingError {
    /// Relative path (`/`-separated).
    pub path: String,
    pub is_dir: bool,
    pub message: String,
}

/// Everything one side's walk found.
#[derive(Debug, Clone, Default)]
pub struct SideListing {
    /// Files and folders, relative paths with `/` separators.
    pub entries: Vec<FileMeta>,
    /// Entries (or whole folders) that could not be read.
    pub errors: Vec<ListingError>,
    /// Folders listed but not looked inside (depth limit).
    pub truncated: HashSet<String>,
}

/// One path of a comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompareEntry {
    /// Relative to both roots, `/`-separated.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub status: CompareStatus,
    pub in_left: bool,
    pub in_right: bool,
    pub left_size: Option<u64>,
    pub right_size: Option<u64>,
    pub left_modified: Option<DateTime<Utc>>,
    pub right_modified: Option<DateTime<Utc>>,
    /// Why two copies are different ("Size differs", "Contents differ"…).
    #[serde(default)]
    pub reason: Option<String>,
    /// What could not be read, for `error` entries.
    #[serde(default)]
    pub error: Option<String>,
    /// A folder whose contents were not compared (depth limit).
    #[serde(default)]
    pub children_skipped: bool,
}

/// Counts and bytes of one status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StatusTotals {
    pub files: u64,
    pub folders: u64,
    /// Bytes of these files on the left / right.
    pub left_bytes: u64,
    pub right_bytes: u64,
}

/// Totals per status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CompareSummary {
    pub left_only: StatusTotals,
    pub right_only: StatusTotals,
    pub same: StatusTotals,
    pub different: StatusTotals,
    pub error: StatusTotals,
    /// How long listing and comparing took (set by the caller).
    pub duration_ms: u64,
}

impl CompareSummary {
    fn totals_mut(&mut self, status: CompareStatus) -> &mut StatusTotals {
        match status {
            CompareStatus::LeftOnly => &mut self.left_only,
            CompareStatus::RightOnly => &mut self.right_only,
            CompareStatus::Same => &mut self.same,
            CompareStatus::Different => &mut self.different,
            CompareStatus::Error => &mut self.error,
        }
    }

    /// The totals of `status`.
    pub fn totals(&self, status: CompareStatus) -> StatusTotals {
        match status {
            CompareStatus::LeftOnly => self.left_only,
            CompareStatus::RightOnly => self.right_only,
            CompareStatus::Same => self.same,
            CompareStatus::Different => self.different,
            CompareStatus::Error => self.error,
        }
    }
}

/// Entries, totals and notes (e.g. checksums that were not available).
#[derive(Debug, Clone)]
pub struct CompareOutput {
    pub entries: Vec<CompareEntry>,
    pub summary: CompareSummary,
    pub notes: Vec<String>,
}

/// Hashes local files with xxHash3 for the sides that are local folders;
/// a remote side has no hash (the comparison then falls back to size and
/// time for those files, and says so).
pub struct LocalSidesHasher {
    pub left_local: bool,
    pub right_local: bool,
}

impl ContentHasher for LocalSidesHasher {
    fn hash(&self, side: SyncSide, file: &FileMeta) -> Option<String> {
        let local = if side == SyncSide::Source { self.left_local } else { self.right_local };
        local.then(|| super::planner::LocalHasher.hash(side, file)).flatten()
    }
}

/// Order paths as a tree: a folder, then everything inside it, then its
/// next sibling (`a`, `a/b`, `a-b` rather than byte order's `a`, `a-b`, `a/b`).
pub fn tree_order(a: &str, b: &str) -> Ordering {
    a.split('/').cmp(b.split('/'))
}

fn parent_of(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(p, _)| p)
}

/// `path` and every folder above it, nearest first (`a/b/c` → `a/b`, `a`).
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(parent_of(path), |&p| parent_of(p))
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), AppError> {
    if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
        return Err(cancelled());
    }
    Ok(())
}

/// The error a cancelled comparison ends with.
pub fn cancelled() -> AppError {
    AppError::Sync {
        message: "Comparison cancelled.".to_string(),
        advice: "Start it again when you are ready.".to_string(),
    }
}

/// Compare two listings: every entry with its status, folders included,
/// in tree order, with totals.
pub fn compare_listings(
    left: &SideListing,
    right: &SideListing,
    opts: &CompareOptions,
    hasher: Option<&dyn ContentHasher>,
    cancel: Option<&AtomicBool>,
) -> Result<CompareOutput, AppError> {
    let (mut entries, notes) = compare_entries(left, right, opts, hasher, cancel)?;
    let summary = finalize(&mut entries);
    Ok(CompareOutput { entries, summary, notes })
}

/// The per-path entries of two listings, before folder statuses are
/// settled ([`finalize`]). Also used to re-compare a few paths after an
/// action. Returns the entries and the comparer's notes.
pub fn compare_entries(
    left: &SideListing,
    right: &SideListing,
    opts: &CompareOptions,
    hasher: Option<&dyn ContentHasher>,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<CompareEntry>, Vec<String>), AppError> {
    let filter = CompiledSyncFilter::new(&opts.filter)?;
    let copy_opts = opts.copy_options();
    let cmp = Comparer::new(&copy_opts, hasher);
    let (lmap, rmap) = (index_entries(left), index_entries(right));
    let (lerr, rerr) = (index_errors(left), index_errors(right));
    // Folders that had something in them before the filter ran.
    let mut had_children: HashSet<String> = HashSet::new();
    for p in lmap.keys().chain(rmap.keys()).chain(lerr.keys()).chain(rerr.keys()) {
        if let Some(parent) = parent_of(p) {
            had_children.insert(parent.to_string());
        }
    }
    let paths: BTreeSet<&String> = lmap.keys().chain(rmap.keys()).chain(lerr.keys()).chain(rerr.keys()).collect();

    let mut entries = Vec::with_capacity(paths.len());
    for (i, path) in paths.into_iter().enumerate() {
        if i % 256 == 0 {
            check_cancel(cancel)?;
        }
        let (l, r) = (lmap.get(path), rmap.get(path));
        let mut entry = CompareEntry {
            path: path.clone(),
            name: name_of(path),
            is_dir: l.or(r).is_some_and(|m| m.is_dir) || lerr.get(path).or(rerr.get(path)).is_some_and(|e| e.is_dir),
            status: CompareStatus::Same,
            in_left: l.is_some() || lerr.contains_key(path),
            in_right: r.is_some() || rerr.contains_key(path),
            left_size: l.filter(|m| !m.is_dir).map(|m| m.size),
            right_size: r.filter(|m| !m.is_dir).map(|m| m.size),
            left_modified: l.and_then(|m| m.modified),
            right_modified: r.and_then(|m| m.modified),
            reason: None,
            error: None,
            children_skipped: left.truncated.contains(path.as_str()) || right.truncated.contains(path.as_str()),
        };
        let unreadable: Vec<String> = [("Left", lerr.get(path)), ("Right", rerr.get(path))]
            .into_iter()
            .filter_map(|(side, e)| e.map(|e| format!("{side}: {}", e.message)))
            .collect();
        if !unreadable.is_empty() {
            entry.status = CompareStatus::Error;
            entry.error = Some(unreadable.join(" "));
            entries.push(entry);
            continue;
        }
        // The filter judges files, by each side's own size.
        let accepted = |m: Option<&FileMeta>| m.is_some_and(|m| m.is_dir || filter.accepts(path, m.size));
        if !accepted(l) && !accepted(r) {
            continue;
        }
        match (l, r) {
            (Some(a), Some(b)) if a.is_dir != b.is_dir => {
                entry.is_dir = true;
                entry.status = CompareStatus::Error;
                entry.error = Some(if a.is_dir {
                    "A folder on the left has the name of a file on the right.".to_string()
                } else {
                    "A file on the left has the name of a folder on the right.".to_string()
                });
            }
            (Some(a), Some(_)) if a.is_dir => {}
            (Some(a), Some(b)) => {
                if !cmp.same(a, b, b.modified) {
                    entry.status = CompareStatus::Different;
                    entry.reason = Some(difference_reason(&cmp, opts.compare_mode, a, b));
                }
            }
            (Some(_), None) => entry.status = CompareStatus::LeftOnly,
            (None, Some(_)) => entry.status = CompareStatus::RightOnly,
            (None, None) => continue,
        }
        entries.push(entry);
    }

    // A folder whose every entry the filter left out is not part of the
    // comparison either (a folder that is really empty stays).
    let mut kept_dirs: HashSet<String> = HashSet::new();
    for e in &entries {
        for a in ancestors(&e.path) {
            if !kept_dirs.insert(a.to_string()) {
                break;
            }
        }
    }
    entries.retain(|e| !e.is_dir || e.status == CompareStatus::Error || kept_dirs.contains(&e.path) || !had_children.contains(&e.path) || e.children_skipped);
    Ok((entries, cmp.notes()))
}

fn index_entries(l: &SideListing) -> HashMap<String, FileMeta> {
    l.entries.iter().map(|f| (f.relative_path.replace('\\', "/"), f.clone())).collect()
}

fn index_errors(l: &SideListing) -> HashMap<String, ListingError> {
    l.errors.iter().map(|e| (e.path.replace('\\', "/"), e.clone())).collect()
}

/// Why two files that both exist do not match.
fn difference_reason(cmp: &Comparer<'_>, mode: CompareMode, a: &FileMeta, b: &FileMeta) -> String {
    if a.size != b.size {
        return "Size differs".to_string();
    }
    match mode {
        CompareMode::Checksum => match (cmp.hash(SyncSide::Source, a), cmp.hash(SyncSide::Dest, b)) {
            (Some(x), Some(y)) if x != y => "Contents differ".to_string(),
            _ => "Modification time differs (no checksum available)".to_string(),
        },
        _ => "Modification time differs".to_string(),
    }
}

/// Put `entries` in tree order, settle folder statuses from their contents
/// (a folder on both sides is the same only when everything in it is) and
/// total them up.
pub fn finalize(entries: &mut [CompareEntry]) -> CompareSummary {
    entries.sort_by(|a, b| tree_order(&a.path, &b.path));
    // Folders with something below them that is not the same.
    let mut dirty: HashSet<String> = HashSet::new();
    for e in entries.iter() {
        let own_mismatch = if e.is_dir {
            e.status == CompareStatus::Error || !(e.in_left && e.in_right)
        } else {
            e.status != CompareStatus::Same
        };
        if own_mismatch {
            for a in ancestors(&e.path) {
                if !dirty.insert(a.to_string()) {
                    break;
                }
            }
        }
    }
    let mut summary = CompareSummary::default();
    for e in entries.iter_mut() {
        if e.is_dir && e.status != CompareStatus::Error {
            (e.status, e.reason) = match (e.in_left, e.in_right) {
                (true, false) => (CompareStatus::LeftOnly, None),
                (false, true) => (CompareStatus::RightOnly, None),
                _ if dirty.contains(&e.path) => (CompareStatus::Different, Some("Contents differ".to_string())),
                _ if e.children_skipped => (CompareStatus::Same, Some("Contents not compared (below the depth limit)".to_string())),
                _ => (CompareStatus::Same, None),
            };
        }
        let t = summary.totals_mut(e.status);
        if e.is_dir {
            t.folders += 1;
        } else {
            t.files += 1;
            t.left_bytes += e.left_size.unwrap_or(0);
            t.right_bytes += e.right_size.unwrap_or(0);
        }
    }
    summary
}

/// What [`merge_refreshed`] changed in a result.
#[derive(Debug, Clone, Default)]
pub struct MergeOutcome {
    pub summary: CompareSummary,
    /// Entries that are new or changed (folders above the affected paths included).
    pub changed: Vec<CompareEntry>,
    /// Paths no longer on either side.
    pub removed: Vec<String>,
}

/// Fold a re-comparison of `affected` paths (`refreshed`: their entries now;
/// a path missing from it is gone from both sides) into `entries`, and
/// settle the folders above them again.
pub fn merge_refreshed(entries: &mut Vec<CompareEntry>, affected: &HashSet<String>, refreshed: Vec<CompareEntry>) -> MergeOutcome {
    let mut watched: HashSet<String> = affected.clone();
    for p in affected {
        watched.extend(ancestors(p).map(str::to_string));
    }
    let before: HashMap<String, CompareEntry> = entries.iter().filter(|e| watched.contains(&e.path)).map(|e| (e.path.clone(), e.clone())).collect();
    let mut fresh: HashMap<String, CompareEntry> = refreshed.into_iter().map(|e| (e.path.clone(), e)).collect();
    entries.retain(|e| !affected.contains(&e.path));
    for (path, mut e) in fresh.drain() {
        if let Some(old) = before.get(&path) {
            e.children_skipped |= old.children_skipped;
        }
        entries.push(e);
    }
    let summary = finalize(entries);
    let mut changed = Vec::new();
    let mut present = HashSet::new();
    for e in entries.iter().filter(|e| watched.contains(&e.path)) {
        present.insert(e.path.clone());
        if before.get(&e.path) != Some(e) {
            changed.push(e.clone());
        }
    }
    let mut removed: Vec<String> = before.keys().filter(|p| !present.contains(*p)).cloned().collect();
    removed.sort_by(|a, b| tree_order(a, b));
    MergeOutcome { summary, changed, removed }
}

// ── Acting on differences ──

/// What to do with the selected entries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompareAction {
    CopyToRight,
    CopyToLeft,
    DeleteLeft,
    DeleteRight,
}

/// One side of a comparison.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CompareSideId {
    Left,
    Right,
}

impl CompareSideId {
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// One step of an action, in the order they run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionStep {
    /// Create a folder that only the other side has.
    MakeDir { side: CompareSideId, path: String },
    /// Copy a file across; `replace` when the target side has a different copy.
    Copy { from: CompareSideId, to: CompareSideId, path: String, size: u64, modified: Option<DateTime<Utc>>, replace: bool },
    /// Delete a file, or a folder once it is empty (files first, deepest folders first).
    Delete { side: CompareSideId, path: String, is_dir: bool, size: u64 },
}

impl ActionStep {
    pub fn path(&self) -> &str {
        match self {
            Self::MakeDir { path, .. } | Self::Copy { path, .. } | Self::Delete { path, .. } => path,
        }
    }
}

/// A selected entry the action leaves alone, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkippedEntry {
    pub path: String,
    pub reason: String,
}

/// The steps an action over a selection takes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionPlan {
    pub action: CompareAction,
    pub steps: Vec<ActionStep>,
    pub skipped: Vec<SkippedEntry>,
    /// Files copied or deleted.
    pub files: u64,
    /// Folders created or deleted.
    pub folders: u64,
    /// Bytes copied or deleted.
    pub bytes: u64,
    /// Copies that replace a different file on the target side.
    pub replaced: u64,
}

impl ActionPlan {
    /// Copies or deletes on `side` (the side the action changes).
    pub fn target(&self) -> CompareSideId {
        match self.action {
            CompareAction::CopyToRight | CompareAction::DeleteRight => CompareSideId::Right,
            CompareAction::CopyToLeft | CompareAction::DeleteLeft => CompareSideId::Left,
        }
    }
}

fn on(e: &CompareEntry, side: CompareSideId) -> bool {
    match side {
        CompareSideId::Left => e.in_left,
        CompareSideId::Right => e.in_right,
    }
}

fn size_on(e: &CompareEntry, side: CompareSideId) -> u64 {
    match side {
        CompareSideId::Left => e.left_size,
        CompareSideId::Right => e.right_size,
    }
    .unwrap_or(0)
}

/// The steps of `action` over the `selected` paths of a comparison. A
/// selected folder stands for everything in it. Copies never include
/// entries that are already the same; deletes remove files, then the
/// selected folders once nothing is left in them.
pub fn plan_action(entries: &[CompareEntry], selected: &[String], action: CompareAction) -> ActionPlan {
    let by_path: HashMap<&str, &CompareEntry> = entries.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut plan = ActionPlan { action, steps: Vec::new(), skipped: Vec::new(), files: 0, folders: 0, bytes: 0, replaced: 0 };
    let target = plan.target();

    // The selection with every folder expanded to its contents, in tree order.
    let roots: Vec<&str> = selected.iter().map(|p| p.trim_matches('/')).collect();
    let mut chosen: Vec<&CompareEntry> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for root in &roots {
        match by_path.get(root) {
            None => plan.skipped.push(SkippedEntry { path: root.to_string(), reason: "No longer in the comparison results.".to_string() }),
            Some(e) => {
                if seen.insert(e.path.as_str()) {
                    chosen.push(e);
                }
                if e.is_dir {
                    let prefix = format!("{}/", e.path);
                    for d in entries.iter().filter(|d| d.path.starts_with(&prefix)) {
                        if seen.insert(d.path.as_str()) {
                            chosen.push(d);
                        }
                    }
                }
            }
        }
    }
    chosen.sort_by(|a, b| tree_order(&a.path, &b.path));
    // Below a folder that could not be read nothing is known: leave it all.
    let unreadable: Vec<&str> = chosen.iter().filter(|e| e.is_dir && e.status == CompareStatus::Error).map(|e| e.path.as_str()).collect();
    let under_unreadable = |p: &str| unreadable.iter().any(|u| p.starts_with(&format!("{u}/")));

    match action {
        CompareAction::CopyToRight | CompareAction::CopyToLeft => {
            let from = target.other();
            let mut dirs: BTreeSet<String> = BTreeSet::new();
            let mut copies = Vec::new();
            for e in &chosen {
                if under_unreadable(&e.path) {
                    continue;
                }
                let skip = match e.status {
                    CompareStatus::Error => Some(format!("Could not be compared: {}", e.error.as_deref().unwrap_or("unreadable"))),
                    _ if e.is_dir => None,
                    CompareStatus::Same => Some("Already the same on both sides.".to_string()),
                    _ if !on(e, from) => Some(format!("Not on the {} side.", from.label())),
                    _ => None,
                };
                if let Some(reason) = skip {
                    plan.skipped.push(SkippedEntry { path: e.path.clone(), reason });
                    continue;
                }
                if e.is_dir {
                    if on(e, from) && !on(e, target) {
                        dirs.insert(e.path.clone());
                    }
                    continue;
                }
                let replace = on(e, target);
                plan.files += 1;
                plan.bytes += size_on(e, from);
                plan.replaced += u64::from(replace);
                let modified = if from == CompareSideId::Left { e.left_modified } else { e.right_modified };
                copies.push(ActionStep::Copy { from, to: target, path: e.path.clone(), size: size_on(e, from), modified, replace });
            }
            // Folders the copies land in that the target side does not have yet.
            let needed: Vec<String> = copies.iter().map(|c| c.path().to_string()).chain(dirs.iter().cloned()).collect();
            for p in &needed {
                for a in ancestors(p) {
                    match by_path.get(a) {
                        Some(d) if !on(d, target) => {
                            dirs.insert(a.to_string());
                        }
                        _ => break,
                    }
                }
            }
            let mut dirs: Vec<String> = dirs.into_iter().collect();
            dirs.sort_by(|a, b| a.matches('/').count().cmp(&b.matches('/').count()).then_with(|| tree_order(a, b)));
            plan.folders = dirs.len() as u64;
            plan.steps.extend(dirs.into_iter().map(|path| ActionStep::MakeDir { side: target, path }));
            plan.steps.extend(copies);
        }
        CompareAction::DeleteLeft | CompareAction::DeleteRight => {
            let explicit: HashSet<&str> = roots.iter().copied().collect();
            let mut files = Vec::new();
            let mut dirs = Vec::new();
            for e in &chosen {
                if under_unreadable(&e.path) {
                    continue;
                }
                if e.status == CompareStatus::Error {
                    plan.skipped.push(SkippedEntry { path: e.path.clone(), reason: format!("Could not be compared: {}", e.error.as_deref().unwrap_or("unreadable")) });
                    continue;
                }
                if !on(e, target) {
                    if explicit.contains(e.path.as_str()) {
                        plan.skipped.push(SkippedEntry { path: e.path.clone(), reason: format!("Not on the {} side.", target.label()) });
                    }
                    continue;
                }
                if e.is_dir {
                    dirs.push(e.path.clone());
                } else {
                    plan.files += 1;
                    plan.bytes += size_on(e, target);
                    files.push(ActionStep::Delete { side: target, path: e.path.clone(), is_dir: false, size: size_on(e, target) });
                }
            }
            // Deepest folders first, so each is empty by the time it goes.
            dirs.sort_by(|a, b| b.matches('/').count().cmp(&a.matches('/').count()).then_with(|| tree_order(a, b)));
            plan.folders = dirs.len() as u64;
            plan.steps.extend(files);
            plan.steps.extend(dirs.into_iter().map(|path| ActionStep::Delete { side: target, path, is_dir: true, size: 0 }));
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn t(secs: i64) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(secs, 0)
    }

    fn file(path: &str, size: u64, secs: i64) -> FileMeta {
        FileMeta {
            relative_path: path.to_string(),
            absolute_path: PathBuf::from(format!("/nonexistent/{path}")),
            size,
            modified: t(secs),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            readonly: false,
            attributes: 0,
        }
    }

    fn dir(path: &str) -> FileMeta {
        FileMeta { is_dir: true, size: 0, ..file(path, 0, 0) }
    }

    fn listing(entries: Vec<FileMeta>) -> SideListing {
        SideListing { entries, ..Default::default() }
    }

    fn status_of(out: &[CompareEntry], path: &str) -> CompareStatus {
        out.iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no entry {path}")).status
    }

    fn sample() -> (SideListing, SideListing) {
        let left = listing(vec![
            dir("docs"),
            file("docs/a.txt", 10, 100),
            file("docs/b.txt", 20, 100),
            file("docs/only-left.txt", 5, 100),
            dir("pics"),
            file("pics/p.jpg", 50, 100),
            dir("left-dir"),
            file("left-dir/x", 1, 100),
            file("top.md", 3, 100),
        ]);
        let right = listing(vec![
            dir("docs"),
            file("docs/a.txt", 10, 100),
            file("docs/b.txt", 21, 100),
            file("docs/only-right.txt", 7, 100),
            dir("pics"),
            file("pics/p.jpg", 50, 100),
            file("top.md", 3, 999),
            dir("right-dir"),
        ]);
        (left, right)
    }

    #[test]
    fn statuses_per_entry_and_folder() {
        let (l, r) = sample();
        let out = compare_listings(&l, &r, &CompareOptions::default(), None, None).unwrap();
        let e = &out.entries;
        assert_eq!(status_of(e, "docs/a.txt"), CompareStatus::Same);
        assert_eq!(status_of(e, "docs/b.txt"), CompareStatus::Different);
        assert_eq!(e.iter().find(|x| x.path == "docs/b.txt").unwrap().reason.as_deref(), Some("Size differs"));
        assert_eq!(status_of(e, "docs/only-left.txt"), CompareStatus::LeftOnly);
        assert_eq!(status_of(e, "docs/only-right.txt"), CompareStatus::RightOnly);
        assert_eq!(status_of(e, "top.md"), CompareStatus::Different);
        assert_eq!(e.iter().find(|x| x.path == "top.md").unwrap().reason.as_deref(), Some("Modification time differs"));
        // Folders: contents decide; one-sided folders are one-sided.
        assert_eq!(status_of(e, "docs"), CompareStatus::Different);
        assert_eq!(status_of(e, "pics"), CompareStatus::Same);
        assert_eq!(status_of(e, "left-dir"), CompareStatus::LeftOnly);
        assert_eq!(status_of(e, "right-dir"), CompareStatus::RightOnly);
        // Tree order: a folder, then its contents.
        let paths: Vec<&str> = e.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(&paths[..5], &["docs", "docs/a.txt", "docs/b.txt", "docs/only-left.txt", "docs/only-right.txt"]);
        let s = &out.summary;
        assert_eq!((s.same.files, s.same.folders), (2, 1));
        assert_eq!((s.different.files, s.different.folders), (2, 1));
        assert_eq!((s.left_only.files, s.left_only.folders, s.left_only.left_bytes), (2, 1, 6));
        assert_eq!((s.right_only.files, s.right_only.folders, s.right_only.right_bytes), (1, 1, 7));
        assert_eq!((s.different.left_bytes, s.different.right_bytes), (23, 24));
    }

    #[test]
    fn compare_modes() {
        let (l, r) = sample();
        let size_only = CompareOptions { compare_mode: CompareMode::SizeOnly, ..Default::default() };
        let out = compare_listings(&l, &r, &size_only, None, None).unwrap();
        assert_eq!(status_of(&out.entries, "top.md"), CompareStatus::Same);
        assert_eq!(status_of(&out.entries, "docs/b.txt"), CompareStatus::Different);

        // 2-second tolerance.
        let l = listing(vec![file("f", 1, 100)]);
        let r = listing(vec![file("f", 1, 101)]);
        let out = compare_listings(&l, &r, &CompareOptions::default(), None, None).unwrap();
        assert_eq!(status_of(&out.entries, "f"), CompareStatus::Different);
        let tolerant = CompareOptions { time_tolerance: true, ..Default::default() };
        let out = compare_listings(&l, &r, &tolerant, None, None).unwrap();
        assert_eq!(status_of(&out.entries, "f"), CompareStatus::Same);
    }

    struct FakeHasher(HashMap<(bool, String), String>);
    impl ContentHasher for FakeHasher {
        fn hash(&self, side: SyncSide, file: &FileMeta) -> Option<String> {
            self.0.get(&(side == SyncSide::Source, file.relative_path.clone())).cloned()
        }
    }

    #[test]
    fn checksum_mode_uses_hashes_and_notes_fallbacks() {
        let l = listing(vec![file("same", 4, 1), file("edited", 4, 1), file("touched", 4, 1), file("nohash", 4, 1)]);
        let r = listing(vec![file("same", 4, 1), file("edited", 4, 1), file("touched", 4, 50), file("nohash", 4, 9)]);
        let mut h = HashMap::new();
        for (p, a, b) in [("same", "x", "x"), ("edited", "x", "y"), ("touched", "z", "z")] {
            h.insert((true, p.to_string()), a.to_string());
            h.insert((false, p.to_string()), b.to_string());
        }
        let hasher = FakeHasher(h);
        let opts = CompareOptions { compare_mode: CompareMode::Checksum, ..Default::default() };
        let out = compare_listings(&l, &r, &opts, Some(&hasher), None).unwrap();
        assert_eq!(status_of(&out.entries, "same"), CompareStatus::Same);
        assert_eq!(status_of(&out.entries, "edited"), CompareStatus::Different);
        assert_eq!(out.entries.iter().find(|e| e.path == "edited").unwrap().reason.as_deref(), Some("Contents differ"));
        // Same bytes, different times: checksum mode says same.
        assert_eq!(status_of(&out.entries, "touched"), CompareStatus::Same);
        // No hash: falls back to size + time, and says so.
        assert_eq!(status_of(&out.entries, "nohash"), CompareStatus::Different);
        assert!(out.notes.iter().any(|n| n.contains("could not be hashed")), "{:?}", out.notes);
    }

    #[test]
    fn filter_engine_applies_and_prunes_emptied_folders() {
        let l = listing(vec![dir("logs"), file("logs/a.log", 1, 1), dir("empty"), file("keep.txt", 1, 1), file("drop.log", 1, 1)]);
        let r = listing(vec![file("keep.txt", 1, 1)]);
        let opts = CompareOptions { filter: SyncFilter { exclude_patterns: vec!["*.log".into()], ..Default::default() }, ..Default::default() };
        let out = compare_listings(&l, &r, &opts, None, None).unwrap();
        let paths: Vec<&str> = out.entries.iter().map(|e| e.path.as_str()).collect();
        // `logs` only held excluded files; `empty` really is empty and stays.
        assert_eq!(paths, ["empty", "keep.txt"]);
    }

    #[test]
    fn error_entries_and_type_mismatch() {
        let mut l = listing(vec![dir("locked"), file("x", 1, 1), dir("clash"), file("ok", 1, 1)]);
        l.errors.push(ListingError { path: "locked".into(), is_dir: true, message: "Permission denied".into() });
        let r = listing(vec![dir("locked"), file("locked/inner", 1, 1), file("x", 1, 1), file("clash", 3, 1), file("ok", 1, 1)]);
        let out = compare_listings(&l, &r, &CompareOptions::default(), None, None).unwrap();
        let locked = out.entries.iter().find(|e| e.path == "locked").unwrap();
        assert_eq!(locked.status, CompareStatus::Error);
        assert_eq!(locked.error.as_deref(), Some("Left: Permission denied"));
        let clash = out.entries.iter().find(|e| e.path == "clash").unwrap();
        assert_eq!(clash.status, CompareStatus::Error);
        assert!(clash.error.as_deref().unwrap().contains("folder on the left"));
        assert_eq!(out.summary.error.folders, 2);
        assert_eq!(status_of(&out.entries, "ok"), CompareStatus::Same);
    }

    #[test]
    fn cancel_flag_stops_the_comparison() {
        let (l, r) = sample();
        let flag = AtomicBool::new(true);
        let err = compare_listings(&l, &r, &CompareOptions::default(), None, Some(&flag)).unwrap_err();
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn depth_limited_folders_say_so() {
        let mut l = listing(vec![dir("deep")]);
        l.truncated.insert("deep".into());
        let mut r = listing(vec![dir("deep")]);
        r.truncated.insert("deep".into());
        let out = compare_listings(&l, &r, &CompareOptions::default(), None, None).unwrap();
        let d = &out.entries[0];
        assert!(d.children_skipped);
        assert_eq!(d.status, CompareStatus::Same);
        assert!(d.reason.as_deref().unwrap().contains("depth"));
    }

    fn sample_entries() -> Vec<CompareEntry> {
        let (l, r) = sample();
        compare_listings(&l, &r, &CompareOptions::default(), None, None).unwrap().entries
    }

    #[test]
    fn copy_selected_skips_same_and_maps_directions() {
        let entries = sample_entries();
        let all: Vec<String> = vec!["docs".into(), "pics".into(), "left-dir".into(), "right-dir".into(), "top.md".into()];

        let to_right = plan_action(&entries, &all, CompareAction::CopyToRight);
        let copies: Vec<(&str, bool)> = to_right
            .steps
            .iter()
            .filter_map(|s| match s {
                ActionStep::Copy { from, to, path, replace, .. } => {
                    assert_eq!((*from, *to), (CompareSideId::Left, CompareSideId::Right));
                    Some((path.as_str(), *replace))
                }
                _ => None,
            })
            .collect();
        assert_eq!(copies, [("docs/b.txt", true), ("docs/only-left.txt", false), ("left-dir/x", false), ("top.md", true)]);
        // The missing folder is created first.
        assert_eq!(to_right.steps[0], ActionStep::MakeDir { side: CompareSideId::Right, path: "left-dir".into() });
        // Same entries are never copied, and say why.
        for same in ["docs/a.txt", "pics/p.jpg"] {
            assert!(to_right.skipped.iter().any(|s| s.path == same && s.reason.contains("Already the same")), "{same}");
            assert!(!to_right.steps.iter().any(|s| s.path() == same));
        }
        assert!(to_right.skipped.iter().any(|s| s.path == "docs/only-right.txt"));
        assert_eq!((to_right.files, to_right.replaced, to_right.bytes), (4, 2, 20 + 5 + 1 + 3));

        let to_left = plan_action(&entries, &all, CompareAction::CopyToLeft);
        let copies: Vec<&str> = to_left
            .steps
            .iter()
            .filter_map(|s| match s {
                ActionStep::Copy { from, to, path, .. } => {
                    assert_eq!((*from, *to), (CompareSideId::Right, CompareSideId::Left));
                    Some(path.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(copies, ["docs/b.txt", "docs/only-right.txt", "top.md"]);
        assert!(to_left.steps.contains(&ActionStep::MakeDir { side: CompareSideId::Left, path: "right-dir".into() }));
        // Bytes come from the side copied from.
        assert_eq!(to_left.bytes, 21 + 7 + 3);
    }

    #[test]
    fn copy_of_a_nested_file_creates_its_missing_folders() {
        let l = listing(vec![dir("a"), dir("a/b"), file("a/b/c.txt", 1, 1)]);
        let out = compare_listings(&l, &listing(vec![]), &CompareOptions::default(), None, None).unwrap();
        let plan = plan_action(&out.entries, &["a/b/c.txt".to_string()], CompareAction::CopyToRight);
        assert_eq!(
            plan.steps,
            vec![
                ActionStep::MakeDir { side: CompareSideId::Right, path: "a".into() },
                ActionStep::MakeDir { side: CompareSideId::Right, path: "a/b".into() },
                ActionStep::Copy { from: CompareSideId::Left, to: CompareSideId::Right, path: "a/b/c.txt".into(), size: 1, modified: t(1), replace: false },
            ]
        );
    }

    #[test]
    fn delete_plan_removes_files_then_emptied_folders() {
        let entries = sample_entries();
        let plan = plan_action(&entries, &["docs".to_string(), "right-dir".to_string()], CompareAction::DeleteLeft);
        let paths: Vec<&str> = plan.steps.iter().map(|s| s.path()).collect();
        // `same` files are deletable from one side; right-only ones are not on the left.
        assert_eq!(paths, ["docs/a.txt", "docs/b.txt", "docs/only-left.txt", "docs"]);
        assert!(matches!(plan.steps.last(), Some(ActionStep::Delete { is_dir: true, side: CompareSideId::Left, .. })));
        assert!(plan.skipped.iter().any(|s| s.path == "right-dir"));
        assert_eq!((plan.files, plan.folders, plan.bytes), (3, 1, 35));
        let right = plan_action(&entries, &["docs/only-right.txt".to_string()], CompareAction::DeleteRight);
        assert_eq!(right.steps, vec![ActionStep::Delete { side: CompareSideId::Right, path: "docs/only-right.txt".into(), is_dir: false, size: 7 }]);
    }

    #[test]
    fn actions_leave_unreadable_entries_alone() {
        let mut l = listing(vec![dir("locked"), file("locked/a", 1, 1)]);
        l.errors.push(ListingError { path: "locked".into(), is_dir: true, message: "denied".into() });
        let out = compare_listings(&l, &listing(vec![]), &CompareOptions::default(), None, None).unwrap();
        for action in [CompareAction::CopyToRight, CompareAction::DeleteLeft] {
            let plan = plan_action(&out.entries, &["locked".to_string()], action);
            assert!(plan.steps.is_empty(), "{action:?}: {:?}", plan.steps);
            assert!(plan.skipped[0].reason.contains("Could not be compared"));
        }
        let gone = plan_action(&out.entries, &["missing".to_string()], CompareAction::CopyToRight);
        assert!(gone.skipped[0].reason.contains("No longer"));
    }

    #[test]
    fn merge_refreshed_updates_entries_and_folders() {
        let mut entries = sample_entries();
        // docs/b.txt was copied right and now matches; only-left was deleted on the left (gone everywhere).
        let mut b = entries.iter().find(|e| e.path == "docs/b.txt").unwrap().clone();
        b.status = CompareStatus::Same;
        b.right_size = Some(20);
        b.reason = None;
        let affected: HashSet<String> = ["docs/b.txt".to_string(), "docs/only-left.txt".to_string()].into();
        let out = merge_refreshed(&mut entries, &affected, vec![b]);
        assert_eq!(out.removed, ["docs/only-left.txt"]);
        let changed: Vec<&str> = out.changed.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(changed, ["docs/b.txt"]);
        // docs still differs (only-right.txt), so its row did not change.
        assert_eq!(status_of(&entries, "docs"), CompareStatus::Different);
        assert_eq!(out.summary.left_only.files, 1);

        // Once only-right is gone too, docs becomes the same.
        let affected: HashSet<String> = ["docs/only-right.txt".to_string()].into();
        let out = merge_refreshed(&mut entries, &affected, vec![]);
        assert!(out.changed.iter().any(|e| e.path == "docs" && e.status == CompareStatus::Same));
    }

    #[test]
    fn tree_order_keeps_folders_with_their_contents() {
        let mut v = vec!["a-b", "a/b", "a", "a/b/c", "b"];
        v.sort_by(|x, y| tree_order(x, y));
        assert_eq!(v, ["a", "a/b", "a/b/c", "a-b", "b"]);
    }
}
