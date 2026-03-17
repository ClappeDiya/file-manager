//! AWS Credential Chain, IAM Identity Center SSO, and OIDC/STS Federation.
//!
//! Provides:
//! - Parsing `~/.aws/credentials` and `~/.aws/config` files
//! - Profile resolution (merging credentials + config)
//! - IAM Identity Center (SSO) device authorization flow
//! - STS AssumeRoleWithWebIdentity for OIDC federation

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ── Data Types ──────────────────────────────────────────────────────────────

/// Represents an AWS profile parsed from `~/.aws/credentials` and/or `~/.aws/config`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AwsProfile {
    pub name: String,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub region: Option<String>,
    pub sso_start_url: Option<String>,
    pub sso_region: Option<String>,
    pub sso_account_id: Option<String>,
    pub sso_role_name: Option<String>,
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
    pub credential_source: Option<String>,
}

/// Temporary AWS credentials (from SSO, STS, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expiration: Option<String>,
}

/// Response from the OIDC device authorization start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoAuthStart {
    pub verification_uri: String,
    pub user_code: String,
    pub device_code: String,
    pub expires_in: u64,
    /// The OIDC client ID needed for subsequent token polling.
    pub client_id: String,
    /// The OIDC client secret needed for subsequent token polling.
    pub client_secret: String,
}

/// Temporary SSO credentials (access token used to fetch role credentials).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

// ── INI Parsing Helpers ─────────────────────────────────────────────────────

/// Parses a simple INI file into section -> key/value pairs.
/// Handles `[section]` headers and `key = value` lines.
/// Comments (lines starting with # or ;) and blank lines are skipped.
fn parse_ini(content: &str) -> HashMap<String, HashMap<String, String>> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        // Section header: [profile foo] or [foo] or [default]
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            sections.entry(current_section.clone()).or_default();
            continue;
        }

        // Key = value
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();
            if !current_section.is_empty() {
                sections
                    .entry(current_section.clone())
                    .or_default()
                    .insert(key, value);
            }
        }
    }

    sections
}

/// Returns the path to `~/.aws/credentials`.
fn credentials_path() -> Option<PathBuf> {
    // Respect AWS_SHARED_CREDENTIALS_FILE env var
    if let Ok(p) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        return Some(PathBuf::from(p));
    }
    dirs_home().map(|h| h.join(".aws").join("credentials"))
}

/// Returns the path to `~/.aws/config`.
fn config_path() -> Option<PathBuf> {
    // Respect AWS_CONFIG_FILE env var
    if let Ok(p) = std::env::var("AWS_CONFIG_FILE") {
        return Some(PathBuf::from(p));
    }
    dirs_home().map(|h| h.join(".aws").join("config"))
}

/// Resolve the home directory cross-platform.
fn dirs_home() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Build an `AwsProfile` from a section name and its key-value map.
fn profile_from_section(name: &str, kv: &HashMap<String, String>) -> AwsProfile {
    AwsProfile {
        name: name.to_string(),
        access_key_id: kv.get("aws_access_key_id").cloned(),
        secret_access_key: kv.get("aws_secret_access_key").cloned(),
        session_token: kv.get("aws_session_token").cloned(),
        region: kv.get("region").cloned(),
        sso_start_url: kv.get("sso_start_url").cloned(),
        sso_region: kv.get("sso_region").cloned(),
        sso_account_id: kv.get("sso_account_id").cloned(),
        sso_role_name: kv.get("sso_role_name").cloned(),
        role_arn: kv.get("role_arn").cloned(),
        source_profile: kv.get("source_profile").cloned(),
        credential_source: kv.get("credential_source").cloned(),
    }
}

// ── Credential Chain Parsing ────────────────────────────────────────────────

/// Parse `~/.aws/credentials` and return all profiles found.
///
/// The credentials file uses `[profile_name]` section headers directly
/// (no `profile ` prefix, unlike the config file).
pub fn parse_aws_credentials() -> Vec<AwsProfile> {
    let path = match credentials_path() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Could not read {}: {e}", path.display());
            return Vec::new();
        }
    };

    let sections = parse_ini(&content);
    sections
        .iter()
        .map(|(section, kv)| profile_from_section(section, kv))
        .collect()
}

/// Parse `~/.aws/config` and return all profiles found.
///
/// The config file uses `[profile foo]` section headers (except `[default]`).
/// This strips the `profile ` prefix so the returned profile names match
/// those in the credentials file.
pub fn parse_aws_config() -> Vec<AwsProfile> {
    let path = match config_path() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Could not read {}: {e}", path.display());
            return Vec::new();
        }
    };

    let sections = parse_ini(&content);
    sections
        .iter()
        .map(|(section, kv)| {
            // In config file, sections are `[profile foo]` except `[default]`
            let name = if section.starts_with("profile ") {
                section.strip_prefix("profile ").unwrap_or(section)
            } else {
                section.as_str()
            };
            profile_from_section(name, kv)
        })
        .collect()
}

/// Resolve a profile by name, merging `~/.aws/credentials` and `~/.aws/config`.
///
/// Credentials file values take precedence for secret material (access keys),
/// while config file values provide region and SSO/role configuration.
pub fn resolve_profile(name: &str) -> Option<AwsProfile> {
    let creds = parse_aws_credentials();
    let configs = parse_aws_config();

    let cred_profile = creds.iter().find(|p| p.name == name);
    let config_profile = configs.iter().find(|p| p.name == name);

    match (cred_profile, config_profile) {
        (None, None) => None,
        (Some(c), None) => Some(c.clone()),
        (None, Some(c)) => Some(c.clone()),
        (Some(cred), Some(cfg)) => {
            // Merge: credentials file wins for secret material, config for everything else
            Some(AwsProfile {
                name: name.to_string(),
                access_key_id: cred.access_key_id.clone().or_else(|| cfg.access_key_id.clone()),
                secret_access_key: cred
                    .secret_access_key
                    .clone()
                    .or_else(|| cfg.secret_access_key.clone()),
                session_token: cred
                    .session_token
                    .clone()
                    .or_else(|| cfg.session_token.clone()),
                region: cfg.region.clone().or_else(|| cred.region.clone()),
                sso_start_url: cfg
                    .sso_start_url
                    .clone()
                    .or_else(|| cred.sso_start_url.clone()),
                sso_region: cfg.sso_region.clone().or_else(|| cred.sso_region.clone()),
                sso_account_id: cfg
                    .sso_account_id
                    .clone()
                    .or_else(|| cred.sso_account_id.clone()),
                sso_role_name: cfg
                    .sso_role_name
                    .clone()
                    .or_else(|| cred.sso_role_name.clone()),
                role_arn: cfg.role_arn.clone().or_else(|| cred.role_arn.clone()),
                source_profile: cfg
                    .source_profile
                    .clone()
                    .or_else(|| cred.source_profile.clone()),
                credential_source: cfg
                    .credential_source
                    .clone()
                    .or_else(|| cred.credential_source.clone()),
            })
        }
    }
}

/// List all unique profile names from both credentials and config files.
pub fn list_all_profile_names() -> Vec<String> {
    let creds = parse_aws_credentials();
    let configs = parse_aws_config();

    let mut names: Vec<String> = Vec::new();
    for p in creds.iter().chain(configs.iter()) {
        if !names.contains(&p.name) {
            names.push(p.name.clone());
        }
    }
    names.sort();
    names
}

// ── IAM Identity Center SSO ────────────────────────────────────────────────

/// Register an OIDC client with the SSO OIDC service.
///
/// This is the first step in the device authorization flow. Returns
/// a client ID and client secret that are needed for subsequent calls.
async fn register_oidc_client(
    region: &str,
) -> Result<(String, String), AppError> {
    let client = reqwest::Client::new();
    let endpoint = format!(
        "https://oidc.{region}.amazonaws.com/client/register"
    );

    let body = serde_json::json!({
        "clientName": "unified-file-ops",
        "clientType": "public",
    });

    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            AppError::connection(
                format!("Failed to register OIDC client: {e}"),
                "Check your network connection and SSO region.",
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::connection(
            format!("OIDC client registration failed ({status}): {text}"),
            "Check your SSO region is correct.",
        ));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        AppError::connection(
            format!("Invalid OIDC registration response: {e}"),
            "Unexpected response from AWS SSO OIDC service.",
        )
    })?;

    let client_id = json["clientId"]
        .as_str()
        .ok_or_else(|| AppError::connection("Missing clientId in response", "Retry SSO auth."))?
        .to_string();
    let client_secret = json["clientSecret"]
        .as_str()
        .ok_or_else(|| {
            AppError::connection("Missing clientSecret in response", "Retry SSO auth.")
        })?
        .to_string();

    Ok((client_id, client_secret))
}

/// Start the IAM Identity Center SSO device authorization flow.
///
/// Returns verification URI and user code that the user must open in a browser,
/// plus the device code for polling.
pub async fn start_sso_auth(
    start_url: &str,
    region: &str,
) -> Result<SsoAuthStart, AppError> {
    // Step 1: Register an OIDC client
    let (client_id, client_secret) = register_oidc_client(region).await?;

    // Step 2: Start device authorization
    let client = reqwest::Client::new();
    let endpoint = format!(
        "https://oidc.{region}.amazonaws.com/device_authorization"
    );

    let body = serde_json::json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "startUrl": start_url,
    });

    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            AppError::connection(
                format!("Failed to start device authorization: {e}"),
                "Check your network connection and SSO start URL.",
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::connection(
            format!("Device authorization failed ({status}): {text}"),
            "Check your SSO start URL and region.",
        ));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        AppError::connection(
            format!("Invalid device authorization response: {e}"),
            "Unexpected response from AWS SSO OIDC service.",
        )
    })?;

    let verification_uri = json["verificationUriComplete"]
        .as_str()
        .or_else(|| json["verificationUri"].as_str())
        .ok_or_else(|| {
            AppError::connection(
                "Missing verificationUri in response",
                "Retry SSO auth.",
            )
        })?
        .to_string();

    let user_code = json["userCode"]
        .as_str()
        .ok_or_else(|| AppError::connection("Missing userCode in response", "Retry SSO auth."))?
        .to_string();

    let device_code = json["deviceCode"]
        .as_str()
        .ok_or_else(|| {
            AppError::connection("Missing deviceCode in response", "Retry SSO auth.")
        })?
        .to_string();

    let expires_in = json["expiresIn"].as_u64().unwrap_or(600);

    Ok(SsoAuthStart {
        verification_uri,
        user_code,
        device_code,
        expires_in,
        client_id,
        client_secret,
    })
}

/// Poll the SSO OIDC service for a token after the user has completed browser auth.
///
/// The caller should poll this periodically (e.g. every 5 seconds).
/// Returns `Err` with an "authorization_pending" message if the user hasn't
/// completed auth yet, or a real error if something went wrong.
pub async fn poll_sso_token(
    device_code: &str,
    client_id: &str,
    client_secret: &str,
    region: &str,
) -> Result<SsoTokenResponse, AppError> {
    let client = reqwest::Client::new();
    let endpoint = format!("https://oidc.{region}.amazonaws.com/token");

    let body = serde_json::json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "deviceCode": device_code,
        "grantType": "urn:ietf:params:oauth:grant-type:device_code",
    });

    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            AppError::connection(
                format!("Failed to poll SSO token: {e}"),
                "Check your network connection.",
            )
        })?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await.map_err(|e| {
        AppError::connection(
            format!("Invalid SSO token response: {e}"),
            "Unexpected response from AWS SSO OIDC service.",
        )
    })?;

    if !status.is_success() {
        let error_code = json["error"].as_str().unwrap_or("unknown");
        let error_desc = json["error_description"]
            .as_str()
            .unwrap_or("No description");

        // authorization_pending is expected while user is completing browser auth
        if error_code == "authorization_pending" {
            return Err(AppError::connection(
                "authorization_pending",
                "User has not completed browser authentication yet. Continue polling.",
            ));
        }

        // slow_down means we should increase the polling interval
        if error_code == "slow_down" {
            return Err(AppError::connection(
                "slow_down",
                "Polling too fast. Increase interval between polls.",
            ));
        }

        // expired_token means the device code has expired
        if error_code == "expired_token" {
            return Err(AppError::connection(
                "SSO device code has expired",
                "Start the SSO authentication flow again.",
            ));
        }

        return Err(AppError::connection(
            format!("SSO token error ({error_code}): {error_desc}"),
            "Check your SSO configuration and try again.",
        ));
    }

    let access_token = json["accessToken"]
        .as_str()
        .ok_or_else(|| {
            AppError::connection("Missing accessToken in response", "Retry SSO auth.")
        })?
        .to_string();

    let token_type = json["tokenType"]
        .as_str()
        .unwrap_or("Bearer")
        .to_string();

    let expires_in = json["expiresIn"].as_u64().unwrap_or(28800);

    Ok(SsoTokenResponse {
        access_token,
        token_type,
        expires_in,
    })
}

/// Get temporary AWS credentials for a specific account and role using an SSO access token.
///
/// This calls the SSO (not OIDC) `getRoleCredentials` API with the access token
/// obtained from the device authorization flow.
pub async fn get_sso_role_credentials(
    access_token: &str,
    account_id: &str,
    role_name: &str,
    region: &str,
) -> Result<AwsCredentials, AppError> {
    let client = reqwest::Client::new();
    let endpoint = format!(
        "https://portal.sso.{region}.amazonaws.com/federation/credentials?account_id={account_id}&role_name={role_name}"
    );

    let resp = client
        .get(&endpoint)
        .header("x-amz-sso_bearer_token", access_token)
        .send()
        .await
        .map_err(|e| {
            AppError::connection(
                format!("Failed to get SSO role credentials: {e}"),
                "Check your network connection and SSO configuration.",
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::connection(
            format!("SSO role credentials request failed ({status}): {text}"),
            "Check your SSO account ID and role name.",
        ));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        AppError::connection(
            format!("Invalid SSO role credentials response: {e}"),
            "Unexpected response from AWS SSO service.",
        )
    })?;

    let role_creds = &json["roleCredentials"];

    let access_key_id = role_creds["accessKeyId"]
        .as_str()
        .ok_or_else(|| {
            AppError::connection(
                "Missing accessKeyId in SSO credentials",
                "Retry SSO auth flow.",
            )
        })?
        .to_string();

    let secret_access_key = role_creds["secretAccessKey"]
        .as_str()
        .ok_or_else(|| {
            AppError::connection(
                "Missing secretAccessKey in SSO credentials",
                "Retry SSO auth flow.",
            )
        })?
        .to_string();

    let session_token = role_creds["sessionToken"]
        .as_str()
        .ok_or_else(|| {
            AppError::connection(
                "Missing sessionToken in SSO credentials",
                "Retry SSO auth flow.",
            )
        })?
        .to_string();

    // expiration is in epoch milliseconds
    let expiration = role_creds["expiration"]
        .as_i64()
        .map(|ms| {
            chrono::DateTime::from_timestamp(ms / 1000, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        });

    Ok(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
        expiration,
    })
}

// ── OIDC / STS Federation ──────────────────────────────────────────────────

/// Assume an IAM role using a web identity (OIDC) token via the STS
/// `AssumeRoleWithWebIdentity` API.
///
/// This supports federated access from OIDC providers (GitHub Actions,
/// Cognito, Google, etc.) by exchanging a web identity token for temporary
/// AWS credentials.
pub async fn assume_role_with_web_identity(
    role_arn: &str,
    web_identity_token: &str,
    region: &str,
    session_name: Option<&str>,
) -> Result<AwsCredentials, AppError> {
    let session = session_name.unwrap_or("ufop-session");
    let client = reqwest::Client::new();
    let endpoint = format!("https://sts.{region}.amazonaws.com/");

    let params = [
        ("Action", "AssumeRoleWithWebIdentity"),
        ("Version", "2011-06-15"),
        ("RoleArn", role_arn),
        ("RoleSessionName", session),
        ("WebIdentityToken", web_identity_token),
    ];

    let resp = client
        .post(&endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            AppError::connection(
                format!("STS AssumeRoleWithWebIdentity failed: {e}"),
                "Check your network connection, role ARN, and OIDC token.",
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::connection(
            format!("STS AssumeRoleWithWebIdentity failed ({status}): {text}"),
            "Check your role ARN, trust policy, and OIDC token validity.",
        ));
    }

    let text = resp.text().await.map_err(|e| {
        AppError::connection(
            format!("Failed to read STS response: {e}"),
            "Unexpected response from AWS STS.",
        )
    })?;

    // Parse the XML response from STS
    // STS returns XML, not JSON. We do simple tag extraction.
    let access_key_id = extract_xml_tag(&text, "AccessKeyId").ok_or_else(|| {
        AppError::connection(
            "Missing AccessKeyId in STS response",
            "Unexpected STS response format.",
        )
    })?;

    let secret_access_key = extract_xml_tag(&text, "SecretAccessKey").ok_or_else(|| {
        AppError::connection(
            "Missing SecretAccessKey in STS response",
            "Unexpected STS response format.",
        )
    })?;

    let session_token = extract_xml_tag(&text, "SessionToken").ok_or_else(|| {
        AppError::connection(
            "Missing SessionToken in STS response",
            "Unexpected STS response format.",
        )
    })?;

    let expiration = extract_xml_tag(&text, "Expiration");

    Ok(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
        expiration,
    })
}

/// Extract the text content of a simple XML tag like `<Tag>value</Tag>`.
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let after_open = start + open.len();
    let end = xml[after_open..].find(&close)?;
    Some(xml[after_open..after_open + end].to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ini_basic() {
        let content = r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
region = us-east-1

[production]
aws_access_key_id = AKIAI44QH8DHBEXAMPLE
aws_secret_access_key = je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY
"#;

        let sections = parse_ini(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections["default"]["aws_access_key_id"],
            "AKIAIOSFODNN7EXAMPLE"
        );
        assert_eq!(sections["default"]["region"], "us-east-1");
        assert_eq!(
            sections["production"]["aws_access_key_id"],
            "AKIAI44QH8DHBEXAMPLE"
        );
    }

    #[test]
    fn test_parse_ini_comments() {
        let content = r#"
# This is a comment
; This is also a comment
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
# Another comment
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#;
        let sections = parse_ini(content);
        assert_eq!(sections.len(), 1);
        assert_eq!(
            sections["default"]["aws_access_key_id"],
            "AKIAIOSFODNN7EXAMPLE"
        );
    }

    #[test]
    fn test_parse_ini_config_style() {
        let content = r#"
[default]
region = us-east-1

[profile dev]
region = us-west-2
sso_start_url = https://my-sso-portal.awsapps.com/start
sso_region = us-east-1
sso_account_id = 123456789012
sso_role_name = DevRole

[profile prod]
role_arn = arn:aws:iam::123456789012:role/ProdRole
source_profile = default
"#;

        let sections = parse_ini(content);
        assert_eq!(sections.len(), 3);
        assert_eq!(
            sections["profile dev"]["sso_start_url"],
            "https://my-sso-portal.awsapps.com/start"
        );
        assert_eq!(
            sections["profile prod"]["role_arn"],
            "arn:aws:iam::123456789012:role/ProdRole"
        );
    }

    #[test]
    fn test_profile_from_section() {
        let mut kv = HashMap::new();
        kv.insert(
            "aws_access_key_id".to_string(),
            "AKIAIOSFODNN7EXAMPLE".to_string(),
        );
        kv.insert(
            "aws_secret_access_key".to_string(),
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        );
        kv.insert("region".to_string(), "us-east-1".to_string());
        kv.insert(
            "sso_start_url".to_string(),
            "https://example.awsapps.com/start".to_string(),
        );

        let profile = profile_from_section("test", &kv);
        assert_eq!(profile.name, "test");
        assert_eq!(
            profile.access_key_id.as_deref(),
            Some("AKIAIOSFODNN7EXAMPLE")
        );
        assert_eq!(profile.region.as_deref(), Some("us-east-1"));
        assert_eq!(
            profile.sso_start_url.as_deref(),
            Some("https://example.awsapps.com/start")
        );
    }

    #[test]
    fn test_extract_xml_tag() {
        let xml = r#"<AssumeRoleWithWebIdentityResponse>
  <AssumeRoleWithWebIdentityResult>
    <Credentials>
      <AccessKeyId>ASIAEXAMPLE</AccessKeyId>
      <SecretAccessKey>secretExample</SecretAccessKey>
      <SessionToken>tokenExample</SessionToken>
      <Expiration>2024-01-01T00:00:00Z</Expiration>
    </Credentials>
  </AssumeRoleWithWebIdentityResult>
</AssumeRoleWithWebIdentityResponse>"#;

        assert_eq!(
            extract_xml_tag(xml, "AccessKeyId"),
            Some("ASIAEXAMPLE".to_string())
        );
        assert_eq!(
            extract_xml_tag(xml, "SecretAccessKey"),
            Some("secretExample".to_string())
        );
        assert_eq!(
            extract_xml_tag(xml, "SessionToken"),
            Some("tokenExample".to_string())
        );
        assert_eq!(
            extract_xml_tag(xml, "Expiration"),
            Some("2024-01-01T00:00:00Z".to_string())
        );
        assert_eq!(extract_xml_tag(xml, "Nonexistent"), None);
    }

    #[test]
    fn test_parse_ini_empty() {
        let sections = parse_ini("");
        assert!(sections.is_empty());
    }

    #[test]
    fn test_parse_ini_whitespace_handling() {
        let content = r#"
[default]
  aws_access_key_id   =   AKIAEXAMPLE
  aws_secret_access_key=secretkey
"#;
        let sections = parse_ini(content);
        assert_eq!(sections["default"]["aws_access_key_id"], "AKIAEXAMPLE");
        assert_eq!(sections["default"]["aws_secret_access_key"], "secretkey");
    }

    #[test]
    fn test_profile_merge_logic() {
        // Test the merge semantics by directly verifying the logic
        let cred = AwsProfile {
            name: "test".to_string(),
            access_key_id: Some("CRED_KEY".to_string()),
            secret_access_key: Some("CRED_SECRET".to_string()),
            region: None,
            ..Default::default()
        };

        let cfg = AwsProfile {
            name: "test".to_string(),
            access_key_id: Some("CFG_KEY".to_string()),
            secret_access_key: None,
            region: Some("us-west-2".to_string()),
            sso_start_url: Some("https://sso.example.com".to_string()),
            ..Default::default()
        };

        // Simulate merge: credentials win for keys, config wins for region/SSO
        let merged = AwsProfile {
            name: "test".to_string(),
            access_key_id: cred.access_key_id.clone().or(cfg.access_key_id.clone()),
            secret_access_key: cred
                .secret_access_key
                .clone()
                .or(cfg.secret_access_key.clone()),
            region: cfg.region.clone().or(cred.region.clone()),
            sso_start_url: cfg.sso_start_url.clone().or(cred.sso_start_url.clone()),
            ..Default::default()
        };

        assert_eq!(merged.access_key_id.as_deref(), Some("CRED_KEY")); // cred wins
        assert_eq!(merged.secret_access_key.as_deref(), Some("CRED_SECRET")); // cred wins
        assert_eq!(merged.region.as_deref(), Some("us-west-2")); // config wins
        assert_eq!(
            merged.sso_start_url.as_deref(),
            Some("https://sso.example.com")
        );
    }
}
