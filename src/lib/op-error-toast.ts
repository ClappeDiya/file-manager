/**
 * op-error-toast
 *
 * Centralises the "destructive operation failed — surface it to the
 * user" pattern. Every fs-mutation handler in `file-manager.tsx`
 * (copy / move / delete / permanent delete / rename / duplicate /
 * create directory / create file …) wrapped its IPC call in:
 *
 *   try { ... } catch (err) { console.error("Copy failed:", err); }
 *
 * — which means a Rust-side failure (permission denied, disk full,
 * source missing, destination read-only) was invisible to the user.
 * They clicked Delete, the OS rejected it, and the file silently
 * stayed where it was with NO indication of what went wrong.
 *
 * This helper funnels every such catch through the canonical
 * structured-toast surface (`useUIStore.addStructuredError`) so:
 *   - the user sees a toast immediately
 *   - the toast `what` carries the operation name ("Delete failed")
 *   - the toast `why` carries the underlying error message
 *   - the toast `appDid` / `userAction` give a clear recovery hint
 *
 * The console.error call is preserved alongside the toast so
 * developer-facing logs keep working — the toast is purely additive.
 *
 * # Why structured toasts
 *
 * Routes through the same channel as every other UFOP "system told
 * me what happened" event (narrator, retry, undo, copy-feedback,
 * since-last-seen). Uniform vocabulary means a future
 * toast-grouping / sound / accessibility pass applies everywhere.
 */
import { useUIStore } from "@/stores/ui-store";

export interface OperationFailureContext {
  /** Optional override for the "what the app did" line. Defaults to
   *  "Did not change anything on disk" which is the truthful answer
   *  for every fs-mutation IPC since the backend either completes
   *  atomically or rolls back on error. */
  appDid?: string;
  /** Optional override for the "what the user can do" line. Defaults
   *  to a generic permissions/path recovery hint. */
  userAction?: string;
}

/**
 * Surface a destructive-operation failure via the structured-toast
 * channel and log it for developer visibility. `operation` is the
 * human-readable name of the operation the user fired ("Copy",
 * "Delete", "Rename") — appears verbatim in the toast `what` line
 * with " failed" appended.
 *
 * Side-effects only: the function returns `void` so callers can fire
 * it from inside a `catch` block without an `await`.
 */
export function reportOperationFailure(
  operation: string,
  err: unknown,
  context?: OperationFailureContext,
): void {
  // Preserve the developer-facing log line. The pre-existing code
  // all logged with this exact format ("Copy failed: <err>"), so
  // matching it keeps existing dev-mode breakpoints and grep
  // workflows working.
  console.error(`${operation} failed:`, err);
  useUIStore.getState().addStructuredError({
    what: `${operation} failed`,
    why:
      err instanceof Error && err.message
        ? err.message
        : typeof err === "string" && err.length > 0
          ? err
          : "The operation returned an error",
    appDid: context?.appDid ?? "Did not change anything on disk",
    userAction:
      context?.userAction ??
      "Check that source files exist, the destination is writable, and you have permission to perform this action",
  });
}
