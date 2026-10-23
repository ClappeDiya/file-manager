# Generated from the ufop-server OpenAPI document by src-tauri/src/server/sdk.rs. Do not edit.
# Regenerate: UPDATE_SDK=1 cargo test --lib sdk
"""Client for the ufop-server REST API."""

import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, List, Optional, Union

API_VERSION = "0.1.0"
ApiToken = Dict[str, Any]
BackupJob = Dict[str, Any]
ApiError = Dict[str, Any]
Fileset = Dict[str, Any]
FolderPolicy = Dict[str, Any]
RetentionPolicy = Dict[str, Any]
SchedulerStatus = Dict[str, Any]
Scope = Dict[str, Any]
Session = Dict[str, Any]
Share = Dict[str, Any]
TaskInfo = Dict[str, Any]


class UfopError(Exception):
    """An error response: `{error_type, message, advice}` and the HTTP status."""

    def __init__(self, status: int, message: str, error_type: Optional[str] = None, advice: Optional[str] = None):
        super().__init__(message)
        self.status = status
        self.error_type = error_type
        self.advice = advice


class UfopClient:
    """ufop-server REST client (standard library only).

    `token` is an access token (from `login`), an automation token or an API
    token (`ufop_pat_…`). `headers` adds headers to every request.
    """

    def __init__(self, base_url: str, token: Optional[str] = None, timeout: float = 30.0, headers: Optional[Dict[str, str]] = None):
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout
        self.headers = dict(headers or {})

    def login(self, password: str) -> Dict[str, Any]:
        """Sign in with the server password and use the access token from then on."""
        session = self.post_auth_login({"password": password})
        self.token = session["access_token"]
        return session

    def _call(self, method: str, path: str, query: Optional[Dict[str, Any]] = None, body: Any = None) -> Any:
        url = self.base_url + path
        if query:
            q = {k: ("true" if v is True else "false" if v is False else v) for k, v in query.items() if v is not None}
            if q:
                url += "?" + urllib.parse.urlencode(q)
        headers = {"Accept": "application/json", **self.headers}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        req = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as res:
                text = res.read().decode()
                return json.loads(text) if text else None
        except urllib.error.HTTPError as e:
            text = e.read().decode(errors="replace")
            try:
                err = json.loads(text)
            except ValueError:
                err = {}
            raise UfopError(e.code, err.get("message") or e.reason, err.get("error_type"), err.get("advice")) from None

    def get_auth_api_tokens(self) -> Dict[str, Any]:
        """List API tokens (no secrets) (API tokens: `admin` scope). `GET /api/v1/auth/api-tokens`"""
        return self._call("GET", f"/api/v1/auth/api-tokens", None, None)

    def post_auth_api_tokens(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Create a scoped API token (interactive sessions only); the secret is returned once (API tokens: `admin` scope). `POST /api/v1/auth/api-tokens`"""
        return self._call("POST", f"/api/v1/auth/api-tokens", None, body)

    def delete_auth_api_tokens_id(self, id_: str) -> Dict[str, Any]:
        """Revoke an API token (API tokens: `admin` scope). `DELETE /api/v1/auth/api-tokens/{id}`"""
        return self._call("DELETE", f"/api/v1/auth/api-tokens/{urllib.parse.quote(id_, safe='')}", None, None)

    def post_auth_automation_token(self) -> Dict[str, Any]:
        """Issue a long-lived automation token (when allowed in settings) (API tokens: `admin` scope). `POST /api/v1/auth/automation-token`"""
        return self._call("POST", f"/api/v1/auth/automation-token", None, None)

    def post_auth_login(self, body: Dict[str, Any]) -> "Session":
        """Sign in with the server password; sets the refresh cookie. `POST /api/v1/auth/login`"""
        return self._call("POST", f"/api/v1/auth/login", None, body)

    def post_auth_logout(self) -> Dict[str, Any]:
        """Revoke the refresh cookie's session. `POST /api/v1/auth/logout`"""
        return self._call("POST", f"/api/v1/auth/logout", None, None)

    def put_auth_password(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Change the server password (API tokens: `admin` scope). `PUT /api/v1/auth/password`"""
        return self._call("PUT", f"/api/v1/auth/password", None, body)

    def post_auth_refresh(self) -> "Session":
        """New access token from the refresh cookie (rotated). `POST /api/v1/auth/refresh`"""
        return self._call("POST", f"/api/v1/auth/refresh", None, None)

    def post_auth_reset(self) -> Dict[str, Any]:
        """Rotate the signing key: every token and session ends (API tokens: `admin` scope). `POST /api/v1/auth/reset`"""
        return self._call("POST", f"/api/v1/auth/reset", None, None)

    def post_auth_signin(self, body: Dict[str, Any]) -> "Session":
        """Redeem a one-time sign-in token. `POST /api/v1/auth/signin`"""
        return self._call("POST", f"/api/v1/auth/signin", None, body)

    def post_auth_signin_token(self) -> Dict[str, Any]:
        """Issue a one-time sign-in token (link for another browser) (API tokens: `admin` scope). `POST /api/v1/auth/signin-token`"""
        return self._call("POST", f"/api/v1/auth/signin-token", None, None)

    def get_backup_cache(self) -> Dict[str, Any]:
        """Local volume cache: folder, size limit, entries, bytes, hits, misses, hit rate (API tokens: `read` scope). `GET /api/v1/backup-cache`"""
        return self._call("GET", f"/api/v1/backup-cache", None, None)

    def delete_backup_cache(self) -> Dict[str, Any]:
        """Delete every cached volume (API tokens: `write` scope). `DELETE /api/v1/backup-cache`"""
        return self._call("DELETE", f"/api/v1/backup-cache", None, None)

    def get_backup_cache_settings(self) -> Dict[str, Any]:
        """Volume cache settings (API tokens: `read` scope). `GET /api/v1/backup-cache/settings`"""
        return self._call("GET", f"/api/v1/backup-cache/settings", None, None)

    def put_backup_cache_settings(self, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Change the volume cache folder, size limit (0 = off) and use for full restores (API tokens: `write` scope). `PUT /api/v1/backup-cache/settings`"""
        return self._call("PUT", f"/api/v1/backup-cache/settings", None, body if body is not None else {})

    def post_backup_schedule_preview(self, body: Dict[str, Any]) -> List[Dict[str, Any]]:
        """Validate a schedule (interval or cron) and list its next runs, marking holiday skips (API tokens: `read` scope). `POST /api/v1/backup-schedule/preview`"""
        return self._call("POST", f"/api/v1/backup-schedule/preview", None, body)

    def get_backup_templates(self) -> List[Dict[str, Any]]:
        """Policy templates (local, then the organization's) (API tokens: `read` scope). `GET /api/v1/backup-templates`"""
        return self._call("GET", f"/api/v1/backup-templates", None, None)

    def post_backup_templates(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Create a policy template (API tokens: `write` scope). `POST /api/v1/backup-templates`"""
        return self._call("POST", f"/api/v1/backup-templates", None, body)

    def get_backup_templates_export(self, ids: Optional[str] = None) -> Dict[str, Any]:
        """Templates as a JSON file (API tokens: `read` scope). `GET /api/v1/backup-templates/export`"""
        return self._call("GET", f"/api/v1/backup-templates/export", {"ids": ids}, None)

    def post_backup_templates_import(self, body: Optional[Dict[str, Any]] = None) -> List[Dict[str, Any]]:
        """Import a template file (same id or name updates) (API tokens: `write` scope). `POST /api/v1/backup-templates/import`"""
        return self._call("POST", f"/api/v1/backup-templates/import", None, body if body is not None else {})

    def get_backup_templates_id(self, id_: str) -> Dict[str, Any]:
        """One policy template (API tokens: `read` scope). `GET /api/v1/backup-templates/{id}`"""
        return self._call("GET", f"/api/v1/backup-templates/{urllib.parse.quote(id_, safe='')}", None, None)

    def put_backup_templates_id(self, id_: str, body: Dict[str, Any]) -> Dict[str, Any]:
        """Update a local policy template (API tokens: `write` scope). `PUT /api/v1/backup-templates/{id}`"""
        return self._call("PUT", f"/api/v1/backup-templates/{urllib.parse.quote(id_, safe='')}", None, body)

    def delete_backup_templates_id(self, id_: str) -> Dict[str, Any]:
        """Delete a local policy template no job uses (API tokens: `write` scope). `DELETE /api/v1/backup-templates/{id}`"""
        return self._call("DELETE", f"/api/v1/backup-templates/{urllib.parse.quote(id_, safe='')}", None, None)

    def post_backup_templates_id_apply(self, id_: str, body: Dict[str, Any]) -> "BackupJob":
        """Base a job on this template too (appended: it wins over earlier ones) (API tokens: `write` scope). `POST /api/v1/backup-templates/{id}/apply`"""
        return self._call("POST", f"/api/v1/backup-templates/{urllib.parse.quote(id_, safe='')}/apply", None, body)

    def post_destinations_test(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Test a destination (optionally create its folder); returns the files found (API tokens: `write` scope). `POST /api/v1/destinations/test`"""
        return self._call("POST", f"/api/v1/destinations/test", None, body)

    def get_filesystem(self, path: Optional[str] = None, folders_only: Optional[bool] = None) -> Dict[str, Any]:
        """Browse the server's file system (source picker) (API tokens: `read` scope). `GET /api/v1/filesystem`"""
        return self._call("GET", f"/api/v1/filesystem", {"path": path, "folders_only": folders_only}, None)

    def get_folder_policies(self) -> Dict[str, Any]:
        """Folder policies set on shared folders (API tokens: `read` scope). `GET /api/v1/folder-policies`"""
        return self._call("GET", f"/api/v1/folder-policies", None, None)

    def put_folder_policies(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Set a folder's own policy (an empty policy removes it; subfolders inherit) (API tokens: `write` scope). `PUT /api/v1/folder-policies`"""
        return self._call("PUT", f"/api/v1/folder-policies", None, body)

    def post_folder_policies_copy(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Copy a folder's own policy to another folder (API tokens: `write` scope). `POST /api/v1/folder-policies/copy`"""
        return self._call("POST", f"/api/v1/folder-policies/copy", None, body)

    def get_folder_policies_effective(self, path: str) -> Dict[str, Any]:
        """The policy in force for a folder (own settings merged with its parents') (API tokens: `read` scope). `GET /api/v1/folder-policies/effective`"""
        return self._call("GET", f"/api/v1/folder-policies/effective", {"path": path}, None)

    def post_gateway_jobs_id_reply(self, id_: str) -> Dict[str, Any]:
        """Gateway agent: answer a job (header x-ufop-gateway-status ok|error; the body is streamed to the client) (API tokens: `write` scope). `POST /api/v1/gateway/jobs/{id}/reply`"""
        return self._call("POST", f"/api/v1/gateway/jobs/{urllib.parse.quote(id_, safe='')}/reply", None, None)

    def post_gateway_leave(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Gateway agent: leave the pool (sent when the agent stops) (API tokens: `write` scope). `POST /api/v1/gateway/leave`"""
        return self._call("POST", f"/api/v1/gateway/leave", None, body)

    def post_gateway_poll(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Gateway agent: register this node and wait (up to 25 s) for a job; 204 when none (write scope) (API tokens: `write` scope). `POST /api/v1/gateway/poll`"""
        return self._call("POST", f"/api/v1/gateway/poll", None, body)

    def get_gateways(self) -> Dict[str, Any]:
        """Gateways and their nodes (online, roots, writable, jobs) (API tokens: `read` scope). `GET /api/v1/gateways`"""
        return self._call("GET", f"/api/v1/gateways", None, None)

    def get_gateways_name_files(self, name: str, path: Optional[str] = None) -> Dict[str, Any]:
        """List a folder through a gateway (`/` = its roots) (API tokens: `read` scope). `GET /api/v1/gateways/{name}/files`"""
        return self._call("GET", f"/api/v1/gateways/{urllib.parse.quote(name, safe='')}/files", {"path": path}, None)

    def put_gateways_name_upload(self, name: str, path: str) -> Dict[str, Any]:
        """Upload the request body as a file through a writable gateway (API tokens: `write` scope). `PUT /api/v1/gateways/{name}/upload`"""
        return self._call("PUT", f"/api/v1/gateways/{urllib.parse.quote(name, safe='')}/upload", {"path": path}, None)

    def get_health(self) -> Dict[str, Any]:
        """Liveness, version and scheduler summary. `GET /api/v1/health`"""
        return self._call("GET", f"/api/v1/health", None, None)

    def get_jobs(self) -> List["BackupJob"]:
        """List backup jobs (API tokens: `read` scope). `GET /api/v1/jobs`"""
        return self._call("GET", f"/api/v1/jobs", None, None)

    def post_jobs(self, body: Dict[str, Any]) -> "BackupJob":
        """Create a job (API tokens: `write` scope). `POST /api/v1/jobs`"""
        return self._call("POST", f"/api/v1/jobs", None, body)

    def post_jobs_import(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Import an exported job configuration (API tokens: `write` scope). `POST /api/v1/jobs/import`"""
        return self._call("POST", f"/api/v1/jobs/import", None, body)

    def post_jobs_run_all(self, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Back up every job now (queued; read-only and busy jobs are skipped) (API tokens: `run` scope). `POST /api/v1/jobs/run-all`"""
        return self._call("POST", f"/api/v1/jobs/run-all", None, body if body is not None else {})

    def get_jobs_id(self, id_: str) -> "BackupJob":
        """Get a job (API tokens: `read` scope). `GET /api/v1/jobs/{id}`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}", None, None)

    def put_jobs_id(self, id_: str, body: Dict[str, Any]) -> "BackupJob":
        """Replace a job (API tokens: `write` scope). `PUT /api/v1/jobs/{id}`"""
        return self._call("PUT", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}", None, body)

    def delete_jobs_id(self, id_: str, delete_local_db: Optional[bool] = None, delete_remote: Optional[bool] = None) -> Dict[str, Any]:
        """Delete a job (API tokens: `write` scope). `DELETE /api/v1/jobs/{id}`"""
        return self._call("DELETE", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}", {"delete_local_db": delete_local_db, "delete_remote": delete_remote}, None)

    def get_jobs_id_cli(self, id_: str) -> Dict[str, Any]:
        """The equivalent `ufop backup` command line (API tokens: `read` scope). `GET /api/v1/jobs/{id}/cli`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/cli", None, None)

    def get_jobs_id_compare(self, id_: str, base: int, compare: int, source: Optional[str] = None) -> Dict[str, Any]:
        """Differences between two versions (API tokens: `read` scope). `GET /api/v1/jobs/{id}/compare`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/compare", {"base": base, "compare": compare, "source": source}, None)

    def put_jobs_id_database_encryption(self, id_: str, body: Dict[str, Any]) -> Dict[str, Any]:
        """Encrypt or decrypt the job's local database now (key kept in the credential store) (API tokens: `write` scope). `PUT /api/v1/jobs/{id}/database-encryption`"""
        return self._call("PUT", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/database-encryption", None, body)

    def get_jobs_id_effective_settings(self, id_: str) -> Dict[str, Any]:
        """Each setting the job runs with and where it comes from (built_in, global, template, job, managed) (API tokens: `read` scope). `GET /api/v1/jobs/{id}/effective-settings`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/effective-settings", None, None)

    def get_jobs_id_estimate(self, id_: str, assumed_bytes_per_sec: Optional[int] = None) -> Dict[str, Any]:
        """Estimate the next backup: files, new data (at most / at least) and duration; reads nothing (API tokens: `read` scope). `GET /api/v1/jobs/{id}/estimate`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/estimate", {"assumed_bytes_per_sec": assumed_bytes_per_sec}, None)

    def get_jobs_id_export(self, id_: str, include_passphrase: Optional[bool] = None, export_passphrase: Optional[str] = None) -> Dict[str, Any]:
        """Export the job configuration (API tokens: `read` scope). `GET /api/v1/jobs/{id}/export`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/export", {"include_passphrase": include_passphrase, "export_passphrase": export_passphrase}, None)

    def get_jobs_id_files(self, id_: str, version: Optional[str] = None, time: Optional[str] = None, allow_partial: Optional[bool] = None, prefix: Optional[str] = None, source: Optional[str] = None) -> Dict[str, Any]:
        """Files in a version (API tokens: `read` scope). `GET /api/v1/jobs/{id}/files`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/files", {"version": version, "time": time, "allow_partial": allow_partial, "prefix": prefix, "source": source}, None)

    def get_jobs_id_lock(self, id_: str) -> Dict[str, Any]:
        """Repository lock status: who holds the destination (host, operation, since, expires) (API tokens: `read` scope). `GET /api/v1/jobs/{id}/lock`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/lock", None, None)

    def delete_jobs_id_lock(self, id_: str, confirm: bool, owner_id: Optional[str] = None) -> Dict[str, Any]:
        """Break the repository lock (only when its holder is no longer running) (API tokens: `write` scope). `DELETE /api/v1/jobs/{id}/lock`"""
        return self._call("DELETE", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/lock", {"confirm": confirm, "owner_id": owner_id}, None)

    def post_jobs_id_operations(self, id_: str, body: Dict[str, Any]) -> Dict[str, Any]:
        """Restore, verify, compact, repair, purge … (a tagged BackupOperation; not `backup`). Verify takes `options`: scope {versions, paths}, mode {type: structure | sample_volumes(count) | sample_percent(percent) | full}, files_percent, parallelism, max_errors, use_cache (API tokens: `run` scope). `POST /api/v1/jobs/{id}/operations`"""
        return self._call("POST", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/operations", None, body)

    def put_jobs_id_passphrase(self, id_: str, body: Dict[str, Any]) -> Dict[str, Any]:
        """Change an encrypted job's passphrase (format 2 repositories rewrite only the header) (API tokens: `write` scope). `PUT /api/v1/jobs/{id}/passphrase`"""
        return self._call("PUT", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/passphrase", None, body)

    def post_jobs_id_point_in_time(self, id_: str, body: Dict[str, Any]) -> "BackupJob":
        """Open the job's versioned S3 destination as it was at a time: creates a read-only job whose index is rebuilt from the object versions of then (API tokens: `write` scope). `POST /api/v1/jobs/{id}/point-in-time`"""
        return self._call("POST", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/point-in-time", None, body)

    def post_jobs_id_retention_preview(self, id_: str, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """What retention would keep and delete (the job's policy or an unsaved one); changes nothing (API tokens: `write` scope). `POST /api/v1/jobs/{id}/retention/preview`"""
        return self._call("POST", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/retention/preview", None, body if body is not None else {})

    def post_jobs_id_run(self, id_: str, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Start a backup (queued); returns the task id (API tokens: `run` scope). `POST /api/v1/jobs/{id}/run`"""
        return self._call("POST", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/run", None, body if body is not None else {})

    def get_jobs_id_search(self, id_: str, q: str, version: Optional[int] = None, source: Optional[str] = None) -> Dict[str, Any]:
        """Search file names across versions (API tokens: `read` scope). `GET /api/v1/jobs/{id}/search`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/search", {"q": q, "version": version, "source": source}, None)

    def get_jobs_id_sources(self, id_: str) -> List[Dict[str, Any]]:
        """Sources (jobs and computers) with versions in the job's repository (API tokens: `read` scope). `GET /api/v1/jobs/{id}/sources`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/sources", None, None)

    def get_jobs_id_versions(self, id_: str, source: Optional[str] = None, tags: Optional[str] = None) -> List["Fileset"]:
        """Backup versions of a job (with note, tags, pin, identity and storage added) (API tokens: `read` scope). `GET /api/v1/jobs/{id}/versions`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/versions", {"source": source, "tags": tags}, None)

    def get_jobs_id_versions_resolve(self, id_: str, version: Optional[str] = None, time: Optional[str] = None, allow_partial: Optional[bool] = None, source: Optional[str] = None) -> Dict[str, Any]:
        """Which version a time (or latest / oldest / number / id) picks (API tokens: `read` scope). `GET /api/v1/jobs/{id}/versions/resolve`"""
        return self._call("GET", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/versions/resolve", {"version": version, "time": time, "allow_partial": allow_partial, "source": source}, None)

    def patch_jobs_id_versions_version(self, id_: str, version: str, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Change a version's note, tags or pin (rewrites its file list) (API tokens: `write` scope). `PATCH /api/v1/jobs/{id}/versions/{version}`"""
        return self._call("PATCH", f"/api/v1/jobs/{urllib.parse.quote(id_, safe='')}/versions/{urllib.parse.quote(version, safe='')}", None, body if body is not None else {})

    def get_modules(self) -> Dict[str, Any]:
        """Enabled and disabled modules (API tokens: `read` scope). `GET /api/v1/modules`"""
        return self._call("GET", f"/api/v1/modules", None, None)

    def get_oauth_providers(self) -> List[Dict[str, Any]]:
        """Configured OAuth providers for cloud destinations (API tokens: `admin` scope). `GET /api/v1/oauth/providers`"""
        return self._call("GET", f"/api/v1/oauth/providers", None, None)

    def get_openapi_json(self) -> Dict[str, Any]:
        """This document. `GET /api/v1/openapi.json`"""
        return self._call("GET", f"/api/v1/openapi.json", None, None)

    def post_rpc_command(self, command: str, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Call a desktop IPC command by name (arguments as the JSON body); see docs/ipc-commands.md (API tokens: `admin` scope). `POST /api/v1/rpc/{command}`"""
        return self._call("POST", f"/api/v1/rpc/{urllib.parse.quote(command, safe='')}", None, body if body is not None else {})

    def get_runs(self, job_id: Optional[str] = None, limit: Optional[int] = None) -> List[Dict[str, Any]]:
        """Run history (API tokens: `read` scope). `GET /api/v1/runs`"""
        return self._call("GET", f"/api/v1/runs", {"job_id": job_id, "limit": limit}, None)

    def post_scheduler_pause(self, body: Optional[Dict[str, Any]] = None) -> "SchedulerStatus":
        """Pause the scheduler (optionally for N minutes) (API tokens: `run` scope). `POST /api/v1/scheduler/pause`"""
        return self._call("POST", f"/api/v1/scheduler/pause", None, body if body is not None else {})

    def post_scheduler_resume(self) -> "SchedulerStatus":
        """Resume the scheduler (API tokens: `run` scope). `POST /api/v1/scheduler/resume`"""
        return self._call("POST", f"/api/v1/scheduler/resume", None, None)

    def get_settings(self) -> Dict[str, Any]:
        """Server settings (API tokens: `read` scope). `GET /api/v1/settings`"""
        return self._call("GET", f"/api/v1/settings", None, None)

    def put_settings(self, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Change server settings (API tokens: `admin` scope). `PUT /api/v1/settings`"""
        return self._call("PUT", f"/api/v1/settings", None, body if body is not None else {})

    def get_shares(self) -> Dict[str, Any]:
        """Share links (tokens are never returned) and the folders that may be shared (API tokens: `read` scope). `GET /api/v1/shares`"""
        return self._call("GET", f"/api/v1/shares", None, None)

    def post_shares(self, body: Dict[str, Any]) -> Dict[str, Any]:
        """Create a share link to a file or folder under --share-root; the link is returned once (API tokens: `write` scope). `POST /api/v1/shares`"""
        return self._call("POST", f"/api/v1/shares", None, body)

    def delete_shares_id(self, id_: str) -> Dict[str, Any]:
        """Turn a share link off (API tokens: `write` scope). `DELETE /api/v1/shares/{id}`"""
        return self._call("DELETE", f"/api/v1/shares/{urllib.parse.quote(id_, safe='')}", None, None)

    def get_shares_id_events(self, id_: str, limit: Optional[int] = None) -> Dict[str, Any]:
        """Access log of a link (views, downloads, uploads, refusals, wrong passwords) (API tokens: `read` scope). `GET /api/v1/shares/{id}/events`"""
        return self._call("GET", f"/api/v1/shares/{urllib.parse.quote(id_, safe='')}/events", {"limit": limit}, None)

    def get_status(self) -> "SchedulerStatus":
        """Scheduler state and tasks (API tokens: `read` scope). `GET /api/v1/status`"""
        return self._call("GET", f"/api/v1/status", None, None)

    def get_tasks(self) -> List["TaskInfo"]:
        """Queued and running tasks (API tokens: `read` scope). `GET /api/v1/tasks`"""
        return self._call("GET", f"/api/v1/tasks", None, None)

    def post_tasks_id_cancel(self, id_: str) -> Dict[str, Any]:
        """Cancel a task (API tokens: `run` scope). `POST /api/v1/tasks/{id}/cancel`"""
        return self._call("POST", f"/api/v1/tasks/{urllib.parse.quote(id_, safe='')}/cancel", None, None)
