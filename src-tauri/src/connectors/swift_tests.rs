//! Swift connector tests against an in-process Swift + Keystone mock that
//! keeps the services' semantics: Keystone v3 password and application
//! credential sign-in with a regional catalog, TempAuth, expiring tokens,
//! delimiter listings in short pages (the subdir equal to the marker is
//! not repeated, as in Swift), ETag checks, Static Large Object manifests
//! (segment etags and sizes verified, `multipart-manifest=get|delete`),
//! 409 on deleting a non-empty container, and TempURL signature checks.

use super::*;
use crate::connectors::live_tests::profile;
use crate::connectors::{ShareOptions, ShareScope};
use crate::core::types::{ConnectionProtocol, SwiftSettings};
use axum::body::Bytes as AxBytes;
use axum::http::{HeaderMap, Method as AxMethod, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

const PAGE: usize = 2;

#[derive(Clone, Default)]
struct Obj {
    data: Vec<u8>,
    content_type: String,
    /// SLO manifest: segment paths (`/container/object`).
    slo: Option<Vec<String>>,
}

#[derive(Default)]
struct Mock {
    base: Mutex<String>,
    tokens: Mutex<HashSet<String>>,
    issued: Mutex<u32>,
    containers: Mutex<BTreeMap<String, BTreeMap<String, Obj>>>,
    account_temp_key: Mutex<Option<String>>,
    auth_calls: Mutex<Vec<String>>,
}

fn q(u: &Uri, k: &str) -> Option<String> {
    url::form_urlencoded::parse(u.query().unwrap_or("").as_bytes()).find(|(a, _)| a == k).map(|(_, v)| v.into_owned())
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn md5(b: &[u8]) -> String {
    super::md5_hex(b)
}

impl Mock {
    fn issue(&self) -> String {
        let mut n = self.issued.lock().unwrap();
        *n += 1;
        let t = format!("tok{n}");
        self.tokens.lock().unwrap().insert(t.clone());
        t
    }

    fn expire_tokens(&self) {
        self.tokens.lock().unwrap().clear();
    }

    fn keystone(&self, b: &[u8]) -> Response {
        let v: Value = serde_json::from_slice(b).unwrap_or_default();
        let id = &v["auth"]["identity"];
        let ok = match id["methods"][0].as_str() {
            Some("password") => {
                let u = &id["password"]["user"];
                let scope = &v["auth"]["scope"]["project"];
                u["name"] == "alice" && u["password"] == "pw" && u["domain"]["name"] == "Default" && scope["name"] == "proj" && scope["domain"]["name"] == "Default"
            }
            Some("application_credential") => id["application_credential"]["id"] == "ac1" && id["application_credential"]["secret"] == "acs" && v["auth"]["scope"].is_null(),
            _ => false,
        };
        self.auth_calls.lock().unwrap().push(id["methods"][0].as_str().unwrap_or("?").to_string());
        if !ok {
            return (StatusCode::UNAUTHORIZED, "bad credentials").into_response();
        }
        let base = self.base.lock().unwrap().clone();
        let body = json!({ "token": { "catalog": [
            { "type": "identity", "endpoints": [ { "interface": "public", "region": "R1", "url": format!("{base}/v3") } ] },
            { "type": "object-store", "endpoints": [
                { "interface": "internal", "region": "R1", "url": format!("{base}/internal") },
                { "interface": "public", "region": "R1", "url": format!("{base}/v1/AUTH_proj") },
                { "interface": "public", "region": "R2", "url": format!("{base}/v1/AUTH_r2/") }
            ] }
        ] } });
        ([("x-subject-token", self.issue())], axum::Json(body)).into_response()
    }

    fn tempauth(&self, h: &HeaderMap) -> Response {
        let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        self.auth_calls.lock().unwrap().push("tempauth".into());
        if get("x-auth-user") != "proj:alice" || get("x-auth-key") != "pw" {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let base = self.base.lock().unwrap().clone();
        let token = self.issue();
        ([("x-storage-url", format!("{base}/v1/AUTH_proj")), ("x-auth-token", token)], "").into_response()
    }

    /// Swift's delimiter listing, at most `PAGE` entries after `marker`.
    fn listing(names: Vec<(String, Value)>, u: &Uri) -> Vec<Value> {
        let pfx = q(u, "prefix").unwrap_or_default();
        let marker = q(u, "marker").unwrap_or_default();
        let delim = q(u, "delimiter");
        let limit = q(u, "limit").and_then(|l| l.parse().ok()).unwrap_or(PAGE).min(PAGE);
        let mut out: Vec<Value> = Vec::new();
        let mut last_dir = String::new();
        for (name, entry) in names {
            if !name.starts_with(&pfx) || name <= marker {
                continue;
            }
            if out.len() >= limit {
                break;
            }
            if let Some(i) = delim.as_ref().and_then(|d| name[pfx.len()..].find(d.as_str())) {
                let dir = name[..pfx.len() + i + 1].to_string();
                if dir != marker && dir != last_dir {
                    out.push(json!({ "subdir": dir }));
                }
                last_dir = dir;
                continue;
            }
            out.push(entry);
        }
        out
    }

    fn handle(&self, m: AxMethod, u: Uri, h: HeaderMap, b: AxBytes) -> Response {
        let path = u.path().to_string();
        if path == "/v3/auth/tokens" && m == AxMethod::POST {
            return self.keystone(&b);
        }
        if path == "/auth/v1.0" {
            return self.tempauth(&h);
        }
        let Some(rest) = path.strip_prefix("/v1/AUTH_proj").or_else(|| path.strip_prefix("/v1/AUTH_r2")) else { return StatusCode::NOT_FOUND.into_response() };
        let (container, key) = match rest.trim_start_matches('/').split_once('/') {
            Some((c, k)) => (decode(c), decode(k)),
            None => (decode(rest.trim_start_matches('/')), String::new()),
        };
        // A TempURL GET carries no token.
        if let (Some(sig), Some(exp)) = (q(&u, "temp_url_sig"), q(&u, "temp_url_expires")) {
            let key_ok = self.account_temp_key.lock().unwrap().clone().is_some_and(|k| super::temp_url_signature(&k, exp.parse().unwrap_or(0), &decode(&path)) == sig);
            if m != AxMethod::GET || !key_ok || exp.parse::<i64>().unwrap_or(0) < chrono::Utc::now().timestamp() {
                return (StatusCode::UNAUTHORIZED, "temp url invalid").into_response();
            }
            return self.get_object(&container, &key, &u);
        }
        let token = h.get("x-auth-token").and_then(|v| v.to_str().ok()).unwrap_or_default();
        if !self.tokens.lock().unwrap().contains(token) {
            return (StatusCode::UNAUTHORIZED, "token expired").into_response();
        }
        if path.starts_with("/v1/AUTH_r2") {
            return if container.is_empty() { StatusCode::NO_CONTENT.into_response() } else { StatusCode::FORBIDDEN.into_response() };
        }
        let mut cs = self.containers.lock().unwrap();
        match (m.as_str(), container.is_empty(), key.is_empty()) {
            ("HEAD", true, _) => {
                let mut r = StatusCode::NO_CONTENT.into_response();
                if let Some(k) = self.account_temp_key.lock().unwrap().clone() {
                    r.headers_mut().insert("x-account-meta-temp-url-key", k.parse().unwrap());
                }
                r
            }
            ("GET", true, _) => {
                let names = cs.iter().map(|(n, objs)| (n.clone(), json!({ "name": n, "count": objs.len(), "bytes": objs.values().map(|o| o.data.len()).sum::<usize>(), "last_modified": "2024-05-06T07:08:09.123456" }))).collect();
                axum::Json(Self::listing(names, &u)).into_response()
            }
            ("PUT", false, true) => {
                let fresh = !cs.contains_key(&container);
                cs.entry(container).or_default();
                if fresh { StatusCode::CREATED } else { StatusCode::ACCEPTED }.into_response()
            }
            ("HEAD", false, true) => if cs.contains_key(&container) { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }.into_response(),
            ("DELETE", false, true) => match cs.get(&container) {
                None => StatusCode::NOT_FOUND.into_response(),
                Some(objs) if !objs.is_empty() => (StatusCode::CONFLICT, "There was a conflict when trying to complete your request.").into_response(),
                Some(_) => {
                    cs.remove(&container);
                    StatusCode::NO_CONTENT.into_response()
                }
            },
            ("GET", false, true) => {
                let Some(objs) = cs.get(&container) else { return StatusCode::NOT_FOUND.into_response() };
                let names = objs.iter().map(|(n, o)| (n.clone(), json!({ "name": n, "bytes": o.data.len(), "content_type": o.content_type, "last_modified": "2024-05-06T07:08:09.123456", "hash": md5(&o.data) }))).collect();
                axum::Json(Self::listing(names, &u)).into_response()
            }
            ("PUT", false, false) => {
                if !cs.contains_key(&container) {
                    return StatusCode::NOT_FOUND.into_response();
                }
                if q(&u, "multipart-manifest").as_deref() == Some("put") {
                    let segs: Vec<Value> = serde_json::from_slice(&b).unwrap_or_default();
                    let mut paths = Vec::new();
                    for s in &segs {
                        let p = s["path"].as_str().unwrap_or_default().to_string();
                        let (sc, sk) = p.trim_start_matches('/').split_once('/').unwrap_or(("", ""));
                        let Some(seg) = cs.get(sc).and_then(|c| c.get(sk)) else { return (StatusCode::BAD_REQUEST, "missing segment").into_response() };
                        if s["etag"].as_str() != Some(md5(&seg.data).as_str()) || s["size_bytes"].as_u64() != Some(seg.data.len() as u64) {
                            return (StatusCode::BAD_REQUEST, "segment mismatch").into_response();
                        }
                        paths.push(p);
                    }
                    cs.get_mut(&container).unwrap().insert(key, Obj { slo: Some(paths), content_type: "application/octet-stream".into(), ..Default::default() });
                    return StatusCode::CREATED.into_response();
                }
                if let Some(e) = h.get("etag").and_then(|v| v.to_str().ok()) {
                    if e != md5(&b) {
                        return (StatusCode::UNPROCESSABLE_ENTITY, "etag mismatch").into_response();
                    }
                }
                let ct = h.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
                cs.get_mut(&container).unwrap().insert(key, Obj { data: b.to_vec(), content_type: ct, slo: None });
                StatusCode::CREATED.into_response()
            }
            ("HEAD", false, false) => match cs.get(&container).and_then(|c| c.get(&key)) {
                None => StatusCode::NOT_FOUND.into_response(),
                Some(o) => {
                    let mut r = StatusCode::OK.into_response();
                    if o.slo.is_some() {
                        r.headers_mut().insert("x-static-large-object", "True".parse().unwrap());
                    }
                    r
                }
            },
            ("GET", false, false) => {
                drop(cs);
                self.get_object(&container, &key, &u)
            }
            ("DELETE", false, false) => {
                let Some(o) = cs.get(&container).and_then(|c| c.get(&key)).cloned() else { return StatusCode::NOT_FOUND.into_response() };
                if q(&u, "multipart-manifest").as_deref() == Some("delete") {
                    let Some(segs) = o.slo else { return (StatusCode::BAD_REQUEST, "Not an SLO manifest").into_response() };
                    for p in segs {
                        let (sc, sk) = p.trim_start_matches('/').split_once('/').unwrap_or(("", ""));
                        if let Some(c) = cs.get_mut(sc) {
                            c.remove(sk);
                        }
                    }
                }
                cs.get_mut(&container).unwrap().remove(&key);
                StatusCode::NO_CONTENT.into_response()
            }
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        }
    }

    fn get_object(&self, container: &str, key: &str, u: &Uri) -> Response {
        let cs = self.containers.lock().unwrap();
        let Some(o) = cs.get(container).and_then(|c| c.get(key)) else { return StatusCode::NOT_FOUND.into_response() };
        match &o.slo {
            Some(segs) if q(u, "multipart-manifest").as_deref() == Some("get") => {
                let v: Vec<Value> = segs.iter().map(|p| json!({ "name": p, "bytes": 0, "hash": "" })).collect();
                axum::Json(v).into_response()
            }
            Some(segs) => {
                let mut data = Vec::new();
                for p in segs {
                    let (sc, sk) = p.trim_start_matches('/').split_once('/').unwrap_or(("", ""));
                    data.extend_from_slice(&cs.get(sc).and_then(|c| c.get(sk)).map(|s| s.data.clone()).unwrap_or_default());
                }
                data.into_response()
            }
            None => o.data.clone().into_response(),
        }
    }

    fn objects(&self, container: &str) -> Vec<String> {
        self.containers.lock().unwrap().get(container).map(|c| c.keys().cloned().collect()).unwrap_or_default()
    }
}

async fn serve() -> (String, Arc<Mock>) {
    let m = Arc::new(Mock::default());
    let m2 = m.clone();
    let app = axum::Router::new()
        .fallback(move |meth: AxMethod, u: Uri, h: HeaderMap, b: AxBytes| {
            let m = m2.clone();
            async move { m.handle(meth, u, h, b) }
        })
        .layer(axum::extract::DefaultBodyLimit::disable());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    *m.base.lock().unwrap() = base.clone();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, m)
}

fn swift_profile(auth_url: &str, user: &str, settings: SwiftSettings) -> ConnectionProfile {
    let mut p = profile(ConnectionProtocol::Swift, "", None, user, "/");
    p.extras.swift = Some(SwiftSettings { auth_url: Some(auth_url.into()), ..settings });
    p
}

fn keystone(region: Option<&str>) -> SwiftSettings {
    SwiftSettings { project_name: Some("proj".into()), region: region.map(String::from), ..Default::default() }
}

fn names(entries: &[FileEntry]) -> Vec<String> {
    let mut v: Vec<String> = entries.iter().map(|e| format!("{}{}", e.name, if e.is_dir { "/" } else { "" })).collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn keystone_round_trip_with_large_objects_and_folders() {
    let (base, mock) = serve().await;
    let c = SwiftConnector::with_segment_size(1000);
    // The auth URL may omit the version: /v3 is added.
    let p = swift_profile(&base, "alice", keystone(Some("R1")));
    assert!(c.connect_with_secret(&p, Some("wrong".into())).await.is_err());
    assert!(!c.is_connected());
    assert!(c.connect_with_secret(&p, None).await.is_err(), "a missing password is refused before any request");
    c.connect_with_secret(&p, Some("pw".into())).await.unwrap();
    assert!(c.is_connected());

    c.make_directory("/docs").await.unwrap();
    c.make_directory("/docs/reports").await.unwrap();
    assert_eq!(mock.containers.lock().unwrap()["docs"]["reports/"].content_type, "application/directory");

    let tmp = tempfile::tempdir().unwrap();
    let small = tmp.path().join("small.txt");
    std::fs::write(&small, b"hello swift").unwrap();
    let big = tmp.path().join("big.bin");
    let data: Vec<u8> = (0..3500u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    std::fs::write(&big, &data).unwrap();

    assert_eq!(c.upload(small.to_str().unwrap(), "/docs/reports/a b.txt").await.unwrap(), 11);
    assert_eq!(c.upload(big.to_str().unwrap(), "/docs/reports/big.bin").await.unwrap(), 3500);
    let segs = mock.objects("docs_segments");
    assert_eq!(segs.len(), 4, "3500 bytes in 1000-byte segments: {segs:?}");
    assert!(segs.iter().all(|s| s.starts_with("reports/big.bin/slo/")));
    for i in 0..5 {
        c.upload(small.to_str().unwrap(), &format!("/docs/reports/sub{i}/f.txt")).await.unwrap();
    }

    // Listings come in pages of two; folders appear once.
    let listing = c.list_remote("/docs/reports").await.unwrap();
    assert_eq!(names(&listing), ["a b.txt", "big.bin", "sub0/", "sub1/", "sub2/", "sub3/", "sub4/"]);
    let big_entry = listing.iter().find(|e| e.name == "big.bin").unwrap();
    assert_eq!((big_entry.path.as_str(), big_entry.modified.map(|t| t.to_rfc3339())), ("/docs/reports/big.bin", Some("2024-05-06T07:08:09.123456+00:00".into())));
    assert_eq!(names(&c.list_remote("/docs").await.unwrap()), ["reports/"]);
    let containers = c.list_remote("/").await.unwrap();
    assert_eq!(names(&containers), ["docs/", "docs_segments/"]);
    assert_eq!(containers[0].path, "/docs");

    let back = tmp.path().join("back.bin");
    assert_eq!(c.download("/docs/reports/big.bin", back.to_str().unwrap()).await.unwrap(), 3500);
    assert_eq!(std::fs::read(&back).unwrap(), data, "the large object reassembles exactly");
    c.download("/docs/reports/a b.txt", back.to_str().unwrap()).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), b"hello swift");

    // Replacing a large object with a small one removes the old segments.
    c.upload(big.to_str().unwrap(), "/docs/reports/big2.bin").await.unwrap();
    assert_eq!(mock.objects("docs_segments").len(), 8);
    c.upload(small.to_str().unwrap(), "/docs/reports/big.bin").await.unwrap();
    assert_eq!(mock.objects("docs_segments").len(), 4);
    // Deleting a large object deletes its segments.
    c.remove("/docs/reports/big2.bin", false).await.unwrap();
    assert!(mock.objects("docs_segments").is_empty());

    // Non-empty folders and containers stay.
    let err = c.remove("/docs/reports", true).await.unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");
    assert!(c.remove("/docs", true).await.is_err());
    for i in 0..5 {
        c.remove(&format!("/docs/reports/sub{i}/f.txt"), false).await.unwrap();
        // A pseudo-folder without a marker deletes cleanly.
        c.remove(&format!("/docs/reports/sub{i}"), true).await.unwrap();
    }
    c.remove("/docs/reports/a b.txt", false).await.unwrap();
    c.remove("/docs/reports/big.bin", false).await.unwrap();
    c.remove("/docs/reports", true).await.unwrap();
    c.remove("/docs", true).await.unwrap();
    c.remove("/docs_segments", true).await.unwrap();
    assert!(mock.containers.lock().unwrap().is_empty());
    assert!(c.remove("/missing/x", false).await.is_err());

    c.disconnect().await.unwrap();
    assert!(!c.is_connected());
    assert!(c.list_remote("/").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_tokens_are_renewed_once() {
    let (base, mock) = serve().await;
    let c = SwiftConnector::new();
    c.connect_with_secret(&swift_profile(&format!("{base}/v3"), "alice", keystone(None)), Some("pw".into())).await.unwrap();
    c.make_directory("/c").await.unwrap();
    mock.expire_tokens();
    assert_eq!(names(&c.list_remote("/").await.unwrap()), ["c/"]);
    assert_eq!(mock.auth_calls.lock().unwrap().len(), 2, "one sign-in, one renewal");
}

#[tokio::test(flavor = "multi_thread")]
async fn region_application_credential_and_tempauth() {
    let (base, mock) = serve().await;
    // Region R2 picks its own public endpoint (the mock refuses containers there).
    let c = SwiftConnector::new();
    c.connect_with_secret(&swift_profile(&base, "alice", keystone(Some("R2"))), Some("pw".into())).await.unwrap();
    assert!(c.make_directory("/x").await.is_err());
    let err = SwiftConnector::new().connect_with_secret(&swift_profile(&base, "alice", keystone(Some("R9"))), Some("pw".into())).await.unwrap_err();
    assert!(err.to_string().contains("object-store endpoint"), "{err}");

    let ac = SwiftSettings { auth_kind: SwiftAuthKind::ApplicationCredential, application_credential_id: Some("ac1".into()), ..Default::default() };
    let c = SwiftConnector::new();
    c.connect_with_secret(&swift_profile(&base, "", ac.clone()), Some("acs".into())).await.unwrap();
    c.make_directory("/ac").await.unwrap();
    assert!(SwiftConnector::new().connect_with_secret(&swift_profile(&base, "", ac), Some("nope".into())).await.is_err());

    let ta = SwiftSettings { auth_kind: SwiftAuthKind::TempAuth, ..Default::default() };
    let c = SwiftConnector::new();
    c.connect_with_secret(&swift_profile(&format!("{base}/auth/v1.0"), "proj:alice", ta.clone()), Some("pw".into())).await.unwrap();
    assert_eq!(names(&c.list_remote("/").await.unwrap()), ["ac/"]);
    assert!(SwiftConnector::new().connect_with_secret(&swift_profile(&format!("{base}/auth/v1.0"), "proj:alice", ta), Some("bad".into())).await.is_err());
    assert!(mock.auth_calls.lock().unwrap().contains(&"application_credential".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn share_links_are_signed_temp_urls() {
    let (base, mock) = serve().await;
    let c = SwiftConnector::new();
    c.connect_with_secret(&swift_profile(&base, "alice", keystone(None)), Some("pw".into())).await.unwrap();
    c.make_directory("/pub").await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("r.txt");
    std::fs::write(&f, b"report").unwrap();
    c.upload(f.to_str().unwrap(), "/pub/q 1/r.txt").await.unwrap();

    let opts = ShareOptions { expires_in_secs: Some(7200), ..Default::default() };
    let err = c.create_share_link("/pub/q 1/r.txt", &opts).await.unwrap_err();
    assert!(err.to_string().contains("TempURL key"), "{err}");

    *mock.account_temp_key.lock().unwrap() = Some("s3cret".into());
    let link = c.create_share_link("/pub/q 1/r.txt", &opts).await.unwrap();
    assert!(link.url.starts_with(&format!("{base}/v1/AUTH_proj/pub/q%201/r.txt?temp_url_sig=")), "{}", link.url);
    assert_eq!(link.scope, ShareScope::Anyone);
    let secs = (link.expires_at.unwrap() - chrono::Utc::now()).num_seconds();
    assert!((7100..=7200).contains(&secs), "{secs}");
    // Anyone with the link can download without a token…
    let r = reqwest::get(&link.url).await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "report");
    // …but not with a tampered signature.
    assert_eq!(reqwest::get(link.url.replace("temp_url_sig=", "temp_url_sig=0")).await.unwrap().status(), 401);

    assert!(c.create_share_link("/pub", &opts).await.is_err());
    assert!(c.create_share_link("/pub/q 1/r.txt", &ShareOptions { allow_edit: true, ..opts.clone() }).await.is_err());
}

#[test]
fn profile_settings_map_to_the_session() {
    let mut p = profile(ConnectionProtocol::Swift, "keystone.example.com", Some(5000), "alice", "/");
    p.extras.swift = Some(SwiftSettings { project_name: Some(" proj ".into()), region: Some("RegionOne".into()), ..Default::default() });
    let cfg = config_for_profile(&p).unwrap();
    assert_eq!(cfg.auth_url, "https://keystone.example.com:5000/v3");
    assert_eq!((cfg.auth_version.as_str(), cfg.project.as_str(), cfg.user_domain.as_str(), cfg.region.as_str()), ("3", "proj", "Default", "RegionOne"));

    p.host = "http://10.0.0.5:8080".into();
    p.extras.swift = Some(SwiftSettings { auth_kind: SwiftAuthKind::TempAuth, ..Default::default() });
    assert_eq!(config_for_profile(&p).unwrap().auth_url, "http://10.0.0.5:8080/auth/v1.0");
    assert_eq!(config_for_profile(&p).unwrap().auth_version, "1");

    p.extras.swift = Some(SwiftSettings { auth_kind: SwiftAuthKind::ApplicationCredential, application_credential_id: Some("id".into()), domain_name: Some("Corp".into()), ..Default::default() });
    let cfg = config_for_profile(&p).unwrap();
    assert_eq!((cfg.application_credential_id.as_str(), cfg.project_domain.as_str()), ("id", "Corp"));

    // Missing settings are caught before connecting.
    p.extras.swift = None;
    assert!(config_for_profile(&p).unwrap_err().to_string().contains("project"));
    p.extras.swift = Some(SwiftSettings { auth_kind: SwiftAuthKind::ApplicationCredential, ..Default::default() });
    assert!(config_for_profile(&p).is_err());
    p.host = String::new();
    p.extras.swift = Some(keystone(None));
    assert!(config_for_profile(&p).unwrap_err().to_string().contains("auth URL"));
}

#[test]
fn temp_url_signature_matches_swift() {
    // From the Swift TempURL docs' algorithm: HMAC-SHA256 over "GET\n<expires>\n<path>".
    let sig = temp_url_signature("mykey", 1_440_619_048, "/v1/AUTH_account/container/object");
    assert_eq!(sig, {
        use hmac::{Hmac, Mac};
        let mut m = Hmac::<sha2::Sha256>::new_from_slice(b"mykey").unwrap();
        m.update(b"GET\n1440619048\n/v1/AUTH_account/container/object");
        hex::encode(m.finalize().into_bytes())
    });
    assert_eq!(sig.len(), 64);
}

#[test]
fn swift_times_parse_as_utc() {
    assert_eq!(parse_time("2024-05-06T07:08:09.123456").unwrap().to_rfc3339(), "2024-05-06T07:08:09.123456+00:00");
    assert_eq!(parse_time("2024-05-06T07:08:09").unwrap().timestamp(), 1_714_979_289);
    assert!(parse_time("yesterday").is_none());
}
