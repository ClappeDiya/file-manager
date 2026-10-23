//! Pre-backup estimate: scan the sources like a backup would, compare
//! them with the previous version and say how much data the next run
//! reads and at most uploads, and roughly how long it takes.
//!
//! Nothing is read or uploaded. A changed file may share most blocks with
//! its previous copy (content-defined chunking finds them again), which
//! only reading it tells, so the new-data figure is an upper bound ("at
//! most"). The lower bound counts new files and the growth of changed
//! files. The duration uses the throughput of recent runs
//! (`bytes_uploaded / duration`) or an assumed speed.

use super::db::BackupDb;
use super::filters::SourceFilter;
use super::{BackupOptions, EntryKind, FileEntry};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Speed assumed when no run history exists (20 MB/s).
pub const DEFAULT_ASSUMED_BYTES_PER_SEC: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EstimateOptions {
    /// Throughput measured from recent runs (bytes/s), if any.
    pub history_bytes_per_sec: Option<u64>,
    /// Throughput to assume without history (0 = no duration estimate).
    pub assumed_bytes_per_sec: u64,
    /// How many of the largest new or changed files to list.
    pub largest: usize,
}

impl Default for EstimateOptions {
    fn default() -> Self {
        Self { history_bytes_per_sec: None, assumed_bytes_per_sec: DEFAULT_ASSUMED_BYTES_PER_SEC, largest: 10 }
    }
}

/// A file the next backup reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstimatedFile {
    pub path: String,
    pub size: u64,
    /// `new` or `changed`.
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupEstimate {
    pub files: u64,
    pub folders: u64,
    pub bytes_examined: u64,
    pub new_files: u64,
    pub changed_files: u64,
    pub unchanged_files: u64,
    pub deleted_files: u64,
    /// Bytes of new and changed files: the most the run uploads (before
    /// compression and deduplication).
    pub new_bytes_at_most: u64,
    /// New files plus the growth of changed files.
    pub new_bytes_at_least: u64,
    /// True when the figure is only an upper bound (changed files exist).
    pub upper_bound: bool,
    /// Whether a previous version was compared against.
    pub has_previous_version: bool,
    pub bytes_per_sec: Option<u64>,
    /// `history`, `assumed` or `none`.
    pub speed_source: String,
    pub estimated_duration_secs: Option<u64>,
    /// Largest new or changed files, biggest first.
    pub largest: Vec<EstimatedFile>,
    pub warnings: Vec<String>,
}

/// Average throughput of runs given as (bytes uploaded, duration ms);
/// runs that uploaded nothing are left out.
pub fn throughput_from_history(runs: &[(u64, u64)]) -> Option<u64> {
    let (bytes, ms) = runs.iter().filter(|(b, ms)| *b > 0 && *ms > 0).fold((0u128, 0u128), |(b, m), (rb, rm)| (b + *rb as u128, m + *rm as u128));
    (ms > 0).then(|| (bytes * 1000 / ms).min(u64::MAX as u128) as u64).filter(|v| *v > 0)
}

/// Estimate the next backup of `sources` against the newest complete
/// version in `db`.
pub fn estimate(db: &BackupDb, sources: &[PathBuf], filter: &SourceFilter, opts: &BackupOptions, eo: &EstimateOptions) -> Result<BackupEstimate, AppError> {
    let mut est = BackupEstimate::default();
    let prev_set = db.filesets()?.into_iter().find(|f| !f.partial);
    let prev: HashMap<String, FileEntry> = match &prev_set {
        Some(fs) => db.files(fs.id)?.into_iter().map(|f| (f.path.clone(), f)).collect(),
        None => HashMap::new(),
    };
    est.has_previous_version = prev_set.is_some();
    let (entries, warnings) = super::engine::scan_sources(sources, filter, opts)?;
    est.warnings = warnings;
    let mut reads: Vec<EstimatedFile> = Vec::new();
    let mut present: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (_, e) in &entries {
        present.insert(e.path.as_str());
        match e.kind {
            EntryKind::Dir => est.folders += 1,
            EntryKind::Symlink => {}
            EntryKind::File => {
                est.files += 1;
                est.bytes_examined += e.size;
                match prev.get(&e.path) {
                    Some(p) if super::engine::unchanged(p, e, opts) => est.unchanged_files += 1,
                    Some(p) => {
                        est.changed_files += 1;
                        est.new_bytes_at_most += e.size;
                        est.new_bytes_at_least += e.size.saturating_sub(p.size);
                        reads.push(EstimatedFile { path: e.path.clone(), size: e.size, status: "changed".into() });
                    }
                    None => {
                        est.new_files += 1;
                        est.new_bytes_at_most += e.size;
                        est.new_bytes_at_least += e.size;
                        reads.push(EstimatedFile { path: e.path.clone(), size: e.size, status: "new".into() });
                    }
                }
            }
        }
    }
    est.deleted_files = prev.iter().filter(|(p, f)| f.kind == EntryKind::File && !present.contains(p.as_str())).count() as u64;
    est.upper_bound = est.changed_files > 0;
    reads.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    reads.truncate(eo.largest);
    est.largest = reads;
    let (bps, source) = match (eo.history_bytes_per_sec.filter(|v| *v > 0), eo.assumed_bytes_per_sec) {
        (Some(h), _) => (Some(h), "history"),
        (None, a) if a > 0 => (Some(a), "assumed"),
        _ => (None, "none"),
    };
    est.bytes_per_sec = bps;
    est.speed_source = source.into();
    est.estimated_duration_secs = bps.map(|b| est.new_bytes_at_most.div_ceil(b));
    Ok(est)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::test_util::write;
    use crate::backup::{OpContext, RetentionPolicy};

    #[test]
    fn throughput_ignores_empty_runs() {
        assert_eq!(throughput_from_history(&[]), None);
        assert_eq!(throughput_from_history(&[(0, 5000)]), None);
        assert_eq!(throughput_from_history(&[(10_000, 1000), (30_000, 1000), (0, 9000)]), Some(20_000));
    }

    #[test]
    fn estimate_counts_new_changed_and_largest() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write(src.path(), "keep.txt", &[1u8; 1000]);
        write(src.path(), "grow.txt", &[2u8; 1000]);
        write(src.path(), "gone.txt", b"bye");
        let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
        let f = SourceFilter::default().compile().unwrap();
        let o = BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, ..Default::default() };
        // No previous version: everything is new.
        let e = estimate(&db, &[src.path().into()], &f, &o, &EstimateOptions::default()).unwrap();
        assert!(!e.has_previous_version && !e.upper_bound);
        assert_eq!((e.files, e.new_files, e.new_bytes_at_most), (3, 3, 2003));
        crate::backup::run_backup(&be, &db, &[src.path().into()], &f, &o, &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();

        write(src.path(), "grow.txt", &[3u8; 1500]);
        std::fs::remove_file(src.path().join("gone.txt")).unwrap();
        write(src.path(), "big-new.bin", &[4u8; 5000]);
        let eo = EstimateOptions { history_bytes_per_sec: Some(1000), largest: 1, ..Default::default() };
        let e = estimate(&db, &[src.path().into()], &f, &o, &eo).unwrap();
        assert!(e.has_previous_version && e.upper_bound);
        assert_eq!((e.new_files, e.changed_files, e.unchanged_files, e.deleted_files), (1, 1, 1, 1));
        assert_eq!(e.new_bytes_at_most, 6500);
        assert_eq!(e.new_bytes_at_least, 5500, "new file + 500 bytes of growth");
        assert_eq!((e.speed_source.as_str(), e.estimated_duration_secs), ("history", Some(7)));
        assert_eq!(e.largest.len(), 1);
        assert!(e.largest[0].path.ends_with("big-new.bin") && e.largest[0].status == "new");
        // Without history the assumed speed is used; 0 turns the duration off.
        let e = estimate(&db, &[src.path().into()], &f, &o, &EstimateOptions { assumed_bytes_per_sec: 0, ..Default::default() }).unwrap();
        assert_eq!((e.speed_source.as_str(), e.estimated_duration_secs), ("none", None));
    }
}
