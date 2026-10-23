//! Named, scoped, expiring API tokens for `ufop-server` (for scripts, CI and
//! monitoring). A token is `ufop_pat_<random>`, stored only as a SHA-256 hash,
//! and grants one scope level:
//!
//! - `read`  — every GET (status, jobs, versions, files, runs, …)
//! - `run`   — plus starting/cancelling backups and pausing/resuming the scheduler
//! - `write` — plus creating, changing and deleting jobs
//! - `admin` — plus settings, raw RPC and `/auth/*` (except minting API tokens,
//!   which needs an interactive sign-in so a leaked token cannot mint more)
//!
//! A token may also be limited to some jobs (`job_ids`; `None` = every job):
//! it then reaches only those jobs' routes, lists show only them, and
//! server-wide routes are closed to it unless it has the `admin` scope
//! (see `access`).
//!
//! Every token expires (1–365 days). Resetting the JWT key revokes them all.

use crate::core::error::AppError;
use crate::storage::pool::DbPool;
use axum::http::Method;
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PREFIX: &str = "ufop_pat_";
pub const MAX_DAYS: i64 = 365;
pub const MAX_TOKENS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    Run,
    Write,
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Run => "run",
            Scope::Write => "write",
            Scope::Admin => "admin",
        }
    }
}

/// The scope a request needs.
pub fn required_scope(method: &Method, path: &str) -> Scope {
    let p = path.trim_end_matches('/');
    // Who the token acts for: any token may ask.
    if p == "/api/v1/auth/me" && method == Method::GET {
        return Scope::Read;
    }
    if p.starts_with("/api/v1/auth/") || p.starts_with("/api/v1/rpc/") || p.starts_with("/api/v1/server/") || p == "/api/v1/users" || p.starts_with("/api/v1/users/") || p == "/api/v1/acl" || p.starts_with("/api/v1/acl/") || p.starts_with("/api/v1/oauth/") || (p == "/api/v1/settings" && method != Method::GET) {
        return Scope::Admin;
    }
    // MCP checks write access per tool call.
    if p == "/mcp" {
        return Scope::Read;
    }
    // Gateway agents relay data in both directions: write, whatever the method.
    if p.starts_with("/api/v1/gateway/") {
        return Scope::Write;
    }
    // Computes upcoming run times; changes nothing.
    if method == Method::GET || method == Method::HEAD || p == "/api/v1/backup-schedule/preview" {
        return Scope::Read;
    }
    let run = (p.starts_with("/api/v1/jobs/") && (p.ends_with("/run") || p.ends_with("/operations")))
        || p == "/api/v1/jobs/run-all"
        || (p.starts_with("/api/v1/tasks/") && p.ends_with("/cancel"))
        || p.starts_with("/api/v1/scheduler/");
    if run {
        Scope::Run
    } else {
        Scope::Write
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
    /// Jobs the token may use (`None` = all).
    pub job_ids: Option<Vec<String>>,
    /// The account whose rights the token can never exceed.
    pub owner: String,
}

/// Who a request authenticated as (inserted as a request extension).
#[derive(Debug, Clone, PartialEq)]
pub enum Principal {
    /// The server password (browser session, CLI login, automation token, pre-auth header).
    Session,
    ApiToken { id: String, scope: Scope },
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn denied(message: &str, advice: &str) -> AppError {
    AppError::PermissionDenied { message: message.into(), advice: advice.into() }
}

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApiToken> {
    let scopes: String = r.get(2)?;
    let scope = serde_json::from_str::<Vec<Scope>>(&scopes).ok().and_then(|v| v.into_iter().max()).unwrap_or(Scope::Read);
    Ok(ApiToken {
        id: r.get(0)?,
        name: r.get(1)?,
        scope,
        created_at: r.get(3)?,
        expires_at: r.get(4)?,
        last_used_at: r.get(5)?,
        revoked: r.get::<_, i64>(6)? != 0,
        job_ids: r.get::<_, Option<String>>(7)?.and_then(|j| serde_json::from_str(&j).ok()),
        owner: r.get(8)?,
    })
}

const COLUMNS: &str = "id, name, scopes, created_at, expires_at, last_used_at, revoked, job_ids, owner";

#[derive(Clone)]
pub struct ApiTokenStore {
    pool: DbPool,
}

impl ApiTokenStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Create a token; the raw value is returned once and never stored.
    pub async fn create(&self, name: &str, scope: Scope, days: i64) -> Result<(ApiToken, String), AppError> {
        self.create_for_jobs(name, scope, days, None).await
    }

    /// Create a token limited to `job_ids` (`None` = every job).
    pub async fn create_for_jobs(&self, name: &str, scope: Scope, days: i64, job_ids: Option<Vec<String>>) -> Result<(ApiToken, String), AppError> {
        self.create_owned(name, scope, days, job_ids, super::users::ADMIN).await
    }

    /// Create a token owned by `owner` (its rights cap the token's).
    pub async fn create_owned(&self, name: &str, scope: Scope, days: i64, job_ids: Option<Vec<String>>, owner: &str) -> Result<(ApiToken, String), AppError> {
        let job_ids = match job_ids {
            Some(ids) => {
                let mut ids: Vec<String> = ids.into_iter().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()).collect();
                ids.sort();
                ids.dedup();
                if ids.is_empty() {
                    return Err(AppError::validation("A token limited to jobs needs at least one job id"));
                }
                Some(ids)
            }
            None => None,
        };
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(AppError::validation("A token needs a name of 1–80 characters"));
        }
        if !(1..=MAX_DAYS).contains(&days) {
            return Err(AppError::validation(format!("Tokens expire after 1 to {MAX_DAYS} days")));
        }
        let mut b = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut b);
        let raw = format!("{PREFIX}{}", hex::encode(b));
        let now = Utc::now();
        let t = ApiToken {
            id: format!("tok_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            name,
            scope,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::days(days)).to_rfc3339(),
            last_used_at: None,
            revoked: false,
            job_ids,
            owner: owner.to_string(),
        };
        let (row, hash) = (t.clone(), sha256_hex(&raw));
        self.pool
            .execute(move |c| {
                let active: i64 = c.query_row("SELECT COUNT(*) FROM server_api_tokens WHERE revoked = 0 AND expires_at > ?1", [Utc::now().to_rfc3339()], |r| r.get(0))?;
                if active as usize >= MAX_TOKENS {
                    return Err(AppError::validation(format!("At most {MAX_TOKENS} active tokens; revoke unused ones first")));
                }
                c.execute(
                    "INSERT INTO server_api_tokens (id, name, token_hash, scopes, created_at, expires_at, job_ids, owner) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![row.id, row.name, hash, serde_json::to_string(&[row.scope])?, row.created_at, row.expires_at, row.job_ids.as_ref().map(serde_json::to_string).transpose()?, row.owner],
                )?;
                Ok(())
            })
            .await?;
        Ok((t, raw))
    }

    pub async fn list(&self) -> Result<Vec<ApiToken>, AppError> {
        self.pool
            .execute(|c| {
                let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM server_api_tokens ORDER BY created_at DESC"))?;
                let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Revoke one token; false when there is none with that id.
    pub async fn revoke(&self, id: &str) -> Result<bool, AppError> {
        let id = id.to_string();
        self.pool.execute(move |c| Ok(c.execute("UPDATE server_api_tokens SET revoked = 1 WHERE id = ?1 AND revoked = 0", [id])? == 1)).await
    }

    pub async fn revoke_all(&self) -> Result<(), AppError> {
        self.pool
            .execute(|c| {
                c.execute("UPDATE server_api_tokens SET revoked = 1", [])?;
                Ok(())
            })
            .await
    }

    /// The token behind a raw value, if it is valid now; records its use.
    pub async fn validate(&self, raw: &str) -> Result<ApiToken, AppError> {
        let hash = sha256_hex(raw);
        let found = self
            .pool
            .execute(move |c| {
                use rusqlite::OptionalExtension;
                let t = c.query_row(&format!("SELECT {COLUMNS} FROM server_api_tokens WHERE token_hash = ?1"), [&hash], from_row).optional()?;
                if let Some(t) = &t {
                    if !t.revoked {
                        c.execute("UPDATE server_api_tokens SET last_used_at = ?1 WHERE id = ?2", rusqlite::params![Utc::now().to_rfc3339(), t.id])?;
                    }
                }
                Ok(t)
            })
            .await?;
        let t = found.ok_or_else(|| denied("Unknown API token", "Create a new token."))?;
        if t.revoked {
            return Err(denied("This API token was revoked", "Create a new token."));
        }
        if chrono::DateTime::parse_from_rfc3339(&t.expires_at).map(|e| e <= Utc::now()).unwrap_or(true) {
            return Err(denied("This API token has expired", "Create a new token."));
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_by_route() {
        let get = Method::GET;
        let post = Method::POST;
        assert_eq!(required_scope(&get, "/api/v1/jobs"), Scope::Read);
        assert_eq!(required_scope(&get, "/api/v1/jobs/j1/files"), Scope::Read);
        assert_eq!(required_scope(&post, "/api/v1/jobs/j1/run"), Scope::Run);
        assert_eq!(required_scope(&post, "/api/v1/jobs/run-all"), Scope::Run);
        assert_eq!(required_scope(&Method::GET, "/api/v1/jobs/j1/estimate"), Scope::Read);
        assert_eq!(required_scope(&post, "/api/v1/tasks/t1/cancel"), Scope::Run);
        assert_eq!(required_scope(&post, "/api/v1/scheduler/pause"), Scope::Run);
        assert_eq!(required_scope(&post, "/api/v1/jobs"), Scope::Write);
        assert_eq!(required_scope(&Method::DELETE, "/api/v1/jobs/j1"), Scope::Write);
        assert_eq!(required_scope(&get, "/api/v1/settings"), Scope::Read);
        assert_eq!(required_scope(&Method::PUT, "/api/v1/settings"), Scope::Admin);
        assert_eq!(required_scope(&Method::GET, "/api/v1/gateway/jobs/j/body"), Scope::Write);
        assert_eq!(required_scope(&Method::GET, "/api/v1/gateways/office/download"), Scope::Read);
        assert_eq!(required_scope(&Method::PUT, "/api/v1/gateways/office/upload"), Scope::Write);
        assert_eq!(required_scope(&post, "/api/v1/rpc/anything"), Scope::Admin);
        assert_eq!(required_scope(&post, "/api/v1/server/shutdown"), Scope::Admin);
        assert_eq!(required_scope(&get, "/api/v1/users"), Scope::Admin);
        assert_eq!(required_scope(&get, "/api/v1/acl/acl_1/preview-delete"), Scope::Admin);
        assert_eq!(required_scope(&get, "/api/v1/auth/me"), Scope::Read);
        assert_eq!(required_scope(&get, "/api/v1/auth/api-tokens"), Scope::Admin);
        assert!(Scope::Admin > Scope::Write && Scope::Write > Scope::Run && Scope::Run > Scope::Read);
    }

    #[tokio::test]
    async fn create_validate_revoke_expire() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let pool = repo.pool().clone();
        let s = ApiTokenStore::new(pool.clone());
        assert!(s.create("", Scope::Read, 30).await.is_err());
        assert!(s.create("x", Scope::Read, 0).await.is_err());
        assert!(s.create("x", Scope::Read, 366).await.is_err());
        let (t, raw) = s.create("CI", Scope::Run, 30).await.unwrap();
        assert!(raw.starts_with(PREFIX));
        let v = s.validate(&raw).await.unwrap();
        assert_eq!((v.id.as_str(), v.scope), (t.id.as_str(), Scope::Run));
        assert!(s.list().await.unwrap()[0].last_used_at.is_some());
        assert!(s.validate("ufop_pat_nope").await.is_err());
        // Never stored in clear.
        let stored: String = pool.execute(|c| Ok(c.query_row("SELECT token_hash FROM server_api_tokens", [], |r| r.get(0))?)).await.unwrap();
        assert_ne!(stored, raw);
        assert!(s.revoke(&t.id).await.unwrap());
        assert!(!s.revoke(&t.id).await.unwrap());
        assert!(s.validate(&raw).await.unwrap_err().to_string().contains("revoked"));
        let (j, raw_j) = s.create_for_jobs("one job", Scope::Run, 5, Some(vec!["b".into(), " a ".into(), "b".into()])).await.unwrap();
        assert_eq!(j.job_ids, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(s.validate(&raw_j).await.unwrap().job_ids, j.job_ids);
        assert!(s.create_for_jobs("none", Scope::Run, 5, Some(vec![" ".into()])).await.is_err());
        assert_eq!(v.job_ids, None, "unlimited tokens stay unlimited");
        let (t2, raw2) = s.create("old", Scope::Read, 1).await.unwrap();
        let past = (Utc::now() - Duration::minutes(1)).to_rfc3339();
        pool.execute(move |c| Ok(c.execute("UPDATE server_api_tokens SET expires_at = ?1 WHERE id = ?2", rusqlite::params![past, t2.id])?)).await.unwrap();
        assert!(s.validate(&raw2).await.unwrap_err().to_string().contains("expired"));
    }
}
