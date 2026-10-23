//! Mirror restore: after restoring into a target folder, remove what the
//! target holds below the restored folders but the selected version does
//! not, so the folders end up exactly as they were backed up.
//!
//! Safety rules:
//! - Only folders that the restore itself recreates (folder entries of the
//!   selection) are scopes. A selected single file never deletes its siblings.
//! - Every path to delete is inside the target folder; anything else is
//!   refused. The target folder itself is never deleted.
//! - Symbolic links are deleted as links; nothing is followed.
//! - On Windows and macOS names are compared without case, so a file whose
//!   name differs only in case from the backed-up one is kept.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One path the mirror removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extra {
    pub path: PathBuf,
    pub is_dir: bool,
}

fn key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s }
}

/// Paths below `scopes` (restored folders) that are neither `expected` nor
/// an ancestor of an expected path, deepest first (the order to delete in).
pub fn plan(target: &Path, scopes: &[PathBuf], expected: &[PathBuf]) -> Vec<Extra> {
    let mut keep: HashSet<String> = HashSet::new();
    for p in expected {
        let mut cur = Some(p.as_path());
        while let Some(c) = cur {
            if !keep.insert(key(c)) || c == target {
                break;
            }
            cur = c.parent();
        }
    }
    let mut roots: Vec<&PathBuf> = scopes.iter().filter(|s| s.starts_with(target)).collect();
    roots.sort();
    roots.dedup();
    // Nested scopes are covered by their parent's walk.
    let mut top: Vec<&PathBuf> = Vec::new();
    for r in roots {
        if !top.iter().any(|t| r.starts_with(t)) {
            top.push(r);
        }
    }
    let mut out = Vec::new();
    for r in top {
        walk(r, &keep, false, &mut out);
    }
    out.retain(|e| e.path.starts_with(target) && e.path != target);
    out.sort_by_key(|e| std::cmp::Reverse(e.path.components().count()));
    out
}

/// Collect extras below `dir`. `all`: the whole subtree is extraneous.
fn walk(dir: &Path, keep: &HashSet<String>, all: bool, out: &mut Vec<Extra>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        let real_dir = md.is_dir();
        let extra = all || !keep.contains(&key(&p));
        if real_dir {
            walk(&p, keep, extra, out);
        }
        if extra {
            out.push(Extra { path: p, is_dir: real_dir });
        }
    }
}

/// Delete planned extras (deepest first). Returns (files, folders) removed
/// and a message per failure.
pub fn delete(target: &Path, extras: &[Extra]) -> (u64, u64, Vec<String>) {
    let (mut files, mut dirs, mut errors) = (0, 0, Vec::new());
    for e in extras {
        if !e.path.starts_with(target) || e.path == target {
            errors.push(format!("{}: outside the restore target, not deleted", e.path.display()));
            continue;
        }
        let r = if e.is_dir { std::fs::remove_dir(&e.path) } else { std::fs::remove_file(&e.path) };
        match r {
            Ok(()) if e.is_dir => dirs += 1,
            Ok(()) => files += 1,
            Err(err) => errors.push(format!("{}: not deleted ({err})", e.path.display())),
        }
    }
    (files, dirs, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::test_util::write;

    #[test]
    fn plans_only_extras_inside_restored_folders() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("target");
        write(&t, "docs/keep.txt", b"k");
        write(&t, "docs/old.txt", b"o");
        write(&t, "docs/junk/a.bin", b"a");
        write(&t, "docs/junk/deep/b.bin", b"b");
        write(&t, "docs/sub/keep2.txt", b"k");
        write(&t, "elsewhere/untouched.txt", b"u"); // not a restored folder
        let expected = vec![t.join("docs"), t.join("docs/keep.txt"), t.join("docs/sub"), t.join("docs/sub/keep2.txt")];
        let plan = plan(&t, &[t.join("docs"), t.join("docs/sub")], &expected);
        let rel: Vec<String> = plan.iter().map(|e| e.path.strip_prefix(&t).unwrap().to_string_lossy().replace('\\', "/")).collect();
        let mut sorted = rel.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["docs/junk", "docs/junk/a.bin", "docs/junk/deep", "docs/junk/deep/b.bin", "docs/old.txt"]);
        // Deepest first, so folders are empty when their turn comes.
        assert!(rel.iter().position(|p| p == "docs/junk/deep/b.bin").unwrap() < rel.iter().position(|p| p == "docs/junk/deep").unwrap());
        let (files, dirs, errors) = delete(&t, &plan);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!((files, dirs), (3, 2));
        assert!(t.join("docs/keep.txt").exists() && t.join("docs/sub/keep2.txt").exists() && t.join("elsewhere/untouched.txt").exists());
        assert!(!t.join("docs/junk").exists() && !t.join("docs/old.txt").exists());
    }

    #[test]
    fn never_outside_the_target() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("target");
        write(d.path(), "outside/x.txt", b"x");
        std::fs::create_dir_all(&t).unwrap();
        // A scope outside the target is ignored.
        assert!(plan(&t, &[d.path().join("outside")], &[]).is_empty());
        // Hand-made extras outside the target are refused.
        let (f, _, errors) = delete(&t, &[Extra { path: d.path().join("outside/x.txt"), is_dir: false }, Extra { path: t.clone(), is_dir: true }]);
        assert_eq!(f, 0);
        assert_eq!(errors.len(), 2);
        assert!(d.path().join("outside/x.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_removed_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("target");
        write(d.path(), "precious/data.txt", b"p");
        std::fs::create_dir_all(t.join("docs")).unwrap();
        std::os::unix::fs::symlink(d.path().join("precious"), t.join("docs/link")).unwrap();
        let plan = plan(&t, &[t.join("docs")], &[t.join("docs")]);
        assert_eq!(plan, vec![Extra { path: t.join("docs/link"), is_dir: false }]);
        delete(&t, &plan);
        assert!(d.path().join("precious/data.txt").exists());
    }
}
