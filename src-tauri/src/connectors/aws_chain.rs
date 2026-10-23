//! AWS credential sources and the default provider chain, shared by the
//! backup S3 store (and usable by any SigV4 client).
//!
//! Sources: static keys, the `AWS_*` environment variables, a shared
//! credentials/config profile, OIDC web identity (`AssumeRoleWithWebIdentity`),
//! ECS/EKS container credentials, EC2 instance metadata (IMDSv2) and `Auto`
//! (the SDK order: environment → web identity → profile → container →
//! instance metadata). An optional STS `AssumeRole` is layered on top of any
//! source. Temporary credentials are cached and fetched again shortly
//! before they expire.
//!
//! Endpoints follow the AWS SDK variables so tests (and unusual setups) can
//! point them elsewhere: `AWS_EC2_METADATA_SERVICE_ENDPOINT`,
//! `AWS_CONTAINER_CREDENTIALS_FULL_URI`, `AWS_ENDPOINT_URL_STS`, plus
//! `UFOP_AWS_ECS_ENDPOINT` for the base of relative container URIs.

use super::aws_credentials::{self, AwsProfile, BaseKeys};
use crate::core::error::AppError;
use crate::core::types::AssumeRoleConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Where the base keys come from.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialSource {
    /// Access key id in the configuration, secret in the credential store.
    #[default]
    Static,
    /// `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`.
    Environment,
    /// A profile of `~/.aws/credentials` / `~/.aws/config`; `file` replaces
    /// the credentials file.
    Profile {
        #[serde(default)]
        name: String,
        #[serde(default)]
        file: Option<String>,
    },
    /// Exchange an OIDC token file (EKS IRSA, GitHub Actions, …) for role credentials.
    WebIdentity {
        #[serde(default)]
        role_arn: String,
        #[serde(default)]
        token_file: String,
    },
    /// EC2 instance profile through IMDSv2.
    InstanceMetadata,
    /// The SDK default chain.
    Auto,
}

impl CredentialSource {
    pub fn is_static(&self) -> bool {
        matches!(self, Self::Static)
    }
}

/// STS AssumeRole on top of the base credentials (`sts_endpoint` and
/// `region` override the STS endpoint and signing region).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AssumeRoleOptions {
    #[serde(flatten)]
    pub role: AssumeRoleConfig,
    pub sts_endpoint: Option<String>,
    pub region: Option<String>,
}

/// Keys plus when they stop working (`None` = long-term keys).
#[derive(Debug, Clone)]
pub struct Credentials {
    pub keys: BaseKeys,
    pub expires: Option<DateTime<Utc>>,
}

/// Environment lookups (the process environment, or a fixed map in tests).
#[derive(Debug, Clone, Default)]
pub struct ChainEnv {
    vars: Option<HashMap<String, String>>,
}

impl ChainEnv {
    pub fn process() -> Self {
        Self { vars: None }
    }

    pub fn fixed(vars: HashMap<String, String>) -> Self {
        Self { vars: Some(vars) }
    }

    pub fn var(&self, k: &str) -> Option<String> {
        match &self.vars {
            Some(m) => m.get(k).cloned(),
            None => std::env::var(k).ok(),
        }
        .filter(|v| !v.trim().is_empty())
    }

    fn home(&self) -> Option<PathBuf> {
        self.var("HOME").or_else(|| self.var("USERPROFILE")).map(PathBuf::from)
    }
}

/// Fetch credentials again this long before they expire.
pub const REFRESH_EARLY: Duration = Duration::from_secs(5 * 60);

const IMDS_DEFAULT: &str = "http://169.254.169.254";
const ECS_DEFAULT: &str = "http://169.254.170.2";

fn missing(what: impl Into<String>, advice: &str) -> AppError {
    AppError::Configuration { message: what.into(), advice: advice.into() }
}

fn expiry(s: Option<&str>) -> Option<DateTime<Utc>> {
    s.and_then(|e| DateTime::parse_from_rfc3339(e).ok()).map(|t| t.with_timezone(&Utc))
}

fn from_sts(c: aws_credentials::AwsCredentials) -> Credentials {
    // STS always returns an expiry; assume an hour when it is unreadable.
    let expires = expiry(c.expiration.as_deref()).or_else(|| Some(Utc::now() + chrono::Duration::hours(1)));
    Credentials { keys: BaseKeys { access_key_id: c.access_key_id, secret_access_key: c.secret_access_key, session_token: Some(c.session_token) }, expires }
}

/// Resolves and caches credentials for one destination.
pub struct CredentialProvider {
    source: CredentialSource,
    /// Static keys (and the key id for `Static`).
    static_keys: Option<BaseKeys>,
    assume: Option<AssumeRoleOptions>,
    region: String,
    env: ChainEnv,
    sts: reqwest::Client,
    /// IMDS / container endpoint client: no proxy, short timeouts.
    local: reqwest::Client,
    cache: tokio::sync::Mutex<Option<Credentials>>,
}

impl CredentialProvider {
    pub fn new(source: CredentialSource, static_keys: Option<BaseKeys>, assume: Option<AssumeRoleOptions>, region: &str, env: ChainEnv) -> Result<Self, AppError> {
        if source.is_static() && static_keys.as_ref().map_or(true, |k| k.access_key_id.trim().is_empty() || k.secret_access_key.trim().is_empty()) {
            return Err(AppError::validation("Enter the access key id and the secret access key"));
        }
        if let Some(a) = &assume {
            if !a.role.role_arn.starts_with("arn:") {
                return Err(AppError::validation("The role to assume should look like arn:aws:iam::123456789012:role/Name"));
            }
        }
        let local = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::internal(format!("HTTP client: {e}")))?;
        Ok(Self { source, static_keys, assume: assume.filter(|a| !a.role.role_arn.trim().is_empty()), region: region.to_string(), env, sts: crate::net::client(), local, cache: tokio::sync::Mutex::new(None) })
    }

    /// Current keys, fetched or refreshed when missing or about to expire.
    pub async fn keys(&self) -> Result<BaseKeys, AppError> {
        let mut cache = self.cache.lock().await;
        if let Some(c) = cache.as_ref() {
            let fresh = c.expires.map_or(true, |e| e - chrono::Duration::from_std(REFRESH_EARLY).unwrap_or_default() > Utc::now());
            if fresh {
                return Ok(c.keys.clone());
            }
            tracing::debug!("AWS credentials expire at {:?}; refreshing", c.expires);
        }
        let c = self.resolve().await?;
        let keys = c.keys.clone();
        *cache = Some(c);
        Ok(keys)
    }

    /// Drop cached credentials (e.g. after the service rejected them).
    pub async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }

    async fn resolve(&self) -> Result<Credentials, AppError> {
        let base = self.base().await?;
        let Some(a) = &self.assume else { return Ok(base) };
        let region = a.region.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| self.region.clone());
        let endpoint = a.sts_endpoint.clone().filter(|e| !e.trim().is_empty()).or_else(|| self.env.var("AWS_ENDPOINT_URL_STS"));
        let t = aws_credentials::assume_role_with(&self.sts, &base.keys, &a.role, &region, endpoint.as_deref()).await?;
        Ok(from_sts(t))
    }

    async fn base(&self) -> Result<Credentials, AppError> {
        match &self.source {
            CredentialSource::Static => Ok(Credentials { keys: self.static_keys.clone().ok_or_else(|| missing("Static keys are missing", "Enter the secret access key."))?, expires: None }),
            CredentialSource::Environment => self.env_keys().ok_or_else(|| missing("AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY are not set", "Set them for the app (or service), or pick another credential source.")),
            CredentialSource::Profile { name, file } => self.profile_keys(name, file.as_deref()).await,
            CredentialSource::WebIdentity { role_arn, token_file } => self.web_identity(role_arn, token_file).await,
            CredentialSource::InstanceMetadata => self.imds().await,
            CredentialSource::Auto => self.auto().await,
        }
    }

    fn env_keys(&self) -> Option<Credentials> {
        let id = self.env.var("AWS_ACCESS_KEY_ID")?;
        let secret = self.env.var("AWS_SECRET_ACCESS_KEY")?;
        Some(Credentials { keys: BaseKeys { access_key_id: id, secret_access_key: secret, session_token: self.env.var("AWS_SESSION_TOKEN") }, expires: expiry(self.env.var("AWS_CREDENTIAL_EXPIRATION").as_deref()) })
    }

    fn sts_endpoint(&self) -> String {
        aws_credentials::sts_url(&self.region, self.env.var("AWS_ENDPOINT_URL_STS").as_deref())
    }

    async fn web_identity(&self, role_arn: &str, token_file: &str) -> Result<Credentials, AppError> {
        if role_arn.trim().is_empty() || token_file.trim().is_empty() {
            return Err(AppError::validation("Web identity needs the role ARN and the token file"));
        }
        let token = tokio::fs::read_to_string(token_file.trim())
            .await
            .map_err(|e| missing(format!("Cannot read the web identity token file {token_file}: {e}"), "Check the path (EKS: AWS_WEB_IDENTITY_TOKEN_FILE) and that the app may read it."))?;
        let session = self.env.var("AWS_ROLE_SESSION_NAME");
        let t = aws_credentials::assume_role_with_web_identity_at(&self.sts, &self.sts_endpoint(), role_arn.trim(), token.trim(), session.as_deref()).await?;
        Ok(from_sts(t))
    }

    fn profile_files(&self, file: Option<&str>) -> (Option<PathBuf>, Option<PathBuf>) {
        let creds = file.filter(|f| !f.trim().is_empty()).map(PathBuf::from).or_else(|| self.env.var("AWS_SHARED_CREDENTIALS_FILE").map(PathBuf::from)).or_else(|| self.env.home().map(|h| h.join(".aws").join("credentials")));
        let config = self.env.var("AWS_CONFIG_FILE").map(PathBuf::from).or_else(|| self.env.home().map(|h| h.join(".aws").join("config")));
        (creds, config)
    }

    fn lookup_profile(&self, name: &str, file: Option<&str>) -> Option<AwsProfile> {
        let (creds, config) = self.profile_files(file);
        let c = creds.map(|p| aws_credentials::parse_credentials_file(&p)).unwrap_or_default();
        let g = config.map(|p| aws_credentials::parse_config_file(&p)).unwrap_or_default();
        aws_credentials::resolve_profile_in(name, &c, &g)
    }

    /// Profile keys: static keys, `role_arn` with `source_profile`,
    /// `credential_source` or `web_identity_token_file`.
    fn profile_keys<'a>(&'a self, name: &'a str, file: Option<&'a str>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Credentials, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let name = if name.trim().is_empty() { self.env.var("AWS_PROFILE").unwrap_or_else(|| "default".into()) } else { name.trim().to_string() };
            self.profile_chain(&name, file, 0).await
        })
    }

    fn profile_chain<'a>(&'a self, name: &'a str, file: Option<&'a str>, depth: u8) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Credentials, AppError>> + Send + 'a>> {
        Box::pin(async move {
            if depth > 4 {
                return Err(missing(format!("AWS profile '{name}': source_profile chain is too deep or circular"), "Check source_profile in ~/.aws/config."));
            }
            let p = self.lookup_profile(name, file).ok_or_else(|| missing(format!("AWS profile '{name}' was not found"), "Check ~/.aws/credentials and ~/.aws/config (or the file set for this destination)."))?;
            if let Some(role) = p.role_arn.clone().filter(|r| !r.is_empty()) {
                if let Some(tf) = p.web_identity_token_file.as_deref().filter(|t| !t.is_empty()) {
                    return self.web_identity(&role, tf).await;
                }
                let base = if let Some(src) = p.source_profile.as_deref().filter(|s| !s.is_empty()) {
                    // A profile that names itself uses its own static keys.
                    if src == name {
                        static_of(&p).ok_or_else(|| missing(format!("AWS profile '{name}' has no keys"), "Add aws_access_key_id / aws_secret_access_key."))?
                    } else {
                        self.profile_chain(src, file, depth + 1).await?
                    }
                } else {
                    match p.credential_source.as_deref() {
                        Some("Environment") => self.env_keys().ok_or_else(|| missing("credential_source = Environment, but AWS_ACCESS_KEY_ID is not set", "Set the variables or change the profile."))?,
                        Some("Ec2InstanceMetadata") => self.imds().await?,
                        Some("EcsContainer") => self.container().await?.ok_or_else(|| missing("credential_source = EcsContainer, but no container credentials are available", "Run inside ECS/EKS or change the profile."))?,
                        other => return Err(missing(format!("AWS profile '{name}' assumes a role without source_profile or a supported credential_source ({other:?})"), "Set source_profile, or credential_source = Environment | Ec2InstanceMetadata | EcsContainer.")),
                    }
                };
                let region = p.region.clone().unwrap_or_else(|| self.region.clone());
                let cfg = AssumeRoleConfig { role_arn: role, ..Default::default() };
                let endpoint = self.env.var("AWS_ENDPOINT_URL_STS");
                return Ok(from_sts(aws_credentials::assume_role_with(&self.sts, &base.keys, &cfg, &region, endpoint.as_deref()).await?));
            }
            if let Some(c) = static_of(&p) {
                return Ok(c);
            }
            if p.credential_process.is_some() {
                return Err(missing(format!("AWS profile '{name}' uses credential_process, which UFOP does not run"), "Use a profile with keys, web identity or a role, or export the keys (aws configure export-credentials --format env) and pick Environment."));
            }
            if p.sso_start_url.is_some() || p.sso_session.is_some() {
                return Err(missing(format!("AWS profile '{name}' uses IAM Identity Center (SSO), which this destination cannot refresh unattended"), "Use a role with source_profile, web identity, instance metadata, or export the session keys and pick Environment."));
            }
            Err(missing(format!("AWS profile '{name}' has no usable credentials"), "Add aws_access_key_id / aws_secret_access_key, or a role_arn."))
        })
    }

    /// ECS / EKS Pod Identity container credentials, when configured.
    async fn container(&self) -> Result<Option<Credentials>, AppError> {
        let url = if let Some(rel) = self.env.var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
            let base = self.env.var("UFOP_AWS_ECS_ENDPOINT").unwrap_or_else(|| ECS_DEFAULT.into());
            format!("{}{}", base.trim_end_matches('/'), rel)
        } else if let Some(full) = self.env.var("AWS_CONTAINER_CREDENTIALS_FULL_URI") {
            full
        } else {
            return Ok(None);
        };
        let token = match self.env.var("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE") {
            Some(f) => Some(tokio::fs::read_to_string(&f).await.map_err(|e| missing(format!("Cannot read {f}: {e}"), "Check AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE."))?.trim().to_string()),
            None => self.env.var("AWS_CONTAINER_AUTHORIZATION_TOKEN"),
        };
        let mut req = self.local.get(&url);
        if let Some(t) = token {
            req = req.header("authorization", t);
        }
        let r = req.send().await.map_err(|e| AppError::connection(format!("Container credentials ({url}): {e}"), "Check the task role / pod identity."))?;
        let v: serde_json::Value = crate::backup_engine::stores::http::check(r, "Container credentials").await?.json().await.map_err(|e| AppError::connection(format!("Container credentials: {e}"), "Unexpected response."))?;
        json_creds(&v).map(Some)
    }

    /// EC2 instance profile through IMDSv2 (session token first).
    async fn imds(&self) -> Result<Credentials, AppError> {
        if self.env.var("AWS_EC2_METADATA_DISABLED").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
            return Err(missing("Instance metadata is disabled (AWS_EC2_METADATA_DISABLED)", "Pick another credential source."));
        }
        let base = self.env.var("AWS_EC2_METADATA_SERVICE_ENDPOINT").unwrap_or_else(|| IMDS_DEFAULT.into());
        let base = base.trim_end_matches('/');
        let net = |e: reqwest::Error| AppError::connection(format!("Instance metadata ({base}): {e}"), "Run on EC2 with an instance profile, or pick another credential source.");
        let token = self.local.put(format!("{base}/latest/api/token")).header("x-aws-ec2-metadata-token-ttl-seconds", "21600").send().await.map_err(net)?;
        let token = crate::backup_engine::stores::http::check(token, "Instance metadata token").await?.text().await.map_err(net)?;
        let get = |path: String| self.local.get(format!("{base}{path}")).header("x-aws-ec2-metadata-token", token.trim()).send();
        let roles = crate::backup_engine::stores::http::check(get("/latest/meta-data/iam/security-credentials/".into()).await.map_err(net)?, "Instance metadata role").await?.text().await.map_err(net)?;
        let role = roles.lines().map(str::trim).find(|l| !l.is_empty()).ok_or_else(|| missing("This instance has no IAM instance profile", "Attach an instance profile with access to the bucket."))?.to_string();
        let r = crate::backup_engine::stores::http::check(get(format!("/latest/meta-data/iam/security-credentials/{role}")).await.map_err(net)?, "Instance metadata credentials").await?;
        let v: serde_json::Value = r.json().await.map_err(net)?;
        if v["Code"].as_str().is_some_and(|c| c != "Success") {
            return Err(AppError::connection(format!("Instance metadata credentials: {}", v["Message"].as_str().unwrap_or("not available")), "Check the instance profile."));
        }
        json_creds(&v)
    }

    async fn auto(&self) -> Result<Credentials, AppError> {
        if let Some(c) = self.env_keys() {
            return Ok(c);
        }
        if let (Some(role), Some(file)) = (self.env.var("AWS_ROLE_ARN"), self.env.var("AWS_WEB_IDENTITY_TOKEN_FILE")) {
            return self.web_identity(&role, &file).await;
        }
        let name = self.env.var("AWS_PROFILE").unwrap_or_else(|| "default".into());
        if self.lookup_profile(&name, None).is_some() {
            return self.profile_chain(&name, None, 0).await;
        }
        if let Some(c) = self.container().await? {
            return Ok(c);
        }
        self.imds().await.map_err(|e| {
            missing(
                format!("No AWS credentials found (environment, web identity, profile '{name}', container, instance metadata): {}", e.user_message()),
                "Set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, configure a profile, or pick static keys.",
            )
        })
    }
}

fn static_of(p: &AwsProfile) -> Option<Credentials> {
    let id = p.access_key_id.clone().filter(|s| !s.is_empty())?;
    let secret = p.secret_access_key.clone().filter(|s| !s.is_empty())?;
    Some(Credentials { keys: BaseKeys { access_key_id: id, secret_access_key: secret, session_token: p.session_token.clone().filter(|s| !s.is_empty()) }, expires: None })
}

/// `{AccessKeyId, SecretAccessKey, Token, Expiration}` (IMDS and container endpoints).
fn json_creds(v: &serde_json::Value) -> Result<Credentials, AppError> {
    let s = |k: &str| v[k].as_str().map(String::from).filter(|s| !s.is_empty());
    let id = s("AccessKeyId").ok_or_else(|| AppError::connection("Credentials response has no AccessKeyId", "Unexpected response."))?;
    let secret = s("SecretAccessKey").ok_or_else(|| AppError::connection("Credentials response has no SecretAccessKey", "Unexpected response."))?;
    Ok(Credentials { keys: BaseKeys { access_key_id: id, secret_access_key: secret, session_token: s("Token") }, expires: expiry(s("Expiration").as_deref()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> ChainEnv {
        ChainEnv::fixed(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    #[tokio::test]
    async fn static_env_and_profile_sources() {
        let keys = BaseKeys { access_key_id: "AK".into(), secret_access_key: "SK".into(), session_token: None };
        assert!(CredentialProvider::new(CredentialSource::Static, None, None, "us-east-1", env(&[])).is_err());
        let p = CredentialProvider::new(CredentialSource::Static, Some(keys), None, "us-east-1", env(&[])).unwrap();
        assert_eq!(p.keys().await.unwrap().access_key_id, "AK");

        let p = CredentialProvider::new(CredentialSource::Environment, None, None, "us-east-1", env(&[("AWS_ACCESS_KEY_ID", "EK"), ("AWS_SECRET_ACCESS_KEY", "ES"), ("AWS_SESSION_TOKEN", "ET")])).unwrap();
        let k = p.keys().await.unwrap();
        assert_eq!((k.access_key_id.as_str(), k.session_token.as_deref()), ("EK", Some("ET")));
        assert!(CredentialProvider::new(CredentialSource::Environment, None, None, "us-east-1", env(&[])).unwrap().keys().await.is_err());

        let d = tempfile::tempdir().unwrap();
        let creds = d.path().join("credentials");
        std::fs::write(&creds, "[default]\naws_access_key_id = DK\naws_secret_access_key = DS\n[proc]\ncredential_process = /bin/creds\n").unwrap();
        let cfgf = d.path().join("config");
        std::fs::write(&cfgf, "[profile sso]\nsso_start_url = https://x.awsapps.com/start\n").unwrap();
        let e = env(&[("AWS_SHARED_CREDENTIALS_FILE", creds.to_str().unwrap()), ("AWS_CONFIG_FILE", cfgf.to_str().unwrap())]);
        let p = CredentialProvider::new(CredentialSource::Profile { name: String::new(), file: None }, None, None, "us-east-1", e.clone()).unwrap();
        assert_eq!(p.keys().await.unwrap().access_key_id, "DK");
        for (name, needle) in [("proc", "credential_process"), ("sso", "IAM Identity Center"), ("nope", "not found")] {
            let p = CredentialProvider::new(CredentialSource::Profile { name: name.into(), file: None }, None, None, "us-east-1", e.clone()).unwrap();
            assert!(p.keys().await.unwrap_err().user_message().contains(needle), "{name}");
        }
        // Auto picks the default profile when no variables are set (and no IMDS is tried).
        let p = CredentialProvider::new(CredentialSource::Auto, None, None, "us-east-1", e).unwrap();
        assert_eq!(p.keys().await.unwrap().access_key_id, "DK");
        assert!(CredentialProvider::new(CredentialSource::Auto, None, Some(AssumeRoleOptions { role: AssumeRoleConfig { role_arn: "Reader".into(), ..Default::default() }, ..Default::default() }), "us-east-1", env(&[])).is_err());
    }

    #[test]
    fn serde_shapes() {
        assert_eq!(serde_json::to_value(CredentialSource::default()).unwrap(), serde_json::json!({ "type": "static" }));
        let p: CredentialSource = serde_json::from_value(serde_json::json!({ "type": "profile", "name": "prod" })).unwrap();
        assert_eq!(p, CredentialSource::Profile { name: "prod".into(), file: None });
        let a: AssumeRoleOptions = serde_json::from_value(serde_json::json!({ "role_arn": "arn:aws:iam::1:role/R", "external_id": "x", "duration_secs": 900, "region": "eu-west-1" })).unwrap();
        assert_eq!((a.role.role_arn.as_str(), a.role.duration_secs, a.region.as_deref(), a.sts_endpoint), ("arn:aws:iam::1:role/R", Some(900), Some("eu-west-1"), None));
    }
}
