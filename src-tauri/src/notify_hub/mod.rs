//! Result notifications beyond HTTP callbacks: SMTP email, Telegram and
//! XMPP (Jabber). All channels share message templates, custom fields,
//! the result filter (`send_on`) and optional path redaction.
//!
//! SMTP and XMPP are small native clients over TCP with STARTTLS (or
//! implicit TLS), so no extra mail/chat libraries are needed.

use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// How a channel secures its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    /// Upgrade with STARTTLS (required).
    #[default]
    StartTls,
    /// TLS from the first byte (SMTPS 465, XMPP direct TLS).
    Implicit,
    /// Plain text (local relays only).
    None,
}

/// Certificate policy shared by channels and HTTP callbacks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CertPolicy {
    pub accept_invalid_certs: bool,
    /// Extra trusted CA / self-signed certificate (PEM).
    pub ca_pem: Option<String>,
    /// SHA-256 fingerprints; when set, only these certificates are accepted.
    pub pinned_sha256: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Channel {
    Email {
        host: String,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        tls: TlsMode,
        #[serde(default)]
        username: String,
        from: String,
        to: Vec<String>,
        #[serde(default = "default_subject")]
        subject: String,
        /// Send text + HTML (`multipart/alternative`). Off for channels from
        /// before HTML mail; new channels turn it on.
        #[serde(default)]
        html: bool,
        /// HTML body template (empty = the built-in report table); values are HTML-escaped.
        #[serde(default)]
        html_template: String,
    },
    Telegram {
        chat_id: String,
        /// Bot API base (tests / self-hosted); empty = `https://api.telegram.org`.
        #[serde(default)]
        api_base: String,
    },
    Xmpp {
        /// Sender account, `user@domain`.
        jid: String,
        /// Recipient JID.
        to: String,
        /// Server host when it differs from the JID domain.
        #[serde(default)]
        host: String,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        tls: TlsMode,
    },
    /// Slack incoming webhook. The URL is a secret: it lives in the
    /// credential store under `credential_id`.
    Slack {},
    /// Microsoft Teams Workflows ("Post to a channel when a webhook request
    /// is received") or legacy incoming-webhook URL, stored like Slack's.
    Teams {},
    /// Any HTTP endpoint, with custom method, headers and body template;
    /// presets fill it in for Pushover, ntfy, Discord, PagerDuty and Gotify.
    /// The channel secret (`{{secret}}`) lives under `credential_id`.
    Webhook {
        url: String,
        #[serde(default = "default_method")]
        method: String,
        /// Empty = `application/json`.
        #[serde(default)]
        content_type: String,
        #[serde(default)]
        headers: Vec<webhook::WebhookHeader>,
        /// Empty = a JSON object of the run's values.
        #[serde(default)]
        body_template: String,
        #[serde(default)]
        preset: webhook::WebhookPreset,
    },
}

fn default_method() -> String {
    "POST".into()
}

pub mod desktop;
pub mod mime;
pub mod profiles;
pub mod run;
pub mod webhook;

fn default_subject() -> String {
    "UFOP {{operation}} {{result}}: {{name}}".into()
}

/// A configured channel with its own result filter and secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    #[serde(flatten)]
    pub channel: Channel,
    /// Results that trigger it (`success`, `warning`, `error`).
    #[serde(default = "default_send_on")]
    pub send_on: Vec<String>,
    /// Credential-store id holding the password / bot token.
    #[serde(default)]
    pub credential_id: String,
    /// Body template; empty = the job's report template.
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub cert: CertPolicy,
}

pub(crate) fn default_send_on() -> Vec<String> {
    vec!["warning".into(), "error".into()]
}

/// Replace absolute paths in `text` with short stable placeholders.
pub fn redact_paths(text: &str, roots: &[String]) -> String {
    let mut out = text.to_string();
    let mut roots: Vec<&String> = roots.iter().filter(|r| r.len() > 1).collect();
    roots.sort_by_key(|r| std::cmp::Reverse(r.len()));
    for r in roots {
        let tag = format!("[source:{}]", &ufop_core::backup::hash_file_name_key(r)[..8]);
        out = out.replace(r.as_str(), &tag);
    }
    // Remaining absolute paths (outside the sources): keep only the file name.
    let re = regex::Regex::new(r#"(?:[A-Za-z]:\\|/)(?:[^\s"'<>|:*?\\/]+[\\/])+([^\s"'<>|:*?\\/]+)"#).expect("valid regex");
    re.replace_all(&out, "…/$1").into_owned()
}

pub(crate) fn tls_config(cert: &CertPolicy) -> Result<Arc<rustls::ClientConfig>, AppError> {
    crate::security::tls::client_config(cert.accept_invalid_certs, cert.ca_pem.as_deref(), &cert.pinned_sha256).map(Arc::new)
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn wrap_tls(stream: Box<dyn Io>, host: &str, cert: &CertPolicy) -> Result<Box<dyn Io>, AppError> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| AppError::validation(format!("Invalid server name '{host}'")))?;
    let conn = tokio_rustls::TlsConnector::from(tls_config(cert)?);
    let s = conn.connect(name, stream).await.map_err(|e| AppError::connection(format!("TLS with {host} failed: {e}"), "Check the certificate, or set a trusted CA."))?;
    Ok(Box::new(s))
}

async fn connect(host: &str, port: u16) -> Result<Box<dyn Io>, AppError> {
    let s = tokio::time::timeout(Duration::from_secs(20), TcpStream::connect((host, port)))
        .await
        .map_err(|_| AppError::connection(format!("Connecting to {host}:{port} timed out"), "Check the host and port."))?
        .map_err(|e| AppError::connection(format!("Connecting to {host}:{port}: {e}"), "Check the host and port."))?;
    Ok(Box::new(s))
}

// ── SMTP ──

struct Smtp {
    io: Box<dyn Io>,
    buf: Vec<u8>,
}

impl Smtp {
    /// Read one (possibly multi-line) reply: (code, text).
    async fn reply(&mut self) -> Result<(u16, String), AppError> {
        let mut lines = Vec::new();
        loop {
            while !self.buf.windows(2).any(|w| w == b"\r\n") {
                let mut chunk = [0u8; 4096];
                let n = tokio::time::timeout(Duration::from_secs(60), self.io.read(&mut chunk)).await.map_err(|_| AppError::connection("SMTP server timed out", "Retry later."))?.map_err(|e| AppError::connection(format!("SMTP read: {e}"), "Retry."))?;
                if n == 0 {
                    return Err(AppError::connection("SMTP server closed the connection", "Check the server settings."));
                }
                self.buf.extend_from_slice(&chunk[..n]);
            }
            let pos = self.buf.windows(2).position(|w| w == b"\r\n").unwrap_or(0);
            let line = String::from_utf8_lossy(&self.buf[..pos]).into_owned();
            self.buf.drain(..pos + 2);
            let done = line.len() < 4 || line.as_bytes()[3] != b'-';
            lines.push(line);
            if done {
                break;
            }
        }
        let code = lines.last().and_then(|l| l.get(..3)).and_then(|c| c.parse().ok()).unwrap_or(0);
        Ok((code, lines.join("\n")))
    }

    async fn cmd(&mut self, line: &str, expect: u16, what: &str) -> Result<String, AppError> {
        self.io.write_all(format!("{line}\r\n").as_bytes()).await.map_err(|e| AppError::connection(format!("SMTP write: {e}"), "Retry."))?;
        let (code, text) = self.reply().await?;
        if code / 100 != expect / 100 {
            return Err(AppError::connection(format!("SMTP {what} rejected: {text}"), "Check the sender, recipients and credentials."));
        }
        Ok(text)
    }
}

/// Send a mail: text only, or text + HTML (`multipart/alternative`) when `html` is set.
#[allow(clippy::too_many_arguments)]
pub async fn send_email(host: &str, port: Option<u16>, tls: TlsMode, username: &str, password: Option<&str>, from: &str, to: &[String], subject: &str, body: &str, html: Option<&str>, cert: &CertPolicy) -> Result<(), AppError> {
    if to.is_empty() {
        return Err(AppError::validation("Add at least one email recipient"));
    }
    let port = port.unwrap_or(match tls {
        TlsMode::Implicit => 465,
        TlsMode::StartTls => 587,
        TlsMode::None => 25,
    });
    let mut io = connect(host, port).await?;
    if tls == TlsMode::Implicit {
        io = wrap_tls(io, host, cert).await?;
    }
    let mut s = Smtp { io, buf: Vec::new() };
    let (code, text) = s.reply().await?;
    if code != 220 {
        return Err(AppError::connection(format!("SMTP greeting: {text}"), "Check the server."));
    }
    let me = crate::backup_engine::hooks::machine_name();
    let mut caps = s.cmd(&format!("EHLO {me}"), 250, "EHLO").await?;
    if tls == TlsMode::StartTls {
        if !caps.to_uppercase().contains("STARTTLS") {
            return Err(AppError::connection("The SMTP server does not offer STARTTLS", "Use implicit TLS (port 465) or, for a local relay only, no encryption."));
        }
        s.cmd("STARTTLS", 220, "STARTTLS").await?;
        let plain = std::mem::replace(&mut s.io, Box::new(tokio::io::duplex(1).0));
        s.io = wrap_tls(plain, host, cert).await?;
        s.buf.clear();
        caps = s.cmd(&format!("EHLO {me}"), 250, "EHLO").await?;
    }
    if !username.is_empty() {
        let pass = password.unwrap_or_default();
        let up = caps.to_uppercase();
        if up.contains("AUTH") && up.contains("PLAIN") {
            let token = base64::engine::general_purpose::STANDARD.encode(format!("\0{username}\0{pass}"));
            s.cmd(&format!("AUTH PLAIN {token}"), 235, "authentication").await?;
        } else {
            s.cmd("AUTH LOGIN", 334, "authentication").await?;
            s.cmd(&base64::engine::general_purpose::STANDARD.encode(username), 334, "authentication").await?;
            s.cmd(&base64::engine::general_purpose::STANDARD.encode(pass), 235, "authentication").await?;
        }
    }
    let addr = |a: &str| a.rsplit('<').next().unwrap_or(a).trim_end_matches('>').trim().to_string();
    s.cmd(&format!("MAIL FROM:<{}>", addr(from)), 250, "sender").await?;
    for r in to {
        s.cmd(&format!("RCPT TO:<{}>", addr(r)), 250, "recipient").await?;
    }
    s.cmd("DATA", 354, "DATA").await?;
    let domain = addr(from).rsplit('@').next().unwrap_or("localhost").to_string();
    let mail = mime::Mail { from, to, subject, text: body, html, domain: &domain };
    let mut msg = mime::build_message(&mail, &chrono::Utc::now().to_rfc2822(), &uuid::Uuid::new_v4().simple().to_string(), &format!("=_ufop_{}", uuid::Uuid::new_v4().simple()));
    msg.push('.');
    s.cmd(&msg, 250, "message").await?;
    let _ = s.cmd("QUIT", 221, "QUIT").await;
    Ok(())
}

// ── Telegram ──

pub async fn send_telegram(api_base: &str, token: &str, chat_id: &str, text: &str, cert: &CertPolicy) -> Result<(), AppError> {
    let base = if api_base.is_empty() { "https://api.telegram.org" } else { api_base.trim_end_matches('/') };
    let http = crate::backup_engine::stores::http::client(&crate::backup_engine::stores::http::TlsOptions { accept_invalid_certs: cert.accept_invalid_certs, ca_pem: cert.ca_pem.clone(), pinned_sha256: cert.pinned_sha256.clone() })?;
    // Telegram limits messages to 4096 characters.
    let text: String = text.chars().take(4000).collect();
    let r = http.post(format!("{base}/bot{token}/sendMessage")).json(&serde_json::json!({ "chat_id": chat_id, "text": text, "disable_web_page_preview": true })).send().await.map_err(|e| crate::backup_engine::stores::http::net("Telegram", e))?;
    let v: serde_json::Value = crate::backup_engine::stores::http::check(r, "Telegram").await?.json().await.map_err(|e| crate::backup_engine::stores::http::net("Telegram", e))?;
    if v["ok"] != true {
        return Err(AppError::connection(format!("Telegram refused the message: {}", v["description"]), "Check the bot token and chat id."));
    }
    Ok(())
}

// ── Slack and Microsoft Teams webhooks ──

fn webhook_client(cert: &CertPolicy) -> Result<reqwest::Client, AppError> {
    crate::backup_engine::stores::http::client(&crate::backup_engine::stores::http::TlsOptions {
        accept_invalid_certs: cert.accept_invalid_certs,
        ca_pem: cert.ca_pem.clone(),
        pinned_sha256: cert.pinned_sha256.clone(),
    })
}

fn check_webhook_url(url: &str, service: &str) -> Result<(), AppError> {
    let u = url::Url::parse(url).map_err(|_| AppError::validation(format!("The {service} webhook URL is not a valid URL")))?;
    if u.scheme() != "https" && !matches!(u.host_str(), Some("127.0.0.1" | "localhost")) {
        return Err(AppError::validation(format!("The {service} webhook URL must use https")));
    }
    Ok(())
}

/// Post `title` and `text` to a Slack incoming webhook.
pub async fn send_slack(webhook_url: &str, title: &str, text: &str, cert: &CertPolicy) -> Result<(), AppError> {
    check_webhook_url(webhook_url, "Slack")?;
    let text: String = format!("*{title}*\n{text}").chars().take(39_000).collect();
    let r = webhook_client(cert)?
        .post(webhook_url)
        .json(&serde_json::json!({ "text": text, "mrkdwn": true }))
        .send()
        .await
        .map_err(|e| crate::backup_engine::stores::http::net("Slack", e))?;
    crate::backup_engine::stores::http::check(r, "Slack").await?;
    Ok(())
}

/// The Adaptive Card Teams Workflows expects (also accepted by legacy
/// incoming webhooks).
pub fn teams_payload(title: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "contentUrl": null,
            "content": {
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "type": "AdaptiveCard",
                "version": "1.4",
                "body": [
                    { "type": "TextBlock", "text": title, "weight": "Bolder", "size": "Medium", "wrap": true },
                    { "type": "TextBlock", "text": text.chars().take(20_000).collect::<String>(), "wrap": true }
                ]
            }
        }]
    })
}

/// Post an Adaptive Card to a Microsoft Teams webhook.
pub async fn send_teams(webhook_url: &str, title: &str, text: &str, cert: &CertPolicy) -> Result<(), AppError> {
    check_webhook_url(webhook_url, "Teams")?;
    let r = webhook_client(cert)?
        .post(webhook_url)
        .json(&teams_payload(title, text))
        .send()
        .await
        .map_err(|e| crate::backup_engine::stores::http::net("Teams", e))?;
    crate::backup_engine::stores::http::check(r, "Teams").await?;
    Ok(())
}

// ── XMPP ──

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

struct Xmpp {
    io: Box<dyn Io>,
    buf: String,
}

impl Xmpp {
    async fn send(&mut self, s: &str) -> Result<(), AppError> {
        self.io.write_all(s.as_bytes()).await.map_err(|e| AppError::connection(format!("XMPP write: {e}"), "Retry."))
    }

    /// Read until one of `marks` appears; returns the buffered text.
    async fn until(&mut self, marks: &[&str]) -> Result<String, AppError> {
        loop {
            if let Some(m) = marks.iter().filter_map(|m| self.buf.find(m).map(|i| i + m.len())).min() {
                // Include the rest of the closing element.
                let end = self.buf[m..].find('>').map(|j| m + j + 1).unwrap_or(m);
                let out: String = self.buf.drain(..end.min(self.buf.len())).collect();
                return Ok(out);
            }
            if self.buf.contains("<failure") || self.buf.contains("<stream:error") || self.buf.contains("type='error'") || self.buf.contains("type=\"error\"") {
                return Err(AppError::connection(format!("XMPP server error: {}", self.buf.chars().take(300).collect::<String>()), "Check the account and recipient."));
            }
            let mut chunk = [0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(30), self.io.read(&mut chunk)).await.map_err(|_| AppError::connection("XMPP server timed out", "Retry."))?.map_err(|e| AppError::connection(format!("XMPP read: {e}"), "Retry."))?;
            if n == 0 {
                return Err(AppError::connection("XMPP server closed the connection", "Check the server settings."));
            }
            self.buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    }

    async fn open(&mut self, domain: &str) -> Result<String, AppError> {
        self.buf.clear();
        self.send(&format!("<?xml version='1.0'?><stream:stream to='{}' xmlns='jabber:client' xmlns:stream='http://etherx.jabber.org/streams' version='1.0'>", xml_escape(domain))).await?;
        self.until(&["</stream:features>"]).await
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn send_xmpp(jid: &str, password: &str, to: &str, host: &str, port: Option<u16>, tls: TlsMode, text: &str, cert: &CertPolicy) -> Result<(), AppError> {
    let (user, domain) = jid.split_once('@').ok_or_else(|| AppError::validation("The XMPP account must look like user@domain"))?;
    let domain = domain.split('/').next().unwrap_or(domain);
    let server = if host.is_empty() { domain } else { host };
    let port = port.unwrap_or(if tls == TlsMode::Implicit { 5223 } else { 5222 });
    let mut io = connect(server, port).await?;
    if tls == TlsMode::Implicit {
        io = wrap_tls(io, domain, cert).await?;
    }
    let mut x = Xmpp { io, buf: String::new() };
    let mut features = x.open(domain).await?;
    if tls == TlsMode::StartTls {
        if !features.contains("starttls") {
            return Err(AppError::connection("The XMPP server does not offer STARTTLS", "Use direct TLS or another server."));
        }
        x.send("<starttls xmlns='urn:ietf:params:xml:ns:xmpp-tls'/>").await?;
        x.until(&["<proceed"]).await?;
        let plain = std::mem::replace(&mut x.io, Box::new(tokio::io::duplex(1).0));
        x.io = wrap_tls(plain, domain, cert).await?;
        features = x.open(domain).await?;
    }
    if !features.contains("PLAIN") {
        return Err(AppError::connection("The XMPP server offers no PLAIN login", "Enable SASL PLAIN over TLS on the server."));
    }
    let token = base64::engine::general_purpose::STANDARD.encode(format!("\0{user}\0{password}"));
    x.send(&format!("<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'>{token}</auth>")).await?;
    x.until(&["<success"]).await?;
    x.open(domain).await?;
    x.send("<iq type='set' id='bind1'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'><resource>ufop</resource></bind></iq>").await?;
    x.until(&["bind1"]).await?;
    x.send("<iq type='set' id='sess1'><session xmlns='urn:ietf:params:xml:ns:xmpp-session'/></iq>").await?;
    let _ = x.until(&["sess1"]).await;
    x.send(&format!("<message to='{}' type='chat' id='{}'><body>{}</body></message>", xml_escape(to), uuid::Uuid::new_v4().simple(), xml_escape(text))).await?;
    // Ping round-trip so the message is processed before we close.
    x.send("<iq type='get' id='ping1'><ping xmlns='urn:xmpp:ping'/></iq>").await?;
    let _ = x.until(&["ping1"]).await;
    let _ = x.send("</stream:stream>").await;
    Ok(())
}

/// Variables a channel renders with: the run's values plus `title`,
/// `message`, the preset values ([`webhook::result_vars`]) and the secret.
fn channel_vars(vars: &BTreeMap<String, String>, result: &str, title: &str, text: &str, secret: Option<&str>) -> BTreeMap<String, String> {
    let mut v = vars.clone();
    for (k, x) in webhook::result_vars(result, vars.get("previous_result").map(String::as_str).filter(|p| !p.is_empty())) {
        v.entry(k.to_string()).or_insert(x);
    }
    v.insert("title".into(), title.to_string());
    v.insert("message".into(), text.to_string());
    if let Some(s) = secret {
        v.insert("secret".into(), s.to_string());
    }
    v
}

/// What a channel would send (subject/title, text and, for HTML mail, the
/// HTML body) — for previews; nothing is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    /// Webhook channels: the request line, headers (secret ones masked) and body.
    pub request: Option<String>,
}

fn message_text(n: &Notification, vars: &BTreeMap<String, String>, body: &str) -> String {
    if n.template.is_empty() { body.to_string() } else { format!("{}\n\n{body}", crate::backup_engine::hooks::render_template(&n.template, vars)) }
}

fn title_of(n: &Notification, vars: &BTreeMap<String, String>) -> String {
    let t = match &n.channel {
        Channel::Email { subject, .. } => subject.clone(),
        _ => default_subject(),
    };
    crate::backup_engine::hooks::render_template(&t, vars)
}

/// Render what `n` would send for a run with `vars` and report `body`.
pub fn preview(n: &Notification, result: &str, vars: &BTreeMap<String, String>, body: &str) -> Preview {
    let text = message_text(n, vars, body);
    let subject = title_of(n, vars);
    let cv = channel_vars(vars, result, &subject, &text, None);
    let html = match &n.channel {
        Channel::Email { html: true, html_template, .. } => Some(mime::render_html(html_template, &cv)),
        _ => None,
    };
    let request = matches!(n.channel, Channel::Webhook { .. }).then(|| {
        let mut masked = cv.clone();
        masked.insert("secret".into(), "******".into());
        let mut n2 = n.clone();
        if let Channel::Webhook { headers, .. } = &mut n2.channel {
            for h in headers.iter_mut().filter(|h| h.secret) {
                h.value = "******".into();
            }
        }
        match webhook::build(&n2, &masked) {
            Ok(r) => {
                let mut s = format!("{} {}\n", r.method, r.url);
                for (k, v) in &r.headers {
                    s.push_str(&format!("{k}: {v}\n"));
                }
                if let Some(b) = r.body {
                    s.push('\n');
                    s.push_str(&b);
                }
                s
            }
            Err(e) => e.user_message(),
        }
    });
    Preview { subject, text, html, request }
}

impl Notification {
    /// Check the channel's settings (not its secret).
    pub fn validate(&self) -> Result<(), AppError> {
        match &self.channel {
            Channel::Email { host, from, to, .. } if host.trim().is_empty() || from.trim().is_empty() || to.is_empty() => {
                Err(AppError::validation("Email notifications need a server, a sender and at least one recipient"))
            }
            Channel::Telegram { chat_id, .. } if chat_id.trim().is_empty() => Err(AppError::validation("Telegram notifications need a chat id")),
            Channel::Xmpp { jid, to, .. } if !jid.contains('@') || to.trim().is_empty() => Err(AppError::validation("XMPP notifications need an account (user@domain) and a recipient")),
            Channel::Webhook { url, method, headers, .. } => {
                webhook::validate_url_and_method(url, method)?;
                webhook::validate_headers(headers)
            }
            _ => Ok(()),
        }
    }

    /// Credential-store ids this channel uses (its secret and secret headers).
    pub fn credential_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = Some(self.credential_id.clone()).into_iter().filter(|c| !c.is_empty()).collect();
        if let Channel::Webhook { headers, .. } = &self.channel {
            v.extend(headers.iter().filter(|h| h.secret && !h.credential_id.is_empty()).map(|h| h.credential_id.clone()));
        }
        v
    }
}

/// Deliver to every channel whose filter matches; returns error lines.
/// Secret webhook headers must already hold their values (the manager
/// fills them from the credential store).
pub async fn deliver(list: &[(Notification, Option<String>)], result: &str, subject_vars: &BTreeMap<String, String>, body: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let previous = subject_vars.get("previous_result").map(String::as_str).filter(|p| !p.is_empty());
    for (n, secret) in list {
        if !n.send_on.iter().any(|s| s == result) || !webhook::wants(n, result, previous) {
            continue;
        }
        let text = message_text(n, subject_vars, body);
        let r = match &n.channel {
            Channel::Email { host, port, tls, username, from, to, html, html_template, .. } => {
                let subject = title_of(n, subject_vars);
                let page = html.then(|| mime::render_html(html_template, &channel_vars(subject_vars, result, &subject, &text, None)));
                send_email(host, *port, *tls, username, secret.as_deref(), from, to, &subject, &text, page.as_deref(), &n.cert).await
            }
            Channel::Telegram { chat_id, api_base } => match secret {
                Some(t) => send_telegram(api_base, t, chat_id, &text, &n.cert).await,
                None => Err(AppError::validation("The Telegram bot token is missing")),
            },
            Channel::Xmpp { jid, to, host, port, tls } => send_xmpp(jid, secret.as_deref().unwrap_or_default(), to, host, *port, *tls, &text, &n.cert).await,
            Channel::Slack {} | Channel::Teams {} => {
                let title = crate::backup_engine::hooks::render_template(&default_subject(), subject_vars);
                match (secret, &n.channel) {
                    (Some(url), Channel::Slack {}) => send_slack(url, &title, &text, &n.cert).await,
                    (Some(url), _) => send_teams(url, &title, &text, &n.cert).await,
                    (None, _) => Err(AppError::validation("The webhook URL is missing")),
                }
            }
            Channel::Webhook { .. } => {
                let title = title_of(n, subject_vars);
                webhook::send(n, &channel_vars(subject_vars, result, &title, &text, secret.as_deref())).await
            }
        };
        if let Err(e) = r {
            errors.push(e.user_message());
        }
    }
    errors
}

#[cfg(test)]
mod tests;
