//! Read-only WebDAV building blocks shared by the server's `/dav` share
//! (`src-tauri/src/server/dav.rs`) and the backup-version drive
//! ([`crate::backup::davserve`]): byte ranges, content types, dates, path
//! segment encoding and the `PROPFIND` XML.

use std::time::SystemTime;

/// Methods a read-only share answers.
pub const ALLOW: &str = "OPTIONS, GET, HEAD, PROPFIND";

/// Parse a single `bytes=a-b` range header value against `len`.
/// `None` = no (usable) range: send everything; `Some(Err(()))` = unsatisfiable.
pub fn parse_range(value: Option<&str>, len: u64) -> Option<Result<(u64, u64), ()>> {
    let v = value?.strip_prefix("bytes=")?;
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

/// HTTP date (`Sun, 06 Nov 1994 08:49:37 GMT`).
pub fn http_date(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

/// Content type by extension. HTML, SVG and scripts are served as bytes so
/// a browser never renders them from the share.
pub fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("txt" | "log" | "md" | "csv") => "text/plain; charset=utf-8",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("json") => "application/json",
        Some("zip") => "application/zip",
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn html_escape(s: &str) -> String {
    xml_escape(s).replace('\'', "&#39;")
}

/// Percent-encode one path segment (everything but unreserved characters).
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'(' | b')' | b'!' | b'*' | b'+' | b',' | b';' | b'=' | b'@' | b'$' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode a percent-encoded segment (invalid UTF-8 or escapes → `None`).
pub fn decode_segment(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Properties of one resource in a `PROPFIND` answer.
pub struct DavProps<'a> {
    pub href: &'a str,
    pub name: &'a str,
    pub is_dir: bool,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub etag: Option<&'a str>,
}

/// One `<D:response>` element.
pub fn prop_response(p: &DavProps) -> String {
    let modified = p.modified.map(http_date).unwrap_or_default();
    let mut x = format!("<D:displayname>{}</D:displayname><D:getlastmodified>{modified}</D:getlastmodified>", xml_escape(p.name));
    if let Some(c) = p.created {
        x.push_str(&format!("<D:creationdate>{}</D:creationdate>", chrono::DateTime::<chrono::Utc>::from(c).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)));
    }
    if p.is_dir {
        x.push_str("<D:resourcetype><D:collection/></D:resourcetype>");
    } else {
        x.push_str(&format!("<D:resourcetype/><D:getcontentlength>{}</D:getcontentlength><D:getcontenttype>{}</D:getcontenttype>", p.len, content_type(p.name)));
        if let Some(t) = p.etag {
            x.push_str(&format!("<D:getetag>{}</D:getetag>", xml_escape(t)));
        }
    }
    format!("<D:response><D:href>{}</D:href><D:propstat><D:prop>{x}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>", xml_escape(p.href))
}

/// Wrap responses in a `<D:multistatus>` document.
pub fn multistatus_xml(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">{body}</D:multistatus>")
}

/// The `PROPFIND` refusal for `Depth: infinity`.
pub const FINITE_DEPTH_XML: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:error xmlns:D=\"DAV:\"><D:propfind-finite-depth/></D:error>";

/// A small HTML folder index: (name, href, is folder).
pub fn index_html(title: &str, entries: &[(String, String, bool)]) -> String {
    let mut rows = String::new();
    for (name, href, dir) in entries {
        rows.push_str(&format!("<li><a href=\"{}\">{}{}</a></li>", html_escape(href), html_escape(name), if *dir { "/" } else { "" }));
    }
    format!("<!doctype html><meta charset=utf-8><title>{t}</title><h1>{t}</h1><ul>{rows}</ul>", t = html_escape(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 10), None);
        assert_eq!(parse_range(Some("bytes=0-3"), 10), Some(Ok((0, 3))));
        assert_eq!(parse_range(Some("bytes=5-"), 10), Some(Ok((5, 9))));
        assert_eq!(parse_range(Some("bytes=-4"), 10), Some(Ok((6, 9))));
        assert_eq!(parse_range(Some("bytes=8-100"), 10), Some(Ok((8, 9))));
        assert_eq!(parse_range(Some("bytes=10-"), 10), Some(Err(())));
        assert_eq!(parse_range(Some("bytes=0-1,4-5"), 10), None);
        assert_eq!(parse_range(Some("items=0-1"), 10), None);
    }

    #[test]
    fn segments_round_trip() {
        for s in ["a b", "c#1.txt", "ü/x", "100%", "x?y&z"] {
            assert_eq!(decode_segment(&encode_segment(s)).unwrap(), s);
        }
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
        assert_eq!(decode_segment("%ff"), None);
        assert_eq!(decode_segment("%2"), None);
        assert_eq!(content_type("x.HTML"), "application/octet-stream");
        assert!(prop_response(&DavProps { href: "/a&b", name: "a&b", is_dir: false, len: 3, modified: None, created: None, etag: Some("\"e\"") }).contains("<D:getcontentlength>3</D:getcontentlength>"));
    }
}
