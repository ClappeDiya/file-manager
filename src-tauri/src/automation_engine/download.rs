//! `RuleAction::DownloadUrl`: fetch a URL into a folder.
//!
//! - http(s) only; at most `MAX_REDIRECTS` redirects, each one checked.
//! - Loopback, private, link-local (cloud metadata), CGNAT and other
//!   non-public addresses are refused unless the rule allows private
//!   addresses. The address checked is the one connected to (DNS answers are
//!   pinned per request), so a name cannot be re-pointed in between.
//! - The body streams to a hidden temporary file next to the target, within
//!   a size limit and with an optional SHA-256 check, then is renamed into
//!   place: a free name (`name (2).ext`) unless overwriting is allowed.
//! - The file name comes from the rule, else `Content-Disposition`, else
//!   the URL's last path segment.

use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
pub struct Download {
    pub url: String,
    pub dest_dir: PathBuf,
    pub file_name: Option<String>,
    pub overwrite: bool,
    pub max_bytes: u64,
    /// Expected SHA-256 (hex); the file is discarded on a mismatch.
    pub sha256: Option<String>,
    pub allow_private: bool,
    pub timeout: Duration,
}

/// Addresses a rule may not reach unless it allows private addresses.
pub fn is_non_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.is_multicast() || v.is_documentation()
                || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT
                || o[0] == 0
                || o[0] >= 240
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_non_public(IpAddr::V4(v4));
            }
            let s = v.segments();
            v.is_loopback() || v.is_unspecified() || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        }
    }
}

/// A safe file name: last component, no control or reserved characters, not hidden.
pub fn safe_file_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c)).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim_end_matches(['.', ' ']).to_string();
    (!cleaned.is_empty() && cleaned.len() <= 255).then_some(cleaned)
}

/// File name from `Content-Disposition` (`filename*=UTF-8''…` preferred).
fn disposition_name(v: &str) -> Option<String> {
    let mut plain = None;
    for part in v.split(';').map(str::trim) {
        if let Some(x) = part.strip_prefix("filename*=") {
            let enc = x.trim_matches('"');
            let raw = enc.split_once("''").map(|(_, r)| r).unwrap_or(enc);
            if let Ok(d) = percent_encoding::percent_decode_str(raw).decode_utf8() {
                return Some(d.into_owned());
            }
        } else if let Some(x) = part.strip_prefix("filename=") {
            plain = Some(x.trim_matches('"').to_string());
        }
    }
    plain
}

fn url_name(u: &url::Url) -> Option<String> {
    let seg = u.path_segments()?.rfind(|s| !s.is_empty())?;
    percent_encoding::percent_decode_str(seg).decode_utf8().ok().map(|s| s.into_owned())
}

/// `name`, or `name (2).ext`, … — the first that does not exist in `dir`.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..10_000).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Resolve `u`'s host, refusing non-public addresses unless allowed; returns the address to pin.
async fn checked_addr(u: &url::Url, allow_private: bool) -> Result<SocketAddr, String> {
    let host = u.host_str().ok_or("The URL has no host")?;
    let port = u.port_or_known_default().ok_or("The URL has no port")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await.map_err(|e| format!("Cannot resolve {host}: {e}"))?.collect();
    let first = addrs.first().copied().ok_or_else(|| format!("Cannot resolve {host}"))?;
    if !allow_private {
        if let Some(bad) = addrs.iter().find(|a| is_non_public(a.ip())) {
            return Err(format!("Download blocked: {host} resolves to the non-public address {} (allow private addresses in the rule to reach it)", bad.ip()));
        }
    }
    Ok(first)
}

/// Download `d.url`; returns the saved file.
pub async fn run(d: &Download) -> Result<PathBuf, String> {
    let mut url = url::Url::parse(d.url.trim()).map_err(|e| format!("Invalid URL '{}': {e}", d.url))?;
    if !d.dest_dir.is_dir() {
        return Err(format!("Destination folder {} does not exist", d.dest_dir.display()));
    }
    let mut hops = 0;
    let resp = loop {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("Unsupported URL scheme '{}' (use http or https)", url.scheme()));
        }
        let addr = checked_addr(&url, d.allow_private).await?;
        let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_string();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, addr)
            .connect_timeout(Duration::from_secs(20))
            .timeout(d.timeout)
            .user_agent(concat!("ufop-automation/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("HTTP client: {e}"))?;
        let r = client.get(url.clone()).send().await.map_err(|e| format!("Download of {url} failed: {e}"))?;
        if r.status().is_redirection() {
            hops += 1;
            if hops > MAX_REDIRECTS {
                return Err(format!("Too many redirects (more than {MAX_REDIRECTS})"));
            }
            let loc = r.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or("A redirect without a Location")?;
            url = url.join(loc).map_err(|e| format!("Invalid redirect '{loc}': {e}"))?;
            continue;
        }
        if !r.status().is_success() {
            return Err(format!("Download of {url} failed: HTTP {}", r.status()));
        }
        break r;
    };
    if resp.content_length().is_some_and(|n| n > d.max_bytes) {
        return Err(format!("Download too large: {} bytes (limit {})", resp.content_length().unwrap_or(0), d.max_bytes));
    }
    let name = d
        .file_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| resp.headers().get(reqwest::header::CONTENT_DISPOSITION).and_then(|v| v.to_str().ok()).and_then(disposition_name))
        .or_else(|| url_name(&url))
        .unwrap_or_else(|| "download".into());
    let name = safe_file_name(&name).ok_or_else(|| format!("Invalid file name '{name}'"))?;
    let temp = d.dest_dir.join(format!(".{name}.ufop-part"));
    let mut resp = resp;
    let write = async {
        let mut f = tokio::fs::File::create(&temp).await.map_err(|e| format!("Cannot write {}: {e}", temp.display()))?;
        let mut hasher = Sha256::new();
        let mut n = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download interrupted: {e}"))? {
            n += chunk.len() as u64;
            if n > d.max_bytes {
                return Err(format!("Download too large: over {} bytes", d.max_bytes));
            }
            hasher.update(&chunk);
            f.write_all(&chunk).await.map_err(|e| format!("Cannot write: {e}"))?;
        }
        f.sync_all().await.map_err(|e| format!("Cannot write: {e}"))?;
        Ok::<_, String>(hex::encode(hasher.finalize()))
    };
    let digest = match write.await {
        Ok(h) => h,
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(e);
        }
    };
    if let Some(want) = d.sha256.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !want.eq_ignore_ascii_case(&digest) {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(format!("Checksum mismatch: expected SHA-256 {want}, got {digest}"));
        }
    }
    let target = if d.overwrite { d.dest_dir.join(&name) } else { free_name(&d.dest_dir, &name) };
    if target.is_dir() {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(format!("{} already exists as a folder", target.display()));
    }
    tokio::fs::rename(&temp, &target).await.map_err(|e| format!("Cannot move the download into place: {e}"))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;

    #[test]
    fn address_classes() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:10.0.0.1", "224.0.0.1"] {
            assert!(is_non_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(!is_non_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn names() {
        assert_eq!(disposition_name("attachment; filename=\"a.pdf\""), Some("a.pdf".into()));
        assert_eq!(disposition_name("attachment; filename=\"x.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"), Some("résumé.pdf".into()));
        assert_eq!(safe_file_name("../../etc/passwd"), Some("passwd".into()));
        assert_eq!(safe_file_name(".bashrc"), Some("bashrc".into()));
        assert_eq!(safe_file_name("..."), None);
        assert_eq!(url_name(&url::Url::parse("https://h/a/My%20Report.pdf?x=1").unwrap()), Some("My Report.pdf".into()));
    }

    async fn server() -> String {
        let app = axum::Router::new()
            .route("/file.bin", get(|| async { vec![7u8; 1000] }))
            .route("/named", get(|| async { ([(header::CONTENT_DISPOSITION, "attachment; filename=\"report q3.csv\"")], "a,b\n1,2\n") }))
            .route("/r1", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/r2")]).into_response() }))
            .route("/r2", get(|| async { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, "/file.bin")]).into_response() }))
            .route("/loop", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/loop")]).into_response() }))
            .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
            .route(
                "/stream",
                get(|| async {
                    // No Content-Length: the limit applies while streaming.
                    let chunks = (0..20).map(|_| Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![1u8; 1000])));
                    axum::body::Body::from_stream(futures_util::stream::iter(chunks))
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://127.0.0.1:{port}")
    }

    fn spec(url: String, dir: &Path) -> Download {
        Download { url, dest_dir: dir.to_path_buf(), file_name: None, overwrite: false, max_bytes: 1 << 20, sha256: None, allow_private: true, timeout: Duration::from_secs(10) }
    }

    #[tokio::test]
    async fn downloads_follow_redirects_name_files_and_enforce_limits() {
        let base = server().await;
        let t = tempfile::tempdir().unwrap();
        let p = run(&spec(format!("{base}/r1"), t.path())).await.unwrap();
        assert_eq!(p, t.path().join("file.bin"), "named after the final URL");
        assert_eq!(std::fs::read(&p).unwrap(), vec![7u8; 1000]);
        // Same name again: a free name; with overwrite: replaced.
        assert_eq!(run(&spec(format!("{base}/file.bin"), t.path())).await.unwrap(), t.path().join("file (2).bin"));
        let o = Download { overwrite: true, ..spec(format!("{base}/file.bin"), t.path()) };
        assert_eq!(run(&o).await.unwrap(), t.path().join("file.bin"));
        assert_eq!(run(&spec(format!("{base}/named"), t.path())).await.unwrap(), t.path().join("report q3.csv"));
        let named = Download { file_name: Some("../evil/x.csv".into()), ..spec(format!("{base}/named"), t.path()) };
        assert_eq!(run(&named).await.unwrap(), t.path().join("x.csv"));

        // Checksum.
        let sha = hex::encode(Sha256::digest(vec![7u8; 1000]));
        assert!(run(&Download { sha256: Some(sha.to_uppercase()), file_name: Some("ok.bin".into()), ..spec(format!("{base}/file.bin"), t.path()) }).await.is_ok());
        let e = run(&Download { sha256: Some("00".repeat(32)), file_name: Some("bad.bin".into()), ..spec(format!("{base}/file.bin"), t.path()) }).await.unwrap_err();
        assert!(e.contains("Checksum mismatch") && !t.path().join("bad.bin").exists());

        // Size limits, by Content-Length and while streaming.
        let e = run(&Download { max_bytes: 999, ..spec(format!("{base}/file.bin"), t.path()) }).await.unwrap_err();
        assert!(e.contains("too large"), "{e}");
        let e = run(&Download { max_bytes: 5000, ..spec(format!("{base}/stream"), t.path()) }).await.unwrap_err();
        assert!(e.contains("too large"), "{e}");
        assert!(run(&spec(format!("{base}/stream"), t.path())).await.is_ok());

        // Errors.
        assert!(run(&spec(format!("{base}/loop"), t.path())).await.unwrap_err().contains("Too many redirects"));
        assert!(run(&spec(format!("{base}/missing"), t.path())).await.unwrap_err().contains("404"));
        assert!(run(&spec("ftp://example.com/x".into(), t.path())).await.unwrap_err().contains("Unsupported URL scheme"));
        assert!(run(&spec(format!("{base}/x"), &t.path().join("nope"))).await.unwrap_err().contains("does not exist"));
        // Private addresses are blocked by default (here: loopback), also via redirects.
        let e = run(&Download { allow_private: false, ..spec(format!("{base}/file.bin"), t.path()) }).await.unwrap_err();
        assert!(e.contains("blocked") && e.contains("127.0.0.1"), "{e}");
        let e = run(&Download { allow_private: false, ..spec("http://169.254.169.254/latest/meta-data/".into(), t.path()) }).await.unwrap_err();
        assert!(e.contains("blocked"), "{e}");
        // No temporary files left behind.
        assert!(std::fs::read_dir(t.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".ufop-part")));
    }
}
