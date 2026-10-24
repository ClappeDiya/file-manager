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

use crate::error::AppError;
use crate::retention::RetentionPolicy;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

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
    pub fn load(dest_root: &Path) -> Self {
        std::fs::read(dest_root.join(VERSIONS_FILE)).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
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
                match std::fs::remove_file(dest_root.join(&v.path)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn version(path: &str, mins_ago: i64) -> StoredVersion {
        StoredVersion { path: path.into(), size: 1, source_modified: None, stored_at: Utc::now() - chrono::Duration::minutes(mins_ago), seq: 0 }
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
