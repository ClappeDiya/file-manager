//! Multi-session hosts (Windows Remote Desktop / Citrix, Linux xrdp, shared
//! servers): several people run the app on one machine at once. Everything
//! machine-wide has to be per user:
//!
//! - temporary files go to a private per-user folder (`user_temp_dir`, 0700
//!   and owned by the user on Unix), never to fixed names in a shared `/tmp`;
//! - secrets written for a helper (`private_file`) are created exclusively
//!   with owner-only permissions, so another user can neither read them nor
//!   plant a symlink in their place;
//! - listeners that default to a fixed port fall back to a stable per-user
//!   port when another session holds it (`per_user_port`);
//! - `SessionInfo` says whether this is a remote/shared session, so the
//!   machine's name in the management console can include the user.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The signed-in user's name (best effort).
pub fn user_name() -> String {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| {
            #[cfg(unix)]
            {
                // SAFETY: getuid has no preconditions and cannot fail.
                format!("uid{}", unsafe { libc::getuid() })
            }
            #[cfg(not(unix))]
            {
                "user".to_string()
            }
        })
}

fn safe_component(s: &str) -> String {
    let c: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    c.trim_matches('.').chars().take(64).collect()
}

/// A private temporary folder for this user: `<temp>/ufop-<user>` (on Unix
/// the uid is used, and the folder must be owned by us with mode 0700; a
/// folder someone else planted is never used).
pub fn user_temp_dir() -> std::io::Result<PathBuf> {
    user_temp_dir_in(&std::env::temp_dir())
}

pub fn user_temp_dir_in(base: &Path) -> std::io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let dir = base.join(format!("ufop-{uid}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let md = std::fs::symlink_metadata(&dir)?;
        if !md.is_dir() || md.uid() != uid {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} is not a folder owned by this user", dir.display())));
        }
        if md.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(dir)
    }
    #[cfg(not(unix))]
    {
        // Windows temp folders are already per user (and per session on RDS).
        let dir = base.join(format!("ufop-{}", safe_component(&user_name())));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// `user_temp_dir()/<name>`, created.
pub fn user_temp_subdir(name: &str) -> PathBuf {
    let base = user_temp_dir().unwrap_or_else(|_| std::env::temp_dir().join(format!("ufop-{}-{}", safe_component(&user_name()), std::process::id())));
    let dir = base.join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Create a new file in `dir` holding `contents`, readable only by this user
/// (`mode` on Unix, e.g. 0o600 or 0o700 for a script). Fails rather than
/// following an existing file or symlink.
pub fn private_file(dir: &Path, name: &str, contents: &[u8], mode: u32) -> std::io::Result<PathBuf> {
    let path = dir.join(name);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = opts.open(&path)?;
    f.write_all(contents)?;
    f.sync_all()?;
    Ok(path)
}

/// A stable port for this user in `base+1 ..= base+span`, for listeners whose
/// default `base` is taken by another session.
pub fn per_user_port(base: u16, span: u16) -> u16 {
    // FNV-1a: stable across runs and platforms.
    let h = user_name().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    base.saturating_add(1 + (h % span.max(1) as u64) as u16)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionInfo {
    pub user: String,
    /// `console`, `rdp`, `citrix`, `xrdp`, `ssh` or `unknown`.
    pub kind: String,
    /// Remote or otherwise shared host: names and ports must be per user.
    pub shared: bool,
    pub session_name: Option<String>,
}

impl SessionInfo {
    pub fn detect() -> Self {
        Self::from_env(|k| std::env::var(k).ok())
    }

    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let user = ["USER", "USERNAME", "LOGNAME"].iter().find_map(|k| get(k).filter(|v| !v.is_empty())).unwrap_or_else(user_name);
        let session_name = get("SESSIONNAME").filter(|s| !s.is_empty());
        let kind = match session_name.as_deref() {
            Some(s) if s.to_ascii_uppercase().starts_with("RDP-") => "rdp",
            Some(s) if s.to_ascii_uppercase().starts_with("ICA-") => "citrix",
            Some(s) if s.eq_ignore_ascii_case("console") => "console",
            _ if get("XRDP_SESSION").is_some() => "xrdp",
            _ if get("SSH_CONNECTION").is_some() => "ssh",
            _ if get("XDG_SESSION_TYPE").is_some() || get("DISPLAY").is_some() || get("WAYLAND_DISPLAY").is_some() => "console",
            _ => "unknown",
        };
        Self { user, shared: matches!(kind, "rdp" | "citrix" | "xrdp" | "ssh"), kind: kind.into(), session_name }
    }

    /// The machine's display name: `host (user)` on shared hosts.
    pub fn machine_name(&self, host: &str) -> Option<String> {
        self.shared.then(|| format!("{host} ({})", self.user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn detects_remote_and_shared_sessions() {
        let rdp = SessionInfo::from_env(env(&[("USERNAME", "alice"), ("SESSIONNAME", "RDP-Tcp#3")]));
        assert_eq!((rdp.kind.as_str(), rdp.shared, rdp.user.as_str()), ("rdp", true, "alice"));
        assert_eq!(rdp.machine_name("TS01").as_deref(), Some("TS01 (alice)"));
        assert_eq!(SessionInfo::from_env(env(&[("SESSIONNAME", "ICA-CGP#12"), ("USERNAME", "b")])).kind, "citrix");
        let console = SessionInfo::from_env(env(&[("SESSIONNAME", "Console"), ("USERNAME", "c")]));
        assert!(!console.shared && console.machine_name("PC").is_none());
        assert_eq!(SessionInfo::from_env(env(&[("USER", "d"), ("XRDP_SESSION", "1"), ("DISPLAY", ":10")])).kind, "xrdp");
        assert_eq!(SessionInfo::from_env(env(&[("USER", "e"), ("SSH_CONNECTION", "1.2.3.4 5 6.7.8.9 22")])).kind, "ssh");
        assert_eq!(SessionInfo::from_env(env(&[("USER", "f"), ("WAYLAND_DISPLAY", "wayland-0")])).kind, "console");
    }

    #[test]
    fn ports_are_stable_per_user_and_in_range() {
        let a = per_user_port(47321, 500);
        assert_eq!(a, per_user_port(47321, 500));
        assert!((47322..=47821).contains(&a));
        assert_eq!(per_user_port(65530, 500), 65535, "never overflows");
    }

    #[cfg(unix)]
    #[test]
    fn temp_folders_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let dir = user_temp_dir_in(base.path()).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        // A loosened folder is tightened again.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(std::fs::metadata(user_temp_dir_in(base.path()).unwrap()).unwrap().permissions().mode() & 0o777, 0o700);

        let f = private_file(&dir, "askpass.sh", b"#!/bin/sh\necho x\n", 0o700).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(private_file(&dir, "askpass.sh", b"again", 0o600).is_err(), "never reuses an existing file");
        // A planted symlink is not followed.
        let target = base.path().join("stolen");
        std::os::unix::fs::symlink(&target, dir.join("trap")).unwrap();
        assert!(private_file(&dir, "trap", b"secret", 0o600).is_err());
        assert!(!target.exists());
        // A planted file where the folder should be is refused.
        let other = tempfile::tempdir().unwrap();
        // SAFETY: getuid has no preconditions.
        std::fs::write(other.path().join(format!("ufop-{}", unsafe { libc::getuid() })), "x").unwrap();
        assert!(user_temp_dir_in(other.path()).is_err());
    }
}
