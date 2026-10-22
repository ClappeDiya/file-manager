/** Browser-side JSON client for the console API: same-origin, typed, errors as `ApiError`. */
import { useReauthStore, type ReauthMethod } from './stores/reauth-store';

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

export async function api<T = Record<string, unknown>>(path: string, init: { method?: string; body?: unknown } = {}, retried = false): Promise<T> {
  const res = await fetch(path, {
    method: init.method ?? (init.body === undefined ? 'GET' : 'POST'),
    headers: init.body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
    credentials: 'same-origin',
  });
  const data = (await res.json().catch(() => ({}))) as T & { error?: string; code?: string; methods?: ReauthMethod[] };
  // A sensitive change needs a recent sign-in: confirm and try once more.
  if (res.status === 403 && data.code === 'reauth_required' && !retried && typeof window !== 'undefined') {
    if (await useReauthStore.getState().request(data.methods ?? ['password'])) return api<T>(path, init, true);
  }
  // An organization requires two-factor sign-in: go and set it up.
  if (res.status === 403 && data.code === 'mfa_setup_required' && typeof window !== 'undefined' && !window.location.pathname.startsWith('/account')) {
    window.location.assign('/account?setup-mfa=1');
  }
  if (!res.ok) throw new ApiError(data.error || `Request failed (HTTP ${res.status})`, res.status);
  return data;
}

export const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));
