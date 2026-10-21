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
                let v: Vec<Value> = st.files.lock().unwrap().iter().filter(|(k, _)| k.starts_with(&pfx)).map(|(k, v)| json!({ "name": k, "bytes": v.len() })).collect();
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
