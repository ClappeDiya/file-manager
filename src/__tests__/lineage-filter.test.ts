/**
 * Lineage Panel filter helper.
 *
 * Locks in the predicate semantics for the engine + failed-only chip
 * row added to the Lineage Panel. Same shape as the Activity Timeline
 * filter so users learn one mental model — these tests guard against
 * the two surfaces drifting apart.
 */
import { describe, it, expect } from "vitest";
import {
  filterLineageEvents,
  type LineageEvent,
} from "@/stores/lineage-store";

function makeEvent(overrides: Partial<LineageEvent> = {}): LineageEvent {
  return {
    id: "ev-1",
    occurred_at: "2026-05-14T10:00:00Z",
    engine: "fs",
    kind: "copy",
    status: "ok",
    subject_path: "/src/a.txt",
    target_path: "/dst/a.txt",
    bytes: 100,
    correlation_id: "cor-1",
    summary: "copy a.txt",
    details_json: "{}",
    undo_token: null,
    ...overrides,
  };
}

describe("filterLineageEvents", () => {
  it("returns the same array reference when no filter is active", () => {
    const events = [makeEvent(), makeEvent({ id: "ev-2" })];
    expect(filterLineageEvents(events, "all", false)).toBe(events);
  });

  it("filters by engine when a specific engine is selected", () => {
    const events = [
      makeEvent({ id: "fs-1", engine: "fs" }),
      makeEvent({ id: "sync-1", engine: "sync" }),
      makeEvent({ id: "fs-2", engine: "fs" }),
    ];
    const result = filterLineageEvents(events, "sync", false);
    expect(result).toHaveLength(1);
    expect(result[0]!.id).toBe("sync-1");
  });

  it("filters to failures only when failedOnly is true", () => {
    const events = [
      makeEvent({ id: "ok-1", status: "ok" }),
      makeEvent({ id: "failed-1", status: "failed" }),
      makeEvent({ id: "cancelled-1", status: "cancelled" }),
      makeEvent({ id: "ok-2", status: "ok" }),
    ];
    const result = filterLineageEvents(events, "all", true);
    expect(result.map((e) => e.id)).toEqual(["failed-1", "cancelled-1"]);
  });

  it("AND-composes engine + failedOnly", () => {
    // "Show me only sync failures" — the most common debugging query.
    const events = [
      makeEvent({ id: "fs-ok", engine: "fs", status: "ok" }),
      makeEvent({ id: "fs-fail", engine: "fs", status: "failed" }),
      makeEvent({ id: "sync-ok", engine: "sync", status: "ok" }),
      makeEvent({ id: "sync-fail", engine: "sync", status: "failed" }),
    ];
    const result = filterLineageEvents(events, "sync", true);
    expect(result).toHaveLength(1);
    expect(result[0]!.id).toBe("sync-fail");
  });

  it("includes every non-ok status under failedOnly (matches timeline semantic)", () => {
    // The Activity Timeline's "Failed only" chip excludes ONLY events
    // with `status === "ok"` — failed, cancelled, AND skipped all
    // surface under it. The lineage filter must match exactly so users
    // see the same set across both surfaces. (This differs from the
    // Retry-eligibility predicate in `ledger-event-flags`, which is
    // narrower because retrying a skip is meaningless.)
    const events = [
      makeEvent({ id: "ok-1", status: "ok" }),
      makeEvent({ id: "skipped-1", status: "skipped" }),
      makeEvent({ id: "failed-1", status: "failed" }),
      makeEvent({ id: "cancelled-1", status: "cancelled" }),
    ];
    const result = filterLineageEvents(events, "all", true);
    expect(result.map((e) => e.id)).toEqual([
      "skipped-1",
      "failed-1",
      "cancelled-1",
    ]);
  });

  it("returns empty when no event matches", () => {
    const events = [makeEvent({ engine: "fs" })];
    expect(filterLineageEvents(events, "sync", false)).toEqual([]);
  });

  it("preserves event order under filtering", () => {
    // Lineage events arrive newest-first from the backend. Filter must
    // not reorder them.
    const events = [
      makeEvent({ id: "a", engine: "fs", occurred_at: "2026-05-14T03:00Z" }),
      makeEvent({ id: "b", engine: "fs", occurred_at: "2026-05-14T02:00Z" }),
      makeEvent({ id: "c", engine: "fs", occurred_at: "2026-05-14T01:00Z" }),
    ];
    const result = filterLineageEvents(events, "fs", false);
    expect(result.map((e) => e.id)).toEqual(["a", "b", "c"]);
  });
});
