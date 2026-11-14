//! Self-hosted OAuth broker (AuthID flow).
//!
//! An administrator registers OAuth applications (their own client id and
//! secret) for providers such as Google, Microsoft, Dropbox or Box. A user
//! opens `/oauth/{provider}/start`, consents at the provider, and gets an
//! **AuthID**. Backup destinations then call `/oauth/token?authid=…` to
//! obtain a fresh access token; the broker keeps the refresh token and
//! renews access tokens when they expire.
//!
//! Tokens are stored through any [`Backend`] (a local folder, or a saved
//! S3/SFTP/WebDAV connection), encrypted with a key derived from the
//! AuthID, so the token store alone reveals nothing.

use super::{ApiError, ServerState};
use crate::core::error::AppError;
use axum::extract::{Path as AxPath, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use ufop_core::backup::crypto::{KdfParams, VolumeCipher};
use ufop_core::backup::Backend;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Extra query parameters for the authorization URL
    /// (e.g. `access_type=offline`, `prompt=consent` for Google).
    #[serde(default)]
    pub extra_auth_params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerConfig {
    /// Public base URL of this server, used for the redirect URI.
    pub public_url: String,
    pub providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Serialize, Deserialize)]
struct StoredToken {
    provider: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
}

#[derive(Clone)]
pub struct OAuthBroker {
    cfg: Arc<BrokerConfig>,
    store: Arc<dyn Backend>,
    pending: Arc<Mutex<HashMap<String, (String, i64)>>>,
    http: reqwest::Client,
}

fn cipher_for(authid: &str) -> Result<VolumeCipher, AppError> {
    VolumeCipher::derive(authid, b"ufop-oauth-broker", KdfParams { memory_kib: 8192, iterations: 1, parallelism: 1 })
}

fn file_name(authid: &str) -> String {
    format!("oauth-{}.json", ufop_core::backup::hash_file_name_key(authid))
}

fn rand_id() -> String {
    ufop_core::backup::crypto::generate_passphrase(40).chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

impl OAuthBroker {
    pub fn new(cfg: BrokerConfig, store: Arc<dyn Backend>) -> Self {
        Self { cfg: Arc::new(cfg), store, pending: Arc::new(Mutex::new(HashMap::new())), http: crate::net::client() }
    }

    fn redirect_uri(&self) -> String {
        format!("{}/oauth/callback", self.cfg.public_url.trim_end_matches('/'))
    }

    fn provider(&self, name: &str) -> Result<&ProviderConfig, AppError> {
        self.cfg.providers.get(name).ok_or_else(|| AppError::not_found(format!("OAuth provider '{name}' not found")))
    }

    pub fn start_url(&self, provider: &str) -> Result<String, AppError> {
        let p = self.provider(provider)?;
        let state = rand_id();
        let now = chrono::Utc::now().timestamp();
        {
            let mut pend = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pend.retain(|_, (_, t)| now - *t < 900);
            pend.insert(state.clone(), (provider.to_string(), now));
        }
        let mut url = url::Url::parse(&p.auth_url).map_err(|e| AppError::validation(format!("Bad auth_url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code").append_pair("client_id", &p.client_id).append_pair("redirect_uri", &self.redirect_uri()).append_pair("state", &state);
            if !p.scopes.is_empty() {
                q.append_pair("scope", &p.scopes.join(" "));
            }
            for (k, v) in &p.extra_auth_params {
                q.append_pair(k, v);
            }
        }
        Ok(url.to_string())
    }

    async fn token_request(&self, p: &ProviderConfig, form: &[(&str, &str)]) -> Result<TokenResponse, AppError> {
        let mut f: Vec<(&str, &str)> = form.to_vec();
        f.push(("client_id", &p.client_id));
        f.push(("client_secret", &p.client_secret));
        let r = self.http.post(&p.token_url).form(&f).send().await.map_err(|e| AppError::connection(format!("Token endpoint unreachable: {e}"), "Check the provider token URL."))?;
        if !r.status().is_success() {
            let s = r.status();
            return Err(AppError::connection(format!("Token endpoint returned {s}"), "Check the client id/secret and redirect URI registered at the provider."));
        }
        r.json().await.map_err(|e| AppError::connection(format!("Bad token response: {e}"), "The provider returned an unexpected answer."))
    }

    async fn save(&self, authid: &str, t: &StoredToken) -> Result<(), AppError> {
        let data = cipher_for(authid)?.seal(&serde_json::to_vec(t)?)?;
        let store = self.store.clone();
        let name = file_name(authid);
        tokio::task::spawn_blocking(move || store.put(&name, &data)).await.map_err(|e| AppError::internal(e.to_string()))?
    }

    async fn load(&self, authid: &str) -> Result<StoredToken, AppError> {
        let store = self.store.clone();
        let name = file_name(authid);
        let data = tokio::task::spawn_blocking(move || store.get(&name))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
            .map_err(|_| AppError::PermissionDenied { message: "Unknown AuthID".into(), advice: "Authorize again to get a new AuthID.".into() })?;
        let pt = cipher_for(authid)?.open(&data).map_err(|_| AppError::PermissionDenied { message: "Unknown AuthID".into(), advice: "Authorize again.".into() })?;
        Ok(serde_json::from_slice(&pt)?)
    }

    /// Complete the authorization: returns the new AuthID.
    pub async fn callback(&self, code: &str, state: &str) -> Result<String, AppError> {
        let provider = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(state)
            .map(|(p, _)| p)
            .ok_or_else(|| AppError::validation("This authorization link expired; start again"))?;
        let p = self.provider(&provider)?.clone();
        let redirect = self.redirect_uri();
        let t = self.token_request(&p, &[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &redirect)]).await?;
        let authid = format!("{provider}:{}", rand_id());
        let stored = StoredToken {
            provider,
            access_token: t.access_token,
            refresh_token: t.refresh_token,
            expires_at: chrono::Utc::now().timestamp() + t.expires_in.unwrap_or(3600),
        };
        self.save(&authid, &stored).await?;
        Ok(authid)
    }

    /// A valid access token for `authid`, refreshing when it is about to expire.
    pub async fn access_token(&self, authid: &str) -> Result<(String, i64), AppError> {
        let mut t = self.load(authid).await?;
        if t.expires_at - chrono::Utc::now().timestamp() > 120 {
            return Ok((t.access_token, t.expires_at));
        }
        let rt = t.refresh_token.clone().ok_or_else(|| AppError::PermissionDenied { message: "The provider did not grant a refresh token".into(), advice: "Authorize again with offline access.".into() })?;
        let p = self.provider(&t.provider)?.clone();
        let r = self.token_request(&p, &[("grant_type", "refresh_token"), ("refresh_token", &rt)]).await?;
        t.access_token = r.access_token;
        if r.refresh_token.is_some() {
            t.refresh_token = r.refresh_token;
        }
        t.expires_at = chrono::Utc::now().timestamp() + r.expires_in.unwrap_or(3600);
        self.save(authid, &t).await?;
        Ok((t.access_token, t.expires_at))
    }

    pub fn providers(&self) -> Vec<String> {
        self.cfg.providers.keys().cloned().collect()
    }
}

fn broker(st: &ServerState) -> Result<&OAuthBroker, ApiError> {
    st.oauth.as_ref().ok_or_else(|| ApiError(AppError::state("The OAuth broker is not configured", "Start ufop-server with --oauth-config.")))
}

async fn start(State(st): State<ServerState>, AxPath(provider): AxPath<String>) -> Result<Response, ApiError> {
    Ok(Redirect::temporary(&broker(&st)?.start_url(&provider)?).into_response())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback(State(st): State<ServerState>, Query(q): Query<CallbackQuery>) -> Result<Response, ApiError> {
    if let Some(e) = q.error {
        return Ok(Html(format!("<h1>Authorization failed</h1><p>{}</p>", html_escape(&e))).into_response());
    }
    let id = broker(&st)?.callback(q.code.as_deref().unwrap_or(""), q.state.as_deref().unwrap_or("")).await?;
    Ok(Html(format!(
        "<!doctype html><title>Authorized</title><h1>Authorization complete</h1><p>Your AuthID (paste it into the backup destination):</p><pre>{}</pre>",
        html_escape(&id)
    ))
    .into_response())
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

#[derive(Deserialize)]
struct TokenQuery {
    authid: String,
}

async fn token(State(st): State<ServerState>, Query(q): Query<TokenQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let (t, exp) = broker(&st)?.access_token(&q.authid).await?;
    Ok(Json(serde_json::json!({"access_token": t, "expires_at": exp})))
}

async fn providers(State(st): State<ServerState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!(st.oauth.as_ref().map(|b| b.providers()).unwrap_or_default())))
}

/// `/oauth/*` is public: the AuthID is the credential.
pub fn public_routes() -> Router<ServerState> {
    Router::new().route("/oauth/{provider}/start", get(start)).route("/oauth/callback", get(callback)).route("/oauth/token", get(token))
}

pub fn protected_routes() -> Router<ServerState> {
    Router::new().route("/api/v1/oauth/providers", get(providers))
}
