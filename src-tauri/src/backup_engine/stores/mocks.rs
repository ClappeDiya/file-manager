//! In-process mock services for providers without a local emulator. Each
//! mock implements the subset of the real API the store uses, checks the
//! authentication it expects, and keeps objects in memory.

use super::tests::exercise;
use super::*;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

type Files = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

#[derive(Clone)]
struct St {
    files: Files,
    base: Arc<Mutex<String>>,
    /// Access tokens accepted; the first request with "stale" gets 401.
    token: Arc<Mutex<String>>,
    extra: Arc<Mutex<BTreeMap<String, String>>>,
}

async fn serve<F, Fut>(f: F) -> (String, St)
where
    F: Fn(St, Method, Uri, HeaderMap, Bytes) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response> + Send + 'static,
{
    let st = St { files: Default::default(), base: Default::default(), token: Arc::new(Mutex::new("fresh".into())), extra: Default::default() };
    let s2 = st.clone();
    let app = axum::Router::new()
        .fallback(move |m: Method, u: Uri, h: HeaderMap, b: Bytes| {
            let f = f.clone();
            let s = s2.clone();
            async move { f(s, m, u, h, b).await }
        })
        .layer(axum::extract::DefaultBodyLimit::disable());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    *st.base.lock().unwrap() = base.clone();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, st)
}

fn q(u: &Uri, k: &str) -> Option<String> {
    url::form_urlencoded::parse(u.query().unwrap_or("").as_bytes()).find(|(a, _)| a == k).map(|(_, v)| v.into_owned())
}

fn bearer_ok(st: &St, h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {}", st.token.lock().unwrap()))
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "expired").into_response()
}

fn jsonr(v: Value) -> Response {
    axum::Json(v).into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "missing").into_response()
}

fn decode(p: &str) -> String {
    urlencoding::decode(p).map(|c| c.into_owned()).unwrap_or_else(|_| p.to_string())
}

/// Token endpoint shared by the OAuth mocks: exchanges the refresh token.
fn token_endpoint(st: &St, body: &Bytes) -> Response {
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(body).into_owned().collect();
    if form.get("grant_type").map(String::as_str) == Some("refresh_token") && form.get("refresh_token").map(String::as_str) == Some("r1") {
        return jsonr(json!({ "access_token": st.token.lock().unwrap().clone(), "expires_in": 3600 }));
    }
    (StatusCode::BAD_REQUEST, "bad refresh").into_response()
}

/// Secret JSON whose access token is stale, forcing a refresh.
fn stale_secret(base: &str) -> String {
    json!({ "access_token": "stale", "refresh_token": "r1", "client_id": "c", "token_url": format!("{base}/oauth2/token") }).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn b2_mock() {
    let (base, st) = serve(|st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = u.path().to_string();
        let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
        let base = st.base.lock().unwrap().clone();
        match (m.as_str(), path.as_str()) {
            ("GET", "/b2api/v2/b2_authorize_account") => {
                use base64::Engine;
                if auth != format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("kid:appkey")) {
                    return unauthorized();
                }
                jsonr(json!({ "apiUrl": base, "downloadUrl": base, "authorizationToken": "tok", "accountId": "acc" }))
            }
            _ if auth != "tok" && auth != "up" => unauthorized(),
            ("POST", "/b2api/v2/b2_list_buckets") => jsonr(json!({ "buckets": [{ "bucketName": body["bucketName"], "bucketId": "bid" }] })),
            ("POST", "/b2api/v2/b2_list_file_names") => {
                let pfx = body["prefix"].as_str().unwrap_or("");
                let files: Vec<Value> = st.files.lock().unwrap().iter().filter(|(k, _)| k.starts_with(pfx)).map(|(k, v)| json!({ "fileName": k, "contentLength": v.len(), "action": "upload" })).collect();
                jsonr(json!({ "files": files, "nextFileName": null }))
            }
            ("POST", "/b2api/v2/b2_get_upload_url") => jsonr(json!({ "uploadUrl": format!("{base}/upload"), "authorizationToken": "up" })),
            ("POST", "/upload") => {
                use sha1::Digest;
                let name = decode(h.get("x-bz-file-name").unwrap().to_str().unwrap());
                if h.get("x-bz-content-sha1").unwrap().to_str().unwrap() != hex::encode(sha1::Sha1::digest(&b)) {
                    return (StatusCode::BAD_REQUEST, "sha1 mismatch").into_response();
                }
                st.files.lock().unwrap().insert(name, b.to_vec());
                jsonr(json!({}))
            }
            ("POST", "/b2api/v2/b2_list_file_versions") => {
                let k = body["startFileName"].as_str().unwrap_or("");
                let files: Vec<Value> = st.files.lock().unwrap().keys().filter(|x| *x == k).map(|x| json!({ "fileName": x, "fileId": format!("id-{x}") })).collect();
                jsonr(json!({ "files": files }))
            }
            ("POST", "/b2api/v2/b2_delete_file_version") => {
                st.files.lock().unwrap().remove(body["fileName"].as_str().unwrap_or(""));
                jsonr(json!({}))
            }
            ("POST", "/b2api/v2/b2_update_file_retention") => {
                st.extra.lock().unwrap().insert(body["fileName"].as_str().unwrap_or("").into(), body["fileRetention"]["mode"].as_str().unwrap_or("").into());
                jsonr(json!({}))
            }
            ("GET", p) if p.starts_with("/file/bkt/") => match st.files.lock().unwrap().get(&decode(&p["/file/bkt/".len()..])) {
                Some(d) => d.clone().into_response(),
                None => not_found(),
            },
            _ => not_found(),
        }
    })
    .await;
    let store = b2::B2Store::connect(b2::B2Config { key_id: "kid".into(), bucket: "bkt".into(), prefix: "pc".into(), api_url: base.clone() }, "appkey".into()).await.unwrap();
    exercise(&store, 400_000).await;
    // Sharded layout through prefix folders.
    let sharded = b2::B2Store::connect(b2::B2Config { key_id: "kid".into(), bucket: "bkt".into(), prefix: "sharded".into(), api_url: base.clone() }, "appkey".into()).await.unwrap();
    let b = std::sync::Arc::new(StoreBackend::new(Box::new(sharded), None));
    tokio::task::spawn_blocking(move || super::tests::backup_roundtrip_with(b.as_ref(), ufop_core::backup::layout::RepoLayout::Sharded { prefix_len: 1 })).await.unwrap();
    store.put("ufop-l.dblock", b"x".to_vec()).await.unwrap();
    store.lock("ufop-l.dblock", chrono::Utc::now(), ufop_core::backup::backend::LockMode::Compliance).await.unwrap();
    assert_eq!(st.extra.lock().unwrap().get("pc/ufop-l.dblock").map(String::as_str), Some("compliance"));
    assert!(b2::B2Store::connect(b2::B2Config { key_id: "kid".into(), bucket: "bkt".into(), prefix: String::new(), api_url: base }, "wrong".into()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn swift_mock() {
    let (base, _st) = serve(|st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = u.path().to_string();
        let base = st.base.lock().unwrap().clone();
        if path == "/v3/auth/tokens" {
            let v: Value = serde_json::from_slice(&b).unwrap();
            if v["auth"]["identity"]["password"]["user"]["password"] != "pw" {
                return unauthorized();
            }
            let body = json!({ "token": { "catalog": [ { "type": "object-store", "endpoints": [ { "interface": "public", "region": "R1", "url": format!("{base}/v1/AUTH_p") } ] } ] } });
            return ([("x-subject-token", "stok")], axum::Json(body)).into_response();
        }
        if h.get("x-auth-token").and_then(|v| v.to_str().ok()) != Some("stok") {
            return unauthorized();
        }
        let Some(rest) = path.strip_prefix("/v1/AUTH_p/cont") else { return not_found() };
        let key = decode(rest.trim_start_matches('/'));
        match (m.as_str(), key.is_empty()) {
            ("PUT", true) => StatusCode::CREATED.into_response(),
            ("GET", true) => {
                let pfx = q(&u, "prefix").unwrap_or_default();
                let marker = q(&u, "marker").unwrap_or_default();
                // Pages of at most 2, like a server with a low listing limit.
                let v: Vec<Value> = st.files.lock().unwrap().iter().filter(|(k, _)| k.starts_with(&pfx) && k.as_str() > marker.as_str()).take(2).map(|(k, v)| json!({ "name": k, "bytes": v.len() })).collect();
                jsonr(Value::Array(v))
            }
            ("PUT", false) => {
                use md5::Digest;
                if h.get("etag").unwrap().to_str().unwrap() != hex::encode(md5::Md5::digest(&b)) {
                    return (StatusCode::UNPROCESSABLE_ENTITY, "etag").into_response();
                }
                st.files.lock().unwrap().insert(key, b.to_vec());
                StatusCode::CREATED.into_response()
            }
            ("GET", false) => st.files.lock().unwrap().get(&key).cloned().map(|d| d.into_response()).unwrap_or_else(not_found),
            ("DELETE", false) => {
                st.files.lock().unwrap().remove(&key);
                StatusCode::NO_CONTENT.into_response()
            }
            _ => not_found(),
        }
    })
    .await;
    let cfg = swift::SwiftConfig { auth_url: format!("{base}/v3"), username: "u".into(), project: "p".into(), region: "R1".into(), container: "cont".into(), prefix: "pc".into(), ..Default::default() };
    let store = swift::SwiftStore::connect(cfg.clone(), "pw".into()).await.unwrap();
    exercise(&store, 300_000).await;
    let sharded = swift::SwiftStore::connect(swift::SwiftConfig { prefix: "sharded".into(), ..cfg.clone() }, "pw".into()).await.unwrap();
    let b = std::sync::Arc::new(StoreBackend::new(Box::new(sharded), None));
    tokio::task::spawn_blocking(move || super::tests::backup_roundtrip_with(b.as_ref(), ufop_core::backup::layout::RepoLayout::Sharded { prefix_len: 1 })).await.unwrap();
    assert!(swift::SwiftStore::connect(cfg, "bad".into()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn dropbox_mock_with_token_refresh() {
    let (base, _st) = serve(|st: St, _m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = u.path().to_string();
        if path == "/oauth2/token" {
            return token_endpoint(&st, &b);
        }
        if !bearer_ok(&st, &h) {
            return unauthorized();
        }
        let arg: Value = h.get("dropbox-api-arg").and_then(|v| serde_json::from_str(v.to_str().ok()?).ok()).unwrap_or(Value::Null);
        let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
        match path.as_str() {
            "/2/files/list_folder" => {
                let p = format!("{}/", body["path"].as_str().unwrap_or(""));
                let e: Vec<Value> = st.files.lock().unwrap().iter().filter(|(k, _)| k.starts_with(&p) && !k[p.len()..].contains('/')).map(|(k, v)| json!({ ".tag": "file", "name": &k[p.len()..], "size": v.len() })).collect();
                jsonr(json!({ "entries": e, "has_more": false, "cursor": "c" }))
            }
            "/2/files/upload" => {
                st.files.lock().unwrap().insert(arg["path"].as_str().unwrap().to_string(), b.to_vec());
                jsonr(json!({}))
            }
            "/2/files/download" => st.files.lock().unwrap().get(arg["path"].as_str().unwrap_or("")).cloned().map(|d| d.into_response()).unwrap_or_else(|| (StatusCode::CONFLICT, "path/not_found").into_response()),
            "/2/files/delete_v2" => {
                st.files.lock().unwrap().remove(body["path"].as_str().unwrap_or(""));
                jsonr(json!({}))
            }
            "/2/files/create_folder_v2" => {
                let mut x = st.extra.lock().unwrap();
                let p = body["path"].as_str().unwrap_or("").to_string();
                if x.contains_key(&p) {
                    return (StatusCode::CONFLICT, "path/conflict/folder").into_response();
                }
                x.insert(p, "dir".into());
                jsonr(json!({}))
            }
            _ => not_found(),
        }
    })
    .await;
    let cfg = oauth_drive::DriveConfig { folder: "Backups/pc".into(), api_base: base.clone(), content_base: base.clone(), ..Default::default() };
    let store = oauth_drive::DriveStore::new(oauth_drive::Kind::Dropbox, cfg, stale_secret(&base)).unwrap();
    exercise(&store, 300_000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn google_drive_mock() {
    let (base, _st) = serve(|st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = u.path().to_string();
        if path == "/oauth2/token" {
            return token_endpoint(&st, &b);
        }
        if !bearer_ok(&st, &h) {
            return unauthorized();
        }
        // extra: id -> "parent\u{0}name\u{0}kind"; files: id -> data
        let new_id = || format!("id{}", uuid::Uuid::new_v4().simple());
        match (m.as_str(), path.as_str()) {
            ("GET", "/drive/v3/files") => {
                let qq = q(&u, "q").unwrap_or_default();
                let parent = qq.split('\'').nth(1).unwrap_or("").to_string();
                let x = st.extra.lock().unwrap();
                let f = st.files.lock().unwrap();
                let files: Vec<Value> = x
                    .iter()
                    .filter_map(|(id, meta)| {
                        let p: Vec<&str> = meta.split('\0').collect();
                        (p[0] == parent).then(|| {
                            let folder = p[2] == "folder";
                            json!({ "id": id, "name": p[1], "mimeType": if folder { "application/vnd.google-apps.folder" } else { "application/octet-stream" }, "size": f.get(id).map(|d| d.len().to_string()) })
                        })
                    })
                    .collect();
                jsonr(json!({ "files": files }))
            }
            ("POST", "/drive/v3/files") => {
                let v: Value = serde_json::from_slice(&b).unwrap();
                let id = new_id();
                st.extra.lock().unwrap().insert(id.clone(), format!("{}\0{}\0folder", v["parents"][0].as_str().unwrap(), v["name"].as_str().unwrap()));
                jsonr(json!({ "id": id }))
            }
            ("POST", "/upload/drive/v3/files") => {
                // multipart/related: JSON metadata part, then the data part.
                let ct = h.get("content-type").unwrap().to_str().unwrap().to_string();
                let boundary = ct.split("boundary=").nth(1).unwrap().to_string();
                let delim = format!("--{boundary}");
                let text = b.to_vec();
                let s = String::from_utf8_lossy(&text).into_owned();
                let parts: Vec<&str> = s.split(&delim).collect();
                let meta: Value = serde_json::from_str(parts[1].split("\r\n\r\n").nth(1).unwrap().trim()).unwrap();
                // Locate the binary part precisely in bytes.
                let marker = b"Content-Type: application/octet-stream\r\n\r\n";
                let hdr_end = find(&text, marker).unwrap() + marker.len();
                let end = rfind(&text, format!("\r\n{delim}--").as_bytes()).unwrap();
                let id = new_id();
                st.files.lock().unwrap().insert(id.clone(), text[hdr_end..end].to_vec());
                st.extra.lock().unwrap().insert(id.clone(), format!("{}\0{}\0file", meta["parents"][0].as_str().unwrap(), meta["name"].as_str().unwrap()));
                jsonr(json!({ "id": id }))
            }
            ("PATCH", p) if p.starts_with("/upload/drive/v3/files/") => {
                st.files.lock().unwrap().insert(p.rsplit('/').next().unwrap().to_string(), b.to_vec());
                jsonr(json!({}))
            }
            ("GET", p) if p.starts_with("/drive/v3/files/") => st.files.lock().unwrap().get(p.rsplit('/').next().unwrap()).cloned().map(|d| d.into_response()).unwrap_or_else(not_found),
            ("DELETE", p) if p.starts_with("/drive/v3/files/") => {
                let id = p.rsplit('/').next().unwrap().to_string();
                st.files.lock().unwrap().remove(&id);
                st.extra.lock().unwrap().remove(&id);
                StatusCode::NO_CONTENT.into_response()
            }
            _ => not_found(),
        }
    })
    .await;
    let cfg = oauth_drive::DriveConfig { folder: "Backups/pc".into(), api_base: base.clone(), content_base: base.clone(), ..Default::default() };
    let store = oauth_drive::DriveStore::new(oauth_drive::Kind::GoogleDrive, cfg, stale_secret(&base)).unwrap();
    exercise(&store, 300_000).await;
}

/// Regression: a Google Drive rate-limit 403 was a rejected sign-in, so
/// `RetryingBackend` gave up after one request. It now backs off and
/// retries; an authorization 403 is still tried once.
#[tokio::test(flavor = "multi_thread")]
async fn google_drive_rate_limit_403_is_retried_by_the_backend() {
    use ufop_core::backup::backend::Throttle;
    use ufop_core::backup::RetryingBackend;
    use ufop_core::retry::RetryPolicy;
    let (base, st) = serve(|st: St, _m: Method, u: Uri, _h: HeaderMap, _b: Bytes| async move {
        if u.path() != "/drive/v3/files" {
            return not_found();
        }
        let mut x = st.extra.lock().unwrap();
        let n: u32 = x.get("calls").and_then(|c| c.parse().ok()).unwrap_or(0) + 1;
        x.insert("calls".into(), n.to_string());
        let reason = x.get("reason").cloned().unwrap_or_default();
        let throttled = x.get("throttled").and_then(|c| c.parse().ok()).unwrap_or(0u32);
        if n <= throttled {
            let body = json!({ "error": { "code": 403, "message": "m", "errors": [{ "domain": "usageLimits", "reason": reason, "message": "m" }] } });
            return (StatusCode::FORBIDDEN, body.to_string()).into_response();
        }
        jsonr(json!({ "files": [{ "id": "f1", "name": "ufop-a.dblock", "mimeType": "application/octet-stream", "size": "3" }] }))
    })
    .await;
    let policy = RetryPolicy { retries: 3, delay_ms: 1, exponential: false, max_delay_ms: 1 };
    let open = |base: &str| {
        let cfg = oauth_drive::DriveConfig { folder: String::new(), api_base: base.to_string(), content_base: base.to_string(), ..Default::default() };
        let store = oauth_drive::DriveStore::new(oauth_drive::Kind::GoogleDrive, cfg, "fresh".into()).unwrap();
        RetryingBackend::new(StoreBackend::new(Box::new(store), None), policy, Throttle::default())
    };
    let set = |reason: &str, throttled: u32| {
        let mut x = st.extra.lock().unwrap();
        x.insert("calls".into(), "0".into());
        x.insert("reason".into(), reason.into());
        x.insert("throttled".into(), throttled.to_string());
    };
    let calls = || st.extra.lock().unwrap().get("calls").and_then(|c| c.parse::<u32>().ok()).unwrap_or(0);
    // Two throttled replies, then success: three requests, the listing arrives.
    set("rateLimitExceeded", 2);
    let b = open(&base);
    let listed = tokio::task::spawn_blocking(move || b.list()).await.unwrap().unwrap();
    assert_eq!(listed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["ufop-a.dblock"]);
    assert_eq!(calls(), 3);
    // A rejected sign-in is not retried.
    set("insufficientPermissions", 10);
    let b = open(&base);
    let err = tokio::task::spawn_blocking(move || b.list()).await.unwrap().unwrap_err();
    assert!(err.is_auth_failure(), "{err}");
    assert_eq!(calls(), 1);
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn rfind(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).rposition(|w| w == n)
}

#[tokio::test(flavor = "multi_thread")]
async fn box_mock() {
    let (base, _st) = serve(|st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = u.path().to_string();
        if !bearer_ok(&st, &h) {
            return unauthorized();
        }
        match (m.as_str(), path.as_str()) {
            ("GET", p) if p.starts_with("/2.0/folders/") && p.ends_with("/items") => {
                let parent = p.split('/').nth(3).unwrap().to_string();
                let x = st.extra.lock().unwrap();
                let f = st.files.lock().unwrap();
                let e: Vec<Value> = x
                    .iter()
                    .filter_map(|(id, meta)| {
                        let p: Vec<&str> = meta.split('\0').collect();
                        (p[0] == parent).then(|| json!({ "id": id, "name": p[1], "type": p[2], "size": f.get(id).map(|d| d.len()).unwrap_or(0) }))
                    })
                    .collect();
                jsonr(json!({ "entries": e, "total_count": e.len() }))
            }
            ("POST", "/2.0/folders") => {
                let v: Value = serde_json::from_slice(&b).unwrap();
                let id = format!("{}", 1000 + st.extra.lock().unwrap().len());
                st.extra.lock().unwrap().insert(id.clone(), format!("{}\0{}\0folder", v["parent"]["id"].as_str().unwrap(), v["name"].as_str().unwrap()));
                jsonr(json!({ "id": id }))
            }
            ("POST", p) if p.starts_with("/api/2.0/files/") && p.ends_with("/content") => {
                let ct = h.get("content-type").unwrap().to_str().unwrap().to_string();
                let boundary = ct.split("boundary=").nth(1).unwrap().to_string();
                let (attrs, data) = parse_form(&b, &boundary);
                let id = if p == "/api/2.0/files/content" {
                    let id = format!("{}", 5000 + st.extra.lock().unwrap().len());
                    st.extra.lock().unwrap().insert(id.clone(), format!("{}\0{}\0file", attrs["parent"]["id"].as_str().unwrap(), attrs["name"].as_str().unwrap()));
                    id
                } else {
                    p.split('/').nth(4).unwrap().to_string()
                };
                st.files.lock().unwrap().insert(id.clone(), data);
                jsonr(json!({ "entries": [ { "id": id } ] }))
            }
            ("GET", p) if p.starts_with("/2.0/files/") => st.files.lock().unwrap().get(p.split('/').nth(3).unwrap()).cloned().map(|d| d.into_response()).unwrap_or_else(not_found),
            ("DELETE", p) if p.starts_with("/2.0/files/") => {
                let id = p.split('/').nth(3).unwrap().to_string();
                st.files.lock().unwrap().remove(&id);
                st.extra.lock().unwrap().remove(&id);
                StatusCode::NO_CONTENT.into_response()
            }
            _ => not_found(),
        }
    })
    .await;
    let cfg = oauth_drive::DriveConfig { folder: "Backups/pc".into(), api_base: base.clone(), content_base: base.clone(), ..Default::default() };
    let store = oauth_drive::DriveStore::new(oauth_drive::Kind::Box, cfg, "fresh".into()).unwrap();
    exercise(&store, 300_000).await;
}

/// (attributes JSON, file bytes) from a multipart/form-data body.
fn parse_form(b: &[u8], boundary: &str) -> (Value, Vec<u8>) {
    let delim = format!("--{boundary}");
    let mut attrs = Value::Null;
    let mut data = Vec::new();
    let mut pos = 0;
    while let Some(i) = find(&b[pos..], delim.as_bytes()) {
        let start = pos + i + delim.len();
        let Some(next) = find(&b[start..], delim.as_bytes()) else { break };
        let part = &b[start..start + next];
        if let Some(h) = find(part, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&part[..h]).into_owned();
            let body = &part[h + 4..part.len() - 2];
            if head.contains("name=\"attributes\"") {
                attrs = serde_json::from_slice(body).unwrap();
            } else if head.contains("name=\"file\"") {
                data = body.to_vec();
            }
        }
        pos = start + next;
    }
    (attrs, data)
}

#[tokio::test(flavor = "multi_thread")]
async fn pcloud_mock() {
    let (base, _st) = serve(|st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = u.path().to_string();
        let base = st.base.lock().unwrap().clone();
        if path.starts_with("/dl/") {
            let k = decode(&path[3..]);
            return st.files.lock().unwrap().get(&k).cloned().map(|d| d.into_response()).unwrap_or_else(not_found);
        }
        if !bearer_ok(&st, &h) {
            return jsonr(json!({ "result": 2000, "error": "Log in failed." }));
        }
        let p = q(&u, "path").unwrap_or_default();
        match (m.as_str(), path.as_str()) {
            ("GET", "/listfolder") => {
                let pre = format!("{p}/");
                let c: Vec<Value> = st.files.lock().unwrap().iter().filter(|(k, _)| k.starts_with(&pre) && !k[pre.len()..].contains('/')).map(|(k, v)| json!({ "name": &k[pre.len()..], "size": v.len(), "isfolder": false })).collect();
                jsonr(json!({ "result": 0, "metadata": { "contents": c } }))
            }
            ("PUT", "/uploadfile") => {
                st.files.lock().unwrap().insert(format!("{p}/{}", q(&u, "filename").unwrap()), b.to_vec());
                jsonr(json!({ "result": 0 }))
            }
            ("GET", "/getfilelink") => jsonr(json!({ "result": 0, "hosts": [base], "path": format!("/dl{}", p) })),
            ("GET", "/deletefile") => {
                st.files.lock().unwrap().remove(&p);
                jsonr(json!({ "result": 0 }))
            }
            ("GET", "/createfolderifnotexists") => jsonr(json!({ "result": 0 })),
            _ => not_found(),
        }
    })
    .await;
    let cfg = oauth_drive::DriveConfig { folder: "Backups/pc".into(), api_base: base.clone(), ..Default::default() };
    let store = oauth_drive::DriveStore::new(oauth_drive::Kind::Pcloud, cfg.clone(), "fresh".into()).unwrap();
    exercise(&store, 300_000).await;
    let bad = oauth_drive::DriveStore::new(oauth_drive::Kind::Pcloud, cfg, "nope".into()).unwrap();
    assert!(bad.list().await.unwrap_err().user_message().contains("2000"));
}

#[tokio::test(flavor = "multi_thread")]
async fn graph_mock_onedrive_sharepoint_group() {
    let (base, st) = serve(|st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
        let path = decode(u.path());
        let base = st.base.lock().unwrap().clone();
        if path == "/oauth2/token" {
            return token_endpoint(&st, &b);
        }
        if let Some(id) = path.strip_prefix("/upload/") {
            // Pre-authorised upload session URL: append the range.
            let k = st.extra.lock().unwrap().get(&format!("session:{id}")).cloned().unwrap();
            let range = h.get("content-range").unwrap().to_str().unwrap().to_string();
            let (_, rest) = range.split_once(' ').unwrap();
            let (span, total) = rest.split_once('/').unwrap();
            let start: usize = span.split('-').next().unwrap().parse().unwrap();
            let mut f = st.files.lock().unwrap();
            let e = f.entry(k).or_default();
            assert_eq!(e.len(), start, "chunks arrive in order");
            e.extend_from_slice(&b);
            let done = e.len() == total.parse::<usize>().unwrap();
            return if done { (StatusCode::CREATED, "{}").into_response() } else { (StatusCode::ACCEPTED, "{}").into_response() };
        }
        if !bearer_ok(&st, &h) {
            return unauthorized();
        }
        // Drive prefix: /v1.0/me/drive, /v1.0/sites/S/drive, /v1.0/groups/G/drive
        let Some(i) = path.find("/drive/") else { return not_found() };
        st.extra.lock().unwrap().insert("last_drive".into(), path[..i].to_string());
        let rest = &path[i + "/drive".len()..];
        let item = |r: &str| r.trim_start_matches("/root:/").trim_end_matches(':').to_string();
        match m.as_str() {
            "GET" if rest.ends_with(":/children") || rest == "/root/children" => {
                let folder = if rest == "/root/children" { String::new() } else { item(rest.trim_end_matches("/children")) };
                let pre = if folder.is_empty() { String::new() } else { format!("{folder}/") };
                let v: Vec<Value> = st.files.lock().unwrap().iter().filter(|(k, _)| k.starts_with(&pre) && !k[pre.len()..].contains('/')).map(|(k, v)| json!({ "name": &k[pre.len()..], "size": v.len(), "file": {} })).collect();
                jsonr(json!({ "value": v }))
            }
            "POST" if rest.ends_with("/children") => StatusCode::CREATED.into_response(),
            "PUT" if rest.ends_with(":/content") => {
                st.files.lock().unwrap().insert(item(rest.trim_end_matches("/content")), b.to_vec());
                StatusCode::CREATED.into_response()
            }
            "POST" if rest.ends_with(":/createUploadSession") => {
                let k = item(rest.trim_end_matches("/createUploadSession"));
                let id = uuid::Uuid::new_v4().simple().to_string();
                st.files.lock().unwrap().insert(k.clone(), Vec::new());
                st.extra.lock().unwrap().insert(format!("session:{id}"), k);
                jsonr(json!({ "uploadUrl": format!("{base}/upload/{id}") }))
            }
            "GET" if rest.ends_with(":/content") => st.files.lock().unwrap().get(&item(rest.trim_end_matches("/content"))).cloned().map(|d| d.into_response()).unwrap_or_else(not_found),
            "DELETE" => {
                st.files.lock().unwrap().remove(&item(rest));
                StatusCode::NO_CONTENT.into_response()
            }
            _ => not_found(),
        }
    })
    .await;
    for (drive, expect) in [("me", "/v1.0/me"), ("site:contoso.sharepoint.com,1,2", "/v1.0/sites/contoso.sharepoint.com,1,2"), ("group:g-123", "/v1.0/groups/g-123")] {
        let cfg = graph::GraphConfig { drive: drive.into(), folder: "Backups/pc".into(), api_base: format!("{base}/v1.0") };
        let store = graph::GraphStore::new(cfg, stale_secret(&base)).unwrap();
        exercise(&store, 9 << 20).await; // > 4 MiB: upload session in 5 MiB chunks
        assert_eq!(st.extra.lock().unwrap().get("last_drive").map(String::as_str), Some(expect));
    }
    assert!(graph::GraphStore::new(graph::GraphConfig { drive: "bogus:1".into(), ..Default::default() }, "t".into()).unwrap().list().await.is_err());
}

// ── Filen ──────────────────────────────────────────────────────────────────
// Gateway / ingest / egest as documented by filen-sdk-ts (`src/api/v3/**`,
// `src/api/client.ts`): `{status, message, code, data}` envelopes, SHA-512
// `Checksum` headers, chunk hashes, and Filen's encrypted metadata, which the
// mock decrypts and checks the way Filen's apps read it.

const FILEN_SALT: &str = "aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY";
const FILEN_PW: &str = "correct horse battery staple";
/// A master key from before a password change, still held by the account.
const FILEN_OLD_KEY: &str = "olderMasterKey0123456789abcdefgh";

#[derive(Default)]
struct FilenState {
    /// uuid → (parent, encrypted `{"name"}`)
    folders: BTreeMap<String, (String, String)>,
    /// uuid → (row as `dir/content` lists it, listed); unlisted = an old version.
    files: BTreeMap<String, (Value, bool)>,
    chunks: BTreeMap<(String, u64), Vec<u8>>,
    chunk_parent: BTreeMap<String, String>,
    logins: usize,
    trashed: Vec<String>,
}

fn filen_ok(data: Value) -> Response {
    jsonr(json!({ "status": true, "message": "ok", "data": data }))
}

fn filen_fail(code: &str) -> Response {
    jsonr(json!({ "status": false, "message": code, "code": code }))
}

fn filen_dec(meta: &str, mk: &str) -> Option<Value> {
    let plain = filen::crypto::decrypt_meta(meta, mk).or_else(|_| filen::crypto::decrypt_meta(meta, FILEN_OLD_KEY)).ok()?;
    serde_json::from_str(&plain).ok()
}

fn filen_route(fs: &Mutex<FilenState>, mk: &str, derived: &str, m: Method, u: Uri, h: HeaderMap, b: Bytes) -> Response {
    use filen::crypto::*;
    let path = u.path().to_string();
    let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let checksum = h.get("checksum").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    if m == Method::POST && path != "/v3/upload" && checksum != sha512_hex(&b) {
        return (StatusCode::BAD_REQUEST, "Checksum header missing or wrong").into_response();
    }
    let mut g = fs.lock().unwrap();
    match (m.as_str(), path.as_str()) {
        ("POST", "/v3/auth/info") => match body["email"].as_str() {
            Some("me@example.com") => filen_ok(json!({ "email": "me@example.com", "authVersion": 2, "salt": FILEN_SALT, "id": 7 })),
            Some("v3@example.com") => filen_ok(json!({ "email": "v3@example.com", "authVersion": 3, "salt": "ab".repeat(128), "id": 8 })),
            _ => filen_fail("email_or_password_wrong"),
        },
        ("POST", "/v3/login") => {
            if body["password"] != derived || body["authVersion"] != 2 || body["email"] != "me@example.com" {
                return filen_fail("email_or_password_wrong");
            }
            if body["twoFactorCode"] != "XXXXXX" {
                return filen_fail("wrong_2fa");
            }
            g.logins += 1;
            filen_ok(json!({ "apiKey": "key1", "masterKeys": null, "publicKey": null, "privateKey": null, "dek": null }))
        }
        _ if auth != "Bearer key1" => filen_fail("api_key_not_found"),
        ("POST", "/v3/user/masterKeys") => {
            // The client sends its known keys encrypted with the newest one.
            if decrypt_meta(body["masterKeys"].as_str().unwrap_or(""), mk).as_deref() != Ok(mk) {
                return filen_fail("invalid_params");
            }
            filen_ok(json!({ "keys": encrypt_meta(&format!("{FILEN_OLD_KEY}|{mk}"), mk) }))
        }
        ("GET", "/v3/user/baseFolder") => filen_ok(json!({ "uuid": "root" })),
        ("GET", "/v3/user/info") => {
            let used: u64 = g.chunks.values().map(|c| c.len() as u64).sum();
            filen_ok(json!({ "id": 7, "email": "me@example.com", "isPremium": 0, "maxStorage": 10_000_000u64, "storageUsed": used, "baseFolderUUID": "root" }))
        }
        ("POST", "/v3/dir/content") => {
            let p = body["uuid"].as_str().unwrap_or("").to_string();
            let folders: Vec<Value> = g.folders.iter().filter(|(_, (par, _))| *par == p).map(|(id, (par, n))| json!({ "uuid": id, "name": n, "parent": par, "color": null, "timestamp": 1, "favorited": 0, "is_sync": 0, "is_default": 0 })).collect();
            let uploads: Vec<Value> = if body["foldersOnly"] == true { vec![] } else { g.files.values().filter(|(f, listed)| *listed && f["parent"] == p.as_str()).map(|(f, _)| f.clone()).collect() };
            filen_ok(json!({ "uploads": uploads, "folders": folders }))
        }
        ("POST", "/v3/dir/create") => {
            let Some(n) = filen_dec(body["name"].as_str().unwrap_or(""), mk) else { return filen_fail("invalid_name") };
            if body["nameHashed"] != hash_name_v2(n["name"].as_str().unwrap_or("")) {
                return filen_fail("invalid_name_hash");
            }
            let id = body["uuid"].as_str().unwrap_or_default().to_string();
            g.folders.insert(id.clone(), (body["parent"].as_str().unwrap_or_default().into(), body["name"].as_str().unwrap_or_default().into()));
            filen_ok(json!({ "uuid": id }))
        }
        ("POST", "/v3/upload") => {
            let (Some(uuid), Some(idx), Some(parent), Some(uk), Some(hash)) = (q(&u, "uuid"), q(&u, "index"), q(&u, "parent"), q(&u, "uploadKey"), q(&u, "hash")) else { return filen_fail("missing_params") };
            if hash != sha512_hex(&b) {
                return filen_fail("chunk_hash_mismatch");
            }
            let params = format!(r#"{{"uuid":"{uuid}","index":"{idx}","parent":"{parent}","uploadKey":"{uk}","hash":"{hash}"}}"#);
            if checksum != sha512_hex(params.as_bytes()) {
                return filen_fail("checksum_mismatch");
            }
            g.chunk_parent.insert(uuid.clone(), parent);
            g.chunks.insert((uuid, idx.parse().unwrap_or(u64::MAX)), b.to_vec());
            filen_ok(json!({ "bucket": "filen-1", "region": "de-1" }))
        }
        ("POST", "/v3/upload/done") | ("POST", "/v3/upload/empty") => {
            let uuid = body["uuid"].as_str().unwrap_or_default().to_string();
            let Some(meta) = filen_dec(body["metadata"].as_str().unwrap_or(""), mk) else { return filen_fail("invalid_metadata") };
            if ["name", "size", "mime", "key", "lastModified", "creation"].iter().any(|k| meta.get(*k).is_none()) {
                return filen_fail("invalid_metadata");
            }
            let (name, key) = (meta["name"].as_str().unwrap_or_default().to_string(), meta["key"].as_str().unwrap_or_default().to_string());
            // name / size / mime are encrypted with the file key; nameHashed is of the plain name.
            let fk = |f: &str| decrypt_meta(body[f].as_str().unwrap_or(""), &key).ok();
            if fk("name").as_deref() != Some(name.as_str()) || fk("size") != Some(meta["size"].to_string()) || fk("mime") != meta["mime"].as_str().map(String::from) || body["nameHashed"] != hash_name_v2(&name) || body["version"] != 2 {
                return filen_fail("invalid_params");
            }
            let chunks = body["chunks"].as_u64().unwrap_or(0);
            // Read the data back the way Filen's apps do: decrypt every chunk with the file key.
            let mut plain = Vec::new();
            for i in 0..chunks {
                let Some(c) = g.chunks.get(&(uuid.clone(), i)) else { return filen_fail("chunk_missing") };
                match decrypt_data(2, c, &key) {
                    Ok(p) => plain.extend(p),
                    Err(_) => return filen_fail("chunk_undecryptable"),
                }
            }
            if Some(plain.len() as u64) != meta["size"].as_u64() || (chunks > 0 && meta["hash"] != sha512_hex(&plain)) {
                return filen_fail("size_mismatch");
            }
            let parent = if path.ends_with("/empty") { body["parent"].as_str().unwrap_or_default().to_string() } else { g.chunk_parent.get(&uuid).cloned().unwrap_or_default() };
            // Filen file versioning: an existing file of the same name becomes an old version.
            for (f, listed) in g.files.values_mut() {
                if f["parent"] == parent.as_str() && filen_dec(f["metadata"].as_str().unwrap_or(""), mk).is_some_and(|m| m["name"] == name.as_str()) {
                    *listed = false;
                }
            }
            let row = json!({ "uuid": uuid, "metadata": body["metadata"], "rm": body["rm"], "timestamp": 1, "chunks": chunks, "size": plain.len(), "bucket": "filen-1", "region": "de-1", "parent": parent, "version": 2, "favorited": 0 });
            g.files.insert(uuid, (row, true));
            filen_ok(json!({ "chunks": chunks, "size": plain.len() }))
        }
        ("POST", "/v3/file/delete/permanent") | ("POST", "/v3/file/trash") => {
            let id = body["uuid"].as_str().unwrap_or_default().to_string();
            if g.files.remove(&id).is_none() {
                return filen_fail("file_not_found");
            }
            if path.ends_with("/trash") {
                g.trashed.push(id.clone());
            }
            g.chunks.retain(|(u, _), _| *u != id);
            filen_ok(Value::Null)
        }
        ("GET", p) if p.starts_with("/de-1/filen-1/") => {
            let parts: Vec<&str> = p.split('/').collect();
            g.chunks.get(&(parts[3].to_string(), parts[4].parse().unwrap_or(u64::MAX))).cloned().map(|c| c.into_response()).unwrap_or_else(not_found)
        }
        _ => not_found(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn filen_mock_encrypted_roundtrip() {
    let (mk, derived) = filen::crypto::derive_v2(FILEN_PW, FILEN_SALT);
    let fs = Arc::new(Mutex::new(FilenState::default()));
    // "Backups" was made by Filen's app before a password change (older master key).
    fs.lock().unwrap().folders.insert("f-backups".into(), ("root".into(), filen::crypto::encrypt_meta(r#"{"name":"Backups"}"#, FILEN_OLD_KEY)));
    let (fs2, mk2) = (fs.clone(), mk.clone());
    let (base, _st) = serve(move |_st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| {
        let (fs, mk, derived) = (fs2.clone(), mk2.clone(), derived.clone());
        async move { filen_route(&fs, &mk, &derived, m, u, h, b) }
    })
    .await;
    let cfg = filen::FilenConfig { email: "me@example.com".into(), folder: "Backups/pc".into(), gateway_url: base.clone(), ingest_url: base.clone(), egest_url: base.clone(), ..Default::default() };
    // A stored API key that no longer works falls back to the password login.
    let secret = json!({ "password": FILEN_PW, "api_key": "stale" }).to_string();
    let store = filen::FilenStore::connect(cfg.clone(), secret).await.unwrap();
    assert_eq!(fs.lock().unwrap().logins, 1);
    exercise(&store, 2 * filen::CHUNK + 1234).await; // three encrypted 1 MiB chunks
    {
        let g = fs.lock().unwrap();
        // The existing folder was found with the older key and reused.
        assert_eq!(g.folders.values().filter(|(p, _)| p == "root").count(), 1);
        assert_eq!(g.folders.values().filter(|(p, _)| p == "f-backups").count(), 1);
        assert!(g.files.is_empty(), "the overwritten version and both volumes are gone");
    }
    // Stored chunks are ciphertext.
    store.put("ufop-x.dblock", b"plaintext!".repeat(20)).await.unwrap();
    {
        let mut g = fs.lock().unwrap();
        assert!(g.chunks.values().all(|c| !c.windows(10).any(|w| w == b"plaintext!")));
        // A file nobody's key opens (another client's) is ignored.
        let pc = g.folders.iter().find(|(_, (p, _))| p == "f-backups").map(|(id, _)| id.clone()).unwrap();
        g.files.insert("foreign".into(), (json!({ "uuid": "foreign", "metadata": filen::crypto::encrypt_meta(r#"{"name":"x"}"#, "someoneElsesKey0123456789abcdefg"), "chunks": 1, "size": 1, "bucket": "filen-1", "region": "de-1", "parent": pc, "version": 2 }), true));
    }
    assert_eq!(store.list().await.unwrap().iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["ufop-x.dblock"]);
    let quota = store.quota().await.unwrap().unwrap();
    assert_eq!(quota.total, 10_000_000);
    assert!(quota.free < quota.total);
    // Trash instead of permanent deletion.
    let trash = filen::FilenStore::connect(filen::FilenConfig { use_trash: true, ..cfg.clone() }, FILEN_PW.into()).await.unwrap();
    trash.delete("ufop-x.dblock").await.unwrap();
    trash.delete("ufop-x.dblock").await.unwrap(); // already gone
    assert_eq!(fs.lock().unwrap().trashed.len(), 1);
    // Full engine backup + restore through the encrypted store.
    let engine = filen::FilenStore::connect(filen::FilenConfig { folder: "Backups/engine".into(), ..cfg.clone() }, FILEN_PW.into()).await.unwrap();
    engine.create_folder().await.unwrap();
    let backend = Arc::new(StoreBackend::new(Box::new(engine), None));
    tokio::task::spawn_blocking(move || super::tests::backup_roundtrip(backend.as_ref())).await.unwrap();
    // Errors: wrong password, auth version 3, missing folder.
    let e = filen::FilenStore::connect(cfg.clone(), "wrong".into()).await.err().unwrap();
    assert!(e.user_message().contains("email_or_password_wrong"), "{}", e.user_message());
    let e = filen::FilenStore::connect(filen::FilenConfig { email: "v3@example.com".into(), ..cfg.clone() }, FILEN_PW.into()).await.err().unwrap();
    assert!(e.user_message().contains("version 3"), "{}", e.user_message());
    let missing = filen::FilenStore::connect(filen::FilenConfig { folder: "Nope".into(), ..cfg }, FILEN_PW.into()).await.unwrap();
    assert!(missing.list().await.is_err());
    missing.delete("ufop-a.dblock").await.unwrap();
}

/// `name → (filename, bytes)` of a multipart/form-data body.
fn form_fields(b: &[u8], content_type: &str) -> BTreeMap<String, (Option<String>, Vec<u8>)> {
    let boundary = content_type.split("boundary=").nth(1).unwrap_or_default().trim_matches('"');
    let delim = format!("--{boundary}");
    let mut out = BTreeMap::new();
    let mut pos = 0;
    while let Some(i) = find(&b[pos..], delim.as_bytes()) {
        let start = pos + i + delim.len();
        let Some(next) = find(&b[start..], delim.as_bytes()) else { break };
        let part = &b[start..start + next];
        if let Some(hh) = find(part, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&part[..hh]).into_owned();
            let name = head.split("name=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or_default().to_string();
            let filename = head.split("filename=\"").nth(1).and_then(|s| s.split('"').next()).map(String::from);
            out.insert(name, (filename, part[hh + 4..part.len() - 2].to_vec()));
        }
        pos = start + next;
    }
    out
}

// ── Drime Cloud ────────────────────────────────────────────────────────────
// The BeDrive-style API as Duplicati's Drime backend uses it: Laravel pages
// (`current_page`, `last_page`, `total`, `data`), folder-scoped listings by
// folder hash, S3 multipart uploads through pre-signed part URLs. Like the
// live service, the mock renames a same-named upload instead of overwriting,
// lists trashed rows in folders, and does not page past page 20.

#[derive(Clone)]
struct DrimeEntry {
    id: u64,
    name: String,
    kind: &'static str,
    parent: Option<u64>,
    created: String,
    deleted: bool,
    data: Vec<u8>,
}

impl DrimeEntry {
    fn hash(&self) -> String {
        format!("h{}x", self.id)
    }
    fn json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "type": self.kind, "hash": self.hash(), "file_size": self.data.len(),
            "mime": if self.kind == "folder" { Value::Null } else { "application/octet-stream".into() }, "parent_id": self.parent, "workspace_id": 0,
            "created_at": self.created, "updated_at": self.created, "deleted_at": if self.deleted { Value::from("2026-01-02T00:00:00.000000Z") } else { Value::Null },
        })
    }
}

#[derive(Default)]
struct DrimeState {
    entries: Vec<DrimeEntry>,
    next: u64,
    /// key → (filename, size, parent, assembled data)
    uploads: BTreeMap<String, (String, u64, Option<u64>, Vec<u8>)>,
    parts: BTreeMap<(String, u64), Vec<u8>>,
    aborted: usize,
    multipart_parts: usize,
}

impl DrimeState {
    fn add(&mut self, name: &str, kind: &'static str, parent: Option<u64>, data: Vec<u8>) -> DrimeEntry {
        self.next += 1;
        // Two entries per second: equal timestamps exercise the inclusive window boundary.
        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(1_767_225_600 + (self.next / 2) as i64, 0).unwrap();
        let mut name = name.to_string();
        if self.entries.iter().any(|e| e.parent == parent && !e.deleted && e.name == name) {
            name = format!("{name} (1)");
        }
        let e = DrimeEntry { id: self.next, name, kind, parent, created: ts.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(), deleted: false, data };
        self.entries.push(e.clone());
        e
    }
}

fn drime_route(ds: &Mutex<DrimeState>, base: &str, m: Method, u: Uri, h: HeaderMap, b: Bytes) -> Response {
    let path = decode(u.path());
    let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    let mut g = ds.lock().unwrap();
    if path == "/auth/login" {
        return if body["email"] == "me@example.com" && body["password"] == "pw" && body["device_name"].is_string() {
            jsonr(json!({ "status": "success", "user": { "id": 1, "email": "me@example.com", "access_token": "dtok", "banned_at": null } }))
        } else {
            (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(json!({ "message": "These credentials do not match our records." }))).into_response()
        };
    }
    if let Some(rest) = path.strip_prefix("/s3/part/") {
        // Pre-signed URL: an Authorization or Content-Type header breaks the signature.
        if h.contains_key("authorization") || h.contains_key("content-type") || q(&u, "X-Amz-Signature").as_deref() != Some("sig") {
            return (StatusCode::FORBIDDEN, "SignatureDoesNotMatch").into_response();
        }
        let (upload, n) = rest.split_once('/').unwrap();
        g.parts.insert((upload.to_string(), n.parse().unwrap()), b.to_vec());
        use md5::Digest;
        return ([("etag", format!("\"{}\"", hex::encode(md5::Md5::digest(&b))))], "").into_response();
    }
    if h.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer dtok") {
        return unauthorized();
    }
    let ws_ok = |v: &Value| v == &json!(0) || v == "0";
    match (m.as_str(), path.as_str()) {
        ("GET", "/drive/file-entries") => {
            let per: usize = q(&u, "perPage").and_then(|p| p.parse().ok()).unwrap_or(50);
            let page: usize = q(&u, "page").and_then(|p| p.parse().ok()).unwrap_or(1);
            if !ws_ok(&Value::from(q(&u, "workspaceId").unwrap_or_default())) {
                return (StatusCode::FORBIDDEN, "workspace").into_response();
            }
            let mut rows: Vec<DrimeEntry> = match q(&u, "folderId") {
                Some(hash) => {
                    assert_eq!(q(&u, "pageId").as_deref(), Some(hash.as_str()));
                    assert_eq!(q(&u, "orderBy").as_deref(), Some("created_at"));
                    let Some(fid) = g.entries.iter().find(|e| e.hash() == hash).map(|e| e.id) else { return not_found() };
                    let mut r: Vec<DrimeEntry> = g.entries.iter().filter(|e| e.parent == Some(fid)).cloned().collect();
                    if let Some(f) = q(&u, "filters") {
                        // encodeDriveFilters: URI-encoded base64 JSON.
                        use base64::Engine;
                        let json_f: Value = serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(decode(&f)).unwrap()).unwrap();
                        assert_eq!((json_f[0]["key"].as_str(), json_f[0]["operator"].as_str()), (Some("created_at"), Some(">=")));
                        let lower = json_f[0]["value"].as_str().unwrap().to_string();
                        r.retain(|e| e.created >= lower);
                    }
                    r.sort_by(|a, b| (a.created.as_str(), a.id).cmp(&(b.created.as_str(), b.id)));
                    r
                }
                None => {
                    let mut r: Vec<DrimeEntry> = g.entries.iter().filter(|e| e.parent.is_none() && !e.deleted && (q(&u, "type").is_none() || q(&u, "type").as_deref() == Some(e.kind))).cloned().collect();
                    r.sort_by(|a, b| a.name.cmp(&b.name));
                    r
                }
            };
            let total = rows.len();
            let last = total.div_ceil(per).max(1);
            // Drime does not serve pages past 20 reliably: it answers with page 20.
            let served = page.min(20);
            let data: Vec<Value> = rows.drain(..).skip((served - 1) * per).take(per).map(|e| e.json()).collect();
            jsonr(json!({ "current_page": served, "last_page": last, "per_page": per, "total": total, "data": data }))
        }
        ("GET", p) if p.starts_with("/folders/") && p.ends_with("/count") => {
            let id: u64 = p.split('/').nth(2).unwrap().parse().unwrap();
            jsonr(json!({ "count": g.entries.iter().filter(|e| e.parent == Some(id) && !e.deleted).count() }))
        }
        ("POST", "/folders") => {
            if !ws_ok(&Value::from(q(&u, "workspaceId").unwrap_or_default())) {
                return (StatusCode::FORBIDDEN, "workspace").into_response();
            }
            let e = g.add(body["name"].as_str().unwrap(), "folder", body["parentId"].as_u64(), vec![]);
            jsonr(json!({ "status": "success", "folder": e.json() }))
        }
        ("POST", "/uploads") => {
            let f = form_fields(&b, h.get("content-type").unwrap().to_str().unwrap());
            let (Some((Some(filename), data)), Some((_, ws))) = (f.get("file").cloned(), f.get("workspaceId")) else { return (StatusCode::UNPROCESSABLE_ENTITY, "file").into_response() };
            assert_eq!(ws, b"0");
            let parent = f.get("parentId").map(|(_, p)| String::from_utf8_lossy(p).parse::<u64>().unwrap());
            let e = g.add(&filename, "file", parent, data);
            jsonr(json!({ "status": "success", "fileEntry": e.json() }))
        }
        ("POST", "/s3/multipart/create") => {
            assert_eq!((body["mime"].as_str(), body["workspaceId"].as_u64()), (Some("application/octet-stream"), Some(0)));
            let key = format!("uploads/{}", uuid::Uuid::new_v4().simple());
            let up = format!("up{}", g.uploads.len() + 1);
            g.uploads.insert(key.clone(), (body["filename"].as_str().unwrap().into(), body["size"].as_u64().unwrap(), body["parentId"].as_u64(), vec![]));
            jsonr(json!({ "status": "success", "key": key, "uploadId": up, "acl": "private" }))
        }
        ("POST", "/s3/multipart/batch-sign-part-urls") => {
            let up = body["uploadId"].as_str().unwrap();
            let urls: Vec<Value> = body["partNumbers"].as_array().unwrap().iter().map(|n| json!({ "partNumber": n, "url": format!("{base}/s3/part/{up}/{n}?X-Amz-Signature=sig") })).collect();
            jsonr(json!({ "status": "success", "urls": urls }))
        }
        ("POST", "/s3/multipart/complete") => {
            use md5::Digest;
            let (key, up) = (body["key"].as_str().unwrap().to_string(), body["uploadId"].as_str().unwrap().to_string());
            let mut data = Vec::new();
            for (i, p) in body["parts"].as_array().unwrap().iter().enumerate() {
                let n = p["PartNumber"].as_u64().unwrap();
                assert_eq!(n, i as u64 + 1, "parts in order, PascalCase");
                let part = g.parts.get(&(up.clone(), n)).cloned().unwrap();
                if p["ETag"] != format!("\"{}\"", hex::encode(md5::Md5::digest(&part))) {
                    return (StatusCode::BAD_REQUEST, "InvalidPart").into_response();
                }
                data.extend(part);
            }
            g.multipart_parts += body["parts"].as_array().unwrap().len();
            g.uploads.get_mut(&key).unwrap().3 = data;
            jsonr(json!({ "status": "success", "location": key }))
        }
        ("POST", "/s3/entries") => {
            let key = g.uploads.keys().find(|k| k.rsplit('/').next() == body["filename"].as_str()).cloned();
            let Some(key) = key else { return (StatusCode::UNPROCESSABLE_ENTITY, "filename").into_response() };
            let (_, size, parent, data) = g.uploads.remove(&key).unwrap();
            assert_eq!((data.len() as u64, body["size"].as_u64(), body["parentId"].as_u64()), (size, Some(size), parent));
            let e = g.add(body["clientName"].as_str().unwrap(), "file", parent, data);
            jsonr(json!({ "status": "success", "fileEntry": e.json() }))
        }
        ("POST", "/s3/multipart/abort") => {
            g.aborted += 1;
            jsonr(json!({ "status": "success" }))
        }
        ("GET", p) if p.starts_with("/file-entries/download/") => {
            let hash = &p["/file-entries/download/".len()..];
            g.entries.iter().find(|e| e.hash() == hash && !e.deleted).map(|e| e.data.clone().into_response()).unwrap_or_else(not_found)
        }
        ("POST", "/file-entries/delete") => {
            let ids: Vec<u64> = body["entryIds"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
            if ids.iter().any(|id| !g.entries.iter().any(|e| e.id == *id && !e.deleted)) {
                return (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(json!({ "message": "The selected entry ids is invalid.", "errors": { "entryIds": ["The selected entry ids is invalid."] } }))).into_response();
            }
            if body["deleteForever"] == true {
                g.entries.retain(|e| !ids.contains(&e.id));
            } else {
                g.entries.iter_mut().filter(|e| ids.contains(&e.id)).for_each(|e| e.deleted = true);
            }
            jsonr(json!({ "status": "success" }))
        }
        ("GET", "/user/space-usage") => jsonr(json!({ "used": g.entries.iter().map(|e| e.data.len() as u64).sum::<u64>(), "available": 1_000_000_000u64, "status": "success" })),
        _ => not_found(),
    }
}

async fn drime_mock() -> (String, Arc<Mutex<DrimeState>>) {
    let ds = Arc::new(Mutex::new(DrimeState::default()));
    let d2 = ds.clone();
    let (base, _st) = serve(move |st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| {
        let ds = d2.clone();
        async move {
            let base = st.base.lock().unwrap().clone();
            drime_route(&ds, &base, m, u, h, b)
        }
    })
    .await;
    (base, ds)
}

#[tokio::test(flavor = "multi_thread")]
async fn drime_mock_multipart_and_listing() {
    let (base, ds) = drime_mock().await;
    let cfg = drime::DrimeConfig { folder: "Backups/pc".into(), api_url: base.clone(), ..Default::default() };
    let store = drime::DrimeStore::new(cfg.clone(), "dtok".into()).unwrap();
    exercise(&store, 2 * drime::PART + 4321).await; // ≥ 5 MiB: S3 multipart in three parts
    {
        let g = ds.lock().unwrap();
        assert_eq!((g.multipart_parts, g.aborted), (3, 0));
        assert!(!g.entries.iter().any(|e| e.name.ends_with("(1)")), "same-named uploads replaced, not renamed");
    }
    // Small and large in one folder, then quota.
    store.put("ufop-q.dblock", vec![1; 10]).await.unwrap();
    let quota = store.quota().await.unwrap().unwrap();
    assert_eq!((quota.total, quota.free), (1_000_000_000, 1_000_000_000 - 10));
    // E-mail/password sign-in instead of a token; a wrong token is refused.
    let login = drime::DrimeStore::new(cfg.clone(), json!({ "email": "me@example.com", "password": "pw" }).to_string()).unwrap();
    assert_eq!(login.list().await.unwrap().len(), 1);
    assert!(drime::DrimeStore::new(cfg.clone(), "wrong".into()).unwrap().list().await.is_err());
    assert!(drime::DrimeStore::new(cfg.clone(), json!({ "email": "me@example.com", "password": "bad" }).to_string()).unwrap().list().await.is_err());
    assert!(drime::DrimeStore::new(cfg.clone(), "{}".into()).is_err());
    // Engine end to end.
    let engine = drime::DrimeStore::new(drime::DrimeConfig { folder: "Backups/engine".into(), ..cfg.clone() }, "dtok".into()).unwrap();
    engine.create_folder().await.unwrap();
    let backend = Arc::new(StoreBackend::new(Box::new(engine), None));
    tokio::task::spawn_blocking(move || super::tests::backup_roundtrip(backend.as_ref())).await.unwrap();
    // The drive root works too.
    let root = drime::DrimeStore::new(drime::DrimeConfig { folder: String::new(), ..cfg }, "dtok".into()).unwrap();
    root.put("ufop-root.dblock", b"r".to_vec()).await.unwrap();
    assert_eq!(root.list().await.unwrap().iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["ufop-root.dblock"]);
    assert_eq!(root.get("ufop-root.dblock").await.unwrap(), b"r");
}

#[tokio::test(flavor = "multi_thread")]
async fn drime_mock_deep_listing_windows_and_trash() {
    let (base, ds) = drime_mock().await;
    // One entry per page: 23 files need more than Drime's 20 pages, so the
    // listing must restart from the last created_at (inclusive) and dedupe.
    let cfg = drime::DrimeConfig { folder: "deep".into(), page_size: 1, soft_delete: true, api_url: base, ..Default::default() };
    let store = drime::DrimeStore::new(cfg, "dtok".into()).unwrap();
    store.create_folder().await.unwrap();
    for i in 0..23 {
        store.put(&format!("ufop-{i:02}.dblock"), vec![i as u8; 3]).await.unwrap();
    }
    let mut names: Vec<String> = store.list().await.unwrap().into_iter().map(|f| f.name).collect();
    names.sort();
    assert_eq!(names, (0..23).map(|i| format!("ufop-{i:02}.dblock")).collect::<Vec<_>>());
    // Soft delete: the row stays (trashed) but is not listed, and the folder count still adds up.
    store.delete("ufop-05.dblock").await.unwrap();
    store.delete("ufop-05.dblock").await.unwrap(); // already gone
    assert_eq!(ds.lock().unwrap().entries.iter().filter(|e| e.deleted).count(), 1);
    assert_eq!(store.list().await.unwrap().len(), 22);
    assert_eq!(store.get("ufop-22.dblock").await.unwrap(), vec![22u8; 3]);
}

// ── Movistar Cloud ─────────────────────────────────────────────────────────
// The `sapi` web API as Duplicati's Movistar Cloud backend drives it: a
// JSESSIONID + validationkey session (cookies and query parameter), an HTML
// page when the session is gone, asynchronous upload validation.

#[derive(Default)]
struct MovistarState {
    session: (String, String),
    logins: usize,
    folders: Vec<(u64, u64, String)>,
    /// (id, folder, name, data, validation polls so far)
    media: Vec<(u64, u64, String, Vec<u8>, u32)>,
    trash: Vec<u64>,
    next: u64,
}

fn movistar_route(ms: &Mutex<MovistarState>, base: &str, m: Method, u: Uri, h: HeaderMap, b: Bytes) -> Response {
    let path = u.path().to_string();
    let cookie = h.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let action = q(&u, "action").unwrap_or_default();
    let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    let mut g = ms.lock().unwrap();
    if path == "/" {
        return ([("set-cookie", "JSESSIONID=pre; Path=/"), ("content-type", "text/html")], "<!DOCTYPE html><title>Movistar Cloud</title>").into_response();
    }
    if path == "/sapi/login" && action == "login" {
        let form: BTreeMap<String, String> = url::form_urlencoded::parse(&b).into_owned().collect();
        // The home page's cookie must come along, as must the device id.
        if !cookie.contains("JSESSIONID=pre") || h.get("x-deviceid").and_then(|v| v.to_str().ok()) != Some("dev-1") {
            return jsonr(json!({ "error": { "code": "LOG-1", "message": "Device not recognised" } }));
        }
        if form.get("login").map(String::as_str) != Some("me@example.com") || form.get("password").map(String::as_str) != Some("pw") {
            return jsonr(json!({ "error": { "code": "LOG-2", "message": "Invalid credentials" } }));
        }
        g.logins += 1;
        g.session = (format!("js{}", g.logins), format!("vk{}", g.logins));
        return jsonr(json!({ "data": { "jsessionid": g.session.0, "validationkey": g.session.1 } }));
    }
    if let Some(id) = path.strip_prefix("/dl/") {
        let id: u64 = id.parse().unwrap();
        return g.media.iter().find(|x| x.0 == id).map(|x| x.3.clone().into_response()).unwrap_or_else(not_found);
    }
    let (js, vk) = g.session.clone();
    if js.is_empty() || q(&u, "validationkey").as_deref() != Some(vk.as_str()) || !cookie.contains(&format!("JSESSIONID={js}")) || !cookie.contains(&format!("validationkey={vk}")) {
        return ([("content-type", "text/html;charset=UTF-8")], "<!DOCTYPE html><title>Movistar Cloud</title>").into_response();
    }
    match (m.as_str(), path.as_str(), action.as_str()) {
        ("GET", "/sapi/media/folder/root", "get") => jsonr(json!({ "data": { "folders": [{ "id": 1, "name": "root" }] } })),
        ("GET", "/sapi/media/folder", "list") => {
            let parent: u64 = q(&u, "parentid").unwrap().parse().unwrap();
            let f: Vec<Value> = g.folders.iter().filter(|x| x.1 == parent).map(|x| json!({ "id": x.0, "name": x.2, "date": 1_700_000_000_000u64 })).collect();
            jsonr(json!({ "data": { "folders": f } }))
        }
        ("POST", "/sapi/media/folder", "save") => {
            let d = &body["data"];
            assert_eq!((d["magic"].as_bool(), d["offline"].as_bool()), (Some(false), Some(false)));
            g.next += 1;
            let id = 100 + g.next;
            g.folders.push((id, d["parentid"].as_u64().unwrap(), d["name"].as_str().unwrap().into()));
            jsonr(json!({ "data": { "folder": { "id": id, "name": d["name"] } } }))
        }
        ("POST", "/sapi/upload", "save") => {
            assert_eq!(q(&u, "acceptasynchronous").as_deref(), Some("true"));
            let f = form_fields(&b, h.get("content-type").unwrap().to_str().unwrap());
            let meta: Value = serde_json::from_slice(&f["data"].1).unwrap();
            let (filename, data) = f["file"].clone();
            let d = &meta["data"];
            let folder = d["folderid"].as_u64().unwrap();
            if d["name"].as_str() != filename.as_deref() || d["size"].as_u64() != Some(data.len() as u64) || d["contenttype"] != "application/octet-stream" || d["modificationdate"].as_str().map(str::len) != Some(16) || !g.folders.iter().any(|x| x.0 == folder) {
                return jsonr(json!({ "error": { "code": "UPL-1", "message": "bad upload metadata" } }));
            }
            g.next += 1;
            let id = 5000 + g.next;
            // Same name: the service stores another item; the client removes the old one.
            g.media.push((id, folder, d["name"].as_str().unwrap().into(), data, 0));
            jsonr(json!({ "id": id.to_string(), "status": "V", "etag": "e1" }))
        }
        ("POST", "/sapi/media", "get") if q(&u, "origin").as_deref() == Some("omh,dropbox") => {
            let id = body["data"]["ids"][0].as_u64().unwrap();
            jsonr(json!({ "data": { "media": [{ "id": id.to_string(), "url": format!("{base}/dl/{id}?sig=1") }] } }))
        }
        ("POST", "/sapi/media", "get") => {
            let folder: u64 = q(&u, "folderid").unwrap().parse().unwrap();
            let limit: usize = q(&u, "limit").unwrap().parse().unwrap();
            assert_eq!(body["data"]["fields"], json!(["name", "modificationdate", "size", "etag"]));
            let v: Vec<Value> = g.media.iter().filter(|x| x.1 == folder).take(limit).map(|x| json!({ "id": x.0.to_string(), "name": x.2, "size": x.3.len(), "modificationdate": 1_700_000_000_000u64, "etag": "e" })).collect();
            jsonr(json!({ "data": { "media": v } }))
        }
        ("POST", "/sapi/media", "get-validation-status") => {
            let id = body["data"]["ids"][0]["id"].as_u64().unwrap();
            let x = g.media.iter_mut().find(|x| x.0 == id).unwrap();
            x.4 += 1;
            // "ufop-slow" is still being validated on the first poll.
            let status = if x.2.starts_with("ufop-slow") && x.4 < 2 { "V" } else { "U" };
            jsonr(json!({ "data": { "ids": [{ "id": id, "status": status }] } }))
        }
        ("POST", "/sapi/media", "get-storage-space") => {
            let used: u64 = g.media.iter().map(|x| x.3.len() as u64).sum();
            jsonr(json!({ "data": { "used": used, "free": 5_000_000_000u64 - used, "softdeleted": g.trash.len(), "nolimit": false } }))
        }
        ("POST", "/sapi/media/file", "delete") => {
            assert_eq!(q(&u, "softdelete").as_deref(), Some("true"));
            let ids: Vec<u64> = body["data"]["files"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
            g.media.retain(|x| !ids.contains(&x.0));
            g.trash.extend(ids);
            jsonr(json!({ "data": {} }))
        }
        _ => not_found(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn movistar_mock_session_and_validation() {
    let ms = Arc::new(Mutex::new(MovistarState::default()));
    let m2 = ms.clone();
    let (base, _st) = serve(move |st: St, m: Method, u: Uri, h: HeaderMap, b: Bytes| {
        let ms = m2.clone();
        async move {
            let base = st.base.lock().unwrap().clone();
            movistar_route(&ms, &base, m, u, h, b)
        }
    })
    .await;
    let cfg = movistar::MovistarConfig { email: "me@example.com".into(), device_id: "dev-1".into(), folder: "Backups/pc".into(), base_url: base.clone(), upload_url: base.clone(), ..Default::default() };
    let store = movistar::MovistarStore::new(cfg.clone(), "pw".into()).unwrap();
    exercise(&store, 300_000).await;
    assert_eq!(ms.lock().unwrap().logins, 1);
    // An upload still being validated is polled until it is usable.
    let t = std::time::Instant::now();
    store.put("ufop-slow.dblock", b"s".to_vec()).await.unwrap();
    assert!(t.elapsed() >= std::time::Duration::from_secs(1));
    assert!(ms.lock().unwrap().media.iter().any(|x| x.2 == "ufop-slow.dblock" && x.4 == 2));
    // The session ends (HTML page): the store signs in again and carries on.
    ms.lock().unwrap().session = ("gone".into(), "gone".into());
    store.delete("ufop-slow.dblock").await.unwrap();
    assert_eq!(ms.lock().unwrap().logins, 2);
    assert_eq!(ms.lock().unwrap().trash.len(), 4, "overwritten, deleted ×2, slow");
    let quota = store.quota().await.unwrap().unwrap();
    assert_eq!(quota.total, 5_000_000_000);
    assert!(store.put("bad name!.dblock", vec![1]).await.is_err());
    // A listing that reaches the limit may be truncated and is refused.
    let small = movistar::MovistarStore::new(movistar::MovistarConfig { list_limit: 2, ..cfg.clone() }, "pw".into()).unwrap();
    small.put("ufop-1.dblock", vec![1]).await.unwrap();
    assert!(small.put("ufop-2.dblock", vec![2]).await.is_ok());
    assert!(small.list().await.unwrap_err().user_message().contains("list limit"));
    // Engine end to end.
    let engine = movistar::MovistarStore::new(movistar::MovistarConfig { folder: "Backups/engine".into(), ..cfg.clone() }, "pw".into()).unwrap();
    engine.create_folder().await.unwrap();
    let backend = Arc::new(StoreBackend::new(Box::new(engine), None));
    tokio::task::spawn_blocking(move || super::tests::backup_roundtrip(backend.as_ref())).await.unwrap();
    // Wrong password / device id.
    assert!(movistar::MovistarStore::new(cfg.clone(), "nope".into()).unwrap().list().await.unwrap_err().user_message().contains("Invalid credentials"));
    let e = movistar::MovistarStore::new(movistar::MovistarConfig { device_id: "other".into(), ..cfg }, "pw".into()).unwrap().list().await.unwrap_err();
    assert!(e.user_message().contains("Device not recognised"), "{}", e.user_message());
}
