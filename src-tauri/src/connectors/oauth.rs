//! Shared OAuth 2.0 helper for Google Drive, Dropbox, and OneDrive connectors (T-027/T-028/T-029).
//!
//! Provides:
//! - Authorization URL generation with PKCE
//! - Token exchange (authorization code -> access + refresh tokens)
//! - Automatic token refresh before expiry
//! - Token storage in CredentialStore (OS keychain)
//! - Localhost redirect listener for desktop OAuth flows

use crate::core::error::AppError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// OAuth 2.0 token pair with expiry tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub token_type: String,
    pub scope: Option<String>,
}

impl OAuthToken {
    /// Check if the token is expired or will expire within 60 seconds.
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() + Duration::seconds(60) >= exp,
            None => false, // Tokens without expiry are assumed valid
        }
    }
}

/// What an OAuth connection keeps in the credential store (under its
/// credential key): the tokens and the app they were issued to, or an
/// AuthID from a self-hosted broker (`server/oauth.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OAuthGrant {
    Token {
        token: OAuthToken,
        /// Client the refresh token belongs to; `None` = the profile's / built-in one.
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default)]
        client_secret: Option<String>,
        /// Token endpoint override (pasted credentials, tests).
        #[serde(default)]
        token_url: Option<String>,
    },
    /// Access tokens come from `{broker_url}/oauth/token?authid=…`; the
    /// broker keeps the refresh token.
    Broker { broker_url: String, authid: String },
}

impl OAuthGrant {
    /// Parse a stored or pasted credential: this enum's JSON, a bare
    /// [`OAuthToken`], the backup-destination format
    /// `{access_token, refresh_token, client_id, client_secret, token_url}`,
    /// or a bare access token.
    pub fn parse(secret: &str) -> Result<Self, AppError> {
        let s = secret.trim();
        if s.is_empty() {
            return Err(AppError::validation("The OAuth credentials are empty"));
        }
        if !s.starts_with('{') {
            return Ok(Self::Token { token: OAuthToken::bearer(s.to_string(), None, None), client_id: None, client_secret: None, token_url: None });
        }
        let v: serde_json::Value = serde_json::from_str(s).map_err(|e| AppError::validation(format!("Invalid OAuth credentials: {e}")))?;
        if v.get("kind").is_some() {
            return serde_json::from_value(v).map_err(|e| AppError::validation(format!("Invalid OAuth credentials: {e}")));
        }
        let text = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(String::from);
        if let (Some(broker_url), Some(authid)) = (text("broker_url"), text("authid")) {
            return Ok(Self::Broker { broker_url, authid });
        }
        let access = text("access_token").unwrap_or_default();
        let refresh = text("refresh_token");
        if access.is_empty() && refresh.is_none() {
            return Err(AppError::validation("The OAuth credentials need an access or refresh token"));
        }
        let expires_at = v.get("expires_at").and_then(|x| x.as_str()).and_then(|t| t.parse::<DateTime<Utc>>().ok());
        // No access token yet: expired, so the first use refreshes it.
        let expires_at = if access.is_empty() { Some(Utc::now() - Duration::seconds(1)) } else { expires_at };
        Ok(Self::Token {
            token: OAuthToken::bearer(access, refresh, expires_at),
            client_id: text("client_id"),
            client_secret: text("client_secret"),
            token_url: text("token_url"),
        })
    }

    pub fn to_json(&self) -> Result<String, AppError> {
        serde_json::to_string(self).map_err(|e| AppError::internal(format!("Failed to serialize OAuth grant: {e}")))
    }
}

impl OAuthToken {
    pub fn bearer(access_token: String, refresh_token: Option<String>, expires_at: Option<DateTime<Utc>>) -> Self {
        Self { access_token, refresh_token, expires_at, token_type: "Bearer".into(), scope: None }
    }
}

/// Where rotated tokens are written (the connection's credential-store entry).
pub trait TokenSink: Send + Sync {
    fn save(&self, grant_json: String) -> crate::core::traits::BoxFuture<'_, Result<(), AppError>>;
}

/// Persist into the credential store under `key`.
pub struct CredentialSink {
    pub store: Arc<crate::security::CredentialStore>,
    pub key: String,
}

impl TokenSink for CredentialSink {
    fn save(&self, grant_json: String) -> crate::core::traits::BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move { self.store.store(&self.key, &grant_json).await })
    }
}

/// How the access token is renewed.
#[derive(Debug, Clone)]
enum RefreshVia {
    /// `grant_type=refresh_token` at the provider's token endpoint.
    Provider,
    /// `GET {broker_url}/oauth/token?authid=…` (self-hosted AuthID broker).
    Broker { broker_url: String, authid: String },
}

/// A refused token request: `invalid_grant` / 400 / 401 mean the sign-in is
/// no longer valid (revoked, expired, changed password).
/// A sign-in broker URL the AuthID (a bearer secret) may be sent to:
/// https, or plain http only to this machine (a broker on localhost).
pub fn check_broker_url(url: &str) -> Result<(), AppError> {
    let refuse = || AppError::validation("The broker URL must start with https:// (plain http:// only for a broker on this computer: localhost, 127.0.0.1 or [::1])");
    let parsed = url::Url::parse(url).map_err(|_| refuse())?;
    let loopback = match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match parsed.scheme() {
        "https" if parsed.host().is_some() => Ok(()),
        "http" if loopback => Ok(()),
        _ => Err(refuse()),
    }
}

fn token_error(what: &str, status: reqwest::StatusCode, body: &str) -> AppError {
    let code = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v["error"].as_str().map(String::from)).unwrap_or_default();
    if matches!(status.as_u16(), 400 | 401 | 403) || code == "invalid_grant" || code == "invalid_client" {
        AppError::auth(
            format!("{what} was refused (HTTP {status}{})", if code.is_empty() { String::new() } else { format!(": {code}") }),
            "Sign in again from Connections (the access was revoked, expired or belongs to another app).",
        )
    } else {
        AppError::Connection { message: format!("{what} failed (HTTP {status})"), advice: "Check your network connection and try again.".to_string() }
    }
}

/// OAuth 2.0 provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    /// Extra parameters to include in the auth URL (provider-specific).
    pub extra_params: Vec<(String, String)>,
}

/// PKCE code verifier and challenge for secure OAuth flows.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge pair using S256.
    pub fn generate() -> Self {
        use base64::Engine;
        use sha2::Digest;

        // Generate 32 random bytes for the code verifier
        let mut verifier_bytes = [0u8; 32];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut verifier_bytes);
        let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

        // SHA-256 hash of verifier, then base64url encode
        let mut hasher = sha2::Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        Self {
            code_verifier,
            code_challenge,
            code_challenge_method: "S256".to_string(),
        }
    }
}

/// Token response from the OAuth provider.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    token_type: Option<String>,
    scope: Option<String>,
}

/// OAuth 2.0 client that handles the full flow.
pub struct OAuthClient {
    config: OAuthConfig,
    http: reqwest::Client,
    token: Arc<RwLock<Option<OAuthToken>>>,
    pkce: Arc<RwLock<Option<PkceChallenge>>>,
    refresh_via: RefreshVia,
    sink: Option<Arc<dyn TokenSink>>,
}

impl OAuthClient {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http: crate::net::client(),
            token: Arc::new(RwLock::new(None)),
            pkce: Arc::new(RwLock::new(None)),
            refresh_via: RefreshVia::Provider,
            sink: None,
        }
    }

    /// Renew access tokens through a self-hosted AuthID broker.
    pub fn with_broker(mut self, broker_url: &str, authid: &str) -> Self {
        self.refresh_via = RefreshVia::Broker { broker_url: broker_url.trim_end_matches('/').to_string(), authid: authid.to_string() };
        self
    }

    /// Persist rotated tokens through `sink`.
    pub fn with_sink(mut self, sink: Arc<dyn TokenSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    /// Restore the tokens of a stored grant (broker grants start without one).
    pub async fn restore_grant(&self, grant: &OAuthGrant) {
        if let OAuthGrant::Token { token, .. } = grant {
            self.set_token(token.clone()).await;
        }
    }

    /// The grant to store for the current token.
    pub async fn grant(&self) -> Option<OAuthGrant> {
        match &self.refresh_via {
            RefreshVia::Broker { broker_url, authid } => Some(OAuthGrant::Broker { broker_url: broker_url.clone(), authid: authid.clone() }),
            RefreshVia::Provider => self.current_token().await.map(|token| OAuthGrant::Token {
                token,
                client_id: Some(self.config.client_id.clone()).filter(|c| !c.is_empty()),
                client_secret: self.config.client_secret.clone(),
                token_url: Some(self.config.token_url.clone()),
            }),
        }
    }

    async fn persist(&self) {
        let Some(sink) = &self.sink else { return };
        if matches!(self.refresh_via, RefreshVia::Broker { .. }) {
            return; // the broker keeps the tokens
        }
        let Some(grant) = self.grant().await else { return };
        match grant.to_json() {
            Ok(json) => {
                if let Err(e) = sink.save(json).await {
                    tracing::warn!("Could not save the renewed OAuth token: {e}");
                }
            }
            Err(e) => tracing::warn!("{e}"),
        }
    }

    /// [`OAuthClient::authorization_url`] without panicking on a bad auth URL.
    pub async fn try_authorization_url(&self) -> Result<(String, String), AppError> {
        url::Url::parse(&self.config.auth_url).map_err(|e| AppError::Configuration {
            message: format!("Invalid OAuth authorization URL: {e}"),
            advice: "Check the provider settings.".to_string(),
        })?;
        Ok(self.authorization_url().await)
    }

    /// Generate the authorization URL that the user should open in their browser.
    /// Returns (url, state) where state should be verified on callback.
    pub async fn authorization_url(&self) -> (String, String) {
        let pkce = PkceChallenge::generate();
        let state = uuid::Uuid::new_v4().to_string();

        let mut url = url::Url::parse(&self.config.auth_url)
            .expect("Invalid auth URL in OAuth config");

        {
            let mut params = url.query_pairs_mut();
            params.append_pair("response_type", "code");
            params.append_pair("client_id", &self.config.client_id);
            params.append_pair("redirect_uri", &self.config.redirect_uri);
            params.append_pair("state", &state);
            params.append_pair("code_challenge", &pkce.code_challenge);
            params.append_pair("code_challenge_method", &pkce.code_challenge_method);

            if !self.config.scopes.is_empty() {
                params.append_pair("scope", &self.config.scopes.join(" "));
            }

            for (key, value) in &self.config.extra_params {
                params.append_pair(key, value);
            }
        }

        // Store PKCE for later exchange
        {
            let mut pkce_lock = self.pkce.write().await;
            *pkce_lock = Some(pkce);
        }

        (url.to_string(), state)
    }

    /// Exchange an authorization code for tokens.
    pub async fn exchange_code(&self, code: &str) -> Result<OAuthToken, AppError> {
        let pkce = {
            let pkce_lock = self.pkce.read().await;
            pkce_lock.clone()
        };

        let mut params = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", self.config.redirect_uri.clone()),
            ("client_id", self.config.client_id.clone()),
        ];

        if let Some(ref secret) = self.config.client_secret {
            params.push(("client_secret", secret.clone()));
        }

        if let Some(ref pkce) = pkce {
            params.push(("code_verifier", pkce.code_verifier.clone()));
        }

        let resp = self.http
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Token exchange request failed: {e}"),
                advice: "Check your network connection and try again.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("OAuth token exchange failed: {status} - {body}");
            return Err(token_error("The sign-in code exchange", status, &body));
        }

        let token_resp: TokenResponse = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse token response: {e}"),
            advice: "The OAuth provider returned an unexpected response.".to_string(),
        })?;

        let token = OAuthToken {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at: token_resp.expires_in.map(|secs| Utc::now() + Duration::seconds(secs)),
            token_type: token_resp.token_type.unwrap_or_else(|| "Bearer".to_string()),
            scope: token_resp.scope,
        };

        // Store in memory
        {
            let mut token_lock = self.token.write().await;
            *token_lock = Some(token.clone());
        }
        self.persist().await;

        Ok(token)
    }

    /// Refresh the access token using the refresh token (or the broker).
    pub async fn refresh_token(&self) -> Result<OAuthToken, AppError> {
        if let RefreshVia::Broker { broker_url, authid } = self.refresh_via.clone() {
            return self.refresh_from_broker(&broker_url, &authid).await;
        }
        let current = {
            let token_lock = self.token.read().await;
            token_lock.clone()
        };

        let refresh_token = current
            .as_ref()
            .and_then(|t| t.refresh_token.clone())
            .ok_or_else(|| AppError::auth(
                "The access token expired and there is no refresh token",
                "Sign in again from Connections.",
            ))?;

        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.clone()),
            ("client_id", self.config.client_id.clone()),
        ];

        if let Some(ref secret) = self.config.client_secret {
            params.push(("client_secret", secret.clone()));
        }

        let resp = self.http
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Token refresh request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("OAuth token refresh failed: {status} - {body}");
            return Err(token_error("Renewing the sign-in", status, &body));
        }

        let token_resp: TokenResponse = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse refresh response: {e}"),
            advice: "The OAuth provider returned an unexpected response.".to_string(),
        })?;

        let new_token = OAuthToken {
            access_token: token_resp.access_token,
            // Some providers return a new refresh token, others keep the old one
            refresh_token: token_resp.refresh_token.or(Some(refresh_token)),
            expires_at: token_resp.expires_in.map(|secs| Utc::now() + Duration::seconds(secs)),
            token_type: token_resp.token_type.unwrap_or_else(|| "Bearer".to_string()),
            scope: token_resp.scope,
        };

        // Update stored token
        {
            let mut token_lock = self.token.write().await;
            *token_lock = Some(new_token.clone());
        }
        // Providers may rotate the refresh token: keep the stored copy current.
        self.persist().await;

        tracing::info!("OAuth token refreshed successfully");
        Ok(new_token)
    }

    /// A fresh access token from the AuthID broker (`server/oauth.rs`):
    /// `{"access_token": "…", "expires_at": <unix seconds>}`.
    async fn refresh_from_broker(&self, broker_url: &str, authid: &str) -> Result<OAuthToken, AppError> {
        check_broker_url(broker_url)?;
        let resp = self
            .http
            .get(format!("{broker_url}/oauth/token"))
            .query(&[("authid", authid)])
            .send()
            .await
            .map_err(|e| AppError::connection(format!("The sign-in broker at {broker_url} is unreachable: {e}"), "Check the broker URL and your network."))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(token_error("The sign-in broker", status, &body));
        }
        let v: serde_json::Value = resp.json().await.map_err(|e| AppError::connection(format!("Unexpected broker reply: {e}"), "Check the broker URL."))?;
        let access = v["access_token"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| AppError::connection("The broker returned no access token", "Check the broker."))?;
        let expires_at = v["expires_at"].as_i64().and_then(|t| DateTime::from_timestamp(t, 0));
        let token = OAuthToken::bearer(access.to_string(), None, expires_at);
        *self.token.write().await = Some(token.clone());
        Ok(token)
    }

    /// Get a valid access token, refreshing if needed.
    pub async fn get_valid_token(&self) -> Result<String, AppError> {
        let current = {
            let token_lock = self.token.read().await;
            token_lock.clone()
        };

        match current {
            Some(token) if !token.is_expired() => Ok(token.access_token),
            Some(_) => {
                tracing::info!("Access token expired, refreshing...");
                let new_token = self.refresh_token().await?;
                Ok(new_token.access_token)
            }
            None if matches!(self.refresh_via, RefreshVia::Broker { .. }) => Ok(self.refresh_token().await?.access_token),
            None => Err(AppError::Connection {
                message: "Not authenticated".to_string(),
                advice: "Connect to the service first.".to_string(),
            }),
        }
    }

    /// Set a token directly (e.g., restored from credential store).
    pub async fn set_token(&self, token: OAuthToken) {
        let mut token_lock = self.token.write().await;
        *token_lock = Some(token);
    }

    /// Get the current token for serialization/storage.
    pub async fn current_token(&self) -> Option<OAuthToken> {
        let token_lock = self.token.read().await;
        token_lock.clone()
    }

    /// Check if we have a token (connected).
    pub async fn has_token(&self) -> bool {
        let token_lock = self.token.read().await;
        token_lock.is_some()
    }

    /// Clear the stored token (disconnect).
    pub async fn clear_token(&self) {
        let mut token_lock = self.token.write().await;
        *token_lock = None;
    }

    /// Serialize token to JSON for credential store persistence.
    pub async fn serialize_token(&self) -> Result<Option<String>, AppError> {
        let token_lock = self.token.read().await;
        match &*token_lock {
            Some(token) => {
                let json = serde_json::to_string(token).map_err(|e| {
                    AppError::internal(format!("Failed to serialize OAuth token: {e}"))
                })?;
                Ok(Some(json))
            }
            None => Ok(None),
        }
    }

    /// Deserialize and restore a token from JSON.
    pub async fn restore_token(&self, json: &str) -> Result<(), AppError> {
        let token: OAuthToken = serde_json::from_str(json).map_err(|e| AppError::Connection {
            message: format!("Failed to deserialize OAuth token: {e}"),
            advice: "Re-authorize the connection.".to_string(),
        })?;
        let mut token_lock = self.token.write().await;
        *token_lock = Some(token);
        Ok(())
    }
}

/// Start a temporary localhost HTTP server to receive the OAuth callback.
/// Returns the authorization code and state received.
pub async fn listen_for_callback(port: u16) -> Result<(String, String), AppError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).await.map_err(|e| AppError::Connection {
        message: format!("Failed to start OAuth callback listener on {addr}: {e}"),
        advice: "Try a different port or close the application using this port.".to_string(),
    })?;

    tracing::info!("OAuth callback listener started on {addr}");

    let (mut stream, _) = listener.accept().await.map_err(|e| AppError::Connection {
        message: format!("Failed to accept OAuth callback: {e}"),
        advice: "Try re-authorizing.".to_string(),
    })?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.map_err(|e| AppError::Connection {
        message: format!("Failed to read OAuth callback: {e}"),
        advice: "Try re-authorizing.".to_string(),
    })?;

    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the GET request for code and state parameters
    let (code, state) = parse_callback_request(&request)?;

    // Send a success response to the browser
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h1>Authorization successful!</h1>\
        <p>You can close this window and return to the application.</p></body></html>";
    let _ = stream.write_all(response.as_bytes()).await;

    Ok((code, state))
}

/// Parse the OAuth callback GET request to extract code and state.
fn parse_callback_request(request: &str) -> Result<(String, String), AppError> {
    // Extract the path from "GET /callback?code=xxx&state=yyy HTTP/1.1"
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| AppError::Connection {
            message: "Invalid OAuth callback request".to_string(),
            advice: "Try re-authorizing.".to_string(),
        })?;

    let url = url::Url::parse(&format!("http://localhost{path}")).map_err(|_| {
        AppError::Connection {
            message: "Failed to parse OAuth callback URL".to_string(),
            advice: "Try re-authorizing.".to_string(),
        }
    })?;

    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| {
            // Check for error
            let error = url
                .query_pairs()
                .find(|(k, _)| k == "error")
                .map(|(_, v)| v.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let desc = url
                .query_pairs()
                .find(|(k, _)| k == "error_description")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            AppError::Connection {
                message: format!("OAuth authorization denied: {error} {desc}"),
                advice: "Grant the required permissions and try again.".to_string(),
            }
        })?;

    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();

    Ok((code, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generation() {
        let pkce = PkceChallenge::generate();
        assert!(!pkce.code_verifier.is_empty());
        assert!(!pkce.code_challenge.is_empty());
        assert_eq!(pkce.code_challenge_method, "S256");
        // Verifier and challenge should be different
        assert_ne!(pkce.code_verifier, pkce.code_challenge);
    }

    #[test]
    fn test_token_expiry_check() {
        let expired = OAuthToken {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: Some(Utc::now() - Duration::seconds(100)),
            token_type: "Bearer".to_string(),
            scope: None,
        };
        assert!(expired.is_expired());

        let valid = OAuthToken {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: Some(Utc::now() + Duration::seconds(3600)),
            token_type: "Bearer".to_string(),
            scope: None,
        };
        assert!(!valid.is_expired());

        // About to expire (within 60s buffer)
        let almost_expired = OAuthToken {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: Some(Utc::now() + Duration::seconds(30)),
            token_type: "Bearer".to_string(),
            scope: None,
        };
        assert!(almost_expired.is_expired());

        // No expiry = never expired
        let no_expiry = OAuthToken {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: None,
            token_type: "Bearer".to_string(),
            scope: None,
        };
        assert!(!no_expiry.is_expired());
    }

    #[test]
    fn test_parse_callback_success() {
        let request = "GET /callback?code=abc123&state=xyz789 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state) = parse_callback_request(request).unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "xyz789");
    }

    #[test]
    fn test_parse_callback_error() {
        let request = "GET /callback?error=access_denied&error_description=User+denied HTTP/1.1\r\n\r\n";
        let result = parse_callback_request(request);
        assert!(result.is_err());
    }

    #[test]
    fn test_token_serialization() {
        let token = OAuthToken {
            access_token: "at_123".to_string(),
            refresh_token: Some("rt_456".to_string()),
            expires_at: Some(Utc::now() + Duration::seconds(3600)),
            token_type: "Bearer".to_string(),
            scope: Some("read write".to_string()),
        };

        let json = serde_json::to_string(&token).unwrap();
        let restored: OAuthToken = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.access_token, "at_123");
        assert_eq!(restored.refresh_token.as_deref(), Some("rt_456"));
    }

    #[tokio::test]
    async fn test_oauth_client_token_lifecycle() {
        let config = OAuthConfig {
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            redirect_uri: "http://localhost:9876/callback".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            extra_params: vec![],
        };

        let client = OAuthClient::new(config);

        // No token initially
        assert!(!client.has_token().await);
        assert!(client.current_token().await.is_none());

        // Set a token
        let token = OAuthToken {
            access_token: "test_token".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(Utc::now() + Duration::seconds(3600)),
            token_type: "Bearer".to_string(),
            scope: None,
        };
        client.set_token(token).await;
        assert!(client.has_token().await);

        // Get valid token
        let access = client.get_valid_token().await.unwrap();
        assert_eq!(access, "test_token");

        // Serialize and restore
        let json = client.serialize_token().await.unwrap().unwrap();
        client.clear_token().await;
        assert!(!client.has_token().await);

        client.restore_token(&json).await.unwrap();
        assert!(client.has_token().await);
    }

    #[tokio::test]
    async fn test_authorization_url_generation() {
        let config = OAuthConfig {
            client_id: "my_client_id".to_string(),
            client_secret: None,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            redirect_uri: "http://localhost:9876/callback".to_string(),
            scopes: vec!["https://www.googleapis.com/auth/drive".to_string()],
            extra_params: vec![("access_type".to_string(), "offline".to_string())],
        };

        let client = OAuthClient::new(config);
        let (url, state) = client.authorization_url().await;

        assert!(url.contains("accounts.google.com"));
        assert!(url.contains("my_client_id"));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("access_type=offline"));
        assert!(!state.is_empty());
    }
}
