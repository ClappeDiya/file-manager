//! Server authentication: a single hashed UI password (Argon2id PHC
//! string), short-lived HS256 access tokens, rotating refresh tokens
//! (stored hashed, delivered as an HttpOnly cookie), one-time sign-in
//! tokens, opt-in long-lived automation tokens, and "reset JWT" which
//! rotates the signing key and revokes every outstanding token.
//!
//! Password checks are serialized and slowed down after failures, with
//! a global bound on pending attempts (login throttling, not API rate
//! limiting).

use crate::security::settings_store::SettingsStore;
use crate::core::error::AppError;
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::Engine;
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const KEY_PASSWORD: &str = "server.password_hash";
const KEY_JWT: &str = "server.jwt_key";
const KEY_AUTOMATION: &str = "server.allow_automation_tokens";

pub const ACCESS_TTL_MIN: i64 = 15;
pub const REFRESH_TTL_DAYS: i64 = 30;
pub const SIGNIN_TTL_MIN: i64 = 5;
pub const AUTOMATION_TTL_DAYS: i64 = 3650;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Access,
    Automation,
    Signin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub kind: TokenKind,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IssuedToken {
    pub token: String,
    pub expires_at: String,
}

fn auth_err(msg: &str) -> AppError {
    AppError::PermissionDenied { message: msg.to_string(), advice: "Sign in again.".to_string() }
}

pub fn hash_password(pw: &str) -> Result<String, AppError> {
    if pw.chars().count() < 8 {
        return Err(AppError::validation("The server password must have at least 8 characters"));
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AppError::internal("password hashing failed"))
}

pub fn verify_password_hash(pw: &str, phc: &str) -> bool {
    PasswordHash::new(phc).map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok()).unwrap_or(false)
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    B64.encode(b)
}

/// Sign / verify HS256 JWTs.
pub fn sign_jwt(key: &[u8], claims: &Claims) -> Result<String, AppError> {
    let header = B64.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let body = B64.encode(serde_json::to_vec(claims)?);
    let input = format!("{header}.{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| AppError::internal("bad jwt key"))?;
    mac.update(input.as_bytes());
    Ok(format!("{input}.{}", B64.encode(mac.finalize().into_bytes())))
}

pub fn verify_jwt(key: &[u8], token: &str) -> Result<Claims, AppError> {
    let mut parts = token.split('.');
    let (h, b, s) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(b), Some(s), None) => (h, b, s),
        _ => return Err(auth_err("Malformed token")),
    };
    let header: serde_json::Value = serde_json::from_slice(&B64.decode(h).map_err(|_| auth_err("Malformed token"))?)
        .map_err(|_| auth_err("Malformed token"))?;
    if header["alg"] != "HS256" {
        return Err(auth_err("Unsupported token algorithm"));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| AppError::internal("bad jwt key"))?;
    mac.update(format!("{h}.{b}").as_bytes());
    mac.verify_slice(&B64.decode(s).map_err(|_| auth_err("Malformed token"))?)
        .map_err(|_| auth_err("Invalid or revoked token"))?;
    let claims: Claims = serde_json::from_slice(&B64.decode(b).map_err(|_| auth_err("Malformed token"))?)
        .map_err(|_| auth_err("Malformed token"))?;
    if claims.exp < Utc::now().timestamp() {
        return Err(auth_err("Token expired"));
    }
    Ok(claims)
}

/// Login throttle: one password check at a time, growing delay after
/// failures, and at most `max_pending` callers waiting.
pub struct LoginThrottle {
    lock: Mutex<()>,
    pending: Semaphore,
    failures: AtomicU32,
    base_delay_ms: u64,
}

impl LoginThrottle {
    pub fn new(max_pending: usize, base_delay_ms: u64) -> Self {
        Self { lock: Mutex::new(()), pending: Semaphore::new(max_pending), failures: AtomicU32::new(0), base_delay_ms }
    }

    pub fn delay_ms(&self) -> u64 {
        let f = self.failures.load(Ordering::SeqCst);
        if f == 0 { 0 } else { (self.base_delay_ms << f.min(6)).min(10_000) }
    }

    /// Run `check` under the throttle; returns `Ok(None)` when too many
    /// attempts are already waiting.
    pub async fn attempt(&self, check: impl FnOnce() -> bool) -> Option<bool> {
        let _slot = self.pending.try_acquire().ok()?;
        let _g = self.lock.lock().await;
        let d = self.delay_ms();
        if d > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(d)).await;
        }
        let ok = check();
        if ok {
            self.failures.store(0, Ordering::SeqCst);
        } else {
            self.failures.fetch_add(1, Ordering::SeqCst);
        }
        Some(ok)
    }
}

/// Authentication service backed by the settings store and `server_tokens`.
#[derive(Clone)]
pub struct AuthService {
    settings: SettingsStore,
    pub throttle: Arc<LoginThrottle>,
}

impl AuthService {
    pub fn new(settings: SettingsStore) -> Self {
        Self { settings, throttle: Arc::new(LoginThrottle::new(10, 250)) }
    }

    pub async fn has_password(&self) -> Result<bool, AppError> {
        Ok(self.settings.get(KEY_PASSWORD).await?.is_some())
    }

    pub async fn set_password(&self, pw: &str) -> Result<(), AppError> {
        self.settings.set_secret(KEY_PASSWORD, &hash_password(pw)?).await?;
        // A new password invalidates every session.
        self.reset_jwt().await
    }

    /// Folder policies of shared folders (see `folders`).
    pub fn folder_policies(&self) -> super::folders::FolderPolicyStore {
        super::folders::FolderPolicyStore::new(self.settings.pool().clone())
    }

    /// Share links (see `shares`).
    pub fn shares(&self) -> super::shares::ShareStore {
        super::shares::ShareStore::new(self.settings.pool().clone())
    }

    /// Named, scoped API tokens (see `api_tokens`).
    pub fn api_tokens(&self) -> super::api_tokens::ApiTokenStore {
        super::api_tokens::ApiTokenStore::new(self.settings.pool().clone())
    }

    async fn jwt_key(&self) -> Result<Vec<u8>, AppError> {
        if let Some(k) = self.settings.get_secret(KEY_JWT).await? {
            return hex::decode(k).map_err(|_| AppError::internal("corrupt jwt key"));
        }
        let mut k = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        self.settings.set_secret(KEY_JWT, &hex::encode(&k)).await?;
        Ok(k)
    }

    /// Rotate the signing key and revoke all refresh and API tokens.
    pub async fn reset_jwt(&self) -> Result<(), AppError> {
        let mut k = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        self.settings.set_secret(KEY_JWT, &hex::encode(&k)).await?;
        self.settings
            .pool()
            .execute(|c| {
                c.execute("UPDATE server_tokens SET revoked = 1", [])?;
                c.execute("UPDATE server_api_tokens SET revoked = 1", [])?;
                Ok(())
            })
            .await
    }

    pub async fn set_automation_allowed(&self, allowed: bool) -> Result<(), AppError> {
        self.settings.set(KEY_AUTOMATION, if allowed { "1" } else { "0" }).await
    }

    pub async fn automation_allowed(&self) -> Result<bool, AppError> {
        Ok(self.settings.get(KEY_AUTOMATION).await?.as_deref() == Some("1"))
    }

    async fn issue(&self, kind: TokenKind, ttl: Duration) -> Result<IssuedToken, AppError> {
        let now = Utc::now();
        let claims = Claims { sub: "admin".into(), kind, iat: now.timestamp(), exp: (now + ttl).timestamp(), jti: uuid::Uuid::new_v4().to_string() };
        Ok(IssuedToken { token: sign_jwt(&self.jwt_key().await?, &claims)?, expires_at: (now + ttl).to_rfc3339() })
    }

    pub async fn issue_access(&self) -> Result<IssuedToken, AppError> {
        self.issue(TokenKind::Access, Duration::minutes(ACCESS_TTL_MIN)).await
    }

    /// Short-lived token that can be exchanged once for a session (e.g. "open UI" from the tray).
    pub async fn issue_signin(&self) -> Result<IssuedToken, AppError> {
        self.issue(TokenKind::Signin, Duration::minutes(SIGNIN_TTL_MIN)).await
    }

    pub async fn issue_automation(&self) -> Result<IssuedToken, AppError> {
        if !self.automation_allowed().await? {
            return Err(AppError::PermissionDenied {
                message: "Automation tokens are disabled".into(),
                advice: "Enable them with `ufop-server --allow-automation-tokens` or the settings API.".into(),
            });
        }
        self.issue(TokenKind::Automation, Duration::days(AUTOMATION_TTL_DAYS)).await
    }

    /// Validate a bearer token; sign-in tokens are not accepted as bearers.
    pub async fn validate(&self, token: &str) -> Result<Claims, AppError> {
        let c = verify_jwt(&self.jwt_key().await?, token)?;
        if c.kind == TokenKind::Signin {
            return Err(auth_err("Sign-in tokens must be redeemed first"));
        }
        Ok(c)
    }

    /// Redeem a one-time sign-in token.
    pub async fn redeem_signin(&self, token: &str) -> Result<(), AppError> {
        let c = verify_jwt(&self.jwt_key().await?, token)?;
        if c.kind != TokenKind::Signin {
            return Err(auth_err("Not a sign-in token"));
        }
        let jti = c.jti.clone();
        let fresh = self
            .settings
            .pool()
            .execute(move |conn| {
                Ok(conn.execute(
                    "INSERT OR IGNORE INTO server_tokens(id, kind, token_hash, expires_at, revoked) VALUES (?1, 'signin_used', ?1, datetime('now','+1 day'), 1)",
                    [&jti],
                )? == 1)
            })
            .await?;
        if !fresh {
            return Err(auth_err("This sign-in link was already used"));
        }
        Ok(())
    }

    pub async fn issue_refresh(&self) -> Result<IssuedToken, AppError> {
        let raw = random_token();
        let h = sha256_hex(&raw);
        let exp = Utc::now() + Duration::days(REFRESH_TTL_DAYS);
        let exp_s = exp.to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        self.settings
            .pool()
            .execute(move |c| {
                c.execute(
                    "INSERT INTO server_tokens(id, kind, token_hash, expires_at, revoked) VALUES (?1, 'refresh', ?2, ?3, 0)",
                    rusqlite::params![id, h, exp_s],
                )?;
                Ok(())
            })
            .await?;
        Ok(IssuedToken { token: raw, expires_at: exp.to_rfc3339() })
    }

    /// Exchange a refresh token: revokes it and issues a new pair (rotation).
    pub async fn refresh(&self, raw: &str) -> Result<(IssuedToken, IssuedToken), AppError> {
        let h = sha256_hex(raw);
        let now = Utc::now().to_rfc3339();
        let ok = self
            .settings
            .pool()
            .execute(move |c| {
                Ok(c.execute(
                    "UPDATE server_tokens SET revoked = 1 WHERE kind = 'refresh' AND token_hash = ?1 AND revoked = 0 AND expires_at > ?2",
                    rusqlite::params![h, now],
                )? == 1)
            })
            .await?;
        if !ok {
            return Err(auth_err("Session expired"));
        }
        Ok((self.issue_access().await?, self.issue_refresh().await?))
    }

    pub async fn revoke_refresh(&self, raw: &str) -> Result<(), AppError> {
        let h = sha256_hex(raw);
        self.settings
            .pool()
            .execute(move |c| {
                c.execute("UPDATE server_tokens SET revoked = 1 WHERE token_hash = ?1", [h])?;
                Ok(())
            })
            .await
    }

    /// Throttled password login. `Ok(None)` = too many pending attempts.
    pub async fn login(&self, pw: &str) -> Result<Option<bool>, AppError> {
        let Some(phc) = self.settings.get_secret(KEY_PASSWORD).await? else {
            return Err(AppError::state("No server password is set", "Start ufop-server with --set-password first."));
        };
        let pw = pw.to_string();
        Ok(self.throttle.attempt(move || verify_password_hash(&pw, &phc)).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_roundtrip_and_tamper() {
        let key = b"0123456789abcdef0123456789abcdef";
        let c = Claims { sub: "admin".into(), kind: TokenKind::Access, iat: 0, exp: Utc::now().timestamp() + 60, jti: "x".into() };
        let t = sign_jwt(key, &c).unwrap();
        assert_eq!(verify_jwt(key, &t).unwrap().jti, "x");
        assert!(verify_jwt(b"another-key-another-key-another!", &t).is_err());
        let mut bad = t.clone();
        bad.push('A');
        assert!(verify_jwt(key, &bad).is_err());
        let expired = Claims { exp: Utc::now().timestamp() - 1, ..c };
        assert!(verify_jwt(key, &sign_jwt(key, &expired).unwrap()).is_err());
    }

    #[test]
    fn password_hashing() {
        assert!(hash_password("short").is_err());
        let h = hash_password("correct horse").unwrap();
        assert!(verify_password_hash("correct horse", &h));
        assert!(!verify_password_hash("wrong horse", &h));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn throttle_delays_after_failures_and_bounds_pending() {
        let t = Arc::new(LoginThrottle::new(1, 10));
        assert_eq!(t.attempt(|| false).await, Some(false));
        assert_eq!(t.attempt(|| false).await, Some(false));
        assert!(t.delay_ms() >= 40);
        assert_eq!(t.attempt(|| true).await, Some(true));
        assert_eq!(t.delay_ms(), 0);
        // With one slot held, a second concurrent attempt is refused.
        let t2 = t.clone();
        let hold = tokio::spawn(async move { t2.attempt(|| { std::thread::sleep(std::time::Duration::from_millis(200)); true }).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(t.attempt(|| true).await, None);
        assert_eq!(hold.await.unwrap(), Some(true));
    }
}
