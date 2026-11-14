//! Desktop OAuth sign-in for Google Drive, Dropbox and OneDrive (RFC 8252):
//! the system browser opens the provider's consent page, a one-shot
//! listener on `127.0.0.1` receives the code, and PKCE binds it to this
//! process ([`crate::fleet_agent::loopback`]). Tokens go to the credential
//! store under the connection's credential key as an [`OAuthGrant`]; each
//! connector's `connect` restores them, refreshes them when they expire and
//! writes rotated tokens back.
//!
//! Which OAuth app (client) signs in: the one passed to the sign-in (your
//! own, Advanced), else the one saved on the profile, else the app built
//! into this build (`UFOP_<PROVIDER>_CLIENT_ID` at build or run time). A
//! build without one says so instead of pretending.
//!
//! Headless machines paste tokens or an AuthID from a self-hosted broker
//! (`ufop-server --oauth-config`, `server/oauth.rs`) instead.

use super::oauth::{CredentialSink, OAuthClient, OAuthConfig, OAuthGrant, TokenSink};
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, ConnectionProtocol, OneDriveAccountKind};
use crate::fleet_agent::loopback::Loopback;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a started sign-in waits for the browser.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// The providers with a desktop sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    GoogleDrive,
    Dropbox,
    OneDrive,
}

/// Authorization and token endpoints (overridable for tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEndpoints {
    pub auth_url: String,
    pub token_url: String,
}

impl OAuthProvider {
    pub fn from_protocol(p: ConnectionProtocol) -> Option<Self> {
        match p {
            ConnectionProtocol::GoogleDrive => Some(Self::GoogleDrive),
            ConnectionProtocol::Dropbox => Some(Self::Dropbox),
            ConnectionProtocol::OneDrive => Some(Self::OneDrive),
            _ => None,
        }
    }

    pub fn protocol(self) -> ConnectionProtocol {
        match self {
            Self::GoogleDrive => ConnectionProtocol::GoogleDrive,
            Self::Dropbox => ConnectionProtocol::Dropbox,
            Self::OneDrive => ConnectionProtocol::OneDrive,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::GoogleDrive => "Google Drive",
            Self::Dropbox => "Dropbox",
            Self::OneDrive => "OneDrive",
        }
    }

    /// Loopback port of the redirect URI. Fixed, because Dropbox only
    /// accepts redirect URIs registered exactly (Google and Microsoft allow
    /// any loopback port for desktop apps).
    pub fn callback_port(self) -> u16 {
        match self {
            Self::GoogleDrive => 9876,
            Self::Dropbox => 9877,
            Self::OneDrive => 9878,
        }
    }

    /// `http://localhost:<port>/callback`, the redirect URI to register.
    pub fn redirect_uri(self) -> String {
        format!("http://localhost:{}{}", self.callback_port(), crate::fleet_agent::loopback::CALLBACK_PATH)
    }

    fn env_prefix(self) -> &'static str {
        match self {
            Self::GoogleDrive => "UFOP_GOOGLE_DRIVE",
            Self::Dropbox => "UFOP_DROPBOX",
            Self::OneDrive => "UFOP_ONEDRIVE",
        }
    }

    /// The OAuth app built into this build: `UFOP_<PROVIDER>_CLIENT_ID`
    /// (and `_CLIENT_SECRET`) set when the app runs, else when it was built.
    pub fn builtin_client(self) -> Option<(String, Option<String>)> {
        self.builtin_client_from(|k| std::env::var(k).ok())
    }

    fn builtin_client_from(self, env: impl Fn(&str) -> Option<String>) -> Option<(String, Option<String>)> {
        let p = self.env_prefix();
        let compiled = |k: &str| -> Option<String> {
            match (self, k) {
                (Self::GoogleDrive, "id") => option_env!("UFOP_GOOGLE_DRIVE_CLIENT_ID"),
                (Self::GoogleDrive, _) => option_env!("UFOP_GOOGLE_DRIVE_CLIENT_SECRET"),
                (Self::Dropbox, "id") => option_env!("UFOP_DROPBOX_CLIENT_ID"),
                (Self::Dropbox, _) => option_env!("UFOP_DROPBOX_CLIENT_SECRET"),
                (Self::OneDrive, "id") => option_env!("UFOP_ONEDRIVE_CLIENT_ID"),
                (Self::OneDrive, _) => option_env!("UFOP_ONEDRIVE_CLIENT_SECRET"),
            }
            .map(String::from)
        };
        let nonempty = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let id = nonempty(env(&format!("{p}_CLIENT_ID"))).or_else(|| nonempty(compiled("id")))?;
        let secret = nonempty(env(&format!("{p}_CLIENT_SECRET"))).or_else(|| nonempty(compiled("secret")));
        Some((id, secret))
    }

    /// Google's desktop-app clients need their (non-confidential) client
    /// secret at the token endpoint; Dropbox and Microsoft public clients
    /// use PKCE alone.
    pub fn uses_client_secret(self) -> bool {
        matches!(self, Self::GoogleDrive)
    }

    /// Where to create your own OAuth app.
    pub fn app_console(self) -> &'static str {
        match self {
            Self::GoogleDrive => "https://console.cloud.google.com/apis/credentials (OAuth client ID, type \"Desktop app\", Drive API enabled)",
            Self::Dropbox => "https://www.dropbox.com/developers/apps (scoped app; add the redirect URI below)",
            Self::OneDrive => "https://entra.microsoft.com → App registrations (\"Mobile and desktop\" platform, redirect http://localhost)",
        }
    }

    pub fn endpoints(self, account: OneDriveAccountKind) -> AuthEndpoints {
        let (auth, token) = match (self, account) {
            (Self::GoogleDrive, _) => ("https://accounts.google.com/o/oauth2/v2/auth", "https://oauth2.googleapis.com/token"),
            (Self::Dropbox, _) => ("https://www.dropbox.com/oauth2/authorize", "https://api.dropboxapi.com/oauth2/token"),
            (Self::OneDrive, OneDriveAccountKind::Personal) => (
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
            ),
            (Self::OneDrive, OneDriveAccountKind::Business) => (
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            ),
        };
        AuthEndpoints { auth_url: auth.into(), token_url: token.into() }
    }

    fn scopes(self) -> Vec<String> {
        match self {
            Self::GoogleDrive => vec!["https://www.googleapis.com/auth/drive".into(), "https://www.googleapis.com/auth/drive.file".into()],
            // Scoped Dropbox apps get the scopes configured on the app.
            Self::Dropbox => vec![],
            Self::OneDrive => vec!["Files.ReadWrite.All".into(), "Sites.Read.All".into(), "offline_access".into()],
        }
    }

    fn extra_params(self) -> Vec<(String, String)> {
        match self {
            // Offline access (a refresh token) and the consent screen, so a
            // re-sign-in gets a new refresh token.
            Self::GoogleDrive => vec![("access_type".into(), "offline".into()), ("prompt".into(), "consent".into())],
            Self::Dropbox => vec![("token_access_type".into(), "offline".into())],
            Self::OneDrive => vec![],
        }
    }

    /// The OAuth configuration for `client_id` with `redirect_uri`.
    pub fn oauth_config(self, client_id: String, client_secret: Option<String>, redirect_uri: String, endpoints: AuthEndpoints) -> OAuthConfig {
        OAuthConfig {
            client_id,
            client_secret: client_secret.filter(|s| !s.is_empty()),
            auth_url: endpoints.auth_url,
            token_url: endpoints.token_url,
            redirect_uri,
            scopes: self.scopes(),
            extra_params: self.extra_params(),
        }
    }
}

fn no_client_error(provider: OAuthProvider) -> AppError {
    AppError::Configuration {
        message: format!("This build of UFOP has no built-in {} app to sign in with", provider.label()),
        advice: format!(
            "Switch to Advanced view and enter your own OAuth client ID for this connection (create one at {}), or sign in with a pasted token / AuthID from your sign-in broker.",
            provider.app_console()
        ),
    }
}

/// Which client signs in: `explicit` (the sign-in dialog), else the
/// profile's own client ID, else the built-in app.
pub fn resolve_client(provider: OAuthProvider, profile: &ConnectionProfile, explicit: Option<(String, Option<String>)>) -> Result<(String, Option<String>), AppError> {
    if let Some((id, secret)) = explicit.filter(|(id, _)| !id.trim().is_empty()) {
        return Ok((id.trim().to_string(), secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())));
    }
    if let Some(id) = profile.extras.oauth.as_ref().and_then(|o| o.client_id.clone()).map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        // A custom client's secret is kept with its tokens, not the profile.
        return Ok((id, None));
    }
    provider.builtin_client().ok_or_else(|| no_client_error(provider))
}

/// The client a new sign-in uses ([`resolve_client`]). Signing in again
/// with your own app and no secret typed reuses the secret saved with the
/// connection's current sign-in (`stored`) when it belongs to the same app.
pub fn sign_in_client(provider: OAuthProvider, profile: &ConnectionProfile, explicit: Option<(String, Option<String>)>, stored: Option<&str>) -> Result<(String, Option<String>), AppError> {
    let (id, secret) = resolve_client(provider, profile, explicit)?;
    if secret.is_some() {
        return Ok((id, secret));
    }
    let saved = stored.and_then(|s| OAuthGrant::parse(s).ok()).and_then(|g| match g {
        OAuthGrant::Token { client_id: Some(c), client_secret, .. } if c.trim() == id => client_secret.filter(|s| !s.trim().is_empty()),
        _ => None,
    });
    Ok((id, saved))
}

fn account_kind(profile: &ConnectionProfile) -> OneDriveAccountKind {
    profile.extras.oauth.as_ref().map(|o| o.account_kind).unwrap_or_default()
}

/// The OAuth client a connector connects with: the stored grant restored,
/// its app resolved, rotated tokens persisted to `credentials` under the
/// profile's credential key. `token_url` overrides the endpoint (tests).
pub(crate) async fn client_for_connect(
    provider: OAuthProvider,
    profile: &ConnectionProfile,
    secret: Option<String>,
    token_url: Option<&str>,
    credentials: Option<Arc<crate::security::CredentialStore>>,
) -> Result<OAuthClient, AppError> {
    // `connect()` without a secret loads the stored sign-in itself.
    let secret = match secret.filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(s),
        None => match &credentials {
            Some(store) => store.retrieve(&crate::security::CredentialStore::credential_key(&profile.id)).await?,
            None => None,
        },
    };
    let secret = secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| AppError::State {
        message: format!("Not signed in to {} yet", provider.label()),
        advice: "Click Sign in on the connection in Connections.".into(),
    })?;
    let grant = OAuthGrant::parse(&secret)?;
    let mut endpoints = provider.endpoints(account_kind(profile));
    let client = match &grant {
        OAuthGrant::Broker { broker_url, authid } => {
            if let Some(t) = token_url {
                endpoints.token_url = t.to_string();
            }
            OAuthClient::new(provider.oauth_config(String::new(), None, provider.redirect_uri(), endpoints)).with_broker(broker_url, authid)
        }
        OAuthGrant::Token { token, client_id, client_secret, token_url: grant_url } => {
            // The refresh token belongs to the app that issued it.
            let (id, sec) = match client_id.clone().filter(|c| !c.is_empty()) {
                Some(id) => (id, client_secret.clone()),
                None => match resolve_client(provider, profile, None) {
                    Ok(c) => c,
                    // A bare access token works until it expires without an app.
                    Err(_) if token.refresh_token.is_none() => (String::new(), None),
                    Err(e) => return Err(e),
                },
            };
            if let Some(u) = token_url.map(String::from).or_else(|| grant_url.clone()) {
                endpoints.token_url = u;
            }
            OAuthClient::new(provider.oauth_config(id, sec, provider.redirect_uri(), endpoints))
        }
    };
    let client = match credentials {
        Some(store) => client.with_sink(Arc::new(CredentialSink { store, key: crate::security::CredentialStore::credential_key(&profile.id) }) as Arc<dyn TokenSink>),
        None => client,
    };
    client.restore_grant(&grant).await;
    Ok(client)
}

/// A started sign-in, returned to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthStart {
    pub state: String,
    pub provider: OAuthProvider,
    /// Open this in a browser (on any device, for the manual-code path).
    pub auth_url: String,
    pub redirect_uri: String,
    /// The system browser was opened.
    pub browser_opened: bool,
}

struct PendingFlow {
    connection_id: Uuid,
    client: Arc<OAuthClient>,
    loopback: Option<Loopback>,
    /// Resolves once the waiter that took `loopback` has closed it.
    released: Option<tokio::sync::oneshot::Receiver<()>>,
    /// A code is being exchanged (the flow stays open until one succeeds).
    exchanging: bool,
    done: Arc<tokio::sync::Notify>,
    started: Instant,
}

/// Sign-ins in progress, by OAuth `state`.
#[derive(Default)]
pub struct OAuthFlows {
    pending: tokio::sync::Mutex<HashMap<String, PendingFlow>>,
}

impl OAuthFlows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a sign-in for `profile`: bind the loopback listener (`port`
    /// overrides the provider's; 0 = any) and build the consent URL.
    pub async fn begin(
        &self,
        provider: OAuthProvider,
        profile: &ConnectionProfile,
        client: (String, Option<String>),
        port: Option<u16>,
        endpoints: Option<AuthEndpoints>,
    ) -> Result<OAuthStart, AppError> {
        let mut pending = self.pending.lock().await;
        pending.retain(|_, f| f.started.elapsed() < SIGN_IN_TIMEOUT * 2);
        // One sign-in per connection: a new one replaces an abandoned one.
        let stale: Vec<String> = pending.iter().filter(|(_, f)| f.connection_id == profile.id).map(|(k, _)| k.clone()).collect();
        let ended: Vec<PendingFlow> = stale.iter().filter_map(|k| pending.remove(k)).collect();
        drop(pending);
        for f in ended {
            end_flow(f).await;
        }
        let loopback = Loopback::bind_on(port.unwrap_or(provider.callback_port()), "localhost").await?;
        let endpoints = endpoints.unwrap_or_else(|| provider.endpoints(account_kind(profile)));
        let oauth = OAuthClient::new(provider.oauth_config(client.0, client.1, loopback.redirect_uri.clone(), endpoints));
        let (auth_url, state) = oauth.try_authorization_url().await?;
        let redirect_uri = loopback.redirect_uri.clone();
        self.pending.lock().await.insert(
            state.clone(),
            PendingFlow { connection_id: profile.id, client: Arc::new(oauth), loopback: Some(loopback), released: None, exchanging: false, done: Arc::new(tokio::sync::Notify::new()), started: Instant::now() },
        );
        Ok(OAuthStart { state, provider, auth_url, redirect_uri, browser_opened: false })
    }

    /// Wait for the browser to come back with the code, then exchange it.
    pub async fn wait(&self, state: &str, timeout: Duration) -> Result<(Uuid, OAuthGrant), AppError> {
        let released;
        let (loopback, done) = {
            let mut pending = self.pending.lock().await;
            let f = pending.get_mut(state).ok_or_else(not_pending)?;
            let loopback = f.loopback.take().ok_or_else(|| AppError::state("This sign-in is already being waited for", "Wait for it to finish."))?;
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            released = tx;
            f.released = Some(rx);
            (loopback, f.done.clone())
        };
        let code = tokio::select! {
            r = loopback.wait_for_code(state, timeout) => Some(r),
            _ = done.notified() => None,
        };
        // The listener (owned by the finished select) is closed: whoever
        // ended this flow may bind the port again.
        drop(released);
        let Some(code) = code else {
            return Err(AppError::state("The sign-in was cancelled or finished another way", "Start it again if you still need it."));
        };
        match code {
            Ok(code) => self.exchange(state, &code, false).await,
            Err(e) => {
                let mut pending = self.pending.lock().await;
                // A pasted code being exchanged right now finishes the flow.
                if pending.get(state).is_some_and(|f| !f.exchanging) {
                    pending.remove(state);
                }
                Err(e)
            }
        }
    }

    /// Finish with a code (or the whole redirect URL) pasted by the user,
    /// for a browser on another device.
    pub async fn complete(&self, state: &str, code_or_url: &str) -> Result<(Uuid, OAuthGrant), AppError> {
        let input = code_or_url.trim();
        let code = if input.contains("code=") {
            let query = input.split_once('?').map(|(_, q)| q).unwrap_or(input);
            let q: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
            if q.get("state").is_some_and(|s| s != state) {
                return Err(AppError::validation("That address belongs to a different sign-in"));
            }
            q.get("code").cloned().unwrap_or_default()
        } else {
            input.to_string()
        };
        if code.is_empty() {
            return Err(AppError::validation("Paste the code (or the whole address the browser was sent to)"));
        }
        // A mistyped code can be pasted again: the flow stays open.
        self.exchange(state, &code, true).await
    }

    /// Exchange `code`; the flow ends (and a waiter is woken) only once it
    /// succeeds, or on any failure unless `keep_on_failure`.
    async fn exchange(&self, state: &str, code: &str, keep_on_failure: bool) -> Result<(Uuid, OAuthGrant), AppError> {
        let (client, connection_id) = {
            let mut pending = self.pending.lock().await;
            let f = pending.get_mut(state).ok_or_else(not_pending)?;
            if f.exchanging {
                return Err(AppError::state("This sign-in is already being finished", "Wait for it to finish."));
            }
            f.exchanging = true;
            (f.client.clone(), f.connection_id)
        };
        let result = match client.exchange_code(code).await {
            Ok(_) => client.grant().await.ok_or_else(|| AppError::internal("Token exchange returned no token")),
            Err(e) => Err(e),
        };
        let mut pending = self.pending.lock().await;
        match result {
            Ok(grant) => match pending.remove(state) {
                Some(f) => {
                    f.done.notify_one();
                    Ok((connection_id, grant))
                }
                // Cancelled (or replaced by a new sign-in) meanwhile.
                None => Err(not_pending()),
            },
            Err(e) => {
                if keep_on_failure {
                    if let Some(f) = pending.get_mut(state) {
                        f.exchanging = false;
                    }
                } else if let Some(f) = pending.remove(state) {
                    f.done.notify_one();
                }
                Err(e)
            }
        }
    }

    /// Abandon a sign-in; returns once its callback port is free again.
    pub async fn cancel(&self, state: &str) {
        let ended = self.pending.lock().await.remove(state);
        if let Some(f) = ended {
            end_flow(f).await;
        }
    }

    pub async fn is_pending(&self, state: &str) -> bool {
        self.pending.lock().await.contains_key(state)
    }
}

/// End a flow taken out of the pending map: wake its waiter and wait (up
/// to 2 s) until the waiter has closed the callback listener, so the port
/// can be bound again right away. A flow nobody waits for still owns its
/// listener, which closes as `f` is dropped here.
async fn end_flow(mut f: PendingFlow) {
    f.done.notify_one();
    if let Some(rx) = f.released.take() {
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
    }
}

fn not_pending() -> AppError {
    AppError::state("No sign-in with this code is in progress (it may have expired)", "Start the sign-in again.")
}

/// What the sign-in screen needs to explain the provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderInfo {
    pub provider: OAuthProvider,
    pub label: String,
    /// This build has its own app: Standard users just click Sign in.
    pub has_builtin_client: bool,
    /// Your own app needs a client secret too (Google desktop apps).
    pub uses_client_secret: bool,
    pub redirect_uri: String,
    pub app_console: String,
}

impl OAuthProviderInfo {
    pub fn of(provider: OAuthProvider) -> Self {
        Self {
            provider,
            label: provider.label().into(),
            has_builtin_client: provider.builtin_client().is_some(),
            uses_client_secret: provider.uses_client_secret(),
            redirect_uri: provider.redirect_uri(),
            app_console: provider.app_console().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::OAuthSettings;

    fn profile(p: ConnectionProtocol) -> ConnectionProfile {
        crate::connectors::live_tests::profile(p, "", None, "", "/")
    }

    #[test]
    fn builtin_client_comes_from_the_environment() {
        let env = |k: &str| (k == "UFOP_DROPBOX_CLIENT_ID").then(|| " app-key ".to_string());
        assert_eq!(OAuthProvider::Dropbox.builtin_client_from(env), Some(("app-key".into(), None)));
        let none = |_: &str| None;
        // Unless this build was compiled with one, there is none.
        assert_eq!(OAuthProvider::Dropbox.builtin_client_from(none).is_some(), option_env!("UFOP_DROPBOX_CLIENT_ID").is_some());
    }

    #[test]
    fn client_resolution_order_and_honest_failure() {
        let mut p = profile(ConnectionProtocol::GoogleDrive);
        assert_eq!(resolve_client(OAuthProvider::GoogleDrive, &p, Some(("mine".into(), Some("sec".into())))).unwrap(), ("mine".into(), Some("sec".into())));
        p.extras.oauth = Some(OAuthSettings { client_id: Some("saved".into()), ..Default::default() });
        assert_eq!(resolve_client(OAuthProvider::GoogleDrive, &p, None).unwrap().0, "saved");
        p.extras.oauth = None;
        if OAuthProvider::GoogleDrive.builtin_client().is_none() {
            let e = resolve_client(OAuthProvider::GoogleDrive, &p, None).unwrap_err();
            assert!(e.to_string().contains("no built-in Google Drive app") && e.to_string().contains("Advanced"), "{e}");
        }
    }

    /// Regression: re-signing in with your own Google desktop app sent no
    /// client secret (it is kept with the tokens, not the profile), so
    /// Google refused the code exchange.
    #[test]
    fn signing_in_again_with_your_own_app_reuses_its_stored_secret() {
        let mut p = profile(ConnectionProtocol::GoogleDrive);
        p.extras.oauth = Some(OAuthSettings { client_id: Some("mine".into()), ..Default::default() });
        let stored = r#"{"kind":"token","token":{"access_token":"a","refresh_token":"r","expires_at":null,"token_type":"Bearer","scope":null},"client_id":"mine","client_secret":"sec"}"#;
        let gd = OAuthProvider::GoogleDrive;
        // From the saved client ID, or the same ID typed in the dialog with an empty secret.
        assert_eq!(sign_in_client(gd, &p, None, Some(stored)).unwrap(), ("mine".into(), Some("sec".into())));
        assert_eq!(sign_in_client(gd, &p, Some(("mine".into(), Some(" ".into()))), Some(stored)).unwrap(), ("mine".into(), Some("sec".into())));
        // A newly typed secret wins; another app never gets this one.
        assert_eq!(sign_in_client(gd, &p, Some(("mine".into(), Some("new".into()))), Some(stored)).unwrap().1.as_deref(), Some("new"));
        assert_eq!(sign_in_client(gd, &p, Some(("other".into(), None)), Some(stored)).unwrap(), ("other".into(), None));
        assert_eq!(sign_in_client(gd, &p, None, None).unwrap(), ("mine".into(), None));
        assert_eq!(sign_in_client(gd, &p, None, Some("not json")).unwrap(), ("mine".into(), None));
    }

    #[test]
    fn grants_parse_every_accepted_format() {
        let OAuthGrant::Token { token, .. } = OAuthGrant::parse("ya29.bare").unwrap() else { panic!() };
        assert_eq!((token.access_token.as_str(), token.refresh_token), ("ya29.bare", None));
        // Backup-destination format (refresh only): expired, so it refreshes first.
        let OAuthGrant::Token { token, client_id, token_url, .. } = OAuthGrant::parse(r#"{"refresh_token":"r","client_id":"c","token_url":"http://t"}"#).unwrap() else { panic!() };
        assert!(token.is_expired() && client_id.as_deref() == Some("c") && token_url.as_deref() == Some("http://t"));
        assert!(matches!(OAuthGrant::parse(r#"{"broker_url":"https://b","authid":"google:x"}"#).unwrap(), OAuthGrant::Broker { .. }));
        let g = OAuthGrant::parse(r#"{"broker_url":"https://b","authid":"a"}"#).unwrap();
        assert!(matches!(OAuthGrant::parse(&g.to_json().unwrap()).unwrap(), OAuthGrant::Broker { authid, .. } if authid == "a"));
        assert!(OAuthGrant::parse("{}").is_err() && OAuthGrant::parse(" ").is_err());
    }

    #[tokio::test]
    async fn not_signed_in_is_not_a_rejected_credential() {
        let e = client_for_connect(OAuthProvider::Dropbox, &profile(ConnectionProtocol::Dropbox), None, None, None).await.err().unwrap();
        assert!(!e.is_auth_failure() && e.to_string().contains("Not signed in"), "{e}");
    }
}
