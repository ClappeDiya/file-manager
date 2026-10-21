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
}

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

fn default_send_on() -> Vec<String> {
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
    let _ = rustls::crypto::ring::default_provider().install_default();
    if cert.accept_invalid_certs {
        #[derive(Debug)]
        struct AcceptAll;
        impl rustls::client::danger::ServerCertVerifier for AcceptAll {
            fn verify_server_cert(&self, _: &rustls::pki_types::CertificateDer<'_>, _: &[rustls::pki_types::CertificateDer<'_>], _: &rustls::pki_types::ServerName<'_>, _: &[u8], _: rustls::pki_types::UnixTime) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            fn verify_tls12_signature(&self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn verify_tls13_signature(&self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
            }
        }
        return Ok(Arc::new(rustls::ClientConfig::builder().dangerous().with_custom_certificate_verifier(Arc::new(AcceptAll)).with_no_client_auth()));
    }
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut pinned: Vec<Vec<u8>> = Vec::new();
    if let Some(pem) = cert.ca_pem.as_deref().filter(|p| !p.trim().is_empty()) {
        for c in rustls_pemfile::certs(&mut pem.as_bytes()).flatten() {
            pinned.push(c.as_ref().to_vec());
            // A CA certificate also works as an extra root; a self-signed
            // leaf does not, which the pin below covers.
            let _ = roots.add(c);
        }
        if pinned.is_empty() {
            return Err(AppError::validation("The trusted certificate is not valid PEM"));
        }
    }
    let webpki = rustls::client::WebPkiServerVerifier::builder(Arc::new(roots)).build().map_err(|e| AppError::internal(format!("TLS setup: {e}")))?;
    if pinned.is_empty() {
        return Ok(Arc::new(rustls::ClientConfig::builder().with_webpki_verifier(webpki).with_no_client_auth()));
    }
    /// Accepts the exact pinned certificate(s); otherwise normal validation.
    #[derive(Debug)]
    struct Pinned {
        pins: Vec<Vec<u8>>,
        inner: Arc<rustls::client::WebPkiServerVerifier>,
    }
    impl rustls::client::danger::ServerCertVerifier for Pinned {
        fn verify_server_cert(&self, ee: &rustls::pki_types::CertificateDer<'_>, im: &[rustls::pki_types::CertificateDer<'_>], name: &rustls::pki_types::ServerName<'_>, ocsp: &[u8], now: rustls::pki_types::UnixTime) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            if self.pins.iter().any(|p| p.as_slice() == ee.as_ref()) {
                return Ok(rustls::client::danger::ServerCertVerified::assertion());
            }
            self.inner.verify_server_cert(ee, im, name, ocsp, now)
        }
        fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            self.inner.verify_tls12_signature(m, c, d)
        }
        fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            self.inner.verify_tls13_signature(m, c, d)
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.inner.supported_verify_schemes()
        }
    }
    Ok(Arc::new(rustls::ClientConfig::builder().dangerous().with_custom_certificate_verifier(Arc::new(Pinned { pins: pinned, inner: webpki })).with_no_client_auth()))
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

/// RFC 2047 encoding for non-ASCII header values.
fn header_value(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(s))
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn send_email(host: &str, port: Option<u16>, tls: TlsMode, username: &str, password: Option<&str>, from: &str, to: &[String], subject: &str, body: &str, cert: &CertPolicy) -> Result<(), AppError> {
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
    let mut msg = format!(
        "From: {from}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}@{domain}>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n",
        to.join(", "),
        header_value(subject),
        chrono::Utc::now().to_rfc2822(),
        uuid::Uuid::new_v4().simple()
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    for chunk in b64.as_bytes().chunks(76) {
        msg.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        msg.push_str("\r\n");
    }
    msg.push('.');
    s.cmd(&msg, 250, "message").await?;
    let _ = s.cmd("QUIT", 221, "QUIT").await;
    Ok(())
}

// ── Telegram ──

pub async fn send_telegram(api_base: &str, token: &str, chat_id: &str, text: &str, cert: &CertPolicy) -> Result<(), AppError> {
    let base = if api_base.is_empty() { "https://api.telegram.org" } else { api_base.trim_end_matches('/') };
    let http = super::stores::http::client(&super::stores::http::TlsOptions { accept_invalid_certs: cert.accept_invalid_certs, ca_pem: cert.ca_pem.clone() })?;
    // Telegram limits messages to 4096 characters.
    let text: String = text.chars().take(4000).collect();
    let r = http.post(format!("{base}/bot{token}/sendMessage")).json(&serde_json::json!({ "chat_id": chat_id, "text": text, "disable_web_page_preview": true })).send().await.map_err(|e| super::stores::http::net("Telegram", e))?;
    let v: serde_json::Value = super::stores::http::check(r, "Telegram").await?.json().await.map_err(|e| super::stores::http::net("Telegram", e))?;
    if v["ok"] != true {
        return Err(AppError::connection(format!("Telegram refused the message: {}", v["description"]), "Check the bot token and chat id."));
    }
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

/// Deliver to every channel whose filter matches; returns error lines.
pub async fn deliver(list: &[(Notification, Option<String>)], result: &str, subject_vars: &BTreeMap<String, String>, body: &str) -> Vec<String> {
    let mut errors = Vec::new();
    for (n, secret) in list {
        if !n.send_on.iter().any(|s| s == result) {
            continue;
        }
        let text = if n.template.is_empty() { body.to_string() } else { format!("{}\n\n{body}", crate::backup_engine::hooks::render_template(&n.template, subject_vars)) };
        let r = match &n.channel {
            Channel::Email { host, port, tls, username, from, to, subject } => {
                send_email(host, *port, *tls, username, secret.as_deref(), from, to, &crate::backup_engine::hooks::render_template(subject, subject_vars), &text, &n.cert).await
            }
            Channel::Telegram { chat_id, api_base } => match secret {
                Some(t) => send_telegram(api_base, t, chat_id, &text, &n.cert).await,
                None => Err(AppError::validation("The Telegram bot token is missing")),
            },
            Channel::Xmpp { jid, to, host, port, tls } => send_xmpp(jid, secret.as_deref().unwrap_or_default(), to, host, *port, *tls, &text, &n.cert).await,
        };
        if let Err(e) = r {
            errors.push(e.user_message());
        }
    }
    errors
}

#[cfg(test)]
mod tests;
