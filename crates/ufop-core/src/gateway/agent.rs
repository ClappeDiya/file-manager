//! The gateway agent: polls the server for jobs and serves them from local
//! roots.

use super::{split_path, Entry, ErrorBody, Hello, Job, Listing, Op, SIZE_HEADER, STATUS_HEADER};
use crate::confine::Roots;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Server base URL (`https://files.example.com`).
    pub server: String,
    /// API token (`ufop_pat_…`) with the write scope.
    pub token: String,
    pub gateway: String,
    pub roots: Vec<PathBuf>,
    pub writable: bool,
    /// Largest upload accepted (bytes); `None` = no limit.
    pub max_upload: Option<u64>,
    /// Jobs handled at once.
    pub concurrency: usize,
}

struct Ctx {
    cfg: AgentConfig,
    roots: Roots,
    http: reqwest::Client,
}

fn err(kind: &str, message: impl Into<String>) -> ErrorBody {
    ErrorBody { kind: kind.into(), message: message.into() }
}

impl Ctx {
    fn url(&self, p: &str) -> String {
        format!("{}/api/v1/gateway{p}", self.cfg.server.trim_end_matches('/'))
    }

    /// A client path → a confined local path (`must_exist` for reads).
    fn local(&self, path: &str, must_exist: bool) -> Result<PathBuf, ErrorBody> {
        let (root, rel) = split_path(path).map_err(|e| err("invalid", e))?;
        if root.is_empty() {
            return Err(err("invalid", "Choose a folder inside one of the gateway's roots"));
        }
        if !self.roots.list().iter().any(|r| r.name == root) {
            return Err(err("not_found", format!("No root named {root}")));
        }
        let spec = format!("{root}:{rel}");
        let r = if must_exist { self.roots.resolve(&spec, true) } else { self.roots.resolve_target(&spec) };
        r.map_err(|e| err(if must_exist { "not_found" } else { "forbidden" }, e))
    }

    fn list(&self, path: &str) -> Result<Listing, ErrorBody> {
        let (root, _) = split_path(path).map_err(|e| err("invalid", e))?;
        if root.is_empty() {
            let entries = self.roots.list().iter().map(|r| Entry { name: r.name.clone(), is_dir: true, size: 0, modified: None }).collect();
            return Ok(Listing { path: "/".into(), entries });
        }
        let dir = self.local(path, true)?;
        let rd = std::fs::read_dir(&dir).map_err(|e| err("not_found", format!("{path}: {e}")))?;
        let mut entries = Vec::new();
        for e in rd.filter_map(Result::ok) {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            if name.starts_with('.') {
                continue;
            }
            // Symlinks are followed only while they stay inside a root.
            let Ok(p) = self.roots.resolve(&e.path().to_string_lossy(), true) else { continue };
            let Ok(m) = std::fs::metadata(&p) else { continue };
            if !(m.is_dir() || m.is_file()) {
                continue;
            }
            let modified = m.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            entries.push(Entry { name, is_dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, modified });
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok(Listing { path: path.to_string(), entries })
    }

    async fn reply_error(&self, id: &str, e: ErrorBody) {
        let _ = self.http.post(self.url(&format!("/jobs/{id}/reply"))).bearer_auth(&self.cfg.token).header(STATUS_HEADER, "error").json(&e).send().await;
    }

    async fn reply_json<T: serde::Serialize>(&self, id: &str, v: &T) {
        let _ = self.http.post(self.url(&format!("/jobs/{id}/reply"))).bearer_auth(&self.cfg.token).header(STATUS_HEADER, "ok").json(v).send().await;
    }

    async fn download(&self, id: &str, path: &str) -> Result<(), ErrorBody> {
        let p = self.local(path, true)?;
        let f = tokio::fs::File::open(&p).await.map_err(|e| err("not_found", format!("{path}: {e}")))?;
        let len = f.metadata().await.map_err(|e| err("failed", e.to_string()))?.len();
        if !p.is_file() {
            return Err(err("invalid", format!("{path} is a folder")));
        }
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(f));
        let r = self
            .http
            .post(self.url(&format!("/jobs/{id}/reply")))
            .bearer_auth(&self.cfg.token)
            .header(STATUS_HEADER, "ok")
            .header(SIZE_HEADER, len.to_string())
            .header("content-type", "application/octet-stream")
            .body(body)
            .send()
            .await;
        if let Err(e) = r {
            tracing::warn!("gateway: sending {path} failed: {e}");
        }
        Ok(())
    }

    async fn upload(&self, id: &str, path: &str, size: Option<u64>) -> Result<u64, ErrorBody> {
        if !self.cfg.writable {
            return Err(err("forbidden", "This gateway is read-only"));
        }
        if let (Some(max), Some(s)) = (self.cfg.max_upload, size) {
            if s > max {
                return Err(err("forbidden", format!("The file is larger than the gateway accepts ({max} bytes)")));
            }
        }
        let target = self.local(path, false)?;
        let name = target.file_name().and_then(|n| n.to_str()).ok_or_else(|| err("invalid", "Give a file name"))?.to_string();
        let dir = target.parent().ok_or_else(|| err("invalid", "Give a file name"))?;
        if !dir.is_dir() {
            return Err(err("not_found", "The folder does not exist"));
        }
        if target.is_dir() {
            return Err(err("invalid", format!("{path} is a folder")));
        }
        let temp = dir.join(format!(".{name}.ufop-part"));
        let mut resp = self.http.get(self.url(&format!("/jobs/{id}/body"))).bearer_auth(&self.cfg.token).send().await.map_err(|e| err("failed", e.to_string()))?;
        if !resp.status().is_success() {
            return Err(err("failed", format!("Fetching the upload failed: {}", resp.status())));
        }
        let write = async {
            let mut f = tokio::fs::File::create(&temp).await.map_err(|e| err("failed", e.to_string()))?;
            let mut n = 0u64;
            while let Some(chunk) = resp.chunk().await.map_err(|e| err("failed", e.to_string()))? {
                n += chunk.len() as u64;
                if self.cfg.max_upload.is_some_and(|m| n > m) {
                    return Err(err("forbidden", "The file is larger than the gateway accepts"));
                }
                f.write_all(&chunk).await.map_err(|e| err("failed", e.to_string()))?;
            }
            f.sync_all().await.map_err(|e| err("failed", e.to_string()))?;
            Ok(n)
        };
        match write.await {
            Ok(n) => {
                tokio::fs::rename(&temp, &target).await.map_err(|e| err("failed", e.to_string()))?;
                Ok(n)
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp).await;
                Err(e)
            }
        }
    }

    async fn handle(self: Arc<Self>, job: Job) {
        let id = job.id.clone();
        let r = match job.op {
            Op::List { path } => match self.list(&path) {
                Ok(l) => {
                    self.reply_json(&id, &l).await;
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Op::Download { path } => self.download(&id, &path).await,
            Op::Upload { path, size } => match self.upload(&id, &path, size).await {
                Ok(n) => {
                    tracing::info!("gateway: received {path} ({n} bytes)");
                    self.reply_json(&id, &serde_json::json!({ "path": path, "bytes": n })).await;
                    Ok(())
                }
                Err(e) => Err(e),
            },
        };
        if let Err(e) = r {
            self.reply_error(&id, e).await;
        }
    }
}

/// Run until `stop` is set. Returns an error only for problems a retry
/// cannot fix (invalid roots or name, a rejected token).
pub async fn run(cfg: AgentConfig, stop: Arc<AtomicBool>) -> Result<(), String> {
    if !super::valid_name(&cfg.gateway) {
        return Err("The gateway name may use letters, digits, '-', '_' and '.' (up to 64)".into());
    }
    let roots = Roots::new(&cfg.roots)?;
    let hello = Hello {
        gateway: cfg.gateway.clone(),
        node: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        host: crate::agent::protocol::hostname(),
        roots: roots.list().iter().map(|r| r.name.clone()).collect(),
        writable: cfg.writable,
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let limit = Arc::new(tokio::sync::Semaphore::new(cfg.concurrency.max(1)));
    let ctx = Arc::new(Ctx { cfg, roots, http });
    let mut backoff = 1u64;
    tracing::info!("gateway {} (node {}) serving {:?}", hello.gateway, hello.node, hello.roots);
    let stopped = || async {
        while !stop.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    while !stop.load(Ordering::SeqCst) {
        let permit = tokio::select! {
            p = limit.clone().acquire_owned() => p.map_err(|e| e.to_string())?,
            _ = stopped() => break,
        };
        let send = ctx.http.post(ctx.url("/poll")).bearer_auth(&ctx.cfg.token).timeout(Duration::from_secs(super::POLL_WAIT_SECS + 15)).json(&hello).send();
        let r = tokio::select! {
            r = send => r,
            _ = stopped() => break,
        };
        match r {
            Ok(resp) if resp.status().as_u16() == 204 => {
                backoff = 1;
                drop(permit);
            }
            Ok(resp) if resp.status().is_success() => {
                backoff = 1;
                match resp.json::<Job>().await {
                    Ok(job) => {
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            ctx.handle(job).await;
                            drop(permit);
                        });
                    }
                    Err(e) => tracing::warn!("gateway: bad job: {e}"),
                }
            }
            Ok(resp) if matches!(resp.status().as_u16(), 401 | 403) => {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("The server refused the token: {body}"));
            }
            other => {
                drop(permit);
                match other {
                    Ok(resp) => tracing::warn!("gateway: poll answered {}", resp.status()),
                    Err(e) => tracing::warn!("gateway: server unreachable: {e}"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                    _ = stopped() => break,
                }
                backoff = (backoff * 2).min(30);
            }
        }
    }
    // Leave the pool at once instead of timing out.
    let _ = ctx.http.post(ctx.url("/leave")).bearer_auth(&ctx.cfg.token).timeout(Duration::from_secs(5)).json(&hello).send().await;
    Ok(())
}
