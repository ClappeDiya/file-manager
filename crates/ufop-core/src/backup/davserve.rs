//! A backup version as a read-only WebDAV drive.
//!
//! [`serve`] starts a small HTTP/1.1 server (one thread per connection,
//! keep-alive) that answers `OPTIONS`, `PROPFIND` (depth 0 and 1), `GET` and
//! `HEAD` (with single byte ranges) from a [`VersionTree`], reading file
//! data through a [`BlockSource`] (and so through the volume cache). Every
//! method that would change something gets `405 Method Not Allowed`.
//!
//! Access needs the drive's random token, either as the first path segment
//! (`http://127.0.0.1:PORT/<token>/`, which mount tools accept without a
//! password prompt) or as the password of HTTP Basic authentication (any
//! user name), or as `Authorization: Bearer <token>`. The server binds to
//! the address it is given; the app and CLI use `127.0.0.1` and port 0 (a
//! random free port) unless told otherwise.
//!
//! With several versions the root lists one folder per version, named by
//! its local time (`2026-10-23 10-00-00`), and each version's files are
//! loaded on first access.

use super::access::{open_file, BlockSource, TreeNode, VersionTree};
use super::{EntryKind, FileEntry};
use crate::error::AppError;
use crate::webdav::{self, DavProps};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_CONNECTIONS: usize = 64;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// Loads one version's entries (by fileset id) on first access.
pub type EntryLoader = dyn Fn(i64) -> Result<Vec<FileEntry>, AppError> + Send + Sync;

/// One version offered by a drive.
#[derive(Debug, Clone)]
pub struct DavVersion {
    pub fileset_id: i64,
    /// RFC 3339 time of the version.
    pub timestamp: String,
}

impl DavVersion {
    /// Folder name of the version in an all-versions drive.
    pub fn label(&self) -> String {
        chrono::DateTime::parse_from_rfc3339(&self.timestamp)
            .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H-%M-%S").to_string())
            .unwrap_or_else(|_| self.timestamp.replace([':', '/'], "-"))
    }
}

/// What a drive serves.
pub struct DavContent {
    src: BlockSource<'static>,
    versions: Vec<DavVersion>,
    /// One version at the root instead of a folder per version.
    single: bool,
    loader: Box<EntryLoader>,
    trees: Mutex<HashMap<i64, Arc<VersionTree>>>,
}

impl DavContent {
    /// `single`: serve `versions[0]` at the root.
    pub fn new(src: BlockSource<'static>, versions: Vec<DavVersion>, single: bool, loader: Box<EntryLoader>) -> Result<Self, AppError> {
        if versions.is_empty() {
            return Err(AppError::validation("There is no backup version to open"));
        }
        Ok(Self { src, versions, single, loader, trees: Mutex::default() })
    }

    fn tree(&self, v: &DavVersion) -> Result<Arc<VersionTree>, AppError> {
        if let Some(t) = self.trees.lock().unwrap_or_else(|e| e.into_inner()).get(&v.fileset_id) {
            return Ok(t.clone());
        }
        let t = Arc::new(VersionTree::new((self.loader)(v.fileset_id)?));
        self.trees.lock().unwrap_or_else(|e| e.into_inner()).insert(v.fileset_id, t.clone());
        Ok(t)
    }

    /// Resolve virtual path segments to a resource.
    fn resolve(&self, segs: &[String]) -> Result<Option<Resource>, AppError> {
        if self.single {
            let t = self.tree(&self.versions[0])?;
            return Ok(t.node(&segs.join("/")).cloned().map(|n| Resource::Node(t.clone(), n)));
        }
        let Some((first, rest)) = segs.split_first() else { return Ok(Some(Resource::Versions)) };
        let Some(v) = self.versions.iter().find(|v| v.label() == *first) else { return Ok(None) };
        let t = self.tree(v)?;
        let n = t.node(&rest.join("/")).cloned().map(|mut n| {
            if rest.is_empty() {
                n.name = first.clone();
                n.mtime = chrono::DateTime::parse_from_rfc3339(&v.timestamp).ok().map(|t| t.timestamp());
            }
            n
        });
        Ok(n.map(|n| Resource::Node(t.clone(), n)))
    }
}

enum Resource {
    /// The root of an all-versions drive.
    Versions,
    Node(Arc<VersionTree>, TreeNode),
}

/// A running drive server; stopped when dropped.
pub struct DavServer {
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    last_access: Arc<AtomicI64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl DavServer {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The drive's address with the token in the path.
    pub fn url(&self) -> String {
        format!("http://{}/{}/", self.addr, self.token)
    }

    /// Unix seconds of the last request (idle timeouts).
    pub fn last_access(&self) -> i64 {
        self.last_access.load(Ordering::Relaxed)
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for DavServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A random token for a drive (128 bits, hex).
pub fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Start serving `content` on `bind` (port 0 = any free port).
pub fn serve(content: Arc<DavContent>, bind: SocketAddr, token: String) -> Result<DavServer, AppError> {
    if token.len() < 16 {
        return Err(AppError::validation("The drive token is too short"));
    }
    let listener = TcpListener::bind(bind).map_err(|e| AppError::connection(format!("Cannot listen on {bind}: {e}"), "Pick another port, or leave it empty for a free one."))?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let last_access = Arc::new(AtomicI64::new(now_secs()));
    let (stop2, last2, tok2) = (stop.clone(), last_access.clone(), token.clone());
    let thread = std::thread::Builder::new()
        .name("ufop-dav".into())
        .spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                if active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    let _ = respond(&mut &stream, 503, "Service Unavailable", &[], b"Too many connections", false, true);
                    continue;
                }
                active.fetch_add(1, Ordering::SeqCst);
                let (c, t, a, l, s) = (content.clone(), tok2.clone(), active.clone(), last2.clone(), stop2.clone());
                let spawned = std::thread::Builder::new().name("ufop-dav-conn".into()).spawn(move || {
                    let _ = connection(stream, &c, &t, &l, &s);
                    a.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })
        .map_err(|e| AppError::internal(format!("drive server thread: {e}")))?;
    Ok(DavServer { addr, token, stop, last_access, thread: Some(thread) })
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    close: bool,
}

fn read_request(r: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (method, path, version) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string(), parts.next().unwrap_or("HTTP/1.0").to_string());
    let mut headers = HashMap::new();
    let mut total = line.len();
    loop {
        let mut h = String::new();
        let n = r.read_line(&mut h)?;
        total += n;
        if n == 0 || total > MAX_HEADER_BYTES {
            return Err(std::io::Error::other("headers too large or cut off"));
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    // Discard a body (PROPFIND requests carry XML we do not need).
    let len: u64 = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > MAX_BODY_BYTES || headers.contains_key("transfer-encoding") {
        return Err(std::io::Error::other("request body not accepted"));
    }
    std::io::copy(&mut r.by_ref().take(len), &mut std::io::sink())?;
    let conn = headers.get("connection").map(|v| v.to_ascii_lowercase()).unwrap_or_default();
    let close = conn.contains("close") || (version == "HTTP/1.0" && !conn.contains("keep-alive"));
    Ok(Some(Request { method, path, headers, close }))
}

fn connection(stream: TcpStream, content: &DavContent, token: &str, last: &AtomicI64, stop: &AtomicBool) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_nodelay(true).ok();
    let mut w = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    while !stop.load(Ordering::SeqCst) {
        let Some(req) = read_request(&mut r)? else { return Ok(()) };
        last.store(now_secs(), Ordering::Relaxed);
        handle(&mut w, &req, content, token)?;
        if req.close {
            return Ok(());
        }
    }
    Ok(())
}

fn respond(w: &mut dyn Write, code: u16, reason: &str, headers: &[(&str, String)], body: &[u8], head: bool, close: bool) -> std::io::Result<()> {
    let mut out = format!("HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nServer: ufop-backup-drive\r\n", body.len());
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if close {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    w.write_all(out.as_bytes())?;
    if !head {
        w.write_all(body)?;
    }
    w.flush()
}

fn text(w: &mut dyn Write, code: u16, reason: &str, msg: &str, extra: &[(&str, String)]) -> std::io::Result<()> {
    let mut h = vec![("Content-Type", "text/plain; charset=utf-8".to_string())];
    h.extend(extra.iter().cloned());
    respond(w, code, reason, &h, msg.as_bytes(), false, false)
}

fn eq_ct(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The request path's segments after authentication, and the href prefix
/// (`/<token>` when the token was in the path).
fn authorize(req: &Request, token: &str) -> Option<(Vec<String>, String)> {
    let path = req.path.split('?').next().unwrap_or("/");
    let mut segs: Vec<String> = Vec::new();
    for s in path.split('/').filter(|s| !s.is_empty()) {
        let d = webdav::decode_segment(s)?;
        if d == "." || d == ".." || d.contains(['/', '\\', '\0']) {
            return None;
        }
        segs.push(d);
    }
    if segs.first().is_some_and(|f| eq_ct(f, token)) {
        segs.remove(0);
        return Some((segs, format!("/{token}")));
    }
    let auth = req.headers.get("authorization").map(String::as_str).unwrap_or("");
    let secret = if let Some(b) = auth.strip_prefix("Basic ") {
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD.decode(b.trim()).ok()?;
        String::from_utf8(raw).ok()?.split_once(':').map(|(_, p)| p.to_string())?
    } else {
        auth.strip_prefix("Bearer ")?.trim().to_string()
    };
    eq_ct(&secret, token).then_some((segs, String::new()))
}

fn href(base: &str, segs: &[String], dir: bool) -> String {
    let mut h = base.to_string();
    for s in segs {
        h.push('/');
        h.push_str(&webdav::encode_segment(s));
    }
    if dir || segs.is_empty() {
        h.push('/');
    }
    h
}

fn time_of(mtime: Option<i64>) -> Option<SystemTime> {
    mtime.map(|m| UNIX_EPOCH + Duration::from_secs(m.max(0) as u64))
}

fn etag(e: Option<&FileEntry>, n: &TreeNode) -> String {
    match e.and_then(|e| e.hash.as_deref()) {
        Some(h) => format!("\"{}\"", &h[..16.min(h.len())]),
        None => format!("\"{:x}-{:x}\"", n.size, n.mtime.unwrap_or(0)),
    }
}

/// One listed child: (name, is folder, size, mtime, etag).
type Listed = (String, bool, u64, Option<i64>, Option<String>);

/// Visible children of a resource.
fn listing(content: &DavContent, res: &Resource) -> Vec<Listed> {
    match res {
        Resource::Versions => content
            .versions
            .iter()
            .map(|v| (v.label(), true, 0, chrono::DateTime::parse_from_rfc3339(&v.timestamp).ok().map(|t| t.timestamp()), None))
            .collect(),
        Resource::Node(t, n) if n.kind == EntryKind::Dir => n
            .children
            .values()
            .filter_map(|p| t.node(p))
            .filter(|c| c.kind != EntryKind::Symlink)
            .map(|c| (c.name.clone(), c.kind == EntryKind::Dir, c.size, c.mtime, (c.kind == EntryKind::File).then(|| etag(t.entry(c), c))))
            .collect(),
        Resource::Node(..) => Vec::new(),
    }
}

fn handle(w: &mut dyn Write, req: &Request, content: &DavContent, token: &str) -> std::io::Result<()> {
    let m = req.method.as_str();
    if !matches!(m, "OPTIONS" | "PROPFIND" | "GET" | "HEAD") {
        return text(w, 405, "Method Not Allowed", "This backup drive is read-only.", &[("Allow", webdav::ALLOW.into())]);
    }
    if m == "OPTIONS" {
        return respond(w, 200, "OK", &[("Allow", webdav::ALLOW.into()), ("DAV", "1".into()), ("MS-Author-Via", "DAV".into())], b"", false, false);
    }
    let Some((segs, base)) = authorize(req, token) else {
        return text(w, 401, "Unauthorized", "This backup drive needs its access token.", &[("WWW-Authenticate", "Basic realm=\"ufop backup drive\", charset=\"UTF-8\"".into())]);
    };
    let res = match content.resolve(&segs) {
        Ok(Some(r)) => r,
        Ok(None) => return text(w, 404, "Not Found", "Not found", &[]),
        Err(e) => return text(w, 500, "Internal Server Error", &e.user_message(), &[]),
    };
    let (name, is_dir, size, mtime, entry_etag) = match &res {
        Resource::Versions => ("backup".to_string(), true, 0, None, None),
        Resource::Node(t, n) => (n.name.clone(), n.kind != EntryKind::File, n.size, n.mtime, (n.kind == EntryKind::File).then(|| etag(t.entry(n), n))),
    };
    if let Resource::Node(_, n) = &res {
        if n.kind == EntryKind::Symlink {
            return text(w, 404, "Not Found", "Symbolic links are not shown on the drive", &[]);
        }
    }
    if m == "PROPFIND" {
        let depth = req.headers.get("depth").map(|d| d.trim().to_ascii_lowercase()).unwrap_or_else(|| "infinity".into());
        if depth != "0" && depth != "1" {
            return respond(w, 403, "Forbidden", &[("Content-Type", "application/xml; charset=utf-8".into())], webdav::FINITE_DEPTH_XML.as_bytes(), false, false);
        }
        let own = href(&base, &segs, is_dir);
        let mut body = webdav::prop_response(&DavProps { href: &own, name: &name, is_dir, len: size, modified: time_of(mtime), created: None, etag: entry_etag.as_deref() });
        if is_dir && depth == "1" {
            for (child, dir, len, mt, tag) in listing(content, &res) {
                let mut s = segs.clone();
                s.push(child.clone());
                body.push_str(&webdav::prop_response(&DavProps { href: &href(&base, &s, dir), name: &child, is_dir: dir, len, modified: time_of(mt), created: None, etag: tag.as_deref() }));
            }
        }
        let xml = webdav::multistatus_xml(&body);
        return respond(w, 207, "Multi-Status", &[("Content-Type", "application/xml; charset=utf-8".into())], xml.as_bytes(), false, false);
    }
    let head = m == "HEAD";
    if is_dir {
        let mut entries = Vec::new();
        if !segs.is_empty() {
            entries.push(("..".to_string(), href(&base, &segs[..segs.len() - 1], true), true));
        }
        for (child, dir, ..) in listing(content, &res) {
            let mut s = segs.clone();
            s.push(child.clone());
            entries.push((child, href(&base, &s, dir), dir));
        }
        let html = webdav::index_html(&format!("/{}", segs.join("/")), &entries);
        return respond(w, 200, "OK", &[("Content-Type", "text/html; charset=utf-8".into()), ("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'".into())], html.as_bytes(), head, false);
    }
    let Resource::Node(tree, node) = &res else { return text(w, 404, "Not Found", "Not found", &[]) };
    let Some(entry) = tree.entry(node) else { return text(w, 404, "Not Found", "Not found", &[]) };
    send_file(w, content, entry, &name, req, entry_etag.unwrap_or_default(), head)
}

fn send_file(w: &mut dyn Write, content: &DavContent, e: &FileEntry, name: &str, req: &Request, tag: String, head: bool) -> std::io::Result<()> {
    let len = e.size;
    if req.headers.get("if-none-match").is_some_and(|v| v.split(',').any(|t| t.trim() == tag || t.trim() == "*")) {
        return respond(w, 304, "Not Modified", &[("ETag", tag)], b"", true, false);
    }
    let if_range_ok = req.headers.get("if-range").map_or(true, |v| *v == tag);
    let range = if if_range_ok { webdav::parse_range(req.headers.get("range").map(String::as_str), len) } else { None };
    let (code, reason, start, count) = match range {
        Some(Err(())) => return respond(w, 416, "Range Not Satisfiable", &[("Content-Range", format!("bytes */{len}"))], b"", false, false),
        Some(Ok((s, end))) => (206, "Partial Content", s, end - s + 1),
        None => (200, "OK", 0, len),
    };
    let mut reader = match open_file(&content.src, e) {
        Ok(r) => r,
        Err(err) => return text(w, 500, "Internal Server Error", &err.user_message(), &[]),
    };
    let mut h = format!("HTTP/1.1 {code} {reason}\r\nContent-Length: {count}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nCache-Control: private, no-cache\r\nETag: {tag}\r\n", webdav::content_type(name));
    if let Some(t) = time_of(e.mtime) {
        h.push_str(&format!("Last-Modified: {}\r\n", webdav::http_date(t)));
    }
    if code == 206 {
        h.push_str(&format!("Content-Range: bytes {start}-{}/{len}\r\n", start + count - 1));
    }
    if head {
        h.push_str("\r\n");
        w.write_all(h.as_bytes())?;
        return w.flush();
    }
    // Read the first piece before the headers, so a missing volume still
    // gets a proper error status.
    use std::io::Seek;
    reader.seek(std::io::SeekFrom::Start(start))?;
    let mut limited = reader.take(count);
    let mut buf = vec![0u8; 256 * 1024];
    let first = match limited.read(&mut buf) {
        Ok(n) => n,
        Err(err) => return text(w, 502, "Bad Gateway", &format!("Reading the backup failed: {err}"), &[]),
    };
    h.push_str("\r\n");
    w.write_all(h.as_bytes())?;
    w.write_all(&buf[..first])?;
    // A failure past this point can only cut the connection short.
    std::io::copy(&mut limited, w)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::access::tests::{repo_with, sample_data};
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::engine::Repo;
    use crate::backup::BackupOptions;

    fn drive(single: bool) -> (DavServer, tempfile::TempDir, tempfile::TempDir, Vec<u8>) {
        let big = sample_data(50_000);
        let (src, dst, db) = repo_with(&[("docs/a.txt", b"hello drive"), ("docs/sub/big.bin", &big), ("top.txt", b"t")]);
        let be: Arc<dyn crate::backup::Backend> = Arc::new(LocalBackend::new(dst.path()));
        let repo = Repo::open(be.as_ref(), &db, Some("pw"), &BackupOptions::default(), false, KdfParams::insecure_fast()).unwrap();
        let bs = BlockSource::shared(&repo, be.clone()).unwrap();
        let versions: Vec<DavVersion> = db.filesets().unwrap().into_iter().map(|f| DavVersion { fileset_id: f.id, timestamp: f.timestamp }).collect();
        let files: HashMap<i64, Vec<FileEntry>> = versions.iter().map(|v| (v.fileset_id, db.files(v.fileset_id).unwrap())).collect();
        drop(repo);
        let content = DavContent::new(bs, versions, single, Box::new(move |id| files.get(&id).cloned().ok_or_else(|| AppError::validation("no version")))).unwrap();
        let s = serve(Arc::new(content), "127.0.0.1:0".parse().unwrap(), new_token()).unwrap();
        (s, src, dst, big)
    }

    /// One raw HTTP/1.1 request; returns (status, headers, body).
    pub(crate) fn http(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)]) -> (u16, HashMap<String, String>, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let mut lines = head.lines();
        let code = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
        let hs = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
        (code, hs, raw[split + 4..].to_vec())
    }

    #[test]
    fn propfind_get_and_ranges_on_a_version() {
        let (s, src, _dst, big) = drive(true);
        let t = format!("/{}", s.token());
        let root = src.path().file_name().unwrap().to_string_lossy().into_owned();
        // Listing the root (the source folder) and a folder.
        let (code, _, body) = http(s.addr(), "PROPFIND", &format!("{t}/"), &[("Depth", "1")]);
        assert_eq!(code, 207);
        let xml = String::from_utf8(body).unwrap();
        assert!(xml.contains(&format!("{t}/{}/", webdav::encode_segment(&root))), "{xml}");
        let (code, _, body) = http(s.addr(), "PROPFIND", &format!("{t}/{root}/docs/"), &[("Depth", "1")]);
        assert_eq!(code, 207);
        let xml = String::from_utf8(body).unwrap();
        assert!(xml.contains("<D:displayname>a.txt</D:displayname>") && xml.contains("<D:getcontentlength>11</D:getcontentlength>"));
        assert!(xml.contains("<D:displayname>sub</D:displayname><D:getlastmodified>"), "{xml}");
        assert_eq!(http(s.addr(), "PROPFIND", &format!("{t}/"), &[("Depth", "infinity")]).0, 403);
        // File bytes, HEAD and ranges.
        let (code, h, body) = http(s.addr(), "GET", &format!("{t}/{root}/docs/a.txt"), &[]);
        assert_eq!((code, body.as_slice()), (200, b"hello drive".as_slice()));
        assert_eq!(h["content-type"], "text/plain; charset=utf-8");
        let (code, h, body) = http(s.addr(), "HEAD", &format!("{t}/{root}/docs/sub/big.bin"), &[]);
        assert_eq!((code, h["content-length"].as_str(), body.len()), (200, "50000", 0));
        let (code, h, body) = http(s.addr(), "GET", &format!("{t}/{root}/docs/sub/big.bin"), &[("Range", "bytes=1000-20999")]);
        assert_eq!(code, 206);
        assert_eq!(h["content-range"], "bytes 1000-20999/50000");
        assert_eq!(body, &big[1000..21000]);
        let (code, _, body) = http(s.addr(), "GET", &format!("{t}/{root}/docs/sub/big.bin"), &[("Range", "bytes=-10")]);
        assert_eq!((code, body.as_slice()), (206, &big[big.len() - 10..]));
        assert_eq!(http(s.addr(), "GET", &format!("{t}/{root}/docs/sub/big.bin"), &[("Range", "bytes=60000-")]).0, 416);
        let (_, _, full) = http(s.addr(), "GET", &format!("{t}/{root}/docs/sub/big.bin"), &[]);
        assert_eq!(full, big);
        let etag = http(s.addr(), "HEAD", &format!("{t}/{root}/docs/a.txt"), &[]).1["etag"].clone();
        assert_eq!(http(s.addr(), "GET", &format!("{t}/{root}/docs/a.txt"), &[("If-None-Match", &etag)]).0, 304);
        assert_eq!(http(s.addr(), "GET", &format!("{t}/{root}/nope"), &[]).0, 404);
        // Folder index page.
        let (code, h, body) = http(s.addr(), "GET", &format!("{t}/{root}/docs/"), &[]);
        assert_eq!(code, 200);
        assert!(h["content-type"].starts_with("text/html") && String::from_utf8(body).unwrap().contains("a.txt"));
    }

    #[test]
    fn read_only_and_token_required() {
        let (s, src, _dst, _) = drive(true);
        let root = src.path().file_name().unwrap().to_string_lossy().into_owned();
        let t = format!("/{}", s.token());
        for m in ["PUT", "DELETE", "MKCOL", "MOVE", "COPY", "PROPPATCH", "LOCK", "POST"] {
            let (code, h, _) = http(s.addr(), m, &format!("{t}/{root}/docs/a.txt"), &[]);
            assert_eq!(code, 405, "{m}");
            assert_eq!(h["allow"], webdav::ALLOW);
        }
        assert_eq!(http(s.addr(), "OPTIONS", "/", &[]).0, 200);
        // No token: refused.
        assert_eq!(http(s.addr(), "GET", &format!("/{root}/docs/a.txt"), &[]).0, 401);
        assert_eq!(http(s.addr(), "GET", "/wrong-token-0123456789/", &[]).0, 401);
        assert_eq!(http(s.addr(), "GET", &format!("{t}/{root}/../x"), &[]).0, 401, "traversal is never resolved");
        // Basic auth with the token as password, and Bearer.
        use base64::Engine;
        let basic = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("any:{}", s.token())));
        let (code, _, body) = http(s.addr(), "GET", &format!("/{root}/docs/a.txt"), &[("Authorization", &basic)]);
        assert_eq!((code, body.as_slice()), (200, b"hello drive".as_slice()));
        let bearer = format!("Bearer {}", s.token());
        assert_eq!(http(s.addr(), "PROPFIND", "/", &[("Authorization", &bearer), ("Depth", "0")]).0, 207);
        let wrong = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("any:nope"));
        assert_eq!(http(s.addr(), "GET", "/", &[("Authorization", &wrong)]).0, 401);
        let addr = s.addr();
        s.stop();
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(300)).map(|mut c| {
            let _ = c.write_all(b"GET / HTTP/1.1\r\n\r\n");
            let mut b = Vec::new();
            c.read_to_end(&mut b).map(|n| n == 0).unwrap_or(true)
        }).unwrap_or(true), "stopped servers do not answer");
    }

    #[test]
    fn all_versions_drive_lists_one_folder_per_version() {
        let (s, src, _dst, _) = drive(false);
        let root = src.path().file_name().unwrap().to_string_lossy().into_owned();
        let t = format!("/{}", s.token());
        let (code, _, body) = http(s.addr(), "PROPFIND", &format!("{t}/"), &[("Depth", "1")]);
        assert_eq!(code, 207);
        let xml = String::from_utf8(body).unwrap();
        let label = xml.split("<D:displayname>").nth(2).and_then(|x| x.split('<').next()).unwrap().to_string();
        assert!(label.len() >= 19 && label.contains(' '), "{label}");
        let (code, _, body) = http(s.addr(), "GET", &format!("{t}/{}/{root}/top.txt", webdav::encode_segment(&label)), &[]);
        assert_eq!((code, body.as_slice()), (200, b"t".as_slice()));
        // Keep-alive: two requests on one connection.
        let mut c = TcpStream::connect(s.addr()).unwrap();
        c.write_all(format!("GET {t}/{}/{root}/top.txt HTTP/1.1\r\nHost: x\r\n\r\nGET {t}/ HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", webdav::encode_segment(&label)).as_bytes()).unwrap();
        let mut raw = String::new();
        c.read_to_string(&mut raw).unwrap();
        assert_eq!(raw.matches("HTTP/1.1 200 OK").count(), 2, "{raw}");
    }
}
