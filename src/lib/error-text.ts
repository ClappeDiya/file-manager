/**
 * Human-readable text for anything a failed IPC call (or ordinary throw) can
 * produce. Tauri rejects with the backend's `{ message, advice }` payload, not
 * an `Error`, so `String(err)` would render as "[object Object]".
 */
export function errorText(err: unknown, fallback?: string): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const e = err as { message: unknown; advice?: unknown };
    const msg = String(e.message);
    return typeof e.advice === "string" && e.advice ? `${msg} ${e.advice}` : msg;
  }
  return fallback ?? String(err);
}
