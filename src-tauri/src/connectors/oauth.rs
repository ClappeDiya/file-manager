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
}

impl OAuthClient {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            token: Arc::new(RwLock::new(None)),
            pkce: Arc::new(RwLock::new(None)),
        }
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
            return Err(AppError::Connection {
                message: format!("OAuth token exchange failed (HTTP {status})"),
                advice: "Re-authorize the connection and try again.".to_string(),
            });
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

        Ok(token)
    }

    /// Refresh the access token using the refresh token.
    pub async fn refresh_token(&self) -> Result<OAuthToken, AppError> {
        let current = {
            let token_lock = self.token.read().await;
            token_lock.clone()
        };

        let refresh_token = current
            .as_ref()
            .and_then(|t| t.refresh_token.clone())
            .ok_or_else(|| AppError::Connection {
                message: "No refresh token available".to_string(),
                advice: "Re-authorize the connection.".to_string(),
            })?;

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
            return Err(AppError::Connection {
                message: format!("Token refresh failed (HTTP {status})"),
                advice: "Re-authorize the connection.".to_string(),
            });
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

        tracing::info!("OAuth token refreshed successfully");
        Ok(new_token)
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
