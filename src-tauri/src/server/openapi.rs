//! OpenAPI 3.1 description of the `ufop-server` REST API, served at
//! `GET /api/v1/openapi.json` (public, like `/health`).
//!
//! One table (`OPS`) lists every operation; the API-token scope each needs
//! comes from `api_tokens::required_scope`, the function that enforces it.
//! A test checks the table against the router so the two cannot drift.

use super::api_tokens;
use axum::http::Method;
use serde_json::{json, Map, Value};

/// A parameter: name, location (`path`/`query`), JSON type, required, description.
type Param = (&'static str, &'static str, &'static str, bool, &'static str);

pub(crate) struct Op {
    pub method: &'static str,
    pub path: &'static str,
    tag: &'static str,
    summary: &'static str,
    /// Needs a session, automation or API token (or reverse-proxy pre-authentication).
    pub auth: bool,
    params: &'static [Param],
    body: Option<fn() -> Value>,
    /// Response schema (200, `application/json` unless `content_type` says otherwise).
    response: fn() -> Value,
    content_type: &'static str,
}

const ID: Param = ("id", "path", "string", true, "Job id");
const VERSION_Q: Param = ("version", "query", "integer", false, "Version index (0 = newest)");

fn any() -> Value {
    json!({ "type": "object", "additionalProperties": true })
}
fn ok_flag() -> Value {
    json!({ "type": "object", "properties": { "ok": { "type": "boolean" } } })
}
fn session() -> Value {
    json!({ "$ref": "#/components/schemas/Session" })
}
fn job() -> Value {
    json!({ "$ref": "#/components/schemas/BackupJob" })
}
fn list() -> Value {
    json!({ "type": "array", "items": { "type": "object", "additionalProperties": true } })
}
fn status() -> Value {
    json!({ "$ref": "#/components/schemas/SchedulerStatus" })
}
fn save_job() -> Value {
    json!({ "type": "object", "required": ["job"], "properties": { "job": job(), "passphrase": { "type": ["string", "null"], "description": "Encryption passphrase (stored sealed)" } } })
}
fn html() -> Value {
    json!({ "type": "string" })
}

macro_rules! op {
    ($m:literal $p:literal, $tag:literal, $sum:literal, auth: $auth:expr, params: $params:expr, body: $body:expr, response: $resp:expr) => {
        Op { method: $m, path: $p, tag: $tag, summary: $sum, auth: $auth, params: $params, body: $body, response: $resp, content_type: "application/json" }
    };
}

pub(crate) const OPS: &[Op] = &[
    op!("get" "/api/v1/health", "general", "Liveness, version and scheduler summary", auth: false, params: &[], body: None, response: || json!({ "type": "object", "properties": { "status": { "const": "ok" }, "version": { "type": "string" }, "uptime_secs": { "type": "integer" }, "scheduler_running": { "type": "boolean" }, "scheduler_paused": { "type": "boolean" }, "active_tasks": { "type": "integer" }, "queued_tasks": { "type": "integer" } } })),
    op!("get" "/api/v1/openapi.json", "general", "This document", auth: false, params: &[], body: None, response: any),
    op!("post" "/api/v1/auth/login", "auth", "Sign in with the server password; sets the refresh cookie", auth: false, params: &[], body: Some(|| json!({ "type": "object", "required": ["password"], "properties": { "password": { "type": "string", "format": "password" } } })), response: session),
    op!("post" "/api/v1/auth/refresh", "auth", "New access token from the refresh cookie (rotated)", auth: false, params: &[], body: None, response: session),
    op!("post" "/api/v1/auth/logout", "auth", "Revoke the refresh cookie's session", auth: false, params: &[], body: None, response: ok_flag),
    op!("post" "/api/v1/auth/signin", "auth", "Redeem a one-time sign-in token", auth: false, params: &[], body: Some(|| json!({ "type": "object", "required": ["token"], "properties": { "token": { "type": "string" } } })), response: session),
    op!("post" "/api/v1/auth/signin-token", "auth", "Issue a one-time sign-in token (link for another browser)", auth: true, params: &[], body: None, response: any),
    op!("post" "/api/v1/auth/automation-token", "auth", "Issue a long-lived automation token (when allowed in settings)", auth: true, params: &[], body: None, response: any),
    op!("get" "/api/v1/auth/api-tokens", "auth", "List API tokens (no secrets)", auth: true, params: &[], body: None, response: || json!({ "type": "object", "properties": { "tokens": { "type": "array", "items": { "$ref": "#/components/schemas/ApiToken" } } } })),
    op!("post" "/api/v1/auth/api-tokens", "auth", "Create a scoped API token (interactive sessions only); the secret is returned once", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["name", "scope"], "properties": { "name": { "type": "string" }, "scope": { "$ref": "#/components/schemas/Scope" }, "expires_in_days": { "type": "integer", "minimum": 1, "maximum": api_tokens::MAX_DAYS, "default": 90 } } })), response: || json!({ "type": "object", "properties": { "token": { "type": "string", "description": "ufop_pat_…" }, "info": { "$ref": "#/components/schemas/ApiToken" } } })),
    op!("delete" "/api/v1/auth/api-tokens/{id}", "auth", "Revoke an API token", auth: true, params: &[("id", "path", "string", true, "Token id")], body: None, response: ok_flag),
    op!("post" "/api/v1/auth/reset", "auth", "Rotate the signing key: every token and session ends", auth: true, params: &[], body: None, response: ok_flag),
    op!("put" "/api/v1/auth/password", "auth", "Change the server password", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["current_password", "new_password"], "properties": { "current_password": { "type": "string", "format": "password" }, "new_password": { "type": "string", "format": "password" } } })), response: ok_flag),
    op!("get" "/api/v1/status", "general", "Scheduler state and tasks", auth: true, params: &[], body: None, response: status),
    op!("get" "/api/v1/modules", "general", "Enabled and disabled modules", auth: true, params: &[], body: None, response: any),
    op!("get" "/api/v1/settings", "general", "Server settings", auth: true, params: &[], body: None, response: || json!({ "type": "object", "properties": { "allow_automation_tokens": { "type": "boolean" } } })),
    op!("put" "/api/v1/settings", "general", "Change server settings", auth: true, params: &[], body: Some(|| json!({ "type": "object", "properties": { "allow_automation_tokens": { "type": "boolean" } } })), response: || json!({ "type": "object", "properties": { "allow_automation_tokens": { "type": "boolean" } } })),
    op!("post" "/api/v1/rpc/{command}", "general", "Call a desktop IPC command by name (arguments as the JSON body); see docs/ipc-commands.md", auth: true, params: &[("command", "path", "string", true, "Command name, e.g. backup_list_jobs")], body: Some(any), response: any),
    op!("get" "/api/v1/jobs", "jobs", "List backup jobs", auth: true, params: &[], body: None, response: || json!({ "type": "array", "items": job() })),
    op!("post" "/api/v1/jobs", "jobs", "Create a job", auth: true, params: &[], body: Some(save_job), response: job),
    op!("post" "/api/v1/jobs/import", "jobs", "Import an exported job configuration", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["config"], "properties": { "config": { "type": "string", "description": "Exported JSON (optionally encrypted)" }, "export_passphrase": { "type": ["string", "null"] }, "import_stats": { "type": "boolean", "default": false }, "save": { "type": "boolean", "default": true } } })), response: any),
    op!("get" "/api/v1/jobs/{id}", "jobs", "Get a job", auth: true, params: &[ID], body: None, response: job),
    op!("put" "/api/v1/jobs/{id}", "jobs", "Replace a job", auth: true, params: &[ID], body: Some(save_job), response: job),
    op!("delete" "/api/v1/jobs/{id}", "jobs", "Delete a job", auth: true, params: &[ID, ("delete_local_db", "query", "boolean", false, "Also delete its local database"), ("delete_remote", "query", "boolean", false, "Also delete the backup at the destination")], body: None, response: ok_flag),
    op!("post" "/api/v1/jobs/{id}/run", "jobs", "Start a backup (queued); returns the task id", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "properties": { "dry_run": { "type": "boolean", "default": false } } })), response: || json!({ "type": "object", "properties": { "task_id": { "type": "string" } } })),
    op!("post" "/api/v1/jobs/{id}/operations", "jobs", "Restore, verify, compact, repair, purge … (a tagged BackupOperation; not `backup`)", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "required": ["type"], "properties": { "type": { "type": "string", "description": "snake_case operation, e.g. restore, verify, compact, repair" } }, "additionalProperties": true })), response: any),
    op!("get" "/api/v1/jobs/{id}/versions", "browse", "Backup versions of a job", auth: true, params: &[ID], body: None, response: list),
    op!("get" "/api/v1/jobs/{id}/files", "browse", "Files in a version", auth: true, params: &[ID, VERSION_Q, ("prefix", "query", "string", false, "Folder to list")], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/search", "browse", "Search file names across versions", auth: true, params: &[ID, ("q", "query", "string", true, "Search text"), VERSION_Q], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/compare", "browse", "Differences between two versions", auth: true, params: &[ID, ("base", "query", "integer", true, "Older version index"), ("compare", "query", "integer", true, "Newer version index")], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/export", "jobs", "Export the job configuration", auth: true, params: &[ID, ("include_passphrase", "query", "boolean", false, "Include the job's passphrase"), ("export_passphrase", "query", "string", false, "Encrypt the export with this passphrase")], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/cli", "jobs", "The equivalent `ufop backup` command line", auth: true, params: &[ID], body: None, response: || json!({ "type": "object", "properties": { "command": { "type": "string" } } })),
    op!("get" "/api/v1/tasks", "tasks", "Queued and running tasks", auth: true, params: &[], body: None, response: || json!({ "type": "array", "items": { "$ref": "#/components/schemas/TaskInfo" } })),
    op!("post" "/api/v1/tasks/{id}/cancel", "tasks", "Cancel a task", auth: true, params: &[("id", "path", "string", true, "Task id")], body: None, response: || json!({ "type": "object", "properties": { "cancelled": { "type": "boolean" } } })),
    op!("get" "/api/v1/runs", "tasks", "Run history", auth: true, params: &[("job_id", "query", "string", false, "Only this job"), ("limit", "query", "integer", false, "Most recent N (default 50)")], body: None, response: list),
    op!("post" "/api/v1/destinations/test", "jobs", "Test a destination (optionally create its folder); returns the files found", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["destination"], "properties": { "destination": { "type": "object", "additionalProperties": true, "description": "BackupDestination" }, "create": { "type": "boolean", "default": false } } })), response: || json!({ "type": "object", "properties": { "files": { "type": "array", "items": { "type": "string" } } } })),
    op!("get" "/api/v1/filesystem", "general", "Browse the server's file system (source picker)", auth: true, params: &[("path", "query", "string", false, "Directory (default: home)"), ("folders_only", "query", "boolean", false, "Only folders")], body: None, response: any),
    op!("post" "/api/v1/scheduler/pause", "tasks", "Pause the scheduler (optionally for N minutes)", auth: true, params: &[], body: Some(|| json!({ "type": "object", "properties": { "minutes": { "type": ["integer", "null"], "minimum": 1 } } })), response: status),
    op!("post" "/api/v1/scheduler/resume", "tasks", "Resume the scheduler", auth: true, params: &[], body: None, response: status),
    op!("get" "/api/v1/oauth/providers", "oauth", "Configured OAuth providers for cloud destinations", auth: true, params: &[], body: None, response: list),
    Op { method: "get", path: "/oauth/{provider}/start", tag: "oauth", summary: "Begin authorization with a provider (redirect)", auth: false, params: &[("provider", "path", "string", true, "Provider id")], body: None, response: html, content_type: "text/html" },
    Op { method: "get", path: "/oauth/callback", tag: "oauth", summary: "Provider redirect target; shows the AuthID to paste into the destination", auth: false, params: &[("code", "query", "string", false, ""), ("state", "query", "string", false, ""), ("error", "query", "string", false, "")], body: None, response: html, content_type: "text/html" },
    op!("get" "/oauth/token", "oauth", "Access token for an AuthID (the AuthID is the credential)", auth: false, params: &[("authid", "query", "string", true, "AuthID from the callback")], body: None, response: || json!({ "type": "object", "properties": { "access_token": { "type": "string" }, "expires_at": { "type": "string", "format": "date-time" } } })),
];

fn components() -> Value {
    json!({
        "securitySchemes": {
            "bearer": { "type": "http", "scheme": "bearer", "description": "Access token from /auth/login, an automation token, or an API token (ufop_pat_…, scope-limited)" },
            "preauth": { "type": "apiKey", "in": "header", "name": super::PREAUTH_HEADER, "description": "Reverse-proxy pre-authentication (--preauth-token)" }
        },
        "schemas": {
            "Error": { "type": "object", "required": ["message"], "properties": { "error_type": { "type": "string" }, "message": { "type": "string" }, "advice": { "type": "string" } } },
            "Session": { "type": "object", "properties": { "access_token": { "type": "string" }, "expires_at": { "type": "string", "format": "date-time" } } },
            "Scope": { "type": "string", "enum": ["read", "run", "write", "admin"], "description": "read < run < write < admin" },
            "ApiToken": { "type": "object", "properties": { "id": { "type": "string" }, "name": { "type": "string" }, "scope": { "$ref": "#/components/schemas/Scope" }, "created_at": { "type": "string" }, "expires_at": { "type": "string" }, "last_used_at": { "type": ["string", "null"] } }, "additionalProperties": true },
            "BackupJob": { "type": "object", "description": "A backup job (sources, destination, schedule, retention, filters, encryption); see docs/api-reference.md", "additionalProperties": true },
            "TaskInfo": { "type": "object", "properties": { "task_id": { "type": "string" }, "job_id": { "type": "string" }, "job_name": { "type": "string" }, "operation": { "type": "string" }, "state": { "type": "string", "enum": ["queued", "running"] }, "queued_at": { "type": "string" }, "started_at": { "type": ["string", "null"] }, "elapsed_secs": { "type": "integer" } }, "additionalProperties": true },
            "SchedulerStatus": { "type": "object", "properties": { "running": { "type": "boolean" }, "paused": { "type": "boolean" }, "tasks": { "type": "array", "items": { "$ref": "#/components/schemas/TaskInfo" } } }, "additionalProperties": true }
        }
    })
}

fn method(m: &str) -> Method {
    m.to_ascii_uppercase().parse().unwrap_or(Method::GET)
}

/// The document.
pub fn document() -> Value {
    let error = |d: &str| json!({ "description": d, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } });
    let mut paths = Map::new();
    for op in OPS {
        let mut o = json!({
            "operationId": format!("{}_{}", op.method, op.path.trim_start_matches('/').replace(['/', '{', '}', '.', '-'], "_").trim_end_matches('_')),
            "tags": [op.tag],
            "summary": op.summary,
            "responses": { "200": { "description": "OK", "content": { op.content_type: { "schema": (op.response)() } } } },
        });
        if !op.params.is_empty() {
            o["parameters"] = op.params.iter().map(|(name, at, ty, req, desc)| json!({ "name": name, "in": at, "required": req, "description": desc, "schema": { "type": ty } })).collect();
        }
        if let Some(b) = op.body {
            o["requestBody"] = json!({ "required": true, "content": { "application/json": { "schema": b() } } });
        }
        if op.auth {
            let scope = api_tokens::required_scope(&method(op.method), op.path);
            o["security"] = json!([{ "bearer": [] }, { "preauth": [] }]);
            o["x-required-scope"] = json!(scope.as_str());
            o["description"] = json!(format!("API tokens need the `{}` scope.", scope.as_str()));
            o["responses"]["401"] = error("Not signed in, the token is invalid or expired, or the API token's scope is too narrow");
        }
        o["responses"]["400"] = error("Invalid request");
        o["responses"]["403"] = error("The client address is outside --allowed-cidrs");
        o["responses"]["421"] = error("The Host header is not in --allowed-hostnames");
        paths.entry(op.path.to_string()).or_insert_with(|| json!({}))[op.method] = o;
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "UFOP server API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "REST API of ufop-server (headless backup server). Errors are `{error_type, message, advice}`. Responses carry security headers; HTTPS, allowed hosts and client networks are server options.",
        },
        "servers": [{ "url": "/" }],
        "tags": [
            { "name": "general" }, { "name": "auth" }, { "name": "jobs" }, { "name": "browse" }, { "name": "tasks" }, { "name": "oauth" }
        ],
        "paths": paths,
        "components": components(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// `(method, path)` of every `.route(…)` in the router sources.
    fn routed() -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        for src in [include_str!("mod.rs"), include_str!("oauth.rs")] {
            for chunk in src.split(".route(\"").skip(1) {
                let Some((path, rest)) = chunk.split_once('"') else { continue };
                let handlers = &rest[..rest.find(".route(").unwrap_or(rest.len()).min(rest.find('\n').unwrap_or(rest.len()))];
                for m in ["get", "post", "put", "delete"] {
                    let calls = [format!(" {m}("), format!(".{m}("), format!("::{m}(")];
                    if calls.iter().any(|c| handlers.contains(c.as_str())) {
                        out.insert((m.to_string(), path.to_string()));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn documents_exactly_the_routes() {
        let documented: BTreeSet<(String, String)> = OPS.iter().map(|o| (o.method.to_string(), o.path.to_string())).collect();
        let routed = routed();
        assert!(routed.len() > 40, "parsed {} routes", routed.len());
        let missing: Vec<_> = routed.difference(&documented).collect();
        let stale: Vec<_> = documented.difference(&routed).collect();
        assert!(missing.is_empty(), "routes without OpenAPI entries: {missing:?}");
        assert!(stale.is_empty(), "OpenAPI entries without routes: {stale:?}");
        assert_eq!(documented.len(), OPS.len(), "duplicate operations");
    }

    #[test]
    fn document_is_well_formed() {
        let d = document();
        assert_eq!(d["openapi"], "3.1.0");
        let paths = d["paths"].as_object().unwrap();
        let mut ids = BTreeSet::new();
        for (path, item) in paths {
            for (m, op) in item.as_object().unwrap() {
                assert!(ids.insert(op["operationId"].as_str().unwrap().to_string()), "duplicate operationId at {m} {path}");
                // Every `{param}` in the path is declared.
                for seg in path.split('/').filter(|s| s.starts_with('{')) {
                    let name = seg.trim_matches(['{', '}']);
                    assert!(op["parameters"].as_array().is_some_and(|ps| ps.iter().any(|p| p["name"] == name && p["in"] == "path")), "{m} {path}: {name}");
                }
                // Every $ref resolves.
                let text = op.to_string();
                for r in text.split("#/components/schemas/").skip(1) {
                    let name: String = r.chars().take_while(|c| c.is_alphanumeric()).collect();
                    assert!(d["components"]["schemas"].get(&name).is_some(), "{m} {path}: unknown schema {name}");
                }
            }
        }
        assert_eq!(d["paths"]["/api/v1/jobs/{id}/run"]["post"]["x-required-scope"], "run");
        assert_eq!(d["paths"]["/api/v1/jobs"]["get"]["x-required-scope"], "read");
        assert_eq!(d["paths"]["/api/v1/rpc/{command}"]["post"]["x-required-scope"], "admin");
        assert!(d["paths"]["/api/v1/health"]["get"].get("security").is_none());
    }
}
