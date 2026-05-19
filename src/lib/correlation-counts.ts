/**
 * correlation-counts
 *
 * Pure helper for the Activity Timeline's "+N more in this op" hint.
 * Given a list of ledger events, returns a Map<correlation_id, count>
 * so the renderer can decorate the first (newest) row of every
 * multi-row group with a count badge.
 *
 * Extracted here so it's unit-testable without rendering the panel and
 * doesn't trigger the react-refresh "fast refresh only works when a
 * file only exports components" warning that exporting from
 * `activity-timeline-panel.tsx` would cause.
 */
import type { LedgerEventWire as LedgerEvent } from "./ledger-tail-extract";

/**
 * Count how many events share each correlation_id in the input list.
 *
 * Events without a correlation_id (or with an empty string) are
 * excluded from the result — they have no group to belong to. The
 * returned Map's iteration order follows insertion order (first-seen
 * correlation_id first), matching the events array's newest-first
 * order from the ledger.
 */
export function correlationCounts(
  events: LedgerEvent[],
): Map<string, number> {
  const counts = new Map<string, number>();
  for (const ev of events) {
    const cid = ev.correlation_id;
    if (cid === null || cid === undefined || cid.length === 0) continue;
    counts.set(cid, (counts.get(cid) ?? 0) + 1);
  }
  return counts;
}
