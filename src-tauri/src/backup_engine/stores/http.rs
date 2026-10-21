//! Shared HTTP plumbing for cloud stores.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// TLS / transport options common to HTTP providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsOptions {
    /// Accept self-signed or otherwise invalid certificates (test setups only).
    pub accept_invalid_certs: bool,
    /// Extra PEM CA certificate to trust (e.g. a private S3 endpoint).
    pub ca_pem: Option<String>,
    /// SHA-256 certificate fingerprints; when set, only these are accepted.
    pub pinned_sha256: Vec<String>,
}

pub fn client(tls: &TlsOptions) -> Result<reqwest::Client, AppError> {
    if !tls.pinned_sha256.is_empty() {
        let cfg = crate::security::tls::client_config(false, None, &tls.pinned_sha256)?;
        return reqwest::Client::builder()
            .use_preconfigured_tls(cfg)
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::internal(format!("HTTP client: {e}")));
    }
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(tls.accept_invalid_certs);
    if let Some(pem) = tls.ca_pem.as_deref().filter(|p| !p.trim().is_empty()) {
        let cert = reqwest::Certificate::from_pem(pem.as_bytes()).map_err(|e| AppError::validation(format!("Invalid CA certificate: {e}")))?;
        b = b.add_root_certificate(cert);
    }
    b.build().map_err(|e| AppError::internal(format!("HTTP client: {e}")))
}

pub fn net(what: &str, e: reqwest::Error) -> AppError {
    AppError::connection(format!("{what}: {e}"), "Check the network connection and the destination settings.")
}

/// Turn a non-success response into a readable error (body excerpt kept).
pub async fn check(resp: reqwest::Response, what: &str) -> Result<reqwest::Response, AppError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let excerpt: String = body.chars().take(400).collect();
    Err(match status.as_u16() {
        401 | 403 => AppError::connection(format!("{what} was refused ({status}): {excerpt}"), "Check the credentials and their permissions."),
        404 => AppError::not_found(format!("{what}: not found {NOT_FOUND_TAG} {excerpt}")),
        429 | 503 => AppError::connection(format!("{what}: service busy ({status})"), "The service is rate-limiting; retries will back off."),
        _ => AppError::connection(format!("{what} failed ({status}): {excerpt}"), "Check the destination settings."),
    })
}

/// Marks 404 errors from [`check`] so callers can tell "missing" apart.
const NOT_FOUND_TAG: &str = "[404]";

/// `true` for a 404 error from [`check`].
pub fn is_not_found(e: &AppError) -> bool {
    e.user_message().contains(NOT_FOUND_TAG)
}

/// Text between `<tag>` and `</tag>` for each occurrence.
pub fn xml_all(xml: &str, tag: &str) -> Vec<String> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(after[..j].to_string());
        rest = &after[j + close.len()..];
    }
    out
}

pub fn xml_one(xml: &str, tag: &str) -> Option<String> {
    xml_all(xml, tag).into_iter().next()
}

/// Minimal XML entity decoding for keys returned by object stores.
pub fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}
