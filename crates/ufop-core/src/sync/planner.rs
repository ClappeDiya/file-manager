//! Sync planner - computes file diffs and produces dry-run previews.
//!
//! Walks source and destination directories, compares entries via
//! size+mtime (fast) or checksum (full), applies selective filters,
//! and produces a `SyncPreview` without touching the filesystem.

use crate::error::AppError;
use crate::sync::filter::CompiledSyncFilter;
use crate::sync_types::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// Metadata for a single file entry during planning.
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub is_dir: bool,
    /// Entry is a symbolic link recorded as-is (`SymlinkMode::CopyLink`).
    pub is_symlink: bool,
    /// Hidden: a dot-file, or the Windows hidden attribute.
    pub hidden: bool,
    pub readonly: bool,
    /// Raw NTFS attribute bits (always 0 off Windows).
    pub attributes: u32,
}

/// Recursively walk a directory and collect file metadata.
pub fn walk_directory(root: &Path) -> Result<Vec<FileMeta>, AppError> {
    walk_directory_with(root, &CopyOptions::default())
}

/// Walk a directory honouring the Robocopy-style selection options:
/// /S, /LEV:n, /XD and symlink handling (/XJ, /SL).
pub fn walk_directory_with(root: &Path, opts: &CopyOptions) -> Result<Vec<FileMeta>, AppError> {
    let mut entries = Vec::new();
    // /LEV:n counts the root as level 1; without /S only the root is copied.
    let max_depth = match (opts.copy_subdirs, opts.max_depth) {
        (false, _) => 1,
        (true, 0) => u32::MAX,
        (true, n) => n,
    };
    walk_recursive(root, root, 1, max_depth, opts, &mut entries)?;
    Ok(entries)
}

fn is_hidden(name: &str, _metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if _metadata.file_attributes() & 0x2 != 0 {
            return true;
        }
    }
    name.starts_with('.')
}

/// Folders the sync engine itself has written into sync roots, and in-flight
/// restartable copies. Never synced, never purged: purging an older
/// `.sync-rollback` would destroy the snapshots a rollback needs, and purging
/// a `.ufop-partial` would throw away the progress /Z exists to keep.
pub const INTERNAL_DIRS: [&str; 2] = [".sync-rollback", ".sync-quarantine"];

/// Modified within the pair's settle window: probably still being written.
fn still_changing(file: &FileMeta, settle_secs: u64, now: DateTime<Utc>) -> bool {
    settle_secs > 0 && file.modified.is_some_and(|m| now - m < chrono::Duration::seconds(settle_secs as i64))
}

pub fn is_internal_entry(name: &str) -> bool {
    INTERNAL_DIRS.contains(&name)
        || name == INVENTORY_FILE
        || name == super::versions::VERSIONS_FILE
        || name.ends_with(super::copier::PARTIAL_SUFFIX)
}

/// Destination inventory written after a clean run (`TrustedInventory`).
pub const INVENTORY_FILE: &str = ".ufop-sync-inventory.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct InventoryEntry {
    path: String,
    size: u64,
    modified: Option<DateTime<Utc>>,
}

/// Read the recorded destination state, if any.
pub fn load_inventory(dest_root: &Path) -> Option<Vec<FileMeta>> {
    let data = std::fs::read(dest_root.join(INVENTORY_FILE)).ok()?;
    let entries: Vec<InventoryEntry> = serde_json::from_slice(&data).ok()?;
    Some(
        entries
            .into_iter()
            .map(|e| FileMeta {
                absolute_path: dest_root.join(&e.path),
                relative_path: e.path,
                size: e.size,
                modified: e.modified,
                is_dir: false,
                is_symlink: false,
                hidden: false,
                readonly: false,
                attributes: 0,
            })
            .collect(),
    )
}

/// Record the destination state for the next `TrustedInventory` run.
pub fn save_inventory(dest_root: &Path, files: &[FileMeta]) -> std::io::Result<()> {
    let entries: Vec<InventoryEntry> = files
        .iter()
        .filter(|f| !f.is_dir)
        .map(|f| InventoryEntry { path: f.relative_path.clone(), size: f.size, modified: f.modified })
        .collect();
    let tmp = dest_root.join(format!("{INVENTORY_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&entries)?)?;
    std::fs::rename(tmp, dest_root.join(INVENTORY_FILE))
}

/// True when a directory is excluded by /XD — matched against its name or its
/// path relative to the sync root.
pub fn dir_excluded(relative_path: &str, opts: &CopyOptions) -> bool {
    if opts.exclude_dirs.is_empty() {
        return false;
    }
    let name = Path::new(relative_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(relative_path);
    let normalized = relative_path.replace('\\', "/");
    opts.exclude_dirs.iter().any(|pat| {
        let pat = pat.trim().trim_end_matches(['/', '\\']).replace('\\', "/");
        !pat.is_empty() && (glob_match(&pat, name) || glob_match(&pat, &normalized))
    })
}

fn walk_recursive(
    root: &Path,
    current: &Path,
    depth: u32,
    max_depth: u32,
    opts: &CopyOptions,
    entries: &mut Vec<FileMeta>,
) -> Result<(), AppError> {
    let read_dir = std::fs::read_dir(current).map_err(|e| AppError::Sync {
        message: format!("Cannot read directory {}: {}", current.display(), e),
        advice: "Check that the directory exists and you have read permission.".to_string(),
    })?;

    for entry_result in read_dir {
        let entry = entry_result.map_err(|e| AppError::Sync {
            message: format!("Error reading entry in {}: {}", current.display(), e),
            advice: "Check file permissions.".to_string(),
        })?;

        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let name = entry.file_name().to_string_lossy().to_string();
        if is_internal_entry(&name) {
            continue;
        }

        let link_meta = std::fs::symlink_metadata(&path).map_err(|e| AppError::Sync {
            message: format!("Cannot read metadata for {}: {}", path.display(), e),
            advice: "Check file permissions.".to_string(),
        })?;

        if link_meta.file_type().is_symlink() {
            match opts.symlinks {
                SymlinkMode::Skip => continue,
                SymlinkMode::CopyLink if cfg!(unix) => {
                    entries.push(FileMeta {
                        relative_path: relative,
                        absolute_path: path,
                        size: 0,
                        modified: link_meta.modified().ok().map(DateTime::<Utc>::from),
                        is_dir: false,
                        is_symlink: true,
                        hidden: is_hidden(&name, &link_meta),
                        readonly: false,
                        attributes: super::winattr::attributes_of(&link_meta),
                    });
                    continue;
                }
                _ => {}
            }
        }

        let metadata = std::fs::metadata(&path).map_err(|e| AppError::Sync {
            message: format!("Cannot read metadata for {}: {}", path.display(), e),
            advice: "Check file permissions.".to_string(),
        })?;

        let modified = metadata.modified().ok().map(DateTime::<Utc>::from);

        if metadata.is_dir() {
            if depth >= max_depth || dir_excluded(&relative, opts) {
                continue;
            }
            entries.push(FileMeta {
                relative_path: relative.clone(),
                absolute_path: path.clone(),
                size: 0,
                modified,
                is_dir: true,
                is_symlink: false,
                hidden: is_hidden(&name, &metadata),
                readonly: false,
                attributes: super::winattr::attributes_of(&metadata),
            });
            walk_recursive(root, &path, depth + 1, max_depth, opts, entries)?;
        } else {
            entries.push(FileMeta {
                relative_path: relative,
                absolute_path: path,
                size: metadata.len(),
                modified,
                is_dir: false,
                is_symlink: false,
                hidden: is_hidden(&name, &metadata),
                readonly: metadata.permissions().readonly(),
                attributes: super::winattr::attributes_of(&metadata),
            });
        }
    }

    Ok(())
}

/// Compare two timestamps with the /FFT (2 s) and /DST (1 h) tolerances.
pub fn times_equal(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>, opts: &CopyOptions) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let diff_ms = (a - b).num_milliseconds().abs();
            let tolerance_ms = if opts.fat_time_tolerance { 2_000 } else { 0 };
            if diff_ms <= tolerance_ms {
                return true;
            }
            opts.dst_tolerance && (diff_ms - 3_600_000).abs() <= tolerance_ms.max(1_000)
        }
        (a, b) => a == b,
    }
}

/// Why a source file is excluded by attribute or age options, if it is.
fn option_exclusion(
    file: &FileMeta,
    opts: &CopyOptions,
    now: DateTime<Utc>,
) -> Option<&'static str> {
    if opts.exclude_hidden && file.hidden {
        return Some("Hidden file excluded (/XA:H)");
    }
    if opts.exclude_readonly && file.readonly {
        return Some("Read-only file excluded (/XA:R)");
    }
    if let Some(reason) = super::winattr::attribute_exclusion(file.attributes, opts) {
        return Some(reason);
    }
    if let Some(modified) = file.modified {
        let age = now - modified;
        if opts.max_age_days > 0 && age > chrono::Duration::days(opts.max_age_days as i64) {
            return Some("Older than maximum age (/MAXAGE)");
        }
        if opts.min_age_days > 0 && age < chrono::Duration::days(opts.min_age_days as i64) {
            return Some("Newer than minimum age (/MINAGE)");
        }
    }
    None
}

/// Check if a file path matches the sync filter criteria. Compiles the
/// filter on every call; loops use [`CompiledSyncFilter`] once instead. A
/// filter with an invalid ordered rule matches nothing.
pub fn matches_filter(relative_path: &str, size: u64, filter: &SyncFilter) -> bool {
    CompiledSyncFilter::new(filter).map(|f| f.accepts(relative_path, size)).unwrap_or(false)
}

/// Wildcard match on the shared rule engine (case-sensitive, `*` crosses `/`).
fn glob_match(pattern: &str, path: &str) -> bool {
    crate::filter_rules::wildcard_matches(pattern, path, true)
}

/// Check path length for Windows compatibility (260 char limit).
fn check_path_length(dest_root: &str, relative_path: &str) -> Option<String> {
    let full_len = dest_root.len() + 1 + relative_path.len();
    if full_len > 260 {
        Some(format!(
            "Path length {} exceeds Windows MAX_PATH (260). May cause issues on Windows.",
            full_len
        ))
    } else if full_len > 240 {
        Some(format!(
            "Path length {} is close to Windows MAX_PATH limit (260).",
            full_len
        ))
    } else {
        None
    }
}

/// Inputs beyond the pair that shape a plan.
#[derive(Default, Clone, Copy)]
pub struct PlanContext<'a> {
    /// Two-way only: what both sides looked like after the last successful
    /// run, for a three-way diff. `None` = first run (union, never delete).
    /// A baseline recorded for other folders is ignored.
    pub baseline: Option<&'a SyncBaseline>,
    /// Checked between entries; when set, planning stops with an error.
    pub cancel: Option<&'a AtomicBool>,
    /// Content hashes for [`CompareMode::Checksum`]. [`plan`] hashes local
    /// files itself when this is `None`; [`diff_trees`] without one compares
    /// by size and time and says so in the preview notes.
    pub hasher: Option<&'a dyn ContentHasher>,
    /// Versioned-backup mode: the stored versions. [`plan`] reads the
    /// destination's index itself when this is `None`.
    pub versions: Option<&'a super::versions::VersionIndex>,
}

/// Content hashes for checksum comparison: a storage provider's own hashes,
/// or local files read and hashed. `None` = no hash for that file.
pub trait ContentHasher {
    fn hash(&self, side: SyncSide, file: &FileMeta) -> Option<String>;
}

/// Hashes local files with xxHash3.
pub struct LocalHasher;

impl ContentHasher for LocalHasher {
    fn hash(&self, _side: SyncSide, file: &FileMeta) -> Option<String> {
        super::executor::compute_file_hash(&file.absolute_path, SyncHashAlgorithm::Xxh3).ok()
    }
}

/// Decides whether files match, per the pair's [`CompareMode`]; caches
/// hashes so each file is read at most once per plan.
struct Comparer<'a> {
    opts: &'a CopyOptions,
    hasher: Option<&'a dyn ContentHasher>,
    hashes: std::cell::RefCell<HashMap<(bool, String), Option<String>>>,
    /// Paths with a side that could not be hashed.
    unhashed: std::cell::RefCell<std::collections::HashSet<String>>,
}

impl<'a> Comparer<'a> {
    fn new(opts: &'a CopyOptions, hasher: Option<&'a dyn ContentHasher>) -> Self {
        Self { opts, hasher, hashes: Default::default(), unhashed: Default::default() }
    }

    fn hash(&self, side: SyncSide, file: &FileMeta) -> Option<String> {
        let hasher = self.hasher?;
        let key = (side == SyncSide::Source, file.relative_path.clone());
        if let Some(h) = self.hashes.borrow().get(&key) {
            return h.clone();
        }
        let h = hasher.hash(side, file);
        if h.is_none() {
            self.unhashed.borrow_mut().insert(file.relative_path.clone());
        }
        self.hashes.borrow_mut().insert(key, h.clone());
        h
    }

    /// Whether the source and destination copies hold the same data
    /// (`dest_time`: the destination time adjusted for clock skew).
    fn same(&self, src: &FileMeta, dst: &FileMeta, dest_time: Option<DateTime<Utc>>) -> bool {
        if src.size != dst.size {
            return false;
        }
        match self.opts.compare_mode {
            CompareMode::SizeOnly => true,
            CompareMode::SizeMtime => times_equal(src.modified, dest_time, self.opts),
            CompareMode::Checksum => match (self.hash(SyncSide::Source, src), self.hash(SyncSide::Dest, dst)) {
                (Some(a), Some(b)) => a == b,
                _ => times_equal(src.modified, dest_time, self.opts),
            },
        }
    }

    /// Whether `file` changed since the baseline recorded its side.
    fn changed_since(&self, side: SyncSide, file: &FileMeta, b: &BaselineEntry) -> bool {
        if file.size != b.size {
            return true;
        }
        let recorded = if side == SyncSide::Source { b.source_modified } else { b.dest_modified };
        let same_time = times_equal(file.modified, recorded, self.opts);
        match self.opts.compare_mode {
            CompareMode::SizeOnly => false,
            CompareMode::SizeMtime => !same_time,
            // Touched but maybe not edited: the hash decides.
            CompareMode::Checksum if !same_time => match (&b.hash, self.hash(side, file)) {
                (Some(then), Some(now)) => *then != now,
                _ => true,
            },
            CompareMode::Checksum => false,
        }
    }

    /// The baseline entry for a file that matches on both sides.
    fn entry(&self, src: &FileMeta, dst: &FileMeta) -> BaselineEntry {
        let hash = (self.opts.compare_mode == CompareMode::Checksum).then(|| self.hash(SyncSide::Source, src)).flatten();
        BaselineEntry { size: src.size, source_modified: src.modified, dest_modified: dst.modified, hash }
    }

    fn notes(&self) -> Vec<String> {
        if self.opts.compare_mode != CompareMode::Checksum {
            return Vec::new();
        }
        match (self.hasher, self.unhashed.borrow().len()) {
            (None, _) => vec!["Checksums are not available here; files were compared by size and modification time.".to_string()],
            (Some(_), 0) => Vec::new(),
            (Some(_), n) => vec![format!("{n} file(s) could not be hashed and were compared by size and modification time.")],
        }
    }
}

/// A plan: the preview, plus the two-way baseline decisions that do not
/// depend on how the run goes.
#[derive(Debug, Clone)]
pub struct Plan {
    pub preview: SyncPreview,
    /// Two-way: paths whose baseline entry the plan alone settles — `Some`
    /// for files that already match on both sides, `None` for files gone
    /// from both. Paths not listed keep their previous entry unless the run
    /// changes them.
    pub settled: Vec<(String, Option<BaselineEntry>)>,
}

/// Compute a dry-run preview for a sync pair.
pub fn compute_preview(pair: &SyncPair) -> Result<SyncPreview, AppError> {
    plan(pair, &PlanContext::default()).map(|p| p.preview)
}

/// Walk both sides of `pair` and plan the run.
pub fn plan(pair: &SyncPair, ctx: &PlanContext<'_>) -> Result<Plan, AppError> {
    let start = Instant::now();

    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);

    // Validate paths
    if !source_root.exists() {
        return Err(AppError::Sync {
            message: format!("Source path does not exist: {}", pair.source_path),
            advice: "Check the source path in your sync pair configuration.".to_string(),
        });
    }

    let opts = &pair.copy_options;
    let baseline = usable_baseline(pair, ctx.baseline);
    // A two-way pair with a baseline whose destination vanished would read
    // as "everything deleted there": refuse instead of emptying the source.
    if let Some(b) = baseline {
        if !b.entries.is_empty() && !dest_root.exists() {
            return Err(AppError::Sync {
                message: format!("Destination folder is missing: {}", pair.dest_path),
                advice: "Reconnect or mount the destination, then run again. Nothing was deleted.".to_string(),
            });
        }
    }

    // Walk the source; learn the destination per the pair's state mode.
    let source_files = walk_directory_with(source_root, opts)?;
    let dest_files = match pair.dest_state {
        _ if !dest_root.exists() => Vec::new(),
        DestStateMode::BlindUpload => Vec::new(),
        DestStateMode::TrustedInventory => match load_inventory(dest_root) {
            Some(inv) => inv,
            None => walk_directory_with(dest_root, opts)?,
        },
        DestStateMode::Scan => walk_directory_with(dest_root, opts)?,
    };

    let local = LocalHasher;
    let index = (pair.mode == SyncMode::VersionedBackup && ctx.versions.is_none() && dest_root.exists())
        .then(|| super::versions::VersionIndex::load(dest_root));
    let ctx = PlanContext {
        hasher: ctx.hasher.or((opts.compare_mode == CompareMode::Checksum).then_some(&local as &dyn ContentHasher)),
        versions: ctx.versions.or(index.as_ref()),
        ..*ctx
    };
    let mut plan = diff_trees(pair, &source_files, &dest_files, &ctx)?;
    plan.preview.duration_ms = start.elapsed().as_millis() as u64;
    Ok(plan)
}

/// The baseline in `ctx`, when it applies to this pair (two-way, same folders,
/// destination observed). Blind upload never lists the destination, so every
/// tracked file would read as "deleted there": deletions may only come from
/// a side that was actually looked at.
fn usable_baseline<'a>(pair: &SyncPair, baseline: Option<&'a SyncBaseline>) -> Option<&'a SyncBaseline> {
    baseline.filter(|b| pair.mode == SyncMode::TwoWay && pair.dest_state != DestStateMode::BlindUpload && b.matches(pair))
}

fn check_cancel(ctx: &PlanContext<'_>) -> Result<(), AppError> {
    if ctx.cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
        return Err(AppError::Sync {
            message: "Comparison cancelled.".to_string(),
            advice: "Start it again when you are ready.".to_string(),
        });
    }
    Ok(())
}

/// Accumulates the entries of a preview.
#[derive(Default)]
struct Acc {
    additions: Vec<SyncDiffEntry>,
    modifications: Vec<SyncDiffEntry>,
    deletions: Vec<SyncDiffEntry>,
    skipped: Vec<SyncDiffEntry>,
    conflicts: Vec<SyncDiffEntry>,
    path_warnings: Vec<SyncDiffEntry>,
    total_bytes: u64,
    fixups: Vec<String>,
    settled: Vec<(String, Option<BaselineEntry>)>,
    notes: Vec<String>,
}

impl Acc {
    fn push(&mut self, entry: SyncDiffEntry) {
        if entry.path_length_warning.is_some() {
            self.path_warnings.push(entry.clone());
        }
        match entry.action {
            SyncAction::Add => {
                self.total_bytes += entry_bytes(&entry);
                self.additions.push(entry)
            }
            SyncAction::Modify => {
                self.total_bytes += entry_bytes(&entry);
                self.modifications.push(entry)
            }
            SyncAction::Delete => self.deletions.push(entry),
            SyncAction::Conflict => self.conflicts.push(entry),
            SyncAction::Skip => self.skipped.push(entry),
        }
    }
}

/// Bytes an add/modify moves: the size of the side it copies from.
fn entry_bytes(e: &SyncDiffEntry) -> u64 {
    match e.direction {
        SyncDirection::ToDest => e.source_size.unwrap_or(0),
        SyncDirection::ToSource => e.dest_size.unwrap_or(0),
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_entry(
    rel_path: &str,
    action: SyncAction,
    src: Option<&FileMeta>,
    dst: Option<&FileMeta>,
    reason: impl Into<String>,
    direction: SyncDirection,
    path_length_warning: Option<String>,
) -> SyncDiffEntry {
    SyncDiffEntry {
        relative_path: rel_path.to_string(),
        action,
        source_size: src.map(|s| s.size),
        dest_size: dst.map(|d| d.size),
        source_modified: src.and_then(|s| s.modified),
        dest_modified: dst.and_then(|d| d.modified),
        reason: reason.into(),
        path_length_warning,
        direction,
    }
}

/// Diff two listings of the same pair without touching the filesystem.
///
/// `source` and `dest` are the files of each side, as [`walk_directory_with`]
/// lists them (relative paths, sizes, modification times). Applies the
/// pair's filters, Robocopy-style options and mode; for a two-way pair with
/// a baseline in `ctx`, runs the three-way comparison. `ctx.cancel` is
/// checked between entries. This is the planner's whole decision logic, so
/// any caller with two listings (a folder comparison, a remote listing) gets
/// exactly the answers a sync would.
pub fn diff_trees(
    pair: &SyncPair,
    source_files: &[FileMeta],
    dest_files: &[FileMeta],
    ctx: &PlanContext<'_>,
) -> Result<Plan, AppError> {
    let start = Instant::now();
    let opts = &pair.copy_options;
    let now = Utc::now();
    let baseline = usable_baseline(pair, ctx.baseline);

    // Build lookup maps by relative path. The completion marker is the
    // engine's own output: never copied, never purged.
    let marker = pair.run_options.completion_marker.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let source_map: HashMap<&str, &FileMeta> = source_files
        .iter()
        .filter(|f| !f.is_dir && Some(f.relative_path.as_str()) != marker)
        .map(|f| (f.relative_path.as_str(), f))
        .collect();

    // Versioned mode: stored versions are the engine's own files — never
    // purged, never compared as if they were synced copies.
    let version_paths = match (pair.mode, ctx.versions) {
        (SyncMode::VersionedBackup, Some(v)) => v.version_paths(),
        _ => Default::default(),
    };
    let dest_map: HashMap<&str, &FileMeta> = dest_files
        .iter()
        .filter(|f| !f.is_dir && Some(f.relative_path.as_str()) != marker && !version_paths.contains(f.relative_path.as_str()))
        .map(|f| (f.relative_path.as_str(), f))
        .collect();

    let filter = CompiledSyncFilter::new(&pair.filter)?;
    let cmp = Comparer::new(opts, ctx.hasher);
    let mut acc = Acc::default();
    if let Some(b) = baseline {
        diff_two_way(pair, &filter, &cmp, &source_map, &dest_map, b, ctx, now, &mut acc)?;
    } else {
        diff_legacy(pair, &filter, &cmp, &source_map, &dest_map, ctx, now, &mut acc)?;
        if pair.mode == SyncMode::TwoWay && pair.dest_state == DestStateMode::BlindUpload {
            acc.notes.push("Blind upload: the destination is not read, so two-way runs only copy source files to it and never delete.".to_string());
        } else if pair.mode == SyncMode::TwoWay {
            acc.notes.push("First two-way run: files are combined in both directions and nothing is deleted. Later runs propagate deletions.".to_string());
        }
    }
    acc.notes.extend(cmp.notes());
    let Acc { additions, modifications, deletions, skipped, conflicts, path_warnings, total_bytes, fixups, settled, notes } = acc;
    let purge_extras = purges_extras(pair);

    // Directory tree: /E creates empty source directories at the destination;
    // purging removes destination directories the source no longer has.
    let source_dirs: std::collections::HashSet<&str> = source_files
        .iter()
        .filter(|f| f.is_dir)
        .map(|f| f.relative_path.as_str())
        .collect();
    let dest_dirs: std::collections::HashSet<&str> = dest_files
        .iter()
        .filter(|f| f.is_dir)
        .map(|f| f.relative_path.as_str())
        .collect();
    let mut dirs_to_create: Vec<String> = if opts.include_empty_dirs || opts.create_only {
        source_dirs
            .iter()
            .filter(|d| !dest_dirs.contains(*d))
            .map(|d| d.to_string())
            .collect()
    } else {
        Vec::new()
    };
    dirs_to_create.sort();
    let mut dirs_to_remove: Vec<String> = if purge_extras {
        dest_dirs
            .iter()
            .filter(|d| !source_dirs.contains(*d))
            .map(|d| d.to_string())
            .collect()
    } else {
        Vec::new()
    };
    // Deepest first so children go before parents.
    dirs_to_remove.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| b.cmp(a)));

    let duration = start.elapsed();

    Ok(Plan {
        preview: SyncPreview {
            pair_id: pair.id,
            pair_name: pair.name.clone(),
            computed_at: Utc::now(),
            duration_ms: duration.as_millis() as u64,
            total_additions: additions.len() as u64,
            total_modifications: modifications.len() as u64,
            total_deletions: deletions.len() as u64,
            total_skipped: skipped.len() as u64,
            total_conflicts: conflicts.len() as u64,
            total_bytes,
            additions,
            modifications,
            deletions,
            skipped,
            conflicts,
            path_warnings,
            dirs_to_create,
            dirs_to_remove,
            fixups,
            notes,
        },
        settled,
    })
}

/// Mirror always purges; /PURGE adds it to one-way and backup modes; /XX
/// turns it off everywhere. Two-way never purges — extras flow back instead.
fn purges_extras(pair: &SyncPair) -> bool {
    pair.mode != SyncMode::TwoWay
        && (pair.mode == SyncMode::Mirror || pair.copy_options.purge)
        && !pair.copy_options.exclude_extra
}

/// Why a file on one side is left out of the plan entirely, if it is:
/// filters, attribute/age options, the settle window or the filename policy
/// (checked against the path the file would be written to).
fn exclusion_reason(
    rel_path: &str,
    file: &FileMeta,
    pair: &SyncPair,
    filter: &CompiledSyncFilter,
    target_root: &Path,
    now: DateTime<Utc>,
) -> Option<String> {
    if !filter.accepts(rel_path, file.size) {
        return Some("Excluded by filter".to_string());
    }
    if let Some(reason) = option_exclusion(file, &pair.copy_options, now) {
        return Some(reason.to_string());
    }
    if still_changing(file, pair.run_options.settle_secs, now) {
        return Some("Still changing (modified moments ago); the next run copies it".to_string());
    }
    if let Err(v) = crate::policy::active().check(&target_root.join(rel_path)) {
        return Some(format!("Blocked by filename policy: {}", v.reason));
    }
    None
}

/// The Robocopy comparison exclusions (/XC, /XO, /XN) for copying `from`
/// over `to` (the `to` time already adjusted for clock skew).
fn comparison_exclusion(
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    same_time: bool,
    opts: &CopyOptions,
) -> Option<&'static str> {
    if same_time && opts.exclude_changed {
        Some("Changed file excluded (/XC)")
    } else if !same_time && from < to && opts.exclude_older {
        Some("Source is older than destination (/XO)")
    } else if !same_time && from > to && opts.exclude_newer {
        Some("Source is newer than destination (/XN)")
    } else {
        None
    }
}

/// Destination time adjusted for the pair's clock-skew offset.
fn adjusted_dest_time(dest: &FileMeta, pair: &SyncPair) -> Option<DateTime<Utc>> {
    match (dest.modified, pair.time_offset_secs) {
        (Some(mtime), Some(offset)) => Some(mtime + chrono::Duration::seconds(offset)),
        (mtime, _) => mtime,
    }
}

/// The one-way / mirror / versioned plan, and two-way without a baseline.
#[allow(clippy::too_many_arguments)]
fn diff_legacy(
    pair: &SyncPair,
    filter: &CompiledSyncFilter,
    cmp: &Comparer<'_>,
    source_map: &HashMap<&str, &FileMeta>,
    dest_map: &HashMap<&str, &FileMeta>,
    ctx: &PlanContext<'_>,
    now: DateTime<Utc>,
    acc: &mut Acc,
) -> Result<(), AppError> {
    let opts = &pair.copy_options;
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);
    let skip = |rel_path: &str, src: &FileMeta, reason: &str| {
        diff_entry(rel_path, SyncAction::Skip, Some(src), dest_map.get(rel_path).copied(), reason, SyncDirection::ToDest, None)
    };

    // Process source files
    for (rel_path, src) in source_map {
        check_cancel(ctx)?;
        if let Some(reason) = exclusion_reason(rel_path, src, pair, filter, dest_root, now) {
            acc.push(skip(rel_path, src, &reason));
            continue;
        }

        // Check path length
        let path_warning = check_path_length(&pair.dest_path, rel_path);

        // Versioned mode: compare with the newest stored version.
        if let Some(latest) = ctx.versions.filter(|_| pair.mode == SyncMode::VersionedBackup).and_then(|v| v.latest(rel_path)) {
            let stored = FileMeta {
                relative_path: latest.path.clone(),
                absolute_path: dest_root.join(&latest.path),
                size: latest.size,
                modified: latest.source_modified,
                is_dir: false,
                is_symlink: false,
                hidden: false,
                readonly: false,
                attributes: 0,
            };
            if opts.include_same || !cmp.same(src, &stored, latest.source_modified) {
                let reason = if opts.include_same { "Stored again (/IS)" } else { "Changed since the last stored version" };
                acc.push(diff_entry(rel_path, SyncAction::Modify, Some(src), dest_map.get(rel_path).copied(), reason, SyncDirection::ToDest, path_warning));
            }
            continue;
        }

        if let Some(dest) = dest_map.get(rel_path) {
            // File exists at destination - check if changed
            // Apply time offset compensation for clock skew
            let adjusted_dest_modified = adjusted_dest_time(dest, pair);

            let same_time = times_equal(src.modified, adjusted_dest_modified, opts);
            let differs = match pair.verify_mode {
                SyncVerifyMode::None => false,
                // Compared per the pair's compare mode (size and time by
                // default); Full verification also checksums after the run.
                SyncVerifyMode::Fast | SyncVerifyMode::Full => !cmp.same(src, dest, adjusted_dest_modified),
            };
            let changed = differs || opts.include_same;

            if changed && differs {
                // Robocopy-style comparison exclusions (/XC, /XO, /XN).
                if let Some(reason) = comparison_exclusion(src.modified, adjusted_dest_modified, same_time, opts) {
                    acc.push(skip(rel_path, src, reason));
                    continue;
                }
            }

            if changed {
                // Check for conflict in two-way mode
                if pair.mode == SyncMode::TwoWay && differs {
                    // Both sides changed = conflict
                    let is_conflict = dest.modified.is_some()
                        && src.modified.is_some()
                        && dest.modified != src.modified
                        && dest.size != src.size;

                    if is_conflict {
                        acc.push(diff_entry(rel_path, SyncAction::Conflict, Some(src), Some(dest), "Both source and destination modified", SyncDirection::ToDest, path_warning));
                        continue;
                    }
                }

                let reason = if differs {
                    "File changed (size or modification time differs)"
                } else {
                    "Identical file re-copied (/IS)"
                };
                acc.push(diff_entry(rel_path, SyncAction::Modify, Some(src), Some(dest), reason, SyncDirection::ToDest, path_warning));
            } else {
                if pair.mode == SyncMode::TwoWay {
                    acc.settled.push((rel_path.to_string(), Some(cmp.entry(src, dest))));
                }
                if opts.fix_timestamps || opts.fix_security {
                    acc.fixups.push(rel_path.to_string());
                }
            }
        } else if opts.exclude_lonely {
            acc.push(skip(rel_path, src, "Not present at destination (/XL)"));
        } else {
            // New file at source
            acc.push(diff_entry(rel_path, SyncAction::Add, Some(src), None, "New file in source", SyncDirection::ToDest, path_warning));
        }
    }

    // Process deletions (files in dest not in source)
    match pair.mode {
        _ if purges_extras(pair) => {
            // Delete files in dest that are not in source
            for (rel_path, dest) in dest_map {
                if !source_map.contains_key(rel_path) {
                    let reason = if pair.mode == SyncMode::Mirror {
                        "Not present in source (mirror mode)"
                    } else {
                        "Not present in source (/PURGE)"
                    };
                    acc.push(diff_entry(rel_path, SyncAction::Delete, None, Some(dest), reason, SyncDirection::ToDest, None));
                }
            }
        }
        SyncMode::TwoWay => {
            // Two-way: files only in dest are copied to the source.
            for (rel_path, dest) in dest_map {
                check_cancel(ctx)?;
                if !source_map.contains_key(rel_path) && exclusion_reason(rel_path, dest, pair, filter, source_root, now).is_none() {
                    acc.push(diff_entry(
                        rel_path,
                        SyncAction::Add,
                        None,
                        Some(dest),
                        "New file in destination (two-way sync)",
                        SyncDirection::ToSource,
                        check_path_length(&pair.source_path, rel_path),
                    ));
                }
            }
        }
        _ => {
            // OneWay, VersionedBackup, or /XX: no deletions from dest
        }
    }
    Ok(())
}

/// Two-way with a baseline: a three-way diff of source, destination and
/// the state both had after the last successful run.
///
/// | source | dest | baseline | action |
/// |---|---|---|---|
/// | changed | unchanged | yes | copy to destination |
/// | unchanged | changed | yes | copy to source |
/// | changed | changed | yes | conflict (unless both now match) |
/// | present | present | no | in sync if they match, else conflict |
/// | unchanged | missing | yes | deleted at destination: delete in source |
/// | changed | missing | yes | changed after the delete: copy back |
/// | present | missing | no | new: copy to destination |
///
/// and the mirror image for files missing in the source.
#[allow(clippy::too_many_arguments)]
fn diff_two_way(
    pair: &SyncPair,
    filter: &CompiledSyncFilter,
    cmp: &Comparer<'_>,
    source_map: &HashMap<&str, &FileMeta>,
    dest_map: &HashMap<&str, &FileMeta>,
    baseline: &SyncBaseline,
    ctx: &PlanContext<'_>,
    now: DateTime<Utc>,
    acc: &mut Acc,
) -> Result<(), AppError> {
    let opts = &pair.copy_options;
    let source_root = Path::new(&pair.source_path);
    let dest_root = Path::new(&pair.dest_path);

    let mut paths: Vec<&str> = source_map.keys().chain(dest_map.keys()).copied().collect();
    paths.extend(baseline.entries.keys().map(String::as_str));
    paths.sort_unstable();
    paths.dedup();

    let (mut deletes_in_source, mut deletes_in_dest) = (0usize, 0usize);
    for rel_path in paths {
        check_cancel(ctx)?;
        let src = source_map.get(rel_path).copied();
        let dst = dest_map.get(rel_path).copied();
        let base = baseline.entries.get(rel_path);

        // Excluded on a side it exists on: leave the path alone entirely, and
        // keep its baseline entry for when it is included again.
        let excluded = src
            .and_then(|s| exclusion_reason(rel_path, s, pair, filter, dest_root, now))
            .or_else(|| dst.and_then(|d| exclusion_reason(rel_path, d, pair, filter, source_root, now)));
        if let Some(reason) = excluded {
            if let Some(s) = src {
                acc.push(diff_entry(rel_path, SyncAction::Skip, Some(s), dst, reason, SyncDirection::ToDest, None));
            }
            continue;
        }
        let to_dest_warning = || check_path_length(&pair.dest_path, rel_path);
        let to_source_warning = || check_path_length(&pair.source_path, rel_path);

        match (src, dst) {
            (Some(s), Some(d)) => {
                let adjusted = adjusted_dest_time(d, pair);
                let (s_changed, d_changed) = match base {
                    Some(b) => (cmp.changed_since(SyncSide::Source, s, b), cmp.changed_since(SyncSide::Dest, d, b)),
                    None => (true, true),
                };
                // Only compared when needed: in checksum mode this reads both files.
                let same = || cmp.same(s, d, adjusted);
                if (!s_changed && !d_changed) || (!opts.include_same && same()) {
                    acc.settled.push((rel_path.to_string(), Some(cmp.entry(s, d))));
                    if opts.fix_timestamps || opts.fix_security {
                        acc.fixups.push(rel_path.to_string());
                    }
                    continue;
                }
                if s_changed && d_changed {
                    let reason = if base.is_some() {
                        "Changed in both source and destination since the last sync"
                    } else {
                        "Different files in source and destination, not seen by an earlier sync"
                    };
                    acc.push(diff_entry(rel_path, SyncAction::Conflict, Some(s), Some(d), reason, SyncDirection::ToDest, to_dest_warning()));
                    continue;
                }
                let (direction, from, to, reason, warning) = if s_changed {
                    (SyncDirection::ToDest, s.modified, adjusted, "Changed in source since the last sync", to_dest_warning())
                } else {
                    (SyncDirection::ToSource, adjusted, s.modified, "Changed in destination since the last sync", to_source_warning())
                };
                let same_time = times_equal(from, to, opts);
                if let Some(reason) = comparison_exclusion(from, to, same_time, opts) {
                    acc.push(diff_entry(rel_path, SyncAction::Skip, Some(s), Some(d), reason, direction, None));
                    continue;
                }
                acc.push(diff_entry(rel_path, SyncAction::Modify, Some(s), Some(d), reason, direction, warning));
            }
            (Some(s), None) => match base {
                Some(b) if !cmp.changed_since(SyncSide::Source, s, b) => {
                    deletes_in_source += 1;
                    acc.push(diff_entry(rel_path, SyncAction::Delete, Some(s), None, "Deleted in destination since the last sync", SyncDirection::ToSource, None));
                }
                Some(_) => acc.push(diff_entry(rel_path, SyncAction::Add, Some(s), None, "Changed in source after it was deleted in destination; copied back", SyncDirection::ToDest, to_dest_warning())),
                None if opts.exclude_lonely => {
                    acc.push(diff_entry(rel_path, SyncAction::Skip, Some(s), None, "Not present at destination (/XL)", SyncDirection::ToDest, None))
                }
                None => acc.push(diff_entry(rel_path, SyncAction::Add, Some(s), None, "New file in source", SyncDirection::ToDest, to_dest_warning())),
            },
            (None, Some(d)) => match base {
                Some(b) if !cmp.changed_since(SyncSide::Dest, d, b) => {
                    deletes_in_dest += 1;
                    acc.push(diff_entry(rel_path, SyncAction::Delete, None, Some(d), "Deleted in source since the last sync", SyncDirection::ToDest, None));
                }
                Some(_) => acc.push(diff_entry(rel_path, SyncAction::Add, None, Some(d), "Changed in destination after it was deleted in source; copied back", SyncDirection::ToSource, to_source_warning())),
                None => acc.push(diff_entry(rel_path, SyncAction::Add, None, Some(d), "New file in destination (two-way sync)", SyncDirection::ToSource, to_source_warning())),
            },
            // Gone from both sides: nothing to do, and nothing to remember.
            (None, None) => acc.settled.push((rel_path.to_string(), None)),
        }
    }

    check_delete_safety(pair, baseline, dest_map.is_empty(), deletes_in_source, "source")?;
    check_delete_safety(pair, baseline, source_map.is_empty(), deletes_in_dest, "destination")?;
    Ok(())
}

/// The two-way safety stop: a side that suddenly lost most (or all) of its
/// files is far more likely unmounted or emptied by mistake than cleaned up
/// on purpose, and propagating that would empty the other side too.
/// `missing_side_empty`: the side the files vanished from has no files left.
fn check_delete_safety(
    pair: &SyncPair,
    baseline: &SyncBaseline,
    missing_side_empty: bool,
    deletes: usize,
    side_deleting: &str,
) -> Result<(), AppError> {
    let pct = pair.copy_options.two_way_max_delete_percent;
    let tracked = baseline.entries.len();
    if pct == 0 || deletes == 0 || tracked == 0 {
        return Ok(());
    }
    let other = if side_deleting == "source" { "destination" } else { "source" };
    let too_many = deletes > 2 && deletes * 100 > pct as usize * tracked;
    if missing_side_empty || too_many {
        let what = if missing_side_empty { format!("the {other} is empty") } else { format!("they are missing in the {other}") };
        return Err(AppError::Sync {
            message: format!(
                "Safety stop: this run would delete {deletes} of {tracked} synced files in the {side_deleting} because {what}. Nothing was changed."
            ),
            advice: format!(
                "Check that the {other} folder is connected and complete. If the deletions are intended, raise or turn off the two-way delete limit ({pct}%) in the pair's options, or reset the pair's sync history."
            ),
        });
    }
    Ok(())
}

/// Export a preview as CSV.
pub fn export_preview_csv(preview: &SyncPreview) -> Result<String, AppError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());

    wtr.write_record([
        "Action",
        "Path",
        "Source Size",
        "Dest Size",
        "Source Modified",
        "Dest Modified",
        "Reason",
        "Warning",
    ])
    .map_err(|e| AppError::Sync {
        message: format!("CSV export error: {}", e),
        advice: "Try again.".to_string(),
    })?;

    let all_entries = preview
        .additions
        .iter()
        .chain(preview.modifications.iter())
        .chain(preview.deletions.iter())
        .chain(preview.skipped.iter())
        .chain(preview.conflicts.iter());

    for entry in all_entries {
        wtr.write_record([
            format!("{:?}", entry.action),
            entry.relative_path.clone(),
            entry.source_size.map(|s| s.to_string()).unwrap_or_default(),
            entry.dest_size.map(|s| s.to_string()).unwrap_or_default(),
            entry
                .source_modified
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            entry
                .dest_modified
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            entry.reason.clone(),
            entry.path_length_warning.clone().unwrap_or_default(),
        ])
        .map_err(|e| AppError::Sync {
            message: format!("CSV export error: {}", e),
            advice: "Try again.".to_string(),
        })?;
    }

    let data = wtr.into_inner().map_err(|e| AppError::Sync {
        message: format!("CSV export error: {}", e),
        advice: "Try again.".to_string(),
    })?;

    String::from_utf8(data).map_err(|e| AppError::Sync {
        message: format!("CSV encoding error: {}", e),
        advice: "Try again.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_dirs() -> (TempDir, TempDir) {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        (src, dst)
    }

    #[test]
    fn test_walk_directory_empty() {
        let dir = TempDir::new().unwrap();
        let entries = walk_directory(dir.path()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_walk_directory_with_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.txt"), "world").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/c.txt"), "nested").unwrap();

        let entries = walk_directory(dir.path()).unwrap();
        let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_matches_filter_include_glob() {
        let filter = SyncFilter {
            include_patterns: vec!["*.txt".to_string()],
            ..Default::default()
        };
        assert!(matches_filter("file.txt", 100, &filter));
        assert!(!matches_filter("file.jpg", 100, &filter));
    }

    #[test]
    fn test_matches_filter_exclude_glob() {
        let filter = SyncFilter {
            exclude_patterns: vec!["*.tmp".to_string()],
            ..Default::default()
        };
        assert!(matches_filter("file.txt", 100, &filter));
        assert!(!matches_filter("file.tmp", 100, &filter));
    }

    #[test]
    fn test_matches_filter_size() {
        let filter = SyncFilter {
            min_size: 100,
            max_size: 1000,
            ..Default::default()
        };
        assert!(!matches_filter("f.txt", 50, &filter));
        assert!(matches_filter("f.txt", 500, &filter));
        assert!(!matches_filter("f.txt", 2000, &filter));
    }

    #[test]
    fn test_matches_filter_file_types() {
        let filter = SyncFilter {
            file_types: vec!["jpg".to_string(), "png".to_string()],
            ..Default::default()
        };
        assert!(matches_filter("photo.jpg", 100, &filter));
        assert!(matches_filter("photo.PNG", 100, &filter));
        assert!(!matches_filter("doc.pdf", 100, &filter));
    }

    #[test]
    fn test_compute_preview_additions() {
        let (src, dst) = setup_dirs();
        fs::write(src.path().join("new.txt"), "new content").unwrap();

        let pair = SyncPair {
            id: uuid::Uuid::new_v4(),
            name: "Test".to_string(),
            source_path: src.path().to_string_lossy().to_string(),
            dest_path: dst.path().to_string_lossy().to_string(),
            mode: SyncMode::OneWay,
            enabled: true,
            last_run: None,
            trigger: SyncTrigger::Manual,
            filter: SyncFilter::default(),
            conflict_policy: SyncConflictPolicy::CreateConflictCopy,
            verify_mode: SyncVerifyMode::Fast,
            checksum_enabled: false,
            created_at: Utc::now(),
            time_offset_secs: None,
            copy_options: CopyOptions::default(),
            ..Default::default()
        };

        let preview = compute_preview(&pair).unwrap();
        assert_eq!(preview.total_additions, 1);
        assert_eq!(preview.additions[0].relative_path, "new.txt");
    }

    #[test]
    fn test_compute_preview_modifications() {
        let (src, dst) = setup_dirs();
        fs::write(src.path().join("file.txt"), "updated content longer").unwrap();
        fs::write(dst.path().join("file.txt"), "old").unwrap();

        let pair = SyncPair {
            id: uuid::Uuid::new_v4(),
            name: "Test".to_string(),
            source_path: src.path().to_string_lossy().to_string(),
            dest_path: dst.path().to_string_lossy().to_string(),
            mode: SyncMode::OneWay,
            enabled: true,
            last_run: None,
            trigger: SyncTrigger::Manual,
            filter: SyncFilter::default(),
            conflict_policy: SyncConflictPolicy::CreateConflictCopy,
            verify_mode: SyncVerifyMode::Fast,
            checksum_enabled: false,
            created_at: Utc::now(),
            time_offset_secs: None,
            copy_options: CopyOptions::default(),
            ..Default::default()
        };

        let preview = compute_preview(&pair).unwrap();
        assert_eq!(preview.total_modifications, 1);
    }

    #[test]
    fn test_compute_preview_mirror_deletions() {
        let (src, dst) = setup_dirs();
        fs::write(dst.path().join("extra.txt"), "should be deleted").unwrap();

        let pair = SyncPair {
            id: uuid::Uuid::new_v4(),
            name: "Test".to_string(),
            source_path: src.path().to_string_lossy().to_string(),
            dest_path: dst.path().to_string_lossy().to_string(),
            mode: SyncMode::Mirror,
            enabled: true,
            last_run: None,
            trigger: SyncTrigger::Manual,
            filter: SyncFilter::default(),
            conflict_policy: SyncConflictPolicy::CreateConflictCopy,
            verify_mode: SyncVerifyMode::Fast,
            checksum_enabled: false,
            created_at: Utc::now(),
            time_offset_secs: None,
            copy_options: CopyOptions::default(),
            ..Default::default()
        };

        let preview = compute_preview(&pair).unwrap();
        assert_eq!(preview.total_deletions, 1);
    }

    #[test]
    fn test_compute_preview_filter_excludes() {
        let (src, dst) = setup_dirs();
        fs::write(src.path().join("keep.txt"), "keep").unwrap();
        fs::write(src.path().join("skip.tmp"), "skip").unwrap();

        let pair = SyncPair {
            id: uuid::Uuid::new_v4(),
            name: "Test".to_string(),
            source_path: src.path().to_string_lossy().to_string(),
            dest_path: dst.path().to_string_lossy().to_string(),
            mode: SyncMode::OneWay,
            enabled: true,
            last_run: None,
            trigger: SyncTrigger::Manual,
            filter: SyncFilter {
                exclude_patterns: vec!["*.tmp".to_string()],
                ..Default::default()
            },
            conflict_policy: SyncConflictPolicy::CreateConflictCopy,
            verify_mode: SyncVerifyMode::Fast,
            checksum_enabled: false,
            created_at: Utc::now(),
            time_offset_secs: None,
            copy_options: CopyOptions::default(),
            ..Default::default()
        };

        let preview = compute_preview(&pair).unwrap();
        assert_eq!(preview.total_additions, 1);
        assert_eq!(preview.total_skipped, 1);
    }

    #[test]
    fn test_export_preview_csv() {
        let preview = SyncPreview {
            pair_id: uuid::Uuid::new_v4(),
            pair_name: "Test".to_string(),
            computed_at: Utc::now(),
            duration_ms: 10,
            additions: vec![SyncDiffEntry {
                relative_path: "new.txt".to_string(),
                action: SyncAction::Add,
                source_size: Some(100),
                dest_size: None,
                source_modified: None,
                dest_modified: None,
                reason: "New file".to_string(),
                path_length_warning: None,
                direction: SyncDirection::ToDest,
            }],
            modifications: Vec::new(),
            deletions: Vec::new(),
            skipped: Vec::new(),
            conflicts: Vec::new(),
            path_warnings: Vec::new(),
            dirs_to_create: Vec::new(),
            dirs_to_remove: Vec::new(),
            fixups: Vec::new(),
            notes: Vec::new(),
            total_additions: 1,
            total_modifications: 0,
            total_deletions: 0,
            total_skipped: 0,
            total_conflicts: 0,
            total_bytes: 100,
        };

        let csv = export_preview_csv(&preview).unwrap();
        assert!(csv.contains("new.txt"));
        assert!(csv.contains("Add"));
    }

    fn pair_for(src: &Path, dst: &Path) -> SyncPair {
        SyncPair { source_path: src.to_string_lossy().to_string(), dest_path: dst.to_string_lossy().to_string(), ..Default::default() }
    }

    fn set_time(path: &Path, secs_ago: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        crate::sync::copier::set_mtime(path, t);
    }

    #[test]
    fn checksum_mode_compares_content() {
        let (src, dst) = setup_dirs();
        // Same bytes, different times: only touched.
        fs::write(src.path().join("touched.txt"), "same").unwrap();
        fs::write(dst.path().join("touched.txt"), "same").unwrap();
        set_time(&src.path().join("touched.txt"), 10);
        set_time(&dst.path().join("touched.txt"), 5000);
        // Same size and time, different bytes.
        fs::write(src.path().join("sneaky.txt"), "abcd").unwrap();
        fs::write(dst.path().join("sneaky.txt"), "wxyz").unwrap();
        set_time(&src.path().join("sneaky.txt"), 100);
        set_time(&dst.path().join("sneaky.txt"), 100);

        let mut pair = pair_for(src.path(), dst.path());
        let p = compute_preview(&pair).unwrap();
        assert_eq!(p.modifications.iter().map(|m| m.relative_path.as_str()).collect::<Vec<_>>(), vec!["touched.txt"]);

        pair.copy_options.compare_mode = CompareMode::Checksum;
        let p = compute_preview(&pair).unwrap();
        assert_eq!(p.modifications.iter().map(|m| m.relative_path.as_str()).collect::<Vec<_>>(), vec!["sneaky.txt"]);
        assert!(p.notes.is_empty(), "local files can always be hashed");
    }

    #[test]
    fn checksum_without_hashes_falls_back_with_a_note() {
        let file = |p: &str, size: u64, secs: i64| FileMeta {
            relative_path: p.into(),
            absolute_path: PathBuf::from(p),
            size,
            modified: Some(Utc::now() - chrono::Duration::seconds(secs)),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            readonly: false,
            attributes: 0,
        };
        let mut pair = SyncPair::default();
        pair.copy_options.compare_mode = CompareMode::Checksum;
        let plan = diff_trees(&pair, &[file("a", 4, 10)], &[file("a", 4, 9000)], &PlanContext::default()).unwrap();
        assert_eq!(plan.preview.total_modifications, 1, "size and time decide without hashes");
        assert!(plan.preview.notes.iter().any(|n| n.contains("Checksums are not available")));

        struct Fixed;
        impl ContentHasher for Fixed {
            fn hash(&self, _: SyncSide, f: &FileMeta) -> Option<String> {
                (f.relative_path != "unhashable").then(|| "same".to_string())
            }
        }
        let ctx = PlanContext { hasher: Some(&Fixed), ..Default::default() };
        let plan = diff_trees(&pair, &[file("a", 4, 10), file("unhashable", 4, 10)], &[file("a", 4, 9000), file("unhashable", 4, 9000)], &ctx).unwrap();
        assert_eq!(plan.preview.modifications.iter().map(|m| m.relative_path.as_str()).collect::<Vec<_>>(), vec!["unhashable"]);
        assert!(plan.preview.notes.iter().any(|n| n.starts_with("1 file(s) could not be hashed")), "{:?}", plan.preview.notes);
    }

    #[test]
    fn size_only_ignores_times() {
        let (src, dst) = setup_dirs();
        fs::write(src.path().join("a.txt"), "1234").unwrap();
        fs::write(dst.path().join("a.txt"), "abcd").unwrap();
        set_time(&dst.path().join("a.txt"), 7200);
        fs::write(src.path().join("b.txt"), "longer").unwrap();
        fs::write(dst.path().join("b.txt"), "short").unwrap();
        let mut pair = pair_for(src.path(), dst.path());
        pair.copy_options.compare_mode = CompareMode::SizeOnly;
        let p = compute_preview(&pair).unwrap();
        assert_eq!(p.modifications.iter().map(|m| m.relative_path.as_str()).collect::<Vec<_>>(), vec!["b.txt"]);
    }

    #[test]
    fn ignore_existing_copies_only_new_files() {
        // The "Ignore existing" preset: /XO + /XN + /XC.
        let (src, dst) = setup_dirs();
        fs::write(src.path().join("new.txt"), "n").unwrap();
        fs::write(src.path().join("older.txt"), "source, older").unwrap();
        fs::write(dst.path().join("older.txt"), "dest").unwrap();
        set_time(&src.path().join("older.txt"), 9000);
        fs::write(src.path().join("newer.txt"), "source, newer").unwrap();
        fs::write(dst.path().join("newer.txt"), "dest").unwrap();
        set_time(&dst.path().join("newer.txt"), 9000);
        fs::write(src.path().join("same-time.txt"), "different size").unwrap();
        fs::write(dst.path().join("same-time.txt"), "x").unwrap();
        set_time(&src.path().join("same-time.txt"), 300);
        set_time(&dst.path().join("same-time.txt"), 300);
        let mut pair = pair_for(src.path(), dst.path());
        pair.copy_options.exclude_older = true;
        pair.copy_options.exclude_newer = true;
        pair.copy_options.exclude_changed = true;
        let p = compute_preview(&pair).unwrap();
        assert_eq!(p.additions.iter().map(|a| a.relative_path.as_str()).collect::<Vec<_>>(), vec!["new.txt"]);
        assert_eq!(p.total_modifications, 0);
        assert_eq!(p.total_skipped, 3);
    }
}
