//! Fleet agent for the desktop app and `ufop-server`: connects this
//! machine to a UFOP management console (the `admin/` app).
//!
//! The protocol client and the heartbeat / command / policy logic are in
//! `ufop_core::agent` (shared with the `ufop` CLI). This module is the
//! [`AgentHost`] backed by the [`BackupManager`]:
//!
//! - settings in the settings store (`fleet_agent.config`); the agent token
//!   in the OS keychain, or the encrypted settings store when there is none
//!   (the same rule as backup passphrases);
//! - remote `rpc` commands go through `backup_engine::api::dispatch`
//!   (only the allow-listed `backup_*` names);
//! - fleet policies become managed jobs `policy-<id>`; central credentials
//!   are stored as destination secrets `central:<id>` and referenced from
//!   jobs as `$central:<id>` placeholders;
//! - every finished backup operation is forwarded as a run report, queued
//!   while the console is unreachable.

use crate::backup_engine::{self, api, BackupJob, BackupManager, RunEvent};
use crate::core::error::AppError;
use crate::security::settings_store::SettingsStore;
use crate::security::CredentialStore;
use serde_json::{json, Value};
use std::sync::Arc;
use ufop_core::agent::policy::{self, managed_job_id, policy_of_job};
use ufop_core::agent::{Agent, AgentConfig, AgentHost, AgentLimits, BoxFuture, ClientInfo, DiagnosticsUpload, ManagedSettings, FleetPolicy, JobSummary, ReportStats, RunReport};

pub const CONFIG_KEY: &str = "fleet_agent.config";
pub const TOKEN_KEY: &str = "fleet-agent-token";
/// The owner's limits on the console (never sent to or changed by it).
pub const LIMITS_KEY: &str = "fleet_agent.limits";
/// A root nothing is inside, used when the saved limits cannot be read.
const UNREADABLE_ROOT: &str = "/ufop-limits-unreadable";
/// PEM file with an extra CA certificate for a console behind a private CA.
pub const ENV_CA_FILE: &str = "UFOP_CONSOLE_CA_FILE";

/// The managed agent instance (`tauri::State<FleetAgent>`).
pub type FleetAgent = Arc<Agent<BackupHost>>;

pub struct BackupHost {
    mgr: BackupManager,
    settings: SettingsStore,
    creds: Arc<CredentialStore>,
    agent_kind: &'static str,
    managed_handler: std::sync::Mutex<Option<ManagedSettingsHandler>>,
}

/// Applies the console's app settings on this kind of client (the desktop
/// app sets one; the server and CLI have no proxy or AI settings to manage).
pub type ManagedSettingsHandler = Arc<dyn Fn(Option<ManagedSettings>) -> BoxFuture<'static, Result<(), AppError>> + Send + Sync>;

impl BackupHost {
    pub fn new(mgr: BackupManager, settings: SettingsStore, creds: Arc<CredentialStore>, agent_kind: &'static str) -> Self {
        Self { mgr, settings, creds, agent_kind, managed_handler: std::sync::Mutex::new(None) }
    }

    /// The owner's limits on what the console may do here (local only).
    pub async fn limits(&self) -> AgentLimits {
        match self.settings.get(LIMITS_KEY).await {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_else(|e| {
                // Unreadable limits must not silently lift them: confine to nothing.
                tracing::warn!("Unreadable console limits, refusing console changes: {e}");
                AgentLimits { roots: vec![UNREADABLE_ROOT.into()], read_only: true, allow_scripts: false, ..Default::default() }
            }),
            _ => AgentLimits::default(),
        }
    }

    pub async fn set_limits(&self, limits: &AgentLimits) -> Result<(), AppError> {
        limits.validate().map_err(AppError::validation)?;
        self.settings.set(LIMITS_KEY, &serde_json::to_string(limits)?).await
    }

    pub fn set_managed_settings_handler(&self, h: ManagedSettingsHandler) {
        *self.managed_handler.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
    }

    fn keychain(&self) -> bool {
        self.creds.is_keychain_available()
    }

    async fn job_for_policy(&self, p: &FleetPolicy) -> Result<(BackupJob, Option<String>), AppError> {
        let id = managed_job_id(&p.id)?;
        let t = policy::prepare_template(&p.job)?;
        let base = self.mgr.get_job(&id).await.unwrap_or_default();
        let mut v = serde_json::to_value(&base)?;
        policy::overlay(&mut v, &t.fields);
        v["id"] = json!(id);
        let mut job: BackupJob = serde_json::from_value(v).map_err(|e| AppError::validation(format!("Policy '{}' has an invalid job template: {e}", p.name)))?;
        if job.description.trim().is_empty() {
            job.description = format!("Managed by the management console (policy \"{}\")", p.name);
        }
        let pass = match (t.passphrase, job.encrypted) {
            (Some(p), _) => Some(p),
            (None, false) => Some(String::new()),
            (None, true) => None,
        };
        Ok((job, pass))
    }
}

/// HTTP client for the console; honours `UFOP_CONSOLE_CA_FILE`.
pub fn http_client() -> Result<reqwest::Client, AppError> {
    let ca_pem = match std::env::var(ENV_CA_FILE).ok().filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(std::fs::read_to_string(&p).map_err(|e| AppError::Configuration {
            message: format!("Cannot read {ENV_CA_FILE} ({p}): {e}"),
            advice: "Point it at a PEM certificate file, or unset it.".into(),
        })?),
        None => None,
    };
    backup_engine::stores::http::client(&backup_engine::stores::http::TlsOptions { ca_pem, ..Default::default() })
}

/// A variable from the environment, else from the machine-wide preload file.
pub fn env_or_preload(exe: &'static str) -> impl Fn(&str) -> Option<String> {
    let preload = ufop_core::preload::Preload::load().map(|p| p.env_for(exe)).unwrap_or_default();
    move |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).or_else(|| preload.get(k).cloned())
}

/// Load the agent and connect the backup manager's run reports to it.
pub async fn init(mgr: BackupManager, settings: SettingsStore, creds: Arc<CredentialStore>, agent_kind: &'static str) -> Result<FleetAgent, AppError> {
    let host = BackupHost::new(mgr.clone(), settings, creds, agent_kind);
    let agent = Agent::load(host, http_client()?).await?;
    attach(&mgr, &agent);
    Ok(agent)
}

/// Forward the backup manager's finished operations to the agent's report queue.
fn attach(mgr: &BackupManager, agent: &FleetAgent) {
    let weak = Arc::downgrade(agent);
    mgr.set_run_listener(Arc::new(move |ev: RunEvent| {
        if let Some(a) = weak.upgrade() {
            a.enqueue_report(&run_report(ev));
        }
    }));
}

/// Like [`init`], but never fails: when the saved settings cannot be read
/// the agent starts unregistered (and saves to the same store).
pub async fn init_or_default(mgr: BackupManager, settings: SettingsStore, creds: Arc<CredentialStore>, agent_kind: &'static str) -> FleetAgent {
    match init(mgr.clone(), settings.clone(), creds.clone(), agent_kind).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("Console agent settings unavailable, starting unregistered: {}", e.user_message());
            let agent = Agent::new(BackupHost::new(mgr.clone(), settings, creds, agent_kind), http_client().unwrap_or_default(), AgentConfig::default(), None);
            attach(&mgr, &agent);
            agent
        }
    }
}

/// Map a finished operation to the console's report format.
pub fn run_report(ev: RunEvent) -> RunReport {
    RunReport {
        backup_id: ev.job_id,
        name: ev.job_name,
        operation: ev.operation,
        result: ev.status,
        started_at: Some(ev.started_at),
        finished_at: Some(ev.finished_at),
        duration_ms: ev.duration_ms,
        message: ev.summary,
        data: ev.report,
        stats: ReportStats {
            version_count: ev.stats.version_count,
            backend_size: ev.stats.backend_size,
            source_size: ev.stats.source_size,
            schedule_interval_minutes: ev.schedule_interval_minutes,
        },
    }
}

/// Start the background loop (after auto-registration from the
/// environment / preload file, when configured). Runs on the current Tokio
/// runtime (`ufop-server`) or Tauri's (desktop `setup`).
pub fn spawn(agent: FleetAgent, exe: &'static str) {
    let env = env_or_preload(exe);
    let heartbeat = env(ufop_core::agent::ENV_HEARTBEAT).and_then(|s| s.trim().parse::<u64>().ok());
    let task = async move {
        if heartbeat.is_some() && heartbeat != agent.config().heartbeat_override {
            agent.set_heartbeat_seconds(heartbeat).await;
        }
        if let Err(e) = agent.auto_register(env).await {
            tracing::warn!("Automatic console registration failed: {}", e.user_message());
        }
        agent.run_loop().await;
    };
    match tokio::runtime::Handle::try_current() {
        Ok(h) => {
            h.spawn(task);
        }
        Err(_) => {
            tauri::async_runtime::spawn(task);
        }
    }
}

fn arg_str(a: &Value, k: &str) -> Result<String, AppError> {
    a.get(k).and_then(|v| v.as_str()).map(String::from).ok_or_else(|| AppError::validation(format!("Missing argument '{k}'")))
}

/// The `fleet_agent_*` commands by name with camelCase args (the server's
/// RPC endpoint; the desktop app uses the Tauri commands).
pub async fn dispatch(agent: &FleetAgent, command: &str, a: &Value) -> Result<Value, AppError> {
    let v = match command {
        "fleet_agent_status" => serde_json::to_value(agent.status())?,
        "fleet_agent_register" => {
            let tags = a.get("tags").and_then(|t| t.as_str()).map(ufop_core::agent::parse_tags).unwrap_or_default();
            serde_json::to_value(agent.register(&arg_str(a, "consoleUrl")?, &arg_str(a, "registrationToken")?, tags).await?)?
        }
        "fleet_agent_start_claim" => serde_json::to_value(agent.start_claim(&arg_str(a, "consoleUrl")?).await?)?,
        "fleet_agent_poll_claim" => serde_json::to_value(agent.poll_claim().await?)?,
        "fleet_agent_cancel_claim" => {
            agent.cancel_claim();
            serde_json::to_value(agent.status())?
        }
        "fleet_agent_unregister" => serde_json::to_value(agent.unregister(a.get("force").and_then(|f| f.as_bool()).unwrap_or(false)).await?)?,
        "fleet_agent_set_enabled" => {
            let on = a.get("enabled").and_then(|f| f.as_bool()).ok_or_else(|| AppError::validation("Missing argument 'enabled'"))?;
            serde_json::to_value(agent.set_enabled(on).await)?
        }
        "fleet_agent_sync_now" => serde_json::to_value(agent.sync_now().await?)?,
        other => return Err(AppError::validation(format!("Unknown command '{other}'"))),
    };
    Ok(v)
}

impl AgentHost for BackupHost {
    fn client_info(&self) -> ClientInfo {
        let mut info = ClientInfo::detect(self.agent_kind, env!("CARGO_PKG_VERSION"));
        // On a shared host (Remote Desktop, Citrix, xrdp) each user's app is
        // its own machine in the console: name it after the user too.
        if info.name.is_none() && self.agent_kind == "desktop" {
            info.name = ufop_core::session::SessionInfo::detect().machine_name(&info.hostname);
        }
        info
    }

    fn load(&self) -> BoxFuture<'_, Result<(AgentConfig, Option<String>), AppError>> {
        Box::pin(async move {
            let cfg = match self.settings.get(CONFIG_KEY).await? {
                Some(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                    tracing::warn!("Unreadable console agent settings, starting unregistered: {e}");
                    AgentConfig::default()
                }),
                None => AgentConfig::default(),
            };
            let token = if self.keychain() { self.creds.retrieve(TOKEN_KEY).await? } else { self.settings.get_secret(TOKEN_KEY).await? };
            Ok((cfg, token))
        })
    }

    fn save_config(&self, cfg: AgentConfig) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move { self.settings.set(CONFIG_KEY, &serde_json::to_string(&cfg)?).await })
    }

    fn save_token(&self, token: Option<String>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            match (token, self.keychain()) {
                (Some(t), true) => self.creds.store(TOKEN_KEY, &t).await,
                (Some(t), false) => self.settings.set_secret(TOKEN_KEY, &t).await,
                (None, _) => {
                    let _ = self.creds.delete(TOKEN_KEY).await;
                    self.settings.delete(TOKEN_KEY).await
                }
            }
        })
    }

    fn jobs(&self) -> BoxFuture<'_, Vec<JobSummary>> {
        Box::pin(async move {
            self.mgr
                .list_jobs()
                .await
                .into_iter()
                .map(|j| JobSummary {
                    managed_by_policy: policy_of_job(&j.id).map(String::from),
                    schedule_interval_minutes: j.schedule.enabled.then(|| j.schedule.nominal_interval_minutes()),
                    last_result: j.stats.last_result,
                    last_run_at: j.stats.last_run_at,
                    id: j.id,
                    name: j.name,
                })
                .collect()
        })
    }

    fn rpc(&self, command: String, args: Value) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move {
            let limits = self.limits().await;
            let job = match (command.as_str(), args.get("id").and_then(Value::as_str)) {
                ("backup_execute" | "backup_start_operation", Some(id)) => self.mgr.get_job(id).await.ok().and_then(|j| serde_json::to_value(j).ok()),
                _ => None,
            };
            guard::check_rpc(&limits, &command, &args, job.as_ref())?;
            api::dispatch(&self.mgr, &command, &args).await
        })
    }

    fn run_backup(&self, job_id: String) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move { Ok(json!({ "taskId": self.mgr.start_backup(&job_id, false).await? })) })
    }

    fn store_credential(&self, credential_id: String, secret: Option<String>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move { self.mgr.set_destination_secret(&credential_id, secret.as_deref().unwrap_or("")).await })
    }

    fn apply_policy(&self, p: FleetPolicy) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let limits = self.limits().await;
            limits.check_write("applying a fleet policy")?;
            let (job, pass) = self.job_for_policy(&p).await?;
            guard::check_job(&limits, &serde_json::to_value(&job)?)?;
            self.mgr.save_job(job, pass).await.map(|_| ())
        })
    }

    fn remove_policy(&self, policy_id: String) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let id = managed_job_id(&policy_id)?;
            if self.mgr.get_job(&id).await.is_err() {
                return Ok(());
            }
            self.mgr.delete_job(&id, false, false).await
        })
    }

    fn managed_policies(&self) -> BoxFuture<'_, Vec<String>> {
        Box::pin(async move { self.mgr.list_jobs().await.iter().filter_map(|j| policy_of_job(&j.id).map(String::from)).collect() })
    }

    fn apply_managed_settings(&self, settings: Option<ManagedSettings>) -> BoxFuture<'_, Result<(), AppError>> {
        let h = self.managed_handler.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Box::pin(async move {
            match h {
                Some(h) => h(settings).await,
                None => Ok(()),
            }
        })
    }

    fn diagnostics(&self) -> BoxFuture<'_, Result<DiagnosticsUpload, AppError>> {
        let kind = self.agent_kind;
        Box::pin(async move {
            let (b, summary) = tokio::task::spawn_blocking(move || crate::diagnostics::collect(kind))
                .await
                .map_err(|e| AppError::internal(format!("Diagnostics task failed: {e}")))??;
            Ok(DiagnosticsUpload { file_name: b.file_name, summary, bytes: b.bytes })
        })
    }
}

pub mod guard;
pub mod loopback;

#[cfg(test)]
mod tests;
