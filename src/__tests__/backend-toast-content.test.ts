/**
 * Iter-23 tests: narrator-backed toast content selection.
 *
 * Covers every branch of `selectToastContent`:
 *
 *   - Narrator path: single new correlation + real narrative
 *     -> use narrator headline + engines + duration
 *   - Fallback: narrator is null (caller didn't fetch one)
 *   - Fallback: narrator returned empty-trace (event_count === 0)
 *   - Fallback: narrator returned the Tauri-unavailable placeholder
 *     (also event_count === 0)
 *   - Multi-correlation tick: narrator path skipped even if
 *     narrative is present (we only narrate singletons)
 *   - Duration formatting: ms / s / m+s
 *   - Engine label falls back to summary.engines when narrative
 *     has empty engines_involved
 *
 * Pure unit tests \u2014 no React, no Tauri, no Zustand.
 */
import { describe, it, expect } from "vitest";
import {
  selectToastContent,
  isRealNarrative,
} from "../lib/backend-toast-content";
import type { BackendSummary } from "../lib/ledger-tail-extract";
import type { OperationNarrative } from "../lib/narrator";

function mkSummary(overrides: Partial<BackendSummary> = {}): BackendSummary {
  return {
    count: 1,
    engines: ["sync"],
    singleSummary: "sync/sync_push: /docs/foo.pdf",
    correlationIds: ["pair-1"],
    ...overrides,
  };
}

function mkNarrative(
  overrides: Partial<OperationNarrative> = {},
): OperationNarrative {
  return {
    correlation_id: "pair-1",
    event_count: 3,
    engines_involved: ["sync"],
    duration_ms: 245,
    headline: "Sync completed 3 files in 245ms",
    story: "Longer multi-sentence story goes here.",
    facts: [],
    ...overrides,
  };
}

describe("isRealNarrative", () => {
  it("returns false for null", () => {
    expect(isRealNarrative(null)).toBe(false);
  });

  it("returns false when event_count === 0 (empty trace or fallback)", () => {
    expect(isRealNarrative(mkNarrative({ event_count: 0 }))).toBe(false);
  });

  it("returns true for a narrative with events", () => {
    expect(isRealNarrative(mkNarrative({ event_count: 1 }))).toBe(true);
    expect(isRealNarrative(mkNarrative({ event_count: 42 }))).toBe(true);
  });
});

describe("selectToastContent", () => {
  describe("narrator path", () => {
    it("uses the narrator headline when there's exactly one new correlation + real narrative", () => {
      const summary = mkSummary();
      const narrative = mkNarrative({
        headline: "Sync completed 3 files in 245ms",
      });
      const content = selectToastContent(
        summary,
        ["pair-1"],
        narrative,
      );
      expect(content.what).toBe("Sync completed 3 files in 245ms");
    });

    it("includes engines + duration in the `why` line", () => {
      const summary = mkSummary();
      const narrative = mkNarrative({
        engines_involved: ["transfer"],
        duration_ms: 3200,
      });
      const content = selectToastContent(
        summary,
        ["pair-1"],
        narrative,
      );
      expect(content.why).toBe(
        "Background activity from transfer \u00b7 3.2s",
      );
    });

    it("falls back to summary.engines when narrative.engines_involved is empty", () => {
      const summary = mkSummary({ engines: ["automation"] });
      const narrative = mkNarrative({ engines_involved: [] });
      const content = selectToastContent(
        summary,
        ["rule-1"],
        narrative,
      );
      expect(content.why).toContain("automation");
    });

    it("omits duration when it's 0 (avoids '\u00b7 0ms' noise)", () => {
      const summary = mkSummary();
      const narrative = mkNarrative({ duration_ms: 0 });
      const content = selectToastContent(
        summary,
        ["pair-1"],
        narrative,
      );
      expect(content.why).not.toContain("0ms");
      expect(content.why).toBe("Background activity from sync");
    });
  });

  describe("aggregate fallback path", () => {
    it("uses singleSummary when narrative is null and count === 1", () => {
      const summary = mkSummary({ singleSummary: "transfer/done: /foo" });
      const content = selectToastContent(summary, ["pair-1"], null);
      expect(content.what).toBe("transfer/done: /foo");
    });

    it("uses the 'N background operations' headline when count > 1", () => {
      const summary = mkSummary({
        count: 5,
        singleSummary: null,
        engines: ["transfer", "sync"],
      });
      const content = selectToastContent(
        summary,
        ["job-1", "pair-2"],
        null,
      );
      expect(content.what).toBe("5 background operations");
    });

    it("pluralises correctly when count === 1 with no singleSummary", () => {
      const summary = mkSummary({ count: 1, singleSummary: null });
      const content = selectToastContent(summary, ["pair-1"], null);
      expect(content.what).toBe("1 background operation");
    });

    it("skips narrator path when there are multiple new correlations even if narrative is present", () => {
      const summary = mkSummary({
        count: 4,
        singleSummary: null,
        engines: ["transfer", "sync"],
      });
      const narrative = mkNarrative({
        headline: "would-be-wrong-to-show",
      });
      const content = selectToastContent(
        summary,
        ["job-1", "pair-1"],
        narrative,
      );
      // Narrator path skipped because 2 new correlations \u2192 we'd
      // under-represent the tick by narrating just one.
      expect(content.what).not.toBe("would-be-wrong-to-show");
      expect(content.what).toBe("4 background operations");
    });

    it("skips narrator path when narrative is the empty-trace fallback", () => {
      const summary = mkSummary();
      const emptyNarrative = mkNarrative({
        event_count: 0,
        headline: "No ledger events recorded for operation pair-1.",
      });
      const content = selectToastContent(
        summary,
        ["pair-1"],
        emptyNarrative,
      );
      // Empty trace \u2192 fall through to aggregate path.
      expect(content.what).not.toBe(
        "No ledger events recorded for operation pair-1.",
      );
      expect(content.what).toBe(summary.singleSummary);
    });

    it("skips narrator path when narrative is the Tauri-unavailable placeholder", () => {
      const summary = mkSummary();
      const unavailable = mkNarrative({
        event_count: 0,
        headline: "Narrator unavailable in preview mode.",
      });
      const content = selectToastContent(
        summary,
        ["pair-1"],
        unavailable,
      );
      expect(content.what).not.toContain("unavailable");
    });
  });

  describe("duration formatting", () => {
    it("formats sub-second as ms", () => {
      const content = selectToastContent(
        mkSummary(),
        ["pair-1"],
        mkNarrative({ duration_ms: 850 }),
      );
      expect(content.why).toContain("850ms");
    });

    it("formats seconds with one decimal", () => {
      const content = selectToastContent(
        mkSummary(),
        ["pair-1"],
        mkNarrative({ duration_ms: 3450 }),
      );
      expect(content.why).toContain("3.5s");
    });

    it("formats minutes + seconds for long operations", () => {
      const content = selectToastContent(
        mkSummary(),
        ["pair-1"],
        mkNarrative({ duration_ms: 72_500 }),
      );
      expect(content.why).toContain("1m 12s");
    });
  });
});
