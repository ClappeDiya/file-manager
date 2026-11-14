//! CSRF protection for cookie-authenticated requests (signed double submit).
//!
//! The refresh token travels in the HttpOnly `ufop_refresh` cookie (scoped
//! to `/api/v1/auth`), so `POST /auth/refresh` and `/auth/logout` are
//! authenticated by a cookie the browser attaches on its own. On sign-in and
//! refresh the server also sets a script-readable `ufop_csrf` cookie whose
//! value is an HMAC of the refresh token under the server's signing key.
//!
//! A state-changing request (`POST`, `PUT`, `PATCH`, `DELETE`) that carries
//! the refresh cookie and is not authenticated by a bearer token or the
//! pre-authentication header must send the same value in `X-CSRF-Token`:
//! the header must equal the cookie and the HMAC of the refresh cookie it
//! came with (constant-time compares). As defense in depth, such requests
//! and the sign-in routes are refused when `Origin` (or, without it,
//! `Referer`) names another site than the server, `--public-url` or a
//! `--cors-origins` entry.

use super::{constant_eq, cookie_value, ServerState, PREAUTH_HEADER, REFRESH_COOKIE};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub const CSRF_COOKIE: &str = "ufop_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Routes that authenticate by their body (a password or a sign-in token):
/// only the origin check applies.
const BODY_AUTH: &[&str] = &["/api/v1/auth/login", "/api/v1/auth/signin"];

fn refuse(message: &str, advice: &str) -> Response {
    (StatusCode::FORBIDDEN, Json(json!({"error_type": "csrf", "message": message, "advice": advice}))).into_response()
}

fn host_of(origin: &str) -> Option<String> {
    let rest = origin.split_once("://").map(|(_, r)| r)?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Whether `Origin` (or `Referer`) is this server or an allowed origin; absent = allowed.
pub(crate) fn origin_allowed(st: &ServerState, h: &HeaderMap) -> bool {
    let claimed = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()).filter(|o| *o != "null").map(str::to_string).or_else(|| h.get(header::REFERER).and_then(|v| v.to_str().ok()).map(str::to_string));
    let Some(claimed) = claimed else {
        // Opaque origins ("null") come from sandboxed or file:// pages.
        return h.get(header::ORIGIN).is_none();
    };
    let Some(host) = host_of(&claimed) else { return false };
    let own = h.get(header::HOST).and_then(|v| v.to_str().ok()).map(|v| v.to_ascii_lowercase());
    if own.as_deref() == Some(host.as_str()) {
        return true;
    }
    st.cfg.public_url.iter().chain(st.cfg.cors_origins.iter()).any(|o| host_of(o).as_deref() == Some(host.as_str()))
}

fn bearer_or_preauth(h: &HeaderMap) -> bool {
    h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("Bearer ")) || h.contains_key(PREAUTH_HEADER)
}

pub(crate) async fn csrf_guard(State(st): State<ServerState>, req: Request, next: Next) -> Response {
    let mutating = matches!(*req.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE);
    if !mutating || bearer_or_preauth(req.headers()) {
        return next.run(req).await;
    }
    let h = req.headers();
    let path = req.uri().path();
    let refresh = cookie_value(h, REFRESH_COOKIE).filter(|v| !v.is_empty());
    if (refresh.is_some() || BODY_AUTH.contains(&path)) && !origin_allowed(&st, h) {
        tracing::warn!("Refused a cross-site {} {path} (Origin/Referer is another site)", req.method());
        return refuse("This request came from another site", "Open the server's own address and try again.");
    }
    if let Some(refresh) = refresh.filter(|_| !BODY_AUTH.contains(&path)) {
        let sent = h.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
        let cookie = cookie_value(h, CSRF_COOKIE).unwrap_or_default();
        let expected = match st.auth.csrf_token(&refresh).await {
            Ok(t) => t,
            Err(e) => return super::ApiError(e).into_response(),
        };
        if sent.is_empty() {
            return refuse("Missing CSRF token", "Send the ufop_csrf cookie's value in the X-CSRF-Token header (sign in again if the cookie is gone).");
        }
        if !(constant_eq(sent, &cookie) && constant_eq(sent, &expected)) {
            return refuse("The CSRF token does not match this session", "Reload the page or sign in again.");
        }
    }
    next.run(req).await
}
