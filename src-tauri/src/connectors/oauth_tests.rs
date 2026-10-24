//! OAuth connections end to end against mocked token and API servers:
//! profile → sign-in (loopback + PKCE) → stored grant → connect → list,
//! refresh on expiry with the rotated token persisted, revoked sign-ins,
//! AuthID broker and pasted tokens.

use super::mock_http::{json, serve, status, Mock, Seen};
use super::oauth::OAuthGrant;
use super::oauth_flow::{AuthEndpoints, OAuthFlows, OAuthProvider};
use super::{ConnectionManager, Connector, DropboxConnector, GoogleDriveConnector, OneDriveConnector};
use crate::commands::oauth_commands::{finish_sign_in, CloudConnectors};
use crate::core::types::{ConnectionProfile, ConnectionProtocol};
use crate::security::CredentialStore;
use axum::response::Response;
use serde_json::json as j;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Token endpoint: code `c1` → (`a1`, `r1`); refresh `r1` → (`a2`, `r2`)
/// (rotating, like Microsoft); refresh `revoked` → `invalid_grant`.
/// Bearer `a1` / `a2` / `pasted` / `brokered` are valid API tokens.
fn token_endpoint(r: &Seen, refreshes: &AtomicU32) -> Response {
    let f = r.form();
    match f.get("grant_type").map(String::as_str) {
        Some("authorization_code") if f.get("code").map(String::as_str) == Some("c1") => {
            if f.get("code_verifier").map_or(true, |v| v.len() < 43) {
                return status(400, r#"{"error":"invalid_request","error_description":"PKCE verifier missing"}"#);
            }
            json(j!({ "access_token": "a1", "refresh_token": "r1", "expires_in": 3600, "token_type": "Bearer" }))
        }
        Some("refresh_token") => match f.get("refresh_token").map(String::as_str) {
            Some("r1") => {
                refreshes.fetch_add(1, Ordering::SeqCst);
                json(j!({ "access_token": "a2", "refresh_token": "r2", "expires_in": 3600 }))
            }
            _ => status(400, r#"{"error":"invalid_grant"}"#),
        },
        _ => status(400, r#"{"error":"invalid_grant"}"#),
    }
}

fn bearer_ok(r: &Seen) -> bool {
    matches!(r.header("authorization").as_deref(), Some("Bearer a1" | "Bearer a2" | "Bearer pasted" | "Bearer brokered"))
}

struct Env {
    mock: Mock,
    refreshes: Arc<AtomicU32>,
    mgr: ConnectionManager,
    store: Arc<CredentialStore>,
}

/// One mock playing the token endpoint, the AuthID broker and the Drive,
/// Dropbox and Graph APIs.
async fn env() -> Env {
    let refreshes = Arc::new(AtomicU32::new(0));
    let r2 = refreshes.clone();
    let mock = serve(move |r| {
        match r.path.as_str() {
            "/token" => return token_endpoint(r, &r2),
            "/oauth/token" => {
                return if r.query_param("authid").as_deref() == Some("google:good") {
                    json(j!({ "access_token": "brokered", "expires_at": chrono::Utc::now().timestamp() + 3600 }))
                } else {
                    status(401, r#"{"error_type":"permission_denied","message":"Unknown AuthID"}"#)
                };
            }
            _ => {}
        }
        if !bearer_ok(r) {
            return status(401, r#"{"error":{"code":401}}"#);
        }
        match r.path.as_str() {
            "/drive/about" => json(j!({ "user": { "emailAddress": "ada@example.com" }, "storageQuota": { "limit": "100", "usage": "40" } })),
            "/drive/files" => json(j!({ "files": [
                { "id": "f1", "name": "Reports", "mimeType": "application/vnd.google-apps.folder" },
                { "id": "f2", "name": "notes.txt", "mimeType": "text/plain", "size": "12" }
            ] })),
            "/dbx/users/get_current_account" => json(j!({ "email": "bob@example.com", "root_info": { ".tag": "user", "root_namespace_id": "1", "home_namespace_id": "1" } })),
            "/dbx/files/list_folder" => json(j!({ "entries": [{ ".tag": "file", "name": "a.txt", "path_display": "/a.txt", "size": 3 }], "cursor": "c", "has_more": false })),
            "/graph/me/drive" => json(j!({ "id": "d1", "owner": { "user": { "email": "cy@example.com" } }, "quota": { "total": 1000, "used": 10 } })),
            "/graph/me/drive/root/children" => json(j!({ "value": [{ "id": "i1", "name": "Docs", "folder": { "childCount": 0 }, "parentReference": { "path": "/drive/root:" } }] })),
            _ => status(404, "no route"),
        }
    })
    .await;
    let repo = crate::storage::Repository::open_in_memory().await.unwrap();
    let store = Arc::new(CredentialStore { cache: Arc::default(), keychain_available: false, file_vault: None });
    Env { mock, refreshes, mgr: ConnectionManager::new(repo, store.clone()), store }
}

impl Env {
    fn gdrive(&self) -> GoogleDriveConnector {
        let b = &self.mock.base;
        GoogleDriveConnector::new().with_credentials(self.store.clone()).with_endpoints(&format!("{b}/drive"), &format!("{b}/upload"), &format!("{b}/token"))
    }
    fn dropbox(&self) -> DropboxConnector {
        let b = &self.mock.base;
        DropboxConnector::new().with_credentials(self.store.clone()).with_endpoints(&format!("{b}/dbx"), &format!("{b}/content"), &format!("{b}/token"))
    }
    fn onedrive(&self) -> OneDriveConnector {
        let b = &self.mock.base;
        OneDriveConnector::new().with_credentials(self.store.clone()).with_endpoints(&format!("{b}/graph"), &format!("{b}/token"))
    }
    async fn saved(&self, protocol: ConnectionProtocol) -> ConnectionProfile {
        let p = super::live_tests::profile(protocol, "", None, "", "/");
        self.mgr.save_connection(p.clone(), None).await.unwrap();
        p
    }
    async fn stored_grant(&self, p: &ConnectionProfile) -> OAuthGrant {
        OAuthGrant::parse(&self.mgr.get_password(p.id).await.unwrap().expect("a stored grant")).unwrap()
    }
}

/// The browser coming back to the loopback redirect.
async fn browser_returns(redirect_uri: &str, query: &str) -> String {
    let url = format!("{}?{query}", redirect_uri.replace("localhost", "127.0.0.1"));
    reqwest::get(url).await.unwrap().text().await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_connect_list_and_refresh_on_expiry() {
    let e = env().await;
    let p = e.saved(ConnectionProtocol::GoogleDrive).await;
    let gd = e.gdrive();
    // Before signing in, connecting says so (and is not a rejected credential).
    let err = gd.connect_with_secret(&p, None).await.unwrap_err();
    assert!(err.to_string().contains("Not signed in") && !err.is_auth_failure());

    // Sign in: consent URL with PKCE, loopback redirect, code exchange.
    let flows = OAuthFlows::new();
    let endpoints = AuthEndpoints { auth_url: format!("{}/auth", e.mock.base), token_url: format!("{}/token", e.mock.base) };
    let start = flows.begin(OAuthProvider::GoogleDrive, &p, ("my-client".into(), Some("my-secret".into())), Some(0), Some(endpoints)).await.unwrap();
    let auth = url::Url::parse(&start.auth_url).unwrap();
    let q: HashMap<String, String> = auth.query_pairs().into_owned().collect();
    assert_eq!(q["client_id"], "my-client");
    assert_eq!(q["redirect_uri"], start.redirect_uri);
    assert_eq!((q["code_challenge_method"].as_str(), q["access_type"].as_str()), ("S256", "offline"));
    assert!(start.redirect_uri.starts_with("http://localhost:") && start.redirect_uri.ends_with("/callback"));
    let state = start.state.clone();
    let waiting = {
        let flows = Arc::new(flows);
        let f2 = flows.clone();
        (tokio::spawn(async move { f2.wait(&state, Duration::from_secs(10)).await }), flows)
    };
    // A forged redirect (wrong state) is ignored; the real one completes.
    assert!(browser_returns(&start.redirect_uri, "code=evil&state=forged").await.contains("not recognised"));
    assert!(browser_returns(&start.redirect_uri, &format!("code=c1&state={}", start.state)).await.contains("close this tab"));
    let (id, grant) = waiting.0.await.unwrap().unwrap();
    assert_eq!(id, p.id);
    let exchange = &e.mock.requests("/token")[0];
    assert_eq!(exchange.form().get("redirect_uri"), Some(&start.redirect_uri));
    assert_eq!(exchange.form().get("client_secret").map(String::as_str), Some("my-secret"));
    assert!(!waiting.1.is_pending(&start.state).await);

    // Store it by connecting with it; the account is named, the custom app remembered.
    let (db, od) = (e.dropbox(), e.onedrive());
    let cloud = CloudConnectors { gdrive: &gd, dropbox: &db, onedrive: &od };
    let signed = finish_sign_in(&e.mgr, &cloud, id, grant).await.unwrap();
    assert_eq!(signed.account.as_deref(), Some("ada@example.com"));
    let saved = e.mgr.get_connection(p.id).await.unwrap().unwrap();
    assert_eq!(saved.extras.oauth.unwrap().client_id.as_deref(), Some("my-client"));
    assert!(matches!(e.stored_grant(&p).await, OAuthGrant::Token { token, client_secret, .. } if token.refresh_token.as_deref() == Some("r1") && client_secret.as_deref() == Some("my-secret")));
    assert!(gd.is_connected());
    let list = gd.list_remote("/").await.unwrap();
    assert_eq!(list.iter().map(|f| (f.name.as_str(), f.is_dir, f.path.as_str())).collect::<Vec<_>>(), vec![("Reports", true, "/f1"), ("notes.txt", false, "/f2")]);
    let files_q = e.mock.requests("/drive/files")[0].query_param("q").unwrap();
    assert_eq!(files_q, "'root' in parents and trashed = false");

    // Later (a new app start): the access token has expired. connect()
    // refreshes it and writes the rotated refresh token back.
    let OAuthGrant::Token { mut token, client_id, client_secret, token_url } = e.stored_grant(&p).await else { panic!() };
    token.expires_at = Some(chrono::Utc::now() - chrono::Duration::minutes(5));
    let expired = OAuthGrant::Token { token, client_id, client_secret, token_url }.to_json().unwrap();
    e.mgr.set_password(p.id, &expired).await.unwrap();
    let fresh = e.gdrive();
    let secrets = e.mgr.get_secrets(p.id).await.unwrap();
    fresh.connect_with_secrets(&saved_profile(&e, &p).await, secrets).await.unwrap();
    assert_eq!(e.refreshes.load(Ordering::SeqCst), 1);
    let refresh = e.mock.requests("/token").into_iter().find(|r| r.form().get("grant_type").map(String::as_str) == Some("refresh_token")).unwrap();
    assert_eq!(refresh.form().get("client_id").map(String::as_str), Some("my-client"));
    let OAuthGrant::Token { token, .. } = e.stored_grant(&p).await else { panic!() };
    assert_eq!((token.access_token.as_str(), token.refresh_token.as_deref()), ("a2", Some("r2")), "rotated tokens are persisted");
    assert!(!token.is_expired());
    assert_eq!(fresh.list_remote("/").await.unwrap().len(), 2);
}

async fn saved_profile(e: &Env, p: &ConnectionProfile) -> ConnectionProfile {
    e.mgr.get_connection(p.id).await.unwrap().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_sign_in_is_a_rejected_credential() {
    let e = env().await;
    let p = e.saved(ConnectionProtocol::GoogleDrive).await;
    let expired = j!({ "kind": "token", "token": { "access_token": "old", "refresh_token": "revoked", "expires_at": "2020-01-01T00:00:00Z", "token_type": "Bearer", "scope": null }, "client_id": "c" });
    let err = e.gdrive().connect_with_secret(&p, Some(expired.to_string())).await.unwrap_err();
    assert!(err.is_auth_failure(), "{err}");
    // A valid-looking but refused access token: one refresh attempt, then rejected.
    let refused = j!({ "kind": "token", "token": { "access_token": "nope", "refresh_token": null, "expires_at": null, "token_type": "Bearer", "scope": null } });
    assert!(e.gdrive().connect_with_secret(&p, Some(refused.to_string())).await.unwrap_err().is_auth_failure());
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_authid_broker_and_pasted_tokens() {
    let e = env().await;
    let p = e.saved(ConnectionProtocol::GoogleDrive).await;
    let (gd, db, od) = (e.gdrive(), e.dropbox(), e.onedrive());
    let cloud = CloudConnectors { gdrive: &gd, dropbox: &db, onedrive: &od };
    // AuthID from a self-hosted broker (server/oauth.rs): the access token comes from /oauth/token.
    let bad = OAuthGrant::Broker { broker_url: e.mock.base.clone(), authid: "google:unknown".into() };
    assert!(finish_sign_in(&e.mgr, &cloud, p.id, bad).await.unwrap_err().is_auth_failure());
    assert!(e.mgr.get_password(p.id).await.unwrap().is_none(), "a refused credential is not stored");
    let good = OAuthGrant::Broker { broker_url: e.mock.base.clone(), authid: "google:good".into() };
    finish_sign_in(&e.mgr, &cloud, p.id, good).await.unwrap();
    assert!(matches!(e.stored_grant(&p).await, OAuthGrant::Broker { authid, .. } if authid == "google:good"));
    assert_eq!(gd.list_remote("/").await.unwrap().len(), 2);
    // A pasted access token (backup-destination JSON format).
    let p2 = e.saved(ConnectionProtocol::GoogleDrive).await;
    let pasted = OAuthGrant::parse(r#"{"access_token":"pasted"}"#).unwrap();
    finish_sign_in(&e.mgr, &cloud, p2.id, pasted).await.unwrap();
    assert!(gd.is_connected());
}

#[tokio::test(flavor = "multi_thread")]
async fn dropbox_and_onedrive_connect_and_list() {
    let e = env().await;
    let (gd, db, od) = (e.gdrive(), e.dropbox(), e.onedrive());
    let cloud = CloudConnectors { gdrive: &gd, dropbox: &db, onedrive: &od };
    let grant = || OAuthGrant::parse(r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","expires_at":"2999-01-01T00:00:00Z"}"#).unwrap();

    let pd = e.saved(ConnectionProtocol::Dropbox).await;
    assert_eq!(finish_sign_in(&e.mgr, &cloud, pd.id, grant()).await.unwrap().account.as_deref(), Some("bob@example.com"));
    let list = db.list_remote("/").await.unwrap();
    assert_eq!((list[0].name.as_str(), list[0].path.as_str()), ("a.txt", "/a.txt"));
    assert_eq!(e.mock.requests("/dbx/files/list_folder")[0].json()["path"], "");

    let po = e.saved(ConnectionProtocol::OneDrive).await;
    assert_eq!(finish_sign_in(&e.mgr, &cloud, po.id, grant()).await.unwrap().account.as_deref(), Some("cy@example.com"));
    let list = od.list_remote("/").await.unwrap();
    assert_eq!((list[0].name.as_str(), list[0].path.as_str(), list[0].is_dir), ("Docs", "/Docs", true));
    assert!(od.is_connected() && db.is_connected());
    // Expired Dropbox token on the next start: refreshed (and the rotated one saved).
    let OAuthGrant::Token { mut token, client_id, client_secret, token_url } = e.stored_grant(&pd).await else { panic!() };
    token.expires_at = Some(chrono::Utc::now() - chrono::Duration::minutes(1));
    let db2 = e.dropbox();
    db2.connect_with_secret(&pd, Some(OAuthGrant::Token { token, client_id, client_secret, token_url }.to_json().unwrap())).await.unwrap();
    assert!(matches!(e.stored_grant(&pd).await, OAuthGrant::Token { token, .. } if token.access_token == "a2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_cancel_and_manual_code() {
    let e = env().await;
    let p = e.saved(ConnectionProtocol::GoogleDrive).await;
    let flows = Arc::new(OAuthFlows::new());
    let endpoints = || Some(AuthEndpoints { auth_url: format!("{}/auth", e.mock.base), token_url: format!("{}/token", e.mock.base) });
    // Cancelling wakes the waiter.
    let start = flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(0), endpoints()).await.unwrap();
    let f2 = flows.clone();
    let st = start.state.clone();
    let w = tokio::spawn(async move { f2.wait(&st, Duration::from_secs(10)).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    flows.cancel(&start.state).await;
    assert!(w.await.unwrap().is_err());
    // A code pasted from another device (the whole redirect address).
    let start = flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(0), endpoints()).await.unwrap();
    assert!(flows.complete(&start.state, &format!("http://localhost:1/callback?code=c1&state=other")).await.is_err());
    let (id, grant) = flows.complete(&start.state, &format!("http://localhost:1/callback?code=c1&state={}", start.state)).await.unwrap();
    assert_eq!(id, p.id);
    assert!(matches!(grant, OAuthGrant::Token { token, .. } if token.access_token == "a1"));
    // A refused code is a rejected sign-in.
    let start = flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(0), endpoints()).await.unwrap();
    assert!(flows.complete(&start.state, "wrong-code").await.unwrap_err().is_auth_failure());
}
