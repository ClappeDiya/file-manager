//! The web server inside the desktop app (Settings > Local web server,
//! Advanced). It is the same `ufop-server` code the standalone program runs
//! ([`crate::server`]), started in this process on request: the same login,
//! accounts, API and web interface, sharing the app's backup jobs and
//! connections.
//!
//! - Off until started, and it never starts without a password; the password
//!   is the server's own account code ([`AuthService::set_password`]).
//! - Loopback only unless the person allows other computers, which also turns
//!   on HTTPS (a generated certificate) so passwords are not sent in the clear.
//! - Stopping it stops only the listener: the server runs with
//!   `ServerConfig::embedded`, so it never shuts down or cancels the app's
//!   backup manager and installs no signal handlers.

use crate::backup_engine::BackupManager;
use crate::core::error::AppError;
use crate::server::auth::AuthService;
use crate::server::lifecycle::Lifecycle;
use crate::server::{self, ServerConfig, ServerState, TlsMode};
use crate::connectors::ConnectionManager;
use crate::security::settings_store::SettingsStore;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Config-table key of the saved settings.
pub const SETTINGS_KEY: &str = "embedded_server.settings";
pub const DEFAULT_PORT: u16 = 8200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddedSettings {
    pub port: u16,
    /// Listen on every network interface (with HTTPS) instead of this computer only.
    pub allow_network: bool,
    /// HTTPS with a generated certificate on this computer too. Always on with `allow_network`.
    pub https: bool,
    /// Folder holding the web interface (an `index.html`); empty = look for one next to the app.
    pub web_root: Option<String>,
}

impl Default for EmbeddedSettings {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, allow_network: false, https: false, web_root: None }
    }
}

impl EmbeddedSettings {
    fn uses_https(&self) -> bool {
        self.https || self.allow_network
    }

    fn validate(&self) -> Result<(), AppError> {
        if self.port < 1024 {
            return Err(AppError::validation("Choose a port from 1024 to 65535"));
        }
        if let Some(root) = self.web_root.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            if !Path::new(root).join("index.html").is_file() {
                return Err(AppError::validation(format!("{root} has no index.html; choose the folder that holds the web interface, or leave it empty")));
            }
        }
        Ok(())
    }
}

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedStatus {
    pub running: bool,
    /// Where it answers (`http://127.0.0.1:8200`), while running.
    pub url: Option<String>,
    pub settings: EmbeddedSettings,
    /// An account password is set, so it can be started.
    pub has_password: bool,
    /// The web interface files were found; without them only the API is served.
    pub web_interface: bool,
    pub started_at: Option<String>,
    /// Why it is not running when it stopped by itself or could not start.
    pub error: Option<String>,
}

struct Running {
    handle: tokio::task::JoinHandle<Result<(), AppError>>,
    life: Lifecycle,
    url: String,
    web_interface: bool,
    started_at: String,
    settings: EmbeddedSettings,
}

struct Inner {
    repo: Repository,
    mgr: BackupManager,
    conns: ConnectionManager,
    store: SettingsStore,
    running: Mutex<Option<Running>>,
    last_error: Mutex<Option<String>>,
}

/// The managed state (`tauri::State<EmbeddedServer>`); cheap to clone.
#[derive(Clone)]
pub struct EmbeddedServer(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where a web interface may be found next to the running app.
fn web_root_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(env) = std::env::var_os("UFOP_SERVER_WEBROOT").filter(|p| !p.is_empty()) {
        v.push(PathBuf::from(env));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("web"));
            v.push(dir.join("../Resources/web"));
            v.push(dir.join("../share/ufop/web"));
            v.push(dir.join("../lib/ufop/web"));
        }
    }
    v
}

/// The folder to serve: the configured one, else the first candidate with an `index.html`.
pub fn find_web_root(configured: Option<&str>, candidates: &[PathBuf]) -> Option<PathBuf> {
    let has_index = |p: &Path| p.join("index.html").is_file();
    if let Some(c) = configured.map(str::trim).filter(|c| !c.is_empty()) {
        return has_index(Path::new(c)).then(|| PathBuf::from(c));
    }
    candidates.iter().find(|c| has_index(c)).cloned()
}

impl EmbeddedServer {
    pub fn new(repo: Repository, mgr: BackupManager, conns: ConnectionManager, store: SettingsStore) -> Self {
        Self(Arc::new(Inner { repo, mgr, conns, store, running: Mutex::new(None), last_error: Mutex::new(None) }))
    }

    pub async fn settings(&self) -> EmbeddedSettings {
        match self.0.repo.get_config(SETTINGS_KEY).await {
            Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
            _ => EmbeddedSettings::default(),
        }
    }

    /// Save new settings. Refused while the server runs: they apply on the next start.
    pub async fn save_settings(&self, settings: EmbeddedSettings) -> Result<EmbeddedSettings, AppError> {
        if self.refresh().is_some() {
            return Err(AppError::state("Stop the web server before changing its settings", "The new settings apply the next time it starts."));
        }
        settings.validate()?;
        self.0.repo.set_config(SETTINGS_KEY, &serde_json::to_string(&settings)?).await?;
        Ok(settings)
    }

    fn auth(&self) -> AuthService {
        AuthService::new(self.0.store.clone())
    }

    /// Set the password of the server's `admin` account (the server's own code).
    pub async fn set_password(&self, password: &str) -> Result<(), AppError> {
        self.auth().set_password(password).await
    }

    /// The running server, after dropping one whose task ended by itself
    /// (its failure is kept for the status).
    fn refresh(&self) -> Option<(String, bool, String, EmbeddedSettings)> {
        let mut guard = lock(&self.0.running);
        if guard.as_ref().is_some_and(|r| r.handle.is_finished()) {
            if let Some(done) = guard.take() {
                let inner = self.0.clone();
                tokio::spawn(async move {
                    let why = match done.handle.await {
                        Ok(Err(e)) => Some(e.user_message()),
                        Err(e) => Some(format!("The web server stopped unexpectedly: {e}")),
                        Ok(Ok(())) => None,
                    };
                    if let Some(w) = why {
                        *lock(&inner.last_error) = Some(w);
                    }
                });
            }
        }
        guard.as_ref().map(|r| (r.url.clone(), r.web_interface, r.started_at.clone(), r.settings.clone()))
    }

    pub async fn status(&self) -> EmbeddedStatus {
        let running = self.refresh();
        let settings = running.as_ref().map(|r| r.3.clone()).unwrap_or(self.settings().await);
        let has_password = self.auth().has_password().await.unwrap_or(false);
        let web_interface = running.as_ref().map(|r| r.1).unwrap_or_else(|| find_web_root(settings.web_root.as_deref(), &web_root_candidates()).is_some());
        EmbeddedStatus {
            running: running.is_some(),
            url: running.as_ref().map(|r| r.0.clone()),
            started_at: running.as_ref().map(|r| r.2.clone()),
            settings,
            has_password,
            web_interface,
            error: if running.is_some() { None } else { lock(&self.0.last_error).clone() },
        }
    }

    pub async fn start(&self) -> Result<EmbeddedStatus, AppError> {
        if self.refresh().is_some() {
            return Err(AppError::state("The web server is already running", "Stop it first to start it again."));
        }
        *lock(&self.0.last_error) = None;
        let settings = self.settings().await;
        settings.validate()?;
        // Never without a way to sign in.
        let auth = self.auth();
        if !auth.has_password().await? {
            return Err(AppError::state("Set a password for the web server first", "Choose a password of at least 8 characters, then start it."));
        }
        let web_root = find_web_root(settings.web_root.as_deref(), &web_root_candidates());
        let https = settings.uses_https();
        let cfg = ServerConfig {
            port: settings.port,
            interface: if settings.allow_network { "any".into() } else { "loopback".into() },
            // Other computers reach it by name or address: the Host check must let them in.
            allowed_hostnames: if settings.allow_network { vec!["*".into()] } else { Vec::new() },
            tls: if https { TlsMode::Generated { dir: crate::paths::data_dir().join("server-tls"), hostnames: Vec::new() } } else { TlsMode::None },
            webroot: web_root.clone(),
            api_only: web_root.is_none(),
            embedded: true,
            ..Default::default()
        };
        let addr = cfg.bind_addr()?;
        // A busy port is a clear message now, not an error in a background task.
        if let Err(e) = std::net::TcpListener::bind(addr) {
            return Err(AppError::state(format!("Port {} cannot be used: {e}", settings.port), "Choose another port, or close the program that uses it."));
        }
        let st = ServerState::new(self.0.mgr.clone(), Some(self.0.conns.clone()), auth, cfg);
        let life = st.life.clone();
        let handle = tokio::spawn(server::serve(st));
        // Ready when it accepts connections; a task that ends first has failed.
        let mut ready = false;
        for _ in 0..100 {
            if handle.is_finished() {
                break;
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", settings.port)).await.is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !ready {
            let reason = if handle.is_finished() {
                match handle.await {
                    Ok(Err(e)) => e.user_message(),
                    Ok(Ok(())) => "it stopped right away".to_string(),
                    Err(e) => e.to_string(),
                }
            } else {
                handle.abort();
                "it did not start listening in time".to_string()
            };
            let message = format!("The web server did not start: {reason}");
            *lock(&self.0.last_error) = Some(message.clone());
            return Err(AppError::state(message, "Check the port and the certificate settings."));
        }
        let scheme = if https { "https" } else { "http" };
        let host = if settings.allow_network { "localhost" } else { "127.0.0.1" };
        *lock(&self.0.running) = Some(Running {
            handle,
            life,
            url: format!("{scheme}://{host}:{}", settings.port),
            web_interface: web_root.is_some(),
            started_at: chrono::Utc::now().to_rfc3339(),
            settings,
        });
        Ok(self.status().await)
    }

    pub async fn stop(&self) -> Result<EmbeddedStatus, AppError> {
        let taken = lock(&self.0.running).take();
        if let Some(r) = taken {
            // A short grace: open requests finish; nothing of the app's own work is touched.
            r.life.request(Some(Duration::from_secs(2)));
            if tokio::time::timeout(Duration::from_secs(15), r.handle).await.is_err() {
                tracing::warn!("The web server did not stop in time");
            }
        }
        Ok(self.status().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::CredentialStore;
    use serde_json::{json, Value};
    use ufop_core::backup::crypto::KdfParams;

    async fn server(dir: &Path) -> EmbeddedServer {
        let repo = Repository::open_in_memory().await.unwrap();
        let creds = Arc::new(CredentialStore::new());
        let mgr = BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, dir.join("dbs"), KdfParams::insecure_fast());
        let conns = ConnectionManager::new(repo.clone(), creds);
        EmbeddedServer::new(repo.clone(), mgr, conns, SettingsStore::plain(repo.pool().clone()))
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    async fn configured(dir: &Path) -> (EmbeddedServer, u16) {
        let s = server(dir).await;
        let port = free_port();
        s.save_settings(EmbeddedSettings { port, ..Default::default() }).await.unwrap();
        (s, port)
    }

    #[test]
    fn settings_are_checked_before_they_are_saved() {
        assert!(EmbeddedSettings { port: 80, ..Default::default() }.validate().unwrap_err().user_message().contains("1024"));
        let d = tempfile::tempdir().unwrap();
        let bad = EmbeddedSettings { web_root: Some(d.path().to_string_lossy().into()), ..Default::default() };
        assert!(bad.validate().unwrap_err().user_message().contains("index.html"));
        std::fs::write(d.path().join("index.html"), "<html></html>").unwrap();
        assert!(bad.validate().is_ok());
        assert!(EmbeddedSettings::default().validate().is_ok());
        assert!(!EmbeddedSettings::default().uses_https());
        assert!(EmbeddedSettings { allow_network: true, ..Default::default() }.uses_https(), "other computers always get HTTPS");
    }

    #[test]
    fn the_web_interface_is_the_chosen_folder_or_the_first_that_has_one() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::write(b.path().join("index.html"), "x").unwrap();
        let cands = vec![a.path().to_path_buf(), b.path().to_path_buf()];
        assert_eq!(find_web_root(None, &cands), Some(b.path().to_path_buf()));
        assert_eq!(find_web_root(Some(""), &cands), Some(b.path().to_path_buf()));
        assert_eq!(find_web_root(Some(&b.path().to_string_lossy()), &[]), Some(b.path().to_path_buf()));
        assert_eq!(find_web_root(Some(&a.path().to_string_lossy()), &cands), None, "a chosen folder without a web interface is not silently replaced");
        assert_eq!(find_web_root(None, &[]), None);
    }

    #[tokio::test]
    async fn it_will_not_start_without_a_password() {
        let d = tempfile::tempdir().unwrap();
        let (s, _) = configured(d.path()).await;
        assert!(!s.status().await.has_password);
        let err = s.start().await.unwrap_err();
        assert!(err.user_message().contains("password"), "{}", err.user_message());
        assert!(!s.status().await.running);
        // Too short a password is refused by the server's own rule.
        assert!(s.set_password("short").await.unwrap_err().user_message().contains("8 characters"));
        assert!(!s.status().await.has_password);
    }

    #[tokio::test]
    async fn it_serves_with_the_servers_login_and_stops_without_touching_the_apps_backups() {
        let d = tempfile::tempdir().unwrap();
        let (s, port) = configured(d.path()).await;
        s.set_password("correct horse battery").await.unwrap();
        assert!(s.status().await.has_password);

        let st = s.start().await.unwrap();
        assert!(st.running && st.error.is_none());
        let base = format!("http://127.0.0.1:{port}");
        assert_eq!(st.url.as_deref(), Some(base.as_str()));
        let http = reqwest::Client::new();
        assert_eq!(http.get(format!("{base}/api/v1/health")).send().await.unwrap().status(), 200);
        // Everything but health needs the password.
        assert_eq!(http.get(format!("{base}/api/v1/jobs")).send().await.unwrap().status(), 401);
        let wrong = http.post(format!("{base}/api/v1/auth/login")).json(&json!({ "password": "wrong password" })).send().await.unwrap();
        assert_eq!(wrong.status(), 401);
        let ok = http.post(format!("{base}/api/v1/auth/login")).json(&json!({ "password": "correct horse battery" })).send().await.unwrap();
        assert_eq!(ok.status(), 200);
        let token = ok.json::<Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
        assert_eq!(http.get(format!("{base}/api/v1/jobs")).bearer_auth(&token).send().await.unwrap().status(), 200);

        // Settings are fixed while it runs, and it cannot be started twice.
        assert!(s.save_settings(EmbeddedSettings { port: free_port(), ..Default::default() }).await.is_err());
        assert!(s.start().await.unwrap_err().user_message().contains("already running"));

        let st = s.stop().await.unwrap();
        assert!(!st.running && st.url.is_none());
        assert!(http.get(format!("{base}/api/v1/health")).send().await.is_err(), "nothing listens any more");
        assert!(!s.0.mgr.is_shutting_down(), "the app's backup manager was left alone");

        // And it can start again, on the same port.
        assert!(s.start().await.unwrap().running);
        assert_eq!(http.get(format!("{base}/api/v1/health")).send().await.unwrap().status(), 200);
        s.stop().await.unwrap();
    }

    #[tokio::test]
    async fn a_busy_port_is_reported_clearly_and_nothing_is_left_running() {
        let d = tempfile::tempdir().unwrap();
        let s = server(d.path()).await;
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        s.save_settings(EmbeddedSettings { port, ..Default::default() }).await.unwrap();
        s.set_password("correct horse battery").await.unwrap();
        let err = s.start().await.unwrap_err();
        assert!(err.user_message().contains(&format!("Port {port}")), "{}", err.user_message());
        assert!(!s.status().await.running);
    }

    #[tokio::test]
    async fn the_web_interface_is_served_when_there_is_one() {
        let d = tempfile::tempdir().unwrap();
        let web = tempfile::tempdir().unwrap();
        std::fs::write(web.path().join("index.html"), "<html><head></head><body>Hello UFOP</body></html>").unwrap();
        let s = server(d.path()).await;
        let port = free_port();
        s.save_settings(EmbeddedSettings { port, web_root: Some(web.path().to_string_lossy().into()), ..Default::default() }).await.unwrap();
        s.set_password("correct horse battery").await.unwrap();
        let st = s.start().await.unwrap();
        assert!(st.web_interface);
        let page = reqwest::get(format!("http://127.0.0.1:{port}/")).await.unwrap().text().await.unwrap();
        assert!(page.contains("Hello UFOP") && page.contains("__UFOP_SERVER__"), "{page}");
        s.stop().await.unwrap();

        // Without one only the API answers, and the status says so.
        let (api_only, port) = configured(d.path()).await;
        api_only.set_password("correct horse battery").await.unwrap();
        let st = api_only.start().await.unwrap();
        assert!(!st.web_interface || find_web_root(None, &web_root_candidates()).is_some());
        if !st.web_interface {
            assert_eq!(reqwest::get(format!("http://127.0.0.1:{port}/")).await.unwrap().status(), 404);
        }
        api_only.stop().await.unwrap();
    }

    #[tokio::test]
    async fn settings_and_password_are_kept_when_the_app_is_restarted() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::open_in_memory().await.unwrap();
        let creds = Arc::new(CredentialStore::new());
        let open = || {
            let mgr = BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, d.path().join("dbs"), KdfParams::insecure_fast());
            let conns = ConnectionManager::new(repo.clone(), creds.clone());
            EmbeddedServer::new(repo.clone(), mgr, conns, SettingsStore::plain(repo.pool().clone()))
        };
        let first = open();
        let port = free_port();
        first.save_settings(EmbeddedSettings { port, https: true, ..Default::default() }).await.unwrap();
        first.set_password("correct horse battery").await.unwrap();
        // A new instance over the same database is what the next app start sees.
        let second = open();
        assert_eq!(second.settings().await, EmbeddedSettings { port, https: true, ..Default::default() });
        assert!(second.status().await.has_password);
        assert!(!second.status().await.running, "it is never started by itself");
    }
}
