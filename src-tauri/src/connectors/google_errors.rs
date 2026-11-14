//! Google API 403 replies. Google answers rate limits, quotas, a disabled
//! API and a refused sign-in all with HTTP 403; only the reason in the body
//! (`error.errors[].reason`, or `error.details[].reason` in the newer
//! format) tells them apart. Shared by the Google Drive connector and the
//! backup stores' HTTP helper.

/// What a Google 403 means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forbidden {
    /// A rate limit, quota or transient backend error: retry with backoff.
    Throttled,
    /// The API is not enabled for the OAuth app's Cloud project.
    ApiDisabled,
    /// The sign-in was refused or lacks permission.
    Authorization,
    /// No recognised reason.
    Unknown,
}

const THROTTLED: &[&str] = &["rateLimitExceeded", "userRateLimitExceeded", "dailyLimitExceeded", "quotaExceeded", "RATE_LIMIT_EXCEEDED", "backendError"];
const API_DISABLED: &[&str] = &["accessNotConfigured", "SERVICE_DISABLED"];
const AUTHORIZATION: &[&str] = &["authError", "insufficientPermissions", "ACCESS_TOKEN_SCOPE_INSUFFICIENT", "invalidCredentials"];

/// Classify a 403 body; also returns the first reason given (for messages).
pub fn classify_403(body: &str) -> (Forbidden, Option<String>) {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let err = &v["error"];
    let reasons: Vec<&str> = err["errors"].as_array().into_iter().chain(err["details"].as_array()).flatten().filter_map(|e| e["reason"].as_str()).collect();
    let has = |list: &[&str]| reasons.iter().any(|r| list.iter().any(|l| r.eq_ignore_ascii_case(l)));
    let kind = if has(THROTTLED) {
        Forbidden::Throttled
    } else if has(API_DISABLED) {
        Forbidden::ApiDisabled
    } else if has(AUTHORIZATION) {
        Forbidden::Authorization
    } else {
        Forbidden::Unknown
    };
    (kind, reasons.first().map(|r| r.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_error_formats() {
        let legacy = r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        assert_eq!(classify_403(legacy), (Forbidden::Throttled, Some("userRateLimitExceeded".into())));
        let details = r#"{"error":{"details":[{"reason":"SERVICE_DISABLED"}]}}"#;
        assert_eq!(classify_403(details).0, Forbidden::ApiDisabled);
        assert_eq!(classify_403(r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#).0, Forbidden::Authorization);
        assert_eq!(classify_403("nope"), (Forbidden::Unknown, None));
    }
}
