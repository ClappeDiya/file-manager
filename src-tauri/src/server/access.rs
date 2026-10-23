//! Who may do what on `ufop-server`.
//!
//! Every authenticated request carries an [`Access`]: the user it acts for
//! (role and per-job access rules) and, for API tokens, the token's scope
//! and optional job list. The effective permission on a job is the
//! intersection of the three:
//!
//! - **user**: the `admin` role has `full` on every job; `operator` and
//!   `viewer` get the strongest permission of the access rules that match
//!   them and the job (no rule = no access), capped at `full` (operator) or
//!   `read` (viewer);
//! - **token scope**: `read` → read, `run` → run, `write`/`admin` → full;
//! - **token jobs** (`job_ids`): a job outside the list → none.
//!
//! Server-wide ("global") routes have their own levels ([`Global`]): any
//! caller (lists are filtered to the jobs it may read), signed-in users and
//! unrestricted tokens, operators, or administrators. A token limited to
//! some jobs only reaches the first level unless it also has the `admin`
//! scope.

use super::api_tokens::Scope;
use axum::http::Method;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Permission on a job, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    #[default]
    None,
    /// See the job, its versions, files and history; download files.
    Read,
    /// Plus start and cancel backups, verification and restores' dry runs.
    Run,
    /// Plus change the job, restore files, change version notes and pins.
    Write,
    /// Plus delete the job, its versions or repository data, compact, break locks.
    Full,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::None => "none",
            Permission::Read => "read",
            Permission::Run => "run",
            Permission::Write => "write",
            Permission::Full => "full",
        }
    }

    pub fn of_scope(s: Scope) -> Self {
        match s {
            Scope::Read => Permission::Read,
            Scope::Run => Permission::Run,
            Scope::Write | Scope::Admin => Permission::Full,
        }
    }
}

/// A server account's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Jobs only through access rules, at most read.
    #[default]
    Viewer,
    /// Jobs through access rules (up to full); may browse the server's folders and test destinations.
    Operator,
    /// Everything: every job, users, rules, settings, tokens, shares, shutdown.
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Operator => "operator",
            Role::Admin => "admin",
        }
    }

    /// The most a rule can give this role on a job.
    pub fn ceiling(self) -> Permission {
        match self {
            Role::Viewer => Permission::Read,
            Role::Operator | Role::Admin => Permission::Full,
        }
    }
}

/// One access rule: `principal` (a user name, `role:<role>` or `*`) gets
/// `permission` on jobs matching `job_pattern` (a job id, or a glob with
/// `*` and `?` matched against the job's id and name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclRule {
    #[serde(default)]
    pub id: String,
    pub principal: String,
    pub job_pattern: String,
    pub permission: Permission,
    /// Delivered by the management console (read-only here).
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub created_at: String,
}

impl AclRule {
    pub fn applies_to(&self, user: &str, role: Role) -> bool {
        let p = self.principal.trim();
        p == "*" || p.eq_ignore_ascii_case(user) || p.strip_prefix("role:").is_some_and(|r| r.eq_ignore_ascii_case(role.as_str()))
    }

    pub fn matches_job(&self, job_id: &str, job_name: &str) -> bool {
        let pat = self.job_pattern.trim();
        pat == job_id || glob(pat, job_id) || glob(&pat.to_lowercase(), &job_name.to_lowercase())
    }
}

/// `*` (any run) and `?` (one character) glob.
pub fn glob(pat: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), s.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// An API token's limits.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenLimit {
    pub id: String,
    pub scope: Scope,
    /// `None` = every job the owner may use.
    pub job_ids: Option<Vec<String>>,
}

/// The rights of one request (request extension).
#[derive(Debug, Clone, PartialEq)]
pub struct Access {
    pub user: String,
    pub role: Role,
    /// The rules that apply to this user (any job).
    pub rules: Arc<Vec<AclRule>>,
    pub token: Option<TokenLimit>,
}

/// Levels of server-wide routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Global {
    /// Anyone authenticated; results are filtered per job.
    Any,
    /// Signed-in users and tokens not limited to some jobs.
    User,
    /// Operators and administrators.
    Operator,
    Admin,
}

impl Access {
    /// The server password holder / pre-authenticated proxy / admin-owned token without limits.
    pub fn admin() -> Self {
        Self { user: "admin".into(), role: Role::Admin, rules: Arc::default(), token: None }
    }

    pub fn job_limited(&self) -> bool {
        self.token.as_ref().is_some_and(|t| t.job_ids.is_some())
    }

    /// What the user (ignoring any token) may do on a job.
    pub fn user_permission(&self, job_id: &str, job_name: &str) -> Permission {
        if self.role == Role::Admin {
            return Permission::Full;
        }
        let best = self.rules.iter().filter(|r| r.applies_to(&self.user, self.role) && r.matches_job(job_id, job_name)).map(|r| r.permission).max().unwrap_or_default();
        best.min(self.role.ceiling())
    }

    /// The effective permission on a job.
    pub fn job_permission(&self, job_id: &str, job_name: &str) -> Permission {
        let user = self.user_permission(job_id, job_name);
        match &self.token {
            None => user,
            Some(t) => {
                if t.job_ids.as_ref().is_some_and(|ids| !ids.iter().any(|i| i == job_id)) {
                    return Permission::None;
                }
                user.min(Permission::of_scope(t.scope))
            }
        }
    }

    pub fn allows_global(&self, need: Global) -> bool {
        let token_ok = match &self.token {
            None => true,
            Some(t) if t.job_ids.is_some() => need == Global::Any || t.scope == Scope::Admin,
            Some(_) => true,
        };
        let role_ok = match need {
            Global::Any | Global::User => true,
            Global::Operator => self.role >= Role::Operator,
            // Token scopes are checked per route by `api_tokens::required_scope`.
            Global::Admin => self.role == Role::Admin,
        };
        token_ok && role_ok
    }

    pub fn is_admin(&self) -> bool {
        self.allows_global(Global::Admin)
    }
}

/// What a route needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Need {
    /// A permission on the job named by the `{id}` path parameter.
    Job(Permission),
    Global(Global),
}

/// The need of `method route` (route template, e.g. `/api/v1/jobs/{id}/run`).
/// Handlers refine it where the body decides (operations, apply template).
pub fn route_need(method: &Method, route: &str) -> Need {
    use Global::*;
    use Permission as P;
    let get = method == Method::GET || method == Method::HEAD;
    if let Some(rest) = route.strip_prefix("/api/v1/jobs/{id}") {
        return match (rest, get) {
            ("/point-in-time", _) => Need::Global(Admin),
            (_, true) if rest == "/export" => Need::Job(P::Full),
            (_, true) => Need::Job(P::Read),
            ("/run" | "/operations", _) => Need::Job(P::Run),
            ("/retention/preview", _) => Need::Job(P::Read),
            ("/hardening/probe", _) => Need::Job(P::Run),
            ("", _) if method == Method::DELETE => Need::Job(P::Full),
            ("/lock" | "/passphrase", _) => Need::Job(P::Full),
            _ => Need::Job(P::Write),
        };
    }
    let g = match (route, get) {
        ("/api/v1/jobs" | "/api/v1/tasks" | "/api/v1/runs" | "/api/v1/status", true) => Any,
        ("/api/v1/jobs/run-all" | "/api/v1/tasks/{id}/cancel" | "/api/v1/backup-schedule/preview", false) => Any,
        ("/api/v1/auth/me", true) => Any,
        ("/api/v1/modules" | "/api/v1/settings" | "/api/v1/metrics" | "/api/v1/backup-cache" | "/api/v1/backup-cache/settings", true) => User,
        ("/api/v1/backup-templates" | "/api/v1/backup-templates/{id}" | "/api/v1/backup-templates/export", true) => User,
        // Applying needs write on the job too (checked by the handler).
        ("/api/v1/backup-templates/{id}/apply", false) => User,
        ("/api/v1/auth/api-tokens" | "/api/v1/auth/api-tokens/{id}" | "/api/v1/auth/signin-token" | "/api/v1/auth/automation-token" | "/api/v1/auth/password", _) => User,
        ("/api/v1/filesystem", true) | ("/api/v1/destinations/test", false) => Operator,
        // The RPC handler checks each command.
        ("/api/v1/rpc/{command}", false) => Any,
        _ => Admin,
    };
    Need::Global(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(scope: Scope, jobs: Option<&[&str]>) -> Access {
        Access { token: Some(TokenLimit { id: "t".into(), scope, job_ids: jobs.map(|j| j.iter().map(|s| s.to_string()).collect()) }), ..Access::admin() }
    }

    #[test]
    fn globs() {
        assert!(glob("*", "anything"));
        assert!(glob("prod-*", "prod-db"));
        assert!(glob("a?c", "abc"));
        assert!(!glob("a?c", "ac"));
        assert!(glob("*db*", "my-db-1"));
        assert!(!glob("prod-*", "dev-db"));
        assert!(glob("", ""));
        assert!(!glob("", "x"));
    }

    #[test]
    fn token_scope_and_jobs_intersect() {
        let a = tok(Scope::Run, Some(&["j1"]));
        assert_eq!(a.job_permission("j1", "One"), Permission::Run);
        assert_eq!(a.job_permission("j2", "Two"), Permission::None);
        assert!(a.allows_global(Global::Any));
        assert!(!a.allows_global(Global::User) && !a.is_admin());
        let all = tok(Scope::Read, None);
        assert_eq!(all.job_permission("j2", "Two"), Permission::Read);
        assert!(all.allows_global(Global::User));
        let adm = tok(Scope::Admin, Some(&["j1"]));
        assert!(adm.is_admin(), "job-limited tokens with the admin scope may use global routes");
        assert_eq!(adm.job_permission("j2", ""), Permission::None);
        assert_eq!(Access::admin().job_permission("x", ""), Permission::Full);
    }

    #[test]
    fn route_needs() {
        let (g, p, d) = (Method::GET, Method::POST, Method::DELETE);
        assert_eq!(route_need(&g, "/api/v1/jobs/{id}/versions"), Need::Job(Permission::Read));
        assert_eq!(route_need(&p, "/api/v1/jobs/{id}/run"), Need::Job(Permission::Run));
        assert_eq!(route_need(&Method::PUT, "/api/v1/jobs/{id}"), Need::Job(Permission::Write));
        assert_eq!(route_need(&d, "/api/v1/jobs/{id}"), Need::Job(Permission::Full));
        assert_eq!(route_need(&d, "/api/v1/jobs/{id}/lock"), Need::Job(Permission::Full));
        assert_eq!(route_need(&g, "/api/v1/jobs/{id}/export"), Need::Job(Permission::Full));
        assert_eq!(route_need(&g, "/api/v1/jobs"), Need::Global(Global::Any));
        assert_eq!(route_need(&p, "/api/v1/jobs"), Need::Global(Global::Admin));
        assert_eq!(route_need(&p, "/api/v1/jobs/run-all"), Need::Global(Global::Any));
        assert_eq!(route_need(&Method::PUT, "/api/v1/settings"), Need::Global(Global::Admin));
        assert_eq!(route_need(&p, "/api/v1/server/shutdown"), Need::Global(Global::Admin));
        assert_eq!(route_need(&g, "/api/v1/filesystem"), Need::Global(Global::Operator));
    }
}

/// What an operation (`POST /jobs/{id}/operations`, RPC) needs on its job.
pub fn operation_need(op: &crate::backup_engine::BackupOperation) -> Permission {
    use crate::backup_engine::BackupOperation as O;
    match op {
        O::Backup { .. } | O::Verify { .. } | O::ListBroken | O::Consistency | O::ListLocks | O::FormatInfo | O::SnapshotTest | O::AutoTune { .. } | O::Replicate { .. } | O::SyncSources | O::RecreateDatabase => Permission::Run,
        O::Restore { .. } | O::ExtractControlFiles { .. } | O::SetVersionMetadata { .. } | O::CopyVersions { .. } => Permission::Write,
        O::Compact | O::Repair | O::DeleteVersions { .. } | O::Purge { .. } | O::PurgeBroken { .. } | O::RebuildMissing | O::ApplyLocks { .. } | O::UpgradeFormat { .. } | O::Reshard { .. } | O::ApplyRetention { .. } | O::EnableSharing { .. } => Permission::Full,
    }
}

/// What an RPC command needs.
#[derive(Debug, Clone, PartialEq)]
pub enum RpcNeed {
    Job { id: String, perm: Permission },
    Global(Global),
    /// Answered with only the jobs the caller may see.
    Filtered,
}

/// Desktop IPC commands over `POST /rpc/{command}` for non-administrators.
pub fn rpc_need(cmd: &str, args: &serde_json::Value) -> RpcNeed {
    use Permission as P;
    let id = || args.get("id").or_else(|| args.get("jobId")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let job = |perm| RpcNeed::Job { id: id(), perm };
    match cmd {
        "backup_list_jobs" | "backup_status" | "backup_run_history" | "backup_run_all" | "backup_cancel" => RpcNeed::Filtered,
        "backup_get_job" | "backup_list_versions" | "backup_list_files" | "backup_search" | "backup_compare" | "backup_list_sources" | "backup_preview_retention" | "backup_estimate" | "backup_export_cli" | "backup_effective_job"
        | "backup_lock_status" | "backup_resolve_version" | "backup_read_file" | "backup_job_placeholders" => job(P::Read),
        "backup_hardening_report" if args.get("probe").and_then(|v| v.as_bool()) == Some(true) => job(P::Run),
        "backup_hardening_report" => job(P::Read),
        "backup_effective_settings" if !id().is_empty() => job(P::Read),
        "backup_run" => job(P::Run),
        "backup_execute" | "backup_start_operation" => match args.get("operation").cloned().map(serde_json::from_value::<crate::backup_engine::BackupOperation>) {
            Some(Ok(op)) => job(operation_need(&op)),
            _ => job(P::Full),
        },
        "backup_set_version_metadata" | "backup_set_database_encryption" | "backup_export_file" => job(P::Write),
        "backup_save_job" => match args.pointer("/job/id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            Some(i) => RpcNeed::Job { id: i.to_string(), perm: P::Write },
            None => RpcNeed::Global(Global::Admin),
        },
        "backup_delete_job" | "backup_break_lock" | "backup_export_config" => job(P::Full),
        "backup_passphrase_strength" | "backup_generate_passphrase" | "backup_capabilities" | "backup_cloud_providers" | "backup_export_is_encrypted" | "backup_schedule_preview" => RpcNeed::Global(Global::Any),
        "backup_list_templates" | "backup_get_template" | "backup_cache_stats" | "backup_get_cache_settings" | "backup_get_global_defaults" | "backup_effective_settings" => RpcNeed::Global(Global::User),
        "server_list_directory" | "list_connections" | "backup_test_destination" | "backup_test_destination_compat" | "backup_preview_sources" | "backup_photos_libraries" | "backup_list_destinations" => RpcNeed::Global(Global::Operator),
        _ => RpcNeed::Global(Global::Admin),
    }
}

fn denied(what: &str) -> crate::core::error::AppError {
    crate::core::error::AppError::PermissionDenied { message: format!("You are not allowed to {what}"), advice: "Ask an administrator for access (server access rules, or a token with more jobs or a wider scope).".into() }
}

impl Access {
    /// Refuse unless the caller has `need` on the job.
    pub async fn require_job(&self, mgr: &crate::backup_engine::BackupManager, job_id: &str, need: Permission) -> Result<(), crate::core::error::AppError> {
        let name = mgr.get_job(job_id).await.map(|j| j.name).unwrap_or_default();
        if self.job_permission(job_id, &name) < need {
            return Err(denied(&format!("{} this job (needs {} access)", verb(need), need.as_str())));
        }
        Ok(())
    }

    pub fn require_global(&self, need: Global) -> Result<(), crate::core::error::AppError> {
        if !self.allows_global(need) {
            return Err(denied(match need {
                Global::Admin => "do this (administrators only)",
                Global::Operator => "do this (operators and administrators only)",
                _ => "use server-wide settings with a token limited to some jobs",
            }));
        }
        Ok(())
    }

    /// Ids of the jobs the caller has at least `need` on.
    pub async fn jobs_with(&self, mgr: &crate::backup_engine::BackupManager, need: Permission) -> Vec<String> {
        mgr.list_jobs().await.into_iter().filter(|j| self.job_permission(&j.id, &j.name) >= need).map(|j| j.id).collect()
    }
}

fn verb(p: Permission) -> &'static str {
    match p {
        Permission::Read | Permission::None => "see",
        Permission::Run => "run",
        Permission::Write => "change",
        Permission::Full => "delete or maintain",
    }
}

/// Route middleware (after sign-in): enforce [`route_need`].
pub(crate) async fn authorize(axum::extract::State(st): axum::extract::State<super::ServerState>, req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(access) = req.extensions().get::<Access>().cloned() else { return next.run(req).await };
    let route = req.extensions().get::<axum::extract::MatchedPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let r = match route_need(req.method(), &route) {
        Need::Global(g) => access.require_global(g),
        Need::Job(p) => {
            let id = req.uri().path().split('/').nth(4).map(super::otlp::percent_decode).unwrap_or_default();
            access.require_job(&st.mgr, &id, p).await
        }
    };
    match r {
        Ok(()) => next.run(req).await,
        Err(e) => super::ApiError(e).into_response(),
    }
}

