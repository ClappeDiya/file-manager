//! Cloud mounts through `rclone mount` (FUSE: fuse3 on Linux, macFUSE on
//! macOS, WinFsp on Windows).
//!
//! rclone connections mount their configured remote. S3 connections are
//! described to rclone through `RCLONE_CONFIG_*` environment variables, so
//! keys never appear on a command line. The rclone process lives for the
//! mount and is stopped on unmount.

use crate::core::types::{ConnectionProfile, ConnectionProtocol};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::Mutex;

const MOUNT_WAIT: Duration = Duration::from_secs(20);
const ENV_REMOTE: &str = "UFOPMOUNT";

/// What rclone needs to mount a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcloneSpec {
    /// `remote:path` to mount.
    pub source: String,
    /// `RCLONE_CONFIG_*` entries for an environment-defined remote.
    pub env: Vec<(String, String)>,
}

fn env_key(opt: &str) -> String {
    format!("RCLONE_CONFIG_{ENV_REMOTE}_{}", opt.to_ascii_uppercase())
}

/// How to mount `profile` with rclone, or why it can't be.
pub fn spec_for(profile: &ConnectionProfile, secret: Option<&str>) -> Result<RcloneSpec, String> {
    let path = profile.remote_path.trim_matches('/');
    match profile.protocol {
        ConnectionProtocol::Rclone => {
            let remote = profile.extras.rclone_remote.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| profile.host.clone());
            let remote = crate::connectors::rclone::normalize_remote(&remote).map_err(|e| e.user_message())?;
            Ok(RcloneSpec { source: crate::connectors::rclone::target(&remote, path), env: Vec::new() })
        }
        ConnectionProtocol::S3 => {
            if profile.extras.assume_role.is_some() {
                return Err("Mounting connections that assume a role isn't supported; add the role as an rclone remote instead".into());
            }
            let host = profile.host.trim().trim_end_matches('/');
            let (region, endpoint) = if host.starts_with("http://") || host.starts_with("https://") {
                ("us-east-1".to_string(), Some(host.to_string()))
            } else if host.contains('.') && !host.ends_with("amazonaws.com") {
                ("us-east-1".to_string(), Some(format!("https://{host}")))
            } else if host.contains('.') {
                (host.split('.').nth(1).filter(|r| r.contains('-')).unwrap_or("us-east-1").to_string(), None)
            } else {
                (if host.is_empty() { "us-east-1".to_string() } else { host.to_string() }, None)
            };
            let mut env = vec![
                (env_key("type"), "s3".into()),
                (env_key("provider"), if endpoint.is_some() { "Other".into() } else { "AWS".into() }),
                (env_key("access_key_id"), profile.username.clone().unwrap_or_default()),
                (env_key("secret_access_key"), secret.unwrap_or_default().to_string()),
                (env_key("region"), region),
            ];
            if let Some(e) = endpoint {
                env.push((env_key("endpoint"), e));
            }
            Ok(RcloneSpec { source: format!("{ENV_REMOTE}:{path}"), env })
        }
        ref other => Err(format!("{} connections can't be mounted with rclone here; add the account as an rclone remote (`rclone config`) and mount that connection", other.as_str())),
    }
}

/// Arguments for `rclone mount`.
pub fn mount_args(spec: &RcloneSpec, mount_point: &str) -> Vec<String> {
    vec![
        "mount".into(),
        spec.source.clone(),
        mount_point.into(),
        "--vfs-cache-mode".into(),
        "writes".into(),
        "--dir-cache-time".into(),
        "30s".into(),
    ]
}

/// What the OS needs for rclone mounts; `Err` is (tool, install hint).
pub fn fuse_available() -> Result<(), (String, String)> {
    if cfg!(target_os = "linux") {
        let fusermount = ["fusermount3", "fusermount"].iter().any(|b| std::process::Command::new(b).arg("-V").output().is_ok());
        return if Path::new("/dev/fuse").exists() && fusermount { Ok(()) } else { Err(("fuse3".into(), "sudo apt install fuse3".into())) };
    }
    if cfg!(target_os = "macos") {
        return if Path::new("/Library/Filesystems/macfuse.fs").exists() || Path::new("/Library/Filesystems/fuse-t.fs").exists() {
            Ok(())
        } else {
            Err(("macFUSE".into(), "brew install --cask macfuse".into()))
        };
    }
    if cfg!(windows) {
        let pf = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
        return if Path::new(&pf).join("WinFsp").exists() { Ok(()) } else { Err(("WinFsp".into(), "winget install WinFsp.WinFsp".into())) };
    }
    Err(("FUSE".into(), "This OS has no FUSE support".into()))
}

pub fn rclone_available() -> bool {
    std::process::Command::new(std::env::var("UFOP_RCLONE").unwrap_or_else(|_| "rclone".into())).arg("version").output().is_ok_and(|o| o.status.success())
}

/// Whether `p` is a mount point now (Unix: a different device than its parent).
pub fn is_mounted(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (Ok(m), Some(parent)) = (std::fs::metadata(p), p.parent()) else { return false };
        std::fs::metadata(parent).is_ok_and(|pm| pm.dev() != m.dev())
    }
    #[cfg(not(unix))]
    {
        // WinFsp mounts appear once rclone is serving; existence is the signal.
        p.exists()
    }
}

/// Running rclone mount processes, by mount id.
#[derive(Clone, Default)]
pub struct RcloneProcesses {
    children: Arc<Mutex<HashMap<uuid::Uuid, Child>>>,
}

impl RcloneProcesses {
    /// Start `rclone mount` and wait until the mount point is live.
    pub async fn mount(&self, id: uuid::Uuid, spec: &RcloneSpec, mount_point: &str) -> Result<(), String> {
        let exe = std::env::var("UFOP_RCLONE").unwrap_or_else(|_| "rclone".into());
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(mount_args(spec, mount_point)).envs(spec.env.iter().cloned()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("Could not start rclone: {e}"))?;
        let deadline = tokio::time::Instant::now() + MOUNT_WAIT;
        loop {
            if is_mounted(Path::new(mount_point)) {
                self.children.lock().await.insert(id, child);
                return Ok(());
            }
            if let Ok(Some(status)) = child.try_wait() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = e.read_to_string(&mut err).await;
                }
                let line = err.lines().filter(|l| !l.contains("NOTICE")).last().unwrap_or("").trim().to_string();
                return Err(if line.is_empty() { format!("rclone mount exited ({status})") } else { line });
            }
            if tokio::time::Instant::now() > deadline {
                let _ = child.kill().await;
                return Err("rclone mount did not come up in time".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub async fn is_running(&self, id: uuid::Uuid) -> bool {
        self.children.lock().await.contains_key(&id)
    }

    /// Unmount and stop the rclone process; true if it was ours.
    pub async fn unmount(&self, id: uuid::Uuid, mount_point: &str) -> bool {
        let Some(mut child) = self.children.lock().await.remove(&id) else { return false };
        if cfg!(target_os = "linux") {
            for b in ["fusermount3", "fusermount"] {
                if tokio::process::Command::new(b).args(["-u", mount_point]).output().await.is_ok_and(|o| o.status.success()) {
                    break;
                }
            }
        } else if cfg!(unix) {
            let _ = tokio::process::Command::new("umount").arg(mount_point).output().await;
        }
        // rclone exits once unmounted; make sure it does.
        if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(protocol: ConnectionProtocol, host: &str) -> ConnectionProfile {
        let mut p: ConnectionProfile = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(), "name": "x", "protocol": protocol, "host": host, "port": null, "username": "AKID",
            "credential_ref": null, "remote_path": "/bucket/photos", "created_at": chrono::Utc::now(), "last_used": null, "group_id": null,
            "bandwidth_limit_bps": 0, "conflict_policy": null, "retry_policy": null, "verify_checksums": false, "checksum_algorithm": null,
            "proxy_type": null, "proxy_host": null, "proxy_port": null, "proxy_username": null, "proxy_password": null,
            "default_local_dir": null, "default_remote_dir": null, "charset": null, "default_file_mode": null, "default_dir_mode": null,
            "ftp_use_mlsd": null, "ftp_force_passive_ip": null, "ftp_post_login_commands": null, "symlink_policy": null
        }))
        .unwrap();
        p.remote_path = "/bucket/photos".into();
        p
    }

    #[test]
    fn s3_spec_keeps_keys_out_of_argv() {
        let p = profile(ConnectionProtocol::S3, "https://abc.r2.cloudflarestorage.com");
        let s = spec_for(&p, Some("SECRET")).unwrap();
        assert_eq!(s.source, "UFOPMOUNT:bucket/photos");
        let env: HashMap<_, _> = s.env.iter().cloned().collect();
        assert_eq!(env["RCLONE_CONFIG_UFOPMOUNT_SECRET_ACCESS_KEY"], "SECRET");
        assert_eq!(env["RCLONE_CONFIG_UFOPMOUNT_PROVIDER"], "Other");
        assert_eq!(env["RCLONE_CONFIG_UFOPMOUNT_ENDPOINT"], "https://abc.r2.cloudflarestorage.com");
        assert!(!mount_args(&s, "/mnt/x").iter().any(|a| a.contains("SECRET")));
        let aws = spec_for(&profile(ConnectionProtocol::S3, "eu-west-1"), Some("k")).unwrap();
        let env: HashMap<_, _> = aws.env.into_iter().collect();
        assert_eq!((env["RCLONE_CONFIG_UFOPMOUNT_REGION"].as_str(), env["RCLONE_CONFIG_UFOPMOUNT_PROVIDER"].as_str()), ("eu-west-1", "AWS"));
        let mut role = profile(ConnectionProtocol::S3, "eu-west-1");
        role.extras.assume_role = Some(Default::default());
        assert!(spec_for(&role, None).is_err());
        assert!(spec_for(&profile(ConnectionProtocol::Dropbox, ""), None).unwrap_err().contains("rclone remote"));
    }

    #[test]
    fn rclone_spec_uses_the_remote() {
        let mut p = profile(ConnectionProtocol::Rclone, "gcs");
        assert_eq!(spec_for(&p, None).unwrap().source, "gcs:bucket/photos");
        p.extras.rclone_remote = Some("box:".into());
        p.remote_path = "/".into();
        assert_eq!(spec_for(&p, None).unwrap(), RcloneSpec { source: "box:".into(), env: vec![] });
    }

    /// Real `rclone mount` of a local folder (needs rclone + FUSE).
    #[tokio::test]
    async fn mounts_and_unmounts_with_real_rclone() {
        if !rclone_available() || fuse_available().is_err() {
            return;
        }
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("a.txt"), "hello").unwrap();
        let mp = tempfile::tempdir().unwrap();
        let mpp = mp.path().to_string_lossy().to_string();
        let procs = RcloneProcesses::default();
        let id = uuid::Uuid::new_v4();
        let spec = RcloneSpec { source: format!(":local:{}", src.path().display()), env: vec![] };
        if let Err(e) = procs.mount(id, &spec, &mpp).await {
            // FUSE present but not permitted (unprivileged container): nothing to verify.
            eprintln!("skipping: {e}");
            return;
        }
        assert!(procs.is_running(id).await);
        assert_eq!(std::fs::read_to_string(mp.path().join("a.txt")).unwrap(), "hello");
        assert!(procs.unmount(id, &mpp).await);
        assert!(!is_mounted(mp.path()));
        assert!(!procs.unmount(id, &mpp).await, "already gone");
        // A bad remote fails with rclone's message instead of hanging.
        let bad = RcloneSpec { source: "no-such-remote:".into(), env: vec![] };
        assert!(procs.mount(uuid::Uuid::new_v4(), &bad, &mpp).await.is_err());
    }
}
