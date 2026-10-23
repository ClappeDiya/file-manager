//! Wire types and the HTTP client for the admin console's `/api/agent/*`
//! routes. Field names follow the console's JSON (camelCase), except the
//! run report, whose keys the console reads in snake_case.

use crate::error::AppError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// What the machine tells the console about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub hostname: String,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub client_version: String,
    /// `desktop`, `server` or `cli`.
    pub agent_kind: String,
}

impl ClientInfo {
    /// Hostname, OS and architecture of this machine.
    pub fn detect(agent_kind: &str, client_version: &str) -> Self {
        Self {
            name: None,
            hostname: hostname(),
            os: std::env::consts::OS.to_string(),
            os_version: os_version(),
            arch: std::env::consts::ARCH.to_string(),
            client_version: client_version.to_string(),
            agent_kind: agent_kind.to_string(),
        }
    }
}

/// The machine's host name (best effort; empty when unknown).
pub use crate::session::hostname;

fn os_version() -> String {
    if cfg!(target_os = "linux") {
        if let Ok(t) = std::fs::read_to_string("/etc/os-release") {
            for l in t.lines() {
                if let Some(v) = l.strip_prefix("PRETTY_NAME=") {
                    return v.trim_matches('"').to_string();
                }
            }
        }
    }
    String::new()
}

/// A backup job as listed in heartbeats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobSummary {
    pub id: String,
    pub name: String,
    pub schedule_interval_minutes: Option<u32>,
    pub last_result: Option<String>,
    pub last_run_at: Option<String>,
    pub managed_by_policy: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Registration {
    pub machine_id: String,
    pub agent_token: String,
    pub organization: String,
    pub heartbeat_seconds: u64,
}

impl std::fmt::Debug for ClaimStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimStart").field("claim_id", &self.claim_id).field("code", &self.code).field("expires_at", &self.expires_at).finish()
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaimStart {
    pub claim_id: String,
    pub code: String,
    pub secret: String,
    pub expires_at: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaimPoll {
    /// `pending`, `claimed`, `collected` or `expired`.
    pub status: String,
    pub machine_id: Option<String>,
    pub agent_token: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCommand {
    pub id: String,
    /// `rpc`, `run_backup`, `apply_policies` or `rotate_token`.
    pub kind: String,
    pub payload: Value,
    pub expires_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HeartbeatResponse {
    pub machine_id: String,
    pub status: String,
    pub commands: Vec<AgentCommand>,
    pub policy_set_version: String,
    pub policies_changed: bool,
    pub heartbeat_seconds: u64,
}

/// A fleet policy: a job template the agent turns into a managed job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FleetPolicy {
    pub id: String,
    pub name: String,
    pub version: u64,
    pub job: Value,
}

/// A central credential delivered for the policies (kept only in the secure store).
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CentralCredential {
    /// `central:<id>`.
    pub credential_id: String,
    pub name: String,
    pub secret: String,
    /// Increases with every rotation (0 from consoles without versions).
    pub version: u64,
    pub updated_at: String,
}

impl std::fmt::Debug for CentralCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CentralCredential").field("credential_id", &self.credential_id).field("name", &self.name).field("secret", &"***").finish()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PolicyBundle {
    pub policy_set_version: String,
    pub policies: Vec<FleetPolicy>,
    pub credentials: Vec<CentralCredential>,
    /// The organization's managed app settings (`None`: nothing managed).
    pub managed_settings: Option<super::ManagedSettings>,
}

/// Size figures the console keeps per backup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportStats {
    pub version_count: u64,
    pub backend_size: u64,
    pub source_size: u64,
    pub schedule_interval_minutes: Option<u32>,
}

/// Largest diagnostics bundle the agent uploads (the console enforces the same).
pub const MAX_DIAGNOSTICS_BYTES: usize = 8 * 1024 * 1024;

/// A diagnostics bundle (zip) built by the host, as posted to `/api/agent/diagnostics`.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticsUpload {
    pub file_name: String,
    /// Small JSON summary shown in the console without opening the zip.
    pub summary: Value,
    pub bytes: Vec<u8>,
}

/// One finished operation, as posted to `/api/agent/report`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunReport {
    pub backup_id: String,
    pub name: String,
    pub operation: String,
    /// `success`, `warning`, `error` or `cancelled`.
    pub result: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: u64,
    pub message: String,
    /// The engine's report (`files_examined`, `bytes_uploaded`, `warnings`…).
    pub data: Value,
    pub stats: ReportStats,
}

/// Why a console request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleError {
    /// The console could not be reached (network, DNS, TLS, timeout).
    Offline(String),
    /// The agent token is unknown or revoked (401).
    Unauthorized(String),
    /// The machine is suspended in the console (423).
    Suspended(String),
    /// Any other refusal.
    Rejected { status: u16, message: String },
    /// The response was not what the protocol promises.
    Invalid(String),
}

impl ConsoleError {
    pub fn is_offline(&self) -> bool {
        matches!(self, Self::Offline(_))
    }
}

impl std::fmt::Display for ConsoleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline(m) => write!(f, "console unreachable: {m}"),
            Self::Unauthorized(m) => write!(f, "{m}"),
            Self::Suspended(m) => write!(f, "{m}"),
            Self::Rejected { status, message } => write!(f, "{message} (HTTP {status})"),
            Self::Invalid(m) => write!(f, "unexpected console response: {m}"),
        }
    }
}

impl From<ConsoleError> for AppError {
    fn from(e: ConsoleError) -> Self {
        match e {
            ConsoleError::Offline(m) => AppError::connection(
                format!("The management console could not be reached ({m})"),
                "Check the console URL and the network; the agent retries on its own.",
            ),
            ConsoleError::Unauthorized(m) => AppError::PermissionDenied {
                message: format!("The console refused this machine: {m}"),
                advice: "The machine was removed or its token revoked in the console. Unregister here and register again.".into(),
            },
            ConsoleError::Suspended(m) => AppError::state(m, "An administrator can resume the machine in the console."),
            ConsoleError::Rejected { status, message } => AppError::Configuration {
                message: format!("The console rejected the request: {message} (HTTP {status})"),
                advice: if status == 401 || status == 410 {
                    "Ask the administrator for a new registration link.".into()
                } else if status == 429 {
                    "Too many attempts; wait a few minutes and try again.".into()
                } else {
                    "Check the console URL and the value you entered.".into()
                },
            },
            ConsoleError::Invalid(m) => AppError::Configuration {
                message: format!("The console answered unexpectedly ({m})"),
                advice: "Check that the URL points at a UFOP management console of a compatible version.".into(),
            },
        }
    }
}

/// Check and normalize a console base URL (`https://console.example.com`).
/// Plain `http` is accepted only for loopback hosts (development consoles)
/// unless `UFOP_CONSOLE_ALLOW_HTTP=1`, because the agent token travels in
/// every request.
pub fn normalize_console_url(url: &str) -> Result<String, AppError> {
    let raw = url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(raw).map_err(|_| AppError::validation("Enter the console address, e.g. https://console.example.com"))?;
    let host = parsed.host_str().unwrap_or_default().to_string();
    if host.is_empty() {
        return Err(AppError::validation("The console URL needs a host name"));
    }
    match parsed.scheme() {
        "https" => {}
        "http" => {
            let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1";
            if !loopback && std::env::var("UFOP_CONSOLE_ALLOW_HTTP").as_deref() != Ok("1") {
                return Err(AppError::Configuration {
                    message: "The console URL must use https".into(),
                    advice: "Use the https:// address of the console (plain http is only allowed for localhost).".into(),
                });
            }
        }
        _ => return Err(AppError::validation("The console URL must start with https://")),
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::validation("The console URL must not contain a query or fragment"));
    }
    Ok(raw.to_string())
}

/// HTTP client for one console.
#[derive(Clone)]
pub struct AgentClient {
    base: String,
    http: reqwest::Client,
    timeout: Duration,
}

impl AgentClient {
    pub fn new(console_url: &str, http: reqwest::Client) -> Result<Self, AppError> {
        Ok(Self { base: normalize_console_url(console_url)?, http, timeout: Duration::from_secs(30) })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    async fn call<T: DeserializeOwned>(&self, path: &str, token: Option<&str>, body: Option<Value>, get: bool) -> Result<T, ConsoleError> {
        let url = format!("{}{path}", self.base);
        let mut rb = if get { self.http.get(&url) } else { self.http.post(&url) };
        rb = rb.timeout(self.timeout).header("accept", "application/json");
        if let Some(t) = token {
            rb = rb.bearer_auth(t);
        }
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        let resp = rb.send().await.map_err(|e| ConsoleError::Offline(describe(&e)))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| ConsoleError::Offline(describe(&e)))?;
        if (200..300).contains(&status) {
            return serde_json::from_str(if text.trim().is_empty() { "{}" } else { &text }).map_err(|e| ConsoleError::Invalid(e.to_string()));
        }
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| format!("HTTP {status}"));
        Err(match status {
            401 if token.is_some() => ConsoleError::Unauthorized(msg),
            423 => ConsoleError::Suspended(msg),
            // Gateways in front of a restarting console: treat like offline.
            502..=504 => ConsoleError::Offline(msg),
            _ => ConsoleError::Rejected { status, message: msg },
        })
    }

    pub async fn register(&self, registration_token: &str, info: &ClientInfo, tags: &[String]) -> Result<Registration, ConsoleError> {
        let r: Registration = self.call("/api/agent/register", None, Some(json!({ "registrationToken": registration_token, "info": info, "tags": tags })), false).await?;
        if r.machine_id.is_empty() || !r.agent_token.starts_with("ufop_agent_") {
            return Err(ConsoleError::Invalid("no machine id or agent token".into()));
        }
        Ok(r)
    }

    /// Redeem a browser sign-in code (loopback redirect + PKCE) for an agent token.
    pub async fn connect_token(&self, code: &str, code_verifier: &str, redirect_uri: &str, info: &ClientInfo, tags: &[String]) -> Result<Registration, ConsoleError> {
        let r: Registration = self
            .call("/api/agent/connect/token", None, Some(json!({ "code": code, "codeVerifier": code_verifier, "redirectUri": redirect_uri, "info": info, "tags": tags })), false)
            .await?;
        if r.machine_id.is_empty() || !r.agent_token.starts_with("ufop_agent_") {
            return Err(ConsoleError::Invalid("no machine id or agent token".into()));
        }
        Ok(r)
    }

    pub async fn start_claim(&self, info: &ClientInfo) -> Result<ClaimStart, ConsoleError> {
        let c: ClaimStart = self.call("/api/agent/claim", None, Some(json!({ "info": info })), false).await?;
        if c.claim_id.is_empty() || c.secret.is_empty() || c.code.is_empty() {
            return Err(ConsoleError::Invalid("incomplete claim".into()));
        }
        Ok(c)
    }

    pub async fn poll_claim(&self, claim_id: &str, secret: &str) -> Result<ClaimPoll, ConsoleError> {
        self.call("/api/agent/claim/poll", None, Some(json!({ "claimId": claim_id, "secret": secret })), false).await
    }

    /// `heartbeat_seconds`: the interval this agent uses (the console stores it
    /// to judge connectivity).
    pub async fn heartbeat(&self, token: &str, info: &ClientInfo, jobs: &[JobSummary], policy_set_version: &str, heartbeat_seconds: Option<u64>) -> Result<HeartbeatResponse, ConsoleError> {
        let mut body = json!({ "info": info, "jobs": jobs, "policySetVersion": policy_set_version });
        if let Some(s) = heartbeat_seconds {
            body["heartbeatSeconds"] = json!(s);
        }
        self.call("/api/agent/heartbeat", Some(token), Some(body), false).await
    }

    pub async fn policies(&self, token: &str) -> Result<PolicyBundle, ConsoleError> {
        self.call("/api/agent/policies", Some(token), None, true).await
    }

    pub async fn policies_applied(&self, token: &str, applied: &BTreeMap<String, u64>, errors: &BTreeMap<String, String>) -> Result<(), ConsoleError> {
        self.call::<Value>("/api/agent/policies/applied", Some(token), Some(json!({ "applied": applied, "errors": errors })), false).await.map(|_| ())
    }

    pub async fn report(&self, token: &str, report: &Value) -> Result<(), ConsoleError> {
        self.call::<Value>("/api/agent/report", Some(token), Some(report.clone()), false).await.map(|_| ())
    }

    /// Upload a diagnostics bundle; returns the console's id for it.
    pub async fn diagnostics(&self, token: &str, d: &DiagnosticsUpload) -> Result<String, ConsoleError> {
        use base64::Engine as _;
        let body = json!({ "fileName": d.file_name, "summary": d.summary, "bundleBase64": base64::engine::general_purpose::STANDARD.encode(&d.bytes) });
        let v: Value = self.call("/api/agent/diagnostics", Some(token), Some(body), false).await?;
        v.get("id").and_then(|i| i.as_str()).map(String::from).ok_or_else(|| ConsoleError::Invalid("no diagnostics id".into()))
    }

    pub async fn complete_command(&self, token: &str, id: &str, ok: bool, result: &Value) -> Result<(), ConsoleError> {
        let path = format!("/api/agent/commands/{}", percent_encode(id));
        self.call::<Value>(&path, Some(token), Some(json!({ "ok": ok, "result": result })), false).await.map(|_| ())
    }

    pub async fn rotate(&self, token: &str) -> Result<String, ConsoleError> {
        let v: Value = self.call("/api/agent/rotate", Some(token), None, false).await?;
        v.get("agentToken").and_then(|t| t.as_str()).filter(|t| t.starts_with("ufop_agent_")).map(String::from).ok_or_else(|| ConsoleError::Invalid("no agentToken".into()))
    }

    pub async fn unregister(&self, token: &str) -> Result<(), ConsoleError> {
        self.call::<Value>("/api/agent/unregister", Some(token), None, false).await.map(|_| ())
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn describe(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(c) = src {
        s.push_str(": ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}
