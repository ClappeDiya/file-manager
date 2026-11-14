//! Shared HTTP plumbing for cloud stores.

use crate::connectors::google_errors::{classify_403, Forbidden};
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
    builder(tls)?.build().map_err(|e| AppError::internal(format!("HTTP client: {e}")))
}

/// A client builder with the TLS options applied (for callers that need more, e.g. a cookie jar).
pub fn builder(tls: &TlsOptions) -> Result<reqwest::ClientBuilder, AppError> {
    if !tls.pinned_sha256.is_empty() {
        let cfg = crate::security::tls::client_config(false, None, &tls.pinned_sha256)?;
        return Ok(crate::net::builder().use_preconfigured_tls(cfg).connect_timeout(Duration::from_secs(30)));
    }
    let mut b = crate::net::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(tls.accept_invalid_certs);
    if let Some(pem) = tls.ca_pem.as_deref().filter(|p| !p.trim().is_empty()) {
        let cert = reqwest::Certificate::from_pem(pem.as_bytes()).map_err(|e| AppError::validation(format!("Invalid CA certificate: {e}")))?;
        b = b.add_root_certificate(cert);
    }
    Ok(b)
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
        // Google answers rate limits and quotas with 403 + a reason; those
        // back off and retry like a 429.
        403 if classify_403(&body).0 == Forbidden::Throttled => {
            AppError::connection(format!("{what}: service is limiting requests ({status}): {excerpt}"), "The service is rate-limiting; retries will back off.")
        }
        // Rejected credentials: not retried (ufop_core::retry::is_retryable).
        401 | 403 => AppError::auth(format!("{what} was refused ({status}): {excerpt}"), "Check the credentials and their permissions."),
        404 => AppError::not_found(format!("{what}: not found {NOT_FOUND_TAG} {excerpt}")),
        429 | 503 => AppError::connection(format!("{what}: service busy ({status})"), "The service is rate-limiting; retries will back off."),
        _ => AppError::connection(format!("{what} failed ({status}): {excerpt}"), "Check the destination settings."),
    })
}

/// Marks 404 errors from [`check`] so callers can tell "missing" apart.
const NOT_FOUND_TAG: &str = "[404]";

/// A "missing" error that [`is_not_found`] recognises.
pub fn not_found(what: &str) -> AppError {
    AppError::not_found(format!("{what}: not found {NOT_FOUND_TAG}"))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(code: u16) -> reqwest::Response {
        resp_with(code, "nope")
    }

    fn resp_with(code: u16, body: &str) -> reqwest::Response {
        reqwest::Response::from(axum::http::Response::builder().status(code).body(body.to_string()).unwrap())
    }

    fn google_403(reason: &str) -> String {
        serde_json::json!({ "error": { "code": 403, "message": "m", "errors": [{ "domain": "usageLimits", "reason": reason, "message": "m" }] } }).to_string()
    }

    /// Regression: a 401/403 was a plain connection error, so the backup
    /// retry policy retried rejected credentials with backoff.
    #[tokio::test]
    async fn rejected_credentials_are_auth_errors() {
        for code in [401, 403] {
            let e = check(resp(code), "Upload").await.unwrap_err();
            assert!(e.is_auth_failure() && !ufop_core::retry::is_retryable(&e), "{code}: {e}");
        }
        let busy = check(resp(503), "Upload").await.unwrap_err();
        assert!(!busy.is_auth_failure() && ufop_core::retry::is_retryable(&busy));
    }

    /// Regression: every 403 became a rejected sign-in, so a Google Drive
    /// backup stopped on its first rate-limited request instead of backing
    /// off. Google answers rate limits and quotas with 403 + a reason.
    #[tokio::test]
    async fn google_rate_limit_403s_are_retried() {
        for reason in ["rateLimitExceeded", "userRateLimitExceeded", "dailyLimitExceeded", "quotaExceeded", "backendError"] {
            let e = check(resp_with(403, &google_403(reason)), "Google Drive upload").await.unwrap_err();
            assert!(!e.is_auth_failure() && ufop_core::retry::is_retryable(&e), "{reason}: {e}");
        }
        // The newer error format puts the reason in error.details[].
        let details = serde_json::json!({ "error": { "code": 403, "status": "PERMISSION_DENIED", "details": [{ "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "RATE_LIMIT_EXCEEDED" }] } }).to_string();
        let e = check(resp_with(403, &details), "Google Drive upload").await.unwrap_err();
        assert!(!e.is_auth_failure() && ufop_core::retry::is_retryable(&e), "{e}");
        // Authorization reasons, and 403s without a throttling reason, stay rejected sign-ins.
        for reason in ["insufficientPermissions", "authError", "accessNotConfigured", "somethingElse"] {
            let e = check(resp_with(403, &google_403(reason)), "Google Drive upload").await.unwrap_err();
            assert!(e.is_auth_failure() && !ufop_core::retry::is_retryable(&e), "{reason}: {e}");
        }
        let e = check(resp_with(401, &google_403("rateLimitExceeded")), "Google Drive upload").await.unwrap_err();
        assert!(e.is_auth_failure(), "a 401 is always a rejected sign-in: {e}");
    }
}
