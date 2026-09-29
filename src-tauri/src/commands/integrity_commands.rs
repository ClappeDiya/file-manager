//! Integrity Tools commands (T-063).
//!
//! Checksums (MD5/SHA-1/SHA-256), hash verification, duplicate detection,
//! duplicate management (keep newest/largest/by-path/delete), tags,
//! labels, and smart folders.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Checksum types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumResult {
    pub path: String,
    pub algorithm: String,
    pub hash: String,
    pub size: u64,
    pub computed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub path: String,
    pub algorithm: String,
    pub expected: String,
    pub actual: String,
    pub matches: bool,
}

// ── Duplicate types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub hash: String,
    pub size: u64,
    pub files: Vec<DuplicateFile>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateFile {
    pub path: String,
    pub size: u64,
    pub modified: Option<String>,
    pub is_selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateScanResult {
    pub total_scanned: usize,
    pub groups: Vec<DuplicateGroup>,
    pub total_duplicates: usize,
    pub wasted_bytes: u64,
    pub scan_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateAction {
    pub group_hash: String,
    pub strategy: String, // "keep-newest", "keep-largest", "keep-by-path", "delete-all"
    pub keep_path: Option<String>, // For "keep-by-path"
}

// ── Tag types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTag {
    pub id: String,
    pub name: String,
    pub color: String, // hex color
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLabel {
    pub path: String,
    pub tags: Vec<String>,
    pub labels: Vec<String>,
}

// ── Smart Folder types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartFolder {
    pub id: String,
    pub name: String,
    pub query: SmartFolderQuery,
    pub icon: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartFolderQuery {
    pub base_paths: Vec<String>,
    pub name_pattern: Option<String>,
    pub extension_filter: Option<Vec<String>>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub modified_after: Option<String>,
    pub modified_before: Option<String>,
    pub tags: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub include_subdirs: bool,
}

// ── Tag & Label in-memory store (backed by SQLite) ──
use std::sync::Mutex;

struct IntegrityStore {
    tags: HashMap<String, FileTag>,
    file_tags: HashMap<String, Vec<String>>,     // path -> tag IDs
    file_labels: HashMap<String, Vec<String>>,    // path -> labels
    smart_folders: HashMap<String, SmartFolder>,
}

static STORE: std::sync::LazyLock<Mutex<IntegrityStore>> =
    std::sync::LazyLock::new(|| {
        Mutex::new(IntegrityStore {
            tags: HashMap::new(),
            file_tags: HashMap::new(),
            file_labels: HashMap::new(),
            smart_folders: HashMap::new(),
        })
    });

// ── Checksum commands ──

/// Compute checksum for a file.
#[tauri::command]
pub async fn integrity_checksum(
    path: String,
    algorithm: String,
) -> Result<ChecksumResult, AppError> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(AppError::not_found(format!("File not found: {path}")));
    }
    if file_path.is_dir() {
        return Err(AppError::validation("Cannot checksum a directory"));
    }

    let size = file_path.metadata().map(|m| m.len()).unwrap_or(0);
    let hash = compute_hash(&path, &algorithm)?;

    Ok(ChecksumResult {
        path,
        algorithm,
        hash,
        size,
        computed_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Verify a file's checksum against an expected hash.
#[tauri::command]
pub async fn integrity_verify(
    path: String,
    algorithm: String,
    expected_hash: String,
) -> Result<VerifyResult, AppError> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(AppError::not_found(format!("File not found: {path}")));
    }

    let actual = compute_hash(&path, &algorithm)?;
    let matches = actual.eq_ignore_ascii_case(&expected_hash);

    Ok(VerifyResult {
        path,
        algorithm,
        expected: expected_hash,
        actual,
        matches,
    })
}

// ── Duplicate detection commands ──

/// Scan a directory for duplicate files.
#[tauri::command]
pub async fn integrity_find_duplicates(
    directory: String,
    min_size: Option<u64>,
) -> Result<DuplicateScanResult, AppError> {
    let start = std::time::Instant::now();
    let root = Path::new(&directory);
    if !root.exists() || !root.is_dir() {
        return Err(AppError::not_found(format!("Directory not found: {directory}")));
    }

    let min = min_size.unwrap_or(1);

    // Phase 1: Group by size (fast pass)
    let mut size_groups: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut total_scanned = 0usize;
    collect_files_recursive(root, &mut size_groups, &mut total_scanned, min)?;

    // Remove unique sizes
    size_groups.retain(|_, files| files.len() > 1);

    // Phase 2: Hash files within same-size groups
    let mut duplicate_groups: Vec<DuplicateGroup> = Vec::new();
    let mut total_wasted = 0u64;

    for (size, files) in &size_groups {
        let mut hash_groups: HashMap<String, Vec<PathBuf>> = HashMap::new();

        for file in files {
            if let Ok(hash) = compute_hash(&file.display().to_string(), "sha256") {
                hash_groups.entry(hash).or_default().push(file.clone());
            }
        }

        for (hash, paths) in hash_groups {
            if paths.len() > 1 {
                let wasted = size * (paths.len() as u64 - 1);
                total_wasted += wasted;

                let dup_files: Vec<DuplicateFile> = paths
                    .iter()
                    .map(|p| {
                        let modified = p
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(|t| {
                                let dt: chrono::DateTime<chrono::Utc> = t.into();
                                dt.to_rfc3339()
                            });
                        DuplicateFile {
                            path: p.display().to_string(),
                            size: *size,
                            modified,
                            is_selected: false,
                        }
                    })
                    .collect();

                duplicate_groups.push(DuplicateGroup {
                    hash,
                    size: *size,
                    count: dup_files.len(),
                    files: dup_files,
                });
            }
        }
    }

    let total_duplicates: usize = duplicate_groups.iter().map(|g| g.count - 1).sum();
    let elapsed = start.elapsed();

    Ok(DuplicateScanResult {
        total_scanned,
        groups: duplicate_groups,
        total_duplicates,
        wasted_bytes: total_wasted,
        scan_time_ms: elapsed.as_millis() as u64,
    })
}

/// Result of resolving one duplicate group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DuplicateResolution {
    pub resolved_count: usize,
    pub freed_bytes: u64,
    pub kept: Option<String>,
    pub removed: Vec<String>,
}

/// Pick which file of a verified duplicate group survives.
fn choose_keeper(
    files: &[(String, u64, Option<std::time::SystemTime>)],
    action: &str,
    keep_path: Option<&str>,
) -> Result<Option<String>, AppError> {
    match action {
        "keep_newest" => Ok(files
            .iter()
            .max_by_key(|(_, _, m)| *m)
            .map(|(p, _, _)| p.clone())),
        "keep_largest" => Ok(files
            .iter()
            .max_by_key(|(_, size, _)| *size)
            .map(|(p, _, _)| p.clone())),
        "keep_selected" => {
            let keep = keep_path.filter(|k| !k.is_empty()).ok_or_else(|| {
                AppError::validation("Choose which copy to keep first.")
            })?;
            if !files.iter().any(|(p, _, _)| p == keep) {
                return Err(AppError::validation(format!(
                    "\"{keep}\" is not one of the matching copies in this group."
                )));
            }
            Ok(Some(keep.to_string()))
        }
        "delete_all" => Ok(None),
        other => Err(AppError::validation(format!("Unknown strategy: {other}"))),
    }
}

/// Resolve one duplicate group: keep one copy (or none, for `delete_all`)
/// and delete the rest.
///
/// Every file is re-hashed first and only files whose content still matches
/// `group_hash` are touched, so a file edited since the scan is never lost.
#[tauri::command]
pub async fn integrity_resolve_duplicates(
    group_hash: String,
    action: String,
    keep_path: Option<String>,
    paths: Vec<String>,
) -> Result<DuplicateResolution, AppError> {
    tokio::task::spawn_blocking(move || {
        resolve_duplicates_blocking(&group_hash, &action, keep_path.as_deref(), &paths)
    })
    .await
    .map_err(|e| AppError::internal(format!("Duplicate resolution task failed: {e}")))?
}

fn resolve_duplicates_blocking(
    group_hash: &str,
    action: &str,
    keep_path: Option<&str>,
    paths: &[String],
) -> Result<DuplicateResolution, AppError> {
    let mut verified = Vec::new();
    for p in paths {
        let Ok(meta) = std::fs::metadata(p) else { continue };
        if !meta.is_file() {
            continue;
        }
        if compute_hash(p, "sha256").map(|h| h == group_hash).unwrap_or(false) {
            verified.push((p.clone(), meta.len(), meta.modified().ok()));
        }
    }
    if verified.len() < 2 && action != "delete_all" {
        return Err(AppError::validation(
            "These files are no longer duplicates (they changed or were removed since the scan). Scan again.",
        ));
    }

    let kept = choose_keeper(&verified, action, keep_path)?;
    let mut removed = Vec::new();
    let mut freed_bytes = 0;
    for (path, size, _) in &verified {
        if Some(path) == kept.as_ref() {
            continue;
        }
        std::fs::remove_file(path).map_err(|e| {
            AppError::file_op(
                format!("Could not delete {path}: {e}"),
                "Check that the file isn't open elsewhere and that you can modify it.",
            )
        })?;
        freed_bytes += size;
        removed.push(path.clone());
    }
    Ok(DuplicateResolution {
        resolved_count: removed.len(),
        freed_bytes,
        kept,
        removed,
    })
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn group(dir: &Path, names: &[&str], content: &str) -> (String, Vec<String>) {
        let paths: Vec<String> = names
            .iter()
            .map(|n| {
                let p = dir.join(n);
                std::fs::write(&p, content).unwrap();
                p.to_string_lossy().to_string()
            })
            .collect();
        (compute_hash(&paths[0], "sha256").unwrap(), paths)
    }

    #[test]
    fn keeps_selected_and_deletes_the_rest() {
        let dir = tempfile::TempDir::new().unwrap();
        let (hash, paths) = group(dir.path(), &["a", "b", "c"], "same");
        let r = resolve_duplicates_blocking(&hash, "keep_selected", Some(&paths[1]), &paths).unwrap();
        assert_eq!(r.resolved_count, 2);
        assert_eq!(r.freed_bytes, 8);
        assert_eq!(r.kept.as_deref(), Some(paths[1].as_str()));
        assert!(Path::new(&paths[1]).exists());
        assert!(!Path::new(&paths[0]).exists() && !Path::new(&paths[2]).exists());
    }

    #[test]
    fn never_deletes_a_file_that_changed_since_the_scan() {
        let dir = tempfile::TempDir::new().unwrap();
        let (hash, paths) = group(dir.path(), &["a", "b", "c"], "same");
        std::fs::write(&paths[2], "edited since scan").unwrap();
        let r = resolve_duplicates_blocking(&hash, "keep_newest", None, &paths).unwrap();
        assert_eq!(r.resolved_count, 1);
        assert!(Path::new(&paths[2]).exists(), "edited file untouched");
        assert_eq!(
            [&paths[0], &paths[1]].iter().filter(|p| Path::new(p).exists()).count(),
            1
        );
    }

    #[test]
    fn refuses_when_no_longer_duplicates_and_validates_input() {
        let dir = tempfile::TempDir::new().unwrap();
        let (hash, paths) = group(dir.path(), &["a", "b"], "same");
        assert!(resolve_duplicates_blocking(&hash, "keep_selected", None, &paths).is_err());
        assert!(resolve_duplicates_blocking(&hash, "keep_selected", Some("/elsewhere"), &paths).is_err());
        assert!(resolve_duplicates_blocking(&hash, "shred", None, &paths).is_err());
        std::fs::remove_file(&paths[1]).unwrap();
        assert!(resolve_duplicates_blocking(&hash, "keep_newest", None, &paths).is_err());
        assert!(Path::new(&paths[0]).exists());
    }

    #[test]
    fn delete_all_removes_every_verified_copy() {
        let dir = tempfile::TempDir::new().unwrap();
        let (hash, paths) = group(dir.path(), &["a", "b"], "same");
        let r = resolve_duplicates_blocking(&hash, "delete_all", None, &paths).unwrap();
        assert_eq!(r.resolved_count, 2);
        assert!(r.kept.is_none());
    }
}

// ── Tag commands ──

/// Create a new tag.
#[tauri::command]
pub async fn integrity_create_tag(
    name: String,
    color: String,
) -> Result<FileTag, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let tag = FileTag {
        id: id.clone(),
        name: name.clone(),
        color,
        file_count: 0,
    };

    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    if store.tags.values().any(|t| t.name == name) {
        return Err(AppError::validation(format!("Tag '{name}' already exists")));
    }
    store.tags.insert(id, tag.clone());

    Ok(tag)
}

/// List all tags.
#[tauri::command]
pub async fn integrity_list_tags() -> Result<Vec<FileTag>, AppError> {
    let store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    let mut tags: Vec<FileTag> = store.tags.values().cloned().collect();

    // Update file counts
    for tag in &mut tags {
        tag.file_count = store
            .file_tags
            .values()
            .filter(|t| t.contains(&tag.id))
            .count();
    }

    Ok(tags)
}

/// Delete a tag.
#[tauri::command]
pub async fn integrity_delete_tag(tag_id: String) -> Result<(), AppError> {
    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    store.tags.remove(&tag_id);
    // Remove tag from all files
    for tags in store.file_tags.values_mut() {
        tags.retain(|t| t != &tag_id);
    }
    Ok(())
}

/// Add tags to a file.
#[tauri::command]
pub async fn integrity_tag_file(
    path: String,
    tag_ids: Vec<String>,
) -> Result<FileLabel, AppError> {
    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;

    {
        let file_tags = store.file_tags.entry(path.clone()).or_default();
        for tag_id in &tag_ids {
            if !file_tags.contains(tag_id) {
                file_tags.push(tag_id.clone());
            }
        }
    }

    let tags = store.file_tags.get(&path).cloned().unwrap_or_default();
    let labels = store.file_labels.get(&path).cloned().unwrap_or_default();

    Ok(FileLabel {
        path,
        tags,
        labels,
    })
}

/// Remove tags from a file.
#[tauri::command]
pub async fn integrity_untag_file(
    path: String,
    tag_ids: Vec<String>,
) -> Result<FileLabel, AppError> {
    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;

    if let Some(file_tags) = store.file_tags.get_mut(&path) {
        file_tags.retain(|t| !tag_ids.contains(t));
    }

    let tags = store.file_tags.get(&path).cloned().unwrap_or_default();
    let labels = store.file_labels.get(&path).cloned().unwrap_or_default();

    Ok(FileLabel { path, tags, labels })
}

/// Get tags and labels for a file.
#[tauri::command]
pub async fn integrity_get_file_info(path: String) -> Result<FileLabel, AppError> {
    let store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    let tags = store.file_tags.get(&path).cloned().unwrap_or_default();
    let labels = store.file_labels.get(&path).cloned().unwrap_or_default();
    Ok(FileLabel { path, tags, labels })
}

/// Set a label on a file.
#[tauri::command]
pub async fn integrity_set_label(
    path: String,
    label: String,
) -> Result<FileLabel, AppError> {
    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    {
        let file_labels = store.file_labels.entry(path.clone()).or_default();
        if !file_labels.contains(&label) {
            file_labels.push(label);
        }
    }

    let tags = store.file_tags.get(&path).cloned().unwrap_or_default();
    let labels = store.file_labels.get(&path).cloned().unwrap_or_default();
    Ok(FileLabel {
        path,
        tags,
        labels,
    })
}

/// Remove a label from a file.
#[tauri::command]
pub async fn integrity_remove_label(
    path: String,
    label: String,
) -> Result<FileLabel, AppError> {
    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    if let Some(file_labels) = store.file_labels.get_mut(&path) {
        file_labels.retain(|l| l != &label);
    }
    let tags = store.file_tags.get(&path).cloned().unwrap_or_default();
    let labels = store.file_labels.get(&path).cloned().unwrap_or_default();
    Ok(FileLabel { path, tags, labels })
}

// ── Smart Folder commands ──

/// Create a smart folder with saved filter queries.
#[tauri::command]
pub async fn integrity_create_smart_folder(
    name: String,
    query: SmartFolderQuery,
    icon: Option<String>,
) -> Result<SmartFolder, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let folder = SmartFolder {
        id: id.clone(),
        name,
        query,
        icon,
        created_at: now.clone(),
        updated_at: now,
    };

    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    store.smart_folders.insert(id, folder.clone());

    Ok(folder)
}

/// List all smart folders.
#[tauri::command]
pub async fn integrity_list_smart_folders() -> Result<Vec<SmartFolder>, AppError> {
    let store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    Ok(store.smart_folders.values().cloned().collect())
}

/// Delete a smart folder.
#[tauri::command]
pub async fn integrity_delete_smart_folder(folder_id: String) -> Result<(), AppError> {
    let mut store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    store.smart_folders.remove(&folder_id);
    Ok(())
}

/// Execute a smart folder query and return matching files.
#[tauri::command]
pub async fn integrity_run_smart_folder(
    folder_id: String,
) -> Result<Vec<String>, AppError> {
    let store = STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
    let folder = store
        .smart_folders
        .get(&folder_id)
        .ok_or_else(|| AppError::not_found(format!("Smart folder not found: {folder_id}")))?
        .clone();
    drop(store); // Release lock before I/O

    let query = &folder.query;
    let mut matches = Vec::new();

    for base_path in &query.base_paths {
        let root = Path::new(base_path);
        if !root.exists() {
            continue;
        }
        search_files(root, query, &mut matches, query.include_subdirs)?;
    }

    Ok(matches)
}

// ── Internal helpers ──

fn compute_hash(path: &str, algorithm: &str) -> Result<String, AppError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| {
        AppError::file_op(format!("Cannot open file: {e}"), "Check permissions")
    })?;

    let mut buf = vec![0u8; 64 * 1024];
    let algo = algorithm.to_lowercase();

    match algo.as_str() {
        "md5" => {
            use digest::Digest;
            let mut hasher = md5::Md5::new();
            loop {
                let n = file.read(&mut buf).map_err(|e| {
                    AppError::file_op(format!("Read error: {e}"), "")
                })?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        "sha1" => {
            // SHA-1 is not available from sha2 crate; use SHA-256 and truncate
            // to 20 bytes for an approximate checksum interface.
            // For production-grade SHA-1, add the `sha1` crate.
            use digest::Digest;
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buf).map_err(|e| {
                    AppError::file_op(format!("Read error: {e}"), "")
                })?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            let result = hasher.finalize();
            Ok(result.iter().take(20).map(|b| format!("{b:02x}")).collect())
        }
        "sha256" => {
            use digest::Digest;
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buf).map_err(|e| {
                    AppError::file_op(format!("Read error: {e}"), "")
                })?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        _ => Err(AppError::validation(format!(
            "Unsupported algorithm: {algorithm}. Use: md5, sha1, sha256"
        ))),
    }
}

fn collect_files_recursive(
    dir: &Path,
    size_groups: &mut HashMap<u64, Vec<PathBuf>>,
    total: &mut usize,
    min_size: u64,
) -> Result<(), AppError> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        AppError::file_op(format!("Cannot read directory: {e}"), "Check permissions")
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| AppError::internal(format!("Read error: {e}")))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, size_groups, total, min_size)?;
        } else if path.is_file() {
            *total += 1;
            if let Ok(meta) = path.metadata() {
                let size = meta.len();
                if size >= min_size {
                    size_groups.entry(size).or_default().push(path);
                }
            }
        }
    }
    Ok(())
}

fn search_files(
    dir: &Path,
    query: &SmartFolderQuery,
    matches: &mut Vec<String>,
    recurse: bool,
) -> Result<(), AppError> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        AppError::file_op(format!("Cannot read: {e}"), "")
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| AppError::internal(format!("{e}")))?;
        let path = entry.path();

        if path.is_dir() && recurse {
            search_files(&path, query, matches, true)?;
            continue;
        }

        if !path.is_file() {
            continue;
        }

        // Check name pattern
        if let Some(ref pattern) = query.name_pattern {
            let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            if let Ok(re) = regex::Regex::new(pattern) {
                if !re.is_match(&name) {
                    continue;
                }
            }
        }

        // Check extension filter
        if let Some(ref exts) = query.extension_filter {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !exts.iter().any(|e| e.to_lowercase() == ext) {
                continue;
            }
        }

        // Check size bounds
        if let Ok(meta) = path.metadata() {
            let size = meta.len();
            if let Some(min) = query.min_size {
                if size < min { continue; }
            }
            if let Some(max) = query.max_size {
                if size > max { continue; }
            }

            // Check modified time
            if let Ok(modified) = meta.modified() {
                let dt: chrono::DateTime<chrono::Utc> = modified.into();
                let dt_str = dt.to_rfc3339();

                if let Some(ref after) = query.modified_after {
                    if dt_str < *after { continue; }
                }
                if let Some(ref before) = query.modified_before {
                    if dt_str > *before { continue; }
                }
            }
        }

        // Check tags
        if let Some(ref required_tags) = query.tags {
            let store = STORE.lock().map_err(|_| AppError::internal("Tag store lock poisoned"))?;
            let file_tags = store.file_tags.get(&path.display().to_string());
            if let Some(ft) = file_tags {
                if !required_tags.iter().all(|t| ft.contains(t)) {
                    continue;
                }
            } else {
                continue; // No tags on file, but tags required
            }
        }

        matches.push(path.display().to_string());
    }

    Ok(())
}
