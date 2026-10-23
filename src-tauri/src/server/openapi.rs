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
const TEMPLATE_ID: Param = ("id", "path", "string", true, "Policy template id (organization templates start with org:)");
const VERSION_Q: Param = ("version", "query", "integer", false, "Version index (0 = newest)");
const VERSION_SEL_Q: Param = ("version", "query", "string", false, "Version: number (0 = newest), latest, oldest or a version id (timestamp or file-list name)");
const TIME_Q: Param = ("time", "query", "string", false, "As of this time: the newest complete version made at or before it (RFC 3339, or 2026-10-01 09:00 in the server's time zone); wins over version");
const PARTIAL_Q: Param = ("allow_partial", "query", "boolean", false, "latest / oldest / time may pick partial versions");
const SOURCE_Q: Param = ("source", "query", "string", false, "Shared repositories: another source id, or * for every source (default: the job's own)");

fn cache_stats() -> Value {
    json!({ "type": "object", "properties": { "dir": { "type": "string" }, "max_bytes": { "type": "integer" }, "entries": { "type": "integer" }, "bytes": { "type": "integer" }, "hits": { "type": "integer" }, "misses": { "type": "integer" }, "corrupt": { "type": "integer" }, "evictions": { "type": "integer" }, "hit_rate": { "type": "number" } } })
}
fn cache_settings() -> Value {
    json!({ "type": "object", "properties": { "dir": { "type": ["string", "null"], "description": "Cache folder (default: the app's cache folder)" }, "max_bytes": { "type": "integer", "default": 2147483648_u64, "description": "0 = cache off" }, "use_for_restore": { "type": "boolean", "default": false } } })
}
fn template() -> Value {
    json!({ "type": "object", "required": ["name"], "properties": {
        "id": { "type": "string" }, "name": { "type": "string" }, "description": { "type": "string" },
        "settings": { "type": "object", "description": "Partial job settings: options (backup option name → value), filter, retention, schedule, maintenance, scripts, notifications, notify_on, throttle, retry, prevent_sleep. Absent = not set.", "additionalProperties": true },
        "updated_at": { "type": "string" }, "managed": { "type": "boolean", "description": "Delivered by the management console (read-only)" }, "enforced": { "type": "boolean", "description": "Managed only: wins over every job's own settings" } } })
}
fn templates() -> Value {
    json!({ "type": "array", "items": template() })
}
fn schedule() -> Value {
    json!({ "type": "object", "description": "BackupSchedule: enabled, start_time (HH:MM), interval_minutes, weekdays (0 = Mon), cron (expressions; when set they replace start time, interval and weekdays), skip_on_battery, calendar_ids", "additionalProperties": true })
}
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
fn folder_policy_result() -> Value {
    json!({ "type": "object", "properties": { "path": { "type": "string" }, "own": { "oneOf": [{ "$ref": "#/components/schemas/FolderPolicy" }, { "type": "null" }] }, "effective": { "type": "object", "additionalProperties": true } } })
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
    Op { method: "get", path: "/api/v1/sdk/{language}", tag: "general", summary: "Generated client library for this API (typescript or python)", auth: false, params: &[("language", "path", "string", true, "typescript or python")], body: None, response: html, content_type: "text/plain" },
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
    op!("post" "/api/v1/jobs/run-all", "jobs", "Back up every job now (queued; read-only and busy jobs are skipped)", auth: true, params: &[], body: Some(|| json!({ "type": "object", "properties": { "job_ids": { "type": "array", "items": { "type": "string" }, "description": "Only these jobs (default: all)" }, "scheduled_only": { "type": "boolean", "default": false }, "dry_run": { "type": "boolean", "default": false } } })), response: || json!({ "type": "object", "properties": { "started": { "type": "array", "items": { "type": "object", "properties": { "job_id": { "type": "string" }, "job_name": { "type": "string" }, "task_id": { "type": "string" } } } }, "skipped": { "type": "array", "items": { "type": "object", "properties": { "job_id": { "type": "string" }, "job_name": { "type": "string" }, "reason": { "type": "string" } } } } } })),
    op!("get" "/api/v1/jobs/{id}/estimate", "jobs", "Estimate the next backup: files, new data (at most / at least) and duration; reads nothing", auth: true, params: &[ID, ("assumed_bytes_per_sec", "query", "integer", false, "Speed to assume without run history (default 20 MiB/s, 0 = no duration)")], body: None, response: || json!({ "type": "object", "properties": { "files": { "type": "integer" }, "folders": { "type": "integer" }, "bytes_examined": { "type": "integer" }, "new_files": { "type": "integer" }, "changed_files": { "type": "integer" }, "unchanged_files": { "type": "integer" }, "deleted_files": { "type": "integer" }, "new_bytes_at_most": { "type": "integer" }, "new_bytes_at_least": { "type": "integer" }, "upper_bound": { "type": "boolean" }, "has_previous_version": { "type": "boolean" }, "bytes_per_sec": { "type": ["integer", "null"] }, "speed_source": { "type": "string", "enum": ["history", "assumed", "none"] }, "estimated_duration_secs": { "type": ["integer", "null"] }, "largest": { "type": "array", "items": { "type": "object", "properties": { "path": { "type": "string" }, "size": { "type": "integer" }, "status": { "type": "string", "enum": ["new", "changed"] } } } }, "warnings": { "type": "array", "items": { "type": "string" } } } })),
    op!("post" "/api/v1/jobs/{id}/run", "jobs", "Start a backup (queued); returns the task id", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "properties": { "dry_run": { "type": "boolean", "default": false } } })), response: || json!({ "type": "object", "properties": { "task_id": { "type": "string" } } })),
    op!("post" "/api/v1/jobs/{id}/operations", "jobs", "Restore, verify, compact, repair, purge … (a tagged BackupOperation; not `backup`). Verify takes `options`: scope {versions, paths}, mode {type: structure | sample_volumes(count) | sample_percent(percent) | full}, files_percent, parallelism, max_errors, use_cache", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "required": ["type"], "properties": { "type": { "type": "string", "description": "snake_case operation, e.g. restore, verify, compact, repair" } }, "additionalProperties": true })), response: any),
    op!("post" "/api/v1/jobs/{id}/point-in-time", "jobs", "Open the job's versioned S3 destination as it was at a time: creates a read-only job whose index is rebuilt from the object versions of then", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "required": ["at"], "properties": { "at": { "type": "string", "format": "date-time" }, "name": { "type": ["string", "null"], "description": "Name of the new job (default: '<job> (as of <time>)')" } } })), response: job),
    op!("put" "/api/v1/jobs/{id}/passphrase", "jobs", "Change an encrypted job's passphrase (format 2 repositories rewrite only the header)", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "required": ["new_passphrase"], "properties": { "current": { "type": ["string", "null"], "description": "Current passphrase (default: the stored one)" }, "new_passphrase": { "type": "string" }, "kdf_preset": { "type": ["string", "null"], "enum": ["interactive", "default", "sensitive", null] } } })), response: any),
    op!("get" "/api/v1/jobs/{id}/lock", "jobs", "Repository lock status: who holds the destination (host, operation, since, expires)", auth: true, params: &[ID], body: None, response: || json!({ "type": "object", "properties": { "locks": { "type": "array", "items": { "type": "object", "properties": { "file": { "type": "string" }, "owner_id": { "type": "string" }, "hostname": { "type": "string" }, "username": { "type": "string" }, "pid": { "type": "integer" }, "app": { "type": "string" }, "version": { "type": "string" }, "operation": { "type": "string" }, "kind": { "type": "string", "enum": ["exclusive", "shared"] }, "acquired_at": { "type": "string", "format": "date-time" }, "expires_at": { "type": "string", "format": "date-time" }, "stale": { "type": "boolean" }, "damaged": { "type": "boolean" } } } }, "exclusive": { "type": "boolean" }, "readers": { "type": "integer" }, "checked_at": { "type": "string", "format": "date-time" } } })),
    op!("delete" "/api/v1/jobs/{id}/lock", "jobs", "Break the repository lock (only when its holder is no longer running)", auth: true, params: &[ID, ("confirm", "query", "boolean", true, "Must be true"), ("owner_id", "query", "string", false, "Only remove the lock with this owner id (the one shown)")], body: None, response: || json!({ "type": "object", "properties": { "removed": { "type": "array", "items": { "type": "string" } }, "holders": { "type": "array", "items": { "type": "string" } } } })),
    op!("put" "/api/v1/jobs/{id}/database-encryption", "jobs", "Encrypt or decrypt the job's local database now (key kept in the credential store)", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "required": ["enabled"], "properties": { "enabled": { "type": "boolean" } } })), response: || json!({ "type": "object", "properties": { "encrypted": { "type": "boolean" }, "files": { "type": "integer" }, "settings": { "type": "integer" } } })),
    op!("get" "/api/v1/jobs/{id}/versions", "browse", "Backup versions of a job (with note, tags, pin, identity and storage added)", auth: true, params: &[ID, SOURCE_Q, ("tags", "query", "string", false, "Comma-separated key=value (or key) tags a version must carry")], body: None, response: || json!({ "type": "array", "items": { "$ref": "#/components/schemas/Fileset" } })),
    op!("patch" "/api/v1/jobs/{id}/versions/{version}", "browse", "Change a version's note, tags or pin (rewrites its file list)", auth: true, params: &[ID, ("version", "path", "integer", true, "Version index (0 = newest)")], body: Some(|| json!({ "type": "object", "properties": { "source": { "type": ["string", "null"] }, "note": { "type": ["string", "null"], "description": "Empty string removes the note" }, "tags": { "type": ["object", "null"], "additionalProperties": { "type": "string" }, "description": "Replaces all tags" }, "pinned": { "type": ["boolean", "null"] }, "pin_reason": { "type": ["string", "null"] } } })), response: any),
    op!("get" "/api/v1/jobs/{id}/sources", "browse", "Sources (jobs and computers) with versions in the job's repository", auth: true, params: &[ID], body: None, response: || json!({ "type": "array", "items": { "type": "object", "properties": { "id": { "type": "string" }, "label": { "type": "string" }, "identity": { "type": ["object", "null"] }, "versions": { "type": "integer" }, "newest": { "type": ["string", "null"] }, "oldest": { "type": ["string", "null"] }, "newest_size": { "type": "integer" }, "own": { "type": "boolean" } } } })),
    op!("post" "/api/v1/jobs/{id}/retention/preview", "browse", "What retention would keep and delete (the job's policy or an unsaved one); changes nothing", auth: true, params: &[ID], body: Some(|| json!({ "type": "object", "properties": { "policy": { "$ref": "#/components/schemas/RetentionPolicy" }, "source": { "type": ["string", "null"] } } })), response: || json!({ "type": "object", "properties": { "policy": { "$ref": "#/components/schemas/RetentionPolicy" }, "legal_hold": { "type": "boolean" }, "keep": { "type": "integer" }, "delete": { "type": "integer" }, "versions": { "type": "array", "items": { "type": "object", "properties": { "id": { "type": "integer" }, "timestamp": { "type": "string" }, "keep": { "type": "boolean" }, "reasons": { "type": "array", "items": { "type": "string" } }, "summary": { "type": "string" } } } } } })),
    op!("get" "/api/v1/jobs/{id}/files", "browse", "Files in a version", auth: true, params: &[ID, VERSION_SEL_Q, TIME_Q, PARTIAL_Q, ("prefix", "query", "string", false, "Folder to list"), SOURCE_Q], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/versions/resolve", "browse", "Which version a time (or latest / oldest / number / id) picks", auth: true, params: &[ID, VERSION_SEL_Q, TIME_Q, PARTIAL_Q, SOURCE_Q], body: None, response: || json!({ "type": "object", "properties": { "index": { "type": "integer" }, "fileset": { "$ref": "#/components/schemas/Fileset" } } })),
    Op { method: "get", path: "/api/v1/jobs/{id}/file", tag: "browse", summary: "One backed-up file's content, streamed (single Range requests answer 206)", auth: true, params: &[ID, ("path", "query", "string", true, "Backed-up path of the file"), VERSION_SEL_Q, TIME_Q, PARTIAL_Q, SOURCE_Q], body: None, response: || json!({ "type": "string", "format": "binary" }), content_type: "application/octet-stream" },
    op!("get" "/api/v1/backup-cache", "general", "Local volume cache: folder, size limit, entries, bytes, hits, misses, hit rate", auth: true, params: &[], body: None, response: cache_stats),
    op!("delete" "/api/v1/backup-cache", "general", "Delete every cached volume", auth: true, params: &[], body: None, response: || json!({ "type": "object", "properties": { "bytes_freed": { "type": "integer" } } })),
    op!("get" "/api/v1/backup-cache/settings", "general", "Volume cache settings", auth: true, params: &[], body: None, response: cache_settings),
    op!("put" "/api/v1/backup-cache/settings", "general", "Change the volume cache folder, size limit (0 = off) and use for full restores", auth: true, params: &[], body: Some(cache_settings), response: cache_stats),
    op!("get" "/api/v1/jobs/{id}/search", "browse", "Search file names across versions", auth: true, params: &[ID, ("q", "query", "string", true, "Search text"), VERSION_Q, SOURCE_Q], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/compare", "browse", "Differences between two versions", auth: true, params: &[ID, ("base", "query", "integer", true, "Older version index"), ("compare", "query", "integer", true, "Newer version index"), SOURCE_Q], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/export", "jobs", "Export the job configuration", auth: true, params: &[ID, ("include_passphrase", "query", "boolean", false, "Include the job's passphrase"), ("export_passphrase", "query", "string", false, "Encrypt the export with this passphrase")], body: None, response: any),
    op!("get" "/api/v1/jobs/{id}/cli", "jobs", "The equivalent `ufop backup` command line", auth: true, params: &[ID], body: None, response: || json!({ "type": "object", "properties": { "command": { "type": "string" } } })),
    op!("get" "/api/v1/tasks", "tasks", "Queued and running tasks", auth: true, params: &[], body: None, response: || json!({ "type": "array", "items": { "$ref": "#/components/schemas/TaskInfo" } })),
    op!("post" "/api/v1/tasks/{id}/cancel", "tasks", "Cancel a task", auth: true, params: &[("id", "path", "string", true, "Task id")], body: None, response: || json!({ "type": "object", "properties": { "cancelled": { "type": "boolean" } } })),
    op!("get" "/api/v1/runs", "tasks", "Run history", auth: true, params: &[("job_id", "query", "string", false, "Only this job"), ("limit", "query", "integer", false, "Most recent N (default 50)")], body: None, response: list),
    op!("post" "/api/v1/destinations/test", "jobs", "Test a destination (optionally create its folder); returns the files found", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["destination"], "properties": { "destination": { "type": "object", "additionalProperties": true, "description": "BackupDestination" }, "create": { "type": "boolean", "default": false } } })), response: || json!({ "type": "object", "properties": { "files": { "type": "array", "items": { "type": "string" } } } })),
    op!("get" "/api/v1/filesystem", "general", "Browse the server's file system (source picker)", auth: true, params: &[("path", "query", "string", false, "Directory (default: home)"), ("folders_only", "query", "boolean", false, "Only folders")], body: None, response: any),
    op!("post" "/api/v1/scheduler/pause", "tasks", "Pause the scheduler (optionally for N minutes)", auth: true, params: &[], body: Some(|| json!({ "type": "object", "properties": { "minutes": { "type": ["integer", "null"], "minimum": 1 } } })), response: status),
    op!("post" "/api/v1/scheduler/resume", "tasks", "Resume the scheduler", auth: true, params: &[], body: None, response: status),
    op!("post" "/api/v1/backup-schedule/preview", "jobs", "Validate a schedule (interval or cron) and list its next runs, marking holiday skips", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["schedule"], "properties": { "schedule": schedule(), "count": { "type": "integer", "minimum": 1, "maximum": 100, "default": 5 } } })), response: || json!({ "type": "array", "items": { "type": "object", "properties": { "at": { "type": "string" }, "skipped_by": { "type": ["string", "null"] } } } })),
    op!("get" "/api/v1/backup-templates", "jobs", "Policy templates (local, then the organization's)", auth: true, params: &[], body: None, response: templates),
    op!("post" "/api/v1/backup-templates", "jobs", "Create a policy template", auth: true, params: &[], body: Some(template), response: template),
    op!("get" "/api/v1/backup-templates/export", "jobs", "Templates as a JSON file", auth: true, params: &[("ids", "query", "string", false, "Comma-separated template ids (default: all)")], body: None, response: any),
    op!("post" "/api/v1/backup-templates/import", "jobs", "Import a template file (same id or name updates)", auth: true, params: &[], body: Some(any), response: templates),
    op!("get" "/api/v1/backup-templates/{id}", "jobs", "One policy template", auth: true, params: &[TEMPLATE_ID], body: None, response: template),
    op!("put" "/api/v1/backup-templates/{id}", "jobs", "Update a local policy template", auth: true, params: &[TEMPLATE_ID], body: Some(template), response: template),
    op!("delete" "/api/v1/backup-templates/{id}", "jobs", "Delete a local policy template no job uses", auth: true, params: &[TEMPLATE_ID], body: None, response: ok_flag),
    op!("post" "/api/v1/backup-templates/{id}/apply", "jobs", "Base a job on this template too (appended: it wins over earlier ones)", auth: true, params: &[TEMPLATE_ID], body: Some(|| json!({ "type": "object", "required": ["job_id"], "properties": { "job_id": { "type": "string" } } })), response: job),
    op!("get" "/api/v1/jobs/{id}/effective-settings", "jobs", "Each setting the job runs with and where it comes from (built_in, global, template, job, managed)", auth: true, params: &[ID], body: None, response: || json!({ "type": "object", "properties": { "job_id": { "type": "string" }, "template_ids": { "type": "array", "items": { "type": "string" } }, "overrides": { "type": ["array", "null"], "items": { "type": "string" } }, "settings": { "type": "array", "items": { "type": "object", "properties": { "field_path": { "type": "string" }, "value": {}, "source": { "type": "object", "properties": { "type": { "type": "string", "enum": ["built_in", "global", "template", "job", "managed"] }, "id": { "type": "string" }, "name": { "type": "string" } } }, "inherited": { "type": ["object", "null"] } } } }, "warnings": { "type": "array", "items": { "type": "string" } } } })),
    op!("get" "/api/v1/oauth/providers", "oauth", "Configured OAuth providers for cloud destinations", auth: true, params: &[], body: None, response: list),
    Op { method: "get", path: "/oauth/{provider}/start", tag: "oauth", summary: "Begin authorization with a provider (redirect)", auth: false, params: &[("provider", "path", "string", true, "Provider id")], body: None, response: html, content_type: "text/html" },
    Op { method: "get", path: "/oauth/callback", tag: "oauth", summary: "Provider redirect target; shows the AuthID to paste into the destination", auth: false, params: &[("code", "query", "string", false, ""), ("state", "query", "string", false, ""), ("error", "query", "string", false, "")], body: None, response: html, content_type: "text/html" },
    op!("get" "/api/v1/shares", "shares", "Share links (tokens are never returned) and the folders that may be shared", auth: true, params: &[], body: None, response: || json!({ "type": "object", "properties": { "shares": { "type": "array", "items": { "$ref": "#/components/schemas/Share" } }, "roots": { "type": "array", "items": { "type": "string" } } } })),
    op!("post" "/api/v1/shares", "shares", "Create a share link to a file or folder under --share-root; the link is returned once", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["name", "path"], "properties": { "name": { "type": "string" }, "path": { "type": "string" }, "kind": { "type": "string", "enum": ["download", "upload", "both"], "default": "download" }, "password": { "type": ["string", "null"], "minLength": 8 }, "expires_in_days": { "type": ["integer", "null"], "minimum": 1, "maximum": 3650 }, "max_downloads": { "type": ["integer", "null"], "minimum": 1 }, "max_upload_mb": { "type": ["integer", "null"], "minimum": 1 } } })), response: || json!({ "type": "object", "properties": { "share": { "$ref": "#/components/schemas/Share" }, "token": { "type": "string" }, "url": { "type": "string", "description": "/s/<token>" } } })),
    op!("delete" "/api/v1/shares/{id}", "shares", "Turn a share link off", auth: true, params: &[("id", "path", "string", true, "Share id")], body: None, response: ok_flag),
    op!("get" "/api/v1/shares/{id}/events", "shares", "Access log of a link (views, downloads, uploads, refusals, wrong passwords)", auth: true, params: &[("id", "path", "string", true, "Share id"), ("limit", "query", "integer", false, "Newest N (default 200, max 1000)")], body: None, response: || json!({ "type": "object", "properties": { "events": { "type": "array", "items": { "type": "object", "properties": { "at": { "type": "string" }, "ip": { "type": "string" }, "action": { "type": "string" }, "detail": { "type": "string" } } } } } })),
    op!("get" "/api/v1/folder-policies", "shares", "Folder policies set on shared folders", auth: true, params: &[], body: None, response: || json!({ "type": "object", "properties": { "policies": { "type": "array", "items": { "type": "object", "properties": { "path": { "type": "string" }, "policy": { "$ref": "#/components/schemas/FolderPolicy" }, "updated_at": { "type": "string" } } } } } })),
    op!("put" "/api/v1/folder-policies", "shares", "Set a folder's own policy (an empty policy removes it; subfolders inherit)", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["path"], "properties": { "path": { "type": "string" }, "policy": { "$ref": "#/components/schemas/FolderPolicy" } } })), response: folder_policy_result),
    op!("get" "/api/v1/folder-policies/effective", "shares", "The policy in force for a folder (own settings merged with its parents')", auth: true, params: &[("path", "query", "string", true, "Folder (or a file in it) under --share-root")], body: None, response: folder_policy_result),
    op!("post" "/api/v1/folder-policies/copy", "shares", "Copy a folder's own policy to another folder", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["from", "to"], "properties": { "from": { "type": "string" }, "to": { "type": "string" } } })), response: folder_policy_result),
    Op { method: "get", path: "/.well-known/oauth-protected-resource", tag: "mcp", summary: "OAuth protected-resource metadata of /mcp (RFC 9728)", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "get", path: "/.well-known/oauth-protected-resource/mcp", tag: "mcp", summary: "Same, at the path-specific location", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "get", path: "/.well-known/oauth-authorization-server", tag: "mcp", summary: "OAuth authorization-server metadata (RFC 8414)", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "post", path: "/oauth2/register", tag: "mcp", summary: "Register a public OAuth client (RFC 7591): redirect_uris (https or loopback http), client_name", auth: false, params: &[], body: Some(any), response: any, content_type: "application/json" },
    Op { method: "get", path: "/oauth2/authorize", tag: "mcp", summary: "Consent page (server password); PKCE S256 required; scopes mcp:read, mcp:write", auth: false, params: &[("client_id", "query", "string", true, ""), ("redirect_uri", "query", "string", true, ""), ("code_challenge", "query", "string", true, "")], body: None, response: html, content_type: "text/html" },
    Op { method: "post", path: "/oauth2/authorize", tag: "mcp", summary: "Submit the consent form; redirects with a code", auth: false, params: &[], body: None, response: html, content_type: "text/html" },
    Op { method: "post", path: "/oauth2/token", tag: "mcp", summary: "authorization_code (with code_verifier) or refresh_token grant (form-encoded); refresh tokens rotate", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "post", path: "/oauth2/revoke", tag: "mcp", summary: "Revoke a refresh or access token (RFC 7009)", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "post", path: "/mcp", tag: "mcp", summary: "MCP over Streamable HTTP (JSON-RPC 2.0; JSON responses); 401 points to the OAuth metadata", auth: false, params: &[], body: Some(any), response: any, content_type: "application/json" },
    Op { method: "get", path: "/mcp", tag: "mcp", summary: "No server-initiated stream (405)", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "delete", path: "/mcp", tag: "mcp", summary: "No sessions to end (405)", auth: false, params: &[], body: None, response: any, content_type: "application/json" },
    Op { method: "get", path: "/s/{token}", tag: "shares", summary: "Public page of a share link (password form, folder listing, upload form)", auth: false, params: &[("token", "path", "string", true, "Link token"), ("p", "query", "string", false, "Folder inside the share")], body: None, response: html, content_type: "text/html" },
    Op { method: "post", path: "/s/{token}/unlock", tag: "shares", summary: "Enter a link's password (form field `password`); sets an unlock cookie", auth: false, params: &[("token", "path", "string", true, "Link token")], body: None, response: html, content_type: "text/html" },
    Op { method: "get", path: "/s/{token}/dl", tag: "shares", summary: "Download a file of a share (counts against its download limit)", auth: false, params: &[("token", "path", "string", true, "Link token"), ("p", "query", "string", false, "File inside the share")], body: None, response: || json!({ "type": "string", "format": "binary" }), content_type: "application/octet-stream" },
    Op { method: "post", path: "/s/{token}/upload", tag: "shares", summary: "Upload files (multipart field `file`, repeatable) into an upload link's folder", auth: false, params: &[("token", "path", "string", true, "Link token"), ("p", "query", "string", false, "Folder inside the share")], body: None, response: html, content_type: "text/html" },
    op!("post" "/api/v1/gateway/poll", "gateways", "Gateway agent: register this node and wait (up to 25 s) for a job; 204 when none (write scope)", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["gateway", "node", "roots"], "properties": { "gateway": { "type": "string" }, "node": { "type": "string" }, "host": { "type": "string" }, "roots": { "type": "array", "items": { "type": "string" } }, "writable": { "type": "boolean" }, "version": { "type": "string" } } })), response: || json!({ "type": "object", "properties": { "id": { "type": "string" }, "op": { "type": "string", "enum": ["list", "download", "upload"] }, "path": { "type": "string" }, "size": { "type": ["integer", "null"] } } })),
    op!("post" "/api/v1/gateway/leave", "gateways", "Gateway agent: leave the pool (sent when the agent stops)", auth: true, params: &[], body: Some(|| json!({ "type": "object", "required": ["gateway", "node"], "properties": { "gateway": { "type": "string" }, "node": { "type": "string" } }, "additionalProperties": true })), response: ok_flag),
    op!("post" "/api/v1/gateway/jobs/{id}/reply", "gateways", "Gateway agent: answer a job (header x-ufop-gateway-status ok|error; the body is streamed to the client)", auth: true, params: &[("id", "path", "string", true, "Job id")], body: None, response: ok_flag),
    Op { method: "get", path: "/api/v1/gateway/jobs/{id}/body", tag: "gateways", summary: "Gateway agent: the data of an upload job", auth: true, params: &[("id", "path", "string", true, "Job id")], body: None, response: || json!({ "type": "string", "format": "binary" }), content_type: "application/octet-stream" },
    op!("get" "/api/v1/gateways", "gateways", "Gateways and their nodes (online, roots, writable, jobs)", auth: true, params: &[], body: None, response: || json!({ "type": "object", "properties": { "gateways": { "type": "array", "items": { "type": "object", "additionalProperties": true } } } })),
    op!("get" "/api/v1/gateways/{name}/files", "gateways", "List a folder through a gateway (`/` = its roots)", auth: true, params: &[("name", "path", "string", true, "Gateway name"), ("path", "query", "string", false, "/<root>/<folder>")], body: None, response: || json!({ "type": "object", "properties": { "path": { "type": "string" }, "entries": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "is_dir": { "type": "boolean" }, "size": { "type": "integer" }, "modified": { "type": ["string", "null"] } } } } } })),
    Op { method: "get", path: "/api/v1/gateways/{name}/download", tag: "gateways", summary: "Download a file through a gateway (streamed, not stored)", auth: true, params: &[("name", "path", "string", true, "Gateway name"), ("path", "query", "string", true, "/<root>/<file>")], body: None, response: || json!({ "type": "string", "format": "binary" }), content_type: "application/octet-stream" },
    op!("put" "/api/v1/gateways/{name}/upload", "gateways", "Upload the request body as a file through a writable gateway", auth: true, params: &[("name", "path", "string", true, "Gateway name"), ("path", "query", "string", true, "/<root>/<file>")], body: None, response: || json!({ "type": "object", "properties": { "path": { "type": "string" }, "bytes": { "type": "integer" } } })),
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
            "Fileset": { "type": "object", "properties": { "id": { "type": "integer" }, "timestamp": { "type": "string", "format": "date-time" }, "volume_name": { "type": "string" }, "partial": { "type": "boolean" }, "file_count": { "type": "integer" }, "total_size": { "type": "integer" }, "source": { "type": "string" }, "note": { "type": ["string", "null"] }, "tags": { "type": "object", "additionalProperties": { "type": "string" } }, "pinned": { "type": "boolean" }, "pin_reason": { "type": ["string", "null"] }, "identity": { "type": ["object", "null"], "properties": { "hostname": { "type": "string" }, "username": { "type": "string" }, "job_id": { "type": "string" }, "source_label": { "type": "string" } } }, "stats": { "type": ["object", "null"], "properties": { "bytes_added": { "type": "integer" }, "new_blocks": { "type": "integer" }, "files_added": { "type": "integer" }, "files_changed": { "type": "integer" }, "files_deleted": { "type": "integer" } } } } },
            "RetentionPolicy": { "type": "object", "required": ["type"], "properties": { "type": { "type": "string", "enum": ["keep_all", "keep_days", "keep_count", "smart", "custom", "counts"] }, "days": { "type": "integer" }, "count": { "type": "integer" }, "rules": { "type": "string" }, "keep_latest": { "type": "integer" }, "keep_hourly": { "type": "integer" }, "keep_daily": { "type": "integer" }, "keep_weekly": { "type": "integer" }, "keep_monthly": { "type": "integer" }, "keep_yearly": { "type": "integer" } } },
            "BackupJob": { "type": "object", "description": "A backup job (sources, destination, schedule, retention, filters, encryption); see docs/api-reference.md", "additionalProperties": true },
            "TaskInfo": { "type": "object", "properties": { "task_id": { "type": "string" }, "job_id": { "type": "string" }, "job_name": { "type": "string" }, "operation": { "type": "string" }, "state": { "type": "string", "enum": ["queued", "running"] }, "queued_at": { "type": "string" }, "started_at": { "type": ["string", "null"] }, "elapsed_secs": { "type": "integer" } }, "additionalProperties": true },
            "FolderPolicy": { "type": "object", "description": "null fields inherit from the parent folder", "properties": { "locked": { "type": ["boolean", "null"] }, "allowed_extensions": { "type": ["array", "null"], "items": { "type": "string" } }, "blocked_extensions": { "type": ["array", "null"], "items": { "type": "string" } }, "max_upload_mb": { "type": ["integer", "null"] }, "require_link_password": { "type": ["boolean", "null"] }, "max_link_days": { "type": ["integer", "null"] }, "intake_naming": { "type": ["string", "null"], "enum": ["keep", "timestamp", "sequence", null] } } },
            "Share": { "type": "object", "properties": { "id": { "type": "string" }, "name": { "type": "string" }, "path": { "type": "string" }, "kind": { "type": "string", "enum": ["download", "upload", "both"] }, "token_prefix": { "type": "string" }, "has_password": { "type": "boolean" }, "expires_at": { "type": ["string", "null"] }, "max_downloads": { "type": ["integer", "null"] }, "downloads": { "type": "integer" }, "max_upload_bytes": { "type": ["integer", "null"] }, "created_at": { "type": "string" }, "revoked_at": { "type": ["string", "null"] }, "last_access_at": { "type": ["string", "null"] } } },
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
            { "name": "general" }, { "name": "auth" }, { "name": "jobs" }, { "name": "browse" }, { "name": "tasks" }, { "name": "shares" }, { "name": "gateways" }, { "name": "oauth" }, { "name": "mcp" }
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
        for src in [include_str!("mod.rs"), include_str!("oauth.rs"), include_str!("shares.rs"), include_str!("folders.rs"), include_str!("gateway.rs"), include_str!("mcp.rs"), include_str!("mcp_oauth.rs")] {
            for chunk in src.split(".route(\"").skip(1) {
                let Some((path, rest)) = chunk.split_once('"') else { continue };
                let handlers = &rest[..rest.find(".route(").unwrap_or(rest.len()).min(rest.find('\n').unwrap_or(rest.len()))];
                for m in ["get", "post", "put", "patch", "delete"] {
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
