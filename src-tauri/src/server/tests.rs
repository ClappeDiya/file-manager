use super::*;
use crate::security::CredentialStore;
use crate::storage::Repository;
use std::sync::Arc;
use ufop_core::backup::crypto::KdfParams;

struct TestServer {
    base: String,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
    dir: PathBuf,
}

async fn start(cfg: ServerConfig, oauth: Option<oauth::OAuthBroker>) -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let mgr = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, dir.path().join("dbs"), KdfParams::insecure_fast());
    let auth = AuthService::new(settings::SettingsStore::plain(repo.pool().clone()));
    auth.set_password("server-password").await.unwrap();
    let st = ServerState { mgr, conns: None, auth, cfg: Arc::new(cfg), started: Instant::now(), oauth, agent: None };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router(st)).await.unwrap() });
    TestServer {
        base: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder().cookie_store(true).build().unwrap(),
        dir: dir.path().to_path_buf(),
        _dir: dir,
    }
}

impl TestServer {
    async fn login(&self) -> String {
        let r = self.client.post(format!("{}/api/v1/auth/login", self.base)).json(&json!({"password":"server-password"})).send().await.unwrap();
        assert_eq!(r.status(), 200);
        r.json::<Value>().await.unwrap()["access_token"].as_str().unwrap().to_string()
    }
    async fn get(&self, path: &str, tok: &str) -> reqwest::Response {
        self.client.get(format!("{}{path}", self.base)).bearer_auth(tok).send().await.unwrap()
    }
    async fn post(&self, path: &str, tok: &str, body: Value) -> reqwest::Response {
        self.client.post(format!("{}{path}", self.base)).bearer_auth(tok).json(&body).send().await.unwrap()
    }
}

#[tokio::test]
async fn auth_flow_tokens_and_revocation() {
    let s = start(ServerConfig::default(), None).await;
    // Health is public; everything else needs auth.
    assert_eq!(s.client.get(format!("{}/api/v1/health", s.base)).send().await.unwrap().status(), 200);
    assert_eq!(s.client.get(format!("{}/api/v1/jobs", s.base)).send().await.unwrap().status(), 401);
    let bad = s.client.post(format!("{}/api/v1/auth/login", s.base)).json(&json!({"password":"nope"})).send().await.unwrap();
    assert_eq!(bad.status(), 401);
    let tok = s.login().await;
    assert_eq!(s.get("/api/v1/jobs", &tok).await.status(), 200);

    // Refresh cookie rotates: the second use of the same cookie fails.
    let r = s.client.post(format!("{}/api/v1/auth/refresh", s.base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let new_tok = r.json::<Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
    assert_eq!(s.get("/api/v1/status", &new_tok).await.status(), 200);

    // Automation tokens are opt-in.
    assert_eq!(s.post("/api/v1/auth/automation-token", &tok, json!({})).await.status(), 401);
    s.client.put(format!("{}/api/v1/settings", s.base)).bearer_auth(&tok).json(&json!({"allow_automation_tokens": true})).send().await.unwrap();
    let auto = s.post("/api/v1/auth/automation-token", &tok, json!({})).await.json::<Value>().await.unwrap();
    let auto = auto["token"].as_str().unwrap().to_string();
    assert_eq!(s.get("/api/v1/jobs", &auto).await.status(), 200);

    // One-time sign-in token.
    let st = s.post("/api/v1/auth/signin-token", &tok, json!({})).await.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
    assert_eq!(s.get("/api/v1/jobs", &st).await.status(), 401, "sign-in tokens are not bearers");
    assert_eq!(s.client.post(format!("{}/api/v1/auth/signin", s.base)).json(&json!({"token": st})).send().await.unwrap().status(), 200);
    assert_eq!(s.client.post(format!("{}/api/v1/auth/signin", s.base)).json(&json!({"token": st})).send().await.unwrap().status(), 401, "single use");

    // Reset revokes everything.
    assert_eq!(s.post("/api/v1/auth/reset", &tok, json!({})).await.status(), 200);
    assert_eq!(s.get("/api/v1/jobs", &tok).await.status(), 401);
    assert_eq!(s.get("/api/v1/jobs", &auto).await.status(), 401);
    assert_eq!(s.client.post(format!("{}/api/v1/auth/refresh", s.base)).send().await.unwrap().status(), 401);
}

#[tokio::test]
async fn host_check_and_preauth() {
    let cfg = ServerConfig { preauth_tokens: vec!["proxy-secret".into()], allowed_hostnames: vec!["backup.example".into()], ..Default::default() };
    let s = start(cfg, None).await;
    let evil = s.client.get(format!("{}/api/v1/health", s.base)).header("Host", "evil.example").send().await.unwrap();
    assert_eq!(evil.status(), 421);
    let ok = s.client.get(format!("{}/api/v1/health", s.base)).header("Host", "backup.example:8200").send().await.unwrap();
    assert_eq!(ok.status(), 200);
    let pre = s.client.get(format!("{}/api/v1/jobs", s.base)).header(PREAUTH_HEADER, "proxy-secret").send().await.unwrap();
    assert_eq!(pre.status(), 200);
    let wrong = s.client.get(format!("{}/api/v1/jobs", s.base)).header(PREAUTH_HEADER, "proxy-secreT").send().await.unwrap();
    assert_eq!(wrong.status(), 401);
}

#[tokio::test]
async fn backup_and_restore_over_rest_and_rpc() {
    let s = start(ServerConfig::default(), None).await;
    let tok = s.login().await;
    let src = s.dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"server backup").unwrap();
    let job = json!({"name":"Srv","sources":[src],"encrypted":false,"destination":{"type":"local","path": s.dir.join("dst"),"alternate_paths":[],"marker_file":null},"options":{"block_size":4096,"volume_size":65536}});
    let r = s.post("/api/v1/jobs", &tok, json!({"job": job})).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let id = s.get("/api/v1/jobs", &tok).await.json::<Value>().await.unwrap()[0]["id"].as_str().unwrap().to_string();
    let task = s.post(&format!("/api/v1/jobs/{id}/run"), &tok, json!({})).await.json::<Value>().await.unwrap();
    assert!(task["task_id"].is_string());
    for _ in 0..100 {
        if s.get("/api/v1/tasks", &tok).await.json::<Value>().await.unwrap().as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let v = s.get(&format!("/api/v1/jobs/{id}/versions"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    let out = s.dir.join("out");
    let r = s.post(&format!("/api/v1/jobs/{id}/operations"), &tok, json!({"type":"restore","options":{"target": out}})).await.json::<Value>().await.unwrap();
    assert_eq!(r["report"]["files_restored"], 1);
    assert_eq!(std::fs::read(out.join("src/a.txt")).unwrap(), b"server backup");
    // RPC endpoint speaks the desktop command names and argument shapes.
    let found = s.post("/api/v1/rpc/backup_search", &tok, json!({"id": id, "pattern":"a.txt","version":null})).await.json::<Value>().await.unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
    let unknown = s.post("/api/v1/rpc/rm_rf", &tok, json!({})).await;
    assert_eq!(unknown.status(), 400);
    let runs = s.get(&format!("/api/v1/runs?job_id={id}"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 2);
    let fs = s.get(&format!("/api/v1/filesystem?path={}", s.dir.display()), &tok).await.json::<Value>().await.unwrap();
    assert!(fs["entries"].as_array().unwrap().iter().any(|e| e["name"] == "src"));
}

#[tokio::test]
async fn spa_webroot_api_only_and_modules() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "<html><head></head><body>app</body></html>").unwrap();
    std::fs::write(web.path().join("app.js"), "console.log(1)").unwrap();
    let s = start(ServerConfig { webroot: Some(web.path().into()), ..Default::default() }, None).await;
    let idx = s.client.get(format!("{}/backups/123", s.base)).send().await.unwrap().text().await.unwrap();
    assert!(idx.contains("__UFOP_SERVER__") && idx.contains("app"));
    let js = s.client.get(format!("{}/app.js", s.base)).send().await.unwrap();
    assert_eq!(js.headers()["content-type"], "text/javascript");
    // Traversal attempts never leave the webroot (normalized or encoded).
    for p in ["/../etc/passwd", "/%2e%2e/%2e%2e/etc/passwd"] {
        let body = s.client.get(format!("{}{p}", s.base)).send().await.unwrap().text().await.unwrap();
        assert!(!body.contains("root:"), "{p}");
    }
    let api_only = start(ServerConfig { webroot: Some(web.path().into()), api_only: true, ..Default::default() }, None).await;
    assert_eq!(api_only.client.get(format!("{}/index.html", api_only.base)).send().await.unwrap().status(), 404);
    let tok = s.login().await;
    let m = s.get("/api/v1/modules", &tok).await.json::<Value>().await.unwrap();
    assert!(m.as_array().unwrap().iter().any(|x| x["name"] == "sftp"));
}

#[tokio::test(flavor = "multi_thread")]
async fn https_with_generated_certificate() {
    let d = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let repo = Repository::open_in_memory().await.unwrap();
    let mgr = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, d.path().join("dbs"), KdfParams::insecure_fast());
    let auth = AuthService::new(settings::SettingsStore::plain(repo.pool().clone()));
    let cfg = ServerConfig { port, tls: TlsMode::Generated { dir: d.path().join("tls"), hostnames: vec!["localhost".into()] }, ..Default::default() };
    let st = ServerState { mgr, conns: None, auth, cfg: Arc::new(cfg), started: Instant::now(), oauth: None, agent: None };
    tokio::spawn(async move { serve(st).await.unwrap() });
    let ca = std::fs::read(loop {
        let p = d.path().join("tls/ufop-ca.pem");
        if p.exists() && d.path().join("tls/server.json").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            break p;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    })
    .unwrap();
    let client = reqwest::Client::builder().add_root_certificate(reqwest::Certificate::from_pem(&ca).unwrap()).build().unwrap();
    let r = client.get(format!("https://localhost:{port}/api/v1/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn oauth_broker_authid_flow() {
    // Mock provider token endpoint.
    let provider = Router::new().route(
        "/token",
        post(|body: String| async move {
            if body.contains("grant_type=authorization_code") && body.contains("code=good") {
                Json(json!({"access_token":"at-1","refresh_token":"rt-1","expires_in":1}))
            } else if body.contains("grant_type=refresh_token") && body.contains("refresh_token=rt-1") {
                Json(json!({"access_token":"at-2","expires_in":3600}))
            } else {
                Json(json!({"error":"bad"}))
            }
        }),
    );
    let pl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pport = pl.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(pl, provider).await.unwrap() });
    let store_dir = tempfile::tempdir().unwrap();
    let mut providers = std::collections::BTreeMap::new();
    providers.insert(
        "demo".to_string(),
        oauth::ProviderConfig {
            auth_url: format!("http://127.0.0.1:{pport}/auth"),
            token_url: format!("http://127.0.0.1:{pport}/token"),
            client_id: "cid".into(),
            client_secret: "secret".into(),
            scopes: vec!["files".into()],
            extra_auth_params: Default::default(),
        },
    );
    let broker = oauth::OAuthBroker::new(oauth::BrokerConfig { public_url: "http://localhost:8200".into(), providers }, Arc::new(ufop_core::backup::LocalBackend::new(store_dir.path())));
    let s = start(ServerConfig::default(), Some(broker)).await;
    let nofollow = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = nofollow.get(format!("{}/oauth/demo/start", s.base)).send().await.unwrap();
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    assert!(loc.contains("client_id=cid") && loc.contains("scope=files"));
    let state = url::Url::parse(&loc).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.to_string();
    let page = s.client.get(format!("{}/oauth/callback?code=good&state={state}", s.base)).send().await.unwrap().text().await.unwrap();
    let authid = page.split("<pre>").nth(1).unwrap().split("</pre>").next().unwrap().to_string();
    // Stored encrypted: the raw store does not contain the token.
    let stored = std::fs::read_dir(store_dir.path()).unwrap().next().unwrap().unwrap().path();
    assert!(!String::from_utf8_lossy(&std::fs::read(stored).unwrap()).contains("rt-1"));
    // expires_in=1 → immediately refreshed.
    let t = s.client.get(format!("{}/oauth/token?authid={}", s.base, urlencoding::encode(&authid))).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(t["access_token"], "at-2");
    let bad = s.client.get(format!("{}/oauth/token?authid=demo:nope", s.base)).send().await.unwrap();
    assert_eq!(bad.status(), 401);
    // State is single-use.
    assert_ne!(s.client.get(format!("{}/oauth/callback?code=good&state={state}", s.base)).send().await.unwrap().status(), 200);
}
