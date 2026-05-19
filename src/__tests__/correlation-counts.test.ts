/**
 * correlation-counts
 *
 * Locks the Activity Timeline's "+N more in this op" group-size helper.
 * The hint attaches to the newest row of every multi-row correlation
 * group, so the counts the renderer derives MUST exactly match what
 * the user sees in the timeline. These tests guard against off-by-one
 * regressions and accidental drift in the empty/null handling.
 */
import { describe, it, expect } from "vitest";
import { correlationCounts } from "@/lib/correlation-counts";
import type { LedgerEventWire as LedgerEvent } from "@/lib/ledger-tail-extract";

function makeEvent(overrides: Partial<LedgerEvent> = {}): LedgerEvent {
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

describe("correlationCounts", () => {
  it("returns an empty map for an empty input", () => {
    expect(correlationCounts([])).toEqual(new Map());
  });

  it("counts a single-event correlation as 1", () => {
    const events = [makeEvent({ correlation_id: "cor-A" })];
    expect(correlationCounts(events).get("cor-A")).toBe(1);
  });

  it("sums every event sharing a correlation_id", () => {
    const events = [
      makeEvent({ id: "1", correlation_id: "cor-A" }),
      makeEvent({ id: "2", correlation_id: "cor-A" }),
      makeEvent({ id: "3", correlation_id: "cor-A" }),
    ];
    expect(correlationCounts(events).get("cor-A")).toBe(3);
  });

  it("tracks multiple distinct correlation_ids independently", () => {
    const events = [
      makeEvent({ id: "1", correlation_id: "cor-A" }),
      makeEvent({ id: "2", correlation_id: "cor-B" }),
      makeEvent({ id: "3", correlation_id: "cor-A" }),
    ];
    const counts = correlationCounts(events);
    expect(counts.get("cor-A")).toBe(2);
    expect(counts.get("cor-B")).toBe(1);
  });

  it("skips events with no correlation_id", () => {
    const events = [
      makeEvent({ id: "1", correlation_id: null }),
      makeEvent({ id: "2", correlation_id: "cor-A" }),
    ];
    const counts = correlationCounts(events);
    expect(counts.size).toBe(1);
    expect(counts.get("cor-A")).toBe(1);
  });

  it("skips events with an empty correlation_id string", () => {
    // Belt-and-suspenders: the backend's serde shouldn't emit empty
    // strings, but the wire is dynamic enough that we don't want a
    // bogus event with `correlation_id: ""` to silently create a
    // phantom group key.
    const events = [
      makeEvent({ id: "1", correlation_id: "" }),
      makeEvent({ id: "2", correlation_id: "cor-A" }),
    ];
    const counts = correlationCounts(events);
    expect(counts.size).toBe(1);
    expect(counts.has("")).toBe(false);
  });

  it("preserves insertion order (first-seen correlation_id first)", () => {
    // Events arrive newest-first; iterating the result Map should
    // also be newest-first so the renderer's first-occurrence dedup
    // ("decorate only the newest row of each group") aligns with the
    // count keys.
    const events = [
      makeEvent({ id: "1", correlation_id: "cor-B" }),
      makeEvent({ id: "2", correlation_id: "cor-A" }),
      makeEvent({ id: "3", correlation_id: "cor-B" }),
    ];
    const counts = correlationCounts(events);
    expect(Array.from(counts.keys())).toEqual(["cor-B", "cor-A"]);
  });
});
