//! Versioned-backup bookkeeping.
//!
//! Versioned mode stores every copy under a version name (`doc.v3.txt`, or
//! the pair's template) and never at the plain path, so the destination
//! alone cannot say which stored file is the newest version of what. The
//! index at the destination root records it: per source file, its versions
//! oldest first with the size and time the source had when each was stored.
//! The planner compares the source with the newest version (so an unchanged
//! file is not stored again), purging leaves versions alone, and
//! `keep_versions` prunes the oldest ones with the retention engine's
//! keep-count rule.
//!
//! Destinations written before the index existed have numbered version
//! files and no index; the first run adopts them
//! ([`VersionIndex::adopt_unindexed`]).

use crate::error::AppError;
use crate::retention::RetentionPolicy;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Index file at the destination root (never synced, never purged).
pub const VERSIONS_FILE: &str = ".ufop-sync-versions.json";

/// One stored version of a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredVersion {
    /// Path of the version file, relative to the destination root.
    pub path: String,
    pub size: u64,
    /// The source file's modification time when this version was stored.
    pub source_modified: Option<DateTime<Utc>>,
    pub stored_at: DateTime<Utc>,
    /// Version number (`{num}`); numbers are never reused after pruning.
    #[serde(default)]
    pub seq: u64,
}

/// Versions per source file (relative path), oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionIndex {
    #[serde(default)]
    pub files: BTreeMap<String, Vec<StoredVersion>>,
}

/// What pruning did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneResult {
    pub removed: Vec<String>,
    pub errors: Vec<String>,
}

impl VersionIndex {
    /// The index of `dest_root`; empty when there is none (or it is unreadable).
    /// Entries whose path could lead outside the destination are dropped
    /// ([`VersionIndex::load_checked`] says which).
    pub fn load(dest_root: &Path) -> Self {
        Self::load_checked(dest_root).0
    }

    /// [`VersionIndex::load`], plus a message for every entry dropped
    /// because its path is not a plain relative path inside the destination.
    /// The index lives in the destination, so whoever can write there (a
    /// shared drive, a synced cloud folder) can write it: nothing in it is
    /// trusted to name a file to read, replace or delete.
    pub fn load_checked(dest_root: &Path) -> (Self, Vec<String>) {
        let mut index: Self = std::fs::read(dest_root.join(VERSIONS_FILE)).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
        let rejected = index.drop_unsafe_paths();
        (index, rejected)
    }

    /// Remove entries (and whole files) whose paths are not plain relative
    /// paths; returns a message per removed entry.
    fn drop_unsafe_paths(&mut self) -> Vec<String> {
        let mut rejected = Vec::new();
        self.files.retain(|rel, versions| {
            if !is_safe_relative(rel) {
                rejected.push(format!("Version index entry \"{rel}\" ignored: not a path inside the destination"));
                return false;
            }
            versions.retain(|v| {
                let ok = is_safe_relative(&v.path);
                if !ok {
                    rejected.push(format!("Version index entry \"{}\" (a version of {rel}) ignored: not a path inside the destination", v.path));
                }
                ok
            });
            !versions.is_empty()
        });
        rejected
    }

    /// Whether `dest_root` has an index (a destination without one was
    /// never versioned, or by an app version without the index).
    pub fn exists(dest_root: &Path) -> bool {
        dest_root.join(VERSIONS_FILE).is_file()
    }

    /// Index the version files an older app version stored for the source
    /// files `source_rels` under `pair`'s name template: `doc.v1.txt`,
    /// `doc.v2.txt`, ... (numbers in order, one missing number tolerated).
    /// Only templates whose names depend on nothing but the file name and the
    /// number (the default does) can be recognised. When timestamps are kept
    /// a version's time is the source's when it was stored, so an unchanged
    /// file is not stored again. Returns the number of versions adopted.
    pub fn adopt_unindexed<'a>(&mut self, pair: &crate::sync_types::SyncPair, source_rels: impl IntoIterator<Item = &'a str>) -> usize {
        let template = pair.run_options.version_template.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(super::executor::DEFAULT_VERSION_TEMPLATE);
        let tokens = crate::naming::tokens::tokens_in(template);
        let numbered = tokens.iter().any(|t| t == "num" || t == "counter");
        if !numbered || !tokens.iter().all(|t| matches!(t.as_str(), "name" | "ext" | "filename" | "num" | "counter")) {
            return 0;
        }
        let dest_root = Path::new(&pair.dest_path);
        let keep_times = pair.copy_options.copy_timestamps;
        let mut adopted = 0;
        for rel in source_rels {
            if self.files.contains_key(rel) {
                continue;
            }
            let namer = VersionNamer::new(&dest_root.join(rel), Some(template));
            let (mut n, mut misses) = (1u64, 0);
            // Kept in number order: pruning goes by `stored_at`.
            let mut prev: Option<DateTime<Utc>> = None;
            while misses < 2 {
                let Ok(path) = namer.name(n) else { break };
                match std::fs::metadata(&path) {
                    Ok(md) if md.is_file() => {
                        misses = 0;
                        let modified: Option<DateTime<Utc>> = md.modified().ok().map(Into::into);
                        let Ok(stored) = path.strip_prefix(dest_root) else { break };
                        self.record(
                            rel,
                            StoredVersion {
                                path: stored.to_string_lossy().replace('\\', "/"),
                                size: md.len(),
                                source_modified: modified.filter(|_| keep_times),
                                stored_at: {
                                    let at = modified.unwrap_or_else(Utc::now);
                                    let at = prev.map_or(at, |p| at.max(p + chrono::Duration::nanoseconds(1)));
                                    prev = Some(at);
                                    at
                                },
                                seq: n,
                            },
                        );
                        adopted += 1;
                    }
                    _ => misses += 1,
                }
                n += 1;
            }
        }
        adopted
    }

    /// Write the index atomically.
    pub fn save(&self, dest_root: &Path) -> std::io::Result<()> {
        let tmp = dest_root.join(format!("{VERSIONS_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, dest_root.join(VERSIONS_FILE))
    }

    pub fn latest(&self, rel: &str) -> Option<&StoredVersion> {
        self.files.get(rel).and_then(|v| v.last())
    }

    pub fn record(&mut self, rel: &str, version: StoredVersion) {
        self.files.entry(rel.to_string()).or_default().push(version);
    }

    /// Every version file the index knows.
    pub fn version_paths(&self) -> HashSet<&str> {
        self.files.values().flatten().map(|v| v.path.as_str()).collect()
    }

    /// Keep the newest `keep` versions of each file and delete the older
    /// version files (retention's keep-count rule; the newest always stays).
    pub fn prune(&mut self, dest_root: &Path, keep: u32) -> Result<PruneResult, AppError> {
        let policy = RetentionPolicy::KeepCount { count: keep };
        let now = Utc::now();
        let mut out = PruneResult::default();
        for versions in self.files.values_mut() {
            if versions.len() <= keep as usize {
                continue;
            }
            let candidates: Vec<(i64, DateTime<Utc>)> = versions.iter().enumerate().map(|(i, v)| (i as i64, v.stored_at)).collect();
            let doomed: HashSet<i64> = policy.select_deletions(&candidates, now)?.into_iter().collect();
            let mut kept = Vec::with_capacity(versions.len());
            for (i, v) in versions.drain(..).enumerate() {
                if !doomed.contains(&(i as i64)) {
                    kept.push(v);
                    continue;
                }
                let target = match resolve_inside(dest_root, &v.path) {
                    Ok(Some(target)) => target,
                    // Already gone.
                    Ok(None) => continue,
                    Err(why) => {
                        // Not ours to delete: dropped from the index, file left alone.
                        out.errors.push(format!("{}: old version not removed: {why}", v.path));
                        continue;
                    }
                };
                match std::fs::remove_file(&target) {
                    Ok(()) => out.removed.push(v.path),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        out.errors.push(format!("{}: could not remove old version: {e}", v.path));
                        kept.push(v);
                    }
                }
            }
            *versions = kept;
        }
        Ok(out)
    }
}

/// Whether `path` is a plain relative path: not empty, not absolute, no
/// drive or root, and nothing but ordinary names (no `.` or `..`), with `\`
/// read as a separator too so a Windows-style `..\x` is caught everywhere.
pub fn is_safe_relative(path: &str) -> bool {
    use std::path::Component;
    let plain = |p: &Path| p.components().all(|c| matches!(c, Component::Normal(_)));
    !path.trim().is_empty() && plain(Path::new(path)) && plain(Path::new(&path.replace('\\', "/")))
}

/// The file `rel` under `dest_root`, with its folder resolved through any
/// symlinks: `Ok(None)` when its folder no longer exists, `Err` when `rel`
/// is not a plain relative path or its folder resolves outside `dest_root`.
/// The returned path's folder is the resolved one, so a symlinked folder
/// cannot redirect the caller elsewhere; the file name itself is left as is
/// (removing a symlink removes the link, not its target).
fn resolve_inside(dest_root: &Path, rel: &str) -> Result<Option<PathBuf>, String> {
    if !is_safe_relative(rel) {
        return Err("not a path inside the destination".into());
    }
    let joined = dest_root.join(rel);
    let (Some(parent), Some(name)) = (joined.parent(), joined.file_name()) else {
        return Err("not a file path".into());
    };
    let root = dest_root.canonicalize().map_err(|e| format!("destination unavailable: {e}"))?;
    let parent = match parent.canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !parent.starts_with(&root) {
        return Err("its folder leads outside the destination".into());
    }
    Ok(Some(parent.join(name)))
}

/// Names the versions of one file from a pair's template.
pub(crate) struct VersionNamer {
    template: String,
    base: crate::naming::TokenContext,
    parent: PathBuf,
    has_ext: bool,
    numbered: bool,
}

impl VersionNamer {
    pub(crate) fn new(path: &Path, template: Option<&str>) -> Self {
        let template = template.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(super::executor::DEFAULT_VERSION_TEMPLATE);
        let mut base = crate::naming::TokenContext::for_path(path);
        // A first version has no file yet: date tokens use the run time.
        if base.modified.is_none() {
            base.modified = Some(Utc::now());
        }
        Self {
            template: template.to_string(),
            base,
            parent: path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf(),
            has_ext: path.extension().is_some(),
            numbered: template.contains("{num") || template.contains("{counter"),
        }
    }

    /// Version `n`'s path. A template without `{num}` gets ` (n)` added
    /// from the second one on.
    pub(crate) fn name(&self, n: u64) -> Result<PathBuf, AppError> {
        use crate::naming::{render, sanitize_segment, UnknownTokens};
        let template = &self.template;
        let mut name = render(template, &self.base.clone().with_counter(n, 0), UnknownTokens::Error).map_err(|e| AppError::Sync {
            message: format!("Version name template \"{template}\" is invalid: {e}"),
            advice: "Fix the version naming template in the pair's options.".to_string(),
        })?;
        // "{name}.{ext}" on a file without an extension would end in a dot.
        if !self.has_ext {
            name = name.trim_end_matches('.').to_string();
        }
        if !self.numbered && n > 1 {
            name = match name.rsplit_once('.') {
                Some((stem, ext)) if self.has_ext => format!("{stem} ({n}).{ext}"),
                _ => format!("{name} ({n})"),
            };
        }
        Ok(self.parent.join(sanitize_segment(name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(path: &str, mins_ago: i64) -> StoredVersion {
        StoredVersion { path: path.into(), size: 1, source_modified: None, stored_at: Utc::now() - chrono::Duration::minutes(mins_ago), seq: 0 }
    }

    #[test]
    fn only_plain_relative_paths_are_safe() {
        for ok in ["doc.v1.txt", "sub/doc.v1.txt", "sub\\doc.v1.txt", "a//b.txt"] {
            assert!(is_safe_relative(ok), "{ok}");
        }
        for bad in ["", " ", "/etc/passwd", "../x", "a/../../x", "..\\x", "a\\..\\..\\x", "./x", "\\\\server\\share\\x"] {
            assert!(!is_safe_relative(bad), "{bad}");
        }
        // A crafted index loses its bad entries on load, and says so.
        let d = tempfile::tempdir().unwrap();
        let mut idx = VersionIndex::default();
        idx.record("doc.txt", version("doc.v1.txt", 2));
        idx.record("doc.txt", version("../../etc/passwd", 1));
        idx.record("../escape.txt", version("escape.v1.txt", 1));
        idx.save(d.path()).unwrap();
        let (loaded, rejected) = VersionIndex::load_checked(d.path());
        assert_eq!(rejected.len(), 2, "{rejected:?}");
        assert_eq!(loaded.files.keys().collect::<Vec<_>>(), vec!["doc.txt"]);
        assert_eq!(loaded.files["doc.txt"].len(), 1);
    }

    #[test]
    fn prune_keeps_the_newest_and_round_trips() {
        let d = tempfile::tempdir().unwrap();
        let mut idx = VersionIndex::default();
        for (i, age) in [(1, 30), (2, 20), (3, 10)] {
            let name = format!("doc.v{i}.txt");
            std::fs::write(d.path().join(&name), "x").unwrap();
            idx.record("doc.txt", version(&name, age));
        }
        idx.record("other.txt", version("other.v1.txt", 5));
        let r = idx.prune(d.path(), 2).unwrap();
        assert_eq!(r.removed, vec!["doc.v1.txt".to_string()]);
        assert!(!d.path().join("doc.v1.txt").exists() && d.path().join("doc.v3.txt").exists());
        assert_eq!(idx.latest("doc.txt").unwrap().path, "doc.v3.txt");
        assert_eq!(idx.files["doc.txt"].len(), 2);
        idx.save(d.path()).unwrap();
        assert_eq!(VersionIndex::load(d.path()), idx);
        assert!(idx.version_paths().contains("other.v1.txt"));
    }
}
