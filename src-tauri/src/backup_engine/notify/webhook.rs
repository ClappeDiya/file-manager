//! Generic webhook channel: custom method, content type, headers (secret
//! ones live in the credential store) and a body template whose
//! placeholders are escaped for the body's format, plus presets for
//! Pushover, ntfy, Discord, PagerDuty Events v2 and Gotify.
//!
//! Template placeholders: `{{var}}` is escaped for where it appears (JSON
//! string content in JSON bodies, percent-encoding in form bodies and URLs,
//! HTML in HTML mail, CR/LF removed in headers); `{{json var}}` inserts a
//! quoted JSON string; `{{raw var}}` inserts the value unchanged (header
//! values still lose CR/LF). Unknown placeholders are left as they are.

use super::{CertPolicy, Channel, Notification};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a rendered placeholder lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    None,
    Json,
    Form,
    Url,
    Html,
    Header,
}

pub fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn json_inner(s: &str) -> String {
    let q = serde_json::to_string(s).unwrap_or_default();
    q[1..q.len().saturating_sub(1).max(1)].to_string()
}

fn header_safe(s: &str) -> String {
    s.chars().map(|c| if c == '\r' || c == '\n' || c == '\0' { ' ' } else { c }).collect()
}

/// Percent-encode everything but RFC 3986 unreserved characters.
const URL_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn escape(v: &str, e: Escape) -> String {
    match e {
        Escape::None => v.to_string(),
        Escape::Json => json_inner(v),
        Escape::Form => url::form_urlencoded::byte_serialize(v.as_bytes()).collect(),
        Escape::Url => percent_encoding::utf8_percent_encode(v, URL_SET).to_string(),
        Escape::Html => html_escape(v),
        Escape::Header => header_safe(v),
    }
}

/// Substitute placeholders, escaping values for `esc` (see the module docs).
pub fn render(template: &str, vars: &BTreeMap<String, String>, esc: Escape) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = after[..end].trim();
        let (modifier, name) = match inner.split_once(char::is_whitespace) {
            Some((m @ ("json" | "raw"), n)) => (Some(m), n.trim()),
            _ => (None, inner),
        };
        match vars.get(name) {
            Some(v) => {
                let s = match modifier {
                    Some("raw") => v.clone(),
                    Some(_) => {
                        let q = serde_json::to_string(v).unwrap_or_default();
                        if esc == Escape::Html { html_escape(&q) } else { q }
                    }
                    None => escape(v, esc),
                };
                out.push_str(&if esc == Escape::Header { header_safe(&s) } else { s });
            }
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Whether `template` uses the placeholder `name` (with or without a modifier).
pub fn uses_var(template: &str, name: &str) -> bool {
    let mut rest = template;
    while let Some(s) = rest.find("{{") {
        let after = &rest[s + 2..];
        let Some(e) = after.find("}}") else { return false };
        let inner = after[..e].trim();
        if inner == name || inner.split_once(char::is_whitespace).is_some_and(|(m, n)| matches!(m, "json" | "raw") && n.trim() == name) {
            return true;
        }
        rest = &after[e + 2..];
    }
    false
}

/// A ready-made service the webhook was created for (changes behaviour
/// only for PagerDuty: resolve on recovery, skip repeated successes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebhookPreset {
    #[default]
    Generic,
    Pushover,
    Ntfy,
    Discord,
    Pagerduty,
    Gotify,
}

impl WebhookPreset {
    /// Longest message the service takes (characters; 0 = no limit).
    fn message_limit(self) -> usize {
        match self {
            Self::Pushover => 1000,
            Self::Discord => 4000,
            Self::Ntfy => 4000,
            Self::Pagerduty => 16_000,
            Self::Gotify | Self::Generic => 0,
        }
    }
}

/// One extra request header. A secret header's value lives in the
/// credential store under `credential_id` (the job keeps it empty).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookHeader {
    pub name: String,
    pub value: String,
    pub secret: bool,
    pub credential_id: String,
}

const FORBIDDEN_HEADERS: &[&str] = &["host", "content-length", "transfer-encoding", "connection", "upgrade", "te", "trailer", "keep-alive", "proxy-connection"];

pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH"];

/// RFC 9110 token characters; no CR/LF in values; no hop-by-hop headers.
pub fn validate_headers(headers: &[WebhookHeader]) -> Result<(), AppError> {
    for h in headers {
        let name = h.name.trim();
        if name.is_empty() || name.len() > 128 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) {
            return Err(AppError::validation(format!("'{}' is not a valid header name (letters, digits and !#$%&'*+-.^_`|~ only)", h.name)));
        }
        if FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(AppError::validation(format!("The {name} header is set by UFOP and cannot be changed")));
        }
        if h.value.contains(['\r', '\n', '\0']) {
            return Err(AppError::validation(format!("The {name} header value cannot contain line breaks")));
        }
        if h.value.len() > 8192 {
            return Err(AppError::validation(format!("The {name} header value is too long")));
        }
        if h.secret && h.value.is_empty() && h.credential_id.trim().is_empty() {
            return Err(AppError::validation(format!("Enter a value for the secret {name} header")));
        }
    }
    Ok(())
}

pub fn validate_url_and_method(url: &str, method: &str) -> Result<(), AppError> {
    if !METHODS.contains(&method.to_ascii_uppercase().as_str()) {
        return Err(AppError::validation(format!("Webhook method '{method}' is not supported (GET, POST, PUT or PATCH)")));
    }
    if url.trim().is_empty() {
        return Err(AppError::validation("Enter the webhook URL"));
    }
    // A URL taken entirely from the secret (Discord, Slack-like) is checked when sent.
    if url.trim().starts_with("{{") {
        return Ok(());
    }
    let u = url::Url::parse(&url.replace("{{", "x").replace("}}", "x")).map_err(|_| AppError::validation("The webhook URL is not a valid URL"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(AppError::validation("The webhook URL must start with http:// or https://"));
    }
    Ok(())
}

/// Per-result values the presets use (also available to custom templates).
pub fn result_vars(result: &str, previous: Option<&str>) -> Vec<(&'static str, String)> {
    let bad = matches!(result, "error" | "warning");
    let (pd, push, ntfy, gotify, color, tags) = match result {
        "error" => ("critical", "1", "4", "8", 0xE5_3935, "rotating_light,ufop"),
        "warning" => ("warning", "0", "3", "5", 0xFB_8C00, "warning,ufop"),
        "success" => ("info", "-1", "2", "2", 0x43_A047, "white_check_mark,ufop"),
        _ => ("info", "0", "3", "4", 0x60_7D8B, "ufop"),
    };
    vec![
        ("pd_severity", pd.into()),
        ("event_action", if result == "success" { "resolve" } else { "trigger" }.into()),
        ("pushover_priority", push.into()),
        ("ntfy_priority", ntfy.into()),
        ("ntfy_tags", tags.into()),
        ("gotify_priority", gotify.into()),
        ("discord_color", color.to_string()),
        ("previous_result", previous.unwrap_or_default().to_string()),
        ("recovered", (!bad && previous.is_some_and(|p| matches!(p, "error" | "warning"))).to_string()),
    ]
}

/// Inputs for [`preset`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetParams {
    /// Server base URL (ntfy, Gotify; optional override for the others).
    pub server: String,
    /// ntfy topic.
    pub topic: String,
    /// Pushover user or group key.
    pub user: String,
    /// ntfy: send the stored secret as a bearer token.
    pub bearer: bool,
}

/// What the secret of a preset's channel is (for the UI label).
pub fn secret_label(p: WebhookPreset) -> &'static str {
    match p {
        WebhookPreset::Pushover => "Application API token",
        WebhookPreset::Ntfy => "Access token (optional)",
        WebhookPreset::Discord => "Discord webhook URL",
        WebhookPreset::Pagerduty => "Integration (routing) key",
        WebhookPreset::Gotify => "Application token",
        WebhookPreset::Generic => "Secret ({{secret}} in the URL, headers or body)",
    }
}

fn base(server: &str, default: &str) -> String {
    let s = server.trim().trim_end_matches('/');
    if s.is_empty() { default.to_string() } else { s.to_string() }
}

/// A webhook channel configured for a service. The secret (token, key or
/// webhook URL) is stored separately under the returned `credential_id`.
pub fn preset(kind: WebhookPreset, p: &PresetParams) -> Result<Notification, AppError> {
    let enc = |s: &str| -> String { url::form_urlencoded::byte_serialize(s.as_bytes()).collect() };
    let h = |name: &str, value: &str| WebhookHeader { name: name.into(), value: value.into(), ..Default::default() };
    let all = vec!["success".to_string(), "warning".into(), "error".into()];
    let (url, content_type, headers, body, send_on) = match kind {
        WebhookPreset::Pushover => {
            if p.user.trim().is_empty() {
                return Err(AppError::validation("Enter the Pushover user (or group) key"));
            }
            (
                format!("{}/1/messages.json", base(&p.server, "https://api.pushover.net")),
                "application/x-www-form-urlencoded".to_string(),
                vec![],
                format!("token={{{{secret}}}}&user={}&title={{{{title}}}}&message={{{{message}}}}&priority={{{{pushover_priority}}}}", enc(p.user.trim())),
                None,
            )
        }
        WebhookPreset::Ntfy => {
            let topic = p.topic.trim();
            if topic.is_empty() || !topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(AppError::validation("Enter an ntfy topic (letters, digits, '-' or '_')"));
            }
            let mut hs = vec![h("Title", "{{title}}"), h("Priority", "{{ntfy_priority}}"), h("Tags", "{{ntfy_tags}}")];
            if p.bearer {
                hs.push(h("Authorization", "Bearer {{secret}}"));
            }
            (format!("{}/{topic}", base(&p.server, "https://ntfy.sh")), "text/plain; charset=utf-8".to_string(), hs, "{{message}}".to_string(), None)
        }
        WebhookPreset::Discord => (
            if p.server.trim().is_empty() { "{{raw secret}}".to_string() } else { p.server.trim().to_string() },
            "application/json".to_string(),
            vec![],
            r#"{"username":"UFOP","content":"**{{title}}**","embeds":[{"title":"{{title}}","description":"{{message}}","color":{{discord_color}}}]}"#.to_string(),
            None,
        ),
        WebhookPreset::Pagerduty => (
            format!("{}/v2/enqueue", base(&p.server, "https://events.pagerduty.com")),
            "application/json".to_string(),
            vec![],
            r#"{"routing_key":"{{secret}}","event_action":"{{event_action}}","dedup_key":"ufop-{{backup_id}}","payload":{"summary":"{{title}}","source":"{{machine}}","severity":"{{pd_severity}}","component":"{{name}}","group":"ufop-backup","custom_details":{"result":"{{result}}","operation":"{{operation}}","details":"{{message}}"}}}"#.to_string(),
            Some(all),
        ),
        WebhookPreset::Gotify => {
            if p.server.trim().is_empty() {
                return Err(AppError::validation("Enter the Gotify server URL"));
            }
            (format!("{}/message?token={{{{secret}}}}", base(&p.server, "")), "application/json".to_string(), vec![], r#"{"title":"{{title}}","message":"{{message}}","priority":{{gotify_priority}}}"#.to_string(), None)
        }
        WebhookPreset::Generic => (p.server.trim().to_string(), "application/json".to_string(), vec![], String::new(), None),
    };
    let n = Notification {
        channel: Channel::Webhook { url, method: "POST".into(), content_type, headers, body_template: body, preset: kind },
        send_on: send_on.unwrap_or_else(super::default_send_on),
        credential_id: uuid::Uuid::new_v4().to_string(),
        template: String::new(),
        cert: CertPolicy::default(),
    };
    Ok(n)
}

/// A prepared request (for sending and for tests/previews).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// Default JSON body: the template variables (never the secret).
fn default_body(vars: &BTreeMap<String, String>) -> String {
    let m: serde_json::Map<String, serde_json::Value> = vars.iter().filter(|(k, _)| k.as_str() != "secret").map(|(k, v)| (k.clone(), v.clone().into())).collect();
    serde_json::Value::Object(m).to_string()
}

/// RFC 2047 for header values that are not plain ASCII (ntfy and most servers decode it).
fn header_encode(v: &str) -> String {
    if v.is_ascii() { v.to_string() } else { super::mime::encode_words(v).replace("\r\n ", " ") }
}

/// Build the request for one webhook channel. `vars` holds the run's
/// values plus `title`, `message` and (when stored) `secret`.
pub fn build(n: &Notification, vars: &BTreeMap<String, String>) -> Result<Request, AppError> {
    let Channel::Webhook { url, method, content_type, headers, body_template, preset } = &n.channel else {
        return Err(AppError::internal("not a webhook channel"));
    };
    let mut vars = vars.clone();
    let limit = preset.message_limit();
    if limit > 0 {
        if let Some(m) = vars.get_mut("message") {
            if m.chars().count() > limit {
                *m = format!("{}…", m.chars().take(limit - 1).collect::<String>());
            }
        }
    }
    let needs_secret = uses_var(url, "secret") || uses_var(body_template, "secret") || headers.iter().any(|h| uses_var(&h.value, "secret"));
    if needs_secret && vars.get("secret").map_or(true, |s| s.is_empty()) {
        return Err(AppError::validation(format!("The webhook's {} is missing; save it first", secret_label(*preset).to_lowercase())));
    }
    let method = method.to_ascii_uppercase();
    validate_url_and_method(url, &method)?;
    validate_headers(headers)?;
    let full_url = render(url, &vars, Escape::Url);
    let parsed = url::Url::parse(&full_url).map_err(|_| AppError::validation("The webhook URL is not a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::validation("The webhook URL must start with http:// or https://"));
    }
    let ct = if content_type.trim().is_empty() { "application/json" } else { content_type.trim() };
    let lc = ct.to_ascii_lowercase();
    let esc = if lc.contains("json") {
        Escape::Json
    } else if lc.starts_with("application/x-www-form-urlencoded") {
        Escape::Form
    } else if lc.contains("html") {
        Escape::Html
    } else {
        Escape::None
    };
    let mut out_headers = Vec::new();
    for h in headers {
        let v = render(&h.value, &vars, Escape::Header);
        out_headers.push((h.name.trim().to_string(), header_encode(&header_safe(&v))));
    }
    let body = if method == "GET" {
        None
    } else {
        out_headers.push(("Content-Type".into(), ct.to_string()));
        Some(if body_template.trim().is_empty() { if esc == Escape::Json { default_body(&vars) } else { vars.get("message").cloned().unwrap_or_default() } } else { render(body_template, &vars, esc) })
    };
    Ok(Request { method, url: full_url, headers: out_headers, body })
}

/// Whether this run's result is sent at all (PagerDuty only resolves after a problem).
pub fn wants(n: &Notification, result: &str, previous: Option<&str>) -> bool {
    match &n.channel {
        Channel::Webhook { preset: WebhookPreset::Pagerduty, .. } if result == "success" => previous.is_some_and(|p| matches!(p, "error" | "warning")),
        _ => true,
    }
}

async fn send_one(req: &Request, cert: &CertPolicy) -> Result<(), AppError> {
    let http = super::webhook_client(cert)?;
    let method = reqwest::Method::from_bytes(req.method.as_bytes()).unwrap_or(reqwest::Method::POST);
    let mut r = http.request(method, &req.url);
    for (k, v) in &req.headers {
        r = r.header(k.as_str(), v.as_str());
    }
    if let Some(b) = &req.body {
        r = r.body(b.clone());
    }
    // Error texts never carry the URL: it may hold a token.
    let host = url::Url::parse(&req.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let what = format!("Webhook {host}");
    let resp = r.send().await.map_err(|e| crate::backup_engine::stores::http::net(&what, e.without_url()))?;
    crate::backup_engine::stores::http::check(resp, &what).await?;
    Ok(())
}

/// Send to one webhook channel. A `test` to PagerDuty triggers and then
/// resolves a test incident.
pub async fn send(n: &Notification, vars: &BTreeMap<String, String>) -> Result<(), AppError> {
    let pd = matches!(n.channel, Channel::Webhook { preset: WebhookPreset::Pagerduty, .. });
    if pd && vars.get("result").is_some_and(|r| r == "test") {
        let mut v = vars.clone();
        v.insert("event_action".into(), "trigger".into());
        send_one(&build(n, &v)?, &n.cert).await?;
        v.insert("event_action".into(), "resolve".into());
        return send_one(&build(n, &v)?, &n.cert).await;
    }
    send_one(&build(n, vars)?, &n.cert).await
}
