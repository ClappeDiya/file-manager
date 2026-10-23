//! Read-only WebDAV (class 1) at `/dav/<root>/…` for the folders given with
//! `--dav-root`; off when none are given.
//!
//! - Methods: `OPTIONS`, `PROPFIND` (depth 0 or 1; infinity is refused as
//!   RFC 4918 allows), `GET` / `HEAD` with single byte ranges, and an HTML
//!   index for folders. Every method that would change something is
//!   answered `405 Method Not Allowed`.
//! - Authentication: HTTP Basic with the server password or an API token
//!   (`ufop_pat_…`, any scope) as the password, or `Authorization: Bearer`.
//!   Failures get `401` with a Basic challenge; password checks go through
//!   the login throttle and successes are remembered for a minute, because
//!   WebDAV clients send credentials with every request.
//! - Network: the server-wide `--allowed-cidrs` applies, and
//!   `--dav-allowed-cidrs` narrows WebDAV further.
//! - Paths are confined to the roots after resolving symlinks
//!   (`ufop_core::confine`); dot-files are neither listed nor served.

use super::{api_tokens, oauth::html_escape, ServerState};
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use base64::Engine;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use ufop_core::confine::Roots;

pub const PREFIX: &str = "/dav";
const ALLOW: &str = "OPTIONS, GET, HEAD, PROPFIND";
const MAX_LISTING: usize = 10_000;
/// Characters escaped in hrefs (path segments).
const SEGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}').add(b'/').add(b'\\').add(b'&').add(b'\'');

pub fn routes() -> Router<ServerState> {
    Router::new().route(PREFIX, any(handle)).route(&format!("{PREFIX}/"), any(handle)).route(&format!("{PREFIX}/{{*path}}"), any(handle))
}

fn plain(status: StatusCode, msg: &str) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], msg.to_string()).into_response()
}

fn not_allowed() -> Response {
    let mut r = plain(StatusCode::METHOD_NOT_ALLOWED, "This WebDAV share is read-only.");
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static(ALLOW));
    r
}

fn challenge() -> Response {
    let mut r = plain(StatusCode::UNAUTHORIZED, "Sign in with the server password or an API token.");
    r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"ufop WebDAV\", charset=\"UTF-8\""));
    r
}

async fn authenticate(st: &ServerState, headers: &HeaderMap) -> Result<(), Response> {
    let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
    let secret = if let Some(b) = auth.strip_prefix("Basic ") {
        let raw = base64::engine::general_purpose::STANDARD.decode(b.trim()).map_err(|_| challenge())?;
        let s = String::from_utf8(raw).map_err(|_| challenge())?;
        s.split_once(':').map(|(_, p)| p.to_string()).ok_or_else(challenge)?
    } else if let Some(t) = auth.strip_prefix("Bearer ") {
        t.trim().to_string()
    } else {
        return Err(challenge());
    };
    if secret.is_empty() {
        return Err(challenge());
    }
    if secret.starts_with(api_tokens::PREFIX) {
        // Cheap (a hash lookup) and revocation must apply at once: never cached.
        return st.auth.api_tokens().validate(&secret).await.map(|_| ()).map_err(|_| challenge());
    }
    if st.auth.validate(&secret).await.is_ok() {
        return Ok(());
    }
    match st.auth.login_remembered(&secret).await {
        Ok(Some(true)) => Ok(()),
        Ok(None) => Err(plain(StatusCode::TOO_MANY_REQUESTS, "Too many sign-in attempts; try again shortly.")),
        _ => Err(challenge()),
    }
}

/// A request path below `/dav`, decoded: (root name, relative segments).
fn parse(path: &str) -> Option<(Option<String>, Vec<String>)> {
    let rest = path.strip_prefix(PREFIX)?;
    let mut segs = Vec::new();
    for s in rest.split('/').filter(|s| !s.is_empty()) {
        let d = percent_decode_str(s).decode_utf8().ok()?.into_owned();
        if d == "." || d == ".." || d.starts_with('.') || d.contains(['/', '\\', '\0']) {
            return None;
        }
        segs.push(d);
    }
    if segs.is_empty() {
        return Some((None, segs));
    }
    let root = segs.remove(0);
    Some((Some(root), segs))
}

fn href(root: &str, segs: &[String], dir: bool) -> String {
    let mut h = format!("{PREFIX}/{}", utf8_percent_encode(root, SEGMENT));
    for s in segs {
        h.push('/');
        h.push_str(&utf8_percent_encode(s, SEGMENT).to_string());
    }
    if dir {
        h.push('/');
    }
    h
}

fn http_date(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn etag(m: &std::fs::Metadata) -> String {
    let mtime = m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    format!("\"{:x}-{:x}\"", m.len(), mtime)
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("txt" | "log" | "md" | "csv") => "text/plain; charset=utf-8",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("json") => "application/json",
        Some("zip") => "application/zip",
        Some("xml") => "application/xml",
        // Never let a browser render served HTML, SVG or scripts.
        _ => "application/octet-stream",
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn prop_response(href: &str, name: &str, m: &std::fs::Metadata) -> String {
    let modified = m.modified().map(http_date).unwrap_or_default();
    let created = m.created().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let mut p = format!("<D:displayname>{}</D:displayname><D:getlastmodified>{modified}</D:getlastmodified>", xml_escape(name));
    if let Some(c) = created {
        p.push_str(&format!("<D:creationdate>{c}</D:creationdate>"));
    }
    if m.is_dir() {
        p.push_str("<D:resourcetype><D:collection/></D:resourcetype>");
    } else {
        p.push_str(&format!(
            "<D:resourcetype/><D:getcontentlength>{}</D:getcontentlength><D:getcontenttype>{}</D:getcontenttype><D:getetag>{}</D:getetag>",
            m.len(),
            content_type(name),
            xml_escape(&etag(m))
        ));
    }
    format!("<D:response><D:href>{}</D:href><D:propstat><D:prop>{p}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>", xml_escape(href))
}

fn multistatus(body: String) -> Response {
    let xml = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">{body}</D:multistatus>");
    (StatusCode::MULTI_STATUS, [(header::CONTENT_TYPE, "application/xml; charset=utf-8")], xml).into_response()
}

/// Visible children of a folder, sorted by name (dot-files and entries that
/// are not plain files or folders, e.g. symlinks out of the root, are left out).
fn children(roots: &Roots, dir: &Path) -> Vec<(String, std::fs::Metadata)> {
    let mut out: Vec<(String, std::fs::Metadata)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().to_str()?.to_string();
                    if name.starts_with('.') {
                        return None;
                    }
                    // Follow symlinks only when they stay inside a root.
                    let p = roots.resolve(&e.path().to_string_lossy(), true).ok()?;
                    let m = std::fs::metadata(p).ok()?;
                    (m.is_dir() || m.is_file()).then_some((name, m))
                })
                .take(MAX_LISTING)
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Parse a single `bytes=a-b` range against `len`; `Err` = unsatisfiable.
fn byte_range(h: &HeaderMap, len: u64) -> Option<Result<(u64, u64), ()>> {
    let v = h.get(header::RANGE)?.to_str().ok()?.strip_prefix("bytes=")?;
    if v.contains(',') {
        return None; // multiple ranges: send the whole file
    }
    let (a, b) = v.split_once('-')?;
    let r = match (a.trim(), b.trim()) {
        ("", n) => n.parse::<u64>().ok().filter(|n| *n > 0).map(|n| (len.saturating_sub(n), len.saturating_sub(1))),
        (s, "") => s.parse::<u64>().ok().map(|s| (s, len.saturating_sub(1))),
        (s, e) => s.parse::<u64>().ok().zip(e.parse::<u64>().ok()).map(|(s, e)| (s, e.min(len.saturating_sub(1)))),
    }?;
    Some(if len == 0 || r.0 > r.1 || r.0 >= len { Err(()) } else { Ok(r) })
}

async fn send_file(path: &Path, m: &std::fs::Metadata, name: &str, headers: &HeaderMap, head: bool) -> Response {
    let len = m.len();
    let tag = etag(m);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(',').any(|t| t.trim() == tag || t.trim() == "*")) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let if_range_ok = headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()).map_or(true, |v| v == tag);
    let range = if if_range_ok { byte_range(headers, len) } else { None };
    let (status, start, count) = match range {
        Some(Err(())) => {
            let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{len}")) {
                r.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            return r;
        }
        Some(Ok((s, e))) => (StatusCode::PARTIAL_CONTENT, s, e - s + 1),
        None => (StatusCode::OK, 0, len),
    };
    let body = if head {
        Body::empty()
    } else {
        let mut f = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(_) => return plain(StatusCode::NOT_FOUND, "Not found"),
        };
        if start > 0 && f.seek(SeekFrom::Start(start)).await.is_err() {
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "Read failed");
        }
        Body::from_stream(tokio_util::io::ReaderStream::new(f.take(count)))
    };
    let mut r = Response::new(body);
    *r.status_mut() = status;
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(name)));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(count));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    if let Ok(v) = HeaderValue::from_str(&tag) {
        h.insert(header::ETAG, v);
    }
    if let Some(v) = m.modified().ok().and_then(|t| HeaderValue::from_str(&http_date(t)).ok()) {
        h.insert(header::LAST_MODIFIED, v);
    }
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{}/{len}", start + count - 1)) {
            h.insert(header::CONTENT_RANGE, v);
        }
    }
    r
}

fn index_page(title: &str, entries: &[(String, String, bool)]) -> Response {
    let mut rows = String::new();
    for (name, href, dir) in entries {
        rows.push_str(&format!("<li><a href=\"{}\">{}{}</a></li>", html_escape(href), html_escape(name), if *dir { "/" } else { "" }));
    }
    let html = format!("<!doctype html><meta charset=utf-8><title>{t}</title><h1>{t}</h1><ul>{rows}</ul>", t = html_escape(title));
    let mut r = (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
    r.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'"));
    r
}

async fn handle(State(st): State<ServerState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    if st.cfg.dav_roots.is_empty() {
        return plain(StatusCode::NOT_FOUND, "WebDAV is not enabled on this server.");
    }
    if !st.cfg.dav_allowed_clients.is_empty() && !st.cfg.dav_allowed_clients.allows(peer.ip()) {
        tracing::warn!("Refused a WebDAV request from {} (not in --dav-allowed-cidrs)", peer.ip());
        return plain(StatusCode::FORBIDDEN, "Connections from this address may not use WebDAV.");
    }
    let method = req.method().clone();
    let headers = req.headers().clone();
    // Read-only: refuse changes before asking for credentials, so clients
    // learn the share is read-only without retrying with a password.
    let propfind = method.as_str() == "PROPFIND";
    if !(propfind || method == Method::GET || method == Method::HEAD || method == Method::OPTIONS) {
        return not_allowed();
    }
    if method == Method::OPTIONS {
        let mut r = StatusCode::OK.into_response();
        let h = r.headers_mut();
        h.insert(header::ALLOW, HeaderValue::from_static(ALLOW));
        h.insert("DAV", HeaderValue::from_static("1"));
        h.insert("MS-Author-Via", HeaderValue::from_static("DAV"));
        return r;
    }
    if let Err(r) = authenticate(&st, &headers).await {
        return r;
    }
    let roots = match Roots::new(&st.cfg.dav_roots) {
        Ok(r) => r,
        Err(e) => return plain(StatusCode::SERVICE_UNAVAILABLE, &format!("WebDAV roots: {e}")),
    };
    let Some((root, segs)) = parse(req.uri().path()) else {
        return plain(StatusCode::NOT_FOUND, "Not found");
    };
    let depth = headers.get("Depth").and_then(|v| v.to_str().ok()).unwrap_or("infinity").trim().to_ascii_lowercase();
    if propfind && depth != "0" && depth != "1" {
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:error xmlns:D=\"DAV:\"><D:propfind-finite-depth/></D:error>";
        return (StatusCode::FORBIDDEN, [(header::CONTENT_TYPE, "application/xml; charset=utf-8")], body).into_response();
    }

    // `/dav/`: the roots.
    let Some(root) = root else {
        let entries: Vec<(String, std::fs::Metadata)> = roots.list().iter().filter_map(|r| std::fs::metadata(&r.path).ok().map(|m| (r.name.clone(), m))).collect();
        if propfind {
            let dir_meta = std::fs::metadata(&roots.list()[0].path);
            let mut body = String::new();
            if let Ok(m) = dir_meta {
                body.push_str(&prop_response(&format!("{PREFIX}/"), "dav", &m));
            }
            if depth == "1" {
                for (name, m) in &entries {
                    body.push_str(&prop_response(&href(name, &[], true), name, m));
                }
            }
            return multistatus(body);
        }
        if method == Method::HEAD {
            return StatusCode::OK.into_response();
        }
        return index_page("WebDAV", &entries.iter().map(|(n, _)| (n.clone(), href(n, &[], true), true)).collect::<Vec<_>>());
    };
    if !roots.list().iter().any(|r| r.name == root) {
        return plain(StatusCode::NOT_FOUND, "Not found");
    }
    let target: PathBuf = match roots.resolve(&format!("{root}:{}", segs.join("/")), true) {
        Ok(p) => p,
        Err(_) => return plain(StatusCode::NOT_FOUND, "Not found"),
    };
    let Ok(meta) = std::fs::metadata(&target) else {
        return plain(StatusCode::NOT_FOUND, "Not found");
    };
    let name = segs.last().cloned().unwrap_or_else(|| root.clone());
    let is_dir = meta.is_dir();
    tracing::info!("WebDAV {} {} from {}", method, roots.display(&target), peer.ip());

    if propfind {
        let mut body = prop_response(&href(&root, &segs, is_dir), &name, &meta);
        if is_dir && depth == "1" {
            for (child, m) in children(&roots, &target) {
                let mut s = segs.clone();
                s.push(child.clone());
                body.push_str(&prop_response(&href(&root, &s, m.is_dir()), &child, &m));
            }
        }
        return multistatus(body);
    }
    if is_dir {
        if method == Method::HEAD {
            return StatusCode::OK.into_response();
        }
        let mut entries = Vec::new();
        if !segs.is_empty() {
            entries.push(("..".to_string(), href(&root, &segs[..segs.len() - 1], true), true));
        }
        for (child, m) in children(&roots, &target) {
            let mut s = segs.clone();
            s.push(child.clone());
            entries.push((child, href(&root, &s, m.is_dir()), m.is_dir()));
        }
        return index_page(&format!("{root}/{}", segs.join("/")), &entries);
    }
    if !meta.is_file() {
        return plain(StatusCode::NOT_FOUND, "Not found");
    }
    send_file(&target, &meta, &name, &headers, method == Method::HEAD).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_decode_and_refuse_traversal_and_dotfiles() {
        assert_eq!(parse("/dav"), Some((None, vec![])));
        assert_eq!(parse("/dav/"), Some((None, vec![])));
        assert_eq!(parse("/dav/docs/a%20b/c.txt"), Some((Some("docs".into()), vec!["a b".into(), "c.txt".into()])));
        for bad in ["/dav/docs/../x", "/dav/docs/%2e%2e/x", "/dav/docs/.ssh", "/dav/docs/a%2Fb", "/dav/docs/%00", "/dav/docs/%ff"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(href("docs", &["a b".into(), "c#1.txt".into()], false), "/dav/docs/a%20b/c%231.txt");
        assert_eq!(href("docs", &[], true), "/dav/docs/");
    }

    #[test]
    fn byte_ranges() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::RANGE, HeaderValue::from_str(v).unwrap());
            m
        };
        assert_eq!(byte_range(&HeaderMap::new(), 10), None);
        assert_eq!(byte_range(&h("bytes=0-3"), 10), Some(Ok((0, 3))));
        assert_eq!(byte_range(&h("bytes=5-"), 10), Some(Ok((5, 9))));
        assert_eq!(byte_range(&h("bytes=-4"), 10), Some(Ok((6, 9))));
        assert_eq!(byte_range(&h("bytes=8-100"), 10), Some(Ok((8, 9))));
        assert_eq!(byte_range(&h("bytes=10-"), 10), Some(Err(())));
        assert_eq!(byte_range(&h("bytes=0-1,4-5"), 10), None);
        assert_eq!(content_type("x.HTML"), "application/octet-stream");
    }
}
