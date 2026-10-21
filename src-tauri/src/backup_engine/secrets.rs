//! Secret providers and placeholders.
//!
//! A job setting (or a stored credential) written as `$name` — or with
//! `${name}` inside a longer value — is replaced at run time by the secret
//! `name` from the configured providers: environment variables, a JSON or
//! `.env` file, Linux `pass`, HashiCorp Vault (KV v1/v2), AWS Secrets
//! Manager, Google Secret Manager or Azure Key Vault. `$$` escapes a literal
//! `$`. Providers are global (every job) or opt-in per job; lookups can be
//! cached in memory or in the secure store.
//!
//! Provider credentials (Vault token, AWS secret key, GCP service-account
//! JSON, Azure client secret) live in the secure store under the provider's
//! `credential_id`, never in the configuration.

use super::stores::http::{self, TlsOptions};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

pub const SETTINGS_KEY: &str = "backup.secret_providers";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderKind {
    /// `NAME` → environment variable `<prefix>NAME`.
    Env {
        #[serde(default)]
        prefix: String,
    },
    /// A JSON object of strings, or `KEY=VALUE` lines (`.env`).
    File { path: String },
    /// `pass show <prefix>/<name>` (first line).
    Pass {
        #[serde(default)]
        executable: String,
        #[serde(default)]
        prefix: String,
    },
    /// HashiCorp Vault KV. `name` is a key of the secret at `path`, or
    /// `other/path#key`.
    Vault {
        address: String,
        #[serde(default = "default_mount")]
        mount: String,
        #[serde(default)]
        path: String,
        #[serde(default)]
        namespace: String,
        #[serde(default = "default_kv")]
        kv_version: u8,
        #[serde(default)]
        tls: TlsOptions,
    },
    /// AWS Secrets Manager. `name` is a secret id, or `id#json_key`; with
    /// `secret_id` set, `name` is a key of that secret's JSON.
    AwsSecretsManager {
        region: String,
        access_key_id: String,
        #[serde(default)]
        endpoint: String,
        #[serde(default)]
        secret_id: String,
        #[serde(default)]
        tls: TlsOptions,
    },
    /// Google Secret Manager (latest version). `name` is the secret id, or
    /// `id#json_key`.
    GcpSecretManager {
        project: String,
        #[serde(default)]
        endpoint: String,
        #[serde(default)]
        tls: TlsOptions,
    },
    /// Azure Key Vault with an app registration (client credentials).
    /// `_` in names maps to `-` (Key Vault names allow only `[0-9a-zA-Z-]`).
    AzureKeyVault {
        vault_url: String,
        tenant_id: String,
        client_id: String,
        /// Token endpoint base; empty = `https://login.microsoftonline.com`.
        #[serde(default)]
        authority: String,
        #[serde(default)]
        tls: TlsOptions,
    },
}

fn default_mount() -> String {
    "secret".into()
}

fn default_kv() -> u8 {
    2
}

impl ProviderKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Env { .. } => "environment",
            Self::File { .. } => "file",
            Self::Pass { .. } => "pass",
            Self::Vault { .. } => "vault",
            Self::AwsSecretsManager { .. } => "aws_secrets_manager",
            Self::GcpSecretManager { .. } => "gcp_secret_manager",
            Self::AzureKeyVault { .. } => "azure_key_vault",
        }
    }

    /// Whether the provider needs a stored credential.
    pub fn needs_credential(&self) -> bool {
        matches!(self, Self::Vault { .. } | Self::AwsSecretsManager { .. } | Self::GcpSecretManager { .. } | Self::AzureKeyVault { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretProvider {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: ProviderKind,
    /// Secure-store id of the provider's own credential.
    #[serde(default)]
    pub credential_id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Used by every job; otherwise only by jobs that list it.
    #[serde(default = "yes")]
    pub global: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// Look secrets up on every run.
    None,
    /// Keep values in memory for `cache_ttl_secs`.
    #[default]
    Memory,
    /// Also keep them in the secure store, so runs work while a provider is
    /// briefly unreachable.
    Persistent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecretSettings {
    pub providers: Vec<SecretProvider>,
    pub cache: CacheMode,
    pub cache_ttl_secs: u64,
}

impl Default for SecretSettings {
    fn default() -> Self {
        Self { providers: Vec::new(), cache: CacheMode::Memory, cache_ttl_secs: 3600 }
    }
}

impl SecretSettings {
    pub fn validate(&self) -> Result<(), AppError> {
        let mut ids = BTreeSet::new();
        for p in &self.providers {
            if p.name.trim().is_empty() {
                return Err(AppError::validation("Every secret provider needs a name"));
            }
            if !ids.insert(p.id.as_str()) {
                return Err(AppError::validation(format!("Duplicate secret provider id '{}'", p.id)));
            }
            let empty = |v: &str, what: &str| if v.trim().is_empty() { Err(AppError::validation(format!("{}: enter the {what}", p.name))) } else { Ok(()) };
            match &p.kind {
                ProviderKind::Env { .. } => {}
                ProviderKind::File { path } => empty(path, "file path")?,
                ProviderKind::Pass { .. } => {}
                ProviderKind::Vault { address, kv_version, .. } => {
                    empty(address, "Vault address")?;
                    if !matches!(kv_version, 1 | 2) {
                        return Err(AppError::validation(format!("{}: KV version must be 1 or 2", p.name)));
                    }
                }
                ProviderKind::AwsSecretsManager { region, access_key_id, .. } => {
                    empty(region, "region")?;
                    empty(access_key_id, "access key id")?;
                }
                ProviderKind::GcpSecretManager { project, .. } => empty(project, "project id")?,
                ProviderKind::AzureKeyVault { vault_url, tenant_id, client_id, .. } => {
                    empty(vault_url, "vault URL")?;
                    empty(tenant_id, "tenant id")?;
                    empty(client_id, "client id")?;
                }
            }
            if p.kind.needs_credential() && p.credential_id.trim().is_empty() {
                return Err(AppError::validation(format!("{}: missing credential id", p.name)));
            }
        }
        Ok(())
    }

    /// Providers for a job, in lookup order: the job's own, then global ones.
    pub fn for_job(&self, job_providers: &[String]) -> Vec<&SecretProvider> {
        let mut out: Vec<&SecretProvider> = job_providers.iter().filter_map(|id| self.providers.iter().find(|p| &p.id == id && p.enabled)).collect();
        for p in self.providers.iter().filter(|p| p.enabled && p.global) {
            if !out.iter().any(|q| q.id == p.id) {
                out.push(p);
            }
        }
        out
    }
}

// ── Placeholders ──

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#' | '@')
}

/// Placeholder names used in `value` (`$name` as the whole value, `${name}`
/// anywhere; `$$` is a literal `$`).
pub fn placeholders(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(name) = whole(value) {
        out.push(name.to_string());
        return out;
    }
    let b = value.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            if b.get(i + 1) == Some(&b'$') {
                i += 2;
                continue;
            }
            if b.get(i + 1) == Some(&b'{') {
                if let Some(end) = value[i + 2..].find('}') {
                    let n = &value[i + 2..i + 2 + end];
                    if !n.is_empty() && n.chars().all(name_char) {
                        out.push(n.to_string());
                    }
                    i += 3 + end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn whole(value: &str) -> Option<&str> {
    let n = value.strip_prefix('$')?;
    (!n.is_empty() && !n.starts_with('$') && !n.starts_with('{') && n.chars().all(name_char)).then_some(n)
}

/// Substitute placeholders with `values` (every name must be present).
pub fn substitute(value: &str, values: &HashMap<String, String>) -> String {
    if let Some(n) = whole(value) {
        return values.get(n).cloned().unwrap_or_else(|| value.to_string());
    }
    if !value.contains('$') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let b = value.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            if b.get(i + 1) == Some(&b'$') {
                out.push('$');
                i += 2;
                continue;
            }
            if b.get(i + 1) == Some(&b'{') {
                if let Some(end) = value[i + 2..].find('}') {
                    let n = &value[i + 2..i + 2 + end];
                    if let Some(v) = values.get(n) {
                        out.push_str(v);
                        i += 3 + end;
                        continue;
                    }
                }
            }
        }
        let ch = value[i..].chars().next().unwrap_or('$');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// JSON object keys whose string values are never treated as placeholders.
/// Filters are skipped because `$RECYCLE.BIN`-style rules are common.
const SKIP_KEYS: &[&str] = &["filter", "id", "name", "description", "created_at", "updated_at", "stats", "last_run", "last_result", "credential_id", "message_template", "template", "subject"];

/// Every placeholder in a JSON configuration (job, destination).
pub fn collect(v: &serde_json::Value) -> BTreeSet<String> {
    fn walk(v: &serde_json::Value, key: &str, out: &mut BTreeSet<String>) {
        if SKIP_KEYS.contains(&key) {
            return;
        }
        match v {
            serde_json::Value::String(s) => out.extend(placeholders(s)),
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
            serde_json::Value::Object(m) => m.iter().for_each(|(k, x)| walk(x, k, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(v, "", &mut out);
    out
}

/// Replace placeholders in a JSON configuration.
pub fn apply(v: &mut serde_json::Value, values: &HashMap<String, String>) {
    fn walk(v: &mut serde_json::Value, key: &str, values: &HashMap<String, String>) {
        if SKIP_KEYS.contains(&key) {
            return;
        }
        match v {
            serde_json::Value::String(s) => *s = substitute(s, values),
            serde_json::Value::Array(a) => a.iter_mut().for_each(|x| walk(x, key, values)),
            serde_json::Value::Object(m) => m.iter_mut().for_each(|(k, x)| walk(x, k, values)),
            _ => {}
        }
    }
    walk(v, "", values)
}

// ── Lookups ──

/// `Ok(None)` = this provider does not have the secret.
pub async fn lookup(p: &SecretProvider, credential: Option<&str>, name: &str) -> Result<Option<String>, AppError> {
    match &p.kind {
        ProviderKind::Env { prefix } => Ok(std::env::var(format!("{prefix}{name}")).ok()),
        ProviderKind::File { path } => {
            let text = tokio::fs::read_to_string(path).await.map_err(|e| AppError::file_op(format!("Secret file {path}: {e}"), "Check the path and permissions."))?;
            Ok(parse_file(&text)?.remove(name))
        }
        ProviderKind::Pass { executable, prefix } => {
            let exe = if executable.trim().is_empty() { "pass" } else { executable.trim() };
            let entry = if prefix.trim().is_empty() { name.to_string() } else { format!("{}/{name}", prefix.trim().trim_end_matches('/')) };
            let out = tokio::process::Command::new(exe)
                .arg("show")
                .arg(&entry)
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| AppError::state(format!("Could not run {exe}: {e}"), "Install pass (passwordstore.org) or set its path."))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                if err.contains("not in the password store") {
                    return Ok(None);
                }
                return Err(AppError::state(format!("pass show {entry} failed: {}", err.trim()), "Check that the GPG agent can decrypt the store."));
            }
            Ok(String::from_utf8_lossy(&out.stdout).lines().next().map(|l| l.to_string()))
        }
        ProviderKind::Vault { address, mount, path, namespace, kv_version, tls } => {
            let token = credential.filter(|t| !t.is_empty()).ok_or_else(|| AppError::validation(format!("{}: the Vault token is missing", p.name)))?;
            let (spath, key) = split_key(name, path);
            let mount = mount.trim_matches('/');
            let spath = spath.trim_matches('/');
            let url = if *kv_version == 1 { format!("{}/v1/{mount}/{spath}", address.trim_end_matches('/')) } else { format!("{}/v1/{mount}/data/{spath}", address.trim_end_matches('/')) };
            let mut req = http::client(tls)?.get(&url).header("X-Vault-Token", token);
            if !namespace.trim().is_empty() {
                req = req.header("X-Vault-Namespace", namespace.trim());
            }
            let r = req.send().await.map_err(|e| http::net("Vault", e))?;
            if r.status().as_u16() == 404 {
                return Ok(None);
            }
            let v: serde_json::Value = http::check(r, "Vault").await?.json().await.map_err(|e| http::net("Vault", e))?;
            let data = if *kv_version == 1 { &v["data"] } else { &v["data"]["data"] };
            Ok(json_field(data, &key))
        }
        ProviderKind::AwsSecretsManager { region, access_key_id, endpoint, secret_id, tls } => {
            let secret = credential.filter(|t| !t.is_empty()).ok_or_else(|| AppError::validation(format!("{}: the AWS secret access key is missing", p.name)))?;
            let (id, key) = if secret_id.trim().is_empty() {
                match name.split_once('#') {
                    Some((i, k)) => (i.to_string(), Some(k.to_string())),
                    None => (name.to_string(), None),
                }
            } else {
                (secret_id.trim().to_string(), Some(name.to_string()))
            };
            let Some(text) = aws_get_secret(region, access_key_id, secret, endpoint, tls, &id).await? else { return Ok(None) };
            Ok(match key {
                Some(k) => serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| json_field(&v, &k)),
                None => Some(text),
            })
        }
        ProviderKind::GcpSecretManager { project, endpoint, tls } => {
            let cred = credential.filter(|t| !t.is_empty()).ok_or_else(|| AppError::validation(format!("{}: the service-account key is missing", p.name)))?;
            let client = http::client(tls)?;
            let token = gcp_token(&client, cred).await?;
            let (id, key) = match name.split_once('#') {
                Some((i, k)) => (i, Some(k)),
                None => (name, None),
            };
            let base = if endpoint.trim().is_empty() { "https://secretmanager.googleapis.com" } else { endpoint.trim().trim_end_matches('/') };
            let r = client.get(format!("{base}/v1/projects/{project}/secrets/{id}/versions/latest:access")).bearer_auth(token).send().await.map_err(|e| http::net("Google Secret Manager", e))?;
            if r.status().as_u16() == 404 {
                return Ok(None);
            }
            let v: serde_json::Value = http::check(r, "Google Secret Manager").await?.json().await.map_err(|e| http::net("Google Secret Manager", e))?;
            use base64::Engine;
            let data = v["payload"]["data"].as_str().unwrap_or_default();
            let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| AppError::connection("Google Secret Manager returned invalid data", "Check the secret."))?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Ok(match key {
                Some(k) => serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| json_field(&v, k)),
                None => Some(text),
            })
        }
        ProviderKind::AzureKeyVault { vault_url, tenant_id, client_id, authority, tls } => {
            let secret = credential.filter(|t| !t.is_empty()).ok_or_else(|| AppError::validation(format!("{}: the client secret is missing", p.name)))?;
            let client = http::client(tls)?;
            let auth = if authority.trim().is_empty() { "https://login.microsoftonline.com" } else { authority.trim().trim_end_matches('/') };
            let r = client
                .post(format!("{auth}/{tenant_id}/oauth2/v2.0/token"))
                .form(&[("grant_type", "client_credentials"), ("client_id", client_id.as_str()), ("client_secret", secret), ("scope", "https://vault.azure.net/.default")])
                .send()
                .await
                .map_err(|e| http::net("Azure sign-in", e))?;
            let t: serde_json::Value = http::check(r, "Azure sign-in").await?.json().await.map_err(|e| http::net("Azure sign-in", e))?;
            let token = t["access_token"].as_str().ok_or_else(|| AppError::connection("Azure returned no access token", "Check the tenant, client id and secret."))?;
            let kv_name: String = name.chars().map(|c| if c == '_' { '-' } else { c }).collect();
            let r = client.get(format!("{}/secrets/{kv_name}", vault_url.trim_end_matches('/'))).query(&[("api-version", "7.4")]).bearer_auth(token).send().await.map_err(|e| http::net("Azure Key Vault", e))?;
            if r.status().as_u16() == 404 {
                return Ok(None);
            }
            let v: serde_json::Value = http::check(r, "Azure Key Vault").await?.json().await.map_err(|e| http::net("Azure Key Vault", e))?;
            Ok(v["value"].as_str().map(String::from))
        }
    }
}

fn split_key(name: &str, default_path: &str) -> (String, String) {
    match name.split_once('#') {
        Some((p, k)) => (p.to_string(), k.to_string()),
        None => (default_path.to_string(), name.to_string()),
    }
}

fn json_field(v: &serde_json::Value, key: &str) -> Option<String> {
    match &v[key] {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Secrets from a JSON object or `KEY=VALUE` lines (`#` comments, optional quotes).
pub fn parse_file(text: &str) -> Result<HashMap<String, String>, AppError> {
    let t = text.trim_start();
    if t.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(t).map_err(|e| AppError::validation(format!("Secret file is not valid JSON: {e}")))?;
        let m = v.as_object().ok_or_else(|| AppError::validation("Secret file must be a JSON object"))?;
        return Ok(m.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect());
    }
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("export ").unwrap_or(l).split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))).unwrap_or(v);
            (k.trim().to_string(), v.to_string())
        })
        .collect())
}

async fn aws_get_secret(region: &str, access_key: &str, secret: &str, endpoint: &str, tls: &TlsOptions, id: &str) -> Result<Option<String>, AppError> {
    let (secret, session) = match secret.split_once('\n') {
        Some((s, t)) => (s.trim(), Some(t.trim()).filter(|t| !t.is_empty())),
        None => (secret.trim(), None),
    };
    let base = if endpoint.trim().is_empty() { format!("https://secretsmanager.{region}.amazonaws.com") } else { endpoint.trim().trim_end_matches('/').to_string() };
    let u = url::Url::parse(&base).map_err(|e| AppError::validation(format!("Invalid endpoint '{base}': {e}")))?;
    let host = match u.port() {
        Some(p) => format!("{}:{p}", u.host_str().unwrap_or_default()),
        None => u.host_str().unwrap_or_default().to_string(),
    };
    let body = serde_json::json!({ "SecretId": id }).to_string();
    let hash = super::stores::s3::sha_hex(body.as_bytes());
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_string(), "application/x-amz-json-1.1".to_string());
    headers.insert("host".to_string(), host);
    headers.insert("x-amz-date".to_string(), chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string());
    headers.insert("x-amz-target".to_string(), "secretsmanager.GetSecretValue".to_string());
    if let Some(t) = session {
        headers.insert("x-amz-security-token".to_string(), t.to_string());
    }
    let auth = super::stores::s3::sign_v4("POST", "/", &[], &headers, &hash, access_key, secret, region, "secretsmanager");
    let mut req = http::client(tls)?.post(format!("{base}/")).header("authorization", auth).body(body);
    for (k, v) in &headers {
        if k != "host" {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    let r = req.send().await.map_err(|e| http::net("AWS Secrets Manager", e))?;
    if r.status().as_u16() == 400 || r.status().as_u16() == 404 {
        let text = r.text().await.unwrap_or_default();
        if text.contains("ResourceNotFoundException") {
            return Ok(None);
        }
        return Err(AppError::connection(format!("AWS Secrets Manager: {}", text.chars().take(300).collect::<String>()), "Check the secret id and permissions."));
    }
    let v: serde_json::Value = http::check(r, "AWS Secrets Manager").await?.json().await.map_err(|e| http::net("AWS Secrets Manager", e))?;
    if let Some(s) = v["SecretString"].as_str() {
        return Ok(Some(s.to_string()));
    }
    use base64::Engine;
    Ok(v["SecretBinary"].as_str().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// Access token from a service-account key JSON, or the value itself when it
/// is already a token.
async fn gcp_token(client: &reqwest::Client, cred: &str) -> Result<String, AppError> {
    let c = cred.trim();
    if !c.starts_with('{') {
        return Ok(c.to_string());
    }
    let sa: super::stores::gcs::ServiceAccount = serde_json::from_str(c).map_err(|e| AppError::validation(format!("Invalid service-account key: {e}")))?;
    let jwt = super::stores::gcs::service_account_jwt(&sa, "https://www.googleapis.com/auth/cloud-platform")?;
    let r = client
        .post(&sa.token_uri)
        .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", jwt.as_str())])
        .send()
        .await
        .map_err(|e| http::net("Google token", e))?;
    let v: serde_json::Value = http::check(r, "Google token").await?.json().await.map_err(|e| http::net("Google token", e))?;
    v["access_token"].as_str().map(String::from).ok_or_else(|| AppError::connection("Google returned no access token", "Check the service account."))
}

// ── Cache ──

#[derive(Default)]
pub struct SecretCache {
    entries: std::sync::Mutex<HashMap<(String, String), (String, Instant)>>,
}

impl SecretCache {
    pub fn get(&self, provider: &str, name: &str, ttl: Duration) -> Option<String> {
        let g = self.entries.lock().ok()?;
        g.get(&(provider.to_string(), name.to_string())).filter(|(_, at)| at.elapsed() < ttl).map(|(v, _)| v.clone())
    }
    pub fn put(&self, provider: &str, name: &str, value: &str) {
        if let Ok(mut g) = self.entries.lock() {
            g.insert((provider.to_string(), name.to_string()), (value.to_string(), Instant::now()));
        }
    }
    pub fn clear(&self) {
        if let Ok(mut g) = self.entries.lock() {
            g.clear();
        }
    }
}

// ── Diagnostics ──

/// What a provider test found (never the secret values).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTest {
    pub provider_id: String,
    pub provider: String,
    pub kind: String,
    pub ok: bool,
    pub has_credential: bool,
    pub latency_ms: f64,
    pub found: Vec<String>,
    pub missing: Vec<String>,
    pub errors: Vec<String>,
}

/// Placeholder status for one job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaceholderReport {
    pub placeholders: Vec<PlaceholderStatus>,
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaceholderStatus {
    pub name: String,
    /// Provider that supplied it (`None` = unresolved).
    pub provider: Option<String>,
    pub from_cache: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_parsing_and_substitution() {
        assert_eq!(placeholders("$db_pass"), vec!["db_pass"]);
        assert_eq!(placeholders("https://h/x?token=${api/key#tok}&u=${user}"), vec!["api/key#tok", "user"]);
        assert!(placeholders("price $$5 and C:\\$Recycle").is_empty());
        assert!(placeholders("$").is_empty() && placeholders("${}").is_empty());
        let vals: HashMap<String, String> = [("user".to_string(), "bob".to_string()), ("db_pass".to_string(), "p@ss".to_string())].into();
        assert_eq!(substitute("$db_pass", &vals), "p@ss");
        assert_eq!(substitute("u=${user}, cost $$5, keep ${other}", &vals), "u=bob, cost $5, keep ${other}");
        assert_eq!(substitute("plain ünïcode", &vals), "plain ünïcode");

        let mut job = serde_json::json!({ "name": "$notme", "filter": { "rules": ["$RECYCLE.BIN"] }, "reports": { "http_urls": ["https://x/${user}"] }, "stats": { "last_error": "$x" } });
        assert_eq!(collect(&job).into_iter().collect::<Vec<_>>(), vec!["user"]);
        apply(&mut job, &vals);
        assert_eq!(job["reports"]["http_urls"][0], "https://x/bob");
        assert_eq!(job["name"], "$notme");
    }

    #[test]
    fn secret_files_and_settings_validation() {
        let m = parse_file("# c\nexport A=1\nB=\"two words\"\nC='x=y'\n\n").unwrap();
        assert_eq!((m["A"].as_str(), m["B"].as_str(), m["C"].as_str()), ("1", "two words", "x=y"));
        let m = parse_file(r#"{"k":"v","n":3}"#).unwrap();
        assert_eq!((m["k"].as_str(), m["n"].as_str()), ("v", "3"));
        assert!(parse_file("{nope").is_err());

        let p = |id: &str, kind: ProviderKind, global: bool| SecretProvider { id: id.into(), name: id.into(), kind, credential_id: "c".into(), enabled: true, global };
        let s = SecretSettings {
            providers: vec![p("g", ProviderKind::Env { prefix: String::new() }, true), p("j", ProviderKind::Env { prefix: "X_".into() }, false), SecretProvider { enabled: false, ..p("off", ProviderKind::Env { prefix: String::new() }, true) }],
            ..Default::default()
        };
        s.validate().unwrap();
        assert_eq!(s.for_job(&[]).iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["g"]);
        assert_eq!(s.for_job(&["j".into(), "off".into()]).iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["j", "g"], "job providers first, disabled skipped");
        let bad = SecretSettings { providers: vec![p("v", ProviderKind::Vault { address: String::new(), mount: "secret".into(), path: String::new(), namespace: String::new(), kv_version: 2, tls: Default::default() }, true)], ..Default::default() };
        assert!(bad.validate().is_err());
        let dup = SecretSettings { providers: vec![p("a", ProviderKind::Env { prefix: String::new() }, true), p("a", ProviderKind::Env { prefix: String::new() }, true)], ..Default::default() };
        assert!(dup.validate().is_err());
    }
}
