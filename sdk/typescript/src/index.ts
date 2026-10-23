// Generated from the ufop-server OpenAPI document by src-tauri/src/server/sdk.rs. Do not edit.
// Regenerate: UPDATE_SDK=1 cargo test --lib sdk

/** API version this client was generated from. */
export const API_VERSION = "0.1.0";

export type ApiToken = { created_at?: string; expires_at?: string; id?: string; last_used_at?: string | null; name?: string; scope?: Scope; [key: string]: unknown };

/** A backup job (sources, destination, schedule, retention, filters, encryption); see docs/api-reference.md */
export type BackupJob = Record<string, unknown>;

export type ApiError = { advice?: string; error_type?: string; message: string };

export type Fileset = { file_count?: number; id?: number; identity?: { hostname?: string; job_id?: string; source_label?: string; username?: string } | null; note?: string | null; partial?: boolean; pin_reason?: string | null; pinned?: boolean; source?: string; stats?: { bytes_added?: number; files_added?: number; files_changed?: number; files_deleted?: number; new_blocks?: number } | null; tags?: Record<string, unknown>; timestamp?: string; total_size?: number; volume_name?: string };

/** null fields inherit from the parent folder */
export type FolderPolicy = { allowed_extensions?: string[] | null; blocked_extensions?: string[] | null; intake_naming?: "keep" | "timestamp" | "sequence" | null; locked?: boolean | null; max_link_days?: number | null; max_upload_mb?: number | null; require_link_password?: boolean | null };

export type RetentionPolicy = { count?: number; days?: number; keep_daily?: number; keep_hourly?: number; keep_latest?: number; keep_monthly?: number; keep_weekly?: number; keep_yearly?: number; rules?: string; type: "keep_all" | "keep_days" | "keep_count" | "smart" | "custom" | "counts" };

export type SchedulerStatus = { paused?: boolean; running?: boolean; tasks?: TaskInfo[]; [key: string]: unknown };

/** read < run < write < admin */
export type Scope = "read" | "run" | "write" | "admin";

export type Session = { access_token?: string; csrf_token?: string; expires_at?: string };

export type Share = { created_at?: string; downloads?: number; expires_at?: string | null; has_password?: boolean; id?: string; kind?: "download" | "upload" | "both"; last_access_at?: string | null; max_downloads?: number | null; max_upload_bytes?: number | null; name?: string; path?: string; revoked_at?: string | null; token_prefix?: string };

export type TaskInfo = { elapsed_secs?: number; job_id?: string; job_name?: string; operation?: string; queued_at?: string; started_at?: string | null; state?: "queued" | "running"; task_id?: string; [key: string]: unknown };

/** An error response: `{error_type, message, advice}` and the HTTP status. */
export class UfopError extends Error {
  readonly status: number;
  readonly errorType: string | undefined;
  readonly advice: string | undefined;

  constructor(status: number, message: string, errorType?: string, advice?: string) {
    super(message);
    this.name = "UfopError";
    this.status = status;
    this.errorType = errorType;
    this.advice = advice;
  }
}

export interface ClientOptions {
  /** Access token (from login), automation token or API token (`ufop_pat_…`). */
  token?: string;
  /** A fetch implementation (default: the global one). */
  fetch?: typeof fetch;
  /** Extra headers, e.g. a reverse-proxy pre-authentication header. */
  headers?: Record<string, string>;
}

export class UfopClient {
  readonly baseUrl: string;
  private token: string | undefined;
  private readonly options: ClientOptions;
  private readonly fetchImpl: typeof fetch;

  constructor(baseUrl: string, options: ClientOptions = {}) {
    this.baseUrl = baseUrl;
    this.options = options;
    this.token = options.token;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  setToken(token: string | undefined): void {
    this.token = token;
  }

  /** Sign in with the server password and use the access token from then on. */
  async login(password: string): Promise<Session> {
    const session = await this.postAuthLogin({ password });
    this.token = session.access_token;
    return session;
  }

  private async call<T>(method: string, path: string, query?: Record<string, unknown>, body?: unknown): Promise<T> {
    const url = new URL(path, this.baseUrl.endsWith("/") ? this.baseUrl : `${this.baseUrl}/`);
    for (const [k, v] of Object.entries(query ?? {})) if (v !== undefined && v !== null) url.searchParams.set(k, String(v));
    const headers: Record<string, string> = { Accept: "application/json", ...this.options.headers };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const res = await this.fetchImpl(url.toString(), { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
    const text = await res.text();
    const data = text ? JSON.parse(text) : undefined;
    if (!res.ok) throw new UfopError(res.status, data?.message ?? res.statusText, data?.error_type, data?.advice);
    return data as T;
  }

  /** List API tokens (no secrets) (API tokens: `admin` scope). `GET /api/v1/auth/api-tokens` */
  getAuthApiTokens(): Promise<{ tokens?: ApiToken[] }> {
    return this.call("GET", `/api/v1/auth/api-tokens`, undefined, undefined);
  }

  /** Create a scoped API token (interactive sessions only); the secret is returned once (API tokens: `admin` scope). `POST /api/v1/auth/api-tokens` */
  postAuthApiTokens(body: { expires_in_days?: number; name: string; scope: Scope }): Promise<{ info?: ApiToken; token?: string }> {
    return this.call("POST", `/api/v1/auth/api-tokens`, undefined, body);
  }

  /** Revoke an API token (API tokens: `admin` scope). `DELETE /api/v1/auth/api-tokens/{id}` */
  deleteAuthApiTokensId(id: string): Promise<{ ok?: boolean }> {
    return this.call("DELETE", `/api/v1/auth/api-tokens/${encodeURIComponent(id)}`, undefined, undefined);
  }

  /** Issue a long-lived automation token (when allowed in settings) (API tokens: `admin` scope). `POST /api/v1/auth/automation-token` */
  postAuthAutomationToken(): Promise<Record<string, unknown>> {
    return this.call("POST", `/api/v1/auth/automation-token`, undefined, undefined);
  }

  /** Sign in with the server password; sets the refresh cookie. `POST /api/v1/auth/login` */
  postAuthLogin(body: { password: string }): Promise<Session> {
    return this.call("POST", `/api/v1/auth/login`, undefined, body);
  }

  /** Revoke the refresh cookie's session; with the cookie it needs X-CSRF-Token (403 otherwise). `POST /api/v1/auth/logout` */
  postAuthLogout(): Promise<{ ok?: boolean }> {
    return this.call("POST", `/api/v1/auth/logout`, undefined, undefined);
  }

  /** Change the server password (API tokens: `admin` scope). `PUT /api/v1/auth/password` */
  putAuthPassword(body: { current_password: string; new_password: string }): Promise<{ ok?: boolean }> {
    return this.call("PUT", `/api/v1/auth/password`, undefined, body);
  }

  /** New access token from the refresh cookie (rotated); needs X-CSRF-Token equal to the ufop_csrf cookie (403 otherwise). `POST /api/v1/auth/refresh` */
  postAuthRefresh(): Promise<Session> {
    return this.call("POST", `/api/v1/auth/refresh`, undefined, undefined);
  }

  /** Rotate the signing key: every token and session ends (API tokens: `admin` scope). `POST /api/v1/auth/reset` */
  postAuthReset(): Promise<{ ok?: boolean }> {
    return this.call("POST", `/api/v1/auth/reset`, undefined, undefined);
  }

  /** Redeem a one-time sign-in token. `POST /api/v1/auth/signin` */
  postAuthSignin(body: { token: string }): Promise<Session> {
    return this.call("POST", `/api/v1/auth/signin`, undefined, body);
  }

  /** Issue a one-time sign-in token (link for another browser) (API tokens: `admin` scope). `POST /api/v1/auth/signin-token` */
  postAuthSigninToken(): Promise<Record<string, unknown>> {
    return this.call("POST", `/api/v1/auth/signin-token`, undefined, undefined);
  }

  /** Local volume cache: folder, size limit, entries, bytes, hits, misses, hit rate (API tokens: `read` scope). `GET /api/v1/backup-cache` */
  getBackupCache(): Promise<{ bytes?: number; corrupt?: number; dir?: string; entries?: number; evictions?: number; hit_rate?: number; hits?: number; max_bytes?: number; misses?: number }> {
    return this.call("GET", `/api/v1/backup-cache`, undefined, undefined);
  }

  /** Delete every cached volume (API tokens: `write` scope). `DELETE /api/v1/backup-cache` */
  deleteBackupCache(): Promise<{ bytes_freed?: number }> {
    return this.call("DELETE", `/api/v1/backup-cache`, undefined, undefined);
  }

  /** Volume cache settings (API tokens: `read` scope). `GET /api/v1/backup-cache/settings` */
  getBackupCacheSettings(): Promise<{ dir?: string | null; max_bytes?: number; use_for_restore?: boolean }> {
    return this.call("GET", `/api/v1/backup-cache/settings`, undefined, undefined);
  }

  /** Change the volume cache folder, size limit (0 = off) and use for full restores (API tokens: `write` scope). `PUT /api/v1/backup-cache/settings` */
  putBackupCacheSettings(body: { dir?: string | null; max_bytes?: number; use_for_restore?: boolean } = {}): Promise<{ bytes?: number; corrupt?: number; dir?: string; entries?: number; evictions?: number; hit_rate?: number; hits?: number; max_bytes?: number; misses?: number }> {
    return this.call("PUT", `/api/v1/backup-cache/settings`, undefined, body);
  }

  /** Validate a schedule (interval or cron) and list its next runs, marking holiday skips (API tokens: `read` scope). `POST /api/v1/backup-schedule/preview` */
  postBackupSchedulePreview(body: { count?: number; schedule: Record<string, unknown> }): Promise<Array<{ at?: string; skipped_by?: string | null }>> {
    return this.call("POST", `/api/v1/backup-schedule/preview`, undefined, body);
  }

  /** Policy templates (local, then the organization's) (API tokens: `read` scope). `GET /api/v1/backup-templates` */
  getBackupTemplates(): Promise<Array<{ description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }>> {
    return this.call("GET", `/api/v1/backup-templates`, undefined, undefined);
  }

  /** Create a policy template (API tokens: `write` scope). `POST /api/v1/backup-templates` */
  postBackupTemplates(body: { description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }): Promise<{ description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }> {
    return this.call("POST", `/api/v1/backup-templates`, undefined, body);
  }

  /** Templates as a JSON file (API tokens: `read` scope). `GET /api/v1/backup-templates/export` */
  getBackupTemplatesExport(query: { ids?: string } = {}): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/backup-templates/export`, query, undefined);
  }

  /** Import a template file (same id or name updates) (API tokens: `write` scope). `POST /api/v1/backup-templates/import` */
  postBackupTemplatesImport(body: Record<string, unknown> = {}): Promise<Array<{ description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }>> {
    return this.call("POST", `/api/v1/backup-templates/import`, undefined, body);
  }

  /** One policy template (API tokens: `read` scope). `GET /api/v1/backup-templates/{id}` */
  getBackupTemplatesId(id: string): Promise<{ description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }> {
    return this.call("GET", `/api/v1/backup-templates/${encodeURIComponent(id)}`, undefined, undefined);
  }

  /** Update a local policy template (API tokens: `write` scope). `PUT /api/v1/backup-templates/{id}` */
  putBackupTemplatesId(id: string, body: { description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }): Promise<{ description?: string; enforced?: boolean; id?: string; managed?: boolean; name: string; settings?: Record<string, unknown>; updated_at?: string }> {
    return this.call("PUT", `/api/v1/backup-templates/${encodeURIComponent(id)}`, undefined, body);
  }

  /** Delete a local policy template no job uses (API tokens: `write` scope). `DELETE /api/v1/backup-templates/{id}` */
  deleteBackupTemplatesId(id: string): Promise<{ ok?: boolean }> {
    return this.call("DELETE", `/api/v1/backup-templates/${encodeURIComponent(id)}`, undefined, undefined);
  }

  /** Base a job on this template too (appended: it wins over earlier ones) (API tokens: `write` scope). `POST /api/v1/backup-templates/{id}/apply` */
  postBackupTemplatesIdApply(id: string, body: { job_id: string }): Promise<BackupJob> {
    return this.call("POST", `/api/v1/backup-templates/${encodeURIComponent(id)}/apply`, undefined, body);
  }

  /** Test a destination (optionally create its folder); returns the files found (API tokens: `write` scope). `POST /api/v1/destinations/test` */
  postDestinationsTest(body: { create?: boolean; destination: Record<string, unknown> }): Promise<{ files?: string[] }> {
    return this.call("POST", `/api/v1/destinations/test`, undefined, body);
  }

  /** Browse the server's file system (source picker) (API tokens: `read` scope). `GET /api/v1/filesystem` */
  getFilesystem(query: { path?: string; folders_only?: boolean } = {}): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/filesystem`, query, undefined);
  }

  /** Folder policies set on shared folders (API tokens: `read` scope). `GET /api/v1/folder-policies` */
  getFolderPolicies(): Promise<{ policies?: Array<{ path?: string; policy?: FolderPolicy; updated_at?: string }> }> {
    return this.call("GET", `/api/v1/folder-policies`, undefined, undefined);
  }

  /** Set a folder's own policy (an empty policy removes it; subfolders inherit) (API tokens: `write` scope). `PUT /api/v1/folder-policies` */
  putFolderPolicies(body: { path: string; policy?: FolderPolicy }): Promise<{ effective?: Record<string, unknown>; own?: FolderPolicy | null; path?: string }> {
    return this.call("PUT", `/api/v1/folder-policies`, undefined, body);
  }

  /** Copy a folder's own policy to another folder (API tokens: `write` scope). `POST /api/v1/folder-policies/copy` */
  postFolderPoliciesCopy(body: { from: string; to: string }): Promise<{ effective?: Record<string, unknown>; own?: FolderPolicy | null; path?: string }> {
    return this.call("POST", `/api/v1/folder-policies/copy`, undefined, body);
  }

  /** The policy in force for a folder (own settings merged with its parents') (API tokens: `read` scope). `GET /api/v1/folder-policies/effective` */
  getFolderPoliciesEffective(query: { path: string }): Promise<{ effective?: Record<string, unknown>; own?: FolderPolicy | null; path?: string }> {
    return this.call("GET", `/api/v1/folder-policies/effective`, query, undefined);
  }

  /** Gateway agent: answer a job (header x-ufop-gateway-status ok|error; the body is streamed to the client) (API tokens: `write` scope). `POST /api/v1/gateway/jobs/{id}/reply` */
  postGatewayJobsIdReply(id: string): Promise<{ ok?: boolean }> {
    return this.call("POST", `/api/v1/gateway/jobs/${encodeURIComponent(id)}/reply`, undefined, undefined);
  }

  /** Gateway agent: leave the pool (sent when the agent stops) (API tokens: `write` scope). `POST /api/v1/gateway/leave` */
  postGatewayLeave(body: { gateway: string; node: string; [key: string]: unknown }): Promise<{ ok?: boolean }> {
    return this.call("POST", `/api/v1/gateway/leave`, undefined, body);
  }

  /** Gateway agent: register this node and wait (up to 25 s) for a job; 204 when none (write scope) (API tokens: `write` scope). `POST /api/v1/gateway/poll` */
  postGatewayPoll(body: { gateway: string; host?: string; node: string; roots: string[]; version?: string; writable?: boolean }): Promise<{ id?: string; op?: "list" | "download" | "upload"; path?: string; size?: number | null }> {
    return this.call("POST", `/api/v1/gateway/poll`, undefined, body);
  }

  /** Gateways and their nodes (online, roots, writable, jobs) (API tokens: `read` scope). `GET /api/v1/gateways` */
  getGateways(): Promise<{ gateways?: Array<Record<string, unknown>> }> {
    return this.call("GET", `/api/v1/gateways`, undefined, undefined);
  }

  /** List a folder through a gateway (`/` = its roots) (API tokens: `read` scope). `GET /api/v1/gateways/{name}/files` */
  getGatewaysNameFiles(name: string, query: { path?: string } = {}): Promise<{ entries?: Array<{ is_dir?: boolean; modified?: string | null; name?: string; size?: number }>; path?: string }> {
    return this.call("GET", `/api/v1/gateways/${encodeURIComponent(name)}/files`, query, undefined);
  }

  /** Upload the request body as a file through a writable gateway (API tokens: `write` scope). `PUT /api/v1/gateways/{name}/upload` */
  putGatewaysNameUpload(name: string, query: { path: string }): Promise<{ bytes?: number; path?: string }> {
    return this.call("PUT", `/api/v1/gateways/${encodeURIComponent(name)}/upload`, query, undefined);
  }

  /** Liveness, version and scheduler summary. `GET /api/v1/health` */
  getHealth(): Promise<{ active_tasks?: number; queued_tasks?: number; scheduler_paused?: boolean; scheduler_running?: boolean; status?: "ok"; uptime_secs?: number; version?: string }> {
    return this.call("GET", `/api/v1/health`, undefined, undefined);
  }

  /** List backup jobs (API tokens: `read` scope). `GET /api/v1/jobs` */
  getJobs(): Promise<BackupJob[]> {
    return this.call("GET", `/api/v1/jobs`, undefined, undefined);
  }

  /** Create a job (API tokens: `write` scope). `POST /api/v1/jobs` */
  postJobs(body: { job: BackupJob; passphrase?: string | null }): Promise<BackupJob> {
    return this.call("POST", `/api/v1/jobs`, undefined, body);
  }

  /** Import an exported job configuration (API tokens: `write` scope). `POST /api/v1/jobs/import` */
  postJobsImport(body: { config: string; export_passphrase?: string | null; import_stats?: boolean; save?: boolean }): Promise<Record<string, unknown>> {
    return this.call("POST", `/api/v1/jobs/import`, undefined, body);
  }

  /** Back up every job now (queued; read-only and busy jobs are skipped) (API tokens: `run` scope). `POST /api/v1/jobs/run-all` */
  postJobsRunAll(body: { dry_run?: boolean; job_ids?: string[]; scheduled_only?: boolean } = {}): Promise<{ skipped?: Array<{ job_id?: string; job_name?: string; reason?: string }>; started?: Array<{ job_id?: string; job_name?: string; task_id?: string }> }> {
    return this.call("POST", `/api/v1/jobs/run-all`, undefined, body);
  }

  /** Get a job (API tokens: `read` scope). `GET /api/v1/jobs/{id}` */
  getJobsId(id: string): Promise<BackupJob> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}`, undefined, undefined);
  }

  /** Replace a job (API tokens: `write` scope). `PUT /api/v1/jobs/{id}` */
  putJobsId(id: string, body: { job: BackupJob; passphrase?: string | null }): Promise<BackupJob> {
    return this.call("PUT", `/api/v1/jobs/${encodeURIComponent(id)}`, undefined, body);
  }

  /** Delete a job (API tokens: `write` scope). `DELETE /api/v1/jobs/{id}` */
  deleteJobsId(id: string, query: { delete_local_db?: boolean; delete_remote?: boolean } = {}): Promise<{ ok?: boolean }> {
    return this.call("DELETE", `/api/v1/jobs/${encodeURIComponent(id)}`, query, undefined);
  }

  /** The equivalent `ufop backup` command line (API tokens: `read` scope). `GET /api/v1/jobs/{id}/cli` */
  getJobsIdCli(id: string): Promise<{ command?: string }> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/cli`, undefined, undefined);
  }

  /** Differences between two versions (API tokens: `read` scope). `GET /api/v1/jobs/{id}/compare` */
  getJobsIdCompare(id: string, query: { base: number; compare: number; source?: string }): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/compare`, query, undefined);
  }

  /** Encrypt or decrypt the job's local database now (key kept in the credential store) (API tokens: `write` scope). `PUT /api/v1/jobs/{id}/database-encryption` */
  putJobsIdDatabaseEncryption(id: string, body: { enabled: boolean }): Promise<{ encrypted?: boolean; files?: number; settings?: number }> {
    return this.call("PUT", `/api/v1/jobs/${encodeURIComponent(id)}/database-encryption`, undefined, body);
  }

  /** Each setting the job runs with and where it comes from (built_in, global, template, job, managed) (API tokens: `read` scope). `GET /api/v1/jobs/{id}/effective-settings` */
  getJobsIdEffectiveSettings(id: string): Promise<{ job_id?: string; overrides?: string[] | null; settings?: Array<{ field_path?: string; inherited?: Record<string, unknown> | null; source?: { id?: string; name?: string; type?: "built_in" | "global" | "template" | "job" | "managed" }; value?: unknown }>; template_ids?: string[]; warnings?: string[] }> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/effective-settings`, undefined, undefined);
  }

  /** Estimate the next backup: files, new data (at most / at least) and duration; reads nothing (API tokens: `read` scope). `GET /api/v1/jobs/{id}/estimate` */
  getJobsIdEstimate(id: string, query: { assumed_bytes_per_sec?: number } = {}): Promise<{ bytes_examined?: number; bytes_per_sec?: number | null; changed_files?: number; deleted_files?: number; estimated_duration_secs?: number | null; files?: number; folders?: number; has_previous_version?: boolean; largest?: Array<{ path?: string; size?: number; status?: "new" | "changed" }>; new_bytes_at_least?: number; new_bytes_at_most?: number; new_files?: number; speed_source?: "history" | "assumed" | "none"; unchanged_files?: number; upper_bound?: boolean; warnings?: string[] }> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/estimate`, query, undefined);
  }

  /** Export the job configuration (API tokens: `read` scope). `GET /api/v1/jobs/{id}/export` */
  getJobsIdExport(id: string, query: { include_passphrase?: boolean; export_passphrase?: string } = {}): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/export`, query, undefined);
  }

  /** Files in a version (API tokens: `read` scope). `GET /api/v1/jobs/{id}/files` */
  getJobsIdFiles(id: string, query: { version?: string; time?: string; allow_partial?: boolean; prefix?: string; source?: string } = {}): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/files`, query, undefined);
  }

  /** Repository lock status: who holds the destination (host, operation, since, expires) (API tokens: `read` scope). `GET /api/v1/jobs/{id}/lock` */
  getJobsIdLock(id: string): Promise<{ checked_at?: string; exclusive?: boolean; locks?: Array<{ acquired_at?: string; app?: string; damaged?: boolean; expires_at?: string; file?: string; hostname?: string; kind?: "exclusive" | "shared"; operation?: string; owner_id?: string; pid?: number; stale?: boolean; username?: string; version?: string }>; readers?: number }> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/lock`, undefined, undefined);
  }

  /** Break the repository lock (only when its holder is no longer running) (API tokens: `write` scope). `DELETE /api/v1/jobs/{id}/lock` */
  deleteJobsIdLock(id: string, query: { confirm: boolean; owner_id?: string }): Promise<{ holders?: string[]; removed?: string[] }> {
    return this.call("DELETE", `/api/v1/jobs/${encodeURIComponent(id)}/lock`, query, undefined);
  }

  /** Restore, verify, compact, repair, purge … (a tagged BackupOperation; not `backup`). Verify takes `options`: scope {versions, paths}, mode {type: structure | sample_volumes(count) | sample_percent(percent) | full}, files_percent, parallelism, max_errors, use_cache (API tokens: `run` scope). `POST /api/v1/jobs/{id}/operations` */
  postJobsIdOperations(id: string, body: { type: string; [key: string]: unknown }): Promise<Record<string, unknown>> {
    return this.call("POST", `/api/v1/jobs/${encodeURIComponent(id)}/operations`, undefined, body);
  }

  /** Change an encrypted job's passphrase (format 2 repositories rewrite only the header) (API tokens: `write` scope). `PUT /api/v1/jobs/{id}/passphrase` */
  putJobsIdPassphrase(id: string, body: { current?: string | null; kdf_preset?: "interactive" | "default" | "sensitive" | null; new_passphrase: string }): Promise<Record<string, unknown>> {
    return this.call("PUT", `/api/v1/jobs/${encodeURIComponent(id)}/passphrase`, undefined, body);
  }

  /** Open the job's versioned S3 destination as it was at a time: creates a read-only job whose index is rebuilt from the object versions of then (API tokens: `write` scope). `POST /api/v1/jobs/{id}/point-in-time` */
  postJobsIdPointInTime(id: string, body: { at: string; name?: string | null }): Promise<BackupJob> {
    return this.call("POST", `/api/v1/jobs/${encodeURIComponent(id)}/point-in-time`, undefined, body);
  }

  /** What retention would keep and delete (the job's policy or an unsaved one); changes nothing (API tokens: `write` scope). `POST /api/v1/jobs/{id}/retention/preview` */
  postJobsIdRetentionPreview(id: string, body: { policy?: RetentionPolicy; source?: string | null } = {}): Promise<{ delete?: number; keep?: number; legal_hold?: boolean; policy?: RetentionPolicy; versions?: Array<{ id?: number; keep?: boolean; reasons?: string[]; summary?: string; timestamp?: string }> }> {
    return this.call("POST", `/api/v1/jobs/${encodeURIComponent(id)}/retention/preview`, undefined, body);
  }

  /** Start a backup (queued); returns the task id (API tokens: `run` scope). `POST /api/v1/jobs/{id}/run` */
  postJobsIdRun(id: string, body: { dry_run?: boolean } = {}): Promise<{ task_id?: string }> {
    return this.call("POST", `/api/v1/jobs/${encodeURIComponent(id)}/run`, undefined, body);
  }

  /** Search file names across versions (API tokens: `read` scope). `GET /api/v1/jobs/{id}/search` */
  getJobsIdSearch(id: string, query: { q: string; version?: number; source?: string }): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/search`, query, undefined);
  }

  /** Sources (jobs and computers) with versions in the job's repository (API tokens: `read` scope). `GET /api/v1/jobs/{id}/sources` */
  getJobsIdSources(id: string): Promise<Array<{ id?: string; identity?: Record<string, unknown> | null; label?: string; newest?: string | null; newest_size?: number; oldest?: string | null; own?: boolean; versions?: number }>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/sources`, undefined, undefined);
  }

  /** Backup versions of a job (with note, tags, pin, identity and storage added) (API tokens: `read` scope). `GET /api/v1/jobs/{id}/versions` */
  getJobsIdVersions(id: string, query: { source?: string; tags?: string } = {}): Promise<Fileset[]> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/versions`, query, undefined);
  }

  /** Which version a time (or latest / oldest / number / id) picks (API tokens: `read` scope). `GET /api/v1/jobs/{id}/versions/resolve` */
  getJobsIdVersionsResolve(id: string, query: { version?: string; time?: string; allow_partial?: boolean; source?: string } = {}): Promise<{ fileset?: Fileset; index?: number }> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/versions/resolve`, query, undefined);
  }

  /** Change a version's note, tags or pin (rewrites its file list) (API tokens: `write` scope). `PATCH /api/v1/jobs/{id}/versions/{version}` */
  patchJobsIdVersionsVersion(id: string, version: string, body: { note?: string | null; pin_reason?: string | null; pinned?: boolean | null; source?: string | null; tags?: Record<string, unknown> | null } = {}): Promise<Record<string, unknown>> {
    return this.call("PATCH", `/api/v1/jobs/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}`, undefined, body);
  }

  /** Enabled and disabled modules (API tokens: `read` scope). `GET /api/v1/modules` */
  getModules(): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/modules`, undefined, undefined);
  }

  /** Configured OAuth providers for cloud destinations (API tokens: `admin` scope). `GET /api/v1/oauth/providers` */
  getOauthProviders(): Promise<Array<Record<string, unknown>>> {
    return this.call("GET", `/api/v1/oauth/providers`, undefined, undefined);
  }

  /** This document. `GET /api/v1/openapi.json` */
  getOpenapiJson(): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/openapi.json`, undefined, undefined);
  }

  /** Call a desktop IPC command by name (arguments as the JSON body); see docs/ipc-commands.md (API tokens: `admin` scope). `POST /api/v1/rpc/{command}` */
  postRpcCommand(command: string, body: Record<string, unknown> = {}): Promise<Record<string, unknown>> {
    return this.call("POST", `/api/v1/rpc/${encodeURIComponent(command)}`, undefined, body);
  }

  /** Run history (API tokens: `read` scope). `GET /api/v1/runs` */
  getRuns(query: { job_id?: string; limit?: number } = {}): Promise<Array<Record<string, unknown>>> {
    return this.call("GET", `/api/v1/runs`, query, undefined);
  }

  /** Pause the scheduler (optionally for N minutes) (API tokens: `run` scope). `POST /api/v1/scheduler/pause` */
  postSchedulerPause(body: { minutes?: number | null } = {}): Promise<SchedulerStatus> {
    return this.call("POST", `/api/v1/scheduler/pause`, undefined, body);
  }

  /** Resume the scheduler (API tokens: `run` scope). `POST /api/v1/scheduler/resume` */
  postSchedulerResume(): Promise<SchedulerStatus> {
    return this.call("POST", `/api/v1/scheduler/resume`, undefined, undefined);
  }

  /** Stop the server gracefully: new mutating requests get 503, running backups stop at a safe point (partial version) within grace_secs (default --shutdown-grace), then the process exits 0; a second call stops at once. Answers 202 (API tokens: `admin` scope). `POST /api/v1/server/shutdown` */
  postServerShutdown(body: { grace_secs?: number | null } = {}): Promise<{ accepted?: boolean; grace_secs?: number; immediate?: boolean; running_tasks?: number }> {
    return this.call("POST", `/api/v1/server/shutdown`, undefined, body);
  }

  /** Server settings (API tokens: `read` scope). `GET /api/v1/settings` */
  getSettings(): Promise<{ allow_automation_tokens?: boolean }> {
    return this.call("GET", `/api/v1/settings`, undefined, undefined);
  }

  /** Change server settings (API tokens: `admin` scope). `PUT /api/v1/settings` */
  putSettings(body: { allow_automation_tokens?: boolean } = {}): Promise<{ allow_automation_tokens?: boolean }> {
    return this.call("PUT", `/api/v1/settings`, undefined, body);
  }

  /** Share links (tokens are never returned) and the folders that may be shared (API tokens: `read` scope). `GET /api/v1/shares` */
  getShares(): Promise<{ roots?: string[]; shares?: Share[] }> {
    return this.call("GET", `/api/v1/shares`, undefined, undefined);
  }

  /** Create a share link to a file or folder under --share-root; the link is returned once (API tokens: `write` scope). `POST /api/v1/shares` */
  postShares(body: { expires_in_days?: number | null; kind?: "download" | "upload" | "both"; max_downloads?: number | null; max_upload_mb?: number | null; name: string; password?: string | null; path: string }): Promise<{ share?: Share; token?: string; url?: string }> {
    return this.call("POST", `/api/v1/shares`, undefined, body);
  }

  /** Turn a share link off (API tokens: `write` scope). `DELETE /api/v1/shares/{id}` */
  deleteSharesId(id: string): Promise<{ ok?: boolean }> {
    return this.call("DELETE", `/api/v1/shares/${encodeURIComponent(id)}`, undefined, undefined);
  }

  /** Access log of a link (views, downloads, uploads, refusals, wrong passwords) (API tokens: `read` scope). `GET /api/v1/shares/{id}/events` */
  getSharesIdEvents(id: string, query: { limit?: number } = {}): Promise<{ events?: Array<{ action?: string; at?: string; detail?: string; ip?: string }> }> {
    return this.call("GET", `/api/v1/shares/${encodeURIComponent(id)}/events`, query, undefined);
  }

  /** Scheduler state and tasks (API tokens: `read` scope). `GET /api/v1/status` */
  getStatus(): Promise<SchedulerStatus> {
    return this.call("GET", `/api/v1/status`, undefined, undefined);
  }

  /** Queued and running tasks (API tokens: `read` scope). `GET /api/v1/tasks` */
  getTasks(): Promise<TaskInfo[]> {
    return this.call("GET", `/api/v1/tasks`, undefined, undefined);
  }

  /** Cancel a task (API tokens: `run` scope). `POST /api/v1/tasks/{id}/cancel` */
  postTasksIdCancel(id: string): Promise<{ cancelled?: boolean }> {
    return this.call("POST", `/api/v1/tasks/${encodeURIComponent(id)}/cancel`, undefined, undefined);
  }
}
