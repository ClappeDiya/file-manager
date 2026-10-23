//! In-process Azure Blob + Microsoft Entra ID mock. Blob requests are
//! accepted with a Shared Key signature (rebuilt from the received request
//! by the connector tests' independent string-to-sign) or with a bearer
//! token this mock issued (and `x-ms-version` ≥ 2017-11-09, as Azure
//! requires for OAuth). The token endpoint checks the client secret, the
//! federated assertion, or the RS256 certificate assertion's signature,
//! thumbprint and audience; managed-identity endpoints check their headers.

use crate::connectors::azure_rest::{DEV_ACCOUNT, DEV_KEY};
use crate::connectors::azure_tests::expected_signature;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const TENANT: &str = "tenant-x";
pub const CLIENT: &str = "app-123";
pub const CLIENT_SECRET: &str = "sp-secret";
pub const FEDERATED: &str = "federated-jwt";
pub const IDENTITY_HEADER: &str = "id-header";

#[derive(Default)]
pub struct AzState {
    pub base: String,
    /// "container/blob" → (data, access tier).
    pub blobs: BTreeMap<String, (Vec<u8>, Option<String>)>,
    pub containers: Vec<String>,
    blocks: BTreeMap<String, Vec<u8>>,
    /// Tokens issued and accepted.
    pub tokens: Vec<String>,
    /// Token lifetime handed out.
    pub ttl: u64,
    /// (grant kind, scope or resource) of each token request.
    pub token_requests: Vec<(String, String)>,
    pub page: usize,
    /// (method, path, query) of blob requests and how they authenticated.
    pub log: Vec<(String, String, String, &'static str)>,
}

pub type Shared = Arc<Mutex<AzState>>;

fn status(code: StatusCode, e: &str) -> Response {
    (code, format!("<?xml version=\"1.0\"?><Error><Code>{e}</Code><Message>{e}</Message></Error>")).into_response()
}

fn b64url(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).unwrap_or_default()
}

fn test_cert() -> (Vec<u8>, Vec<u8>) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/backup_engine/stores/testdata");
    let key_pem = std::fs::read(format!("{dir}/test-rsa-key.pem")).unwrap();
    let cert_pem = std::fs::read(format!("{dir}/test-rsa-cert.pem")).unwrap();
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice()).unwrap().unwrap();
    let kp = ring::signature::RsaKeyPair::from_pkcs8(key.secret_der()).unwrap();
    let cert = rustls_pemfile::certs(&mut cert_pem.as_slice()).next().unwrap().unwrap().to_vec();
    (kp.public().as_ref().to_vec(), cert)
}

/// Verify a certificate client assertion: signature, x5t thumbprint, audience.
fn check_assertion(jwt: &str, aud: &str) -> bool {
    use sha1::Digest;
    let (public, cert) = test_cert();
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let header: serde_json::Value = serde_json::from_slice(&b64url(parts[0])).unwrap_or_default();
    let claims: serde_json::Value = serde_json::from_slice(&b64url(parts[1])).unwrap_or_default();
    let thumb = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha1::Sha1::digest(&cert));
    let key = ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, public);
    header["alg"] == "RS256"
        && header["x5t"].as_str() == Some(thumb.as_str())
        && claims["aud"].as_str() == Some(aud)
        && claims["iss"].as_str() == Some(CLIENT)
        && claims["exp"].as_i64().unwrap_or(0) > chrono::Utc::now().timestamp()
        && key.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &b64url(parts[2])).is_ok()
}

fn issue(st: &mut AzState, kind: &str, scope: &str) -> Response {
    let t = format!("aad-{}", st.tokens.len() + 1);
    st.tokens.push(t.clone());
    st.token_requests.push((kind.to_string(), scope.to_string()));
    axum::Json(serde_json::json!({ "token_type": "Bearer", "access_token": t, "expires_in": st.ttl })).into_response()
}

fn route(s: &Shared, m: Method, u: Uri, h: HeaderMap, b: Bytes) -> Response {
    let mut st = s.lock().unwrap();
    let path = u.path().to_string();
    let q: BTreeMap<String, String> = url::form_urlencoded::parse(u.query().unwrap_or("").as_bytes()).into_owned().collect();
    let hv = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    // ── Entra ID token endpoint ──
    if path == format!("/{TENANT}/oauth2/v2.0/token") && m == Method::POST {
        let f: BTreeMap<String, String> = url::form_urlencoded::parse(&b).into_owned().collect();
        let scope = f.get("scope").cloned().unwrap_or_default();
        if f.get("grant_type").map(String::as_str) != Some("client_credentials") || f.get("client_id").map(String::as_str) != Some(CLIENT) {
            return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": "invalid_client", "error_description": "AADSTS700016: unknown application" }))).into_response();
        }
        let aud = format!("{}{path}", st.base);
        let kind = match (f.get("client_secret"), f.get("client_assertion")) {
            (Some(sec), None) if sec == CLIENT_SECRET => "secret",
            (None, Some(a)) if f.get("client_assertion_type").map(String::as_str) == Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer") && a == FEDERATED => "federated",
            (None, Some(a)) if check_assertion(a, &aud) => "certificate",
            _ => return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "invalid_client", "error_description": "AADSTS7000215: Invalid client secret provided." }))).into_response(),
        };
        return issue(&mut st, kind, &scope);
    }
    // ── Managed identity: App Service style and instance metadata ──
    if path == "/msi" {
        if hv("x-identity-header") != IDENTITY_HEADER || q.get("api-version").map(String::as_str) != Some("2019-08-01") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let r = q.get("resource").cloned().unwrap_or_default();
        return issue(&mut st, "managed-app-service", &r);
    }
    if path == "/metadata/identity/oauth2/token" {
        if hv("metadata") != "true" {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let r = format!("{}{}", q.get("resource").cloned().unwrap_or_default(), q.get("client_id").map(|c| format!("#{c}")).unwrap_or_default());
        return issue(&mut st, "managed-imds", &r);
    }
    // ── Blob service (path style: /<account>/<container>[/<blob>]) ──
    let Some(rel) = path.strip_prefix(&format!("/{DEV_ACCOUNT}")) else { return status(StatusCode::NOT_FOUND, "InvalidUri") };
    let auth = hv("authorization");
    let how: &'static str = if let Some(tok) = auth.strip_prefix("Bearer ") {
        if !st.tokens.iter().any(|t| t == tok) {
            return status(StatusCode::UNAUTHORIZED, "InvalidAuthenticationInfo");
        }
        if hv("x-ms-version").as_str() < "2017-11-09" {
            return status(StatusCode::BAD_REQUEST, "AuthenticationFailed");
        }
        "bearer"
    } else {
        let key = base64::engine::general_purpose::STANDARD.decode(DEV_KEY).unwrap();
        if auth != expected_signature(&key, &m, &path, u.query().unwrap_or(""), &h) {
            return status(StatusCode::FORBIDDEN, "AuthenticationFailed");
        }
        "shared_key"
    };
    st.log.push((m.to_string(), path.clone(), u.query().unwrap_or("").to_string(), how));
    let segs: Vec<String> = rel.trim_start_matches('/').splitn(2, '/').map(|s| urlencoding::decode(s).map(|c| c.into_owned()).unwrap_or_default()).collect();
    let container = segs.first().cloned().unwrap_or_default();
    let blob = segs.get(1).cloned().unwrap_or_default();
    let full = format!("{container}/{blob}");
    let page = if st.page == 0 { 5000 } else { st.page };
    match (m.as_str(), blob.is_empty()) {
        ("PUT", true) => {
            if st.containers.contains(&container) {
                return status(StatusCode::CONFLICT, "ContainerAlreadyExists");
            }
            st.containers.push(container);
            StatusCode::CREATED.into_response()
        }
        ("GET", true) if q.get("comp").map(String::as_str) == Some("list") => {
            let pfx = q.get("prefix").cloned().unwrap_or_default();
            let delim = q.get("delimiter").cloned().filter(|d| !d.is_empty());
            let mut entries: Vec<(String, Option<usize>)> = Vec::new();
            let start_key = format!("{container}/{pfx}");
            for (k, (d, _)) in st.blobs.range(start_key.clone()..).take_while(|(k, _)| k.starts_with(&start_key)) {
                let name = &k[container.len() + 1..];
                match delim.as_deref().and_then(|dl| name[pfx.len()..].find(dl).map(|i| name[..pfx.len() + i + dl.len()].to_string())) {
                    Some(p) => {
                        if entries.last().map(|(e, s)| s.is_none() && *e == p) != Some(true) {
                            entries.push((p, None));
                        }
                    }
                    None => entries.push((name.to_string(), Some(d.len()))),
                }
            }
            let start: usize = q.get("marker").and_then(|m| m.strip_prefix("m").and_then(|n| n.parse().ok())).unwrap_or(0);
            let chunk: Vec<_> = entries.iter().skip(start).take(page).collect();
            let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><EnumerationResults><Blobs>");
            for (n, size) in &chunk {
                let n = n.replace('&', "&amp;");
                match size {
                    Some(sz) => out.push_str(&format!("<Blob><Name>{n}</Name><Properties><Content-Length>{sz}</Content-Length></Properties></Blob>")),
                    None => out.push_str(&format!("<BlobPrefix><Name>{n}</Name></BlobPrefix>")),
                }
            }
            out.push_str("</Blobs>");
            if start + chunk.len() < entries.len() {
                out.push_str(&format!("<NextMarker>m{}</NextMarker>", start + chunk.len()));
            } else {
                out.push_str("<NextMarker />");
            }
            (StatusCode::OK, out + "</EnumerationResults>").into_response()
        }
        ("PUT", false) if q.get("comp").map(String::as_str) == Some("block") => {
            st.blocks.insert(format!("{full}#{}", q.get("blockid").cloned().unwrap_or_default()), b.to_vec());
            StatusCode::CREATED.into_response()
        }
        ("PUT", false) if q.get("comp").map(String::as_str) == Some("blocklist") => {
            let body = String::from_utf8_lossy(&b).into_owned();
            let mut data = Vec::new();
            for id in crate::backup_engine::stores::http::xml_all(&body, "Latest") {
                match st.blocks.remove(&format!("{full}#{id}")) {
                    Some(d) => data.extend(d),
                    None => return status(StatusCode::BAD_REQUEST, "InvalidBlockList"),
                }
            }
            let tier = h.get("x-ms-access-tier").and_then(|v| v.to_str().ok()).map(String::from);
            st.blobs.insert(full, (data, tier));
            StatusCode::CREATED.into_response()
        }
        ("PUT", false) => {
            if !st.containers.contains(&container) {
                return status(StatusCode::NOT_FOUND, "ContainerNotFound");
            }
            let tier = h.get("x-ms-access-tier").and_then(|v| v.to_str().ok()).map(String::from);
            st.blobs.insert(full, (b.to_vec(), tier));
            StatusCode::CREATED.into_response()
        }
        ("GET", false) => match st.blobs.get(&full) {
            Some((d, _)) => d.clone().into_response(),
            None => status(StatusCode::NOT_FOUND, "BlobNotFound"),
        },
        ("DELETE", false) => match st.blobs.remove(&full) {
            Some(_) => StatusCode::ACCEPTED.into_response(),
            None => status(StatusCode::NOT_FOUND, "BlobNotFound"),
        },
        _ => status(StatusCode::BAD_REQUEST, "UnsupportedRequest"),
    }
}

/// Start the mock; returns (base URL, blob endpoint, state).
pub async fn serve() -> (String, String, Shared) {
    let s: Shared = Arc::new(Mutex::new(AzState { ttl: 3600, ..Default::default() }));
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
    (base.clone(), format!("{base}/{DEV_ACCOUNT}"), s)
}
