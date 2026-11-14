//! Server accounts and per-job access rules (`server_users`, `server_acl`).
//!
//! Accounts sign in with a user name and password (Argon2id PHC hashes;
//! a hash can be provisioned directly). Roles and rules are explained in
//! `access`. The pre-multi-user server password becomes the `admin`
//! account on first use ([`UserStore::migrate_legacy_admin`]); tokens
//! issued before keep working as that account's.
//!
//! Accounts and rules delivered by the management console are `managed`:
//! read-only here, and replaced as a whole on every delivery.

use super::access::{AclRule, Permission, Role};
use crate::core::error::AppError;
use crate::storage::pool::DbPool;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const ADMIN: &str = "admin";
pub const MAX_USERS: usize = 1000;
pub const MAX_RULES: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub username: String,
    pub role: Role,
    pub disabled: bool,
    /// Delivered by the management console (read-only here).
    pub managed: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A password, or a ready Argon2 PHC string (`$argon2id$…`).
#[derive(Debug, Clone)]
pub enum Secret {
    Password(String),
    Hash(String),
}

impl Secret {
    fn into_hash(self) -> Result<String, AppError> {
        match self {
            Secret::Password(p) => super::auth::hash_password(&p),
            Secret::Hash(h) => {
                validate_phc(&h)?;
                Ok(h)
            }
        }
    }
}

/// Accept only Argon2 PHC strings.
pub fn validate_phc(h: &str) -> Result<(), AppError> {
    match argon2::password_hash::PasswordHash::new(h.trim()) {
        Ok(p) if p.algorithm.as_str().starts_with("argon2") && p.hash.is_some() => Ok(()),
        _ => Err(AppError::validation("The password hash must be an Argon2 PHC string ($argon2id$v=19$m=…,t=…,p=…$salt$hash); make one with `ufop-server hash-password`")),
    }
}

pub fn validate_username(u: &str) -> Result<(), AppError> {
    let ok = (1..=64).contains(&u.len()) && u.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@-".contains(&b)) && !u.starts_with('-');
    if !ok {
        return Err(AppError::validation("User names have 1–64 letters, digits, '.', '_', '@' or '-'"));
    }
    if u == "*" || u.starts_with("role:") {
        return Err(AppError::validation("That user name is reserved"));
    }
    Ok(())
}

/// Changes to an account (absent = unchanged).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserPatch {
    pub role: Option<Role>,
    pub disabled: Option<bool>,
    pub password: Option<String>,
    pub password_hash: Option<String>,
}

/// What deleting a rule would change: one row per (user, job) that loses access.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuleEffect {
    pub username: String,
    pub job_id: String,
    pub job_name: String,
    pub before: Permission,
    pub after: Permission,
}

fn row_user(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let role: String = r.get(1)?;
    Ok(User {
        username: r.get(0)?,
        role: serde_json::from_value(serde_json::Value::String(role)).unwrap_or_default(),
        disabled: r.get::<_, i64>(2)? != 0,
        managed: r.get::<_, i64>(3)? != 0,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

fn row_rule(r: &rusqlite::Row<'_>) -> rusqlite::Result<AclRule> {
    let perm: String = r.get(3)?;
    Ok(AclRule {
        id: r.get(0)?,
        principal: r.get(1)?,
        job_pattern: r.get(2)?,
        permission: serde_json::from_value(serde_json::Value::String(perm)).unwrap_or_default(),
        managed: r.get::<_, i64>(4)? != 0,
        created_at: r.get(5)?,
    })
}

const USER_COLS: &str = "username, role, disabled, managed, created_at, updated_at";
const RULE_COLS: &str = "id, principal, job_pattern, permission, managed, created_at";

fn managed_refusal(what: &str) -> AppError {
    AppError::PermissionDenied { message: format!("This {what} is managed by your organization; changing it here is not allowed"), advice: "Change it in the management console.".into() }
}

#[derive(Clone)]
pub struct UserStore {
    pool: DbPool,
}

impl UserStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<User>, AppError> {
        self.pool
            .execute(|c| {
                let mut st = c.prepare(&format!("SELECT {USER_COLS} FROM server_users ORDER BY username COLLATE NOCASE"))?;
                let rows = st.query_map([], row_user)?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get(&self, username: &str) -> Result<Option<User>, AppError> {
        let u = username.to_string();
        self.pool
            .execute(move |c| {
                use rusqlite::OptionalExtension;
                Ok(c.query_row(&format!("SELECT {USER_COLS} FROM server_users WHERE username = ?1 COLLATE NOCASE"), [u], row_user).optional()?)
            })
            .await
    }

    pub async fn is_empty(&self) -> Result<bool, AppError> {
        self.pool.execute(|c| Ok(c.query_row("SELECT COUNT(*) FROM server_users", [], |r| r.get::<_, i64>(0))? == 0)).await
    }

    /// The account and its password hash (checked by `AuthService` under its login throttle).
    pub async fn credentials(&self, username: &str) -> Result<Option<(User, String)>, AppError> {
        let u = username.to_string();
        self.pool
            .execute(move |c| {
                use rusqlite::OptionalExtension;
                Ok(c.query_row(&format!("SELECT {USER_COLS}, password_hash FROM server_users WHERE username = ?1 COLLATE NOCASE"), [u], |r| Ok((row_user(r)?, r.get::<_, String>(6)?))).optional()?)
            })
            .await
    }

    pub async fn create(&self, username: &str, secret: Secret, role: Role, disabled: bool) -> Result<User, AppError> {
        self.insert(username, secret, role, disabled, false).await
    }

    async fn insert(&self, username: &str, secret: Secret, role: Role, disabled: bool, managed: bool) -> Result<User, AppError> {
        let username = username.trim().to_string();
        validate_username(&username)?;
        let hash = secret.into_hash()?;
        let now = Utc::now().to_rfc3339();
        let user = User { username: username.clone(), role, disabled, managed, created_at: now.clone(), updated_at: now };
        let u = user.clone();
        self.pool
            .execute(move |c| {
                let n: i64 = c.query_row("SELECT COUNT(*) FROM server_users", [], |r| r.get(0))?;
                if n as usize >= MAX_USERS {
                    return Err(AppError::validation(format!("At most {MAX_USERS} accounts")));
                }
                let exists: i64 = c.query_row("SELECT COUNT(*) FROM server_users WHERE username = ?1 COLLATE NOCASE", [&u.username], |r| r.get(0))?;
                if exists > 0 {
                    return Err(AppError::state(format!("There is already an account '{}'", u.username), "Pick another name or change that account."));
                }
                c.execute(
                    "INSERT INTO server_users (username, password_hash, role, disabled, managed, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    rusqlite::params![u.username, hash, u.role.as_str(), u.disabled as i64, u.managed as i64, u.created_at],
                )?;
                Ok(())
            })
            .await?;
        Ok(user)
    }

    /// Enabled administrators other than `except`.
    async fn other_admins(&self, except: &str) -> Result<i64, AppError> {
        let e = except.to_string();
        self.pool.execute(move |c| Ok(c.query_row("SELECT COUNT(*) FROM server_users WHERE role = 'admin' AND disabled = 0 AND username <> ?1 COLLATE NOCASE", [e], |r| r.get(0))?)).await
    }

    pub async fn update(&self, username: &str, patch: UserPatch) -> Result<User, AppError> {
        let cur = self.get(username).await?.ok_or_else(|| not_found(username))?;
        if cur.managed {
            return Err(managed_refusal("account"));
        }
        let demoted = patch.role.is_some_and(|r| r != Role::Admin) || patch.disabled == Some(true);
        if cur.role == Role::Admin && !cur.disabled && demoted && self.other_admins(&cur.username).await? == 0 {
            return Err(AppError::state("This is the last enabled administrator", "Make another account an administrator first."));
        }
        let hash = match (patch.password, patch.password_hash) {
            (Some(p), _) => Some(Secret::Password(p).into_hash()?),
            (None, Some(h)) => Some(Secret::Hash(h).into_hash()?),
            (None, None) => None,
        };
        let (name, role, disabled, now) = (cur.username.clone(), patch.role.unwrap_or(cur.role), patch.disabled.unwrap_or(cur.disabled), Utc::now().to_rfc3339());
        self.pool
            .execute(move |c| {
                c.execute("UPDATE server_users SET role = ?2, disabled = ?3, updated_at = ?4 WHERE username = ?1", rusqlite::params![name, role.as_str(), disabled as i64, now])?;
                if let Some(h) = hash {
                    c.execute("UPDATE server_users SET password_hash = ?2 WHERE username = ?1", rusqlite::params![name, h])?;
                }
                Ok(())
            })
            .await?;
        self.get(&cur.username).await?.ok_or_else(|| not_found(username))
    }

    /// Delete an account, its sessions and its API tokens.
    pub async fn delete(&self, username: &str) -> Result<(), AppError> {
        let cur = self.get(username).await?.ok_or_else(|| not_found(username))?;
        if cur.managed {
            return Err(managed_refusal("account"));
        }
        if cur.role == Role::Admin && !cur.disabled && self.other_admins(&cur.username).await? == 0 {
            return Err(AppError::state("This is the last enabled administrator", "Make another account an administrator first."));
        }
        let name = cur.username;
        self.pool
            .execute(move |c| {
                c.execute("DELETE FROM server_users WHERE username = ?1", [&name])?;
                c.execute("UPDATE server_tokens SET revoked = 1 WHERE subject = ?1 COLLATE NOCASE", [&name])?;
                c.execute("UPDATE server_api_tokens SET revoked = 1 WHERE owner = ?1 COLLATE NOCASE", [&name])?;
                Ok(())
            })
            .await
    }

    /// The old single server password becomes the `admin` account (once, when there are no accounts).
    pub async fn migrate_legacy_admin(&self, legacy_hash: Option<String>) -> Result<bool, AppError> {
        let Some(h) = legacy_hash else { return Ok(false) };
        if !self.is_empty().await? {
            return Ok(false);
        }
        self.insert(ADMIN, Secret::Hash(h), Role::Admin, false, false).await?;
        tracing::info!("The server password is now the password of the 'admin' account");
        Ok(true)
    }

    /// Set the `admin` account's password (created when missing, re-enabled as administrator).
    pub async fn set_admin_password(&self, hash: String) -> Result<(), AppError> {
        match self.get(ADMIN).await? {
            None => self.insert(ADMIN, Secret::Hash(hash), Role::Admin, false, false).await.map(|_| ()),
            Some(_) => {
                let now = Utc::now().to_rfc3339();
                self.pool
                    .execute(move |c| {
                        c.execute("UPDATE server_users SET password_hash = ?1, role = 'admin', disabled = 0, updated_at = ?2 WHERE username = ?3", rusqlite::params![hash, now, ADMIN])?;
                        Ok(())
                    })
                    .await
            }
        }
    }

    // ── Access rules ──

    pub async fn rules(&self) -> Result<Vec<AclRule>, AppError> {
        self.pool
            .execute(|c| {
                let mut st = c.prepare(&format!("SELECT {RULE_COLS} FROM server_acl ORDER BY created_at, id"))?;
                let rows = st.query_map([], row_rule)?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// The rules that apply to an account.
    pub async fn rules_for(&self, user: &User) -> Result<Arc<Vec<AclRule>>, AppError> {
        if user.role == Role::Admin {
            return Ok(Arc::default());
        }
        let all = self.rules().await?;
        Ok(Arc::new(all.into_iter().filter(|r| r.applies_to(&user.username, user.role)).collect()))
    }

    pub async fn add_rule(&self, principal: &str, job_pattern: &str, permission: Permission) -> Result<AclRule, AppError> {
        self.insert_rule(principal, job_pattern, permission, false).await
    }

    async fn insert_rule(&self, principal: &str, job_pattern: &str, permission: Permission, managed: bool) -> Result<AclRule, AppError> {
        let principal = principal.trim().to_string();
        let job_pattern = job_pattern.trim().to_string();
        let valid_principal = principal == "*"
            || principal.strip_prefix("role:").is_some_and(|r| ["admin", "operator", "viewer"].contains(&r.to_ascii_lowercase().as_str()))
            || validate_username(&principal).is_ok();
        if !valid_principal {
            return Err(AppError::validation("A rule's principal is a user name, role:operator, role:viewer, role:admin or *"));
        }
        if job_pattern.is_empty() || job_pattern.len() > 200 {
            return Err(AppError::validation("A rule needs a job id or pattern (1–200 characters; * and ? match any text and one character)"));
        }
        let rule = AclRule { id: format!("acl_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]), principal, job_pattern, permission, managed, created_at: Utc::now().to_rfc3339() };
        let r = rule.clone();
        self.pool
            .execute(move |c| {
                let n: i64 = c.query_row("SELECT COUNT(*) FROM server_acl", [], |x| x.get(0))?;
                if n as usize >= MAX_RULES {
                    return Err(AppError::validation(format!("At most {MAX_RULES} access rules")));
                }
                c.execute(
                    "INSERT INTO server_acl (id, principal, job_pattern, permission, managed, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![r.id, r.principal, r.job_pattern, r.permission.as_str(), r.managed as i64, r.created_at],
                )?;
                Ok(())
            })
            .await?;
        Ok(rule)
    }

    pub async fn delete_rule(&self, id: &str) -> Result<(), AppError> {
        let rule = self.rules().await?.into_iter().find(|r| r.id == id).ok_or_else(|| AppError::FileOperation { message: format!("Access rule {id} not found"), advice: "List the rules to see their ids.".into() })?;
        if rule.managed {
            return Err(managed_refusal("access rule"));
        }
        let id = id.to_string();
        self.pool.execute(move |c| Ok(c.execute("DELETE FROM server_acl WHERE id = ?1", [id]).map(|_| ())?)).await
    }

    /// Who would lose what if rule `id` were deleted.
    pub async fn preview_rule_delete(&self, id: &str, jobs: &[(String, String)]) -> Result<Vec<RuleEffect>, AppError> {
        let all = self.rules().await?;
        if !all.iter().any(|r| r.id == id) {
            return Err(AppError::FileOperation { message: format!("Access rule {id} not found"), advice: "List the rules to see their ids.".into() });
        }
        let without: Vec<AclRule> = all.iter().filter(|r| r.id != id).cloned().collect();
        let mut out = Vec::new();
        for u in self.list().await?.into_iter().filter(|u| u.role != Role::Admin) {
            let access = |rules: &[AclRule]| super::access::Access { user: u.username.clone(), role: u.role, rules: Arc::new(rules.to_vec()), token: None };
            let (a, b) = (access(&all), access(&without));
            for (jid, jname) in jobs {
                let (before, after) = (a.user_permission(jid, jname), b.user_permission(jid, jname));
                if before != after {
                    out.push(RuleEffect { username: u.username.clone(), job_id: jid.clone(), job_name: jname.clone(), before, after });
                }
            }
        }
        Ok(out)
    }

    /// Apply the console's `serverAccess` section (`None` removes managed accounts and rules).
    pub async fn apply_managed(&self, m: Option<&ufop_core::agent::ManagedServerAccess>) -> Result<Vec<String>, AppError> {
        let parse = |v: &str| serde_json::Value::String(v.to_string());
        let (users, rules) = match m {
            Some(m) => (
                m.users.iter().filter_map(|u| serde_json::from_value::<Role>(parse(&u.role)).ok().map(|r| (u.username.trim().to_string(), u.password_hash.trim().to_string(), r, u.disabled))).collect(),
                m.rules.iter().filter_map(|r| serde_json::from_value::<Permission>(parse(&r.permission)).ok().map(|p| (r.principal.clone(), r.job_pattern.clone(), p))).collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        self.replace_managed(users, rules).await
    }

    /// Replace the console-managed accounts and rules (local ones are kept;
    /// a managed account never replaces a local one of the same name).
    pub async fn replace_managed(&self, users: Vec<(String, String, Role, bool)>, rules: Vec<(String, String, Permission)>) -> Result<Vec<String>, AppError> {
        let mut problems = Vec::new();
        let local: Vec<String> = self.list().await?.into_iter().filter(|u| !u.managed).map(|u| u.username.to_lowercase()).collect();
        self.pool
            .execute(|c| {
                c.execute("DELETE FROM server_users WHERE managed = 1", [])?;
                c.execute("DELETE FROM server_acl WHERE managed = 1", [])?;
                Ok(())
            })
            .await?;
        for (name, hash, role, disabled) in users {
            if local.contains(&name.to_lowercase()) {
                problems.push(format!("account '{name}' exists on this server and was not replaced"));
                continue;
            }
            if let Err(e) = self.insert(&name, Secret::Hash(hash), role, disabled, true).await {
                problems.push(format!("account '{name}': {}", e.user_message()));
            }
        }
        for (p, pat, perm) in rules {
            if let Err(e) = self.insert_rule(&p, &pat, perm, true).await {
                problems.push(format!("rule {p} {pat}: {}", e.user_message()));
            }
        }
        Ok(problems)
    }
}

fn not_found(u: &str) -> AppError {
    AppError::FileOperation { message: format!("Account '{u}' not found"), advice: "List the accounts to see their names.".into() }
}

/// An Argon2 hash of a random password: unknown names cost the same as wrong passwords.
pub fn dummy_hash() -> &'static str {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| super::auth::hash_password(&uuid::Uuid::new_v4().to_string()).unwrap_or_default())
}
