/**
 * Iter-20 tests: ledger tail extraction pure helpers.
 *
 * Covers every edge case of `processTailBatch` so the polling hook
 * can trust the extraction layer completely:
 *
 *   - First tick seeds the watermark without firing (no startup
 *     refresh cascade)
 *   - Empty batch returns empty affectedPaths and preserves watermark
 *   - Watermark unchanged when newest event matches current watermark
 *   - Mutations newer than watermark yield their paths
 *   - Read-only events (`list`, `query`, unknown kinds) are filtered
 *   - Error / running events are filtered (nothing actually changed)
 *   - Deduplication within a batch (same path from two events)
 *   - Subject + target both collected (for copy/move)
 *   - Missing subject/target (null) handled safely
 *   - Oldest-first ordering within the batch
 *   - Watermark advances to newest event id, not oldest
 *
 * Pure unit tests: no React, no Tauri, no timers.
 */
import { describe, it, expect } from "vitest";
import {
  processTailBatch,
  isMutationKind,
  MUTATION_KINDS,
  createCorrelationDedupState,
  filterNewCorrelations,
  DEFAULT_DEDUP_CAPACITY,
  type LedgerEventWire,
} from "../lib/ledger-tail-extract";

function ev(
  id: string,
  kind: string,
  subject: string | null,
  target: string | null = null,
  status: string = "ok",
  engine: string = "fs",
  correlationId: string | null = null,
): LedgerEventWire {
  return {
    id,
    occurred_at: new Date().toISOString(),
    engine,
    kind,
    status,
    subject_path: subject,
    target_path: target,
    bytes: null,
    correlation_id: correlationId,
    summary: `${engine}/${kind}: ${subject ?? "?"}`,
    details_json: "{}",
    undo_token: null,
  };
}

describe("isMutationKind", () => {
  it("accepts every fs mutation kind", () => {
    for (const k of [
      "copy",
      "move",
      "delete",
      "purge",
      "rename",
      "duplicate",
      "create_file",
      "create_folder",
      "create_directory",
      "create_symlink",
      "set_permissions",
    ]) {
      expect(isMutationKind(k)).toBe(true);
    }
  });

  it("accepts backend engine kinds (transfer/sync/automation)", () => {
    expect(isMutationKind("transfer")).toBe(true);
    expect(isMutationKind("sync_pull")).toBe(true);
    expect(isMutationKind("automation_run")).toBe(true);
  });

  it("rejects read-only and unknown kinds", () => {
    expect(isMutationKind("list")).toBe(false);
    expect(isMutationKind("query")).toBe(false);
    expect(isMutationKind("info")).toBe(false);
    expect(isMutationKind("open")).toBe(false);
    expect(isMutationKind("some_new_kind_nobody_added")).toBe(false);
  });

  it("has a non-empty immutable kind set", () => {
    expect(MUTATION_KINDS.size).toBeGreaterThan(5);
  });
});

describe("processTailBatch", () => {
  it("returns empty + preserved watermark on empty input", () => {
    const out = processTailBatch([], "existing-watermark");
    expect(out.affectedPaths).toEqual([]);
    expect(out.newWatermark).toBe("existing-watermark");
    expect(out.backendSummary).toBeNull();
  });

  it("first tick (null watermark) seeds watermark WITHOUT firing refreshes or toasts", () => {
    const events = [
      ev("new-1", "copy", "/a", "/b"),
      ev("old-1", "move", "/c", "/d"),
    ];
    const out = processTailBatch(events, null);
    // Critical: no paths dispatched on the very first tick.
    expect(out.affectedPaths).toEqual([]);
    // But the watermark is set so the next tick has a baseline.
    expect(out.newWatermark).toBe("new-1");
    // And no toast either \u2014 historical backend events on startup
    // must not surprise the user with "X things happened while you
    // were away" (iter 21 explicit guarantee).
    expect(out.backendSummary).toBeNull();
  });

  it("watermark unchanged when newest event matches watermark", () => {
    const events = [ev("same", "copy", "/a", "/b")];
    const out = processTailBatch(events, "same");
    expect(out.affectedPaths).toEqual([]);
    expect(out.newWatermark).toBe("same");
  });

  it("collects paths from mutations newer than the watermark", () => {
    // DESC order: newest first.
    const events = [
      ev("new-2", "move", "/dest/file.txt", "/dest"),
      ev("new-1", "copy", "/src/a.txt", "/dest/a.txt"),
      ev("old-1", "delete", "/old/thing"),
    ];
    const out = processTailBatch(events, "old-1");
    // Walked in reverse (oldest-new first). new-1 came before new-2.
    expect(out.affectedPaths).toContain("/src/a.txt");
    expect(out.affectedPaths).toContain("/dest/a.txt");
    expect(out.affectedPaths).toContain("/dest/file.txt");
    expect(out.affectedPaths).toContain("/dest");
    // /old/thing was at/before the watermark \u2014 must NOT appear.
    expect(out.affectedPaths).not.toContain("/old/thing");
    expect(out.newWatermark).toBe("new-2");
  });

  it("filters out read-only kinds", () => {
    const events = [
      ev("new-2", "list", "/dir"),
      ev("new-1", "copy", "/src", "/dst"),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    // "list" kind is a read \u2014 does NOT fire refresh.
    expect(out.affectedPaths).not.toContain("/dir");
    expect(out.affectedPaths).toEqual(["/src", "/dst"]);
  });

  it("filters out error events", () => {
    const events = [
      ev("new", "copy", "/failed-src", "/failed-dst", "error"),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    // Error events didn't actually change files.
    expect(out.affectedPaths).toEqual([]);
    // Watermark still advances so we don't re-consider this event.
    expect(out.newWatermark).toBe("new");
  });

  it("filters out running / in-progress events", () => {
    const events = [
      ev("new", "transfer", "/src", "/dst", "running"),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.affectedPaths).toEqual([]);
  });

  it("deduplicates paths within a batch", () => {
    const events = [
      ev("new-3", "copy", "/shared", "/other"),
      ev("new-2", "move", "/shared", "/another"),
      ev("new-1", "delete", "/shared"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    const shared = out.affectedPaths.filter((p) => p === "/shared");
    expect(shared).toHaveLength(1);
  });

  it("collects BOTH subject and target paths (copy/move need both)", () => {
    const events = [
      ev("new-1", "copy", "/src/file.txt", "/dst/file.txt"),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.affectedPaths).toContain("/src/file.txt");
    expect(out.affectedPaths).toContain("/dst/file.txt");
  });

  it("handles events with missing subject or target safely", () => {
    const events = [
      ev("new-2", "create_folder", "/new-dir", null),
      ev("new-1", "delete", null, "/deleted"),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.affectedPaths).toContain("/new-dir");
    expect(out.affectedPaths).toContain("/deleted");
    expect(out.affectedPaths).not.toContain(null);
  });

  it("skips events at and beyond the watermark even if they look newer", () => {
    // Simulate a tiny race: watermark points to an event in the
    // middle of the batch (the caller already processed it).
    const events = [
      ev("new-1", "copy", "/fresh"),
      ev("WATERMARK", "move", "/already-processed"),
      ev("old-1", "move", "/very-old"),
    ];
    const out = processTailBatch(events, "WATERMARK");
    expect(out.affectedPaths).toEqual(["/fresh"]);
    expect(out.affectedPaths).not.toContain("/already-processed");
    expect(out.affectedPaths).not.toContain("/very-old");
    expect(out.newWatermark).toBe("new-1");
  });

  it("preserves oldest-first order within the new batch", () => {
    const events = [
      ev("new-3", "copy", "/third"),
      ev("new-2", "copy", "/second"),
      ev("new-1", "copy", "/first"),
      ev("old", "move", "/x"),
    ];
    const out = processTailBatch(events, "old");
    // Walked DESC list in reverse: first, second, third.
    expect(out.affectedPaths).toEqual(["/first", "/second", "/third"]);
  });
});

describe("processTailBatch backendSummary (iter 21)", () => {
  it("returns null when there are no backend events in the batch", () => {
    const events = [
      ev("new-1", "copy", "/src", "/dst"),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    // Only fs events \u2014 no toast should fire.
    expect(out.backendSummary).toBeNull();
  });

  it("fires backendSummary with count=1 + singleSummary when exactly one backend event is new", () => {
    const events = [
      ev(
        "new-1",
        "transfer_complete",
        "/remote/foo.bin",
        "/local/backup",
        "ok",
        "transfer",
      ),
      ev("old", "move", "/a", "/b"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.backendSummary).not.toBeNull();
    expect(out.backendSummary?.count).toBe(1);
    expect(out.backendSummary?.engines).toEqual(["transfer"]);
    expect(out.backendSummary?.singleSummary).toContain("transfer");
  });

  it("aggregates multiple backend events; singleSummary is null when count > 1", () => {
    const events = [
      ev("new-3", "sync_push", "/a", null, "ok", "sync"),
      ev("new-2", "automation_run", "/b", null, "ok", "automation"),
      ev("new-1", "transfer_complete", "/c", "/d", "ok", "transfer"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.backendSummary?.count).toBe(3);
    // Distinct engines in first-seen order (oldest-first after
    // reverse walk: transfer, automation, sync).
    expect(out.backendSummary?.engines).toEqual([
      "transfer",
      "automation",
      "sync",
    ]);
    // Single summary must be null when there are multiple events
    // \u2014 hook falls back to the aggregate toast.
    expect(out.backendSummary?.singleSummary).toBeNull();
  });

  it("fs and non-fs events in the same batch: fs is NOT counted in the backendSummary", () => {
    const events = [
      ev("new-3", "copy", "/a", "/b", "ok", "fs"), // frontend \u2014 excluded
      ev("new-2", "sync_pull", "/remote", "/local", "ok", "sync"),
      ev("new-1", "copy", "/c", "/d", "ok", "fs"), // excluded
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    // backendSummary only counts the one sync event.
    expect(out.backendSummary?.count).toBe(1);
    expect(out.backendSummary?.engines).toEqual(["sync"]);
    // affectedPaths still includes paths from fs events (they
    // need their own refresh on the rare case the frontend
    // handler forgot to dispatch).
    expect(out.affectedPaths).toContain("/a");
    expect(out.affectedPaths).toContain("/remote");
  });

  it("error and running backend events do NOT contribute to backendSummary", () => {
    const events = [
      ev("new-2", "transfer_complete", "/a", null, "error", "transfer"),
      ev("new-1", "sync_push", "/b", null, "running", "sync"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    // Neither event contributes because they didn't actually
    // change files. affectedPaths is empty and backendSummary
    // is null \u2014 the user should NOT be told about failures or
    // in-progress work via the iter-21 toast.
    expect(out.affectedPaths).toEqual([]);
    expect(out.backendSummary).toBeNull();
  });

  it("deduplicates engine names within a burst", () => {
    const events = [
      ev("new-3", "sync_push", "/a", null, "ok", "sync"),
      ev("new-2", "sync_push", "/b", null, "ok", "sync"),
      ev("new-1", "sync_push", "/c", null, "ok", "sync"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.backendSummary?.count).toBe(3);
    // Engine list deduplicated \u2014 "sync" appears once even though
    // three sync events fired.
    expect(out.backendSummary?.engines).toEqual(["sync"]);
  });

  it("first tick (null watermark) returns backendSummary: null even if there are backend events", () => {
    const events = [
      ev("new-1", "transfer_complete", "/a", null, "ok", "transfer"),
    ];
    const out = processTailBatch(events, null);
    // Iter 21 contract: no toast on the first tick for historical
    // events \u2014 same reasoning as no affectedPaths.
    expect(out.backendSummary).toBeNull();
  });
});

describe("processTailBatch correlationIds (iter 22)", () => {
  it("collects distinct correlation_ids from backend events in first-seen order", () => {
    const events = [
      // DESC order: newest first. Walk-in-reverse yields old\u2192new:
      // job-A first, then job-B, then job-C.
      ev("new-3", "transfer_complete", "/c", null, "ok", "transfer", "job-C"),
      ev("new-2", "transfer_complete", "/b", null, "ok", "transfer", "job-B"),
      ev("new-1", "transfer_complete", "/a", null, "ok", "transfer", "job-A"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.backendSummary?.correlationIds).toEqual([
      "job-A",
      "job-B",
      "job-C",
    ]);
  });

  it("deduplicates repeated correlation_ids within a single batch", () => {
    const events = [
      ev("new-3", "sync_push", "/c", null, "ok", "sync", "pair-1"),
      ev("new-2", "sync_push", "/b", null, "ok", "sync", "pair-1"),
      ev("new-1", "sync_push", "/a", null, "ok", "sync", "pair-1"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    // Three events sharing one correlation \u2014 collapses to one id.
    expect(out.backendSummary?.count).toBe(3);
    expect(out.backendSummary?.correlationIds).toEqual(["pair-1"]);
  });

  it("null correlation_id events are counted but NOT added to correlationIds", () => {
    const events = [
      ev("new-2", "sync_push", "/a", null, "ok", "sync", "pair-1"),
      ev("new-1", "sync_push", "/b", null, "ok", "sync", null),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    expect(out.backendSummary?.count).toBe(2);
    // Only the non-null correlation ends up in the list.
    expect(out.backendSummary?.correlationIds).toEqual(["pair-1"]);
  });

  it("fs events never contribute to correlationIds even when they have a correlation", () => {
    const events = [
      ev("new-2", "copy", "/a", "/b", "ok", "fs", "fs-undo-1"),
      ev("new-1", "transfer_complete", "/c", null, "ok", "transfer", "job-X"),
      ev("old", "move", "/x", "/y"),
    ];
    const out = processTailBatch(events, "old");
    // Only the backend correlation appears \u2014 fs-undo-1 is
    // excluded because fs events are handler-toasted already.
    expect(out.backendSummary?.correlationIds).toEqual(["job-X"]);
  });
});

describe("filterNewCorrelations (iter 22 dedup ring)", () => {
  it("empty input returns empty newCorrelations and unchanged state", () => {
    const state = createCorrelationDedupState();
    const result = filterNewCorrelations(state, []);
    expect(result.newCorrelations).toEqual([]);
    expect(result.nextState).toBe(state); // reference identity preserved
  });

  it("adds a fresh correlation and returns it as new", () => {
    const state = createCorrelationDedupState();
    const result = filterNewCorrelations(state, ["job-A"]);
    expect(result.newCorrelations).toEqual(["job-A"]);
    expect(result.nextState.seen).toEqual(["job-A"]);
  });

  it("returns empty newCorrelations when every id is already seen", () => {
    const state: ReturnType<typeof createCorrelationDedupState> = {
      seen: ["job-A", "job-B"],
      capacity: 32,
    };
    const result = filterNewCorrelations(state, ["job-A", "job-B"]);
    expect(result.newCorrelations).toEqual([]);
    // State unchanged \u2014 reference identity preserved for perf.
    expect(result.nextState).toBe(state);
  });

  it("returns only the new ids from a mixed batch", () => {
    const state: ReturnType<typeof createCorrelationDedupState> = {
      seen: ["job-A"],
      capacity: 32,
    };
    const result = filterNewCorrelations(state, [
      "job-A",
      "job-B",
      "job-C",
    ]);
    expect(result.newCorrelations).toEqual(["job-B", "job-C"]);
    expect(result.nextState.seen).toEqual(["job-A", "job-B", "job-C"]);
  });

  it("does NOT mutate the input state", () => {
    const state = createCorrelationDedupState();
    const before = [...state.seen];
    filterNewCorrelations(state, ["job-A", "job-B"]);
    expect(state.seen).toEqual(before);
  });

  it("evicts oldest correlations (FIFO) when capacity overflows", () => {
    const state: ReturnType<typeof createCorrelationDedupState> = {
      seen: ["old-1", "old-2", "old-3"],
      capacity: 3,
    };
    const result = filterNewCorrelations(state, ["new-1", "new-2"]);
    // Only the most recent 3 survive.
    expect(result.nextState.seen).toEqual(["old-3", "new-1", "new-2"]);
    expect(result.nextState.capacity).toBe(3);
  });

  it("handles repeated ids within a single incoming batch", () => {
    const state = createCorrelationDedupState();
    const result = filterNewCorrelations(state, [
      "job-A",
      "job-A",
      "job-A",
    ]);
    // Only one of the three is counted as new.
    expect(result.newCorrelations).toEqual(["job-A"]);
    expect(result.nextState.seen).toEqual(["job-A"]);
  });

  it("uses DEFAULT_DEDUP_CAPACITY (32) when none is specified", () => {
    const state = createCorrelationDedupState();
    expect(state.capacity).toBe(DEFAULT_DEDUP_CAPACITY);
    expect(DEFAULT_DEDUP_CAPACITY).toBe(32);
  });

  it("simulates iter-22 scenario: long-running sync fires one toast, not 15", () => {
    // Tick 1: sync pair-1 appears for the first time.
    let state = createCorrelationDedupState();
    const t1 = filterNewCorrelations(state, ["pair-1"]);
    expect(t1.newCorrelations).toEqual(["pair-1"]);
    state = t1.nextState;

    // Ticks 2-15: sync pair-1 keeps firing events. Each tick would
    // have produced a toast under iter 21, but iter 22 dedup
    // filters every one of them.
    for (let i = 0; i < 14; i++) {
      const filtered = filterNewCorrelations(state, ["pair-1"]);
      expect(filtered.newCorrelations).toEqual([]);
      state = filtered.nextState;
    }

    // Tick 16: sync pair-1 finally finishes and a NEW operation
    // (automation rule-2) starts \u2014 iter 22 correctly emits.
    const t16 = filterNewCorrelations(state, ["rule-2"]);
    expect(t16.newCorrelations).toEqual(["rule-2"]);
  });
});
