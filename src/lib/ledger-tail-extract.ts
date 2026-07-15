/**
 * Ledger tail extraction (iter 20).
 *
 * Pure helpers that turn a batch of `LedgerEvent`s returned by the
 * `ledger_recent` IPC into the set of paths the UI needs to refresh.
 *
 * Split into its own module (separate from the React hook that calls
 * it) so the extraction logic is unit-testable without mocking Tauri
 * or React. The hook keeps the effectful bits; this file stays pure.
 *
 * Design notes:
 *
 * - **Watermark by id**: callers track the id of the newest event
 *   they have already dispatched. On each poll, we only act on
 *   events strictly newer than that watermark, and we advance the
 *   watermark to the newest seen event. This avoids double-dispatch
 *   when poll intervals overlap with ledger writes.
 *
 * - **Path-only output**: we collect both `subject_path` and
 *   `target_path` (for ops like move/copy where both sides matter),
 *   filter out `null`s, and deduplicate. The caller passes the
 *   resulting list straight into `dispatchRefresh` — the iter-18
 *   helper handles the actual "does this affect my pane?" check.
 *
 * - **Kind filter**: ledger events include both MUTATIONS (copy,
 *   move, delete, ...) and READS (list, query, info). Only mutations
 *   should trigger a refresh. We filter via an allow-list of mutation
 *   kinds rather than a deny-list of read kinds so a new mutation
 *   kind added in the future defaults to "no refresh" until it's
 *   explicitly allowed \u2014 safer than accidentally firing refreshes
 *   on unrelated read events. The list is explicit for auditability.
 *
 * - **Status filter**: we skip `error` events because a failed op
 *   didn't actually change any files; refreshing would be pointless.
 *   `running` / `started` events also don't need refresh because
 *   nothing has landed yet; we wait for the `ok` follow-up event.
 */

import { FS_INTENT_KIND } from "@/lib/fs-kinds";

/** Wire format returned by `ledger_recent` \u2014 mirror of
 *  `LedgerEvent` in `src-tauri/src/ledger/mod.rs`. */
export interface LedgerEventWire {
  id: string;
  occurred_at: string;
  engine: string;
  kind: string;
  status: string;
  subject_path: string | null;
  target_path: string | null;
  bytes: number | null;
  correlation_id: string | null;
  summary: string;
  details_json: string;
  undo_token: string | null;
}

/**
 * Allow-list of ledger event kinds that the tail poller treats as
 * "something changed on disk". Covers every kind emitted by
 * `file_ops_commands::record_fs_ok` plus the engines that write to
 * the ledger from background tasks (transfer, sync, automation).
 *
 * Kept as a plain Set so callers can test membership in O(1). New
 * mutation kinds should be added here explicitly rather than
 * inferred \u2014 see the module doc for why.
 */
export const MUTATION_KINDS: ReadonlySet<string> = new Set([
  // File ops (frontend-initiated). The delete kinds come from the shared
  // source rather than being spelled here: this set previously listed
  // "delete"/"purge", which the engine has never written, so no delete ever
  // triggered a refresh.
  "copy",
  "move",
  FS_INTENT_KIND.deleteTrash,
  FS_INTENT_KIND.deletePermanent,
  "rename",
  "duplicate",
  "create_file",
  "create_folder",
  "create_directory",
  "create_symlink",
  "set_permissions",
  // Bulk operations
  "archive",
  "send_to",
  // Transfer engine (backend-initiated)
  "transfer",
  "transfer_complete",
  "upload",
  "download",
  // Sync engine (backend-initiated)
  "sync",
  "sync_pull",
  "sync_push",
  // Automation engine (rule-triggered)
  "automation_run",
  "rule_execute",
]);

/** Is this event kind one that actually changed files? */
export function isMutationKind(kind: string): boolean {
  return MUTATION_KINDS.has(kind);
}

// ────────────────────────────────────────────────
// Iter 22: correlation-aware toast dedup ring
// ────────────────────────────────────────────────

/** Bounded FIFO ring of recently-toasted correlation_ids. The hook
 *  owns a single instance via `useRef` and feeds it through
 *  `filterNewCorrelations` on every tick. Immutable API: updates
 *  return a new state object so React's reference equality works
 *  for debugging, though the hook stores it in a ref and doesn't
 *  re-render on mutation. */
export interface CorrelationDedupState {
  /** Already-toasted correlation_ids, oldest-first. Cap at
   *  `capacity` \u2014 when full, adding a new one evicts the oldest. */
  seen: string[];
  /** Maximum number of correlation_ids to remember. 32 is plenty
   *  for realistic burst rates: even if every backend engine
   *  fires a new correlation every 10 seconds, 32 covers ~5
   *  minutes of distinct operations. Beyond that the oldest
   *  correlation is unlikely to still be actively running, so
   *  re-toasting it (should it ever fire again) is acceptable. */
  capacity: number;
}

/** Default capacity for the dedup ring. Exported so tests can
 *  assert the value without re-declaring it. */
export const DEFAULT_DEDUP_CAPACITY = 32;

/** Create a fresh empty dedup state. Call once per hook mount;
 *  every tick should reuse the same state via `useRef`. */
export function createCorrelationDedupState(
  capacity: number = DEFAULT_DEDUP_CAPACITY,
): CorrelationDedupState {
  return { seen: [], capacity };
}

/** Result of filtering a batch of incoming correlation_ids
 *  against the dedup state. */
export interface CorrelationFilterResult {
  /** correlation_ids that are NEW relative to the dedup state.
   *  The hook fires a toast ONLY when this array is non-empty. */
  newCorrelations: string[];
  /** Updated dedup state with the new correlations appended and
   *  the oldest evicted to respect the capacity cap. */
  nextState: CorrelationDedupState;
}

/**
 * Filter `incoming` against a `CorrelationDedupState`, returning
 * the subset that's new and the advanced state. Pure: does not
 * mutate the input state.
 *
 * Behaviour:
 *   - Empty `incoming` \u2192 no new correlations, state unchanged
 *   - All `incoming` already in `state.seen` \u2192 empty
 *     `newCorrelations`, state unchanged
 *   - Mixed already-seen and new \u2192 returns only the new ones,
 *     advances the state
 *   - Capacity overflow \u2192 FIFO eviction (oldest first)
 */
export function filterNewCorrelations(
  state: CorrelationDedupState,
  incoming: string[],
): CorrelationFilterResult {
  if (incoming.length === 0) {
    return { newCorrelations: [], nextState: state };
  }
  const seenSet = new Set(state.seen);
  const newCorrelations: string[] = [];
  for (const id of incoming) {
    if (!seenSet.has(id)) {
      seenSet.add(id);
      newCorrelations.push(id);
    }
  }
  if (newCorrelations.length === 0) {
    return { newCorrelations: [], nextState: state };
  }
  // Append new correlations and evict the oldest if we've gone
  // over capacity. FIFO keeps the most recent N in the ring so
  // the most likely re-firing correlations stay deduped.
  const combined = [...state.seen, ...newCorrelations];
  const trimmed =
    combined.length > state.capacity
      ? combined.slice(combined.length - state.capacity)
      : combined;
  return {
    newCorrelations,
    nextState: { seen: trimmed, capacity: state.capacity },
  };
}

/** Summary of BACKEND mutations (non-fs engines) surfaced as a
 *  single aggregated toast per tick. Iter 21 uses this to tell
 *  the user WHY a pane just auto-refreshed \u2014 e.g. "sync engine
 *  pulled 3 files". `fs` engine events are excluded because they
 *  are frontend-initiated and already get a toast directly from
 *  the handler that issued them \u2014 double-toasting would be
 *  confusing. */
export interface BackendSummary {
  /** Total number of non-error backend mutation events in this
   *  batch. When 0, the hook should NOT fire any toast. */
  count: number;
  /** Distinct non-fs engine names that fired in this batch,
   *  first-seen order. Used to label the toast when `count > 1`. */
  engines: string[];
  /** When `count === 1`, the `summary` string from that single
   *  event \u2014 useful to show the user the specific thing that
   *  happened (e.g. "transfer: 12 MB → /backup/foo.bin"). When
   *  `count > 1`, null and the hook falls back to the aggregate
   *  toast text. */
  singleSummary: string | null;
  /** Iter 22: distinct correlation_ids from the backend events
   *  in this batch, in first-seen order. Used by the dedup ring
   *  in `use-ledger-tail-poll` to skip toasts for correlations
   *  already surfaced in an earlier tick. A long-running sync
   *  operation emits many events across many ticks, all sharing
   *  the same correlation (sync pair_id / transfer job_id /
   *  automation rule_id), so this field collapses them to a
   *  single toast per logical operation.
   *
   *  Events with null correlation_id are NOT included here;
   *  the hook treats them as "undedupable" and always emits. */
  correlationIds: string[];
}

/** Result of processing a batch of ledger events against a watermark. */
export interface TailBatchResult {
  /** Unique paths affected by mutations newer than the watermark,
   *  in first-seen order. Empty when nothing changed. */
  affectedPaths: string[];
  /** New watermark \u2014 the id of the newest event seen, or the
   *  previous watermark when the batch was empty. */
  newWatermark: string | null;
  /** Non-null only when at least one backend (non-fs) mutation
   *  event is new in this batch. The hook uses this to fire a
   *  single aggregated toast per tick explaining the
   *  auto-refresh. */
  backendSummary: BackendSummary | null;
}

/**
 * Process a batch of `LedgerEvent`s against the current watermark
 * and return the set of paths that need to be refreshed, plus the
 * advanced watermark.
 *
 * Pre-conditions:
 *   - `events` is the most-recent-first list returned by
 *     `ledger_recent` (the IPC already sorts by occurred_at DESC).
 *   - `watermark` is the id of the newest event processed in the
 *     previous tick, or `null` on the very first tick.
 *
 * Post-conditions:
 *   - When `events` is empty, returns `{ affectedPaths: [],
 *     newWatermark: watermark }` (unchanged).
 *   - When `watermark` is `null` (first tick), seeds the watermark
 *     to the newest event's id but returns `affectedPaths: []` so
 *     the initial poll does NOT fire a refresh cascade. This
 *     prevents the "every pane refreshes once at startup" glitch.
 *   - Events at or older than `watermark` are skipped (the
 *     comparison uses list order, which is monotonic because the
 *     backend sorts DESC).
 */
export function processTailBatch(
  events: LedgerEventWire[],
  watermark: string | null,
): TailBatchResult {
  if (events.length === 0) {
    return { affectedPaths: [], newWatermark: watermark, backendSummary: null };
  }
  // `events` is DESC-sorted, so events[0] is the newest.
  const newestId = events[0].id;
  if (watermark === null) {
    // First tick: seed the watermark without firing any refreshes.
    // This prevents the "everything refreshes on startup" glitch
    // where every historical event fires a path-aware refresh. No
    // toast on first tick for the same reason.
    return { affectedPaths: [], newWatermark: newestId, backendSummary: null };
  }
  if (watermark === newestId) {
    // No new events since the last tick.
    return { affectedPaths: [], newWatermark: watermark, backendSummary: null };
  }
  // Step 1: take events strictly newer than the watermark from
  // the DESC-sorted input. These are still in newest-first order.
  const newEvents: LedgerEventWire[] = [];
  for (const ev of events) {
    if (ev.id === watermark) break;
    newEvents.push(ev);
  }
  // Step 2: walk the new events in REVERSE (i.e. oldest-first)
  // so the resulting path list reads in chronological "order of
  // operations" order. Within each event we push subject before
  // target in their natural wire order so, e.g., a copy event
  // produces `[src, dst]` \u2014 predictable for tests and easier to
  // reason about when debugging.
  //
  // Ordering doesn't affect correctness (the consumer is
  // `shouldRefreshPane`, which only checks set membership), but
  // chronological order is strictly nicer for any future caller
  // that wants to iterate the list linearly or log it.
  //
  // Iter 21: also track backend (non-fs) mutations so the hook
  // can surface a single aggregated toast explaining the auto-
  // refresh. We compute this in the same loop to avoid a second
  // pass over `newEvents`.
  const seen = new Set<string>();
  const affected: string[] = [];
  const backendEngines: string[] = [];
  const backendEngineSet = new Set<string>();
  const backendCorrelationIds: string[] = [];
  const backendCorrelationIdSet = new Set<string>();
  let backendCount = 0;
  let firstBackendSummary: string | null = null;
  for (let i = newEvents.length - 1; i >= 0; i--) {
    const ev = newEvents[i];
    if (!isMutationKind(ev.kind)) continue;
    if (ev.status === "error" || ev.status === "running") continue;
    for (const p of [ev.subject_path, ev.target_path]) {
      if (p && !seen.has(p)) {
        seen.add(p);
        affected.push(p);
      }
    }
    // Iter 21: aggregate non-fs engines for the toast summary.
    // `fs` events are excluded because the frontend handler that
    // issued them already fired its own toast \u2014 double-toasting
    // would be confusing. Every other engine is a background
    // source the user has no other feedback channel for.
    if (ev.engine !== "fs") {
      backendCount += 1;
      if (firstBackendSummary === null) {
        firstBackendSummary = ev.summary;
      }
      if (!backendEngineSet.has(ev.engine)) {
        backendEngineSet.add(ev.engine);
        backendEngines.push(ev.engine);
      }
      // Iter 22: collect distinct correlation_ids so the hook's
      // dedup ring can skip toasts for logical operations already
      // surfaced in an earlier tick. Skip null correlations \u2014
      // they are undedupable and handled separately by the hook.
      if (
        ev.correlation_id &&
        !backendCorrelationIdSet.has(ev.correlation_id)
      ) {
        backendCorrelationIdSet.add(ev.correlation_id);
        backendCorrelationIds.push(ev.correlation_id);
      }
    }
  }
  const backendSummary: BackendSummary | null =
    backendCount > 0
      ? {
          count: backendCount,
          engines: backendEngines,
          // Only show the specific summary text when there's exactly
          // ONE backend event \u2014 aggregating summaries for bursts
          // would clutter the toast.
          singleSummary: backendCount === 1 ? firstBackendSummary : null,
          correlationIds: backendCorrelationIds,
        }
      : null;
  return {
    affectedPaths: affected,
    newWatermark: newestId,
    backendSummary,
  };
}
