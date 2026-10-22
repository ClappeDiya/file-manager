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
    tokio::spawn(async move { axum::serve(listener, router(st).into_make_service_with_connect_info::<SocketAddr>()).await.unwrap() });
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
