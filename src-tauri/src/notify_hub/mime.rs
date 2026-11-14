//! Mail message building: RFC 2047 header words, base64 bodies wrapped at
//! 76 characters and `multipart/alternative` (text + HTML) messages, plus
//! the default HTML report template.

use base64::Engine;
use std::collections::BTreeMap;

/// RFC 2047 encoded words (UTF-8, base64) for a non-ASCII header value,
/// split so each word stays under 75 characters and folded with CRLF + space.
pub fn encode_words(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in s.chars() {
        // 39 bytes encode to 52 base64 characters (+12 for the wrapper), so
        // even the first word after "Subject: " stays within 78 characters.
        if chunk.len() + c.len_utf8() > 39 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words.iter().map(|w| format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(w))).collect::<Vec<_>>().join("\r\n ")
}

/// Base64 of `body`, wrapped at 76 characters with CRLF.
pub fn base64_lines(body: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    let mut out = String::with_capacity(b64.len() + b64.len() / 76 * 2 + 2);
    for chunk in b64.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push_str("\r\n");
    }
    out
}

/// What [`build_message`] needs.
pub struct Mail<'a> {
    pub from: &'a str,
    pub to: &'a [String],
    pub subject: &'a str,
    pub text: &'a str,
    /// Adds a text/html alternative when set.
    pub html: Option<&'a str>,
    pub domain: &'a str,
}

/// The full message (headers and body, CRLF line ends, without the final
/// `.`). Bodies are base64, so no line starts with a dot.
pub fn build_message(m: &Mail<'_>, date: &str, message_id: &str, boundary: &str) -> String {
    let mut msg = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {date}\r\nMessage-ID: <{message_id}@{}>\r\nMIME-Version: 1.0\r\n",
        m.from,
        m.to.join(", "),
        encode_words(m.subject),
        m.domain
    );
    match m.html {
        None => {
            msg.push_str("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n");
            msg.push_str(&base64_lines(m.text));
        }
        Some(html) => {
            msg.push_str(&format!("Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\nThis is a multi-part message in MIME format.\r\n"));
            for (ct, body) in [("text/plain", m.text), ("text/html", html)] {
                msg.push_str(&format!("--{boundary}\r\nContent-Type: {ct}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
                msg.push_str(&base64_lines(body));
            }
            msg.push_str(&format!("--{boundary}--\r\n"));
        }
    }
    msg
}

/// Default HTML report: a table of the run's values and the report text.
/// Every `{{var}}` is HTML-escaped.
pub const DEFAULT_HTML_TEMPLATE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>{{title}}</title></head>
<body style="margin:0;padding:16px;background:#f4f5f7;font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1f2933">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:640px;margin:0 auto;background:#ffffff;border:1px solid #d9dde3;border-radius:6px">
<tr><td style="padding:16px 20px;border-bottom:3px solid {{result_color}}">
<div style="font-size:12px;color:#616e7c;text-transform:uppercase;letter-spacing:.04em">UFOP {{operation}}</div>
<div style="font-size:18px;font-weight:600;margin-top:4px">{{name}}: <span style="color:{{result_color}}">{{result}}</span></div>
</td></tr>
<tr><td style="padding:12px 20px">
<table role="presentation" width="100%" cellpadding="4" cellspacing="0" style="font-size:14px;border-collapse:collapse">
<tr><td style="color:#616e7c;width:35%">Machine</td><td>{{machine}}</td></tr>
<tr><td style="color:#616e7c">Duration</td><td>{{duration}}</td></tr>
<tr><td style="color:#616e7c">Files examined</td><td>{{files}}</td></tr>
<tr><td style="color:#616e7c">Bytes uploaded</td><td>{{bytes}}</td></tr>
<tr><td style="color:#616e7c">Errors</td><td>{{errors}}</td></tr>
</table>
</td></tr>
<tr><td style="padding:0 20px 16px"><pre style="white-space:pre-wrap;word-break:break-word;font-size:12px;background:#f8f9fb;border:1px solid #e4e7eb;border-radius:4px;padding:10px;margin:0">{{message}}</pre></td></tr>
</table>
</body></html>
"#;

/// Accent colour per result for HTML templates.
pub fn result_color(result: &str) -> &'static str {
    match result {
        "error" => "#c62828",
        "warning" => "#e65100",
        "success" => "#2e7d32",
        _ => "#3e4c59",
    }
}

/// Render an HTML body (`template` empty = the default) with escaped values.
pub fn render_html(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut v = vars.clone();
    let result = v.get("result").cloned().unwrap_or_default();
    v.entry("result_color".into()).or_insert_with(|| result_color(&result).into());
    v.remove("secret");
    super::webhook::render(if template.trim().is_empty() { DEFAULT_HTML_TEMPLATE } else { template }, &v, super::webhook::Escape::Html)
}
