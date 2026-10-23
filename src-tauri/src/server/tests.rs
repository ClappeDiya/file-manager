use super::*;
use crate::security::CredentialStore;
use crate::storage::Repository;
use std::sync::Arc;
use ufop_core::backup::crypto::KdfParams;

struct TestServer {
    base: String,
    mgr: BackupManager,
    client: reqwest::Client,
    jar: Arc<reqwest::cookie::Jar>,
    _dir: tempfile::TempDir,
    dir: PathBuf,
}

async fn start(cfg: ServerConfig, oauth: Option<oauth::OAuthBroker>) -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let mgr = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, dir.path().join("dbs"), KdfParams::insecure_fast());
    let auth = AuthService::new(settings::SettingsStore::plain(repo.pool().clone()));
    auth.set_password("server-password").await.unwrap();
    let st = ServerState { oauth, ..ServerState::new(mgr.clone(), None, auth, cfg) };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router(st).into_make_service_with_connect_info::<SocketAddr>()).await.unwrap() });
    let jar = Arc::new(reqwest::cookie::Jar::default());
    TestServer {
        base: format!("http://127.0.0.1:{port}"),
        mgr,
        jar: jar.clone(),
        client: reqwest::Client::builder().cookie_provider(jar).build().unwrap(),
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
    /// The `ufop_csrf` cookie the client holds.
    fn csrf(&self) -> String {
        use reqwest::cookie::CookieStore;
        self.jar.cookies(&self.base.parse().unwrap()).and_then(|h| h.to_str().ok().and_then(|c| c.split("; ").find_map(|kv| kv.strip_prefix("ufop_csrf=").map(String::from)))).unwrap_or_default()
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
    let r = s.client.post(format!("{}/api/v1/auth/refresh", s.base)).header("X-CSRF-Token", s.csrf()).send().await.unwrap();
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
    let after_reset = s.client.post(format!("{}/api/v1/auth/refresh", s.base)).header("X-CSRF-Token", s.csrf()).send().await.unwrap().status();
    assert!(after_reset == 401 || after_reset == 403, "{after_reset}");
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
    // Repository lock status and guarded force-unlock.
    let lock = s.get(&format!("/api/v1/jobs/{id}/lock"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(lock["locks"].as_array().unwrap().len(), 0);
    assert_eq!(lock["exclusive"], false);
    std::fs::write(s.dir.join("dst/ufop-lock.json"), serde_json::to_vec(&json!({"owner_id":"x","hostname":"nas","operation":"compact","kind":"exclusive","acquired_at":"2030-01-01T00:00:00Z","expires_at":"2030-01-01T00:30:00Z"})).unwrap()).unwrap();
    let lock = s.get(&format!("/api/v1/jobs/{id}/lock"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!((lock["exclusive"].as_bool(), lock["locks"][0]["hostname"].as_str()), (Some(true), Some("nas")));
    let refused = s.client.delete(format!("{}/api/v1/jobs/{id}/lock", s.base)).bearer_auth(&tok).send().await.unwrap();
    assert!(!refused.status().is_success(), "needs confirm=true");
    let broken = s.client.delete(format!("{}/api/v1/jobs/{id}/lock?confirm=true&owner_id=x", s.base)).bearer_auth(&tok).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(broken["removed"][0], "ufop-lock.json");
    let rpc = s.post("/api/v1/rpc/backup_lock_status", &tok, json!({"jobId": id})).await.json::<Value>().await.unwrap();
    assert_eq!(rpc["locks"].as_array().unwrap().len(), 0);
    // Local database encryption on demand.
    let enc = s.client.put(format!("{}/api/v1/jobs/{id}/database-encryption", s.base)).bearer_auth(&tok).json(&json!({"enabled": true})).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(enc["encrypted"], true);
    assert_eq!(s.get(&format!("/api/v1/jobs/{id}/versions"), &tok).await.json::<Value>().await.unwrap().as_array().unwrap().len(), 1);
    // Version metadata, tag filter, sources and the retention preview.
    let patched = s.client.patch(format!("{}/api/v1/jobs/{id}/versions/0", s.base)).bearer_auth(&tok).json(&json!({"note": "first", "pinned": true, "tags": {"env": "prod"}})).send().await.unwrap();
    assert_eq!(patched.status(), 200, "{}", patched.text().await.unwrap());
    let v = s.get(&format!("/api/v1/jobs/{id}/versions?tags=env=prod"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!((v[0]["note"].as_str(), v[0]["pinned"].as_bool()), (Some("first"), Some(true)));
    assert!(v[0]["stats"]["bytes_added"].as_u64().unwrap() > 0);
    assert!(s.get(&format!("/api/v1/jobs/{id}/versions?tags=env=dev"), &tok).await.json::<Value>().await.unwrap().as_array().unwrap().is_empty());
    let srcs = s.get(&format!("/api/v1/jobs/{id}/sources"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(srcs[0]["own"], true);
    let p = s.post(&format!("/api/v1/jobs/{id}/retention/preview"), &tok, json!({"policy": {"type": "counts", "keep_daily": 7}})).await.json::<Value>().await.unwrap();
    assert_eq!((p["keep"].as_u64(), p["versions"][0]["summary"].as_str()), (Some(1), Some("kept: newest version, daily 7, pinned")));
    let rpc = s.post("/api/v1/rpc/backup_preview_retention", &tok, json!({"id": id})).await.json::<Value>().await.unwrap();
    assert_eq!(rpc["delete"], 0);
    let unknown = s.post("/api/v1/rpc/rm_rf", &tok, json!({})).await;
    assert_eq!(unknown.status(), 400);
    let runs = s.get(&format!("/api/v1/runs?job_id={id}"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 3);
    let fs = s.get(&format!("/api/v1/filesystem?path={}", s.dir.display()), &tok).await.json::<Value>().await.unwrap();
    assert!(fs["entries"].as_array().unwrap().iter().any(|e| e["name"] == "src"));
    // Restore by time, one file's bytes (with ranges) and the volume cache.
    let cache = s.client.put(format!("{}/api/v1/backup-cache/settings", s.base)).bearer_auth(&tok).json(&json!({"dir": s.dir.join("cache"), "max_bytes": 1u64 << 30})).send().await.unwrap();
    assert_eq!(cache.status(), 200, "{}", cache.text().await.unwrap());
    let r = s.get(&format!("/api/v1/jobs/{id}/versions/resolve?time=2099-01-01T00:00:00Z"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(r["index"], 0, "{r}");
    assert_eq!(s.get(&format!("/api/v1/jobs/{id}/versions/resolve?time=2001-01-01T00:00:00Z"), &tok).await.status(), 400, "before every version");
    let files = s.get(&format!("/api/v1/jobs/{id}/files?version=latest"), &tok).await.json::<Value>().await.unwrap();
    assert!(files.as_array().unwrap().len() >= 2);
    let fpath = src.join("a.txt").to_string_lossy().replace('/', "%2F");
    let full = s.get(&format!("/api/v1/jobs/{id}/file?path={fpath}&version=latest"), &tok).await;
    assert_eq!(full.status(), 200);
    assert_eq!(full.headers()["accept-ranges"], "bytes");
    assert_eq!(full.bytes().await.unwrap().as_ref(), b"server backup");
    let part = s.client.get(format!("{}/api/v1/jobs/{id}/file?path={fpath}&time=2099-01-01%2000:00", s.base)).bearer_auth(&tok).header("Range", "bytes=7-").send().await.unwrap();
    assert_eq!(part.status(), 206);
    assert_eq!(part.headers()["content-range"], "bytes 7-12/13");
    assert_eq!(part.bytes().await.unwrap().as_ref(), b"backup");
    let bad = s.client.get(format!("{}/api/v1/jobs/{id}/file?path={fpath}", s.base)).bearer_auth(&tok).header("Range", "bytes=99-").send().await.unwrap();
    assert_eq!(bad.status(), 416);
    let stats = s.get("/api/v1/backup-cache", &tok).await.json::<Value>().await.unwrap();
    assert!(stats["entries"].as_u64().unwrap() >= 1, "{stats}");
    let cleared = s.client.delete(format!("{}/api/v1/backup-cache", s.base)).bearer_auth(&tok).send().await.unwrap().json::<Value>().await.unwrap();
    assert!(cleared["bytes_freed"].as_u64().unwrap() > 0);
    let rpc = s.post("/api/v1/rpc/backup_read_file", &tok, json!({"id": id, "path": src.join("a.txt"), "offset": 0, "length": 6})).await.json::<Value>().await.unwrap();
    assert_eq!(rpc["data"], "c2VydmVy", "{rpc}");
    // A mirror restore over the API needs confirm_delete.
    let mirror = s.post(&format!("/api/v1/jobs/{id}/operations"), &tok, json!({"type":"restore","options":{"target": out, "delete_extraneous": true}})).await;
    assert_eq!(mirror.status(), 409);
    let mirror = s.post(&format!("/api/v1/jobs/{id}/operations"), &tok, json!({"type":"restore","options":{"target": out, "delete_extraneous": true},"overrides":{"confirm_delete": true}})).await.json::<Value>().await.unwrap();
    assert_eq!(mirror["status"], "success", "{mirror}");
    // Estimate and run-all.
    std::fs::write(src.join("b.txt"), b"new since the backup").unwrap();
    let est = s.get(&format!("/api/v1/jobs/{id}/estimate?assumed_bytes_per_sec=1000"), &tok).await.json::<Value>().await.unwrap();
    assert_eq!((est["new_files"].as_u64(), est["unchanged_files"].as_u64()), (Some(1), Some(1)), "{est}");
    let rpc_est = s.post("/api/v1/rpc/backup_estimate", &tok, json!({"jobId": id})).await.json::<Value>().await.unwrap();
    assert_eq!(rpc_est["new_files"], 1);
    let all = s.post("/api/v1/jobs/run-all", &tok, json!({"dry_run": true})).await.json::<Value>().await.unwrap();
    assert_eq!(all["started"].as_array().map(|a| a.len()), Some(1), "{all}");
    for _ in 0..100 {
        if s.get("/api/v1/tasks", &tok).await.json::<Value>().await.unwrap().as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
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
    let st = ServerState::new(mgr, None, auth, cfg);
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

#[tokio::test]
async fn client_address_allowlist_by_tcp_peer() {
    let deny = start(ServerConfig { allowed_clients: ufop_core::netacl::IpAllowlist::parse(&["10.0.0.0/8"]).unwrap(), ..Default::default() }, None).await;
    let r = deny.client.get(format!("{}/api/v1/health", deny.base)).header("x-forwarded-for", "10.1.2.3").send().await.unwrap();
    assert_eq!(r.status(), 403, "a forwarded header does not change the peer");
    assert!(r.json::<Value>().await.unwrap()["message"].as_str().unwrap().contains("127.0.0.1"));
    let allow = start(ServerConfig { allowed_clients: ufop_core::netacl::IpAllowlist::parse(&["127.0.0.0/8", "::1"]).unwrap(), ..Default::default() }, None).await;
    assert_eq!(allow.client.get(format!("{}/api/v1/health", allow.base)).send().await.unwrap().status(), 200);
    let tok = allow.login().await;
    assert_eq!(allow.get("/api/v1/jobs", &tok).await.status(), 200);
}

#[tokio::test]
async fn scoped_expiring_api_tokens() {
    let s = start(ServerConfig::default(), None).await;
    let tok = s.login().await;
    let mint = |scope: &'static str| s.post("/api/v1/auth/api-tokens", &tok, json!({"name": format!("{scope} token"), "scope": scope, "expires_in_days": 30}));
    let read = mint("read").await.json::<Value>().await.unwrap();
    let read_tok = read["token"].as_str().unwrap().to_string();
    assert!(read_tok.starts_with("ufop_pat_"));
    assert_eq!(read["info"]["scope"], "read");
    let run_tok = mint("run").await.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
    let admin_tok = mint("admin").await.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
    assert_eq!(s.post("/api/v1/auth/api-tokens", &tok, json!({"name": "x", "scope": "read", "expires_in_days": 400})).await.status(), 400);

    // read: GETs only.
    assert_eq!(s.get("/api/v1/jobs", &read_tok).await.status(), 200);
    let r = s.post("/api/v1/jobs/nope/run", &read_tok, json!({})).await;
    assert_eq!(r.status(), 401);
    assert!(r.json::<Value>().await.unwrap()["message"].as_str().unwrap().contains("needs run"));
    // run: may start backups (the job does not exist, so not found rather than refused) but not change jobs.
    assert_ne!(s.post("/api/v1/jobs/nope/run", &run_tok, json!({})).await.status(), 401);
    assert_eq!(s.post("/api/v1/scheduler/pause", &run_tok, json!({})).await.status(), 200);
    assert_eq!(s.client.delete(format!("{}/api/v1/jobs/nope", s.base)).bearer_auth(&run_tok).send().await.unwrap().status(), 401);
    assert_eq!(s.get("/api/v1/auth/api-tokens", &run_tok).await.status(), 401);
    // admin: lists tokens, but cannot mint new ones.
    let list = s.get("/api/v1/auth/api-tokens", &admin_tok).await.json::<Value>().await.unwrap();
    assert_eq!(list["tokens"].as_array().unwrap().len(), 3);
    assert!(!list.to_string().contains("ufop_pat_"), "raw tokens are never listed");
    assert_eq!(s.post("/api/v1/auth/api-tokens", &admin_tok, json!({"name": "more", "scope": "admin"})).await.status(), 401);

    // Revocation, then a JWT reset revokes the rest.
    let id = read["info"]["id"].as_str().unwrap();
    assert_eq!(s.client.delete(format!("{}/api/v1/auth/api-tokens/{id}", s.base)).bearer_auth(&tok).send().await.unwrap().status(), 200);
    assert_eq!(s.get("/api/v1/jobs", &read_tok).await.status(), 401);
    assert_eq!(s.client.delete(format!("{}/api/v1/auth/api-tokens/{id}", s.base)).bearer_auth(&tok).send().await.unwrap().status(), 404);
    assert_eq!(s.post("/api/v1/auth/reset", &tok, json!({})).await.status(), 200);
    assert_eq!(s.get("/api/v1/jobs", &run_tok).await.status(), 401);
}

#[tokio::test]
async fn security_headers_and_hsts_only_with_https() {
    let plain = start(ServerConfig::default(), None).await;
    let r = plain.client.get(format!("{}/api/v1/health", plain.base)).send().await.unwrap();
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    assert_eq!(r.headers()["x-frame-options"], "DENY");
    assert!(r.headers().get("strict-transport-security").is_none(), "no HSTS over plain HTTP");
    // The router decides from the configured mode (serve() terminates TLS in front of it).
    let tls = TlsMode::Generated { dir: plain.dir.join("tls"), hostnames: vec!["localhost".into()] };
    let https = start(ServerConfig { tls: tls.clone(), hsts_include_subdomains: true, ..Default::default() }, None).await;
    let r = https.client.get(format!("{}/api/v1/health", https.base)).send().await.unwrap();
    assert_eq!(r.headers()["strict-transport-security"], "max-age=31536000; includeSubDomains");
    // Errors carry the headers too.
    let r = https.client.get(format!("{}/api/v1/jobs", https.base)).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert!(r.headers().get("strict-transport-security").is_some());
    let off = start(ServerConfig { tls, hsts_max_age: 0, ..Default::default() }, None).await;
    assert!(off.client.get(format!("{}/api/v1/health", off.base)).send().await.unwrap().headers().get("strict-transport-security").is_none());
}

#[tokio::test]
async fn openapi_document_is_public_and_matches_enforced_scopes() {
    let s = start(ServerConfig::default(), None).await;
    let r = s.client.get(format!("{}/api/v1/openapi.json", s.base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let doc = r.json::<Value>().await.unwrap();
    assert_eq!(doc["openapi"], "3.1.0");
    let tok = s.login().await;
    let read_tok = s.post("/api/v1/auth/api-tokens", &tok, json!({"name": "reader", "scope": "read"})).await.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
    // Every operation documented as needing more than `read` refuses a read token; `read` ones admit it.
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let Some(scope) = op["x-required-scope"].as_str() else { continue };
            let url = format!("{}{}", s.base, path.replace("{id}", "nope").replace("{command}", "backup_list_jobs"));
            let req = match method.as_str() {
                "get" => s.client.get(&url),
                "post" => s.client.post(&url).json(&json!({})),
                "put" => s.client.put(&url).json(&json!({})),
                _ => s.client.delete(&url),
            };
            let status = req.bearer_auth(&read_tok).send().await.unwrap().status();
            if scope == "read" {
                assert_ne!(status, 401, "{method} {path} should admit a read token");
            } else {
                assert_eq!(status, 401, "{method} {path} needs {scope}");
            }
        }
    }
}

#[tokio::test]
async fn share_portal_links_downloads_uploads_and_passwords() {
    let off = start(ServerConfig::default(), None).await;
    let tok = off.login().await;
    let r = off.post("/api/v1/shares", &tok, json!({"name": "x", "path": "/tmp"})).await;
    assert_eq!(r.status(), 409, "sharing is off without --share-root");

    let root = tempfile::tempdir().unwrap();
    let r_path = root.path();
    std::fs::create_dir_all(r_path.join("docs/sub")).unwrap();
    std::fs::write(r_path.join("docs/report.pdf"), b"%PDF report").unwrap();
    std::fs::write(r_path.join("docs/sub/a b.txt"), b"hello").unwrap();
    std::fs::write(r_path.join("docs/.env"), b"SECRET=1").unwrap();
    std::fs::create_dir_all(r_path.join("drop")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"nope").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), r_path.join("docs/escape")).unwrap();

    let s = start(ServerConfig { share_roots: vec![r_path.to_path_buf()], ..Default::default() }, None).await;
    let tok = s.login().await;
    let anon = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).cookie_store(true).build().unwrap();
    let page = |url: String| {
        let anon = anon.clone();
        async move {
            let r = anon.get(url).send().await.unwrap();
            (r.status().as_u16(), r.headers().get("content-security-policy").is_some(), r.text().await.unwrap())
        }
    };

    // Outside the share roots, hidden paths and bad options are refused.
    assert_eq!(s.post("/api/v1/shares", &tok, json!({"name": "x", "path": outside.path()})).await.status(), 400);
    assert_eq!(s.post("/api/v1/shares", &tok, json!({"name": "x", "path": r_path.join("docs/.env")})).await.status(), 400);
    assert_eq!(s.post("/api/v1/shares", &tok, json!({"name": "x", "path": r_path.join("docs/report.pdf"), "kind": "upload"})).await.status(), 400);

    // A folder link: browse, download, nothing outside, no dot-files.
    let made = s.post("/api/v1/shares", &tok, json!({"name": "Q3 docs", "path": r_path.join("docs"), "max_downloads": 2})).await.json::<Value>().await.unwrap();
    let url = format!("{}{}", s.base, made["url"].as_str().unwrap());
    let id = made["share"]["id"].as_str().unwrap().to_string();
    assert!(made["share"].get("token").is_none());
    let (code, csp, html) = page(url.clone()).await;
    assert_eq!((code, csp), (200, true));
    assert!(html.contains("Q3 docs") && html.contains("report.pdf") && html.contains("sub") && !html.contains(".env"), "{html}");
    let (_, _, html) = page(format!("{url}?p=sub")).await;
    assert!(html.contains("a b.txt") && html.contains("dl?p=sub/a%20b.txt"), "{html}");
    let dl = anon.get(format!("{url}/dl?p=sub/a%20b.txt")).send().await.unwrap();
    assert_eq!(dl.status(), 200);
    assert!(dl.headers()["content-disposition"].to_str().unwrap().contains("filename*=UTF-8''a%20b.txt"));
    assert_eq!(dl.bytes().await.unwrap().as_ref(), b"hello");
    for bad in ["?p=..", "?p=../drop", "?p=.env", "/dl?p=.env", "/dl?p=../../etc/passwd", "/dl?p=escape/secret.txt", "?p=escape"] {
        let (code, _, html) = page(format!("{url}{bad}")).await;
        assert_eq!(code, 404, "{bad}");
        assert!(!html.contains("nope") && !html.contains("SECRET"), "{bad}");
    }
    // Download limit: two in total.
    assert_eq!(anon.get(format!("{url}/dl?p=report.pdf")).send().await.unwrap().status(), 200);
    assert_eq!(anon.get(format!("{url}/dl?p=report.pdf")).send().await.unwrap().status(), 410);
    // Download-only links refuse uploads.
    let part = reqwest::multipart::Form::new().part("file", reqwest::multipart::Part::bytes(b"x".to_vec()).file_name("x.txt"));
    assert_eq!(anon.post(format!("{url}/upload")).multipart(part).send().await.unwrap().status(), 403);

    // An upload link with a password and a size limit.
    let drop = s.post("/api/v1/shares", &tok, json!({"name": "Drop box", "path": r_path.join("drop"), "kind": "upload", "password": "open sesame", "max_upload_mb": 1})).await.json::<Value>().await.unwrap();
    let durl = format!("{}{}", s.base, drop["url"].as_str().unwrap());
    let did = drop["share"]["id"].as_str().unwrap().to_string();
    let (_, _, html) = page(durl.clone()).await;
    assert!(html.contains("protected by a password") && !html.contains("Upload files"));
    let up = |name: &'static str, body: Vec<u8>| reqwest::multipart::Form::new().part("file", reqwest::multipart::Part::bytes(body).file_name(name));
    assert_eq!(anon.post(format!("{durl}/upload")).multipart(up("early.txt", b"x".to_vec())).send().await.unwrap().status(), 401);
    let wrong = anon.post(format!("{durl}/unlock")).form(&[("password", "nope")]).send().await.unwrap();
    assert_eq!(wrong.status(), 401);
    let right = anon.post(format!("{durl}/unlock")).form(&[("password", "open sesame")]).send().await.unwrap();
    assert_eq!(right.status(), 303);
    assert!(right.headers()["set-cookie"].to_str().unwrap().contains("HttpOnly"));
    let (_, _, html) = page(durl.clone()).await;
    assert!(html.contains("Upload files") && !html.contains("<table>"), "upload links list nothing: {html}");
    let r = anon.post(format!("{durl}/upload")).multipart(up("../../evil.txt", b"one".to_vec()).part("file", reqwest::multipart::Part::bytes(b"two".to_vec()).file_name("evil.txt"))).send().await.unwrap();
    assert_eq!(r.status(), 303);
    assert!(r.headers()["location"].to_str().unwrap().contains("uploaded=2"));
    assert_eq!(std::fs::read(r_path.join("drop/evil.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(r_path.join("drop/evil (2).txt")).unwrap(), b"two", "no overwrite");
    assert!(!r_path.join("evil.txt").exists());
    let big = anon.post(format!("{durl}/upload")).multipart(up("big.bin", vec![0u8; 1024 * 1024 + 1])).send().await.unwrap();
    assert_eq!(big.status(), 413);
    assert!(std::fs::read_dir(r_path.join("drop")).unwrap().filter_map(Result::ok).all(|e| !e.file_name().to_string_lossy().starts_with(".ufop-upload")), "partial upload removed");
    assert_eq!(anon.get(format!("{durl}/dl?p=evil.txt")).send().await.unwrap().status(), 403, "upload links do not serve files");

    // Wrong passwords are throttled.
    let other = reqwest::Client::new();
    for _ in 0..10 {
        other.post(format!("{durl}/unlock")).form(&[("password", "guess")]).send().await.unwrap();
    }
    assert_eq!(other.post(format!("{durl}/unlock")).form(&[("password", "open sesame")]).send().await.unwrap().status(), 429);

    // Access log, listing and revocation.
    let ev = s.get(&format!("/api/v1/shares/{id}/events"), &tok).await.json::<Value>().await.unwrap();
    let actions: Vec<&str> = ev["events"].as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap()).collect();
    assert!(actions.contains(&"download") && actions.contains(&"view") && actions.contains(&"refused"), "{actions:?}");
    let dev = s.get(&format!("/api/v1/shares/{did}/events"), &tok).await.json::<Value>().await.unwrap();
    let dactions: Vec<&str> = dev["events"].as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap()).collect();
    assert!(dactions.contains(&"upload") && dactions.contains(&"wrong_password") && dactions.contains(&"throttled"), "{dactions:?}");
    let list = s.get("/api/v1/shares", &tok).await.json::<Value>().await.unwrap();
    assert_eq!(list["shares"].as_array().unwrap().len(), 2);
    assert_eq!(list["shares"][1]["downloads"], 2);
    assert_eq!(s.client.delete(format!("{}/api/v1/shares/{id}", s.base)).bearer_auth(&tok).send().await.unwrap().status(), 200);
    assert_eq!(page(url.clone()).await.0, 410);
    assert_eq!(page(format!("{}/s/not-a-real-token", s.base)).await.0, 404);

    // A one-file link.
    let one = s.post("/api/v1/shares", &tok, json!({"name": "Report", "path": r_path.join("docs/report.pdf")})).await.json::<Value>().await.unwrap();
    let ourl = format!("{}{}", s.base, one["url"].as_str().unwrap());
    assert!(page(ourl.clone()).await.2.contains("report.pdf"));
    assert_eq!(anon.get(format!("{ourl}/dl")).send().await.unwrap().bytes().await.unwrap().as_ref(), b"%PDF report");
    assert_eq!(anon.get(format!("{ourl}/dl?p=../sub/a%20b.txt")).send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn folder_policies_lock_filter_name_and_constrain_links() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path().canonicalize().unwrap();
    std::fs::create_dir_all(r.join("inbox/invoices")).unwrap();
    std::fs::create_dir_all(r.join("archive")).unwrap();
    let s = start(ServerConfig { share_roots: vec![r.clone()], ..Default::default() }, None).await;
    let tok = s.login().await;
    let put = |path: &std::path::Path, policy: Value| s.client.put(format!("{}/api/v1/folder-policies", s.base)).bearer_auth(&tok).json(&json!({"path": path, "policy": policy})).send();

    // The root's rules are inherited; the nearest setting wins.
    assert_eq!(put(&r, json!({"blocked_extensions": ["exe"], "max_link_days": 30})).await.unwrap().status(), 200);
    let inv = put(&r.join("inbox/invoices"), json!({"allowed_extensions": [".PDF"], "intake_naming": "sequence", "max_upload_mb": 1})).await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(inv["own"]["allowed_extensions"], json!(["pdf"]));
    assert_eq!(inv["effective"]["blocked_extensions"], json!(["exe"]));
    assert_eq!(inv["effective"]["max_link_days"], 30);
    assert_eq!(put(&r.join("archive"), json!({"locked": true, "require_link_password": true})).await.unwrap().status(), 200);
    assert_eq!(put(&std::path::PathBuf::from("/etc"), json!({"locked": true})).await.unwrap().status(), 400, "outside the share roots");
    assert_eq!(put(&r, json!({"max_upload_mb": 0})).await.unwrap().status(), 400);

    // Link rules: expiry within 30 days; locked folders take no upload links; passwords where required.
    let mk = |body: Value| s.post("/api/v1/shares", &tok, body);
    assert!(mk(json!({"name": "x", "path": r.join("inbox/invoices"), "kind": "upload"})).await.json::<Value>().await.unwrap()["message"].as_str().unwrap().contains("within 30 days"));
    assert_eq!(mk(json!({"name": "x", "path": r.join("archive"), "kind": "upload", "password": "long enough pw", "expires_in_days": 5})).await.status(), 400);
    assert!(mk(json!({"name": "x", "path": r.join("archive"), "expires_in_days": 5})).await.json::<Value>().await.unwrap()["message"].as_str().unwrap().contains("need a password"));
    assert_eq!(mk(json!({"name": "x", "path": r.join("archive"), "password": "long enough pw", "expires_in_days": 5})).await.status(), 200);
    let drop = mk(json!({"name": "Invoices", "path": r.join("inbox/invoices"), "kind": "upload", "expires_in_days": 7})).await.json::<Value>().await.unwrap();
    let durl = format!("{}{}", s.base, drop["url"].as_str().unwrap());

    // Uploads: accepted types only, numbered in arrival order, size cap from the folder.
    let anon = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let file = |name: &'static str, body: Vec<u8>| reqwest::multipart::Part::bytes(body).file_name(name);
    let up = anon
        .post(format!("{durl}/upload"))
        .multipart(reqwest::multipart::Form::new().part("file", file("a.pdf", b"1".to_vec())).part("file", file("evil.exe", b"x".to_vec())).part("file", file("notes.txt", b"t".to_vec())).part("file", file("b.PDF", b"2".to_vec())))
        .send()
        .await
        .unwrap();
    assert_eq!(up.status(), 303);
    assert!(up.headers()["location"].to_str().unwrap().contains("uploaded=2&refused=2"));
    let mut names: Vec<String> = std::fs::read_dir(r.join("inbox/invoices")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["000001-a.pdf", "000002-b.PDF"]);
    let only_bad = anon.post(format!("{durl}/upload")).multipart(reqwest::multipart::Form::new().part("file", file("x.exe", b"x".to_vec()))).send().await.unwrap();
    assert_eq!(only_bad.status(), 415);
    let big = anon.post(format!("{durl}/upload")).multipart(reqwest::multipart::Form::new().part("file", file("big.pdf", vec![0u8; 1024 * 1024 + 1]))).send().await.unwrap();
    assert_eq!(big.status(), 413);
    let page = anon.get(&durl).send().await.unwrap().text().await.unwrap();
    assert!(page.contains("Accepted: .pdf") && page.contains("1.0 MB per file"), "{page}");

    // Locking the folder stops uploads through existing links.
    assert_eq!(put(&r.join("inbox"), json!({"locked": true})).await.unwrap().status(), 200);
    assert_eq!(anon.post(format!("{durl}/upload")).multipart(reqwest::multipart::Form::new().part("file", file("c.pdf", b"3".to_vec()))).send().await.unwrap().status(), 423);
    assert!(anon.get(&durl).send().await.unwrap().text().await.unwrap().contains("does not take uploads"));

    // Copy a policy; read the effective one; an empty policy removes it.
    let cp = s.post("/api/v1/folder-policies/copy", &tok, json!({"from": r.join("inbox/invoices"), "to": r.join("archive")})).await.json::<Value>().await.unwrap();
    assert_eq!(cp["own"]["intake_naming"], "sequence");
    assert_eq!(cp["effective"]["locked"], false, "the copy replaced the archive's own lock");
    let eff = s.get(&format!("/api/v1/folder-policies/effective?path={}", r.join("inbox/invoices").display()), &tok).await.json::<Value>().await.unwrap();
    assert_eq!(eff["effective"]["locked"], true, "inherited from inbox");
    assert_eq!(eff["effective"]["sources"].as_array().unwrap().len(), 3);
    assert_eq!(put(&r.join("inbox"), json!({})).await.unwrap().json::<Value>().await.unwrap()["own"], Value::Null);
    assert_eq!(s.get("/api/v1/folder-policies", &tok).await.json::<Value>().await.unwrap()["policies"].as_array().unwrap().len(), 3);
    assert_eq!(s.post("/api/v1/folder-policies/copy", &tok, json!({"from": r.join("inbox"), "to": r.join("archive")})).await.status(), 400, "nothing to copy");
}

#[tokio::test]
async fn webdav_is_read_only_authenticated_and_confined() {
    let roots = tempfile::tempdir().unwrap();
    let docs = roots.path().join("docs");
    std::fs::create_dir_all(docs.join("sub dir")).unwrap();
    std::fs::write(docs.join("hello.txt"), b"hello webdav").unwrap();
    std::fs::write(docs.join("sub dir/a&b.txt"), b"x").unwrap();
    std::fs::write(docs.join(".secret"), b"hidden").unwrap();
    let outside = roots.path().join("outside.txt");
    std::fs::write(&outside, b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, docs.join("escape.txt")).unwrap();

    // Off unless --dav-root is given.
    let off = start(ServerConfig::default(), None).await;
    assert_eq!(off.client.get(format!("{}/dav/", off.base)).basic_auth("u", Some("server-password")).send().await.unwrap().status(), 404);

    let s = start(ServerConfig { dav_roots: vec![docs.clone()], ..Default::default() }, None).await;
    let m = |name: &str| reqwest::Method::from_bytes(name.as_bytes()).unwrap();
    let dav = |method: &str, path: &str| s.client.request(m(method), format!("{}{path}", s.base));
    let pw = |r: reqwest::RequestBuilder| r.basic_auth("anyone", Some("server-password"));

    // OPTIONS needs no credentials and advertises class 1, read-only.
    let r = dav("OPTIONS", "/dav/docs/").send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["dav"], "1");
    assert_eq!(r.headers()["allow"], "OPTIONS, GET, HEAD, PROPFIND");

    // Credentials: none or wrong → 401 with a Basic challenge.
    let r = dav("PROPFIND", "/dav/").header("Depth", "1").send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert!(r.headers()["www-authenticate"].to_str().unwrap().starts_with("Basic realm="));
    assert_eq!(dav("PROPFIND", "/dav/").basic_auth("u", Some("wrong")).header("Depth", "1").send().await.unwrap().status(), 401);

    // Roots listing.
    let r = pw(dav("PROPFIND", "/dav/")).header("Depth", "1").send().await.unwrap();
    assert_eq!(r.status(), 207);
    let body = r.text().await.unwrap();
    assert!(body.contains("<D:href>/dav/docs/</D:href>"), "{body}");

    // Folder listing: depth 1, hidden files and symlinks out of the root left out, names escaped.
    let body = pw(dav("PROPFIND", "/dav/docs")).header("Depth", "1").send().await.unwrap().text().await.unwrap();
    assert!(body.contains("<D:href>/dav/docs/hello.txt</D:href>") && body.contains("<D:getcontentlength>12</D:getcontentlength>"), "{body}");
    assert!(body.contains("<D:href>/dav/docs/sub%20dir/</D:href>") && body.contains("<D:collection/>"));
    assert!(!body.contains(".secret") && !body.contains("escape.txt"), "{body}");
    let body = pw(dav("PROPFIND", "/dav/docs/sub%20dir/")).header("Depth", "1").send().await.unwrap().text().await.unwrap();
    assert!(body.contains("/dav/docs/sub%20dir/a%26b.txt") && body.contains("<D:displayname>a&amp;b.txt</D:displayname>"), "{body}");
    // Depth 0 is only the resource itself; infinity is refused.
    let body = pw(dav("PROPFIND", "/dav/docs/")).header("Depth", "0").send().await.unwrap().text().await.unwrap();
    assert!(!body.contains("hello.txt"));
    let r = pw(dav("PROPFIND", "/dav/docs/")).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert!(r.text().await.unwrap().contains("propfind-finite-depth"));

    // GET / HEAD / ranges / conditional.
    let r = pw(dav("GET", "/dav/docs/hello.txt")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let etag = r.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(r.headers()["accept-ranges"], "bytes");
    assert_eq!(r.text().await.unwrap(), "hello webdav");
    let r = pw(dav("GET", "/dav/docs/hello.txt")).header("Range", "bytes=6-").send().await.unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.headers()["content-range"], "bytes 6-11/12");
    assert_eq!(r.text().await.unwrap(), "webdav");
    assert_eq!(pw(dav("GET", "/dav/docs/hello.txt")).header("Range", "bytes=50-").send().await.unwrap().status(), 416);
    assert_eq!(pw(dav("GET", "/dav/docs/hello.txt")).header("If-None-Match", &etag).send().await.unwrap().status(), 304);
    let r = pw(dav("HEAD", "/dav/docs/hello.txt")).send().await.unwrap();
    assert_eq!((r.status().as_u16(), r.headers()["content-length"].to_str().unwrap()), (200, "12"));
    let r = pw(dav("GET", "/dav/docs/")).send().await.unwrap();
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    assert!(r.text().await.unwrap().contains("href=\"/dav/docs/hello.txt\""));

    // Confinement: hidden files, traversal, symlinks out of the root, unknown roots.
    for p in ["/dav/docs/.secret", "/dav/docs/%2e%2e/outside.txt", "/dav/docs/escape.txt", "/dav/nope/hello.txt", "/dav/docs/missing"] {
        assert_eq!(pw(dav("GET", p)).send().await.unwrap().status(), 404, "{p}");
    }

    // Read-only: every change is refused, authenticated or not, and nothing changes.
    for method in ["PUT", "DELETE", "MKCOL", "MOVE", "COPY", "PROPPATCH", "LOCK", "UNLOCK", "POST"] {
        let r = pw(dav(method, "/dav/docs/hello.txt")).body("changed").send().await.unwrap();
        assert_eq!(r.status(), 405, "{method}");
        assert_eq!(r.headers()["allow"], "OPTIONS, GET, HEAD, PROPFIND");
    }
    assert_eq!(std::fs::read(docs.join("hello.txt")).unwrap(), b"hello webdav");

    // API tokens (any scope) as the Basic password or a bearer; revocation applies at once.
    let tok = s.login().await;
    let minted = s.post("/api/v1/auth/api-tokens", &tok, json!({"name": "dav", "scope": "read", "expires_in_days": 1})).await.json::<Value>().await.unwrap();
    let pat = minted["token"].as_str().unwrap().to_string();
    assert_eq!(dav("GET", "/dav/docs/hello.txt").basic_auth("x", Some(&pat)).send().await.unwrap().status(), 200);
    assert_eq!(dav("GET", "/dav/docs/hello.txt").bearer_auth(&tok).send().await.unwrap().status(), 200);
    let id = minted["info"]["id"].as_str().unwrap();
    s.client.delete(format!("{}/api/v1/auth/api-tokens/{id}", s.base)).bearer_auth(&tok).send().await.unwrap();
    assert_eq!(dav("GET", "/dav/docs/hello.txt").basic_auth("x", Some(&pat)).send().await.unwrap().status(), 401);

    // A password change forgets the remembered password immediately.
    let r = s.client.put(format!("{}/api/v1/auth/password", s.base)).bearer_auth(&tok).json(&json!({"current_password": "server-password", "new_password": "another-password"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(pw(dav("GET", "/dav/docs/hello.txt")).send().await.unwrap().status(), 401);
    assert_eq!(dav("GET", "/dav/docs/hello.txt").basic_auth("u", Some("another-password")).send().await.unwrap().status(), 200);

    // --dav-allowed-cidrs narrows who may use WebDAV.
    let narrowed = start(
        ServerConfig { dav_roots: vec![docs.clone()], dav_allowed_clients: ufop_core::netacl::IpAllowlist::parse(&["10.0.0.0/8".to_string()]).unwrap(), ..Default::default() },
        None,
    )
    .await;
    assert_eq!(narrowed.client.get(format!("{}/dav/docs/hello.txt", narrowed.base)).basic_auth("u", Some("server-password")).send().await.unwrap().status(), 403);
    assert_eq!(narrowed.client.get(format!("{}/api/v1/health", narrowed.base)).send().await.unwrap().status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_gateway_relays_through_an_ha_pool_without_storing() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use ufop_core::gateway::{run, AgentConfig};
    let s = start(ServerConfig::default(), None).await;
    let tok = s.login().await;
    let mint = |scope: &'static str| s.post("/api/v1/auth/api-tokens", &tok, json!({"name": format!("gw {scope}"), "scope": scope, "expires_in_days": 1}));
    let write_tok = mint("write").await.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
    let read_tok = mint("read").await.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();

    let files = tempfile::tempdir().unwrap();
    let docs = files.path().join("docs");
    std::fs::create_dir_all(docs.join("sub")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    std::fs::write(docs.join("big.bin"), &big).unwrap();
    std::fs::write(docs.join(".secret"), b"x").unwrap();
    std::fs::write(files.path().join("outside.txt"), b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(files.path().join("outside.txt"), docs.join("escape.txt")).unwrap();

    let gw = |path: &str| format!("{}/api/v1/gateways/office{path}", s.base);
    // Nothing connected yet.
    assert_eq!(s.client.get(gw("/files?path=/")).bearer_auth(&read_tok).send().await.unwrap().status(), 404);

    let agent = |token: String, writable: bool| {
        let stop = Arc::new(AtomicBool::new(false));
        let cfg = AgentConfig { server: s.base.clone(), token, gateway: "office".into(), roots: vec![docs.clone()], writable, max_upload: Some(10 << 20), concurrency: 4 };
        let h = tokio::spawn(run(cfg, stop.clone()));
        (stop, h)
    };
    // An agent needs the write scope.
    let (_, h) = agent(read_tok.clone(), false);
    assert!(h.await.unwrap().unwrap_err().contains("refused the token"));

    let (stop_a, a) = agent(write_tok.clone(), true);
    let (stop_b, b) = agent(write_tok.clone(), false);
    let mut online = 0;
    for _ in 0..100 {
        let v = s.get("/api/v1/gateways", &read_tok).await.json::<Value>().await.unwrap();
        online = v["gateways"][0]["online_nodes"].as_u64().unwrap_or(0);
        if online == 2 {
            assert_eq!(v["gateways"][0]["name"], "office");
            assert_eq!(v["gateways"][0]["writable"], true);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(online, 2, "both nodes registered");

    // Listing: roots, then a folder without dot-files or links out of the root.
    let roots = s.client.get(gw("/files?path=/")).bearer_auth(&read_tok).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(roots["entries"][0]["name"], "docs");
    let l = s.client.get(gw("/files?path=/docs")).bearer_auth(&read_tok).send().await.unwrap().json::<Value>().await.unwrap();
    let names: Vec<&str> = l["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["sub", "big.bin"]);

    // Downloads stream through, several at once, served by the pool.
    let mut handles = Vec::new();
    for _ in 0..6 {
        let (c, url, t) = (s.client.clone(), gw("/download?path=/docs/big.bin"), read_tok.clone());
        handles.push(tokio::spawn(async move {
            let r = c.get(url).bearer_auth(t).send().await.unwrap();
            assert_eq!(r.status(), 200);
            assert_eq!(r.headers()["content-length"], "3000000");
            r.bytes().await.unwrap().to_vec()
        }));
    }
    for h in handles {
        assert!(h.await.unwrap() == big, "relayed bytes are identical");
    }
    for (p, code) in [("/docs/missing.bin", 404), ("/docs/.secret", 400), ("/docs/../outside.txt", 400), ("/docs/escape.txt", 404), ("/nope/x", 404)] {
        assert_eq!(s.client.get(gw(&format!("/download?path={p}"))).bearer_auth(&read_tok).send().await.unwrap().status(), code, "{p}");
    }

    // Uploads need the write scope and a writable node; written via a temp name.
    assert_eq!(s.client.put(gw("/upload?path=/docs/sub/new.txt")).bearer_auth(&read_tok).body("hello").send().await.unwrap().status(), 401);
    let r = s.client.put(gw("/upload?path=/docs/sub/new.txt")).bearer_auth(&write_tok).body("hello gateway").send().await.unwrap();
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    assert_eq!(std::fs::read(docs.join("sub/new.txt")).unwrap(), b"hello gateway");
    assert!(!docs.join("sub/.new.txt.ufop-part").exists());
    assert_eq!(s.client.put(gw("/upload?path=/docs/.hidden")).bearer_auth(&write_tok).body("x").send().await.unwrap().status(), 400);
    assert_eq!(s.client.put(gw("/upload?path=/docs/huge.bin")).bearer_auth(&write_tok).body(vec![0u8; 11 << 20]).send().await.unwrap().status(), 403);
    assert!(!docs.join("huge.bin").exists() && !docs.join(".huge.bin.ufop-part").exists());

    // Failover: with the writable node gone, reads continue on the other node
    // and uploads are refused straight away.
    stop_a.store(true, Ordering::SeqCst);
    a.await.unwrap().unwrap();
    let v = s.get("/api/v1/gateways", &read_tok).await.json::<Value>().await.unwrap();
    assert_eq!(v["gateways"][0]["online_nodes"], 1);
    let r = s.client.get(gw("/download?path=/docs/big.bin")).bearer_auth(&read_tok).send().await.unwrap();
    assert_eq!(r.bytes().await.unwrap().len(), big.len());
    assert_eq!(s.client.put(gw("/upload?path=/docs/x.txt")).bearer_auth(&write_tok).body("x").send().await.unwrap().status(), 403);

    stop_b.store(true, Ordering::SeqCst);
    b.await.unwrap().unwrap();
    assert_eq!(s.client.get(gw("/files?path=/")).bearer_auth(&read_tok).send().await.unwrap().status(), 404, "the empty gateway is gone");
}

/// The generated clients, run for real against a server: Python (standard
/// library) and TypeScript (Node's type stripping), when installed.
#[tokio::test]
async fn generated_clients_work_against_a_live_server() {
    let s = start(ServerConfig::default(), None).await;
    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sdk");
    // The server hands out the same clients.
    let served = s.client.get(format!("{}/api/v1/sdk/python", s.base)).send().await.unwrap();
    assert_eq!(served.status(), 200);
    assert_eq!(served.text().await.unwrap(), sdk::python(&openapi::document()));
    assert_eq!(s.client.get(format!("{}/api/v1/sdk/cobol", s.base)).send().await.unwrap().status(), 404);

    let run = |program: &str, args: Vec<String>| {
        let out = std::process::Command::new(program).args(&args).output();
        out.ok().map(|o| (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string()))
    };
    let py = format!(
        r#"import json, sys
sys.path.insert(0, {dir:?})
from ufop_client import UfopClient, UfopError
c = UfopClient({base:?})
try:
    c.get_jobs()
    raise SystemExit("unauthenticated call succeeded")
except UfopError as e:
    assert e.status == 401, e.status
c.login("server-password")
print(json.dumps({{"status": c.get_health()["status"], "jobs": c.get_jobs(), "tokens": len(c.get_auth_api_tokens()["tokens"]), "paused": c.post_scheduler_pause({{"minutes": 5}})["paused"]}}))
"#,
        dir = sdk.join("python").to_string_lossy(),
        base = s.base
    );
    if let Some((ok, out, err)) = tokio::task::spawn_blocking(move || run("python3", vec!["-c".into(), py])).await.unwrap() {
        assert!(ok, "python client failed: {err}");
        let v: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v, json!({ "status": "ok", "jobs": [], "tokens": 0, "paused": true }));
    }
    let script = s.dir.join("client.mts");
    std::fs::write(
        &script,
        format!(
            r#"import {{ UfopClient, UfopError }} from {index:?};
const c = new UfopClient({base:?});
try {{ await c.getJobs(); throw new Error("unauthenticated call succeeded"); }} catch (e) {{ if (!(e instanceof UfopError) || e.status !== 401) throw e; }}
await c.login("server-password");
const health = await c.getHealth();
const resumed = await c.postSchedulerResume();
console.log(JSON.stringify({{ status: health.status, jobs: await c.getJobs(), paused: resumed.paused }}));
"#,
            index = sdk.join("typescript/src/index.ts").to_string_lossy(),
            base = s.base
        ),
    )
    .unwrap();
    let args = vec!["--experimental-strip-types".into(), "--no-warnings".into(), script.to_string_lossy().into_owned()];
    if let Some((ok, out, err)) = tokio::task::spawn_blocking(move || run("node", args)).await.unwrap() {
        assert!(ok, "typescript client failed: {err}");
        let v: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v, json!({ "status": "ok", "jobs": [], "paused": false }));
    }
}

/// The MCP authorization flow end to end: discovery from a 401, dynamic
/// registration, consent with PKCE, tokens, tool calls within the granted
/// scope, refresh rotation and revocation.
#[tokio::test]
async fn mcp_oauth_flow_and_scoped_tools() {
    use base64::Engine;
    use sha2::Digest;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("notes.txt"), "hello mcp").unwrap();
    let s = start(ServerConfig { mcp_roots: vec![root.path().to_path_buf()], mcp_allow_write: true, ..Default::default() }, None).await;
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let rpc = |method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let mcp = |token: Option<String>, body: Value| {
        let mut r = http.post(format!("{}/mcp", s.base)).json(&body);
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        r.send()
    };

    // 1. Discovery starts from the 401.
    let denied = mcp(None, rpc("initialize", json!({}))).await.unwrap();
    assert_eq!(denied.status(), 401);
    let challenge = denied.headers()["www-authenticate"].to_str().unwrap().to_string();
    let meta_url = challenge.split("resource_metadata=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    assert_eq!(meta_url, format!("{}/.well-known/oauth-protected-resource", s.base));
    let prm: Value = http.get(&meta_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(prm["resource"], format!("{}/mcp", s.base));
    let asm: Value = http.get(format!("{}/.well-known/oauth-authorization-server", prm["authorization_servers"][0].as_str().unwrap())).send().await.unwrap().json().await.unwrap();
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));

    // 2. Registration: https or loopback redirects only.
    let reg = |uris: Value| http.post(asm["registration_endpoint"].as_str().unwrap()).json(&json!({ "client_name": "Test client", "redirect_uris": uris })).send();
    assert_eq!(reg(json!(["http://evil.example.com/cb"])).await.unwrap().status(), 400);
    let client: Value = reg(json!(["http://127.0.0.1:9/cb"])).await.unwrap().json().await.unwrap();
    let client_id = client["client_id"].as_str().unwrap().to_string();

    // 3. Consent with PKCE.
    let verifier = "v".repeat(10) + &uuid::Uuid::new_v4().simple().to_string() + &uuid::Uuid::new_v4().simple().to_string();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let auth_url = asm["authorization_endpoint"].as_str().unwrap().to_string();
    let form = |scope: &str, password: &str, decision: &str| {
        vec![
            ("response_type", "code".to_string()),
            ("client_id", client_id.clone()),
            ("redirect_uri", "http://127.0.0.1:9/cb".to_string()),
            ("code_challenge", challenge.clone()),
            ("code_challenge_method", "S256".to_string()),
            ("state", "xyz".to_string()),
            ("scope", scope.to_string()),
            ("resource", format!("{}/mcp", s.base)),
            ("password", password.to_string()),
            ("decision", decision.to_string()),
        ]
    };
    let page = http.get(&auth_url).query(&form("mcp:read mcp:write", "", "")[..8]).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains("Allow <b>Test client</b>"));
    let mut bad_redirect = form("mcp:read", "", "")[..8].to_vec();
    bad_redirect[2].1 = "http://127.0.0.1:9/other".into();
    assert_eq!(http.get(&auth_url).query(&bad_redirect).send().await.unwrap().status(), 400, "unregistered redirect: no redirect at all");
    assert_eq!(http.post(&auth_url).form(&form("mcp:read", "wrong", "allow")).send().await.unwrap().status(), 401);
    let denied = http.post(&auth_url).form(&form("mcp:read", "", "deny")).send().await.unwrap();
    assert!(denied.headers()["location"].to_str().unwrap().contains("error=access_denied"));
    let code_for = |scope: &'static str| {
        let (http, auth_url, f) = (http.clone(), auth_url.clone(), form(scope, "server-password", "allow"));
        async move {
            let r = http.post(&auth_url).form(&f).send().await.unwrap();
            assert_eq!(r.status(), 303);
            let loc = url::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
            let q: std::collections::HashMap<String, String> = loc.query_pairs().into_owned().collect();
            assert_eq!(q["state"], "xyz");
            q["code"].clone()
        }
    };

    // 4. Tokens: a wrong verifier burns the code; the right one gets a pair.
    let token_url = asm["token_endpoint"].as_str().unwrap().to_string();
    let exchange = |code: String, v: String| http.post(&token_url).form(&[("grant_type", "authorization_code".to_string()), ("code", code), ("redirect_uri", "http://127.0.0.1:9/cb".into()), ("client_id", client_id.clone()), ("code_verifier", v)]).send();
    let code = code_for("mcp:read").await;
    assert_eq!(exchange(code.clone(), "x".repeat(50)).await.unwrap().status(), 400);
    assert_eq!(exchange(code, verifier.clone()).await.unwrap().status(), 400, "single use");
    let read: Value = exchange(code_for("mcp:read").await, verifier.clone()).await.unwrap().json().await.unwrap();
    assert_eq!(read["scope"], "mcp:read");
    let read_token = read["access_token"].as_str().unwrap().to_string();

    // 5. Read scope: read tools only.
    let init: Value = mcp(Some(read_token.clone()), rpc("initialize", json!({ "protocolVersion": "2025-06-18" }))).await.unwrap().json().await.unwrap();
    assert!(init["result"]["instructions"].as_str().unwrap().contains("read-only"));
    let names = |v: Value| v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let tools = names(mcp(Some(read_token.clone()), rpc("tools/list", json!({}))).await.unwrap().json().await.unwrap());
    assert!(tools.contains(&"read_text_file".to_string()) && !tools.contains(&"write_file".to_string()), "{tools:?}");
    let text: Value = mcp(Some(read_token.clone()), rpc("tools/call", json!({ "name": "read_text_file", "arguments": { "path": "notes.txt" } }))).await.unwrap().json().await.unwrap();
    assert_eq!(text["result"]["structuredContent"]["text"], "hello mcp");
    assert_eq!(mcp(Some(read_token.clone()), json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await.unwrap().status(), 202);
    let cross = http.post(format!("{}/mcp", s.base)).bearer_auth(&read_token).header("Origin", "http://evil.example.com").json(&rpc("ping", json!({}))).send().await.unwrap();
    assert_eq!(cross.status(), 403, "other browser origins are refused");

    // 6. Write scope: writes work, confined to the root.
    let rw: Value = exchange(code_for("mcp:read mcp:write").await, verifier.clone()).await.unwrap().json().await.unwrap();
    let rw_token = rw["access_token"].as_str().unwrap().to_string();
    let wrote: Value = mcp(Some(rw_token.clone()), rpc("tools/call", json!({ "name": "write_file", "arguments": { "path": "out/new.txt", "content": "via mcp" } }))).await.unwrap().json().await.unwrap();
    assert_eq!(wrote["result"]["isError"], false, "{wrote}");
    assert_eq!(std::fs::read_to_string(root.path().join("out/new.txt")).unwrap(), "via mcp");
    let escape: Value = mcp(Some(rw_token.clone()), rpc("tools/call", json!({ "name": "write_file", "arguments": { "path": "../escape.txt", "content": "x" } }))).await.unwrap().json().await.unwrap();
    assert_eq!(escape["result"]["isError"], true);

    // 7. Refresh rotates: the old access and refresh tokens stop working.
    let refresh = |rt: String| http.post(&token_url).form(&[("grant_type", "refresh_token".to_string()), ("refresh_token", rt), ("client_id", client_id.clone())]).send();
    let rotated: Value = refresh(rw["refresh_token"].as_str().unwrap().into()).await.unwrap().json().await.unwrap();
    assert_eq!(rotated["scope"], "mcp:read mcp:write");
    assert_eq!(mcp(Some(rw_token), rpc("ping", json!({}))).await.unwrap().status(), 401);
    assert_eq!(refresh(rw["refresh_token"].as_str().unwrap().into()).await.unwrap().status(), 400, "refresh tokens are single use");
    let new_token = rotated["access_token"].as_str().unwrap().to_string();
    assert_eq!(mcp(Some(new_token.clone()), rpc("ping", json!({}))).await.unwrap().status(), 200);

    // 8. The grants are ordinary API tokens; revocation ends them.
    let tok = s.login().await;
    let listed: Value = s.get("/api/v1/auth/api-tokens", &tok).await.json().await.unwrap();
    assert!(listed["tokens"].as_array().unwrap().iter().any(|t| t["name"] == "MCP: Test client"));
    http.post(asm["revocation_endpoint"].as_str().unwrap()).form(&[("token", rotated["refresh_token"].as_str().unwrap())]).send().await.unwrap();
    assert_eq!(mcp(Some(new_token), rpc("ping", json!({}))).await.unwrap().status(), 401);
    // A session token works too (the server password holder).
    assert_eq!(mcp(Some(tok), rpc("ping", json!({}))).await.unwrap().status(), 200);
}

#[tokio::test]
async fn mcp_is_off_without_roots() {
    let s = start(ServerConfig::default(), None).await;
    assert_eq!(s.client.post(format!("{}/mcp", s.base)).json(&json!({})).send().await.unwrap().status(), 404);
    assert_eq!(s.client.get(format!("{}/.well-known/oauth-authorization-server", s.base)).send().await.unwrap().status(), 404);
    assert_eq!(s.client.post(format!("{}/oauth2/register", s.base)).json(&json!({ "redirect_uris": ["https://a.example.com/cb"] })).send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn policy_templates_effective_settings_and_schedule_preview_over_rest() {
    let s = start(ServerConfig::default(), None).await;
    let tok = s.login().await;
    let t = s.post("/api/v1/backup-templates", &tok, json!({"name": "Servers", "settings": {"retention": {"type": "keep_count", "count": 4}, "options": {"compression_level": 2}}})).await;
    assert_eq!(t.status(), 200, "{}", t.text().await.unwrap());
    let t = t.json::<Value>().await.unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    assert_eq!(s.post("/api/v1/backup-templates", &tok, json!({"name": "Bad", "settings": {"options": {"block_size": 1}}})).await.status(), 400);
    let src = s.dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let job = json!({"name":"Srv","sources":[src],"encrypted":false,"destination":{"type":"local","path": s.dir.join("dst"),"alternate_paths":[],"marker_file":null}});
    let id = s.post("/api/v1/jobs", &tok, json!({"job": job})).await.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    let j = s.post(&format!("/api/v1/backup-templates/{tid}/apply"), &tok, json!({"job_id": id})).await.json::<Value>().await.unwrap();
    assert_eq!(j["template_ids"], json!([tid]));
    let eff = s.get(&format!("/api/v1/jobs/{id}/effective-settings"), &tok).await.json::<Value>().await.unwrap();
    let ret = eff["settings"].as_array().unwrap().iter().find(|x| x["field_path"] == "retention").unwrap().clone();
    assert_eq!(ret["source"]["type"], "template");
    assert_eq!(ret["value"]["count"], 4);
    // In use: refused; listing and export.
    assert_eq!(s.client.delete(format!("{}/api/v1/backup-templates/{tid}", s.base)).bearer_auth(&tok).send().await.unwrap().status(), 409);
    assert_eq!(s.get("/api/v1/backup-templates", &tok).await.json::<Value>().await.unwrap().as_array().unwrap().len(), 1);
    let exported = s.get("/api/v1/backup-templates/export", &tok).await.text().await.unwrap();
    assert!(exported.contains("ufop-backup-templates"));
    let imported = s.client.post(format!("{}/api/v1/backup-templates/import", s.base)).bearer_auth(&tok).body(exported).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(imported[0]["id"], tid.as_str());
    // Organization templates are read-only here.
    let org = ufop_core::agent::ManagedBackupTemplate { id: "std".into(), name: "Org".into(), settings: json!({}), ..Default::default() };
    assert!(s.mgr.set_managed_templates(Some(&[org])).is_empty());
    assert_eq!(s.client.delete(format!("{}/api/v1/backup-templates/org:std", s.base)).bearer_auth(&tok).send().await.unwrap().status(), 403);
    // Cron schedules: preview and a friendly error.
    let p = s.post("/api/v1/backup-schedule/preview", &tok, json!({"schedule": {"enabled": true, "cron": ["30 2 * * *"]}, "count": 3})).await.json::<Value>().await.unwrap();
    assert_eq!(p.as_array().unwrap().len(), 3);
    let bad = s.post("/api/v1/backup-schedule/preview", &tok, json!({"schedule": {"enabled": true, "cron": ["99 * * * *"]}})).await;
    assert_eq!(bad.status(), 400);
    assert!(bad.text().await.unwrap().contains("Invalid cron expression"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_shutdown_checkpoints_running_backup_and_refuses_new_work() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let mgr = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, dir.path().join("dbs"), KdfParams::insecure_fast());
    let auth = AuthService::new(settings::SettingsStore::plain(repo.pool().clone()));
    auth.set_password("server-password").await.unwrap();
    let st = ServerState::new(mgr.clone(), None, auth, ServerConfig::default());
    let drain = tokio::spawn(lifecycle::drain_when_requested(st.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, router(st).into_make_service_with_connect_info::<SocketAddr>()).await.unwrap() });
    let jar = Arc::new(reqwest::cookie::Jar::default());
    let s = TestServer { base, mgr, client: reqwest::Client::builder().cookie_provider(jar.clone()).build().unwrap(), jar, dir: dir.path().to_path_buf(), _dir: dir };
    let tok = s.login().await;

    // A slow backup: incompressible files through a 40 kB/s upload limit.
    let src = s.dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..24 {
        let data: Vec<u8> = (0..16_384u32).map(|j| ((j.wrapping_mul(2_654_435_761) ^ (i * 7919)) >> 7) as u8).collect();
        std::fs::write(src.join(format!("f{i:02}.bin")), data).unwrap();
    }
    let dst = s.dir.join("dst");
    let job = json!({"name":"Slow","sources":[src],"encrypted":false,"destination":{"type":"local","path": dst,"alternate_paths":[],"marker_file":null},"options":{"block_size":4096,"volume_size":32768,"compression_level":0},"throttle":{"upload_bps":40000,"download_bps":0}});
    let id = s.post("/api/v1/jobs", &tok, json!({"job": job})).await.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(s.post(&format!("/api/v1/jobs/{id}/run"), &tok, json!({})).await.status(), 200);
    for _ in 0..200 {
        let t = s.get("/api/v1/tasks", &tok).await.json::<Value>().await.unwrap();
        if t[0]["state"] == "running" && t[0]["progress"].is_object() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let r = s.post("/api/v1/server/shutdown", &tok, json!({"grace_secs": 30})).await;
    assert_eq!(r.status(), 202);
    let r = r.json::<Value>().await.unwrap();
    assert_eq!((r["immediate"].as_bool(), r["grace_secs"].as_u64(), r["running_tasks"].as_u64()), (Some(false), Some(30), Some(1)));
    // New mutating requests are refused with Retry-After; reads still work.
    let refused = s.post(&format!("/api/v1/jobs/{id}/run"), &tok, json!({})).await;
    assert_eq!(refused.status(), 503);
    assert!(refused.headers()["retry-after"].to_str().unwrap().parse::<u64>().unwrap() >= 30);
    assert_eq!(s.get("/api/v1/jobs", &tok).await.status(), 200);
    assert!(s.mgr.start_backup(&id, false).await.unwrap_err().user_message().contains("shutting down"));

    let report = tokio::time::timeout(std::time::Duration::from_secs(40), drain).await.unwrap().unwrap();
    assert_eq!((report.cancelled, report.still_running), (1, 0), "{report:?}");
    // The stopped backup left a partial version and no repository lock.
    let versions = s.mgr.list_versions(&id).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert!(versions[0].partial, "{versions:?}");
    assert!(s.mgr.lock_status(&id).await.unwrap().locks.is_empty());
    let run = &s.mgr.run_history(Some(id.clone()), 1).await.unwrap()[0];
    assert_ne!(run.status, "success", "{run:?}");
}

#[test]
fn second_stop_request_is_immediate() {
    let l = lifecycle::Lifecycle::new(std::time::Duration::from_secs(60));
    assert!(!l.is_draining());
    assert_eq!(l.request(None), lifecycle::StopRequest::Graceful);
    assert_eq!(l.grace().as_secs(), 60);
    assert!(l.is_draining() && !l.is_immediate());
    assert_eq!(l.request(Some(std::time::Duration::from_secs(1))), lifecycle::StopRequest::Immediate);
    assert!(l.is_immediate());
    assert_eq!(l.grace().as_secs(), 60, "the first request's grace stays");
}

#[tokio::test]
async fn csrf_double_submit_on_cookie_routes() {
    let cfg = ServerConfig { cors_origins: vec!["https://console.example".into()], ..Default::default() };
    let s = start(cfg, None).await;
    let r = s.client.post(format!("{}/api/v1/auth/login", s.base)).json(&json!({"password":"server-password"})).send().await.unwrap();
    let body: Value = r.json().await.unwrap();
    let csrf = s.csrf();
    assert!(!csrf.is_empty() && body["csrf_token"] == csrf.as_str(), "the cookie and the body carry the token");
    let refresh = |hdr: Option<&str>, origin: Option<&str>| {
        let mut rq = s.client.post(format!("{}/api/v1/auth/refresh", s.base));
        if let Some(h) = hdr {
            rq = rq.header("X-CSRF-Token", h);
        }
        if let Some(o) = origin {
            rq = rq.header("Origin", o);
        }
        rq.send()
    };
    // Missing or mismatched token: 403, and the session is untouched.
    let missing = refresh(None, None).await.unwrap();
    assert_eq!(missing.status(), 403);
    assert_eq!(missing.json::<Value>().await.unwrap()["error_type"], "csrf");
    assert_eq!(refresh(Some("forged"), None).await.unwrap().status(), 403);
    // A cross-site Origin is refused even with the right token; allowed origins pass.
    assert_eq!(refresh(Some(&csrf), Some("https://evil.example")).await.unwrap().status(), 403);
    assert_eq!(refresh(Some(&csrf), Some("null")).await.unwrap().status(), 403);
    let ok = refresh(Some(&csrf), Some(&s.base)).await.unwrap();
    assert_eq!(ok.status(), 200);
    // Rotated with the refresh token: the old value no longer works.
    let rotated = ok.json::<Value>().await.unwrap()["csrf_token"].as_str().unwrap().to_string();
    assert_ne!(rotated, csrf);
    assert_eq!(s.csrf(), rotated);
    assert_eq!(refresh(Some(&csrf), None).await.unwrap().status(), 403);
    let tok = refresh(Some(&rotated), Some("https://console.example")).await.unwrap().json::<Value>().await.unwrap()["access_token"].as_str().unwrap().to_string();
    // A cookie value copied into the header must also match the session's HMAC.
    let refresh_cookie = {
        use reqwest::cookie::CookieStore;
        let all = s.jar.cookies(&format!("{}/api/v1/auth/refresh", s.base).parse().unwrap()).unwrap();
        all.to_str().unwrap().split("; ").find(|c| c.starts_with("ufop_refresh=")).unwrap().to_string()
    };
    let bare = reqwest::Client::new();
    let jar_forged = bare.post(format!("{}/api/v1/auth/refresh", s.base)).header("X-CSRF-Token", "AAAA").header("Cookie", format!("{refresh_cookie}; ufop_csrf=AAAA")).send().await.unwrap();
    assert_eq!(jar_forged.status(), 403);
    // Bearer-authenticated requests are exempt (the cookie is sent along on /auth/*).
    let mint = s.post("/api/v1/auth/signin-token", &tok, json!({})).await;
    assert_eq!(mint.status(), 200);
    // Sign-in routes only get the origin check.
    let cross_login = s.client.post(format!("{}/api/v1/auth/login", s.base)).header("Origin", "https://evil.example").json(&json!({"password":"server-password"})).send().await.unwrap();
    assert_eq!(cross_login.status(), 403);
    // Logout with the cookie needs the token too.
    assert_eq!(s.client.post(format!("{}/api/v1/auth/logout", s.base)).send().await.unwrap().status(), 403);
    let csrf = s.csrf();
    assert_eq!(s.client.post(format!("{}/api/v1/auth/logout", s.base)).header("X-CSRF-Token", &csrf).send().await.unwrap().status(), 200);
    assert!(s.csrf().is_empty(), "logout clears the cookie");
}

#[tokio::test]
async fn request_ids_are_echoed_generated_and_put_in_errors() {
    let s = start(ServerConfig::default(), None).await;
    let ok = s.client.get(format!("{}/api/v1/health", s.base)).header("X-Request-Id", "client-req.42").send().await.unwrap();
    assert_eq!(ok.headers()["x-request-id"], "client-req.42");
    // Unsafe or missing ids are replaced with a UUIDv4.
    let gen = s.client.get(format!("{}/api/v1/health", s.base)).header("X-Request-Id", "bad id <x>").send().await.unwrap();
    let id = gen.headers()["x-request-id"].to_str().unwrap().to_string();
    assert!(uuid::Uuid::parse_str(&id).is_ok_and(|u| u.get_version_num() == 4), "{id}");
    let none = s.client.get(format!("{}/api/v1/health", s.base)).send().await.unwrap();
    assert_ne!(none.headers()["x-request-id"].to_str().unwrap(), id);
    // JSON error bodies carry the id.
    let err = s.client.get(format!("{}/api/v1/jobs", s.base)).header("X-Request-Id", "trace-me").send().await.unwrap();
    assert_eq!(err.status(), 401);
    assert_eq!(err.headers()["x-request-id"], "trace-me");
    let body: Value = err.json().await.unwrap();
    assert_eq!((body["request_id"].as_str(), body["error_type"].as_str()), (Some("trace-me"), Some("permission_denied")));
    let missing = s.get("/api/v1/jobs/nope", &s.login().await).await;
    let body: Value = missing.json().await.unwrap();
    assert!(body["request_id"].is_string() && body["message"].is_string(), "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_metrics_parse_and_cover_jobs_http_and_cache() {
    use super::metrics::parse::parse;
    let s = start(ServerConfig::default(), None).await;
    assert_eq!(s.client.get(format!("{}/api/v1/metrics", s.base)).send().await.unwrap().status(), 401, "needs a sign-in or read token");
    let tok = s.login().await;
    let src = s.dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"metrics").unwrap();
    let job = json!({"name":"Metered \"job\"","sources":[src],"encrypted":false,"destination":{"type":"local","path": s.dir.join("dst"),"alternate_paths":[],"marker_file":null},"schedule":{"enabled":true,"interval_minutes":60}});
    let id = s.post("/api/v1/jobs", &tok, json!({"job": job})).await.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    let r = s.mgr.execute(&id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert!(matches!(r.status.as_str(), "success" | "warning"), "{r:?}");
    let res = s.get("/api/v1/metrics", &tok).await;
    assert_eq!(res.status(), 200);
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/plain; version=0.0.4"));
    let text = res.text().await.unwrap();
    let (types, samples) = parse(&text);
    let find = |name: &str, l: &[(&str, &str)]| samples.iter().find(|x| x.name == name && l.iter().all(|(k, v)| x.labels.get(*k).map(String::as_str) == Some(*v))).map(|x| x.value);
    assert_eq!(types["ufop_http_request_duration_seconds"], "histogram");
    assert_eq!(types["ufop_backup_runs_total"], "counter");
    assert_eq!(find("ufop_build_info", &[("version", env!("CARGO_PKG_VERSION"))]), Some(1.0));
    assert_eq!(find("ufop_backup_job_info", &[("job_id", &id), ("job_name", "Metered \"job\"")]), Some(1.0));
    assert_eq!(find("ufop_backup_runs_total", &[("job_id", &id), ("operation", "backup"), ("result", &r.status)]), Some(1.0));
    assert!(find("ufop_backup_last_success_timestamp_seconds", &[("job_id", &id)]).unwrap() > 1.7e9);
    assert!(find("ufop_backup_last_uploaded_bytes", &[("job_id", &id)]).unwrap() > 0.0);
    assert_eq!(find("ufop_backup_last_files", &[("job_id", &id)]), Some(1.0));
    assert!(find("ufop_backup_last_duration_seconds", &[("job_id", &id), ("result", &r.status)]).is_some());
    assert_eq!(find("ufop_repository_lock_held", &[("job_id", &id)]), Some(0.0));
    assert_eq!(find("ufop_operations_queued", &[]), Some(0.0));
    assert!(find("ufop_scheduler_next_run_timestamp_seconds", &[("job_id", &id), ("operation", "backup")]).unwrap() > 1.7e9);
    assert!(find("ufop_volume_cache_hits_total", &[]).is_some());
    // HTTP series use the route template, and histograms are cumulative.
    let route = "/api/v1/jobs";
    assert!(find("ufop_http_requests_total", &[("method", "POST"), ("route", route), ("status", "200")]).unwrap() >= 1.0);
    assert!(find("ufop_http_requests_total", &[("method", "GET"), ("route", "/api/v1/metrics"), ("status", "401")]).is_some());
    let inf = find("ufop_http_request_duration_seconds_bucket", &[("method", "POST"), ("route", route), ("status", "200"), ("le", "+Inf")]).unwrap();
    let count = find("ufop_http_request_duration_seconds_count", &[("method", "POST"), ("route", route), ("status", "200")]).unwrap();
    assert_eq!(inf, count);
    assert!(!text.contains(&s.dir.display().to_string()), "no raw paths in labels");
    assert!(samples.iter().all(|x| !x.labels.get("route").is_some_and(|r| r.contains(&id))), "routes are templates");
}

#[tokio::test]
async fn standalone_metrics_listener_with_basic_auth() {
    let s = start(ServerConfig::default(), None).await;
    let repo = Repository::open_in_memory().await.unwrap();
    let auth = AuthService::new(settings::SettingsStore::plain(repo.pool().clone()));
    let st = ServerState::new(s.mgr.clone(), None, auth, ServerConfig::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, super::metrics::standalone_router(st, Some("prom:scrape-me".into()))).await.unwrap() });
    let url = format!("http://{addr}/metrics");
    let c = reqwest::Client::new();
    let no = c.get(&url).send().await.unwrap();
    assert_eq!(no.status(), 401);
    assert!(no.headers()["www-authenticate"].to_str().unwrap().starts_with("Basic"));
    assert_eq!(c.get(&url).basic_auth("prom", Some("wrong")).send().await.unwrap().status(), 401);
    let ok = c.get(&url).basic_auth("prom", Some("scrape-me")).send().await.unwrap();
    assert_eq!(ok.status(), 200);
    assert!(ok.text().await.unwrap().contains("ufop_build_info{"));
}

/// A local HTTP receiver that records (method, path, headers, body) of every request.
async fn mock_receiver() -> (SocketAddr, Arc<std::sync::Mutex<Vec<(String, String, HeaderMap, Vec<u8>)>>>) {
    let got: Arc<std::sync::Mutex<Vec<(String, String, HeaderMap, Vec<u8>)>>> = Arc::default();
    let g = got.clone();
    let app = Router::new().fallback(move |req: Request| {
        let g = g.clone();
        async move {
            let (parts, body) = req.into_parts();
            let bytes = axum::body::to_bytes(body, 16 << 20).await.unwrap_or_default();
            g.lock().unwrap().push((parts.method.to_string(), parts.uri.path().to_string(), parts.headers, bytes.to_vec()));
            StatusCode::OK
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, got)
}

fn otlp_spans(bodies: &[(String, String, HeaderMap, Vec<u8>)]) -> Vec<Value> {
    bodies
        .iter()
        .filter(|b| b.1 == "/v1/traces")
        .flat_map(|b| {
            let v: Value = serde_json::from_slice(&b.3).unwrap();
            assert_eq!(v["resourceSpans"][0]["resource"]["attributes"][0], json!({"key": "service.name", "value": {"stringValue": "ufop-test"}}));
            assert_eq!(v["resourceSpans"][0]["scopeSpans"][0]["scope"]["name"], "ufop-server");
            v["resourceSpans"][0]["scopeSpans"][0]["spans"].as_array().unwrap().clone()
        })
        .collect()
}

fn attr<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
    span["attributes"].as_array().unwrap().iter().find(|a| a["key"] == key).map(|a| &a["value"])
}

// Current-thread runtime: the server's tasks run on this thread, under the test's subscriber.
#[tokio::test]
async fn otlp_exports_request_and_backup_spans_with_w3c_parent() {
    use tracing_subscriber::layer::SubscriberExt;
    let (addr, got) = mock_receiver().await;
    let (layer, handle) = otlp::install(otlp::OtlpConfig { endpoint: format!("http://{addr}"), headers: vec![("x-api-key".into(), "collector-key".into())], service_name: "ufop-test".into(), ..Default::default() });
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));

    let s = start(ServerConfig::default(), None).await;
    let tok = s.login().await;
    // Other tests register the same callsites concurrently without this subscriber.
    tracing::callsite::rebuild_interest_cache();
    let src = s.dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"traced").unwrap();
    let job = json!({"name":"Traced","sources":[src],"encrypted":false,"destination":{"type":"local","path": s.dir.join("dst"),"alternate_paths":[],"marker_file":null}});
    let id = s.post("/api/v1/jobs", &tok, json!({"job": job})).await.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    let caller = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let r = s.client.post(format!("{}/api/v1/jobs/{id}/run", s.base)).bearer_auth(&tok).header("traceparent", caller).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    for _ in 0..200 {
        if s.mgr.active_task_count() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(handle.flush(std::time::Duration::from_secs(10)).await);

    let bodies = got.lock().unwrap().clone();
    assert!(bodies.iter().filter(|b| b.1 == "/v1/traces").all(|b| b.0 == "POST" && b.2["x-api-key"] == "collector-key" && b.2["content-type"] == "application/json"));
    let spans = otlp_spans(&bodies);
    let run = spans.iter().find(|s| s["name"] == "POST /api/v1/jobs/{id}/run").unwrap_or_else(|| panic!("{spans:#?}"));
    assert_eq!(run["traceId"], "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(run["parentSpanId"], "00f067aa0ba902b7");
    assert_eq!(run["kind"], 2);
    assert_eq!(attr(run, "http.status_code"), Some(&json!({"intValue": "200"})));
    assert_eq!(attr(run, "http.route"), Some(&json!({"stringValue": "/api/v1/jobs/{id}/run"})));
    assert!(attr(run, "request_id").is_some() && attr(run, "traceparent").is_none() && attr(run, "otel.kind").is_none());
    assert!(run["startTimeUnixNano"].as_str().unwrap().parse::<u128>().unwrap() <= run["endTimeUnixNano"].as_str().unwrap().parse::<u128>().unwrap());
    let op = spans.iter().find(|s| s["name"] == "backup.operation" && attr(s, "job.id") == Some(&json!({"stringValue": id}))).unwrap();
    assert_eq!((op["traceId"].as_str(), op["parentSpanId"].as_str(), op["kind"].as_u64()), (Some("4bf92f3577b34da6a3ce929d0e0e4736"), run["spanId"].as_str(), Some(1)));
    assert_eq!(attr(op, "backup.operation"), Some(&json!({"stringValue": "backup"})));
    assert!(matches!(attr(op, "backup.result").and_then(|v| v["stringValue"].as_str()), Some("success" | "warning")), "{op}");
    assert_eq!(op["status"]["code"], 0);
    // Requests without a traceparent start their own traces.
    let login = spans.iter().find(|s| s["name"] == "POST /api/v1/jobs").unwrap_or_else(|| panic!("{spans:#?}"));
    assert!(login.get("parentSpanId").is_none());
    assert_ne!(login["traceId"], run["traceId"]);
}

#[tokio::test]
async fn otlp_sampling_and_error_status() {
    use tracing_subscriber::layer::SubscriberExt;
    let (addr, got) = mock_receiver().await;
    let (layer, handle) = otlp::install(otlp::OtlpConfig { endpoint: format!("http://{addr}/v1/traces"), service_name: "ufop-test".into(), sample_ratio: 0.0, ..Default::default() });
    {
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        // Not sampled (ratio 0, no parent): dropped with its children.
        tracing::info_span!("http.request", http.method = "GET", http.route = "/a", http.status_code = 200u64, traceparent = "").in_scope(|| {
            tracing::info_span!("backup.operation", backup.result = "success").in_scope(|| {});
        });
        // A sampled caller wins over the ratio; failures carry an error status.
        let req = tracing::info_span!("http.request", http.method = "POST", http.route = "/b", http.status_code = tracing::field::Empty, traceparent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01");
        req.in_scope(|| {
            let op = tracing::info_span!("backup.operation", backup.result = tracing::field::Empty, backup.error = tracing::field::Empty);
            op.record("backup.result", "error");
            op.record("backup.error", "Destination full");
            drop(op);
            // Unrelated spans are not exported.
            tracing::info_span!("hyper.connection").in_scope(|| {});
        });
        req.record("http.status_code", 502u64);
        drop(req);
        // An unsampled caller is respected.
        tracing::info_span!("http.request", http.method = "GET", traceparent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-00").in_scope(|| {});
    }
    assert!(handle.flush(std::time::Duration::from_secs(10)).await);
    let spans = otlp_spans(&got.lock().unwrap());
    assert_eq!(spans.len(), 2, "{spans:#?}");
    let req = spans.iter().find(|s| s["name"] == "POST /b").unwrap();
    assert_eq!(req["status"], json!({"code": 2, "message": "HTTP 502"}));
    let op = spans.iter().find(|s| s["name"] == "backup.operation").unwrap();
    assert_eq!(op["status"], json!({"code": 2, "message": "Destination full"}));
    assert_eq!(op["parentSpanId"], req["spanId"]);
    assert_eq!(handle.dropped.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pushgateway_pushes_periodically_and_on_shutdown() {
    let (addr, got) = mock_receiver().await;
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let mgr = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, dir.path().join("dbs"), KdfParams::insecure_fast());
    let auth = AuthService::new(settings::SettingsStore::plain(repo.pool().clone()));
    let st = ServerState::new(mgr, None, auth, ServerConfig::default());
    let cfg = otlp::PushConfig { url: format!("http://{addr}/"), job: "ufop".into(), instance: "node-1".into(), interval: std::time::Duration::from_secs(3600), basic_auth: Some("pg:secret".into()), bearer: None };
    let task = otlp::spawn_pushgateway(st.clone(), cfg);
    for _ in 0..100 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(got.lock().unwrap().len(), 1, "first push at start");
    st.life.request(None);
    tokio::time::timeout(std::time::Duration::from_secs(10), task).await.unwrap().unwrap();
    let pushes = got.lock().unwrap().clone();
    assert_eq!(pushes.len(), 2, "plus a final push on shutdown");
    for (method, path, headers, body) in &pushes {
        assert_eq!((method.as_str(), path.as_str()), ("PUT", "/metrics/job/ufop/instance/node-1"));
        assert_eq!(headers["authorization"], "Basic cGc6c2VjcmV0");
        assert!(headers["content-type"].to_str().unwrap().starts_with("text/plain; version=0.0.4"));
        let (_, samples) = super::metrics::parse::parse(std::str::from_utf8(body).unwrap());
        assert!(samples.iter().any(|x| x.name == "ufop_build_info"));
    }
    let last = std::str::from_utf8(&pushes[1].3).unwrap();
    assert!(last.contains("ufop_shutting_down 1"), "the final push shows the drain");
}
