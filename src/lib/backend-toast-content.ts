/**
 * Backend toast content selection (iter 23).
 *
 * Pure helper that picks the best toast text for a batch of backend
 * mutations. Decouples the content-selection logic from the hook's
 * effectful bits (IPC, Zustand, timers) so it's unit-testable
 * without mocking React or Tauri.
 *
 * Selection logic:
 *
 *   1. When we have a REAL narrator narrative (not the
 *      "no events" or "unavailable" fallback) AND exactly one new
 *      correlation, use the narrator's `headline` and derive
 *      `why` from its engines + duration. This produces
 *      dramatically more readable toasts like:
 *
 *          what: "Sync completed 3 files in 245ms"
 *          why:  "Background activity from sync \u00b7 245ms"
 *
 *      instead of the raw ledger summary like:
 *
 *          what: "sync/sync_push: /Users/md/docs/foo.pdf"
 *          why:  "Background activity from sync"
 *
 *   2. When we have no narrative, fall back to the iter-21/22
 *      aggregate behavior: `singleSummary` when count === 1,
 *      generic "N background operations" otherwise.
 *
 * The "do we have a real narrative" test relies on
 * `narrative.event_count > 0` \u2014 both the narrator's empty-trace
 * case AND the frontend wrapper's Tauri-unavailable fallback set
 * `event_count: 0`, so this single check correctly identifies
 * both failure modes.
 */
import type { BackendSummary } from "./ledger-tail-extract";
import type { OperationNarrative } from "./narrator";

/** Shape returned by `selectToastContent`. Only the two content
 *  fields are computed here; `appDid` / `userAction` are stable
 *  strings chosen by the caller. */
export interface ToastContent {
  /** Primary toast headline \u2014 the narrator's `headline` when
   *  available, otherwise an aggregate or single-summary string. */
  what: string;
  /** Secondary "why" line \u2014 engines + optional narrator duration. */
  why: string;
}

/**
 * Returns true when `narrative` represents a real ledger trace
 * with actual events (as opposed to the empty-trace or
 * unavailable-IPC fallback).
 *
 * Exported for tests so the discriminator logic can be verified
 * independently of the selection caller.
 */
export function isRealNarrative(
  narrative: OperationNarrative | null,
): boolean {
  return narrative !== null && narrative.event_count > 0;
}

/**
 * Pick the toast content for a backend-mutation tick.
 *
 * The `narrative` parameter may be null when the caller chose not
 * to fetch one (e.g. because there are multiple new correlations
 * and narrating all of them would be wasteful). In that case we
 * always fall back to the aggregate or single-summary path.
 */
export function selectToastContent(
  summary: BackendSummary,
  newCorrelations: string[],
  narrative: OperationNarrative | null,
): ToastContent {
  // Branch 1: narrator path. Requires exactly one new correlation
  // AND a real narrative (event_count > 0). Multiple new
  // correlations skip this branch because picking just one
  // would under-represent the tick's activity.
  if (newCorrelations.length === 1 && isRealNarrative(narrative)) {
    // Safe to assert non-null because isRealNarrative returned
    // true, which requires narrative !== null.
    const n = narrative!;
    const engineLabel =
      n.engines_involved.length > 0
        ? n.engines_involved.join(", ")
        : summary.engines.join(", ");
    const duration =
      n.duration_ms > 0 ? ` \u00b7 ${formatDuration(n.duration_ms)}` : "";
    return {
      what: n.headline,
      why: `Background activity from ${engineLabel}${duration}`,
    };
  }

  // Branch 2: aggregate fallback (iter 21/22 behavior).
  const what =
    summary.count === 1 && summary.singleSummary
      ? summary.singleSummary
      : `${summary.count} background operation${summary.count === 1 ? "" : "s"}`;
  return {
    what,
    why: `Background activity from ${summary.engines.join(", ")}`,
  };
}

/** Format a duration for the `why` line. Keeps the output compact
 *  so it fits on one line in the toast stack: `245ms`, `3.2s`,
 *  `1m 12s`. */
function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.floor((ms % 60_000) / 1000);
  return `${minutes}m ${seconds}s`;
}
