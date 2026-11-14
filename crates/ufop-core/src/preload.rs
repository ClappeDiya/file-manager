//! Machine-wide preload configuration: per-executable default arguments,
//! environment values and database settings from a trusted JSON file.
//!
//! Location: `$UFOP_PRELOAD_SETTINGS`, else `/etc/ufop/preload.json`
//! (Linux), `/Library/Application Support/ufop/preload.json` (macOS) or
//! `%PROGRAMDATA%\ufop\preload.json` (Windows).
//!
//! ```json
//! { "env":  { "*": { "UFOP_LOG_LEVEL": "info" }, "ufop-server": { "TZ": "Europe/Berlin" } },
//!   "args": { "*": [], "ufop-server": ["--port=8300"], "ufop": ["--json"] },
//!   "db":   { "ufop-server": { "path": "/var/lib/ufop/app.sqlite" } } }
//! ```
//! Preload arguments are inserted *before* the user's arguments, so
//! explicit command-line values win. Environment values never override a
//! variable that is already set. On Unix the file is ignored when other
//! users can write it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const ENV_PATH: &str = "UFOP_PRELOAD_SETTINGS";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DbSettings {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preload {
    pub env: BTreeMap<String, BTreeMap<String, String>>,
    pub args: BTreeMap<String, Vec<String>>,
    pub db: BTreeMap<String, DbSettings>,
}

pub fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_PATH) {
        return PathBuf::from(p);
    }
    if cfg!(windows) {
        PathBuf::from(std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".into())).join("ufop").join("preload.json")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ufop/preload.json")
    } else {
        PathBuf::from("/etc/ufop/preload.json")
    }
}

/// Whether the file may be trusted (Unix: not group/world writable).
pub fn is_trusted(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return std::fs::metadata(path).map(|m| m.permissions().mode() & 0o022 == 0).unwrap_or(false);
    }
    #[allow(unreachable_code)]
    path.exists()
}

impl Preload {
    /// Load from the default location; `None` when absent or untrusted.
    pub fn load() -> Option<Self> {
        Self::load_from(&default_path())
    }

    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        if !path.exists() {
            return None;
        }
        if !is_trusted(path) {
            tracing::warn!("Ignoring preload file {}: writable by other users", path.display());
            return None;
        }
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|d| serde_json::from_slice(&d).map_err(|e| e.to_string())) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!("Ignoring preload file {}: {e}", path.display());
                None
            }
        }
    }

    /// Arguments for `exe` (generic `*` first, then executable-specific).
    pub fn args_for(&self, exe: &str) -> Vec<String> {
        let mut v = self.args.get("*").cloned().unwrap_or_default();
        v.extend(self.args.get(exe).cloned().unwrap_or_default());
        v
    }

    /// Environment values for `exe` that are not already set.
    pub fn env_for(&self, exe: &str) -> BTreeMap<String, String> {
        let mut m = self.env.get("*").cloned().unwrap_or_default();
        m.extend(self.env.get(exe).cloned().unwrap_or_default());
        m.retain(|k, _| std::env::var_os(k).is_none());
        m
    }

    /// Apply environment values to this process. Call early in `main`,
    /// before threads are spawned.
    pub fn apply_env(&self, exe: &str) {
        for (k, v) in self.env_for(exe) {
            // SAFETY: called at startup before other threads exist.
            unsafe { std::env::set_var(k, v) };
        }
    }

    /// Program argv with preload arguments inserted after argv[0].
    pub fn merge_args(&self, exe: &str, argv: Vec<String>) -> Vec<String> {
        let mut it = argv.into_iter();
        let mut out: Vec<String> = it.next().into_iter().collect();
        out.extend(self.args_for(exe));
        out.extend(it);
        out
    }

    pub fn db_path_for(&self, exe: &str) -> Option<String> {
        self.db.get(exe).and_then(|d| d.path.clone()).or_else(|| self.db.get("*").and_then(|d| d.path.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_and_env_precedence() {
        let p: Preload = serde_json::from_str(
            r#"{"args":{"*":["--a"],"ufop-server":["--port=1"]},"env":{"*":{"UFOP_TEST_PRELOAD_X":"1","PATH":"nope"}},"db":{"*":{"path":"/x.db"}}}"#,
        )
        .unwrap();
        assert_eq!(p.merge_args("ufop-server", vec!["bin".into(), "--port=2".into()]), vec!["bin", "--a", "--port=1", "--port=2"]);
        let env = p.env_for("ufop");
        assert!(env.contains_key("UFOP_TEST_PRELOAD_X"));
        assert!(!env.contains_key("PATH"), "existing variables are never overridden");
        assert_eq!(p.db_path_for("ufop-server").as_deref(), Some("/x.db"));
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_file_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("preload.json");
        std::fs::write(&f, "{}").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Preload::load_from(&f).is_some());
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(Preload::load_from(&f).is_none());
    }
}
