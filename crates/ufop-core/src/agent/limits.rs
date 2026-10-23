//! Limits the machine's owner puts on what the management console may do
//! here. They are local settings: the console can neither see nor change
//! them through the agent protocol.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// How paths that pass through symbolic links are judged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkPolicy {
    /// Resolve links: a path is allowed only where it really points inside a root.
    #[default]
    StayInside,
    /// Judge the path as written (links may lead outside the roots).
    Follow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLimits {
    /// Folders the console may use for sources, local destinations and
    /// restore targets; empty = anywhere.
    pub roots: Vec<String>,
    /// The console may view and run existing jobs, but not change jobs or
    /// settings, restore files or delete backup data.
    pub read_only: bool,
    pub symlinks: SymlinkPolicy,
    /// Jobs from the console may carry pre/post scripts (commands run here).
    pub allow_scripts: bool,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self { roots: Vec::new(), read_only: false, symlinks: SymlinkPolicy::StayInside, allow_scripts: true }
    }
}

/// `a/./b/../c` → `a/c`; `None` for relative paths or `..` above the root.
pub fn normalize(p: &Path) -> Option<PathBuf> {
    if !p.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// Resolve symlinks in the longest existing ancestor; the rest is appended.
fn resolve(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut rest = Vec::new();
    loop {
        if let Ok(c) = std::fs::canonicalize(&existing) {
            let mut out = c;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.file_name().map(|n| n.to_os_string()), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

fn inside(path: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let lower = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        let (p, r) = (lower(path), lower(root));
        let r = r.trim_end_matches('\\');
        p == r || p.starts_with(&format!("{r}\\"))
    } else {
        path.starts_with(root)
    }
}

impl AgentLimits {
    pub fn validate(&self) -> Result<(), String> {
        for r in &self.roots {
            if normalize(Path::new(r)).is_none() {
                return Err(format!("'{r}' is not an absolute folder"));
            }
        }
        Ok(())
    }

    /// Whether the console may use `path` (always, when no roots are set).
    pub fn check_path(&self, path: &str) -> Result<(), AppError> {
        if self.roots.is_empty() {
            return Ok(());
        }
        let denied = || AppError::PermissionDenied {
            message: format!("'{path}' is outside the folders this machine allows the console to use"),
            advice: "Change the allowed folders in Settings → Managed by Console on the machine.".into(),
        };
        let p = normalize(Path::new(path.trim())).ok_or_else(denied)?;
        let follow = self.symlinks == SymlinkPolicy::Follow;
        let p = if follow { p } else { resolve(&p) };
        let ok = self.roots.iter().filter_map(|r| normalize(Path::new(r))).any(|r| inside(&p, &if follow { r } else { resolve(&r) }));
        if ok {
            Ok(())
        } else {
            Err(denied())
        }
    }

    pub fn check_write(&self, what: &str) -> Result<(), AppError> {
        if self.read_only {
            return Err(AppError::PermissionDenied {
                message: format!("This machine is read-only for the console: {what} is not allowed"),
                advice: "Turn off read-only in Settings → Managed by Console on the machine.".into(),
            });
        }
        Ok(())
    }

    pub fn check_scripts(&self, has_scripts: bool) -> Result<(), AppError> {
        if has_scripts && !self.allow_scripts {
            return Err(AppError::PermissionDenied {
                message: "This machine does not run scripts from the console".into(),
                advice: "Remove the pre/post scripts from the job, or allow console scripts on the machine.".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(roots: &[&Path], symlinks: SymlinkPolicy) -> AgentLimits {
        AgentLimits { roots: roots.iter().map(|r| r.to_string_lossy().to_string()).collect(), symlinks, ..Default::default() }
    }

    #[test]
    fn no_roots_allow_everything_and_defaults_are_permissive() {
        let l = AgentLimits::default();
        assert!(l.check_path("/etc/shadow").is_ok());
        assert!(l.check_write("saving a job").is_ok());
        assert!(l.check_scripts(true).is_ok());
    }

    #[test]
    fn paths_must_be_absolute_and_inside_after_normalizing() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("data");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let l = limits(&[&root], SymlinkPolicy::StayInside);
        assert!(l.check_path(&root.join("docs").to_string_lossy()).is_ok());
        assert!(l.check_path(&root.join("new/not/yet").to_string_lossy()).is_ok(), "targets that do not exist yet");
        assert!(l.check_path(&root.to_string_lossy()).is_ok());
        assert!(l.check_path(&format!("{}/docs/../../escape", root.display())).is_err());
        assert!(l.check_path(&format!("{}-sibling", root.display())).is_err(), "prefix of a name is not inside");
        assert!(l.check_path("relative/path").is_err());
        assert!(l.check_path("").is_err());
        let msg = l.check_path("/elsewhere").unwrap_err().user_message();
        assert!(msg.contains("outside the folders"), "{msg}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_a_root_are_refused_unless_followed() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("data");
        let outside = d.path().join("secret");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let via_link = root.join("link").join("file").to_string_lossy().to_string();
        assert!(limits(&[&root], SymlinkPolicy::StayInside).check_path(&via_link).is_err());
        assert!(limits(&[&root], SymlinkPolicy::Follow).check_path(&via_link).is_ok());
        // A root that is itself a link is judged by where it points.
        let root_link = d.path().join("root-link");
        std::os::unix::fs::symlink(&root, &root_link).unwrap();
        assert!(limits(&[&root_link], SymlinkPolicy::StayInside).check_path(&root.join("x").to_string_lossy()).is_ok());
    }

    #[test]
    fn read_only_scripts_and_validation() {
        let l = AgentLimits { read_only: true, allow_scripts: false, ..Default::default() };
        assert!(l.check_write("restoring files").unwrap_err().user_message().contains("read-only"));
        assert!(l.check_scripts(false).is_ok());
        assert!(l.check_scripts(true).is_err());
        assert!(AgentLimits { roots: vec!["relative".into()], ..Default::default() }.validate().is_err());
        let parsed: AgentLimits = serde_json::from_str(r#"{"roots":["/data"]}"#).unwrap();
        assert!(parsed.allow_scripts && parsed.symlinks == SymlinkPolicy::StayInside);
    }
}
