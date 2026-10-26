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
            "/dbx/files/delete_v2" => json(j!({ "metadata": { ".tag": "file", "name": "x.docx" } })),
            "/content/files/upload" => json(j!({ "name": "x.txt", "path_display": "/in/x.txt", "size": 3 })),
            "/dbx/files/list_folder" => json(j!({ "entries": [{ ".tag": "file", "name": "a.txt", "path_display": "/a.txt", "size": 3 }], "cursor": "c", "has_more": false })),
            "/graph/me/drive" => json(j!({ "id": "d1", "owner": { "user": { "email": "cy@example.com" } }, "quota": { "total": 1000, "used": 10 } })),
            "/dbx/files/move_v2" | "/dbx/files/copy_v2" => json(j!({ "metadata": { ".tag": "file", "name": "b.txt" } })),
            "/graph/me/drive/root:/Archive" => json(j!({ "id": "p9", "name": "Archive", "folder": { "childCount": 0 } })),
            "/graph/me/drive/root:/Docs/a%2Etxt" if r.method == "PATCH" => json(j!({ "id": "i2", "name": "b.txt" })),
            p if p.starts_with("/drive/files/") && r.method == "PATCH" => json(j!({ "id": "x" })),
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
    // Plain connect() (no secret passed) loads the stored sign-in itself.
    let plain = e.gdrive();
    plain.connect(&saved_profile(&e, &p).await).await.unwrap();
    assert!(plain.is_connected());
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
    assert_eq!((list[0].name.as_str(), list[0].path.as_str(), list[0].is_dir), ("Docs", "/drive/root:/Docs", true));
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
    assert!(flows.complete(&start.state, "http://localhost:1/callback?code=c1&state=other").await.is_err());
    let (id, grant) = flows.complete(&start.state, &format!("http://localhost:1/callback?code=c1&state={}", start.state)).await.unwrap();
    assert_eq!(id, p.id);
    assert!(matches!(grant, OAuthGrant::Token { token, .. } if token.access_token == "a1"));
    // A refused code is a rejected sign-in.
    let start = flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(0), endpoints()).await.unwrap();
    assert!(flows.complete(&start.state, "wrong-code").await.unwrap_err().is_auth_failure());
    // Regression: the flow was dropped before the exchange, so a mistyped
    // code meant starting over. It stays open until a code is accepted.
    assert!(flows.is_pending(&start.state).await, "a refused code keeps the sign-in open");
    let (id, _) = flows.complete(&start.state, "c1").await.unwrap();
    assert_eq!(id, p.id);
    assert!(!flows.is_pending(&start.state).await);
}

/// Regression: restarting a sign-in (a new one for the connection, or
/// cancel then sign in again) bound the fixed callback port while the old
/// waiter, not yet scheduled, still held it: "Cannot listen for the
/// sign-in on port 9876". (Single-threaded, so the waiter cannot run first
/// by luck.)
#[tokio::test]
async fn restarting_a_sign_in_reuses_the_callback_port() {
    let p = super::live_tests::profile(ConnectionProtocol::GoogleDrive, "", None, "", "/");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let flows = Arc::new(OAuthFlows::new());
    let ep = || Some(AuthEndpoints { auth_url: "http://127.0.0.1:1/auth".into(), token_url: "http://127.0.0.1:1/token".into() });
    let waiter = |state: String| {
        let f = flows.clone();
        tokio::spawn(async move { f.wait(&state, Duration::from_secs(30)).await })
    };
    let s1 = flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(port), ep()).await.unwrap();
    let w1 = waiter(s1.state.clone());
    tokio::task::yield_now().await; // the waiter now holds the listener
    let s2 = flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(port), ep()).await.expect("a new sign-in replaces the waiting one");
    assert!(w1.await.unwrap().is_err());
    let w2 = waiter(s2.state.clone());
    tokio::task::yield_now().await;
    flows.cancel(&s2.state).await;
    flows.begin(OAuthProvider::GoogleDrive, &p, ("c".into(), None), Some(port), ep()).await.expect("signing in again after Cancel");
    assert!(w2.await.unwrap().is_err());
}

/// Regression: one shared Dropbox instance serves the Connections panel.
/// An automation upload for connection B re-signed it as B, so the user,
/// still browsing A, had their next delete / upload / listing run against
/// account B.
#[tokio::test(flavor = "multi_thread")]
async fn background_work_for_another_account_leaves_the_browsed_one_alone() {
    use crate::commands::connector_commands::{ensure_connected, user_instance};
    let e = env().await;
    let (pa, pb) = (e.saved(ConnectionProtocol::Dropbox).await, e.saved(ConnectionProtocol::Dropbox).await);
    e.mgr.set_password(pa.id, r#"{"access_token":"a1"}"#).await.unwrap();
    e.mgr.set_password(pb.id, r#"{"access_token":"pasted"}"#).await.unwrap();
    let db = Arc::new(e.dropbox());
    let registry = super::ConnectorRegistry::with_cloud(Arc::new(e.gdrive()), db.clone(), Arc::new(e.onedrive()));
    let bearer = |path: &str| e.mock.requests(path).last().and_then(|r| r.header("authorization"));

    // The user browses A (connector_connect).
    db.connect_with_secrets(&pa, e.mgr.get_secrets(pa.id).await.unwrap()).await.unwrap();
    // An automation rule uploads for B.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.txt");
    std::fs::write(&file, b"abc").unwrap();
    let c = ensure_connected(&registry, &e.mgr, "dropbox", Some(&pb.id.to_string())).await.unwrap();
    c.upload(&file.to_string_lossy(), "/in/x.txt").await.unwrap();
    assert_eq!(bearer("/content/files/upload").as_deref(), Some("Bearer pasted"), "B's upload uses B's token");

    // The instance the panel browses is still signed in as A: its listing
    // and delete go to A's account.
    assert_eq!(db.connected_profile_id(), Some(pa.id), "background work must not re-sign the shared instance");
    let shared = registry.get("dropbox").unwrap();
    shared.list_remote("/").await.unwrap();
    assert_eq!(bearer("/dbx/files/list_folder").as_deref(), Some("Bearer a1"));
    shared.remove("/Reports/x.docx", false).await.unwrap();
    assert_eq!(bearer("/dbx/files/delete_v2").as_deref(), Some("Bearer a1"), "the panel's delete must hit A");

    // A sign-in for B (the wizard) does re-sign the shared instance. The
    // panel's file commands name the connection they browse, so they still
    // run as A, on a dedicated instance, leaving the shared one as B.
    db.connect_with_secrets(&pb, e.mgr.get_secrets(pb.id).await.unwrap()).await.unwrap();
    let panel = user_instance(&registry, &e.mgr, "dropbox", Some(&pa.id.to_string())).await.unwrap();
    panel.remove("/Reports/y.docx", false).await.unwrap();
    assert_eq!(bearer("/dbx/files/delete_v2").as_deref(), Some("Bearer a1"));
    panel.list_remote("/").await.unwrap();
    assert_eq!(bearer("/dbx/files/list_folder").as_deref(), Some("Bearer a1"));
    assert_eq!(db.connected_profile_id(), Some(pb.id));
    // Commands that name no connection keep using the shared instance.
    let unnamed = user_instance(&registry, &e.mgr, "dropbox", None).await.unwrap();
    assert_eq!(unnamed.connected_profile_id(), Some(pb.id));
}

/// Regression: the one shared Dropbox instance stayed signed in as the
/// last-connected account, so an action for connection B (automation
/// upload, share link, quota, permanent delete) ran against account A.
/// Each action now runs signed in as the connection it names, without
/// re-signing the shared instance.
#[tokio::test(flavor = "multi_thread")]
async fn an_action_runs_signed_in_as_the_connection_it_names() {
    use crate::commands::connector_commands::{ensure_connected, user_instance};
    let e = env().await;
    let (pa, pb) = (e.saved(ConnectionProtocol::Dropbox).await, e.saved(ConnectionProtocol::Dropbox).await);
    e.mgr.set_password(pa.id, r#"{"access_token":"a1"}"#).await.unwrap();
    e.mgr.set_password(pb.id, r#"{"access_token":"pasted"}"#).await.unwrap();
    let db = Arc::new(e.dropbox());
    let registry = super::ConnectorRegistry::with_cloud(Arc::new(e.gdrive()), db.clone(), Arc::new(e.onedrive()));
    let last_bearer = || e.mock.requests("/dbx/files/list_folder").last().and_then(|r| r.header("authorization"));
    let (a, b) = (pa.id.to_string(), pb.id.to_string());

    // The panel connects A.
    db.connect_with_secrets(&pa, e.mgr.get_secrets(pa.id).await.unwrap()).await.unwrap();
    db.list_remote("/").await.unwrap();
    assert_eq!(last_bearer().as_deref(), Some("Bearer a1"));
    // An interactive action for B (quota, share link) runs as B.
    user_instance(&registry, &e.mgr, "dropbox", Some(&b)).await.unwrap().list_remote("/").await.unwrap();
    assert_eq!(last_bearer().as_deref(), Some("Bearer pasted"), "B's action must use B's token");
    // A background action for B (automation upload) runs as B too.
    ensure_connected(&registry, &e.mgr, "dropbox", Some(&b)).await.unwrap().list_remote("/").await.unwrap();
    assert_eq!(last_bearer().as_deref(), Some("Bearer pasted"), "B's automation must use B's token");
    assert_eq!(db.connected_profile_id(), Some(pa.id), "neither re-signs the shared instance");
    // Already signed in as A: the shared instance, no new sign-in check.
    let checks = e.mock.requests("/dbx/users/get_current_account").len();
    let c = ensure_connected(&registry, &e.mgr, "dropbox", Some(&a)).await.unwrap();
    assert_eq!(c.connected_profile_id(), Some(pa.id));
    user_instance(&registry, &e.mgr, "dropbox", Some(&a)).await.unwrap();
    assert_eq!(e.mock.requests("/dbx/users/get_current_account").len(), checks);

    // B flagged as credentials-rejected: background work for B is refused,
    // a user action for B (share link, quota) still tries — and succeeds.
    e.mgr.record_connect_outcome(pb.id, &Err::<(), _>(crate::core::error::AppError::auth("refused", "sign in"))).await;
    assert!(ensure_connected(&registry, &e.mgr, "dropbox", Some(&b)).await.err().is_some_and(|e| e.is_auth_failure()));
    let ub = user_instance(&registry, &e.mgr, "dropbox", Some(&b)).await.unwrap();
    assert_eq!(ub.connected_profile_id(), Some(pb.id));
    assert_eq!(db.connected_profile_id(), Some(pa.id), "A stays in place");
    db.disconnect().await.unwrap();
    assert_eq!(db.connected_profile_id(), None);
}

/// B2 remembers whose application key it authorized with.
#[tokio::test(flavor = "multi_thread")]
async fn b2_tracks_the_connection_it_is_authorized_as() {
    let mock = serve(move |r| {
        let base = format!("http://{}", r.header("host").unwrap_or_default());
        json(j!({ "accountId": "acc", "authorizationToken": "tok", "apiUrl": base, "downloadUrl": base, "recommendedPartSize": 100u64, "absoluteMinimumPartSize": 5u64, "allowed": { "capabilities": [] } }))
    })
    .await;
    let c = super::B2Connector::with_authorize_url(&format!("{}/b2api/v2/b2_authorize_account", mock.base));
    let (pa, pb) = (super::live_tests::profile(ConnectionProtocol::B2, "", None, "kid", "/"), super::live_tests::profile(ConnectionProtocol::B2, "", None, "kid2", "/"));
    c.connect_with_secret(&pa, Some("k".into())).await.unwrap();
    assert!(super::connected_as(&c, Some(pa.id)) && !super::connected_as(&c, Some(pb.id)));
    // A failed attempt for B (no key) keeps the A authorization and owner.
    assert!(c.connect_with_secret(&pb, None).await.is_err());
    assert_eq!(c.connected_profile_id(), Some(pa.id));
    c.connect_with_secret(&pb, Some("k2".into())).await.unwrap();
    assert_eq!(c.connected_profile_id(), Some(pb.id));
}

/// Regression: signing out connection B disconnected the one shared
/// instance whatever account it held, dropping connection A, which the user
/// was browsing. Only an instance signed in as B is disconnected.
#[tokio::test(flavor = "multi_thread")]
async fn signing_out_another_connection_keeps_the_browsed_one() {
    use crate::commands::oauth_commands::sign_out;
    let e = env().await;
    let (pa, pb) = (e.saved(ConnectionProtocol::Dropbox).await, e.saved(ConnectionProtocol::Dropbox).await);
    e.mgr.set_password(pa.id, r#"{"access_token":"a1"}"#).await.unwrap();
    e.mgr.set_password(pb.id, r#"{"access_token":"pasted"}"#).await.unwrap();
    let (gd, db, od) = (e.gdrive(), e.dropbox(), e.onedrive());
    let cloud = CloudConnectors { gdrive: &gd, dropbox: &db, onedrive: &od };
    db.connect_with_secrets(&pa, e.mgr.get_secrets(pa.id).await.unwrap()).await.unwrap();

    sign_out(&e.mgr, &cloud, &pb.id.to_string()).await.unwrap();
    assert!(e.mgr.get_password(pb.id).await.unwrap().is_none(), "B's sign-in is forgotten");
    assert!(db.is_connected(), "A stays connected");
    assert_eq!(db.connected_profile_id(), Some(pa.id));
    assert!(e.mgr.get_password(pa.id).await.unwrap().is_some());

    // Signing out A, which the instance holds, disconnects it.
    sign_out(&e.mgr, &cloud, &pa.id.to_string()).await.unwrap();
    assert!(!db.is_connected());
    assert!(e.mgr.get_password(pa.id).await.unwrap().is_none());
}

/// Pane renames and moves within one cloud connection (`rename_entry`) and
/// Dropbox's server-side copy (`server_copy`).
#[tokio::test(flavor = "multi_thread")]
async fn cloud_rename_move_and_server_copy() {
    let e = env().await;
    let (gd, db, od) = (e.gdrive(), e.dropbox(), e.onedrive());
    let cloud = CloudConnectors { gdrive: &gd, dropbox: &db, onedrive: &od };
    let grant = || OAuthGrant::parse(r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","expires_at":"2999-01-01T00:00:00Z"}"#).unwrap();

    // Google Drive: same folder = rename by id; another folder = move first.
    let pg = e.saved(ConnectionProtocol::GoogleDrive).await;
    finish_sign_in(&e.mgr, &cloud, pg.id, grant()).await.unwrap();
    gd.rename_entry("/f1/f2", "/f1/renamed.txt").await.unwrap();
    let patch = e.mock.requests("/drive/files/f2");
    assert_eq!((patch.len(), patch[0].json()["name"].as_str()), (1, Some("renamed.txt")));
    gd.rename_entry("/f1/f2", "/f9/renamed.txt").await.unwrap();
    let patch = e.mock.requests("/drive/files/f2");
    assert_eq!(patch.len(), 3, "a move PATCH, then the name");
    assert_eq!((patch[1].query_param("addParents").as_deref(), patch[1].query_param("removeParents").as_deref()), (Some("f9"), Some("f1")));
    // A native document downloads with its export extension.
    let doc = crate::core::types::FileEntry { name: "Plan".into(), path: "/d1".into(), is_dir: false, is_symlink: false, size: 0, modified: None, created: None, is_hidden: false, extension: Some("gdoc".into()), permissions: None };
    assert_eq!(gd.download_name(&doc), "Plan.docx");
    assert_eq!(od.download_name(&doc), "Plan", "other providers keep the name");

    // OneDrive: PATCH by path, with the new parent looked up by path.
    let po = e.saved(ConnectionProtocol::OneDrive).await;
    finish_sign_in(&e.mgr, &cloud, po.id, grant()).await.unwrap();
    od.rename_entry("/drive/root:/Docs/a.txt", "/Archive/b.txt").await.unwrap();
    let patch = e.mock.requests("/graph/me/drive/root:/Docs/a%2Etxt");
    assert_eq!(patch[0].json(), j!({ "name": "b.txt", "parentReference": { "id": "p9" } }));
    od.rename_entry("/Docs/a.txt", "/Docs/b.txt").await.unwrap();
    assert_eq!(e.mock.requests("/graph/me/drive/root:/Docs/a%2Etxt")[1].json(), j!({ "name": "b.txt" }));
    assert!(od.rename_entry("/Docs/a.txt", "/Docs/bad:name").await.is_err(), "OneDrive naming rules apply");

    // Dropbox: move_v2 and copy_v2.
    let pd = e.saved(ConnectionProtocol::Dropbox).await;
    finish_sign_in(&e.mgr, &cloud, pd.id, grant()).await.unwrap();
    db.rename_entry("/a.txt", "/in/b.txt").await.unwrap();
    assert_eq!(e.mock.requests("/dbx/files/move_v2")[0].json()["to_path"], "/in/b.txt");
    assert!(db.server_copy("/a.txt", "/in/c.txt").await.unwrap());
    assert_eq!(e.mock.requests("/dbx/files/copy_v2")[0].json()["from_path"], "/a.txt");
    // A connector without a server-side copy relays instead.
    assert!(!super::nfs::NfsConnector::new().server_copy("/a", "/b").await.unwrap());
}

/// Regression (WS-Z1): the tokens from a finished sign-in were stored only
/// after connecting with them worked. A network error at that moment threw
/// them away, and the one-time code could not be exchanged again: the user
/// had to sign in once more. Now they are stored, and the result says the
/// service could not be reached yet.
#[tokio::test(flavor = "multi_thread")]
async fn a_network_error_after_sign_in_keeps_the_tokens() {
    let e = env().await;
    let p = e.saved(ConnectionProtocol::GoogleDrive).await;
    // The API cannot be reached (nothing listens on that port).
    let gd = GoogleDriveConnector::new().with_credentials(e.store.clone()).with_endpoints("http://127.0.0.1:9/drive", "http://127.0.0.1:9/upload", "http://127.0.0.1:9/token");
    let (db, od) = (e.dropbox(), e.onedrive());
    let cloud = CloudConnectors { gdrive: &gd, dropbox: &db, onedrive: &od };
    let grant = OAuthGrant::parse(r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","expires_at":"2999-01-01T00:00:00Z"}"#).unwrap();
    let signed = finish_sign_in(&e.mgr, &cloud, p.id, grant).await.expect("signed in although the service was not reachable");
    assert!(signed.warning.as_deref().is_some_and(|w| w.contains("sign-in is kept")), "{:?}", signed.warning);
    assert!(matches!(e.stored_grant(&p).await, OAuthGrant::Token { token, .. } if token.refresh_token.as_deref() == Some("r1")));
    // Review: the custom app the sign-in used is remembered here too.
    let saved = e.mgr.get_connection(p.id).await.unwrap().unwrap();
    assert_eq!(saved.extras.oauth.and_then(|o| o.client_id).as_deref(), Some("c"));
}
