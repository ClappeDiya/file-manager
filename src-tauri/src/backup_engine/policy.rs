//! Process-wide module allowlists: which destination backends,
//! encryption modules and compression modules jobs may use.
//!
//! Configured by `--allowed-backends` / `--allowed-encryption` /
//! `--allowed-compression` on the server, or the matching
//! `UFOP_ALLOWED_*` environment variables. `UFOP_DISABLE_MODULES`
//! (comma-separated) removes modules from whatever is otherwise allowed —
//! the switch for keeping optional modules from loading at all.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};

pub const BACKENDS: &[&str] = &["local", "sftp", "ftp", "ftps", "webdav", "s3"];
pub const ENCRYPTION: &[&str] = &["aes", "none"];
pub const COMPRESSION: &[&str] = &["deflate", "none"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulePolicy {
    /// `None` = all allowed.
    pub backends: Option<Vec<String>>,
    pub encryption: Option<Vec<String>>,
    pub compression: Option<Vec<String>>,
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub kind: String,
    pub name: String,
    pub enabled: bool,
}

fn list(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
}

impl ModulePolicy {
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).map(|v| list(&v));
        Self {
            backends: get("UFOP_ALLOWED_BACKENDS"),
            encryption: get("UFOP_ALLOWED_ENCRYPTION"),
            compression: get("UFOP_ALLOWED_COMPRESSION"),
            disabled: get("UFOP_DISABLE_MODULES").unwrap_or_default(),
        }
    }

    pub fn from_lists(backends: Option<&str>, encryption: Option<&str>, compression: Option<&str>, disabled: Option<&str>) -> Self {
        let env = Self::from_env();
        Self {
            backends: backends.map(list).or(env.backends),
            encryption: encryption.map(list).or(env.encryption),
            compression: compression.map(list).or(env.compression),
            disabled: disabled.map(list).unwrap_or(env.disabled),
        }
    }

    fn allowed(&self, allow: &Option<Vec<String>>, name: &str) -> bool {
        !self.disabled.iter().any(|d| d == name) && allow.as_ref().map_or(true, |a| a.iter().any(|x| x == name))
    }

    pub fn modules(&self) -> Vec<ModuleInfo> {
        let mut v = Vec::new();
        for (kind, names, allow) in [("backend", BACKENDS, &self.backends), ("encryption", ENCRYPTION, &self.encryption), ("compression", COMPRESSION, &self.compression)] {
            for n in names {
                v.push(ModuleInfo { kind: kind.into(), name: n.to_string(), enabled: self.allowed(allow, n) });
            }
        }
        v
    }

    pub fn check_job(&self, backend: &str, encrypted: bool, compressed: bool) -> Result<(), AppError> {
        let deny = |kind: &str, name: &str| AppError::PermissionDenied {
            message: format!("The {kind} module '{name}' is not allowed on this machine"),
            advice: "Ask your administrator, or choose another option allowed by the module policy.".into(),
        };
        if !backend.is_empty() && !self.allowed(&self.backends, backend) {
            return Err(deny("destination", backend));
        }
        let enc = if encrypted { "aes" } else { "none" };
        if !self.allowed(&self.encryption, enc) {
            return Err(deny("encryption", enc));
        }
        let comp = if compressed { "deflate" } else { "none" };
        if !self.allowed(&self.compression, comp) {
            return Err(deny("compression", comp));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_and_disable() {
        let p = ModulePolicy::from_lists(Some("local,s3"), Some("aes"), None, Some("s3"));
        assert!(p.check_job("local", true, true).is_ok());
        assert!(p.check_job("s3", true, true).is_err(), "disabled wins over allow");
        assert!(p.check_job("sftp", true, true).is_err());
        assert!(p.check_job("local", false, true).is_err(), "unencrypted not allowed");
        assert!(ModulePolicy::default().check_job("ftp", false, false).is_ok());
        assert_eq!(p.modules().iter().filter(|m| m.enabled).count(), 1 + 1 + 2);
    }
}
