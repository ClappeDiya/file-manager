/**
 * ledger-event-flags
 *
 * Pure predicates that decide whether an activity-timeline row should
 * surface the Pin (re-runnable Quickflow) or Retry (failure mirror)
 * affordance. Extracted into a standalone module so the React panel
 * stays a pure component file (fast-refresh clean) and so the rules
 * can be exhaustively tested without rendering anything.
 *
 * Pin and Retry are designed as strict mirrors: same engine, same
 * kind allow-list, same correlation-id requirement — only the status
 * set differs. The mutual-exclusion invariant is asserted by the
 * accompanying test in `__tests__/timeline-event-flags.test.ts`.
 *
 * Must stay in lock-step with `automation_engine::pinner` on the Rust
 * side: `PINNABLE_KINDS`, `PIN_STATUSES`, and `RETRY_STATUSES` are
 * the four constants that gate the cross-boundary symmetry.
 */
import type { LedgerEventWire as LedgerEvent } from "./ledger-tail-extract";

/**
 * Ledger event kinds that can be promoted to a re-runnable manual
 * Quickflow via the Operation Pin flow, or re-attempted via the
 * Operation Retry flow. Mirror of `pinner::PINNABLE_KINDS`.
 */
export const PINNABLE_KINDS: ReadonlySet<string> = new Set(["copy", "move"]);

/**
 * Statuses that the Retry flow accepts. Mirror of
 * `pinner::RETRY_STATUSES`. `skipped` is intentionally excluded — a
 * skip is an explicit decision, not a transient failure to recover
 * from.
 */
export const RETRYABLE_STATUSES: ReadonlySet<string> = new Set([
  "failed",
  "cancelled",
]);

/** True for successful fs.copy / fs.move events that carry a correlation id. */
export function isPinnableEvent(event: LedgerEvent): boolean {
  return (
    event.engine === "fs" &&
    event.status === "ok" &&
    PINNABLE_KINDS.has(event.kind) &&
    event.correlation_id !== null &&
    event.correlation_id.length > 0
  );
}

/**
 * Failure-side counterpart of `isPinnableEvent`. A row is retryable
 * when it is a failed (or cancelled) fs.copy / fs.move event with a
 * correlation id — i.e. the same shape Pinner consumes, just with the
 * opposite status. Pin and Retry are *mutually exclusive* on any given
 * row because each ledger event has exactly one status; the timeline
 * row therefore renders at most one of the two buttons.
 */
export function isRetryableEvent(event: LedgerEvent): boolean {
  return (
    event.engine === "fs" &&
    RETRYABLE_STATUSES.has(event.status) &&
    PINNABLE_KINDS.has(event.kind) &&
    event.correlation_id !== null &&
    event.correlation_id.length > 0
  );
}

/**
 * Fs-engine event kinds eligible for Undo/Redo per-entry affordances —
 * mirror of `UNDOABLE_FS_KINDS` in `commands::undo_commands` (Rust). The
 * `fs.undone` / `fs.redone` markers carry the same correlation_id as
 * the original op, so without this kind filter both the marker row and
 * the original op row would surface the Redo button. Keep this list
 * in lock-step with the Rust constant; both sides MUST agree.
 */
export const UNDOABLE_FS_KINDS: ReadonlySet<string> = new Set([
  "copy",
  "duplicate",
  "move",
  "rename",
  "create_folder",
  "create_file",
]);

/**
 * True when an event row is the kind that Undo/Redo affordances should
 * attach to — i.e. an original fs operation, not a marker (`fs.undone`,
 * `fs.redone`) or a non-fs engine event. The marker rows still appear
 * in the timeline for observability, they just don't carry the action
 * buttons themselves.
 */
export function isUndoableKindEvent(event: LedgerEvent): boolean {
  return (
    event.engine === "fs" &&
    UNDOABLE_FS_KINDS.has(event.kind) &&
    event.correlation_id !== null &&
    event.correlation_id.length > 0
  );
}
