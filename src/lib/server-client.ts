/**
 * Client for `ufop-server` (headless mode). When the page is served by the
 * server, `window.__UFOP_SERVER__` is injected into index.html and IPC
 * commands go to `POST /api/v1/rpc/<command>` with the same arguments the
 * desktop app passes to Tauri `invoke`.
 *
 * The access token lives only in memory; the refresh token is an
 * HttpOnly cookie scoped to /api/v1/auth, so a reload re-establishes the
 * session without exposing it to scripts. Refresh and sign-out send the
 * `ufop_csrf` cookie's value as `X-CSRF-Token` (double-submit CSRF check).
 */

interface ServerFlag {
  api: string;
}

declare global {
  interface Window {
    __UFOP_SERVER__?: ServerFlag;
  }
}

let accessToken: string | null = null;
const listeners = new Set<(signedIn: boolean) => void>();

export function isServerMode(): boolean {
  return typeof window !== "undefined" && !!window.__UFOP_SERVER__;
}

function api(path: string): string {
  return `${window.__UFOP_SERVER__?.api ?? "/api/v1"}${path}`;
}

function notify() {
  listeners.forEach((l) => l(accessToken !== null));
}

export function onAuthChange(fn: (signedIn: boolean) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function isSignedIn(): boolean {
  return accessToken !== null;
}

async function errorFrom(r: Response): Promise<unknown> {
  try {
    return await r.json();
  } catch {
    return { message: `Server returned ${r.status}`, advice: "" };
  }
}

/** Sign in; `username` defaults to `admin` (the original server password). */
export async function login(
  password: string,
  username?: string,
): Promise<void> {
  const r = await fetch(api("/auth/login"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify(username ? { username, password } : { password }),
  });
  if (!r.ok) throw await errorFrom(r);
  accessToken = (await r.json()).access_token;
  notify();
}

function hasSessionCookie(): boolean {
  return (
    typeof document !== "undefined" &&
    document.cookie.split(";").some((c) => c.trim().startsWith("ufop_session="))
  );
}

/**
 * The double-submit CSRF token (`ufop_csrf` cookie, set with the refresh
 * cookie): requests authenticated by the refresh cookie send it as
 * `X-CSRF-Token`.
 */
export function csrfToken(): string | null {
  if (typeof document === "undefined") return null;
  const c = document.cookie
    .split(";")
    .map((x) => x.trim())
    .find((x) => x.startsWith("ufop_csrf="));
  return c ? decodeURIComponent(c.slice("ufop_csrf=".length)) : null;
}

function csrfHeaders(): Record<string, string> {
  const t = csrfToken();
  return t ? { "X-CSRF-Token": t } : {};
}

/** Exchange the refresh cookie for a new access token. */
export async function refresh(): Promise<boolean> {
  if (!hasSessionCookie()) {
    accessToken = null;
    notify();
    return false;
  }
  const r = await fetch(api("/auth/refresh"), {
    method: "POST",
    headers: csrfHeaders(),
    credentials: "same-origin",
  });
  if (!r.ok) {
    accessToken = null;
    notify();
    return false;
  }
  accessToken = (await r.json()).access_token;
  notify();
  return true;
}

export async function logout(): Promise<void> {
  await fetch(api("/auth/logout"), {
    method: "POST",
    headers: csrfHeaders(),
    credentials: "same-origin",
  }).catch(() => undefined);
  accessToken = null;
  notify();
}

/** Call a backup command on the server; retries once after refreshing an expired token. */
/**
 * Call the server's REST API (`/api/v1<path>`) with the session's access
 * token, renewing it once from the refresh cookie when it has expired.
 */
export async function serverApi<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const r = await serverRaw(
    method,
    path,
    body !== undefined ? JSON.stringify(body) : undefined,
    body !== undefined ? "application/json" : undefined,
  );
  return (await r.json()) as T;
}

/**
 * A REST call with a raw body and response (file downloads and uploads);
 * throws the server's `{message, advice}` on failure.
 */
export async function serverRaw(
  method: string,
  path: string,
  body?: BodyInit,
  contentType?: string,
): Promise<Response> {
  for (let attempt = 0; attempt < 2; attempt++) {
    if (!accessToken && !(await refresh())) {
      throw {
        message: "Not signed in",
        advice: "Sign in to the backup server.",
      };
    }
    const r = await fetch(api(path), {
      method,
      headers: {
        ...(contentType ? { "Content-Type": contentType } : {}),
        Authorization: `Bearer ${accessToken}`,
      },
      credentials: "same-origin",
      body,
    });
    if (r.status === 401 && attempt === 0) {
      accessToken = null;
      continue;
    }
    if (!r.ok) throw await errorFrom(r);
    return r;
  }
  throw { message: "Not signed in", advice: "Sign in to the backup server." };
}

/** An IPC command over `POST /api/v1/rpc/<command>`. */
export function serverRpc<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return serverApi<T>(
    "POST",
    `/rpc/${encodeURIComponent(command)}`,
    args ?? {},
  );
}

/** The signed-in account (`GET /auth/me`). */
export interface ServerMe {
  username: string;
  role: "admin" | "operator" | "viewer";
  is_admin: boolean;
  token: { id: string; scope: string; job_ids: string[] | null } | null;
  jobs: { job_id: string; job_name: string; permission: string }[];
}

export function whoAmI(): Promise<ServerMe> {
  return serverApi<ServerMe>("GET", "/auth/me");
}

/** Test hook. */
export function __setAccessTokenForTests(t: string | null) {
  accessToken = t;
}
