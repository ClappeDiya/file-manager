//! Sign-in for Google Drive, Dropbox and OneDrive connections (OAuth 2.0
//! with PKCE and a loopback redirect; see `connectors::oauth_flow`).
//!
//! Desktop: `oauth_begin` opens the browser, `oauth_await` waits for it and
//! stores the tokens (or `oauth_complete` takes a pasted code when the
//! browser runs on another device). Headless: `oauth_import_grant` takes
//! pasted tokens or an AuthID from a self-hosted broker.

use crate::connectors::oauth::OAuthGrant;
use crate::connectors::oauth_flow::{self, OAuthFlows, OAuthProvider, OAuthProviderInfo, OAuthStart, SIGN_IN_TIMEOUT};
use crate::connectors::{ConnectionManager, Connector, DropboxConnector, GoogleDriveConnector, OneDriveConnector};
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, ConnectionProtocol, OAuthSettings};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

/// A finished sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthSignedIn {
    pub connection_id: String,
    pub provider: OAuthProvider,
    /// The account signed in (e-mail or name), as the service reports it.
    pub account: Option<String>,
}

/// The app's OAuth connector instances (shared with the connector registry).
pub struct CloudConnectors<'a> {
    pub gdrive: &'a GoogleDriveConnector,
    pub dropbox: &'a DropboxConnector,
    pub onedrive: &'a OneDriveConnector,
}

impl CloudConnectors<'_> {
    fn connector(&self, p: OAuthProvider) -> &dyn Connector {
        match p {
            OAuthProvider::GoogleDrive => self.gdrive,
            OAuthProvider::Dropbox => self.dropbox,
            OAuthProvider::OneDrive => self.onedrive,
        }
    }

    async fn grant(&self, p: OAuthProvider) -> Option<OAuthGrant> {
        match p {
            OAuthProvider::GoogleDrive => self.gdrive.current_grant().await,
            OAuthProvider::Dropbox => self.dropbox.current_grant().await,
            OAuthProvider::OneDrive => self.onedrive.current_grant().await,
        }
    }

    fn account(&self, p: OAuthProvider) -> Option<String> {
        match p {
            OAuthProvider::GoogleDrive => self.gdrive.account(),
            OAuthProvider::Dropbox => self.dropbox.account(),
            OAuthProvider::OneDrive => self.onedrive.account(),
        }
    }
}

fn parse_id(connection_id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(connection_id).map_err(|_| AppError::validation(format!("Invalid connection ID: {connection_id}")))
}

async fn oauth_profile(manager: &ConnectionManager, connection_id: &str) -> Result<(ConnectionProfile, OAuthProvider), AppError> {
    let profile = manager
        .get_connection(parse_id(connection_id)?)
        .await?
        .ok_or_else(|| AppError::not_found("The connection no longer exists"))?;
    let provider = OAuthProvider::from_protocol(profile.protocol)
        .ok_or_else(|| AppError::validation(format!("{} connections do not sign in with OAuth", profile.protocol.as_str())))?;
    Ok((profile, provider))
}

/// Check `grant` by connecting with it, then store it (with the tokens the
/// connect may have renewed) as the connection's credential. Nothing is
/// stored when the service refuses it.
pub async fn finish_sign_in(manager: &ConnectionManager, cloud: &CloudConnectors<'_>, connection_id: Uuid, grant: OAuthGrant) -> Result<OAuthSignedIn, AppError> {
    let mut profile = manager.get_connection(connection_id).await?.ok_or_else(|| AppError::not_found("The connection no longer exists"))?;
    let provider = OAuthProvider::from_protocol(profile.protocol).ok_or_else(|| AppError::validation("This connection does not sign in with OAuth"))?;
    let connector = cloud.connector(provider);
    connector.connect_with_secret(&profile, Some(grant.to_json()?)).await?;
    let stored = cloud.grant(provider).await.unwrap_or(grant);
    // A sign-in with your own app: remember it, so the next sign-in uses it too.
    if let OAuthGrant::Token { client_id: Some(id), .. } = &stored {
        let builtin = provider.builtin_client().map(|(b, _)| b);
        if builtin.as_deref() != Some(id.as_str()) {
            let o = profile.extras.oauth.get_or_insert_with(OAuthSettings::default);
            o.client_id = Some(id.clone());
        }
    }
    // Saving stores the credential and clears any credentials-rejected flag.
    manager.save_connection(profile, Some(stored.to_json()?)).await?;
    Ok(OAuthSignedIn { connection_id: connection_id.to_string(), provider, account: cloud.account(provider) })
}

/// What the sign-in screen should say about a provider (is there a
/// built-in app, which redirect URI your own app needs).
#[tauri::command]
pub async fn oauth_provider_info(protocol: String) -> Result<OAuthProviderInfo, AppError> {
    let provider = OAuthProvider::from_protocol(ConnectionProtocol::from_str_lossy(&protocol))
        .ok_or_else(|| AppError::validation(format!("{protocol} connections do not sign in with OAuth")))?;
    Ok(OAuthProviderInfo::of(provider))
}

/// Start signing in a saved Google Drive / Dropbox / OneDrive connection:
/// returns the consent URL (opened in the system browser unless
/// `open_browser` is false) and the `state` to pass to `oauth_await` or
/// `oauth_complete`. `client_id` / `client_secret`: your own OAuth app
/// (Advanced); otherwise the connection's or the built-in one.
#[tauri::command]
pub async fn oauth_begin(
    app: tauri::AppHandle,
    connection_id: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    open_browser: Option<bool>,
    manager: State<'_, ConnectionManager>,
    flows: State<'_, Arc<OAuthFlows>>,
) -> Result<OAuthStart, AppError> {
    let (profile, provider) = oauth_profile(&manager, &connection_id).await?;
    let client = oauth_flow::resolve_client(provider, &profile, client_id.map(|id| (id, client_secret)))?;
    let mut start = flows.begin(provider, &profile, client, None, None).await?;
    if open_browser.unwrap_or(true) {
        #[allow(deprecated)]
        {
            use tauri_plugin_shell::ShellExt;
            start.browser_opened = app.shell().open(&start.auth_url, None).map_err(|e| tracing::warn!("Could not open the browser: {e}")).is_ok();
        }
    }
    Ok(start)
}

/// Wait (up to five minutes) for the browser to return from the consent
/// page, then store the tokens and connect.
#[tauri::command]
pub async fn oauth_await(
    state: String,
    manager: State<'_, ConnectionManager>,
    flows: State<'_, Arc<OAuthFlows>>,
    gdrive: State<'_, Arc<GoogleDriveConnector>>,
    dropbox: State<'_, Arc<DropboxConnector>>,
    onedrive: State<'_, Arc<OneDriveConnector>>,
) -> Result<OAuthSignedIn, AppError> {
    let (id, grant) = flows.wait(&state, SIGN_IN_TIMEOUT).await?;
    let cloud = CloudConnectors { gdrive: &gdrive, dropbox: &dropbox, onedrive: &onedrive };
    finish_sign_in(&manager, &cloud, id, grant).await
}

/// Finish a sign-in with the code (or the whole address) the browser was
/// sent to, when that browser runs on another device.
#[tauri::command]
pub async fn oauth_complete(
    state: String,
    code: String,
    manager: State<'_, ConnectionManager>,
    flows: State<'_, Arc<OAuthFlows>>,
    gdrive: State<'_, Arc<GoogleDriveConnector>>,
    dropbox: State<'_, Arc<DropboxConnector>>,
    onedrive: State<'_, Arc<OneDriveConnector>>,
) -> Result<OAuthSignedIn, AppError> {
    let (id, grant) = flows.complete(&state, &code).await?;
    let cloud = CloudConnectors { gdrive: &gdrive, dropbox: &dropbox, onedrive: &onedrive };
    finish_sign_in(&manager, &cloud, id, grant).await
}

/// Abandon a sign-in started with `oauth_begin`.
#[tauri::command]
pub async fn oauth_cancel(state: String, flows: State<'_, Arc<OAuthFlows>>) -> Result<(), AppError> {
    flows.cancel(&state).await;
    Ok(())
}

/// Headless sign-in: paste tokens (`token`: JSON with an access and/or
/// refresh token, or a bare access token) or an AuthID from a self-hosted
/// broker (`broker_url` + `authid`, see `ufop-server --oauth-config`). The
/// credential is checked by connecting before it is stored.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn oauth_import_grant(
    connection_id: String,
    token: Option<String>,
    broker_url: Option<String>,
    authid: Option<String>,
    manager: State<'_, ConnectionManager>,
    gdrive: State<'_, Arc<GoogleDriveConnector>>,
    dropbox: State<'_, Arc<DropboxConnector>>,
    onedrive: State<'_, Arc<OneDriveConnector>>,
) -> Result<OAuthSignedIn, AppError> {
    let (profile, _) = oauth_profile(&manager, &connection_id).await?;
    let grant = grant_from_input(token, broker_url, authid)?;
    let cloud = CloudConnectors { gdrive: &gdrive, dropbox: &dropbox, onedrive: &onedrive };
    finish_sign_in(&manager, &cloud, profile.id, grant).await
}

/// The grant a headless sign-in pasted.
pub fn grant_from_input(token: Option<String>, broker_url: Option<String>, authid: Option<String>) -> Result<OAuthGrant, AppError> {
    let nonempty = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    match (nonempty(token), nonempty(broker_url), nonempty(authid)) {
        (_, Some(url), Some(authid)) => {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(AppError::validation("The broker URL must start with https://"));
            }
            Ok(OAuthGrant::Broker { broker_url: url.trim_end_matches('/').to_string(), authid })
        }
        (_, None, Some(_)) => Err(AppError::validation("Enter the broker URL the AuthID came from")),
        (Some(t), _, None) => OAuthGrant::parse(&t),
        _ => Err(AppError::validation("Paste a token, or enter a broker URL and AuthID")),
    }
}

/// Forget a connection's stored sign-in (and disconnect it).
#[tauri::command]
pub async fn oauth_sign_out(
    connection_id: String,
    manager: State<'_, ConnectionManager>,
    gdrive: State<'_, Arc<GoogleDriveConnector>>,
    dropbox: State<'_, Arc<DropboxConnector>>,
    onedrive: State<'_, Arc<OneDriveConnector>>,
) -> Result<(), AppError> {
    let (profile, provider) = oauth_profile(&manager, &connection_id).await?;
    let cloud = CloudConnectors { gdrive: &gdrive, dropbox: &dropbox, onedrive: &onedrive };
    cloud.connector(provider).disconnect().await?;
    manager.delete_password(profile.id).await
}

/// Whether a saved OAuth connection has a stored sign-in.
#[tauri::command]
pub async fn oauth_status(connection_id: String, manager: State<'_, ConnectionManager>) -> Result<bool, AppError> {
    let (profile, _) = oauth_profile(&manager, &connection_id).await?;
    Ok(manager.get_password(profile.id).await?.is_some_and(|s| OAuthGrant::parse(&s).is_ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_input() {
        assert!(matches!(grant_from_input(None, Some("https://b.example/".into()), Some("google:x".into())).unwrap(), OAuthGrant::Broker { broker_url, .. } if broker_url == "https://b.example"));
        assert!(grant_from_input(None, None, Some("google:x".into())).is_err());
        assert!(grant_from_input(None, Some("ftp://b".into()), Some("x".into())).is_err());
        assert!(matches!(grant_from_input(Some("tok".into()), None, None).unwrap(), OAuthGrant::Token { .. }));
        assert!(grant_from_input(Some(" ".into()), None, None).is_err());
    }
}
