//! The agent: enrollment, the heartbeat loop, command execution, policy
//! sync and the offline report queue. What the agent manages (jobs,
//! secrets, persistence) is supplied by an [`AgentHost`]: the desktop app
//! and `ufop-server` back it with the backup manager, the CLI with its own
//! small job store.

use super::policy;
use super::managed::ManagedSettings;
use super::protocol::*;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Remote client calls the agent executes (the console's `RPC_ALLOW` list):
/// job management, runs and maintenance. Calls that read secrets back or
/// touch arbitrary local files are not on it, whatever the console sends.
pub const RPC_ALLOW: &[&str] = &[
    "backup_list_jobs", "backup_get_job", "backup_save_job", "backup_delete_job", "backup_test_destination", "backup_run", "backup_execute",
    "backup_cancel", "backup_status", "backup_pause", "backup_resume", "backup_list_versions", "backup_list_files", "backup_search", "backup_compare",
    "backup_run_history", "backup_export_cli", "backup_capabilities", "backup_preview_sources", "backup_effective_job", "backup_list_destinations",
    "backup_database_inventory", "backup_diagnose", "backup_job_placeholders", "backup_test_secret_providers", "backup_start_operation",
    "backup_cloud_providers", "backup_remote_list", "backup_get_global_defaults", "backup_set_global_defaults", "backup_test_notification",
];

/// Reports kept while the console is unreachable (oldest dropped first).
pub const MAX_PENDING_REPORTS: usize = 100;
/// Key of the managed app settings in applied/error reports.
pub const MANAGED_SETTINGS_KEY: &str = "managed_settings";
const MIN_HEARTBEAT: u64 = 15;
const MAX_HEARTBEAT: u64 = 3600;
const MAX_BACKOFF: u64 = 300;

pub const ENV_URL: &str = "UFOP_CONSOLE_URL";
pub const ENV_TOKEN: &str = "UFOP_CONSOLE_REGISTRATION_TOKEN";
pub const ENV_TAGS: &str = "UFOP_CONSOLE_TAGS";
/// Heartbeat interval in seconds (15–3600; default: the console's, 60).
pub const ENV_HEARTBEAT: &str = "UFOP_CONSOLE_HEARTBEAT_SECONDS";

/// Persisted agent settings (no secrets: the token is stored separately).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub console_url: String,
    pub machine_id: String,
    pub organization: String,
    /// Heartbeats and command execution on/off (registration is kept).
    pub enabled: bool,
    pub heartbeat_seconds: u64,
    /// Interval chosen on this machine (sent with heartbeats); `None` = the console's.
    pub heartbeat_override: Option<u64>,
    pub tags: Vec<String>,
    pub registered_at: Option<String>,
    /// `link` or `claim`.
    pub registered_via: String,
    /// As the console last reported it: `active`, `suspended` or `revoked`.
    pub machine_status: String,
    pub policy_set_version: String,
    pub applied_policies: BTreeMap<String, u64>,
    pub policy_errors: BTreeMap<String, String>,
    /// Central credentials currently in the secure store (`central:<id>`).
    pub central_credentials: Vec<String>,
    pub last_heartbeat_at: Option<String>,
    pub last_error: Option<String>,
    pub pending_reports: Vec<Value>,
    pub dropped_reports: u64,
    /// Set by an explicit unregister: preload/env auto-registration stays off.
    pub auto_register_declined: bool,
    /// Version of the managed app settings in force (0 = none).
    pub managed_settings_version: u64,
}

impl AgentConfig {
    pub fn is_registered(&self) -> bool {
        !self.machine_id.is_empty() && !self.console_url.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimStatus {
    pub console_url: String,
    pub code: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedPolicy {
    pub policy_id: String,
    pub job_id: String,
    pub version: u64,
    pub error: Option<String>,
}

/// What the UI and `ufop agent status` show.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub registered: bool,
    pub enabled: bool,
    pub console_url: String,
    pub machine_id: String,
    pub organization: String,
    pub machine_status: String,
    pub heartbeat_seconds: u64,
    pub tags: Vec<String>,
    pub registered_at: Option<String>,
    pub registered_via: String,
    pub last_heartbeat_at: Option<String>,
    pub last_error: Option<String>,
    pub policy_set_version: String,
    pub managed_policies: Vec<ManagedPolicy>,
    pub pending_reports: usize,
    pub dropped_reports: u64,
    pub claim: Option<ClaimStatus>,
}

/// Outcome of one claim poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimPollResult {
    /// `pending`, `registered`, `expired` or `none` (no claim in progress).
    pub state: String,
    pub status: AgentStatus,
}

/// Outcome of a policy sync.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicySync {
    pub policy_set_version: String,
    pub applied: BTreeMap<String, u64>,
    pub errors: BTreeMap<String, String>,
    pub removed: Vec<String>,
}

/// What the agent drives on this machine.
pub trait AgentHost: Send + Sync + 'static {
    fn client_info(&self) -> ClientInfo;
    /// Saved configuration and agent token.
    fn load(&self) -> BoxFuture<'_, Result<(AgentConfig, Option<String>), AppError>>;
    fn save_config(&self, cfg: AgentConfig) -> BoxFuture<'_, Result<(), AppError>>;
    /// Store (`Some`) or delete (`None`) the agent token in the secure store.
    fn save_token(&self, token: Option<String>) -> BoxFuture<'_, Result<(), AppError>>;
    fn jobs(&self) -> BoxFuture<'_, Vec<JobSummary>>;
    /// Execute an allow-listed client call (`backup_*`) with its JSON args.
    fn rpc(&self, command: String, args: Value) -> BoxFuture<'_, Result<Value, AppError>>;
    /// Start a backup of a job now.
    fn run_backup(&self, job_id: String) -> BoxFuture<'_, Result<Value, AppError>>;
    /// Store (`Some`) or remove (`None`) a central credential (`central:<id>`) in the secure store.
    fn store_credential(&self, credential_id: String, secret: Option<String>) -> BoxFuture<'_, Result<(), AppError>>;
    /// Create or update the managed job `policy-<id>` from a policy.
    fn apply_policy(&self, policy: FleetPolicy) -> BoxFuture<'_, Result<(), AppError>>;
    /// Remove the managed job of a policy that no longer applies.
    fn remove_policy(&self, policy_id: String) -> BoxFuture<'_, Result<(), AppError>>;
    /// Policy ids that have a managed job here.
    fn managed_policies(&self) -> BoxFuture<'_, Vec<String>>;
    /// Build a diagnostics bundle (logs, versions, OS; no secrets). Hosts
    /// without one keep the default.
    /// Apply the organization's app settings (`None`: release them to the user).
    fn apply_managed_settings(&self, _settings: Option<ManagedSettings>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async { Ok(()) })
    }
    fn diagnostics(&self) -> BoxFuture<'_, Result<DiagnosticsUpload, AppError>> {
        Box::pin(async { Err(AppError::state("Diagnostics are not available on this client", "Collect logs on the machine instead.")) })
    }
}

struct PendingClaim {
    console_url: String,
    claim_id: String,
    secret: String,
    code: String,
    expires_at: String,
}

pub struct Agent<H: AgentHost> {
    host: H,
    http: reqwest::Client,
    cfg: Mutex<AgentConfig>,
    token: Mutex<Option<String>>,
    claim: Mutex<Option<PendingClaim>>,
    wake: tokio::sync::Notify,
    cycle_lock: tokio::sync::Mutex<()>,
    policies_fetched: AtomicBool,
    failures: AtomicU32,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn err_json(e: &AppError) -> Value {
    json!({ "error": e.user_message() })
}

/// Comma/space separated tags → normalized list.
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split([',', ' ']).map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

impl<H: AgentHost> Agent<H> {
    /// Load the saved state through `host`.
    pub async fn load(host: H, http: reqwest::Client) -> Result<Arc<Self>, AppError> {
        let (cfg, token) = host.load().await?;
        Ok(Self::new(host, http, cfg, token))
    }

    /// An agent with the given state (when it cannot be loaded: `AgentConfig::default()`).
    pub fn new(host: H, http: reqwest::Client, cfg: AgentConfig, token: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            host,
            http,
            cfg: Mutex::new(cfg),
            token: Mutex::new(token.filter(|t| !t.is_empty())),
            claim: Mutex::new(None),
            wake: tokio::sync::Notify::new(),
            cycle_lock: tokio::sync::Mutex::new(()),
            policies_fetched: AtomicBool::new(false),
            failures: AtomicU32::new(0),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_set_url(&self, url: &str) {
        self.update(|c| c.console_url = url.to_string());
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn config(&self) -> AgentConfig {
        lock(&self.cfg).clone()
    }

    fn token(&self) -> Option<String> {
        lock(&self.token).clone()
    }

    fn client(&self, url: &str) -> Result<AgentClient, AppError> {
        AgentClient::new(url, self.http.clone())
    }

    async fn persist(&self) {
        let c = self.config();
        if let Err(e) = self.host.save_config(c).await {
            tracing::warn!("Could not save the console agent settings: {}", e.user_message());
        }
    }

    fn update(&self, f: impl FnOnce(&mut AgentConfig)) {
        f(&mut lock(&self.cfg));
    }

    pub fn status(&self) -> AgentStatus {
        let c = self.config();
        let mut managed: Vec<ManagedPolicy> = c
            .applied_policies
            .iter()
            .map(|(id, v)| ManagedPolicy { policy_id: id.clone(), job_id: format!("{}{id}", policy::MANAGED_PREFIX), version: *v, error: c.policy_errors.get(id).cloned() })
            .collect();
        for (id, e) in c.policy_errors.iter().filter(|(id, _)| !c.applied_policies.contains_key(*id)) {
            managed.push(ManagedPolicy { policy_id: id.clone(), job_id: format!("{}{id}", policy::MANAGED_PREFIX), version: 0, error: Some(e.clone()) });
        }
        let claim = lock(&self.claim).as_ref().map(|p| ClaimStatus { console_url: p.console_url.clone(), code: p.code.clone(), expires_at: p.expires_at.clone() });
        AgentStatus {
            registered: c.is_registered() && self.token().is_some(),
            enabled: c.enabled,
            console_url: c.console_url,
            machine_id: c.machine_id,
            organization: c.organization,
            machine_status: c.machine_status,
            heartbeat_seconds: c.heartbeat_seconds,
            tags: c.tags,
            registered_at: c.registered_at,
            registered_via: c.registered_via,
            last_heartbeat_at: c.last_heartbeat_at,
            last_error: c.last_error,
            policy_set_version: c.policy_set_version,
            managed_policies: managed,
            pending_reports: c.pending_reports.len(),
            dropped_reports: c.dropped_reports,
            claim,
        }
    }

    fn ensure_unregistered(&self) -> Result<(), AppError> {
        if self.config().is_registered() && self.token().is_some() {
            return Err(AppError::state("This machine is already registered with a console", "Unregister it first to register with another console."));
        }
        Ok(())
    }

    async fn finish_enrollment(&self, url: &str, reg: Registration, tags: Vec<String>, via: &str) -> Result<AgentStatus, AppError> {
        let Registration { machine_id, agent_token: token, organization, heartbeat_seconds: heartbeat } = reg;
        self.host.save_token(Some(token.clone())).await?;
        *lock(&self.token) = Some(token);
        self.update(|c| {
            let keep = c.heartbeat_override;
            *c = AgentConfig {
                heartbeat_override: keep,
                console_url: url.to_string(),
                machine_id,
                organization,
                enabled: true,
                heartbeat_seconds: keep.unwrap_or(heartbeat).clamp(MIN_HEARTBEAT, MAX_HEARTBEAT),
                tags,
                registered_at: Some(now()),
                registered_via: via.into(),
                machine_status: "active".into(),
                ..Default::default()
            }
        });
        self.policies_fetched.store(false, Ordering::SeqCst);
        self.persist().await;
        self.wake.notify_one();
        Ok(self.status())
    }

    /// Enroll with a registration link token (`ufop_reg_…`).
    pub async fn register(&self, console_url: &str, registration_token: &str, tags: Vec<String>) -> Result<AgentStatus, AppError> {
        self.ensure_unregistered()?;
        let token = registration_token.trim();
        if !token.starts_with("ufop_reg_") {
            return Err(AppError::validation("A registration token starts with ufop_reg_ (copy it from the console's registration link)"));
        }
        let client = self.client(console_url)?;
        let r = client.register(token, &self.host.client_info(), &tags).await?;
        *lock(&self.claim) = None;
        self.finish_enrollment(client.base_url(), r, tags, "link").await
    }

    /// The console URL a browser sign-in starts at (`/fleet/connect`), for the
    /// loopback `redirect_uri`, `state` and PKCE S256 `code_challenge`.
    pub fn connect_url(&self, console_url: &str, redirect_uri: &str, state: &str, code_challenge: &str) -> Result<String, AppError> {
        self.ensure_unregistered()?;
        let base = normalize_console_url(console_url)?;
        let info = self.host.client_info();
        let mut u = reqwest::Url::parse(&format!("{base}/fleet/connect")).map_err(|e| AppError::validation(format!("Invalid console URL: {e}")))?;
        u.query_pairs_mut()
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("hostname", &info.hostname)
            .append_pair("name", info.name.as_deref().unwrap_or(&info.hostname));
        Ok(u.to_string())
    }

    /// Finish a browser sign-in: redeem the code from the loopback redirect.
    pub async fn register_with_code(&self, console_url: &str, code: &str, code_verifier: &str, redirect_uri: &str, tags: Vec<String>) -> Result<AgentStatus, AppError> {
        self.ensure_unregistered()?;
        let client = self.client(console_url)?;
        let r = client.connect_token(code, code_verifier, redirect_uri, &self.host.client_info(), &tags).await?;
        *lock(&self.claim) = None;
        self.finish_enrollment(client.base_url(), r, tags, "sso").await
    }

    /// Start interactive claiming: show the returned code to the user.
    pub async fn start_claim(&self, console_url: &str) -> Result<ClaimStatus, AppError> {
        self.ensure_unregistered()?;
        let client = self.client(console_url)?;
        let c = client.start_claim(&self.host.client_info()).await?;
        let st = ClaimStatus { console_url: client.base_url().to_string(), code: c.code.clone(), expires_at: c.expires_at.clone() };
        *lock(&self.claim) = Some(PendingClaim { console_url: st.console_url.clone(), claim_id: c.claim_id, secret: c.secret, code: c.code, expires_at: c.expires_at });
        Ok(st)
    }

    /// Check whether an administrator entered the claim code.
    pub async fn poll_claim(&self) -> Result<ClaimPollResult, AppError> {
        let pending = lock(&self.claim).as_ref().map(|p| (p.console_url.clone(), p.claim_id.clone(), p.secret.clone()));
        let Some((url, id, secret)) = pending else {
            return Ok(ClaimPollResult { state: "none".into(), status: self.status() });
        };
        let r = self.client(&url)?.poll_claim(&id, &secret).await?;
        match (r.status.as_str(), r.agent_token, r.machine_id) {
            ("collected", Some(token), Some(mid)) => {
                *lock(&self.claim) = None;
                let status = self.finish_enrollment(&url, Registration { machine_id: mid, agent_token: token, organization: String::new(), heartbeat_seconds: 60 }, Vec::new(), "claim").await?;
                Ok(ClaimPollResult { state: "registered".into(), status })
            }
            ("pending", _, _) => Ok(ClaimPollResult { state: "pending".into(), status: self.status() }),
            _ => {
                // Expired, or collected without a token (already used): start over.
                *lock(&self.claim) = None;
                Ok(ClaimPollResult { state: "expired".into(), status: self.status() })
            }
        }
    }

    pub fn cancel_claim(&self) {
        *lock(&self.claim) = None;
    }

    /// Leave the console. `force` also forgets the registration when the
    /// console cannot be reached. Managed jobs and central credentials are
    /// removed (the backup data at the destinations stays).
    pub async fn unregister(&self, force: bool) -> Result<AgentStatus, AppError> {
        let _g = self.cycle_lock.lock().await;
        let c = self.config();
        if let (true, Some(t)) = (c.is_registered(), self.token()) {
            match self.client(&c.console_url)?.unregister(&t).await {
                Ok(()) | Err(ConsoleError::Unauthorized(_)) => {}
                Err(e) if force => tracing::warn!("Unregister: {e}; forgetting the registration anyway"),
                Err(e) => return Err(e.into()),
            }
        }
        for id in self.host.managed_policies().await {
            if let Err(e) = self.host.remove_policy(id.clone()).await {
                tracing::warn!("Could not remove the managed job of policy {id}: {}", e.user_message());
            }
        }
        for cred in &c.central_credentials {
            let _ = self.host.store_credential(cred.clone(), None).await;
        }
        if c.managed_settings_version > 0 {
            if let Err(e) = self.host.apply_managed_settings(None).await {
                tracing::warn!("Could not release the managed app settings: {}", e.user_message());
            }
        }
        self.host.save_token(None).await?;
        *lock(&self.token) = None;
        self.update(|c| *c = AgentConfig { auto_register_declined: true, ..Default::default() });
        self.persist().await;
        Ok(self.status())
    }

    pub async fn set_enabled(&self, enabled: bool) -> AgentStatus {
        self.update(|c| c.enabled = enabled);
        self.persist().await;
        self.wake.notify_one();
        self.status()
    }

    /// Use this heartbeat interval (clamped to 15–3600 s; `None`: the console's).
    pub async fn set_heartbeat_seconds(&self, secs: Option<u64>) -> AgentStatus {
        let s = secs.map(|s| s.clamp(MIN_HEARTBEAT, MAX_HEARTBEAT));
        self.update(|c| {
            c.heartbeat_override = s;
            if let Some(s) = s {
                c.heartbeat_seconds = s;
            }
        });
        self.persist().await;
        self.wake.notify_one();
        self.status()
    }

    /// Replace the agent token (the old one stops working).
    pub async fn rotate_token(&self) -> Result<(), AppError> {
        let c = self.config();
        let t = self.token().ok_or_else(|| AppError::state("This machine is not registered", "Register it with a console first."))?;
        let new = self.client(&c.console_url)?.rotate(&t).await?;
        self.host.save_token(Some(new.clone())).await?;
        *lock(&self.token) = Some(new);
        Ok(())
    }

    /// Queue a run report; it is sent on the next cycle (now, when online).
    pub fn enqueue_report(&self, report: &RunReport) {
        let Ok(v) = serde_json::to_value(report) else { return };
        {
            let mut c = lock(&self.cfg);
            if !c.is_registered() || !c.enabled {
                return;
            }
            c.pending_reports.push(v);
            let over = c.pending_reports.len().saturating_sub(MAX_PENDING_REPORTS);
            if over > 0 {
                c.pending_reports.drain(..over);
                c.dropped_reports += over as u64;
            }
        }
        self.wake.notify_one();
    }

    /// Wake the loop for an immediate cycle.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    async fn flush_reports(&self, client: &AgentClient, token: &str) -> Result<(), ConsoleError> {
        loop {
            let Some(next) = self.config().pending_reports.first().cloned() else { return Ok(()) };
            match client.report(token, &next).await {
                Ok(()) => {}
                // A report the console cannot use is dropped, not retried forever.
                Err(ConsoleError::Rejected { status, message }) if (400..500).contains(&status) && status != 429 => {
                    tracing::warn!("Console rejected a run report: {message}");
                }
                Err(e) => {
                    self.persist().await;
                    return Err(e);
                }
            }
            self.update(|c| {
                if c.pending_reports.first() == Some(&next) {
                    c.pending_reports.remove(0);
                }
            });
            self.persist().await;
        }
    }

    async fn execute(&self, client: &AgentClient, token: &str, cmd: &AgentCommand) -> (bool, Value) {
        let p = &cmd.payload;
        let r: Result<Value, AppError> = match cmd.kind.as_str() {
            "rpc" => {
                let name = p.get("command").and_then(|c| c.as_str()).unwrap_or_default().to_string();
                if !RPC_ALLOW.contains(&name.as_str()) {
                    Err(AppError::PermissionDenied { message: format!("'{name}' may not be called remotely"), advice: "Only job management, run and maintenance calls are allowed.".into() })
                } else {
                    let args = match p.get("args") {
                        Some(v @ Value::Object(_)) => v.clone(),
                        _ => json!({}),
                    };
                    self.host.rpc(name, args).await
                }
            }
            "run_backup" => match p.get("jobId").and_then(|j| j.as_str()) {
                Some(id) if !id.is_empty() => self.host.run_backup(id.to_string()).await,
                _ => Err(AppError::validation("run_backup needs a jobId")),
            },
            "apply_policies" => self.sync_policies(client, token).await.and_then(|s| Ok(serde_json::to_value(s)?)),
            "collect_diagnostics" => self.upload_diagnostics(client, token).await.map(|id| json!({ "diagnosticsId": id })),
            other => Err(AppError::validation(format!("Unknown command kind '{other}'"))),
        };
        match r {
            Ok(v) => (true, v),
            Err(e) => (false, err_json(&e)),
        }
    }

    async fn upload_diagnostics(&self, client: &AgentClient, token: &str) -> Result<String, AppError> {
        let d = self.host.diagnostics().await?;
        Self::upload_bundle(client, token, &d).await
    }

    async fn upload_bundle(client: &AgentClient, token: &str, d: &DiagnosticsUpload) -> Result<String, AppError> {
        if d.bytes.len() > MAX_DIAGNOSTICS_BYTES {
            return Err(AppError::validation(format!("The bundle is {} bytes; the limit is {MAX_DIAGNOSTICS_BYTES}", d.bytes.len())));
        }
        Ok(client.diagnostics(token, d).await?)
    }

    /// Send a bundle the caller built (feedback with its optional diagnostics)
    /// to the console the way diagnostics go. Returns the console's id for it
    /// and the console's address; nothing is reported as sent unless the
    /// console accepted it.
    pub async fn send_bundle(&self, d: &DiagnosticsUpload) -> Result<(String, String), AppError> {
        let c = self.config();
        let t = self.token().filter(|_| c.is_registered()).ok_or_else(|| AppError::state("This machine is not registered", "Register it with a console first."))?;
        let id = Self::upload_bundle(&self.client(&c.console_url)?, &t, d).await?;
        Ok((id, c.console_url))
    }

    /// Build a diagnostics bundle and send it to the console ("Send diagnostics").
    pub async fn send_diagnostics(&self) -> Result<String, AppError> {
        let c = self.config();
        let t = self.token().filter(|_| c.is_registered()).ok_or_else(|| AppError::state("This machine is not registered", "Register it with a console first."))?;
        self.upload_diagnostics(&self.client(&c.console_url)?, &t).await
    }

    async fn sync_policies(&self, client: &AgentClient, token: &str) -> Result<PolicySync, AppError> {
        let bundle = client.policies(token).await?;
        let mut delivered = Vec::new();
        let mut errors = BTreeMap::new();
        // Versions of the credentials now in the secure store, reported to the
        // console as `credential:<id>` so it can follow a rotation.
        let mut credential_versions = BTreeMap::new();
        for c in bundle.credentials {
            let Some(id) = policy::central_credential_id(&c.credential_id) else {
                tracing::warn!("Ignoring a central credential with a malformed id");
                continue;
            };
            let key = format!("credential:{}", id.trim_start_matches(policy::CENTRAL_PREFIX));
            match self.host.store_credential(id.clone(), Some(c.secret)).await {
                Ok(()) => {
                    if c.version > 0 {
                        credential_versions.insert(key, c.version);
                    }
                    delivered.push(id)
                }
                Err(e) => {
                    tracing::warn!("Could not store central credential {id}: {}", e.user_message());
                    errors.insert(key, e.user_message());
                }
            }
        }
        let previous = self.config();
        for old in previous.central_credentials.iter().filter(|o| !delivered.contains(o)) {
            let _ = self.host.store_credential(old.clone(), None).await;
        }
        let managed = self.host.managed_policies().await;
        let mut applied = BTreeMap::new();
        for p in &bundle.policies {
            let unchanged = previous.applied_policies.get(&p.id) == Some(&p.version) && managed.contains(&p.id);
            if unchanged {
                applied.insert(p.id.clone(), p.version);
                continue;
            }
            match self.host.apply_policy(p.clone()).await {
                Ok(()) => {
                    applied.insert(p.id.clone(), p.version);
                }
                Err(e) => {
                    errors.insert(p.id.clone(), e.user_message());
                }
            }
        }
        let mut removed = Vec::new();
        for id in managed.iter().filter(|id| !bundle.policies.iter().any(|p| &p.id == *id)) {
            match self.host.remove_policy(id.clone()).await {
                Ok(()) => removed.push(id.clone()),
                Err(e) => tracing::warn!("Could not remove the managed job of policy {id}: {}", e.user_message()),
            }
        }
        let mut settings_version = previous.managed_settings_version;
        let wanted = bundle.managed_settings.as_ref().map(|s| s.version).unwrap_or(0);
        // Also once per start: hosts that keep no copy get them back.
        let first_sync = !self.policies_fetched.load(Ordering::SeqCst);
        if wanted != settings_version || (first_sync && wanted > 0) {
            let r = match &bundle.managed_settings {
                Some(s) => s.validate().map_err(AppError::validation),
                None => Ok(()),
            };
            match r {
                Ok(()) => match self.host.apply_managed_settings(bundle.managed_settings.clone()).await {
                    Ok(()) => settings_version = wanted,
                    Err(e) => {
                        errors.insert(MANAGED_SETTINGS_KEY.into(), e.user_message());
                    }
                },
                Err(e) => {
                    errors.insert(MANAGED_SETTINGS_KEY.into(), e.user_message());
                }
            }
        }
        // The console sees the settings version next to the policies.
        let mut reported = applied.clone();
        reported.extend(credential_versions);
        if settings_version > 0 {
            reported.insert(MANAGED_SETTINGS_KEY.into(), settings_version);
        }
        if let Err(e) = client.policies_applied(token, &reported, &errors).await {
            tracing::warn!("Could not report applied policies: {e}");
        }
        self.update(|c| {
            c.policy_set_version = bundle.policy_set_version.clone();
            c.applied_policies = applied.clone();
            c.policy_errors = errors.clone();
            c.central_credentials = delivered;
            c.managed_settings_version = settings_version;
        });
        self.policies_fetched.store(true, Ordering::SeqCst);
        self.persist().await;
        Ok(PolicySync { policy_set_version: bundle.policy_set_version, applied, errors, removed })
    }

    /// One round: send queued reports, heartbeat, run commands, sync policies.
    pub async fn cycle(&self) -> Result<(), AppError> {
        let _g = self.cycle_lock.lock().await;
        let c = self.config();
        let Some(token) = self.token() else { return Ok(()) };
        if !c.is_registered() || !c.enabled {
            return Ok(());
        }
        let client = self.client(&c.console_url)?;
        let r = async {
            self.flush_reports(&client, &token).await?;
            let jobs = self.host.jobs().await;
            client.heartbeat(&token, &self.host.client_info(), &jobs, &c.policy_set_version, c.heartbeat_override).await
        }
        .await;
        let hb = match r {
            Ok(hb) => hb,
            Err(e) => {
                self.update(|c| {
                    c.last_error = Some(e.to_string());
                    match &e {
                        ConsoleError::Unauthorized(_) => c.machine_status = "revoked".into(),
                        ConsoleError::Suspended(_) => c.machine_status = "suspended".into(),
                        _ => {}
                    }
                });
                self.persist().await;
                return Err(e.into());
            }
        };
        self.update(|c| {
            c.last_heartbeat_at = Some(now());
            c.last_error = None;
            if !hb.status.is_empty() {
                c.machine_status = hb.status.clone();
            }
            if hb.heartbeat_seconds > 0 && c.heartbeat_override.is_none() {
                c.heartbeat_seconds = hb.heartbeat_seconds.clamp(MIN_HEARTBEAT, MAX_HEARTBEAT);
            }
        });
        // Before the commands, so they see the current managed jobs. Central
        // credentials live only in the secure store; after a restart a host
        // without one (the CLI) needs them again, so sync once per run.
        if hb.policies_changed || !self.policies_fetched.load(Ordering::SeqCst) {
            if let Err(e) = self.sync_policies(&client, &token).await {
                self.update(|c| c.last_error = Some(e.user_message()));
            }
        }
        let mut token = token;
        for cmd in &hb.commands {
            if cmd.kind == "rotate_token" {
                // Complete with the new token: the old one stops working at once.
                let (ok, result) = match self.rotate_token().await {
                    Ok(()) => (true, json!({ "rotated": true })),
                    Err(e) => (false, err_json(&e)),
                };
                token = self.token().unwrap_or(token);
                if let Err(e) = client.complete_command(&token, &cmd.id, ok, &result).await {
                    tracing::warn!("Could not report command {}: {e}", cmd.id);
                }
                continue;
            }
            let (ok, result) = self.execute(&client, &token, cmd).await;
            if let Err(e) = client.complete_command(&token, &cmd.id, ok, &result).await {
                tracing::warn!("Could not report command {}: {e}", cmd.id);
            }
        }
        self.persist().await;
        Ok(())
    }

    /// Run a cycle now and return the new status.
    pub async fn sync_now(&self) -> Result<AgentStatus, AppError> {
        let c = self.config();
        if !c.is_registered() || self.token().is_none() {
            return Err(AppError::state("This machine is not registered with a console", "Register it with a registration link or a claim code."));
        }
        if !c.enabled {
            return Err(AppError::state("The console agent is turned off", "Turn it on to sync with the console."));
        }
        self.cycle().await?;
        Ok(self.status())
    }

    /// Register from `UFOP_CONSOLE_URL` / `UFOP_CONSOLE_REGISTRATION_TOKEN`
    /// (`UFOP_CONSOLE_TAGS`) — usually set by the preload file — when not
    /// registered and not explicitly unregistered. `Ok(true)` when it did.
    pub async fn auto_register(&self, env: impl Fn(&str) -> Option<String>) -> Result<bool, AppError> {
        let c = self.config();
        if (c.is_registered() && self.token().is_some()) || c.auto_register_declined {
            return Ok(false);
        }
        let (Some(url), Some(token)) = (env(ENV_URL).filter(|s| !s.trim().is_empty()), env(ENV_TOKEN).filter(|s| !s.trim().is_empty())) else {
            return Ok(false);
        };
        let tags = env(ENV_TAGS).map(|t| parse_tags(&t)).unwrap_or_default();
        self.register(&url, &token, tags).await?;
        tracing::info!("Registered with the management console at {url}");
        Ok(true)
    }

    fn next_wait(&self, ok: bool) -> Duration {
        let hb = self.config().heartbeat_seconds;
        let hb = if hb == 0 { 60 } else { hb.clamp(MIN_HEARTBEAT, MAX_HEARTBEAT) };
        if ok {
            self.failures.store(0, Ordering::SeqCst);
            return Duration::from_secs(hb);
        }
        let n = self.failures.fetch_add(1, Ordering::SeqCst).min(5);
        Duration::from_secs((15u64 << n).min(MAX_BACKOFF).min(hb.max(15) * 4))
    }

    /// The background loop: a cycle every heartbeat interval (backing off
    /// while the console is unreachable), or at once when woken.
    pub async fn run_loop(self: Arc<Self>) {
        loop {
            let active = {
                let c = self.config();
                c.is_registered() && c.enabled && self.token().is_some() && c.machine_status != "revoked"
            };
            let wait = if active {
                match self.cycle().await {
                    Ok(()) => self.next_wait(true),
                    Err(e) => {
                        tracing::debug!("Console agent cycle failed: {}", e.user_message());
                        self.next_wait(false)
                    }
                }
            } else {
                Duration::from_secs(300)
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.wake.notified() => {}
            }
        }
    }
}
