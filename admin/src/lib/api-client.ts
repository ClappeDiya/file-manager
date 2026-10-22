/** Browser-side JSON client for the console API: same-origin, typed, errors as `ApiError`. */
export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

export async function api<T = Record<string, unknown>>(path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
  const res = await fetch(path, {
    method: init.method ?? (init.body === undefined ? 'GET' : 'POST'),
    headers: init.body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
    credentials: 'same-origin',
  });
  const data = (await res.json().catch(() => ({}))) as T & { error?: string };
  if (!res.ok) throw new ApiError(data.error || `Request failed (HTTP ${res.status})`, res.status);
  return data;
}

export const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));
