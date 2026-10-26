//! File name compatibility (the *Fix File Name Issues* and *Restore Original
//! Names* guided flows): find names a destination cannot store (Windows,
//! FAT32, macOS, OneDrive, S3…), rename them to the suggested names, and
//! rename them back later. Each rename is recorded in the compat mapping
//! table (`compat_engine::mapping`), which is what makes it reversible.

use crate::compat_engine::mapping::{create_record, MappingDb, MappingRecord};
use crate::compat_engine::profiles::{all_profiles, merge_profiles, resolve_chain, DestProfile};
use crate::compat_engine::{CompatChecker, InterventionTier};
use crate::core::error::AppError;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

/// Entries a scan reads at most.
pub const SCAN_LIMIT: usize = 50_000;
/// Problems a scan reports at most.
pub const ISSUE_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatProfileInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// A name the destination cannot store as it is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatIssue {
    pub path: String,
    pub name: String,
    /// The name it would get (the same name when no automatic fix exists).
    pub suggested_name: String,
    pub rules: Vec<String>,
    /// 1: adjusted silently, 2: visible fix, 3: needs a decision.
    pub tier: u8,
    /// Can be renamed to `suggested_name` (a fix exists and nothing has that name).
    pub fixable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatScan {
    pub issues: Vec<CompatIssue>,
    pub scanned: usize,
    /// The scan stopped at [`SCAN_LIMIT`] entries or [`ISSUE_LIMIT`] problems.
    pub truncated: bool,
}

/// A rename to apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatFix {
    pub path: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatRenameReport {
    /// `(old path, new path)`.
    pub renamed: Vec<(String, String)>,
    pub failed: Vec<CompatFailure>,
}

/// A file renamed for compatibility, as the restore flow lists it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatRenameRecord {
    pub id: String,
    pub original_name: String,
    pub translated_name: String,
    pub folder: String,
    /// The file is still there under its new name.
    pub exists: bool,
    /// Nothing else has taken the original name.
    pub restorable: bool,
    pub timestamp: String,
}

fn profile(id: &str) -> Result<DestProfile, AppError> {
    let chain = resolve_chain(id);
    if chain.is_empty() {
        return Err(AppError::validation(format!("Unknown destination type '{id}'")));
    }
    Ok(merge_profiles(&chain))
}

fn tier_number(t: InterventionTier) -> u8 {
    t as u8
}

/// A plain new name: no folder separators, not `.`/`..`, not empty.
fn valid_new_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Find the names under `root` (and `root`'s children's names) that `profile` cannot store.
pub fn scan(root: &Path, profile: &DestProfile) -> CompatScan {
    let mut out = CompatScan::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<(String, PathBuf, bool)> = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ft = e.file_type().ok();
            let is_dir = ft.map(|t| t.is_dir()).unwrap_or(false);
            names.push((name, e.path(), is_dir));
        }
        names.sort_by(|a, b| a.0.cmp(&b.0));
        let refs: Vec<&str> = names.iter().map(|n| n.0.as_str()).collect();
        let batch = CompatChecker::check_batch(&refs, profile);
        for t in batch.translations {
            let Some((_, path, _)) = names.iter().find(|n| n.0 == t.original_name) else { continue };
            let changed = t.translated_name != t.original_name;
            let taken = changed && names.iter().any(|n| n.0 == t.translated_name);
            out.issues.push(CompatIssue {
                path: path.to_string_lossy().into_owned(),
                name: t.original_name.clone(),
                suggested_name: t.translated_name.clone(),
                rules: t.rules_applied.clone(),
                tier: tier_number(t.tier),
                fixable: changed && !taken && valid_new_name(&t.translated_name),
            });
            if out.issues.len() >= ISSUE_LIMIT {
                out.truncated = true;
                return out;
            }
        }
        for (_, path, is_dir) in names {
            out.scanned += 1;
            if out.scanned >= SCAN_LIMIT {
                out.truncated = true;
                return out;
            }
            if is_dir {
                stack.push(path);
            }
        }
    }
    out.issues.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Rename `fixes` (deepest first, so a folder is renamed after what is in
/// it) and return the mapping records to store.
pub fn apply_fixes(fixes: &[CompatFix], profile_id: &str) -> (CompatRenameReport, Vec<MappingRecord>) {
    let mut report = CompatRenameReport::default();
    let mut records = Vec::new();
    let mut order: Vec<&CompatFix> = fixes.iter().collect();
    order.sort_by_key(|f| std::cmp::Reverse(Path::new(&f.path).components().count()));
    for fix in order {
        let path = Path::new(&fix.path);
        let fail = |reason: &str| CompatFailure { path: fix.path.clone(), reason: reason.into() };
        if !valid_new_name(&fix.new_name) {
            report.failed.push(fail("The new name may not be empty or contain / or \\."));
            continue;
        }
        let (Some(parent), Some(old_name)) = (path.parent(), path.file_name()) else {
            report.failed.push(fail("Not a file or folder inside a folder."));
            continue;
        };
        let old_name = old_name.to_string_lossy().into_owned();
        if old_name == fix.new_name {
            continue;
        }
        let target = parent.join(&fix.new_name);
        if std::fs::symlink_metadata(path).is_err() {
            report.failed.push(fail("It is no longer there."));
            continue;
        }
        if std::fs::symlink_metadata(&target).is_ok() {
            report.failed.push(fail("Something already has the new name."));
            continue;
        }
        match std::fs::rename(path, &target) {
            Ok(()) => {
                let parent_s = parent.to_string_lossy().into_owned();
                records.push(create_record(&old_name, &fix.new_name, &fix.path, &parent_s, profile_id, &[], 2));
                report.renamed.push((fix.path.clone(), target.to_string_lossy().into_owned()));
            }
            Err(e) => report.failed.push(fail(&format!("Could not rename: {e}"))),
        }
    }
    (report, records)
}

/// Destination types a scan can check against.
#[tauri::command]
pub async fn compat_profiles() -> Result<Vec<CompatProfileInfo>, AppError> {
    Ok(all_profiles().into_iter().map(|p| CompatProfileInfo { id: p.id, name: p.name, description: p.description }).collect())
}

/// Names under `path` that the destination type `profile_id` cannot store.
#[tauri::command]
pub async fn compat_scan_folder(path: String, profile_id: String) -> Result<CompatScan, AppError> {
    let profile = profile(&profile_id)?;
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(AppError::file_op(format!("'{path}' is not a folder"), "Choose a folder on this computer."));
    }
    tokio::task::spawn_blocking(move || scan(&root, &profile)).await.map_err(|e| AppError::internal(format!("Scan failed: {e}")))
}

/// Rename files and folders to compatible names; each rename is recorded so
/// *Restore Original Names* can undo it.
#[tauri::command]
pub async fn compat_fix_names(fixes: Vec<CompatFix>, profile_id: String, repo: State<'_, Repository>) -> Result<CompatRenameReport, AppError> {
    profile(&profile_id)?;
    let pid = profile_id.clone();
    let (report, records) = tokio::task::spawn_blocking(move || apply_fixes(&fixes, &pid)).await.map_err(|e| AppError::internal(format!("Rename failed: {e}")))?;
    if !records.is_empty() {
        MappingDb::new(repo.pool().clone()).store_batch(&records).await?;
    }
    Ok(report)
}

fn view(r: &MappingRecord) -> CompatRenameRecord {
    let folder = Path::new(&r.dest_path);
    let exists = std::fs::symlink_metadata(folder.join(&r.translated_name)).is_ok();
    let restorable = exists && std::fs::symlink_metadata(folder.join(&r.original_name)).is_err();
    CompatRenameRecord { id: r.id.clone(), original_name: r.original_name.clone(), translated_name: r.translated_name.clone(), folder: r.dest_path.clone(), exists, restorable, timestamp: r.timestamp.to_rfc3339() }
}

/// Files renamed for compatibility (newest first), optionally under `folder`.
#[tauri::command]
pub async fn compat_list_renames(folder: Option<String>, repo: State<'_, Repository>) -> Result<Vec<CompatRenameRecord>, AppError> {
    let db = MappingDb::new(repo.pool().clone());
    let records = match folder.filter(|f| !f.is_empty()) {
        Some(f) => db.list_for_dest(&f, 1000).await?,
        None => db.search("", 1000).await?,
    };
    Ok(records.iter().map(view).collect())
}

/// Give renamed files their original names back (and forget those renames).
#[tauri::command]
pub async fn compat_restore_names(ids: Vec<String>, repo: State<'_, Repository>) -> Result<CompatRenameReport, AppError> {
    let db = MappingDb::new(repo.pool().clone());
    let mut report = CompatRenameReport::default();
    for id in ids {
        let Some(r) = db.get(&id).await? else {
            report.failed.push(CompatFailure { path: id, reason: "This rename is no longer recorded.".into() });
            continue;
        };
        let folder = Path::new(&r.dest_path);
        let (from, to) = (folder.join(&r.translated_name), folder.join(&r.original_name));
        let from_s = from.to_string_lossy().into_owned();
        let fail = |reason: String| CompatFailure { path: from_s.clone(), reason };
        if std::fs::symlink_metadata(&from).is_err() {
            report.failed.push(fail("It is no longer there.".into()));
        } else if std::fs::symlink_metadata(&to).is_ok() {
            report.failed.push(fail(format!("Something is already called '{}'.", r.original_name)));
        } else if let Err(e) = std::fs::rename(&from, &to) {
            report.failed.push(fail(format!("Could not rename (this computer may not allow the original name): {e}")));
        } else {
            db.delete(&id).await?;
            report.renamed.push((from_s.clone(), to.to_string_lossy().into_owned()));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_windows_cannot_store_and_marks_what_can_be_fixed() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("sub")).unwrap();
        std::fs::write(t.path().join("ok.txt"), "").unwrap();
        std::fs::write(t.path().join("a:b.txt"), "").unwrap();
        std::fs::write(t.path().join("sub/what?.txt"), "").unwrap();
        std::fs::write(t.path().join("sub/CON"), "").unwrap();
        let p = profile("windows-ntfs").unwrap();
        let s = scan(t.path(), &p);
        let names: Vec<&str> = s.issues.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"a:b.txt") && names.contains(&"what?.txt") && names.contains(&"CON"), "{names:?}");
        assert!(!names.contains(&"ok.txt"));
        let colon = s.issues.iter().find(|i| i.name == "a:b.txt").unwrap();
        assert!(colon.fixable && colon.suggested_name != "a:b.txt" && !colon.suggested_name.contains(':'));
        assert!(s.scanned >= 4 && !s.truncated);
    }

    #[test]
    fn a_suggested_name_that_is_taken_is_not_fixable() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a:b.txt"), "").unwrap();
        let p = profile("windows-ntfs").unwrap();
        let suggested = scan(t.path(), &p).issues[0].suggested_name.clone();
        std::fs::write(t.path().join(&suggested), "").unwrap();
        let issue = scan(t.path(), &p).issues.into_iter().find(|i| i.name == "a:b.txt").unwrap();
        assert!(!issue.fixable);
    }

    #[test]
    fn fixes_rename_inside_first_and_refuse_bad_or_taken_names() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("bad:dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x?.txt"), "1").unwrap();
        std::fs::write(t.path().join("taken.txt"), "").unwrap();
        std::fs::write(t.path().join("y.txt"), "").unwrap();
        let fixes = vec![
            CompatFix { path: dir.to_string_lossy().into(), new_name: "bad_dir".into() },
            CompatFix { path: dir.join("x?.txt").to_string_lossy().into(), new_name: "x_.txt".into() },
            CompatFix { path: t.path().join("y.txt").to_string_lossy().into(), new_name: "taken.txt".into() },
            CompatFix { path: t.path().join("y.txt").to_string_lossy().into(), new_name: "../escape.txt".into() },
            CompatFix { path: t.path().join("gone.txt").to_string_lossy().into(), new_name: "g.txt".into() },
        ];
        let (report, records) = apply_fixes(&fixes, "windows-ntfs");
        assert_eq!(report.renamed.len(), 2);
        assert!(t.path().join("bad_dir/x_.txt").exists());
        assert_eq!(report.failed.len(), 3);
        assert!(t.path().join("y.txt").exists() && !t.path().join("../escape.txt").exists());
        // The folder's record names its parent; the file's names the folder before it was renamed.
        assert_eq!(records.iter().map(|r| (r.original_name.as_str(), r.translated_name.as_str())).collect::<Vec<_>>(), [("x?.txt", "x_.txt"), ("bad:dir", "bad_dir")]);
    }

    #[tokio::test]
    async fn renames_are_recorded_and_can_be_undone() {
        let repo = Repository::open_in_memory().await.unwrap();
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a:b.txt"), "data").unwrap();
        let fixes = vec![CompatFix { path: t.path().join("a:b.txt").to_string_lossy().into(), new_name: "a_b.txt".into() }];
        let (report, records) = apply_fixes(&fixes, "windows-ntfs");
        assert_eq!(report.renamed.len(), 1);
        let db = MappingDb::new(repo.pool().clone());
        db.store_batch(&records).await.unwrap();
        let listed: Vec<CompatRenameRecord> = db.search("", 10).await.unwrap().iter().map(view).collect();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].exists && listed[0].restorable);
        // Undo through the same steps the command takes.
        let r = db.get(&listed[0].id).await.unwrap().unwrap();
        std::fs::rename(Path::new(&r.dest_path).join(&r.translated_name), Path::new(&r.dest_path).join(&r.original_name)).unwrap();
        assert!(db.delete(&r.id).await.unwrap());
        assert_eq!(std::fs::read_to_string(t.path().join("a:b.txt")).unwrap(), "data");
        assert!(db.get(&r.id).await.unwrap().is_none());
    }

    #[test]
    fn unknown_destination_types_are_refused() {
        assert!(profile("nope").is_err());
        assert!(valid_new_name("a.txt") && !valid_new_name("a/b") && !valid_new_name("..") && !valid_new_name(""));
    }
}
