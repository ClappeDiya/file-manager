//! `ufop gateway …` — the on-premises file gateway (`ufop_core::gateway`).
//!
//! `serve` runs the agent: it connects out to a ufop-server and serves the
//! given folders to that server's users (no inbound port; run several with
//! the same name for high availability). The other commands use a gateway
//! through the server you signed in to with `ufop server login`.

use super::server::{show, urlencode, Client};
use crate::OutputFormat;
use clap::Subcommand;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

#[derive(Subcommand, Debug)]
pub enum GatewayCommands {
    /// Serve local folders through a ufop-server until stopped (Ctrl+C)
    Serve {
        /// Server URL
        #[arg(long, env = "UFOP_SERVER_URL")]
        server: String,
        /// API token with the write scope (`ufop server tokens create --scope write`)
        #[arg(long, env = "UFOP_GATEWAY_TOKEN", hide_env_values = true)]
        token: String,
        /// Gateway name clients use; agents with the same name share the work
        #[arg(long)]
        name: String,
        /// Folder to serve (repeatable); reachable as /<folder name>/…
        #[arg(long = "root", required = true)]
        roots: Vec<PathBuf>,
        /// Accept uploads
        #[arg(long)]
        writable: bool,
        /// Largest upload in MB (default: no limit)
        #[arg(long)]
        max_upload_mb: Option<u64>,
        /// Jobs handled at once
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
    },
    /// Gateways connected to the server and their nodes
    List,
    /// List a folder through a gateway (`/` = its roots)
    Ls {
        gateway: String,
        #[arg(default_value = "/")]
        path: String,
    },
    /// Download a file through a gateway
    Get {
        gateway: String,
        /// /<root>/<file>
        path: String,
        /// Local file (default: the file's name in the current folder)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Upload a local file through a writable gateway
    Put {
        gateway: String,
        local: PathBuf,
        /// /<root>/<file> on the gateway
        path: String,
    },
}

async fn error_of(r: reqwest::Response) -> String {
    let v: serde_json::Value = r.json().await.unwrap_or_default();
    format!("{} {}", v["message"].as_str().unwrap_or("Request failed"), v["advice"].as_str().unwrap_or("")).trim().to_string()
}

pub async fn execute(cmd: GatewayCommands, format: &OutputFormat) -> Result<u8, Box<dyn std::error::Error>> {
    match cmd {
        GatewayCommands::Serve { server, token, name, roots, writable, max_upload_mb, concurrency } => {
            let stop = Arc::new(AtomicBool::new(false));
            let s2 = stop.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                s2.store(true, Ordering::SeqCst);
            });
            let cfg = ufop_core::gateway::AgentConfig { server, token, gateway: name.clone(), roots, writable, max_upload: max_upload_mb.map(|m| m << 20), concurrency };
            eprintln!("Serving gateway '{name}' ({}); press Ctrl+C to stop.", if writable { "uploads allowed" } else { "read-only" });
            ufop_core::gateway::run(cfg, stop).await?;
            Ok(0)
        }
        GatewayCommands::List => {
            let mut c = Client::new()?;
            let v = c.call(reqwest::Method::GET, "/gateways", None).await?;
            show(format, if matches!(format, OutputFormat::Human | OutputFormat::Csv) { &v["gateways"] } else { &v });
            Ok(0)
        }
        GatewayCommands::Ls { gateway, path } => {
            let mut c = Client::new()?;
            let v = c.call(reqwest::Method::GET, &format!("/gateways/{gateway}/files?path={}", urlencode(&path)), None).await?;
            show(format, if matches!(format, OutputFormat::Human | OutputFormat::Csv) { &v["entries"] } else { &v });
            Ok(0)
        }
        GatewayCommands::Get { gateway, path, output } => {
            let c = Client::new()?;
            let out = output.unwrap_or_else(|| PathBuf::from(path.rsplit('/').next().unwrap_or("download")));
            let mut r = c.raw(reqwest::Method::GET, &format!("/gateways/{gateway}/download?path={}", urlencode(&path))).send().await?;
            if !r.status().is_success() {
                return Err(error_of(r).await.into());
            }
            let part = out.with_extension(format!("{}ufop-part", out.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()));
            let mut f = tokio::fs::File::create(&part).await?;
            let mut n = 0u64;
            while let Some(chunk) = r.chunk().await? {
                f.write_all(&chunk).await?;
                n += chunk.len() as u64;
            }
            f.sync_all().await?;
            drop(f);
            tokio::fs::rename(&part, &out).await?;
            show(format, &serde_json::json!({ "path": path, "saved": out.display().to_string(), "bytes": n }));
            Ok(0)
        }
        GatewayCommands::Put { gateway, local, path } => {
            let c = Client::new()?;
            let f = tokio::fs::File::open(&local).await?;
            let len = f.metadata().await?.len();
            let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(f));
            let r = c.raw(reqwest::Method::PUT, &format!("/gateways/{gateway}/upload?path={}", urlencode(&path))).header("content-length", len).body(body).send().await?;
            if !r.status().is_success() {
                return Err(error_of(r).await.into());
            }
            show(format, &r.json::<serde_json::Value>().await.unwrap_or_default());
            Ok(0)
        }
    }
}
