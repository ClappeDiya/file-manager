//! Azure connector tests.
//!
//! - Data Lake (dfs) and Files run against an in-process mock that verifies
//!   every request's Shared Key signature from what it actually received
//!   (an implementation of the documented string-to-sign independent of the
//!   client's) and keeps the services' semantics: pagination, append/flush
//!   positions, range bounds, renames, non-empty folder deletes.
//! - Blob runs against Azurite (`UFOP_TEST_AZURE=http://127.0.0.1:10000`),
//!   which checks Shared Key and SAS signatures itself.

use super::azure_rest::{DEV_ACCOUNT, DEV_KEY};
use super::live_tests::{profile, round_trip};
use super::{Connector, ConnectorRegistry, ShareOptions};
use crate::core::types::ConnectionProtocol;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const PAGE: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Svc {
    /// Blob: folder markers and delimiter listings only (the rest runs on Azurite).
    Blob,
    Dfs,
    File,
}

#[derive(Clone, Default)]
struct Node {
    dir: bool,
    data: Vec<u8>,
    /// Data Lake: appended but not yet flushed.
    pending: Vec<u8>,
}

struct Mock {
    svc: Svc,
    key: Vec<u8>,
    /// Path-style endpoint (`/<account>/…`, like Azurite) or host-style.
    path_style: bool,
    /// root (file system / share) → path → node ("" = the root itself).
    roots: Mutex<BTreeMap<String, BTreeMap<String, Node>>>,
    requests: Mutex<Vec<String>>,
}

fn hmac(key: &[u8], s: &str) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    m.update(s.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').map(|(k, v)| (decode(k), decode(&v.replace('+', " ")))).unwrap_or((decode(p), String::new()))).collect()
}

/// The documented Shared Key string-to-sign, rebuilt from the received request.
pub(crate) fn expected_signature(key: &[u8], method: &Method, raw_path: &str, raw_query: &str, h: &HeaderMap) -> String {
    let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let len = match get("content-length").as_str() {
        "0" => String::new(),
        l => l.to_string(),
    };
    let mut ms: Vec<(String, String)> = h.iter().filter(|(k, _)| k.as_str().starts_with("x-ms-")).map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").trim().to_string())).collect();
    ms.sort();
    let mut q: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, v) in query_pairs(raw_query) {
        q.entry(k.to_lowercase()).or_default().push(v);
    }
    let mut resource = format!("/{DEV_ACCOUNT}{raw_path}");
    for (k, mut v) in q {
        v.sort();
        resource.push_str(&format!("\n{k}:{}", v.join(",")));
    }
    let mut sts = [method.as_str().to_string(), get("content-encoding"), get("content-language"), len, get("content-md5"), get("content-type"), String::new(), get("if-modified-since"), get("if-match"), get("if-none-match"), get("if-unmodified-since"), get("range")].join("\n");
    sts.push('\n');
    for (k, v) in ms {
        sts.push_str(&format!("{k}:{v}\n"));
    }
    sts.push_str(&resource);
    format!("SharedKey {DEV_ACCOUNT}:{}", hmac(key, &sts))
}

fn status(code: StatusCode, err: &str) -> Response {
    (code, format!("<Error><Code>{err}</Code></Error>")).into_response()
}

fn children<'a>(tree: &'a BTreeMap<String, Node>, dir: &str) -> Vec<(&'a String, &'a Node)> {
    tree.iter().filter(|(p, _)| !p.is_empty() && p.rsplit_once('/').map(|(d, _)| d).unwrap_or("") == dir).collect()
}

fn ensure_parents(tree: &mut BTreeMap<String, Node>, p: &str) {
    let mut acc = String::new();
    for seg in p.split('/').collect::<Vec<_>>().split_last().map(|(_, rest)| rest.to_vec()).unwrap_or_default() {
        acc = if acc.is_empty() { seg.to_string() } else { format!("{acc}/{seg}") };
        tree.entry(acc.clone()).or_insert(Node { dir: true, ..Default::default() });
    }
}

async fn handle(State(m): State<Arc<Mock>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let body = to_bytes(body, 64 << 20).await.unwrap().to_vec();
    let raw_path = parts.uri.path().to_string();
    let raw_query = parts.uri.query().unwrap_or("").to_string();
    let q: BTreeMap<String, String> = query_pairs(&raw_query).into_iter().collect();
    let h = &parts.headers;
    let hv = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    m.requests.lock().unwrap().push(format!("{} {raw_path}?{raw_query}", parts.method));

    // Path below the endpoint, decoded into root + path.
    let rel = if m.path_style { raw_path.strip_prefix(&format!("/{DEV_ACCOUNT}")).unwrap_or("").to_string() } else { raw_path.clone() };
    let segs: Vec<String> = rel.split('/').filter(|s| !s.is_empty()).map(decode).collect();
    let (root, path) = match segs.split_first() {
        Some((r, rest)) => (r.clone(), rest.join("/")),
        None => (String::new(), String::new()),
    };

    // Authentication: a file SAS on reads, else Shared Key.
    if let Some(sig) = q.get("sig") {
        let sts = format!("{}\n{}\n{}\n/file/{DEV_ACCOUNT}/{root}/{path}\n\n\n{}\n{}\n\n\n\n\n", q["sp"], q["st"], q["se"], q["spr"], q["sv"]);
        if m.svc != Svc::File || parts.method != Method::GET || q.get("sr").map(String::as_str) != Some("f") || *sig != hmac(&m.key, &sts) {
            return status(StatusCode::FORBIDDEN, "AuthenticationFailed");
        }
    } else {
        if hv("x-ms-version").is_empty() || hv("x-ms-date").is_empty() {
            return status(StatusCode::BAD_REQUEST, "MissingRequiredHeader");
        }
        if hv("authorization") != expected_signature(&m.key, &parts.method, &raw_path, &raw_query, h) {
            return status(StatusCode::FORBIDDEN, "AuthenticationFailed");
        }
    }

    let mut roots = m.roots.lock().unwrap();
    let page = |items: Vec<String>, marker: Option<&String>| -> (Vec<String>, String) {
        let start = marker.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
        let next = if start + PAGE < items.len() { (start + PAGE).to_string() } else { String::new() };
        (items.into_iter().skip(start).take(PAGE).collect(), next)
    };
    let method = parts.method.clone();
    match m.svc {
        Svc::Blob => {
            let Some(tree) = roots.get_mut(&root) else { return status(StatusCode::NOT_FOUND, "ContainerNotFound") };
            let raw_rest = rel.trim_start_matches('/').split_once('/').map(|(_, r)| decode(r)).unwrap_or_default();
            match method.as_str() {
                "PUT" if hv("x-ms-blob-type") == "BlockBlob" => {
                    tree.insert(raw_rest, Node { data: body, ..Default::default() });
                    StatusCode::CREATED.into_response()
                }
                "GET" if q.get("comp").map(String::as_str) == Some("list") => {
                    let pfx = q.get("prefix").cloned().unwrap_or_default();
                    let mut prefixes = std::collections::BTreeSet::new();
                    let mut blobs = Vec::new();
                    for (name, n) in tree.iter().filter(|(k, _)| !k.is_empty() && k.starts_with(&pfx)) {
                        match name[pfx.len()..].find('/') {
                            Some(i) => {
                                prefixes.insert(format!("{pfx}{}", &name[pfx.len()..=pfx.len() + i]));
                            }
                            None => blobs.push(format!("<Blob><Name>{name}</Name><Properties><Content-Length>{}</Content-Length></Properties></Blob>", n.data.len())),
                        }
                    }
                    let px: String = prefixes.iter().map(|p| format!("<BlobPrefix><Name>{p}</Name></BlobPrefix>")).collect();
                    format!("<?xml version=\"1.0\"?><EnumerationResults><Blobs>{}{px}</Blobs><NextMarker/></EnumerationResults>", blobs.concat()).into_response()
                }
                "DELETE" => match tree.remove(&raw_rest) {
                    Some(_) => StatusCode::ACCEPTED.into_response(),
                    None => status(StatusCode::NOT_FOUND, "BlobNotFound"),
                },
                _ => status(StatusCode::BAD_REQUEST, "UnsupportedRequest"),
            }
        }
        Svc::Dfs => {
            let cont = q.get("continuation");
            match (method.as_str(), root.is_empty(), path.is_empty()) {
                ("GET", true, _) if q.get("resource").map(String::as_str) == Some("account") => {
                    let (items, next) = page(roots.keys().map(|n| format!("{{\"name\":\"{n}\",\"lastModified\":\"Mon, 01 Jan 2024 00:00:00 GMT\"}}")).collect(), cont);
                    ([("x-ms-continuation", next)], format!("{{\"filesystems\":[{}]}}", items.join(","))).into_response()
                }
                ("PUT", false, true) if q.get("resource").map(String::as_str) == Some("filesystem") => {
                    if roots.contains_key(&root) {
                        return status(StatusCode::CONFLICT, "FilesystemAlreadyExists");
                    }
                    roots.insert(root, BTreeMap::from([(String::new(), Node { dir: true, ..Default::default() })]));
                    StatusCode::CREATED.into_response()
                }
                _ => {
                    let Some(tree) = roots.get_mut(&root) else { return status(StatusCode::NOT_FOUND, "FilesystemNotFound") };
                    match method.as_str() {
                        "GET" if q.get("resource").map(String::as_str) == Some("filesystem") => {
                            let dir = q.get("directory").cloned().unwrap_or_default();
                            if !tree.get(&dir).is_some_and(|n| n.dir) {
                                return status(StatusCode::NOT_FOUND, "PathNotFound");
                            }
                            let all = children(tree, &dir).into_iter().map(|(p, n)| format!("{{\"name\":\"{p}\",\"isDirectory\":\"{}\",\"contentLength\":\"{}\",\"lastModified\":\"Mon, 01 Jan 2024 00:00:00 GMT\"}}", n.dir, n.data.len())).collect();
                            let (items, next) = page(all, cont);
                            ([("x-ms-continuation", next)], format!("{{\"paths\":[{}]}}", items.join(","))).into_response()
                        }
                        "PUT" if !hv("x-ms-rename-source").is_empty() => {
                            let src = hv("x-ms-rename-source");
                            let src_segs: Vec<String> = src.split('/').filter(|s| !s.is_empty()).map(decode).collect();
                            let src_path = src_segs[1..].join("/");
                            if src_segs.first() != Some(&root) || !tree.contains_key(&src_path) {
                                return status(StatusCode::NOT_FOUND, "SourcePathNotFound");
                            }
                            let moved: Vec<(String, Node)> = tree.iter().filter(|(p, _)| **p == src_path || p.starts_with(&format!("{src_path}/"))).map(|(p, n)| (p.clone(), n.clone())).collect();
                            for (p, _) in &moved {
                                tree.remove(p);
                            }
                            ensure_parents(tree, &path);
                            for (p, n) in moved {
                                tree.insert(format!("{path}{}", &p[src_path.len()..]), n);
                            }
                            StatusCode::CREATED.into_response()
                        }
                        "PUT" => {
                            let dir = match q.get("resource").map(String::as_str) {
                                Some("directory") => true,
                                Some("file") => false,
                                _ => return status(StatusCode::BAD_REQUEST, "InvalidQueryParameterValue"),
                            };
                            ensure_parents(tree, &path);
                            tree.insert(path, Node { dir, ..Default::default() });
                            StatusCode::CREATED.into_response()
                        }
                        "PATCH" => {
                            let Some(n) = tree.get_mut(&path).filter(|n| !n.dir) else { return status(StatusCode::NOT_FOUND, "PathNotFound") };
                            let pos: usize = q.get("position").and_then(|p| p.parse().ok()).unwrap_or(usize::MAX);
                            match q.get("action").map(String::as_str) {
                                Some("append") if pos == n.pending.len() => {
                                    n.pending.extend_from_slice(&body);
                                    StatusCode::ACCEPTED.into_response()
                                }
                                Some("flush") if pos == n.pending.len() && body.is_empty() => {
                                    n.data = n.pending.clone();
                                    StatusCode::OK.into_response()
                                }
                                _ => status(StatusCode::BAD_REQUEST, "InvalidFlushPosition"),
                            }
                        }
                        "GET" => match tree.get(&path) {
                            Some(n) if !n.dir => n.data.clone().into_response(),
                            _ => status(StatusCode::NOT_FOUND, "PathNotFound"),
                        },
                        "DELETE" => {
                            if !tree.contains_key(&path) || path.is_empty() {
                                return status(StatusCode::NOT_FOUND, "PathNotFound");
                            }
                            if q.get("recursive").map(String::as_str) != Some("true") && !children(tree, &path).is_empty() {
                                return status(StatusCode::CONFLICT, "DirectoryNotEmpty");
                            }
                            tree.retain(|p, _| *p != path && !p.starts_with(&format!("{path}/")));
                            StatusCode::OK.into_response()
                        }
                        _ => status(StatusCode::METHOD_NOT_ALLOWED, "UnsupportedHttpVerb"),
                    }
                }
            }
        }
        Svc::File => {
            let marker = q.get("marker");
            let restype = q.get("restype").map(String::as_str);
            let comp = q.get("comp").map(String::as_str);
            if root.is_empty() {
                if method == Method::GET && comp == Some("list") {
                    let (items, next) = page(roots.keys().map(|n| format!("<Share><Name>{n}</Name><Properties><Last-Modified>Mon, 01 Jan 2024 00:00:00 GMT</Last-Modified></Properties></Share>")).collect(), marker);
                    return format!("<?xml version=\"1.0\"?><EnumerationResults><Shares>{}</Shares><NextMarker>{next}</NextMarker></EnumerationResults>", items.concat()).into_response();
                }
                return status(StatusCode::BAD_REQUEST, "InvalidUri");
            }
            if method == Method::PUT && restype == Some("share") {
                if roots.contains_key(&root) {
                    return status(StatusCode::CONFLICT, "ShareAlreadyExists");
                }
                roots.insert(root, BTreeMap::from([(String::new(), Node { dir: true, ..Default::default() })]));
                return StatusCode::CREATED.into_response();
            }
            let Some(tree) = roots.get_mut(&root) else { return status(StatusCode::NOT_FOUND, "ShareNotFound") };
            let parent = path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            match (method.as_str(), restype, comp) {
                ("GET", Some("directory"), Some("list")) => {
                    if !tree.get(&path).is_some_and(|n| n.dir) {
                        return status(StatusCode::NOT_FOUND, "ResourceNotFound");
                    }
                    let all = children(tree, &path)
                        .into_iter()
                        .map(|(p, n)| {
                            let name = p.rsplit('/').next().unwrap_or(p);
                            if n.dir {
                                format!("<Directory><Name>{name}</Name><Properties /></Directory>")
                            } else {
                                format!("<File><Name>{name}</Name><Properties><Content-Length>{}</Content-Length><LastWriteTime>2024-01-01T00:00:00.0000000Z</LastWriteTime></Properties></File>", n.data.len())
                            }
                        })
                        .collect();
                    let (items, next) = page(all, marker);
                    format!("<?xml version=\"1.0\"?><EnumerationResults><Entries>{}</Entries><NextMarker>{next}</NextMarker></EnumerationResults>", items.concat()).into_response()
                }
                ("PUT", Some("directory"), None) => {
                    if !tree.get(&parent).is_some_and(|n| n.dir) {
                        return status(StatusCode::NOT_FOUND, "ParentNotFound");
                    }
                    if tree.contains_key(&path) {
                        return status(StatusCode::CONFLICT, "ResourceAlreadyExists");
                    }
                    if hv("x-ms-file-attributes") != "Directory" {
                        return status(StatusCode::BAD_REQUEST, "InvalidHeaderValue");
                    }
                    tree.insert(path, Node { dir: true, ..Default::default() });
                    StatusCode::CREATED.into_response()
                }
                ("PUT", None, Some("range")) => {
                    let Some(n) = tree.get_mut(&path).filter(|n| !n.dir) else { return status(StatusCode::NOT_FOUND, "ResourceNotFound") };
                    let r = hv("x-ms-range");
                    let Some((a, b)) = r.strip_prefix("bytes=").and_then(|r| r.split_once('-')).and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))) else {
                        return status(StatusCode::BAD_REQUEST, "InvalidRange");
                    };
                    if hv("x-ms-write") != "update" || b < a || b - a + 1 != body.len() || body.len() > 4 << 20 || b >= n.data.len() {
                        return status(StatusCode::RANGE_NOT_SATISFIABLE, "InvalidRange");
                    }
                    n.data[a..=b].copy_from_slice(&body);
                    StatusCode::CREATED.into_response()
                }
                ("PUT", None, Some("rename")) => {
                    let src = hv("x-ms-file-rename-source");
                    let src_path = url::Url::parse(&src).ok().map(|u| u.path().to_string()).unwrap_or_default();
                    let src_rel = if m.path_style { src_path.strip_prefix(&format!("/{DEV_ACCOUNT}")).unwrap_or("").to_string() } else { src_path };
                    let ss: Vec<String> = src_rel.split('/').filter(|s| !s.is_empty()).map(decode).collect();
                    let sp = ss[1..].join("/");
                    if ss.first() != Some(&root) || !tree.contains_key(&sp) {
                        return status(StatusCode::NOT_FOUND, "ResourceNotFound");
                    }
                    if tree.contains_key(&path) && hv("x-ms-file-rename-replace-if-exists") != "true" {
                        return status(StatusCode::CONFLICT, "ResourceAlreadyExists");
                    }
                    let n = tree.remove(&sp).unwrap();
                    tree.insert(path, n);
                    StatusCode::OK.into_response()
                }
                ("PUT", None, None) if hv("x-ms-type") == "file" => {
                    if !tree.get(&parent).is_some_and(|n| n.dir) {
                        return status(StatusCode::NOT_FOUND, "ParentNotFound");
                    }
                    let len: usize = hv("x-ms-content-length").parse().unwrap_or(0);
                    tree.insert(path, Node { dir: false, data: vec![0; len], pending: vec![] });
                    StatusCode::CREATED.into_response()
                }
                ("GET", None, None) => match tree.get(&path) {
                    Some(n) if !n.dir => n.data.clone().into_response(),
                    _ => status(StatusCode::NOT_FOUND, "ResourceNotFound"),
                },
                ("DELETE", rt, None) => {
                    let want_dir = rt == Some("directory");
                    match tree.get(&path) {
                        Some(n) if n.dir == want_dir && !path.is_empty() => {}
                        _ => return status(StatusCode::NOT_FOUND, "ResourceNotFound"),
                    }
                    if want_dir && !children(tree, &path).is_empty() {
                        return status(StatusCode::CONFLICT, "DirectoryNotEmpty");
                    }
                    tree.remove(&path);
                    StatusCode::ACCEPTED.into_response()
                }
                _ => status(StatusCode::BAD_REQUEST, "UnsupportedRequest"),
            }
        }
    }
}

/// Start a mock; returns its endpoint and state.
async fn mock(svc: Svc, path_style: bool, roots: &[&str]) -> (String, Arc<Mock>) {
    let m = Arc::new(Mock {
        svc,
        key: base64::engine::general_purpose::STANDARD.decode(DEV_KEY).unwrap(),
        path_style,
        roots: Mutex::new(roots.iter().map(|r| (r.to_string(), BTreeMap::from([(String::new(), Node { dir: true, ..Default::default() })]))).collect()),
        requests: Mutex::new(Vec::new()),
    });
    let app = axum::Router::new().fallback(handle).with_state(m.clone()).layer(axum::extract::DefaultBodyLimit::disable());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://127.0.0.1:{port}");
    (if path_style { format!("{base}/{DEV_ACCOUNT}") } else { base }, m)
}

fn patterned(n: usize) -> Vec<u8> {
    (0..n as u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect()
}

/// Shared checks beyond the generic round trip: multi-chunk upload with no
/// staging leftovers, nested folders, listing across pages, non-empty
/// folder delete refused, root delete refused, wrong key refused.
async fn azure_family(protocol: &str, proto: ConnectionProtocol, endpoint: &str, root: &str) {
    let p = profile(proto, endpoint, None, DEV_ACCOUNT, "/");
    round_trip(protocol, p.clone(), Some(DEV_KEY), &format!("/{root}")).await;

    let reg = ConnectorRegistry::new();
    let c = reg.get(protocol).unwrap();
    c.connect_with_secret(&p, Some(DEV_KEY.into())).await.unwrap();
    let roots = c.list_remote("/").await.unwrap();
    assert!(roots.iter().any(|e| e.name == root && e.is_dir && e.path == format!("/{root}")), "{roots:?}");

    let dir = format!("/{root}/big dir");
    c.make_directory(&dir).await.unwrap();
    c.make_directory(&format!("{dir}/nested")).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("big.bin");
    let data = patterned(9 * 1024 * 1024 + 123); // three chunks
    std::fs::write(&src, &data).unwrap();
    for name in ["a.bin", "b.bin", "c #1.bin"] {
        assert_eq!(c.upload(src.to_str().unwrap(), &format!("{dir}/{name}")).await.unwrap(), data.len() as u64);
    }
    // Overwrite in place.
    std::fs::write(&src, b"replaced").unwrap();
    assert_eq!(c.upload(src.to_str().unwrap(), &format!("{dir}/b.bin")).await.unwrap(), 8);

    let listing = c.list_remote(&dir).await.unwrap();
    let mut names: Vec<(&str, bool, u64)> = listing.iter().map(|e| (e.name.as_str(), e.is_dir, e.size)).collect();
    names.sort();
    assert_eq!(names, vec![("a.bin", false, data.len() as u64), ("b.bin", false, 8), ("c #1.bin", false, data.len() as u64), ("nested", true, 0)], "listing spans pages, no staging files");
    assert!(listing.iter().all(|e| e.path.starts_with(&format!("{dir}/"))));

    let back = tmp.path().join("back.bin");
    c.download(&format!("{dir}/c #1.bin"), back.to_str().unwrap()).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), data);

    assert!(c.remove(&dir, true).await.is_err(), "non-empty folder");
    assert!(c.remove(&format!("/{root}"), true).await.is_err(), "root");
    for name in ["a.bin", "b.bin", "c #1.bin"] {
        c.remove(&format!("{dir}/{name}"), false).await.unwrap();
    }
    c.remove(&format!("{dir}/nested"), true).await.unwrap();
    c.remove(&dir, true).await.unwrap();
    assert!(!c.list_remote(&format!("/{root}")).await.unwrap().iter().any(|e| e.name == "big dir"));

    // Not connected after a failed connect.
    let wrong = base64::engine::general_purpose::STANDARD.encode([7u8; 64]);
    assert!(c.connect_with_secret(&p, Some(wrong)).await.is_err());
    assert!(!c.is_connected());
    assert!(c.connect_with_secret(&p, None).await.is_err());
}

#[tokio::test]
async fn adls_gen2_against_signature_checking_mock() {
    let (ep, m) = mock(Svc::Dfs, false, &["lake"]).await;
    azure_family("adls_gen2", ConnectionProtocol::AdlsGen2, &ep, "lake").await;
    // Uploads went through a staging name and an atomic rename.
    let reqs = m.requests.lock().unwrap().clone();
    assert!(reqs.iter().any(|r| r.contains(".a.bin.ufop-part") && r.starts_with("PUT")), "{reqs:?}");
    // A file system can be created; listing it works through the connector.
    let c = AdlsGen2ConnectorProbe::connect(&ep).await;
    c.make_directory("/newfs").await.unwrap();
    assert!(c.list_remote("/").await.unwrap().iter().any(|e| e.name == "newfs"));
    let link = c.create_share_link("/lake/x.bin", &ShareOptions::default()).await.unwrap();
    assert!(link.url.contains("&sr=b&sp=r&"), "{}", link.url);
}

#[tokio::test]
async fn azure_files_against_signature_checking_mock() {
    let (ep, m) = mock(Svc::File, true, &["share"]).await;
    azure_family("azure_files", ConnectionProtocol::AzureFiles, &ep, "share").await;
    let reqs = m.requests.lock().unwrap().clone();
    assert!(reqs.iter().any(|r| r.contains("comp=rename")), "{reqs:?}");
    assert!(reqs.iter().any(|r| r.contains("comp=range")));

    // A read-only SAS link downloads without credentials.
    let reg = ConnectorRegistry::new();
    let c = reg.get("azure_files").unwrap();
    c.connect_with_secret(&profile(ConnectionProtocol::AzureFiles, &ep, None, DEV_ACCOUNT, "/share"), Some(DEV_KEY.into())).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("s.txt"), b"shared via SAS").unwrap();
    c.upload(tmp.path().join("s.txt").to_str().unwrap(), "/share/s t.txt").await.unwrap();
    let link = c.create_share_link("/share/s t.txt", &ShareOptions { expires_in_secs: Some(3600), ..Default::default() }).await.unwrap();
    assert!(link.expires_at.is_some());
    let r = reqwest::get(&link.url).await.unwrap();
    assert_eq!(r.status(), 200, "{}", link.url);
    assert_eq!(r.text().await.unwrap(), "shared via SAS");
    let tampered = link.url.replace("sp=r", "sp=rw");
    assert_eq!(reqwest::get(&tampered).await.unwrap().status(), 403);
    assert!(c.create_share_link("/share/s t.txt", &ShareOptions { allow_edit: true, ..Default::default() }).await.is_err());
    assert!(c.create_share_link("/share", &ShareOptions::default()).await.is_err());

    // Missing parent folder is reported, not silently created.
    assert!(c.make_directory("/share/no/such/parent").await.is_err());
    // A connection string works in place of the key.
    let cs = format!("DefaultEndpointsProtocol=http;AccountName={DEV_ACCOUNT};AccountKey={DEV_KEY};FileEndpoint={ep}");
    c.connect_with_secret(&profile(ConnectionProtocol::AzureFiles, "", None, "", "/"), Some(cs)).await.unwrap();
    assert!(c.list_remote("/").await.unwrap().iter().any(|e| e.name == "share"));
}

#[tokio::test]
async fn azure_blob_folder_markers_against_mock() {
    let (ep, m) = mock(Svc::Blob, true, &["cont"]).await;
    let c = super::AzureBlobConnector::new();
    c.connect_with_secret(&profile(ConnectionProtocol::AzureBlob, &ep, None, DEV_ACCOUNT, "/cont"), Some(DEV_KEY.into())).await.unwrap();
    c.make_directory("/cont/new folder").await.unwrap();
    c.make_directory("/cont/new folder/inner").await.unwrap();
    assert!(m.roots.lock().unwrap()["cont"].contains_key("new folder/"), "zero-byte `dir/` marker");
    let l = c.list_remote("/cont").await.unwrap();
    assert_eq!(l.iter().map(|e| (e.name.as_str(), e.is_dir)).collect::<Vec<_>>(), vec![("new folder", true)]);
    // The folder's own marker is not listed as a file inside it.
    let inner = c.list_remote("/cont/new folder").await.unwrap();
    assert_eq!(inner.iter().map(|e| (e.name.as_str(), e.is_dir)).collect::<Vec<_>>(), vec![("inner", true)]);
    assert!(c.remove("/cont/new folder", true).await.is_err(), "not empty");
    c.remove("/cont/new folder/inner", true).await.unwrap();
    c.remove("/cont/new folder", true).await.unwrap();
    assert!(c.list_remote("/cont").await.unwrap().is_empty());
}

struct AdlsGen2ConnectorProbe;
impl AdlsGen2ConnectorProbe {
    async fn connect(ep: &str) -> super::AdlsGen2Connector {
        let c = super::AdlsGen2Connector::new();
        c.connect_with_secret(&profile(ConnectionProtocol::AdlsGen2, ep, None, DEV_ACCOUNT, "/"), Some(DEV_KEY.into())).await.unwrap();
        c
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn azure_blob_against_azurite() {
    let Ok(base) = std::env::var("UFOP_TEST_AZURE") else { return };
    let ep = format!("{base}/{DEV_ACCOUNT}");
    let container = format!("ufop{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let reg = ConnectorRegistry::new();
    let c = reg.get("azure_blob").unwrap();
    let p = profile(ConnectionProtocol::AzureBlob, &ep, None, DEV_ACCOUNT, "/");
    c.connect_with_secret(&p, Some(DEV_KEY.into())).await.unwrap();
    // Azurite cannot route blob names ending in "/", so folder markers
    // (`make_directory` below the container) are covered by the mock test.
    c.make_directory(&format!("/{container}")).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("big.bin");
    let data = patterned(9 * 1024 * 1024 + 7); // three blocks
    std::fs::write(&src, &data).unwrap();
    std::fs::write(tmp.path().join("small.txt"), b"small").unwrap();
    assert_eq!(c.upload(tmp.path().join("small.txt").to_str().unwrap(), &format!("/{container}/small.txt")).await.unwrap(), 5);
    c.upload(src.to_str().unwrap(), &format!("/{container}/docs/big file.bin")).await.unwrap();
    c.upload(src.to_str().unwrap(), &format!("/{container}/docs/sub/deep.bin")).await.unwrap();
    let mut l: Vec<(String, bool, u64)> = c.list_remote(&format!("/{container}/docs")).await.unwrap().into_iter().map(|e| (e.name, e.is_dir, e.size)).collect();
    l.sort();
    assert_eq!(l, vec![("big file.bin".into(), false, data.len() as u64), ("sub".into(), true, 0)]);
    let back = tmp.path().join("back.bin");
    c.download(&format!("/{container}/docs/big file.bin"), back.to_str().unwrap()).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), data);

    // Azurite verifies the SAS signature: the link downloads anonymously.
    let link = c.create_share_link(&format!("/{container}/docs/big file.bin"), &ShareOptions::default()).await.unwrap();
    let r = reqwest::get(&link.url).await.unwrap();
    assert_eq!(r.status(), 200, "{}", link.url);
    assert_eq!(r.bytes().await.unwrap().len(), data.len());
    assert_eq!(reqwest::get(link.url.replace("sp=r", "sp=rw")).await.unwrap().status(), 403);

    assert!(c.remove(&format!("/{container}/docs"), true).await.is_err(), "non-empty");
    assert!(c.remove(&format!("/{container}"), true).await.is_err(), "container");
    c.remove(&format!("/{container}/docs/sub/deep.bin"), false).await.unwrap();
    // A virtual folder without a marker disappears with its last blob; removing it is fine.
    c.remove(&format!("/{container}/docs/sub"), true).await.unwrap();
    let root: Vec<String> = c.list_remote(&format!("/{container}")).await.unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(root, vec!["docs".to_string(), "small.txt".to_string()]);
    let wrong = base64::engine::general_purpose::STANDARD.encode([7u8; 64]);
    assert!(c.connect_with_secret(&p, Some(wrong)).await.is_err());
    assert!(!c.is_connected());
    // UseDevelopmentStorage connection string.
    c.connect_with_secret(&profile(ConnectionProtocol::AzureBlob, &ep, None, "", "/"), Some("UseDevelopmentStorage=true".into())).await.unwrap();
    assert!(c.list_remote("/").await.unwrap().iter().any(|e| e.name == container));
}
