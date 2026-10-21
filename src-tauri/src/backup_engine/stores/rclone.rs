//! Rclone bridge: any of rclone's 70+ storage systems (MEGA, Jottacloud,
//! Koofr, Sugarsync, …) as a backup destination, through the `rclone`
//! command-line tool and a configured remote.

use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
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
}

impl Default for RcloneConfig {
    fn default() -> Self {
        Self { remote: String::new(), path: String::new(), binary: "rclone".into(), config_file: None, extra_args: Vec::new() }
    }
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
        let mut cmd = tokio::process::Command::new(&self.cfg.binary);
        if let Some(c) = &self.cfg.config_file {
            cmd.arg("--config").arg(c);
        }
        cmd.args(&self.cfg.extra_args).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        if let Some(p) = &self.config_pass {
            cmd.env("RCLONE_CONFIG_PASS", p);
        }
        let mut child = cmd.spawn().map_err(|e| AppError::connection(format!("Could not run {}: {e}", self.cfg.binary), "Install rclone (https://rclone.org) or set its path."))?;
        if let (Some(data), Some(mut si)) = (stdin, child.stdin.take()) {
            si.write_all(&data).await.map_err(|e| AppError::connection(format!("rclone input: {e}"), "Retry."))?;
            drop(si);
        }
        let out = child.wait_with_output().await.map_err(|e| AppError::connection(format!("rclone: {e}"), "Retry."))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").to_string();
            return Err(AppError::connection(format!("rclone {} failed: {last}", args.first().unwrap_or(&"")), "Check the rclone remote with `rclone lsd <remote>:`."));
        }
        Ok(out.stdout)
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
