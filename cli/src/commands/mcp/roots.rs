//! Root confinement for `ufop mcp`: every path a client names must resolve —
//! after following symlinks — inside one of the configured roots.
//!
//! Paths are absolute (inside a root) or relative to the first root, or
//! written `<root name>:relative/path`. Existing paths are canonicalized, so a
//! symlink that points outside is refused. A path that does not exist yet
//! (for writes) is resolved from its nearest existing ancestor, and `..` in
//! the part that does not exist is refused.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Root {
    pub name: String,
    /// Canonical path.
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Roots(Vec<Root>);

impl Roots {
    /// Canonicalize the given directories; names are their last component, made unique.
    pub fn new(dirs: &[PathBuf]) -> Result<Self, String> {
        if dirs.is_empty() {
            return Err("give at least one --root directory".into());
        }
        let mut roots: Vec<Root> = Vec::new();
        for d in dirs {
            let path = d.canonicalize().map_err(|e| format!("root {}: {e}", d.display()))?;
            if !path.is_dir() {
                return Err(format!("root {} is not a directory", d.display()));
            }
            let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "root".into());
            let mut name = base.clone();
            let mut i = 2;
            while roots.iter().any(|r| r.name == name) {
                name = format!("{base}-{i}");
                i += 1;
            }
            roots.push(Root { name, path });
        }
        Ok(Self(roots))
    }

    pub fn list(&self) -> &[Root] {
        &self.0
    }

    fn root_of(&self, p: &Path) -> Option<&Root> {
        self.0.iter().find(|r| p.starts_with(&r.path))
    }

    /// Resolve a client path to a canonical path inside a root.
    /// `must_exist: false` allows a path whose last components do not exist yet.
    pub fn resolve(&self, input: &str, must_exist: bool) -> Result<PathBuf, String> {
        if input.contains('\0') {
            return Err("path contains a NUL byte".into());
        }
        let joined = self.join(input)?;
        let resolved = if must_exist { joined.canonicalize().map_err(|e| format!("{input}: {e}"))? } else { canonical_prefix(&joined).map_err(|e| format!("{input}: {e}"))? };
        if self.root_of(&resolved).is_none() {
            return Err(format!("{input}: outside the allowed roots"));
        }
        Ok(resolved)
    }

    /// Like `resolve`, for a path that will be created or replaced: the path
    /// itself must not be a root, and an existing symlink at it is refused
    /// (writing would follow it).
    pub fn resolve_target(&self, input: &str) -> Result<PathBuf, String> {
        let p = self.resolve(input, false)?;
        if self.0.iter().any(|r| r.path == p) {
            return Err(format!("{input}: is a root"));
        }
        let joined = self.join(input)?;
        if std::fs::symlink_metadata(&joined).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Err(format!("{input}: is a symbolic link"));
        }
        Ok(p)
    }

    /// The path as the client sees it: `<root name>:relative`.
    pub fn display(&self, p: &Path) -> String {
        match self.root_of(p) {
            Some(r) => {
                let rel = p.strip_prefix(&r.path).unwrap_or(p).to_string_lossy().replace('\\', "/");
                format!("{}:{rel}", r.name)
            }
            None => p.display().to_string(),
        }
    }

    fn join(&self, input: &str) -> Result<PathBuf, String> {
        let input = input.trim();
        if let Some((name, rel)) = input.split_once(':').filter(|(n, _)| self.0.iter().any(|r| r.name == *n)) {
            let r = self.0.iter().find(|r| r.name == name).ok_or("unknown root")?;
            let rel = Path::new(rel.trim_start_matches(['/', '\\']));
            return Ok(r.path.join(rel));
        }
        let p = Path::new(input);
        if p.is_absolute() {
            Ok(p.to_path_buf())
        } else {
            Ok(self.0[0].path.join(p))
        }
    }
}

/// Canonicalize the longest existing prefix of `p`, then append the rest
/// (which must be plain names: no `..`, `.`, or prefixes).
fn canonical_prefix(p: &Path) -> std::io::Result<PathBuf> {
    let mut existing = p.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while std::fs::symlink_metadata(&existing).is_err() {
        let name = existing.file_name().map(|n| n.to_os_string());
        let comp = existing.components().next_back();
        match (name, comp) {
            (Some(n), Some(Component::Normal(_))) => {
                rest.push(n);
                if !existing.pop() {
                    break;
                }
            }
            _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "'..' or '.' in a path that does not exist")),
        }
    }
    let mut out = existing.canonicalize()?;
    for n in rest.into_iter().rev() {
        out.push(n);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confines_paths_to_roots() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("data");
        let outside = d.path().join("secret");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("key"), "x").unwrap();
        std::fs::write(root.join("sub/a.txt"), "a").unwrap();
        let roots = Roots::new(std::slice::from_ref(&root)).unwrap();
        let canon = root.canonicalize().unwrap();

        assert_eq!(roots.resolve("sub/a.txt", true).unwrap(), canon.join("sub/a.txt"));
        assert_eq!(roots.resolve("data:/sub/a.txt", true).unwrap(), canon.join("sub/a.txt"));
        assert_eq!(roots.resolve(&canon.join("sub").to_string_lossy(), true).unwrap(), canon.join("sub"));
        assert_eq!(roots.display(&canon.join("sub/a.txt")), "data:sub/a.txt");
        assert!(roots.resolve("../secret/key", true).unwrap_err().contains("outside"));
        assert!(roots.resolve(&outside.join("key").to_string_lossy(), true).unwrap_err().contains("outside"));
        assert!(roots.resolve("sub/../../secret/key", false).is_err());
        assert!(roots.resolve("new/../../secret/x", false).is_err(), "'..' in a missing part");
        assert_eq!(roots.resolve("new/dir/file", false).unwrap(), canon.join("new/dir/file"));
        assert!(roots.resolve_target("data:").unwrap_err().contains("is a root"));
        assert!(roots.resolve("a\0b", false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("r");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(d.path().join("passwd"), "x").unwrap();
        std::os::unix::fs::symlink(d.path(), root.join("up")).unwrap();
        std::os::unix::fs::symlink(d.path().join("passwd"), root.join("link")).unwrap();
        std::fs::write(root.join("inside"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("inside"), root.join("ok-link")).unwrap();
        let roots = Roots::new(&[root]).unwrap();
        assert!(roots.resolve("up/passwd", true).unwrap_err().contains("outside"));
        assert!(roots.resolve("link", true).unwrap_err().contains("outside"));
        assert!(roots.resolve("up/newfile", false).unwrap_err().contains("outside"));
        assert!(roots.resolve("ok-link", true).is_ok(), "links inside the root are followed");
        assert!(roots.resolve_target("ok-link").unwrap_err().contains("symbolic link"));
    }

    #[test]
    fn root_names_are_unique() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("x/docs"), d.path().join("y/docs"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let r = Roots::new(&[a, b]).unwrap();
        assert_eq!(r.list().iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["docs", "docs-2"]);
        assert!(Roots::new(&[]).is_err());
    }
}
