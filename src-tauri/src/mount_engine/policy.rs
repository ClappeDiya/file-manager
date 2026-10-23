//! Mount options (cache, read-only) and the organization's mount profile
//! (`ufop_core::agent::ManagedMounts`) that constrains them.
//!
//! The profile wins: a forced cache mode or read-only replaces what the user
//! chose, size and age limits cap it, protocols outside the allowed list and
//! mount points outside the mount root are refused. `effective` returns the
//! options to use and notes on what the profile changed.

use super::expand_home;
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ufop_core::agent::ManagedMounts;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// Read and write straight through; nothing is kept.
    Off,
    /// Only files open for read and write are cached.
    Minimal,
    /// Writes are cached and uploaded in the background (the default).
    #[default]
    Writes,
    /// Whole files are cached on read and write: they stay readable offline.
    Full,
}

impl CacheMode {
    pub fn as_rclone(self) -> &'static str {
        match self {
            CacheMode::Off => "off",
            CacheMode::Minimal => "minimal",
            CacheMode::Writes => "writes",
            CacheMode::Full => "full",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => CacheMode::Off,
            "minimal" => CacheMode::Minimal,
            "writes" => CacheMode::Writes,
            "full" => CacheMode::Full,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct MountOptions {
    pub cache_mode: CacheMode,
    /// Largest cache, in MB (`None` = rclone's default: unlimited).
    pub max_cache_mb: Option<u64>,
    /// Cached files older than this are dropped (`None` = rclone's default, 1 hour).
    pub max_cache_age_hours: Option<u32>,
    pub read_only: bool,
}

/// The options to mount with under `policy`, and what the policy changed.
pub fn effective(requested: &MountOptions, protocol: &str, mount_point: &Path, policy: Option<&ManagedMounts>) -> Result<(MountOptions, Vec<String>), AppError> {
    let mut o = requested.clone();
    let mut notes = Vec::new();
    let Some(p) = policy else { return Ok((o, notes)) };
    if let Some(allowed) = &p.allowed_protocols {
        if !allowed.iter().any(|a| a.eq_ignore_ascii_case(protocol)) {
            return Err(AppError::PermissionDenied {
                message: format!("Your organization does not allow mounting {protocol} connections"),
                advice: format!("Allowed: {}. Use sync or transfer instead.", if allowed.is_empty() { "none".to_string() } else { allowed.join(", ") }),
            });
        }
    }
    if let Some(root) = p.mount_root.as_deref() {
        let root = PathBuf::from(expand_home(root));
        let root = root.canonicalize().unwrap_or(root);
        if !mount_point.starts_with(&root) {
            return Err(AppError::PermissionDenied {
                message: format!("Mount points must be inside {}", root.display()),
                advice: "Choose a folder there (your organization's mount profile).".into(),
            });
        }
    }
    if p.read_only == Some(true) && !o.read_only {
        o.read_only = true;
        notes.push("read-only (organization policy)".into());
    }
    if let Some(mode) = p.cache_mode.as_deref().and_then(CacheMode::parse) {
        if mode != o.cache_mode {
            o.cache_mode = mode;
            notes.push(format!("cache mode {} (organization policy)", mode.as_rclone()));
        }
    }
    if let Some(max) = p.max_cache_mb {
        if o.max_cache_mb.map_or(true, |m| m > max) {
            o.max_cache_mb = Some(max);
            notes.push(format!("cache up to {max} MB (organization policy)"));
        }
    }
    if let Some(max) = p.max_cache_age_hours {
        if o.max_cache_age_hours.map_or(true, |h| h > max) {
            o.max_cache_age_hours = Some(max);
            notes.push(format!("cached files kept up to {max} h (organization policy)"));
        }
    }
    Ok((o, notes))
}

/// `rclone mount` flags for `o`, with the cache in `cache_dir`.
pub fn rclone_flags(o: &MountOptions, cache_dir: &Path) -> Vec<String> {
    let mut v = vec!["--vfs-cache-mode".into(), o.cache_mode.as_rclone().into(), "--cache-dir".into(), cache_dir.to_string_lossy().into_owned()];
    if let Some(mb) = o.max_cache_mb {
        v.extend(["--vfs-cache-max-size".into(), format!("{mb}M")]);
    }
    if let Some(h) = o.max_cache_age_hours {
        v.extend(["--vfs-cache-max-age".into(), format!("{h}h")]);
    }
    if o.read_only {
        v.push("--read-only".into());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_policy_keeps_the_choice() {
        let o = MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(10_000), ..Default::default() };
        assert_eq!(effective(&o, "rclone", Path::new("/home/u/x"), None).unwrap(), (o, vec![]));
    }

    #[test]
    fn policy_forces_caps_and_refuses() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("Drives");
        std::fs::create_dir_all(&root).unwrap();
        let p = ManagedMounts {
            allowed_protocols: Some(vec!["rclone".into(), "s3".into()]),
            read_only: Some(true),
            cache_mode: Some("full".into()),
            max_cache_mb: Some(2048),
            max_cache_age_hours: Some(24),
            mount_root: Some(root.to_string_lossy().into_owned()),
        };
        let asked = MountOptions { cache_mode: CacheMode::Off, max_cache_mb: Some(50_000), max_cache_age_hours: Some(2), read_only: false };
        let (o, notes) = effective(&asked, "S3", &root.canonicalize().unwrap().join("bucket"), Some(&p)).unwrap();
        assert_eq!(o, MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(2048), max_cache_age_hours: Some(2), read_only: true });
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(effective(&asked, "smb", &root.join("x"), Some(&p)).unwrap_err().to_string().contains("does not allow mounting smb"));
        assert!(effective(&asked, "rclone", Path::new("/tmp/elsewhere"), Some(&p)).unwrap_err().to_string().contains("must be inside"));
    }

    #[test]
    fn flags() {
        let o = MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(512), max_cache_age_hours: Some(48), read_only: true };
        assert_eq!(
            rclone_flags(&o, Path::new("/c/m1")).join(" "),
            "--vfs-cache-mode full --cache-dir /c/m1 --vfs-cache-max-size 512M --vfs-cache-max-age 48h --read-only"
        );
        assert_eq!(rclone_flags(&MountOptions::default(), Path::new("/c")).join(" "), "--vfs-cache-mode writes --cache-dir /c");
    }
}
