// Generated from the ufop-server OpenAPI document by src-tauri/src/server/sdk.rs. Do not edit.
// Regenerate: UPDATE_SDK=1 cargo test --lib sdk

/** API version this client was generated from. */
export const API_VERSION = "0.1.0";

export type ApiToken = { created_at?: string; expires_at?: string; id?: string; last_used_at?: string | null; name?: string; scope?: Scope; [key: string]: unknown };

/** A backup job (sources, destination, schedule, retention, filters, encryption); see docs/api-reference.md */
export type BackupJob = Record<string, unknown>;

export type ApiError = { advice?: string; error_type?: string; message: string };

/** null fields inherit from the parent folder */
export type FolderPolicy = { allowed_extensions?: string[] | null; blocked_extensions?: string[] | null; intake_naming?: "keep" | "timestamp" | "sequence" | null; locked?: boolean | null; max_link_days?: number | null; max_upload_mb?: number | null; require_link_password?: boolean | null };

export type SchedulerStatus = { paused?: boolean; running?: boolean; tasks?: TaskInfo[]; [key: string]: unknown };

/** read < run < write < admin */
export type Scope = "read" | "run" | "write" | "admin";

export type Session = { access_token?: string; expires_at?: string };

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

  /** Revoke the refresh cookie's session. `POST /api/v1/auth/logout` */
  postAuthLogout(): Promise<{ ok?: boolean }> {
    return this.call("POST", `/api/v1/auth/logout`, undefined, undefined);
  }

  /** Change the server password (API tokens: `admin` scope). `PUT /api/v1/auth/password` */
  putAuthPassword(body: { current_password: string; new_password: string }): Promise<{ ok?: boolean }> {
    return this.call("PUT", `/api/v1/auth/password`, undefined, body);
  }

  /** New access token from the refresh cookie (rotated). `POST /api/v1/auth/refresh` */
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
  getJobsIdCompare(id: string, query: { base: number; compare: number }): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/compare`, query, undefined);
  }

  /** Export the job configuration (API tokens: `read` scope). `GET /api/v1/jobs/{id}/export` */
  getJobsIdExport(id: string, query: { include_passphrase?: boolean; export_passphrase?: string } = {}): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/export`, query, undefined);
  }

  /** Files in a version (API tokens: `read` scope). `GET /api/v1/jobs/{id}/files` */
  getJobsIdFiles(id: string, query: { version?: number; prefix?: string } = {}): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/files`, query, undefined);
  }

  /** Restore, verify, compact, repair, purge … (a tagged BackupOperation; not `backup`) (API tokens: `run` scope). `POST /api/v1/jobs/{id}/operations` */
  postJobsIdOperations(id: string, body: { type: string; [key: string]: unknown }): Promise<Record<string, unknown>> {
    return this.call("POST", `/api/v1/jobs/${encodeURIComponent(id)}/operations`, undefined, body);
  }

  /** Change an encrypted job's passphrase (format 2 repositories rewrite only the header) (API tokens: `write` scope). `PUT /api/v1/jobs/{id}/passphrase` */
  putJobsIdPassphrase(id: string, body: { current?: string | null; kdf_preset?: "interactive" | "default" | "sensitive" | null; new_passphrase: string }): Promise<Record<string, unknown>> {
    return this.call("PUT", `/api/v1/jobs/${encodeURIComponent(id)}/passphrase`, undefined, body);
  }

  /** Start a backup (queued); returns the task id (API tokens: `run` scope). `POST /api/v1/jobs/{id}/run` */
  postJobsIdRun(id: string, body: { dry_run?: boolean } = {}): Promise<{ task_id?: string }> {
    return this.call("POST", `/api/v1/jobs/${encodeURIComponent(id)}/run`, undefined, body);
  }

  /** Search file names across versions (API tokens: `read` scope). `GET /api/v1/jobs/{id}/search` */
  getJobsIdSearch(id: string, query: { q: string; version?: number }): Promise<Record<string, unknown>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/search`, query, undefined);
  }

  /** Backup versions of a job (API tokens: `read` scope). `GET /api/v1/jobs/{id}/versions` */
  getJobsIdVersions(id: string): Promise<Array<Record<string, unknown>>> {
    return this.call("GET", `/api/v1/jobs/${encodeURIComponent(id)}/versions`, undefined, undefined);
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
