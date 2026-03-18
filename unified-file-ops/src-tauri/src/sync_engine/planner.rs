//! Sync planner - computes file diffs and produces dry-run previews.
//!
//! Walks source and destination directories, compares entries via
//! size+mtime (fast) or checksum (full), applies selective filters,
//! and produces a `SyncPreview` without touching the filesystem.

use crate::core::error::AppError;
use crate::core::types::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Metadata for a single file entry during planning.
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub is_dir: bool,
}

/// Recursively walk a directory and collect file metadata.
pub fn walk_directory(root: &Path) -> Result<Vec<FileMeta>, AppError> {
    let mut entries = Vec::new();
    walk_recursive(root, root, &mut entries)?;
    Ok(entries)
}

fn walk_recursive(
    root: &Path,
    current: &Path,
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
        let metadata = std::fs::metadata(&path).map_err(|e| AppError::Sync {
            message: format!("Cannot read metadata for {}: {}", path.display(), e),
            advice: "Check file permissions.".to_string(),
        })?;

        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let modified = metadata
            .modified()
            .ok()
            .map(DateTime::<Utc>::from);

        if metadata.is_dir() {
            entries.push(FileMeta {
                relative_path: relative.clone(),
                absolute_path: path.clone(),
                size: 0,
                modified,
                is_dir: true,
            });
            walk_recursive(root, &path, entries)?;
        } else {
            entries.push(FileMeta {
                relative_path: relative,
                absolute_path: path,
                size: metadata.len(),
                modified,
                is_dir: false,
            });
        }
    }

    Ok(())
}

/// Check if a file path matches the sync filter criteria.
pub fn matches_filter(relative_path: &str, size: u64, filter: &SyncFilter) -> bool {
    // Check include patterns (if any specified, must match at least one)
    if !filter.include_patterns.is_empty() {
        let matches_any = filter
            .include_patterns
            .iter()
            .any(|pat| glob_match(pat, relative_path));
        if !matches_any {
            return false;
        }
    }

    // Check exclude patterns
    if filter
        .exclude_patterns
        .iter()
        .any(|pat| glob_match(pat, relative_path))
    {
        return false;
    }

    // Check include regex
    if !filter.include_regex.is_empty() {
        let matches_any = filter.include_regex.iter().any(|pat| {
            regex::Regex::new(pat)
                .map(|re| re.is_match(relative_path))
                .unwrap_or(false)
        });
        if !matches_any {
            return false;
        }
    }

    // Check exclude regex
    if filter.exclude_regex.iter().any(|pat| {
        regex::Regex::new(pat)
            .map(|re| re.is_match(relative_path))
            .unwrap_or(false)
    }) {
        return false;
    }

    // Size filters
    if filter.min_size > 0 && size < filter.min_size {
        return false;
    }
    if filter.max_size > 0 && size > filter.max_size {
        return false;
    }

    // File type filters
    if !filter.file_types.is_empty() {
        let ext = Path::new(relative_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !filter.file_types.iter().any(|t| t.eq_ignore_ascii_case(ext)) {
            return false;
        }
    }

    // Exclude file types
    if !filter.exclude_file_types.is_empty() {
        let ext = Path::new(relative_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if filter
            .exclude_file_types
            .iter()
            .any(|t| t.eq_ignore_ascii_case(ext))
        {
            return false;
        }
    }

    true
}

/// Simple glob matching using the `glob` crate pattern logic.
fn glob_match(pattern: &str, path: &str) -> bool {
    glob::Pattern::new(pattern)
        .map(|p| p.matches(path))
        .unwrap_or(false)
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

/// Compute a dry-run preview for a sync pair.
pub fn compute_preview(pair: &SyncPair) -> Result<SyncPreview, AppError> {
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

    // Walk both directories
    let source_files = walk_directory(source_root)?;
    let dest_files = if dest_root.exists() {
        walk_directory(dest_root)?
    } else {
        Vec::new()
    };

    // Build lookup maps by relative path
    let source_map: HashMap<&str, &FileMeta> = source_files
        .iter()
        .filter(|f| !f.is_dir)
        .map(|f| (f.relative_path.as_str(), f))
        .collect();

    let dest_map: HashMap<&str, &FileMeta> = dest_files
        .iter()
        .filter(|f| !f.is_dir)
        .map(|f| (f.relative_path.as_str(), f))
        .collect();

    let mut additions = Vec::new();
    let mut modifications = Vec::new();
    let mut deletions = Vec::new();
    let mut skipped = Vec::new();
    let mut conflicts = Vec::new();
    let mut path_warnings = Vec::new();
    let mut total_bytes: u64 = 0;

    // Process source files
    for (rel_path, src) in &source_map {
        // Apply filter
        if !matches_filter(rel_path, src.size, &pair.filter) {
            skipped.push(SyncDiffEntry {
                relative_path: rel_path.to_string(),
                action: SyncAction::Skip,
                source_size: Some(src.size),
                dest_size: dest_map.get(rel_path).map(|d| d.size),
                source_modified: src.modified,
                dest_modified: dest_map.get(rel_path).and_then(|d| d.modified),
                reason: "Excluded by filter".to_string(),
                path_length_warning: None,
            });
            continue;
        }

        // Check path length
        let path_warning = check_path_length(&pair.dest_path, rel_path);

        if let Some(dest) = dest_map.get(rel_path) {
            // File exists at destination - check if changed
            // Apply time offset compensation for clock skew
            let adjusted_dest_modified = match (dest.modified, pair.time_offset_secs) {
                (Some(mtime), Some(offset)) => {
                    Some(mtime + chrono::Duration::seconds(offset))
                }
                (mtime, _) => mtime,
            };

            let changed = match pair.verify_mode {
                SyncVerifyMode::None => false,
                SyncVerifyMode::Fast => {
                    src.size != dest.size || src.modified != adjusted_dest_modified
                }
                SyncVerifyMode::Full => {
                    // For full mode in preview, use size+mtime as approximation
                    // (actual checksum comparison happens during execution)
                    src.size != dest.size || src.modified != adjusted_dest_modified
                }
            };

            if changed {
                // Check for conflict in two-way mode
                if pair.mode == SyncMode::TwoWay {
                    // Both sides changed = conflict
                    let is_conflict = dest.modified.is_some()
                        && src.modified.is_some()
                        && dest.modified != src.modified
                        && dest.size != src.size;

                    if is_conflict {
                        let entry = SyncDiffEntry {
                            relative_path: rel_path.to_string(),
                            action: SyncAction::Conflict,
                            source_size: Some(src.size),
                            dest_size: Some(dest.size),
                            source_modified: src.modified,
                            dest_modified: dest.modified,
                            reason: "Both source and destination modified".to_string(),
                            path_length_warning: path_warning.clone(),
                        };
                        conflicts.push(entry.clone());
                        if path_warning.is_some() {
                            path_warnings.push(entry);
                        }
                        continue;
                    }
                }

                let entry = SyncDiffEntry {
                    relative_path: rel_path.to_string(),
                    action: SyncAction::Modify,
                    source_size: Some(src.size),
                    dest_size: Some(dest.size),
                    source_modified: src.modified,
                    dest_modified: dest.modified,
                    reason: "File changed (size or modification time differs)".to_string(),
                    path_length_warning: path_warning.clone(),
                };
                total_bytes += src.size;
                modifications.push(entry.clone());
                if path_warning.is_some() {
                    path_warnings.push(entry);
                }
            }
        } else {
            // New file at source
            let entry = SyncDiffEntry {
                relative_path: rel_path.to_string(),
                action: SyncAction::Add,
                source_size: Some(src.size),
                dest_size: None,
                source_modified: src.modified,
                dest_modified: None,
                reason: "New file in source".to_string(),
                path_length_warning: path_warning.clone(),
            };
            total_bytes += src.size;
            additions.push(entry.clone());
            if path_warning.is_some() {
                path_warnings.push(entry);
            }
        }
    }

    // Process deletions (files in dest not in source)
    match pair.mode {
        SyncMode::Mirror => {
            // Mirror mode: delete files in dest that are not in source
            for (rel_path, dest) in &dest_map {
                if !source_map.contains_key(rel_path) {
                    deletions.push(SyncDiffEntry {
                        relative_path: rel_path.to_string(),
                        action: SyncAction::Delete,
                        source_size: None,
                        dest_size: Some(dest.size),
                        source_modified: None,
                        dest_modified: dest.modified,
                        reason: "Not present in source (mirror mode)".to_string(),
                        path_length_warning: None,
                    });
                }
            }
        }
        SyncMode::TwoWay => {
            // Two-way: files only in dest should be copied to source (treated as additions in reverse)
            for (rel_path, dest) in &dest_map {
                if !source_map.contains_key(rel_path)
                    && matches_filter(rel_path, dest.size, &pair.filter) {
                        additions.push(SyncDiffEntry {
                            relative_path: rel_path.to_string(),
                            action: SyncAction::Add,
                            source_size: None,
                            dest_size: Some(dest.size),
                            source_modified: None,
                            dest_modified: dest.modified,
                            reason: "New file in destination (two-way sync)".to_string(),
                            path_length_warning: check_path_length(&pair.source_path, rel_path),
                        });
                        total_bytes += dest.size;
                    }
            }
        }
        _ => {
            // OneWay, VersionedBackup: no deletions from dest
        }
    }

    let duration = start.elapsed();

    Ok(SyncPreview {
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
    })
}

/// Export a preview as CSV.
pub fn export_preview_csv(preview: &SyncPreview) -> Result<String, AppError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());

    wtr.write_record(["Action", "Path", "Source Size", "Dest Size", "Source Modified", "Dest Modified", "Reason", "Warning"])
        .map_err(|e| AppError::Sync {
            message: format!("CSV export error: {}", e),
            advice: "Try again.".to_string(),
        })?;

    let all_entries = preview.additions.iter()
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
            entry.source_modified.map(|t| t.to_rfc3339()).unwrap_or_default(),
            entry.dest_modified.map(|t| t.to_rfc3339()).unwrap_or_default(),
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
    use tempfile::TempDir;
    use std::fs;

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
            }],
            modifications: Vec::new(),
            deletions: Vec::new(),
            skipped: Vec::new(),
            conflicts: Vec::new(),
            path_warnings: Vec::new(),
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
}
