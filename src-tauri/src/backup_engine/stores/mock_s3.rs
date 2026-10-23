//! In-process S3 (path-style), STS, EC2 instance metadata (IMDSv2) and ECS
//! container-credential mock. Every S3 and STS `AssumeRole` request's
//! Signature V4 is recomputed with the reference signer and compared; the
//! session token of temporary keys is checked too. Buckets are versioned
//! (puts add versions, deletes add delete markers), so ListObjectVersions
//! and `versionId` reads work for point-in-time tests.

use super::s3::sign_v4;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Version {
    pub id: String,
    /// `None` = delete marker.
    pub data: Option<Vec<u8>>,
    pub modified: DateTime<Utc>,
    pub class: Option<String>,
}

#[derive(Default)]
pub struct S3State {
    /// "bucket/key" → versions, oldest first.
    pub objects: BTreeMap<String, Vec<Version>>,
    pub buckets: Vec<String>,
    /// Access key → (secret, required session token).
    pub keys: HashMap<String, (String, Option<String>)>,
    /// Fixed time for new versions (else now).
    pub now: Option<DateTime<Utc>>,
    /// Keys per listing page.
    pub page: usize,
    /// Lifetime of issued temporary credentials.
    pub ttl_secs: i64,
    /// Accepted OIDC token for AssumeRoleWithWebIdentity.
    pub web_token: String,
    /// Accepted ECS authorization header.
    pub ecs_auth: String,
    pub issued: usize,
    /// (method, path, query) of every request.
    pub log: Vec<(String, String, String)>,
    /// STS form bodies received.
    pub sts: Vec<String>,
    next_version: usize,
}

pub type Shared = Arc<Mutex<S3State>>;

fn xml(body: String) -> Response {
    ([("content-type", "application/xml")], body).into_response()
}

fn err(code: StatusCode, s3code: &str) -> Response {
    (code, format!("<Error><Code>{s3code}</Code><Message>{s3code}</Message></Error>")).into_response()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn params(u: &Uri) -> Vec<(String, String)> {
    url::form_urlencoded::parse(u.query().unwrap_or("").as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn param(p: &[(String, String)], k: &str) -> Option<String> {
    p.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone())
}

/// Recompute the request's SigV4 signature for `service`.
fn verify(st: &S3State, m: &Method, u: &Uri, h: &HeaderMap, body: &[u8], service: &str) -> Result<(), Box<Response>> {
    let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let cred = auth.split("Credential=").nth(1).and_then(|c| c.split(',').next()).unwrap_or("");
    let mut parts = cred.split('/');
    let (ak, _date, region) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let Some((secret, token)) = st.keys.get(ak) else { return Err(Box::new(err(StatusCode::FORBIDDEN, "InvalidAccessKeyId"))) };
    if let Some(t) = token {
        if h.get("x-amz-security-token").and_then(|v| v.to_str().ok()) != Some(t.as_str()) {
            return Err(Box::new(err(StatusCode::FORBIDDEN, "InvalidToken")));
        }
    }
    let signed = auth.split("SignedHeaders=").nth(1).and_then(|c| c.split(',').next()).unwrap_or("");
    let mut headers = BTreeMap::new();
    for k in signed.split(';') {
        let v = if k == "host" { h.get("host") } else { h.get(k) };
        headers.insert(k.to_string(), v.and_then(|v| v.to_str().ok()).unwrap_or("").to_string());
    }
    let payload = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(body));
    if service == "s3" && h.get("x-amz-content-sha256").and_then(|v| v.to_str().ok()) != Some(payload.as_str()) {
        return Err(Box::new(err(StatusCode::BAD_REQUEST, "XAmzContentSHA256Mismatch")));
    }
    let want = sign_v4(m.as_str(), u.path(), &params(u), &headers, &payload, ak, secret, region, service);
    if want != auth {
        return Err(Box::new(err(StatusCode::FORBIDDEN, "SignatureDoesNotMatch")));
    }
    Ok(())
}

fn issue(st: &mut S3State) -> (String, String, String, String) {
    st.issued += 1;
    let n = st.issued;
    let (ak, sk, tok) = (format!("ASIATEMP{n}"), format!("tempsecret{n}"), format!("token{n}"));
    st.keys.insert(ak.clone(), (sk.clone(), Some(tok.clone())));
    let exp = (Utc::now() + chrono::Duration::seconds(st.ttl_secs)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    (ak, sk, tok, exp)
}

fn creds_json(c: (String, String, String, String)) -> Response {
    axum::Json(serde_json::json!({ "Code": "Success", "AccessKeyId": c.0, "SecretAccessKey": c.1, "Token": c.2, "Expiration": c.3 })).into_response()
}

fn route(s: &Shared, m: Method, u: Uri, h: HeaderMap, b: Bytes) -> Response {
    let mut st = s.lock().unwrap();
    st.log.push((m.to_string(), u.path().to_string(), u.query().unwrap_or("").to_string()));
    let path = u.path().to_string();
    // ── Instance metadata (IMDSv2) ──
    if path == "/latest/api/token" && m == Method::PUT {
        if h.get("x-aws-ec2-metadata-token-ttl-seconds").is_none() {
            return StatusCode::BAD_REQUEST.into_response();
        }
        return "imds-session".into_response();
    }
    if let Some(rest) = path.strip_prefix("/latest/meta-data/iam/security-credentials/") {
        if h.get("x-aws-ec2-metadata-token").and_then(|v| v.to_str().ok()) != Some("imds-session") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        return match rest {
            "" => "backup-role\n".into_response(),
            "backup-role" => creds_json(issue(&mut st)),
            _ => StatusCode::NOT_FOUND.into_response(),
        };
    }
    // ── ECS container credentials ──
    if path == "/ecs/creds" {
        if h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("") != st.ecs_auth {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        return creds_json(issue(&mut st));
    }
    // ── STS ──
    if path == "/" && m == Method::POST {
        let form: BTreeMap<String, String> = url::form_urlencoded::parse(&b).into_owned().collect();
        st.sts.push(String::from_utf8_lossy(&b).into_owned());
        let action = form.get("Action").cloned().unwrap_or_default();
        match action.as_str() {
            "AssumeRole" => {
                if let Err(r) = verify(&st, &m, &u, &h, &b, "sts") {
                    return *r;
                }
            }
            "AssumeRoleWithWebIdentity" => {
                if form.get("WebIdentityToken") != Some(&st.web_token) {
                    return err(StatusCode::FORBIDDEN, "InvalidIdentityToken");
                }
            }
            _ => return err(StatusCode::BAD_REQUEST, "InvalidAction"),
        }
        let (ak, sk, tok, exp) = issue(&mut st);
        return xml(format!("<{action}Response><{action}Result><Credentials><AccessKeyId>{ak}</AccessKeyId><SecretAccessKey>{sk}</SecretAccessKey><SessionToken>{tok}</SessionToken><Expiration>{exp}</Expiration></Credentials></{action}Result></{action}Response>"));
    }
    // ── S3 (path style: /bucket[/key]) ──
    if let Err(r) = verify(&st, &m, &u, &h, &b, "s3") {
        return *r;
    }
    let p = params(&u);
    let trimmed = path.trim_start_matches('/');
    let (bucket, key) = match trimmed.split_once('/') {
        Some((b, k)) => (b.to_string(), urlencoding::decode(k).map(|c| c.into_owned()).unwrap_or_default()),
        None => (trimmed.to_string(), String::new()),
    };
    let full = format!("{bucket}/{key}");
    let now = st.now.unwrap_or_else(Utc::now);
    let page = if st.page == 0 { 1000 } else { st.page };
    match (m.as_str(), key.is_empty()) {
        ("HEAD", true) => if st.buckets.contains(&bucket) { StatusCode::OK.into_response() } else { StatusCode::NOT_FOUND.into_response() },
        ("PUT", true) => {
            if !st.buckets.contains(&bucket) {
                st.buckets.push(bucket);
            }
            StatusCode::OK.into_response()
        }
        ("GET", true) if param(&p, "versions").is_some() => {
            let pfx = format!("{bucket}/{}", param(&p, "prefix").unwrap_or_default());
            let (km, vm) = (param(&p, "key-marker").unwrap_or_default(), param(&p, "version-id-marker").unwrap_or_default());
            // (key, version) newest first per key, keys in order.
            let mut all = Vec::new();
            for (k, vs) in st.objects.range(pfx.clone()..).take_while(|(k, _)| k.starts_with(&pfx)) {
                for (i, v) in vs.iter().enumerate().rev() {
                    all.push((k[bucket.len() + 1..].to_string(), v.clone(), i == vs.len() - 1));
                }
            }
            let start = if km.is_empty() { 0 } else { all.iter().position(|(k, v, _)| *k == km && v.id == vm).map_or(all.len(), |i| i + 1) };
            let chunk: Vec<_> = all.iter().skip(start).take(page).collect();
            let mut out = String::from("<ListVersionsResult>");
            for (k, v, latest) in &chunk {
                let tag = if v.data.is_some() { "Version" } else { "DeleteMarker" };
                let size = v.data.as_ref().map_or(String::new(), |d| format!("<Size>{}</Size>", d.len()));
                out.push_str(&format!("<{tag}><Key>{}</Key><VersionId>{}</VersionId><IsLatest>{latest}</IsLatest><LastModified>{}</LastModified>{size}</{tag}>", esc(k), v.id, v.modified.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
            }
            if start + chunk.len() < all.len() {
                let (k, v, _) = chunk.last().unwrap();
                out.push_str(&format!("<IsTruncated>true</IsTruncated><NextKeyMarker>{}</NextKeyMarker><NextVersionIdMarker>{}</NextVersionIdMarker>", esc(k), v.id));
            } else {
                out.push_str("<IsTruncated>false</IsTruncated>");
            }
            xml(out + "</ListVersionsResult>")
        }
        ("GET", true) => {
            let rel = param(&p, "prefix").unwrap_or_default();
            let pfx = format!("{bucket}/{rel}");
            let delim = param(&p, "delimiter").filter(|d| !d.is_empty());
            let mut entries: Vec<(String, Option<usize>)> = Vec::new(); // (key or common prefix, size)
            for (k, vs) in st.objects.range(pfx.clone()..).take_while(|(k, _)| k.starts_with(&pfx)) {
                let Some(d) = vs.last().and_then(|v| v.data.as_ref()) else { continue };
                let rel_key = &k[bucket.len() + 1..];
                match delim.as_deref().and_then(|dl| rel_key[rel.len()..].find(dl).map(|i| rel_key[..rel.len() + i + dl.len()].to_string())) {
                    Some(cp) => {
                        if entries.last().map(|(e, s)| s.is_none() && *e == cp) != Some(true) {
                            entries.push((cp, None));
                        }
                    }
                    None => entries.push((rel_key.to_string(), Some(d.len()))),
                }
            }
            let start: usize = param(&p, "continuation-token").and_then(|t| t.strip_prefix("p&").and_then(|n| n.parse().ok())).unwrap_or(0);
            let chunk: Vec<_> = entries.iter().skip(start).take(page).collect();
            let mut out = String::from("<ListBucketResult>");
            for (k, size) in &chunk {
                match size {
                    Some(n) => out.push_str(&format!("<Contents><Key>{}</Key><Size>{n}</Size></Contents>", esc(k))),
                    None => out.push_str(&format!("<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>", esc(k))),
                }
            }
            if start + chunk.len() < entries.len() {
                out.push_str(&format!("<IsTruncated>true</IsTruncated><NextContinuationToken>p&amp;{}</NextContinuationToken>", start + chunk.len()));
            } else {
                out.push_str("<IsTruncated>false</IsTruncated>");
            }
            xml(out + "</ListBucketResult>")
        }
        ("PUT", false) => {
            if !st.buckets.contains(&bucket) {
                return err(StatusCode::NOT_FOUND, "NoSuchBucket");
            }
            st.next_version += 1;
            let v = Version { id: format!("v{}", st.next_version), data: Some(b.to_vec()), modified: now, class: h.get("x-amz-storage-class").and_then(|v| v.to_str().ok()).map(String::from) };
            st.objects.entry(full).or_default().push(v);
            StatusCode::OK.into_response()
        }
        ("GET" | "HEAD", false) => {
            let vs = st.objects.get(&full);
            let v = match param(&p, "versionId") {
                Some(id) => vs.and_then(|vs| vs.iter().find(|v| v.id == id)),
                None => vs.and_then(|vs| vs.last()),
            };
            match v.and_then(|v| v.data.as_ref().map(|d| (d, v))) {
                Some((d, v)) => {
                    let lm = v.modified.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
                    if m == Method::HEAD {
                        ([("last-modified", lm)], "").into_response()
                    } else {
                        ([("last-modified", lm)], d.clone()).into_response()
                    }
                }
                None => err(StatusCode::NOT_FOUND, "NoSuchKey"),
            }
        }
        ("DELETE", false) => {
            st.next_version += 1;
            let id = format!("v{}", st.next_version);
            if let Some(vs) = st.objects.get_mut(&full) {
                vs.push(Version { id, data: None, modified: now, class: None });
            }
            StatusCode::NO_CONTENT.into_response()
        }
        _ => err(StatusCode::NOT_IMPLEMENTED, "NotImplemented"),
    }
}

/// Start the mock; the access key `AKIDBASE` / `basesecret` is accepted.
pub async fn serve() -> (String, Shared) {
    let s: Shared = Arc::new(Mutex::new(S3State { ttl_secs: 3600, web_token: "oidc-token".into(), ecs_auth: "ecs-secret".into(), ..Default::default() }));
    s.lock().unwrap().keys.insert("AKIDBASE".into(), ("basesecret".into(), None));
    let s2 = s.clone();
    let app = axum::Router::new()
        .fallback(move |m: Method, u: Uri, h: HeaderMap, b: Bytes| {
            let s = s2.clone();
            async move { route(&s, m, u, h, b) }
        })
        .layer(axum::extract::DefaultBodyLimit::disable());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, s)
}
