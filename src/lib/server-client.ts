/**
 * Client for `ufop-server` (headless mode). When the page is served by the
 * server, `window.__UFOP_SERVER__` is injected into index.html and IPC
 * commands go to `POST /api/v1/rpc/<command>` with the same arguments the
 * desktop app passes to Tauri `invoke`.
 *
 * The access token lives only in memory; the refresh token is an
 * HttpOnly cookie scoped to /api/v1/auth, so a reload re-establishes the
 * session without exposing it to scripts.
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

export async function login(password: string): Promise<void> {
  const r = await fetch(api("/auth/login"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ password }),
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

/** Exchange the refresh cookie for a new access token. */
export async function refresh(): Promise<boolean> {
  if (!hasSessionCookie()) {
    accessToken = null;
    notify();
    return false;
  }
  const r = await fetch(api("/auth/refresh"), {
    method: "POST",
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
        ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
        Authorization: `Bearer ${accessToken}`,
      },
      credentials: "same-origin",
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (r.status === 401 && attempt === 0) {
      accessToken = null;
      continue;
    }
    if (!r.ok) throw await errorFrom(r);
    return (await r.json()) as T;
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

/** Test hook. */
export function __setAccessTokenForTests(t: string | null) {
  accessToken = t;
}
