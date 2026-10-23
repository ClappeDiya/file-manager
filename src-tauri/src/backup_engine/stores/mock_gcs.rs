//! In-process Google Cloud Storage JSON API + OAuth token endpoint mock.
//! The token endpoint verifies the service-account JWT (RS256 signature
//! with the test key, audience, issuer) and records the requested scope;
//! storage calls need a token it issued, and writes need a token whose
//! scope allows writing (as GCS enforces).

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const SA_EMAIL: &str = "backup@proj.iam.gserviceaccount.com";

#[derive(Default)]
pub struct GcsState {
    pub base: String,
    /// "bucket/object" → (data, storage class).
    pub objects: BTreeMap<String, (Vec<u8>, Option<String>)>,
    /// token → scope.
    pub tokens: BTreeMap<String, String>,
    pub scopes: Vec<String>,
}

pub type Shared = Arc<Mutex<GcsState>>;

/// Service-account key JSON for the test key, pointed at the mock.
pub fn service_account(base: &str) -> String {
    let key = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backup_engine/stores/testdata/test-rsa-key.pem")).unwrap();
    serde_json::json!({ "type": "service_account", "client_email": SA_EMAIL, "private_key": key, "token_uri": format!("{base}/token") }).to_string()
}

fn b64url(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).unwrap_or_default()
}

fn jwt_ok(jwt: &str, aud: &str) -> Option<String> {
    let pem = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backup_engine/stores/testdata/test-rsa-key.pem")).ok()?;
    let key = rustls_pemfile::private_key(&mut pem.as_slice()).ok()??;
    let public = ring::signature::RsaKeyPair::from_pkcs8(key.secret_der()).ok()?.public().as_ref().to_vec();
    let p: Vec<&str> = jwt.split('.').collect();
    let claims: serde_json::Value = serde_json::from_slice(&b64url(p.get(1)?)).ok()?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, public).verify(format!("{}.{}", p[0], p[1]).as_bytes(), &b64url(p.get(2)?)).ok()?;
    (claims["aud"] == aud && claims["iss"] == SA_EMAIL).then(|| claims["scope"].as_str().unwrap_or_default().to_string())
}

fn json(v: serde_json::Value) -> Response {
    axum::Json(v).into_response()
}

fn route(s: &Shared, m: Method, u: Uri, h: HeaderMap, b: Bytes) -> Response {
    let mut st = s.lock().unwrap();
    let path = u.path().to_string();
    let q: BTreeMap<String, String> = url::form_urlencoded::parse(u.query().unwrap_or("").as_bytes()).into_owned().collect();
    if path == "/token" {
        let f: BTreeMap<String, String> = url::form_urlencoded::parse(&b).into_owned().collect();
        let aud = format!("{}/token", st.base);
        let Some(scope) = f.get("assertion").and_then(|a| jwt_ok(a, &aud)).filter(|_| f.get("grant_type").map(String::as_str) == Some("urn:ietf:params:oauth:grant-type:jwt-bearer")) else {
            return (StatusCode::BAD_REQUEST, json(serde_json::json!({ "error": "invalid_grant" }))).into_response();
        };
        let t = format!("ya29.{}", st.tokens.len() + 1);
        st.tokens.insert(t.clone(), scope.clone());
        st.scopes.push(scope);
        return json(serde_json::json!({ "access_token": t, "expires_in": 3599, "token_type": "Bearer" }));
    }
    let tok = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|a| a.strip_prefix("Bearer ")).unwrap_or("");
    let Some(scope) = st.tokens.get(tok).cloned() else { return (StatusCode::UNAUTHORIZED, "invalid token").into_response() };
    let writes = m != Method::GET && m != Method::HEAD;
    if writes && !scope.ends_with("full_control") && !scope.ends_with("read_write") {
        return (StatusCode::FORBIDDEN, json(serde_json::json!({ "error": { "code": 403, "message": "Insufficient Permission" } }))).into_response();
    }
    let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (m.as_str(), seg.as_slice()) {
        ("GET", ["storage", "v1", "b", _bucket]) => json(serde_json::json!({ "name": seg[3] })),
        ("GET", ["storage", "v1", "b", bucket, "o"]) => {
            let pfx = q.get("prefix").cloned().unwrap_or_default();
            let delim = q.contains_key("delimiter");
            let start = format!("{bucket}/{pfx}");
            let (mut items, mut prefixes) = (Vec::new(), Vec::<String>::new());
            for (k, (d, _)) in st.objects.range(start.clone()..).take_while(|(k, _)| k.starts_with(&start)) {
                let name = &k[bucket.len() + 1..];
                match name[pfx.len()..].find('/').filter(|_| delim) {
                    Some(i) => {
                        let p = name[..pfx.len() + i + 1].to_string();
                        if !prefixes.contains(&p) {
                            prefixes.push(p);
                        }
                    }
                    None => items.push(serde_json::json!({ "name": name, "size": d.len().to_string() })),
                }
            }
            json(serde_json::json!({ "items": items, "prefixes": prefixes }))
        }
        ("POST", ["upload", "storage", "v1", "b", bucket, "o"]) => {
            let ct = h.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
            let Some(boundary) = ct.split("boundary=").nth(1) else { return StatusCode::BAD_REQUEST.into_response() };
            let sep = format!("--{boundary}");
            let text = b.to_vec();
            let find = |hay: &[u8], n: &[u8], from: usize| hay[from..].windows(n.len()).position(|w| w == n).map(|i| i + from);
            let (Some(m1), Some(d0)) = (find(&text, b"\r\n\r\n", 0), find(&text, sep.as_bytes(), 10)) else { return StatusCode::BAD_REQUEST.into_response() };
            let meta: serde_json::Value = serde_json::from_slice(text[m1 + 4..d0].trim_ascii()).unwrap_or_default();
            let Some(p0) = find(&text, b"\r\n\r\n", d0) else { return StatusCode::BAD_REQUEST.into_response() };
            let Some(end) = find(&text, format!("\r\n{sep}--").as_bytes(), p0) else { return StatusCode::BAD_REQUEST.into_response() };
            let data = text[p0 + 4..end].to_vec();
            if let Some(c) = meta["crc32c"].as_str() {
                if c != base64::engine::general_purpose::STANDARD.encode(super::gcs::crc32c(&data).to_be_bytes()) {
                    return (StatusCode::BAD_REQUEST, "crc32c mismatch").into_response();
                }
            }
            let name = meta["name"].as_str().unwrap_or_default().to_string();
            st.objects.insert(format!("{bucket}/{name}"), (data, meta["storageClass"].as_str().map(String::from)));
            json(serde_json::json!({ "name": name }))
        }
        ("GET" | "DELETE", ["storage", "v1", "b", bucket, "o", obj]) => {
            let key = format!("{bucket}/{}", urlencoding::decode(obj).map(|c| c.into_owned()).unwrap_or_default());
            if m == Method::DELETE {
                return match st.objects.remove(&key) {
                    Some(_) => StatusCode::NO_CONTENT.into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                };
            }
            match st.objects.get(&key) {
                Some((d, _)) => {
                    let hash = format!("crc32c={}", base64::engine::general_purpose::STANDARD.encode(super::gcs::crc32c(d).to_be_bytes()));
                    ([("x-goog-hash", hash)], d.clone()).into_response()
                }
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
        _ => StatusCode::NOT_IMPLEMENTED.into_response(),
    }
}

pub async fn serve() -> (String, Shared) {
    let s: Shared = Arc::new(Mutex::new(GcsState::default()));
    let s2 = s.clone();
    let app = axum::Router::new()
        .fallback(move |m: Method, u: Uri, h: HeaderMap, b: Bytes| {
            let s = s2.clone();
            async move { route(&s, m, u, h, b) }
        })
        .layer(axum::extract::DefaultBodyLimit::disable());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    s.lock().unwrap().base = base.clone();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, s)
}
