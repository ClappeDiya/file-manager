//! Rclone bridge: any of rclone's 70+ storage systems (MEGA, Jottacloud,
//! Koofr, Sugarsync, …) as a backup destination, through the `rclone`
//! command-line tool and a configured remote.
//!
//! Process control: every rclone call runs in its own process group (unix)
//! and is killed (with anything it started) when it times out, when it
//! prints nothing before the startup timeout, or when the operation is
//! dropped (cancelled). Its stderr goes into the error's advice. The
//! environment can be passed through selectively.

use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RcloneConfig {
    /// Remote name from `rclone config` (e.g. `mega`), or an on-the-fly
    /// remote such as `:local:` / `:sftp,host=…:`.
    pub remote: String,
    /// Folder on the remote.
    pub path: String,
    /// rclone executable.
    pub binary: String,
    /// Alternative rclone config file.
    pub config_file: Option<String>,
    /// Extra flags passed to every call (e.g. `--tpslimit 4`).
    pub extra_args: Vec<String>,
    /// Kill a call that runs longer than this (seconds; 0 = no limit).
    pub timeout_secs: u64,
    /// Kill a call that prints nothing and has not finished within this
    /// many seconds (hung sign-in or config prompt; 0 = no limit).
    pub startup_timeout_secs: u64,
    /// Extra environment variables for rclone (`RCLONE_*` options, proxies, …).
    pub env: BTreeMap<String, String>,
    /// Start rclone with an empty environment except the system basics
    /// (PATH, HOME, temp folders) and `env_passthrough`.
    pub isolate_env: bool,
    /// Variables of the app's environment passed to rclone when isolated.
    pub env_passthrough: Vec<String>,
}

impl Default for RcloneConfig {
    fn default() -> Self {
        Self {
            remote: String::new(),
            path: String::new(),
            binary: "rclone".into(),
            config_file: None,
            extra_args: Vec::new(),
            timeout_secs: 0,
            startup_timeout_secs: 0,
            env: BTreeMap::new(),
            isolate_env: false,
            env_passthrough: Vec::new(),
        }
    }
}

/// Variables an isolated rclone still gets (it needs them to start and to
/// find its config and temp folders).
const BASE_ENV: &[&str] = &["PATH", "HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "SYSTEMROOT", "TMP", "TEMP", "TMPDIR", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"];

/// Kills the child's whole process group when dropped: after the call,
/// on timeouts and when the operation is cancelled.
struct Reaper {
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
}

impl Drop for Reaper {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // The child leads its own group: take rclone and its helpers down.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

/// The last lines of stderr, for error advice.
fn stderr_tail(err: &str) -> String {
    let lines: Vec<&str> = err.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(5)..].join(" | ");
    tail.chars().take(600).collect()
}

pub struct RcloneStore {
    cfg: RcloneConfig,
    /// rclone config password (`RCLONE_CONFIG_PASS`), when the config is encrypted.
    config_pass: Option<String>,
}

impl RcloneStore {
    pub fn new(cfg: RcloneConfig, config_pass: Option<String>) -> Self {
        Self { cfg, config_pass: config_pass.filter(|p| !p.is_empty()) }
    }

    fn target(&self, name: Option<&str>) -> String {
        let r = self.cfg.remote.trim();
        let sep = if r.ends_with(':') { "" } else { ":" };
        let path = self.cfg.path.trim_end_matches('/');
        match name {
            Some(n) if path.is_empty() => format!("{r}{sep}{n}"),
            Some(n) => format!("{r}{sep}{path}/{n}"),
            None => format!("{r}{sep}{path}"),
        }
    }

    async fn run(&self, args: &[&str], stdin: Option<Vec<u8>>) -> Result<Vec<u8>, AppError> {
        let what = args.first().copied().unwrap_or("");
        let mut cmd = tokio::process::Command::new(&self.cfg.binary);
        if let Some(c) = &self.cfg.config_file {
            cmd.arg("--config").arg(c);
        }
        cmd.args(&self.cfg.extra_args).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        // Killed when the operation is dropped; the group kill (below) also
        // reaches anything rclone started.
        cmd.kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        if self.cfg.isolate_env {
            cmd.env_clear();
            for k in BASE_ENV.iter().copied().chain(self.cfg.env_passthrough.iter().map(String::as_str)) {
                if let Ok(v) = std::env::var(k) {
                    cmd.env(k, v);
                }
            }
        }
        cmd.envs(&self.cfg.env);
        if let Some(p) = &self.config_pass {
            cmd.env("RCLONE_CONFIG_PASS", p);
        }
        let mut child = cmd.spawn().map_err(|e| AppError::connection(format!("Could not run {}: {e}", self.cfg.binary), "Install rclone (https://rclone.org) or set its path."))?;
        let reaper = Reaper { pid: child.id() };
        let (mut out, mut err, input) = (child.stdout.take(), child.stderr.take(), child.stdin.take());
        let feed = async {
            if let (Some(data), Some(mut si)) = (stdin, input) {
                si.write_all(&data).await.map_err(|e| AppError::connection(format!("rclone input: {e}"), "Retry."))?;
                drop(si);
            }
            Ok::<(), AppError>(())
        };
        let started = tokio::sync::Notify::new();
        let read_out = async {
            let mut buf = Vec::new();
            if let Some(o) = out.as_mut() {
                let mut chunk = [0u8; 64 * 1024];
                loop {
                    let n = o.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    started.notify_one();
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            buf
        };
        let read_err = async {
            let mut buf = Vec::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_end(&mut buf).await;
            }
            buf
        };
        let work = async {
            let (fed, stdout, stderr) = tokio::join!(feed, read_out, read_err);
            let status = child.wait().await.map_err(|e| AppError::connection(format!("rclone: {e}"), "Retry."))?;
            Ok::<_, AppError>((fed, status, stdout, stderr))
        };
        let limit = |secs: u64| if secs == 0 { Duration::MAX } else { Duration::from_secs(secs) };
        let startup = async {
            if self.cfg.startup_timeout_secs == 0 {
                std::future::pending::<()>().await;
            }
            tokio::time::timeout(limit(self.cfg.startup_timeout_secs), started.notified()).await.is_ok()
        };
        tokio::pin!(work, startup);
        let deadline = tokio::time::sleep(limit(self.cfg.timeout_secs).min(Duration::from_secs(10 * 365 * 86_400)));
        tokio::pin!(deadline);
        let mut started_ok = false;
        let (fed, status, stdout, stderr) = loop {
            tokio::select! {
                r = &mut work => break r?,
                ok = &mut startup, if !started_ok => {
                    if !ok {
                        return Err(AppError::connection(
                            format!("rclone {what} printed nothing within {}s and was stopped", self.cfg.startup_timeout_secs),
                            "rclone may be waiting for a password or a sign-in: check the remote with `rclone lsd <remote>:` in a terminal, set the config password, or raise the startup timeout.",
                        ));
                    }
                    started_ok = true;
                }
                _ = &mut deadline => {
                    return Err(AppError::connection(
                        format!("rclone {what} took longer than {}s and was stopped", self.cfg.timeout_secs),
                        "Raise the rclone timeout for large volumes or slow remotes, or check the connection.",
                    ));
                }
            }
        };
        // rclone exited; anything it left behind in its group goes too.
        drop(reaper);
        // A failing rclone may exit before reading its input: its own error
        // explains more than the broken pipe that leaves on our side.
        if !status.success() {
            let err = String::from_utf8_lossy(&stderr);
            let last = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").to_string();
            let tail = stderr_tail(&err);
            let advice = if tail.is_empty() { "Check the rclone remote with `rclone lsd <remote>:`.".to_string() } else { format!("rclone said: {tail}. Check the remote with `rclone lsd <remote>:`.") };
            return Err(AppError::connection(format!("rclone {what} failed: {last}"), advice));
        }
        fed?;
        Ok(stdout)
    }
}

#[derive(Deserialize)]
struct LsEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Size")]
    size: i64,
    #[serde(rename = "IsDir", default)]
    is_dir: bool,
}

impl ObjectStore for RcloneStore {
    fn describe(&self) -> String {
        format!("rclone://{}", self.target(None))
    }
    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let out = self.run(&["lsjson", "--files-only", "--no-modtime", "--no-mimetype", &self.target(None)], None).await?;
            let v: Vec<LsEntry> = serde_json::from_slice(&out).map_err(|e| AppError::connection(format!("Unexpected rclone output: {e}"), "Update rclone."))?;
            Ok(v.into_iter().filter(|e| !e.is_dir).map(|e| RemoteFile { name: e.name, size: e.size.max(0) as u64 }).collect())
        })
    }
    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            self.run(&["rcat", "--size", &data.len().to_string(), &self.target(Some(name))], Some(data)).await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move { self.run(&["cat", &self.target(Some(name))], None).await })
    }
    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            self.run(&["deletefile", &self.target(Some(name))], None).await?;
            Ok(())
        })
    }
    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            self.run(&["mkdir", &self.target(None)], None).await?;
            Ok(())
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake rclone: `lsjson` lists one file, `cat` echoes the env, `rcat`
    /// fails with stderr, `mkdir` hangs silently after starting a helper.
    fn fake(dir: &std::path::Path) -> String {
        let p = dir.join("rclone");
        let pidfile = dir.join("helper.pid");
        std::fs::write(
            &p,
            format!(
                r#"#!/bin/sh
case "$*" in
  *lsjson*) echo '[{{"Name":"ufop-a.dblock","Size":5,"IsDir":false}},{{"Name":"sub","Size":0,"IsDir":true}}]' ;;
  *rcat*) echo "NOTICE: first line" >&2; echo "Failed to rcat: quota exceeded on remote" >&2; exit 3 ;;
  *cat*) printf '%s|%s|%s' "$UFOP_FAKE" "$RCLONE_CONFIG_PASS" "${{SECRET_FROM_APP:-unset}}" ;;
  *mkdir*) sleep 60 & echo $! > {pid}; sleep 60 ;;
  *deletefile*) echo started; sleep 60 ;;
esac
"#,
                pid = pidfile.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[tokio::test]
    async fn timeouts_kill_the_process_group_env_and_stderr() {
        let d = tempfile::tempdir().unwrap();
        let cfg = RcloneConfig { remote: "fake:".into(), binary: fake(d.path()), ..Default::default() };
        let s = RcloneStore::new(cfg.clone(), Some("cfgpass".into()));
        assert_eq!(s.list().await.unwrap(), vec![RemoteFile { name: "ufop-a.dblock".into(), size: 5 }]);
        // Extra variables and the config password reach rclone.
        let mut c = cfg.clone();
        c.env.insert("UFOP_FAKE".into(), "on".into());
        std::env::set_var("SECRET_FROM_APP", "leaked");
        let out = RcloneStore::new(c.clone(), Some("cfgpass".into())).get("x").await.unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "on|cfgpass|leaked");
        // Isolated: only the basics and the passthrough list.
        c.isolate_env = true;
        let out = RcloneStore::new(c.clone(), None).get("x").await.unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "on||unset");
        c.env_passthrough = vec!["SECRET_FROM_APP".into()];
        let out = RcloneStore::new(c, None).get("x").await.unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "on||leaked");
        // Failures carry rclone's stderr in the advice.
        let e = s.put("v", b"data".to_vec()).await.unwrap_err();
        assert!(e.user_message().contains("quota exceeded"), "{}", e.user_message());
        match &e {
            AppError::Connection { advice, .. } => assert!(advice.contains("first line") && advice.contains("quota exceeded"), "{advice}"),
            other => panic!("{other:?}"),
        }
        // Silent hang → startup timeout; rclone and its helper are killed.
        let hung = RcloneStore::new(RcloneConfig { startup_timeout_secs: 1, ..cfg.clone() }, None);
        let t = std::time::Instant::now();
        let e = hung.create_folder().await.unwrap_err();
        assert!(e.user_message().contains("printed nothing within 1s"), "{}", e.user_message());
        assert!(t.elapsed() < Duration::from_secs(10));
        let helper: i32 = std::fs::read_to_string(d.path().join("helper.pid")).unwrap().trim().parse().unwrap();
        for _ in 0..50 {
            if !alive(helper) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(helper), "the helper rclone started was killed with its group");
        // Output started but the call runs too long → overall timeout.
        let slow = RcloneStore::new(RcloneConfig { timeout_secs: 1, startup_timeout_secs: 5, ..cfg.clone() }, None);
        let e = slow.delete("v").await.unwrap_err();
        assert!(e.user_message().contains("longer than 1s"), "{}", e.user_message());
        // A dropped (cancelled) call kills rclone too.
        std::fs::remove_file(d.path().join("helper.pid")).unwrap();
        let s2 = RcloneStore::new(cfg, None);
        let _ = tokio::time::timeout(Duration::from_millis(700), s2.create_folder()).await;
        let helper: i32 = std::fs::read_to_string(d.path().join("helper.pid")).unwrap().trim().parse().unwrap();
        for _ in 0..50 {
            if !alive(helper) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(helper), "cancelled operation left no process behind");
    }
}
