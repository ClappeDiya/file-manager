/**
 * useLedgerTailPoll (iter 20).
 *
 * Closes the auto-refresh loop for BACKEND-initiated mutations. The
 * iter 18 / iter 19 work made every frontend-initiated mutation
 * dispatch a path-aware refresh. This hook extends that pattern to
 * any mutation that lands in the operation ledger \u2014 transfer
 * engine, sync engine, automation, watcher, safety interlock \u2014
 * none of which go through the frontend mutation handlers.
 *
 * Architecture:
 *
 *   1. Every few seconds (bounded, defaults to 4s), poll
 *      `ledger_recent(limit)` \u2014 the existing IPC that returns the
 *      most recent events in DESC order.
 *   2. Track a watermark: the id of the newest event seen on the
 *      previous tick.
 *   3. Feed the batch + watermark into the pure helper
 *      `processTailBatch`, which returns the affected paths and the
 *      advanced watermark.
 *   4. If any paths changed, fire a single `dispatchRefresh(paths)`
 *      that the iter-18 listener then delivers to every pane whose
 *      viewed directory intersects.
 *
 * Safety properties:
 *
 *   - **No startup cascade**: the first tick seeds the watermark
 *     without firing any refresh (see `processTailBatch`).
 *   - **Pauses when hidden**: polls are skipped when the document
 *     is not visible, so a backgrounded app consumes zero power.
 *   - **Pauses outside Tauri**: `tauriInvokeSafe` returns `[]` in
 *     browser dev mode, and an empty batch is a no-op.
 *   - **Fail-soft**: any IPC error is swallowed (logged at debug)
 *     and the watermark is preserved so the next tick tries again.
 *   - **Bounded memory**: only tracks one string (the watermark).
 *   - **DRY**: reuses the exact `dispatchRefresh` helper every
 *     frontend mutation handler uses, so the rest of the refresh
 *     pipeline is shared end-to-end.
 */
import { useEffect, useRef } from "react";
import { tauriInvokeSafe, isTauriAvailable } from "./use-tauri";
import { dispatchRefresh } from "@/lib/refresh-affected";
import {
  processTailBatch,
  createCorrelationDedupState,
  filterNewCorrelations,
  type BackendSummary,
  type CorrelationDedupState,
  type LedgerEventWire,
} from "@/lib/ledger-tail-extract";
import { selectToastContent } from "@/lib/backend-toast-content";
import { narrateOperation, type OperationNarrative } from "@/lib/narrator";
import { useUIStore } from "@/stores/ui-store";

/** Default poll interval in milliseconds. Four seconds balances
 *  latency (users see backend changes within ~4s) against load
 *  (15 queries/minute is imperceptible). */
const DEFAULT_POLL_MS = 4000;

/** Max events fetched per poll. 50 is comfortably larger than the
 *  burst rate of any single engine under normal operation. */
const BATCH_LIMIT = 50;

/** Iter 21/22/23: compose a user-friendly toast describing a
 *  batch of backend mutations and hand it to the ui-store toast
 *  surface. Iter 23 upgrades the text selection to use the Rust
 *  narrator when available.
 *
 *  Narrator path (iter 23):
 *    - Only fires when exactly one new correlation is in the
 *      batch (narrating one of many would under-represent the
 *      tick, and narrating all would be wasteful).
 *    - Falls back gracefully on IPC failure \u2014 `narrateOperation`
 *      already returns a fallback object with `event_count: 0`
 *      which the pure helper detects and routes to the aggregate
 *      path.
 *
 *  Aggregate path (iter 21/22 fallback):
 *    - `count === 1` \u2192 show `singleSummary`
 *    - `count > 1` \u2192 "N background operations"
 *
 *  Content selection is split into a pure helper
 *  (`selectToastContent`) so it's unit-testable without mocking
 *  React or Tauri. This function only handles side effects: the
 *  async narrator fetch and the Zustand toast write.
 *
 *  Uses the `addStructuredError` channel even though these are
 *  success messages, because that's the single unified toast
 *  surface in the codebase (iter-17 architecture). */
async function emitBackendSummaryToast(
  summary: BackendSummary,
  newCorrelations: string[],
): Promise<void> {
  // Only fetch a narrative when narrating actually changes the
  // output \u2014 i.e., there's exactly one new correlation. Multi-
  // correlation ticks skip the IPC entirely.
  let narrative: OperationNarrative | null = null;
  if (newCorrelations.length === 1) {
    narrative = await narrateOperation(newCorrelations[0]);
  }
  const content = selectToastContent(summary, newCorrelations, narrative);
  useUIStore.getState().addStructuredError({
    what: content.what,
    why: content.why,
    appDid:
      "Auto-refreshed any pane viewing the affected paths so the file list stays fresh",
    userAction:
      "Open the Activity Timeline (\u2318\u21E7Y) for a full correlation trace",
  });
}

/**
 * Mount-once hook that starts a background ledger-tail poll. Call
 * from the file-manager orchestrator so the polling runs as long
 * as the app is open. Returns nothing \u2014 all side effects are
 * dispatched via `dispatchRefresh` which the existing pane
 * listeners already consume.
 */
export function useLedgerTailPoll(pollMs: number = DEFAULT_POLL_MS): void {
  // Watermark persists across ticks. `useRef` is intentional: we
  // never want React to re-render when the watermark advances.
  const watermarkRef = useRef<string | null>(null);
  // Iter 22: correlation-aware dedup ring. A long-running backend
  // operation (60s sync) emits events across ~15 poll ticks, all
  // sharing the same correlation_id. Without dedup each tick fires
  // a toast, spamming the user with 15 duplicates. The ring
  // remembers recently-toasted correlations so only genuinely new
  // logical operations produce toasts.
  const dedupStateRef = useRef<CorrelationDedupState>(
    createCorrelationDedupState(),
  );

  useEffect(() => {
    if (!isTauriAvailable()) {
      // Browser dev mode: polling would be pointless because the
      // ledger IPC returns an empty list via the fail-soft path.
      // Skip entirely so we don't spin a no-op interval.
      return;
    }

    let cancelled = false;

    async function tick() {
      if (cancelled) return;
      // Pause polling while the document is hidden. The user can't
      // see stale state on a hidden tab, so burning CPU / battery
      // polling is pure waste.
      if (
        typeof document !== "undefined" &&
        document.visibilityState === "hidden"
      ) {
        return;
      }
      const events = await tauriInvokeSafe<LedgerEventWire[]>(
        "ledger_recent",
        { limit: BATCH_LIMIT },
        [],
      );
      if (cancelled) return;
      const result = processTailBatch(events, watermarkRef.current);
      watermarkRef.current = result.newWatermark;
      if (result.affectedPaths.length > 0) {
        dispatchRefresh(result.affectedPaths);
      }
      // Iter 21 + 22: aggregate backend mutations (non-fs engines)
      // into one toast per tick — but ALSO dedup by correlation_id
      // so a long-running logical operation that spans many ticks
      // fires a single toast, not one per tick. The toast contract:
      //
      //   - exactly one toast per tick at most
      //   - no toast on the first tick (startup contract)
      //   - no toast for error / running events
      //   - no toast when every correlation_id in the batch has
      //     already been surfaced in a recent tick (iter 22 dedup)
      //   - single-event specificity when count === 1; aggregate
      //     headline otherwise
      //
      // Events with null correlation_id are "undedupable" and
      // always emit their toast — rare in practice because every
      // backend engine sets a stable correlation per operation
      // (transfer job_id, sync pair_id, automation rule_id).
      if (result.backendSummary) {
        const summary = result.backendSummary;
        if (summary.correlationIds.length === 0) {
          // No correlations at all \u2014 fall through to the toast
          // unconditionally (legacy iter-21 behaviour). Pass an
          // empty new-correlations list so the narrator path is
          // skipped and the pure helper uses the aggregate text.
          await emitBackendSummaryToast(summary, []);
        } else {
          const filter = filterNewCorrelations(
            dedupStateRef.current,
            summary.correlationIds,
          );
          dedupStateRef.current = filter.nextState;
          if (filter.newCorrelations.length > 0) {
            await emitBackendSummaryToast(summary, filter.newCorrelations);
          }
        }
      }
    }

    // Kick off the first tick immediately so the watermark seeds
    // before any events fire. `processTailBatch` guarantees the
    // first tick fires zero refreshes regardless.
    void tick();
    const interval = window.setInterval(() => {
      void tick();
    }, pollMs);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [pollMs]);
}
