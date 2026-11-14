//! Optional Prometheus exporter of the desktop app (K108 extra).
//!
//! Off by default. When turned on (Settings → Advanced → Prometheus
//! metrics), the app serves `GET /metrics` on **127.0.0.1** only, on a
//! configurable port (default 9464), with the same backup figures as
//! `ufop-server`'s `/metrics` ([`super::metrics::render_backup`]) plus the
//! app's version and uptime. An optional token is required as
//! `Authorization: Bearer <token>`; only its SHA-256 is stored.
//!
//! The settings live in the `config` table (`desktop.metrics`), so the
//! exporter starts again with the app.

use super::metrics::{render_backup, Exposition};
use super::BackupManager;
use crate::core::error::AppError;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Config-table key of the exporter settings.
pub const SETTINGS_KEY: &str = "desktop.metrics";
/// Default port (the one OpenTelemetry's Prometheus exporter uses).
pub const DEFAULT_PORT: u16 = 9464;
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

fn default_port() -> u16 {
    DEFAULT_PORT
}

/// Stored settings (no secret: the token only as its hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsSettings {
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Hex SHA-256 of the scrape token, when one is required.
    pub token_sha256: Option<String>,
}

impl Default for MetricsSettings {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT, token_sha256: None }
    }
}

/// What the Settings screen sends.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetricsSettingsUpdate {
    pub enabled: bool,
    pub port: Option<u16>,
    /// `None`: keep the current token; `""`: no token; else the new token.
    pub token: Option<String>,
}

/// The exporter as the Settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricsStatus {
    pub enabled: bool,
    pub port: u16,
    pub token_set: bool,
    pub listening: bool,
    /// `http://127.0.0.1:<port>/metrics` while listening.
    pub url: Option<String>,
    /// Why the listener is not running although it is turned on.
    pub error: Option<String>,
}

#[derive(Default)]
struct Running {
    port: u16,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Default)]
struct Inner {
    settings: MetricsSettings,
    running: Option<Running>,
    error: Option<String>,
}

/// The desktop exporter (cheap to clone; managed Tauri state).
#[derive(Clone)]
pub struct DesktopMetrics {
    mgr: BackupManager,
    started: Instant,
    inner: Arc<Mutex<Inner>>,
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl DesktopMetrics {
    pub fn new(mgr: BackupManager) -> Self {
        Self { mgr, started: Instant::now(), inner: Arc::new(Mutex::new(Inner::default())) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn status(&self) -> MetricsStatus {
        let i = self.lock();
        let port = i.running.as_ref().map(|r| r.port);
        MetricsStatus {
            enabled: i.settings.enabled,
            port: i.settings.port,
            token_set: i.settings.token_sha256.is_some(),
            listening: port.is_some(),
            url: port.map(|p| format!("http://127.0.0.1:{p}/metrics")),
            error: i.error.clone(),
        }
    }

    /// Load the saved settings and start the listener when it is turned on
    /// (at app start). A port in use is reported in the status, not fatal.
    pub async fn start_from_settings(&self) -> MetricsStatus {
        let saved = match self.mgr.config_get(SETTINGS_KEY).await {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_default(),
            _ => MetricsSettings::default(),
        };
        self.lock().settings = saved;
        self.restart().await
    }

    /// Save new settings and (re)start or stop the listener.
    pub async fn configure(&self, u: MetricsSettingsUpdate) -> Result<MetricsStatus, AppError> {
        let mut s = self.lock().settings.clone();
        let port = u.port.unwrap_or(s.port);
        if port < 1024 {
            return Err(AppError::validation("Choose a port from 1024 to 65535 for the metrics listener"));
        }
        s.enabled = u.enabled;
        s.port = port;
        match u.token.as_deref().map(str::trim) {
            None => {}
            Some("") => s.token_sha256 = None,
            Some(t) if t.len() < 8 => return Err(AppError::validation("Use a token of at least 8 characters, or none")),
            Some(t) => s.token_sha256 = Some(sha256_hex(t)),
        }
        self.mgr.config_set(SETTINGS_KEY, serde_json::to_string(&s)?).await?;
        self.lock().settings = s;
        Ok(self.restart().await)
    }

    /// Stop the listener and wait (briefly) until its port is free again.
    pub async fn stop(&self) {
        let Some(r) = self.lock().running.take() else { return };
        if let Some(tx) = r.stop {
            let _ = tx.send(());
        }
        if let Some(mut task) = r.task {
            if tokio::time::timeout(std::time::Duration::from_secs(5), &mut task).await.is_err() {
                task.abort();
            }
        }
    }

    /// Apply the current settings: a token change applies to the running
    /// listener; a port change or turning it off stops it first.
    async fn restart(&self) -> MetricsStatus {
        let settings = self.lock().settings.clone();
        let running = self.lock().running.as_ref().map(|r| r.port);
        if settings.enabled && running == Some(settings.port) {
            return self.status();
        }
        self.stop().await;
        self.lock().error = None;
        if !settings.enabled {
            return self.status();
        }
        match self.listen(settings.port).await {
            Ok(r) => self.lock().running = Some(r),
            Err(e) => {
                tracing::warn!("Prometheus metrics listener: {}", e.user_message());
                self.lock().error = Some(e.user_message());
            }
        }
        self.status()
    }

    async fn listen(&self, port: u16) -> Result<Running, AppError> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| AppError::Configuration {
            message: format!("Cannot listen for metrics on {addr}: {e}"),
            advice: "Pick another port in Settings → Advanced → Prometheus metrics.".into(),
        })?;
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let app = axum::Router::new().route("/metrics", axum::routing::get(scrape)).with_state(self.clone());
        let task = tokio::spawn(async move {
            let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = rx.await;
            });
            if let Err(e) = serve.await {
                tracing::warn!("Prometheus metrics listener stopped: {e}");
            }
        });
        tracing::info!("Prometheus metrics on http://127.0.0.1:{port}/metrics");
        Ok(Running { port, stop: Some(tx), task: Some(task) })
    }

    /// The exposition: app version and uptime, then the backup figures.
    pub async fn render(&self) -> String {
        let mut o = Exposition::default();
        o.head("ufop_build_info", "gauge", "Version of this UFOP desktop app (always 1).");
        o.sample("ufop_build_info", &[("version", env!("CARGO_PKG_VERSION")), ("os", std::env::consts::OS), ("arch", std::env::consts::ARCH), ("app", "desktop")], 1);
        o.head("ufop_uptime_seconds", "gauge", "Seconds since the app started.");
        o.sample("ufop_uptime_seconds", &[], self.started.elapsed().as_secs());
        render_backup(&self.mgr, &mut o).await;
        o.0
    }

    fn authorized(&self, h: &HeaderMap) -> bool {
        let Some(want) = self.lock().settings.token_sha256.clone() else { return true };
        let got = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
        !got.is_empty() && constant_eq(&sha256_hex(got.trim()), &want)
    }
}

async fn scrape(State(m): State<DesktopMetrics>, h: HeaderMap) -> Response {
    if !m.authorized(&h) {
        return (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer realm=\"ufop-metrics\"")], "Unauthorized\n").into_response();
    }
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], m.render().await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn manager() -> BackupManager {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let dir = tempfile::tempdir().unwrap().keep();
        BackupManager::new(repo.pool().clone(), Arc::new(crate::security::CredentialStore::new()), None, None, dir)
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    async fn get(port: u16, token: Option<&str>) -> (u16, String) {
        let mut r = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/metrics"));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        let res = r.send().await.unwrap();
        (res.status().as_u16(), res.text().await.unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn off_by_default_then_serves_loopback_metrics_with_an_optional_token() {
        let mgr = manager().await;
        let m = DesktopMetrics::new(mgr.clone());
        let st = m.start_from_settings().await;
        assert_eq!((st.enabled, st.listening, st.port, st.token_set), (false, false, DEFAULT_PORT, false));

        let port = free_port();
        let st = m.configure(MetricsSettingsUpdate { enabled: true, port: Some(port), token: None }).await.unwrap();
        assert!(st.listening, "{st:?}");
        assert_eq!(st.url.as_deref(), Some(format!("http://127.0.0.1:{port}/metrics").as_str()));
        let (code, body) = get(port, None).await;
        assert_eq!(code, 200);
        let (_, samples) = crate::server::metrics::parse::parse(&body);
        let build = samples.iter().find(|s| s.name == "ufop_build_info").unwrap();
        assert_eq!(build.labels.get("app").map(String::as_str), Some("desktop"));
        assert!(samples.iter().any(|s| s.name == "ufop_scheduler_paused"), "the backup figures are included");

        // A token is required once set; only its hash is stored.
        let st = m.configure(MetricsSettingsUpdate { enabled: true, port: Some(port), token: Some("scrape-me-please".into()) }).await.unwrap();
        assert!(st.token_set && st.listening);
        assert_eq!(get(port, None).await.0, 401);
        assert_eq!(get(port, Some("wrong-token")).await.0, 401);
        assert_eq!(get(port, Some("scrape-me-please")).await.0, 200);
        let saved = mgr.config_get(SETTINGS_KEY).await.unwrap().unwrap();
        assert!(!saved.contains("scrape-me-please") && saved.contains(&sha256_hex("scrape-me-please")));
        assert!(m.configure(MetricsSettingsUpdate { enabled: true, port: Some(80), token: None }).await.is_err());
        assert!(m.configure(MetricsSettingsUpdate { enabled: true, port: None, token: Some("short".into()) }).await.is_err());

        // A restart keeps the settings; turning it off stops the listener.
        m.stop().await;
        let again = DesktopMetrics::new(mgr.clone());
        let st = again.start_from_settings().await;
        assert!(st.enabled && st.listening && st.token_set && st.port == port, "{st:?}");
        let st = again.configure(MetricsSettingsUpdate { enabled: false, port: None, token: Some(String::new()) }).await.unwrap();
        assert!(!st.listening && !st.token_set);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(reqwest::Client::new().get(format!("http://127.0.0.1:{port}/metrics")).send().await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_port_in_use_is_reported_not_fatal() {
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = busy.local_addr().unwrap().port();
        let m = DesktopMetrics::new(manager().await);
        let st = m.configure(MetricsSettingsUpdate { enabled: true, port: Some(port), token: None }).await.unwrap();
        assert!(st.enabled && !st.listening);
        assert!(st.error.unwrap().contains(&port.to_string()));
    }
}
