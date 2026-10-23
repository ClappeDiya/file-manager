//! REST routes for accounts and access rules (administrators), and
//! `GET /auth/me` (anyone signed in).

use super::access::{Access, AclRule, Permission, Role};
use super::users::{Secret, UserPatch};
use super::{ok, ApiResult, ServerState};
use crate::core::error::AppError;
use axum::extract::{Path as AxPath, State};
use axum::routing::{delete, get};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;

pub fn protected_routes() -> Router<ServerState> {
    Router::new()
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/users", get(list_users).post(create_user))
        .route("/api/v1/users/{username}", get(get_user).put(update_user).delete(delete_user))
        .route("/api/v1/acl", get(list_rules).post(add_rule))
        .route("/api/v1/acl/{id}", delete(delete_rule))
        .route("/api/v1/acl/{id}/preview-delete", get(preview_delete))
}

/// Who the caller is and what it may do on each job.
async fn me(State(st): State<ServerState>, Extension(acc): Extension<Access>) -> ApiResult {
    let jobs: Vec<_> = st
        .mgr
        .list_jobs()
        .await
        .into_iter()
        .map(|j| (acc.job_permission(&j.id, &j.name), j))
        .filter(|(p, _)| *p > Permission::None)
        .map(|(p, j)| json!({"job_id": j.id, "job_name": j.name, "permission": p}))
        .collect();
    ok(json!({
        "username": acc.user,
        "role": acc.role,
        "is_admin": acc.is_admin() && !acc.job_limited(),
        "token": acc.token.as_ref().map(|t| json!({"id": t.id, "scope": t.scope, "job_ids": t.job_ids})),
        "jobs": jobs,
    }))
}

async fn list_users(State(st): State<ServerState>) -> ApiResult {
    ok(json!({"users": st.auth.users().list().await?}))
}

#[derive(Deserialize)]
struct NewUser {
    username: String,
    #[serde(default)]
    password: Option<String>,
    /// A ready Argon2 PHC string instead of a password.
    #[serde(default)]
    password_hash: Option<String>,
    #[serde(default)]
    role: Role,
    #[serde(default)]
    disabled: bool,
}

async fn create_user(State(st): State<ServerState>, Json(b): Json<NewUser>) -> ApiResult {
    st.auth.ensure_accounts().await?;
    let secret = match (b.password, b.password_hash) {
        (Some(p), None) => Secret::Password(p),
        (None, Some(h)) => Secret::Hash(h),
        _ => return Err(AppError::validation("Give a password or a password_hash (not both)").into()),
    };
    let u = st.auth.users().create(&b.username, secret, b.role, b.disabled).await?;
    tracing::info!("Created account '{}' ({})", u.username, u.role.as_str());
    ok(u)
}

async fn get_user(State(st): State<ServerState>, AxPath(username): AxPath<String>) -> ApiResult {
    match st.auth.users().get(&username).await? {
        Some(u) => ok(u),
        None => Err(AppError::FileOperation { message: format!("Account '{username}' not found"), advice: "List the accounts to see their names.".into() }.into()),
    }
}

async fn update_user(State(st): State<ServerState>, AxPath(username): AxPath<String>, Json(p): Json<UserPatch>) -> ApiResult {
    let signs_out = p.password.is_some() || p.password_hash.is_some() || p.disabled == Some(true);
    let u = st.auth.users().update(&username, p).await?;
    if signs_out {
        let name = u.username.clone();
        st.auth.settings_pool().execute(move |c| Ok(c.execute("UPDATE server_tokens SET revoked = 1 WHERE subject = ?1 COLLATE NOCASE", [name]).map(|_| ())?)).await?;
    }
    tracing::info!("Changed account '{}'", u.username);
    ok(u)
}

async fn delete_user(State(st): State<ServerState>, AxPath(username): AxPath<String>) -> ApiResult {
    st.auth.users().delete(&username).await?;
    tracing::info!("Deleted account '{username}'");
    ok(json!({"ok": true}))
}

async fn list_rules(State(st): State<ServerState>) -> ApiResult {
    ok(json!({"rules": st.auth.users().rules().await?}))
}

#[derive(Deserialize)]
struct NewRule {
    principal: String,
    job_pattern: String,
    permission: Permission,
}

async fn add_rule(State(st): State<ServerState>, Json(b): Json<NewRule>) -> ApiResult {
    let r: AclRule = st.auth.users().add_rule(&b.principal, &b.job_pattern, b.permission).await?;
    tracing::info!("Added access rule {}: {} {} on {}", r.id, r.principal, r.permission.as_str(), r.job_pattern);
    ok(r)
}

async fn delete_rule(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    st.auth.users().delete_rule(&id).await?;
    tracing::info!("Deleted access rule {id}");
    ok(json!({"ok": true}))
}

/// Who would lose which access if the rule were deleted.
async fn preview_delete(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    let jobs: Vec<(String, String)> = st.mgr.list_jobs().await.into_iter().map(|j| (j.id, j.name)).collect();
    ok(json!({"rule_id": id, "changes": st.auth.users().preview_rule_delete(&id, &jobs).await?}))
}
