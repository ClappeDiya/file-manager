/**
 * Iter 45 — Pure unit tests for `pickJumpRingTarget`.
 *
 * The Jump Ring keyboard gesture (⌘⇧L forward, ⌘⇧⌥L reverse)
 * routes through this pure function. Every decision edge case is
 * covered here so future iters can refactor the call site without
 * silently breaking ring semantics.
 *
 *   - Empty hits → empty target, ring reset
 *   - All-current-path hits → fallback to raw first hit (preserves
 *     iter 42's pre-ring "never silent" guarantee)
 *   - First press → returns distinct[0] at position 1
 *   - Rapid re-press forward within window → cycles index +1
 *   - Rapid re-press reverse within window → cycles index -1
 *     with modular wrap across 0 → length-1
 *   - Press after window expires → resets to head
 *   - Press with stale distinct list (top changed) → resets
 *   - Distinct dedup: repeated hit paths collapse to one slot
 *   - Current path filtering removes it wherever it appears
 *   - Cycle-window boundary (inclusive/exclusive) behaviour
 *
 * Pure, no React, no Tauri, no timers — `now` is injected.
 */
import { describe, it, expect } from "vitest";
import { FS_INTENT_KIND } from "@/lib/fs-kinds";
import {
  pickJumpRingTarget,
  JUMP_RING_CYCLE_WINDOW_MS,
  deriveLabel,
  labelForKind,
  formatRelativeTime,
  type JumpRingState,
} from "@/lib/ledger-dispatch";

function hit(path: string) {
  return { path };
}

function freshRing(): JumpRingState {
  return { index: 0, lastPressTime: 0, distinctPaths: [] };
}

describe("pickJumpRingTarget", () => {
  it("returns empty target and reset ring for empty hits", () => {
    const out = pickJumpRingTarget([], "/anywhere", freshRing(), "forward", 1000);
    expect(out.target).toBe("");
    expect(out.ringPosition).toBe(0);
    expect(out.ringSize).toBe(0);
    expect(out.nextRing.distinctPaths).toEqual([]);
    expect(out.nextRing.lastPressTime).toBe(1000);
  });

  it("falls back to the raw first hit when every hit is the current path", () => {
    const hits = [hit("/a"), hit("/a"), hit("/a")];
    const out = pickJumpRingTarget(hits, "/a", freshRing(), "forward", 1000);
    expect(out.target).toBe("/a");
    expect(out.ringPosition).toBe(1);
    expect(out.ringSize).toBe(1);
    // Ring resets in the fallback branch so the next press starts fresh.
    expect(out.nextRing.distinctPaths).toEqual([]);
  });

  it("filters out the current path on first press", () => {
    const hits = [hit("/a"), hit("/b"), hit("/c")];
    const out = pickJumpRingTarget(hits, "/a", freshRing(), "forward", 1000);
    expect(out.target).toBe("/b");
    expect(out.ringPosition).toBe(1);
    expect(out.ringSize).toBe(2);
    expect(out.nextRing.distinctPaths).toEqual(["/b", "/c"]);
    expect(out.nextRing.index).toBe(0);
  });

  it("dedupes repeated hit paths", () => {
    const hits = [hit("/b"), hit("/b"), hit("/c"), hit("/b")];
    const out = pickJumpRingTarget(hits, "/home", freshRing(), "forward", 1000);
    expect(out.nextRing.distinctPaths).toEqual(["/b", "/c"]);
    expect(out.ringSize).toBe(2);
  });

  it("cycles forward on rapid re-press within the window", () => {
    const hits = [hit("/b"), hit("/c"), hit("/d")];
    const ring: JumpRingState = {
      index: 0,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c", "/d"],
    };
    const out = pickJumpRingTarget(hits, "/home", ring, "forward", 1500);
    expect(out.target).toBe("/c");
    expect(out.ringPosition).toBe(2);
    expect(out.nextRing.index).toBe(1);
  });

  it("cycles forward with modular wrap at end of ring", () => {
    const hits = [hit("/b"), hit("/c"), hit("/d")];
    const ring: JumpRingState = {
      index: 2,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c", "/d"],
    };
    const out = pickJumpRingTarget(hits, "/home", ring, "forward", 1500);
    expect(out.target).toBe("/b");
    expect(out.ringPosition).toBe(1);
    expect(out.nextRing.index).toBe(0);
  });

  it("cycles reverse on rapid re-press within the window", () => {
    const hits = [hit("/b"), hit("/c"), hit("/d")];
    const ring: JumpRingState = {
      index: 1,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c", "/d"],
    };
    const out = pickJumpRingTarget(hits, "/home", ring, "reverse", 1500);
    expect(out.target).toBe("/b");
    expect(out.ringPosition).toBe(1);
    expect(out.nextRing.index).toBe(0);
  });

  it("cycles reverse with modular wrap at start of ring", () => {
    const hits = [hit("/b"), hit("/c"), hit("/d")];
    const ring: JumpRingState = {
      index: 0,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c", "/d"],
    };
    const out = pickJumpRingTarget(hits, "/home", ring, "reverse", 1500);
    expect(out.target).toBe("/d");
    expect(out.ringPosition).toBe(3);
    expect(out.nextRing.index).toBe(2);
  });

  it("resets to the head when the cycle window has expired", () => {
    const hits = [hit("/b"), hit("/c"), hit("/d")];
    const ring: JumpRingState = {
      index: 2,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c", "/d"],
    };
    const out = pickJumpRingTarget(
      hits,
      "/home",
      ring,
      "forward",
      1000 + JUMP_RING_CYCLE_WINDOW_MS + 1,
    );
    expect(out.target).toBe("/b");
    expect(out.ringPosition).toBe(1);
    expect(out.nextRing.index).toBe(0);
  });

  it("treats the exact window boundary as expired (reset)", () => {
    const hits = [hit("/b"), hit("/c")];
    const ring: JumpRingState = {
      index: 0,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c"],
    };
    const out = pickJumpRingTarget(
      hits,
      "/home",
      ring,
      "forward",
      1000 + JUMP_RING_CYCLE_WINDOW_MS,
    );
    expect(out.target).toBe("/b");
    expect(out.nextRing.index).toBe(0);
  });

  it("resets when a new distinct top appears between presses", () => {
    const hits = [hit("/x"), hit("/c"), hit("/d")];
    const ring: JumpRingState = {
      index: 1,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c", "/d"],
    };
    const out = pickJumpRingTarget(hits, "/home", ring, "forward", 1500);
    // Fresh ledger event pushed "/x" to the top, so the ring resets.
    expect(out.target).toBe("/x");
    expect(out.ringPosition).toBe(1);
    expect(out.nextRing.distinctPaths).toEqual(["/x", "/c", "/d"]);
  });

  it("skips the current path wherever it appears in the hit list", () => {
    const hits = [hit("/a"), hit("/c"), hit("/a"), hit("/d")];
    const out = pickJumpRingTarget(hits, "/a", freshRing(), "forward", 1000);
    expect(out.nextRing.distinctPaths).toEqual(["/c", "/d"]);
    expect(out.target).toBe("/c");
  });

  it("injected cycleWindowMs controls the reset decision", () => {
    const hits = [hit("/b"), hit("/c")];
    const ring: JumpRingState = {
      index: 0,
      lastPressTime: 1000,
      distinctPaths: ["/b", "/c"],
    };
    // With a narrower 500ms window, elapsed 600ms → reset.
    const out = pickJumpRingTarget(
      hits,
      "/home",
      ring,
      "forward",
      1600,
      500,
    );
    expect(out.target).toBe("/b");
    expect(out.nextRing.index).toBe(0);
  });

  it("reverse direction on a single-slot distinct list stays on that slot", () => {
    const hits = [hit("/b")];
    const ring: JumpRingState = {
      index: 0,
      lastPressTime: 1000,
      distinctPaths: ["/b"],
    };
    const out = pickJumpRingTarget(hits, "/home", ring, "reverse", 1500);
    expect(out.target).toBe("/b");
    expect(out.ringPosition).toBe(1);
    expect(out.nextRing.index).toBe(0);
  });
});

// ──────────────────────────────────────────────────────────────
// Iter 47 — Pure helper coverage
// Every helper below drives a user-facing surface (breadcrumb
// labels, badge strings, toast verbs, relative timestamps).
// Single-character typos here would propagate to Instant Jump,
// Jump Ring, Activity Timeline, and the operation-preview toast
// simultaneously, so lock each branch down with a test.
// ──────────────────────────────────────────────────────────────

describe("deriveLabel", () => {
  it("returns the last segment of a POSIX path", () => {
    expect(deriveLabel("/a/b/c.txt")).toBe("c.txt");
  });

  it("handles trailing slashes", () => {
    expect(deriveLabel("/a/b/")).toBe("b");
  });

  it("handles a bare filename with no separators", () => {
    expect(deriveLabel("readme.md")).toBe("readme.md");
  });

  it("handles Windows-style backslash separators", () => {
    expect(deriveLabel("C:\\Users\\md\\notes.md")).toBe("notes.md");
  });

  it("handles mixed POSIX + Windows separators", () => {
    expect(deriveLabel("C:\\Users/md\\docs/report.pdf")).toBe("report.pdf");
  });

  it("returns the original path for the root", () => {
    // "/" splits into an empty array; fallback returns the input.
    expect(deriveLabel("/")).toBe("/");
  });

  it("returns the empty string for empty input", () => {
    expect(deriveLabel("")).toBe("");
  });

  it("handles a folder path with a single segment", () => {
    expect(deriveLabel("/Projects")).toBe("Projects");
  });
});

describe("labelForKind", () => {
  it("maps edit_text to 'edited' with amber tone", () => {
    const { label, tone } = labelForKind("edit_text");
    expect(label).toBe("edited");
    expect(tone).toContain("amber");
  });

  it("maps copy to 'copied'", () => {
    expect(labelForKind("copy").label).toBe("copied");
  });

  it("maps move to 'moved'", () => {
    expect(labelForKind("move").label).toBe("moved");
  });

  it("maps rename to 'renamed'", () => {
    expect(labelForKind("rename").label).toBe("renamed");
  });

  it("maps duplicate to 'duplicated'", () => {
    expect(labelForKind("duplicate").label).toBe("duplicated");
  });

  it("maps the engine's delete kinds to plain language", () => {
    // "delete" is not a kind the engine writes; delete_trash used to fall
    // through to the default branch and render as the raw "delete trash".
    expect(labelForKind(FS_INTENT_KIND.deleteTrash).label).toBe("deleted");
    expect(labelForKind(FS_INTENT_KIND.deletePermanent).label).toBe(
      "permanently deleted",
    );
  });

  it("never renders a delete kind as its raw identifier", () => {
    for (const kind of [
      FS_INTENT_KIND.deleteTrash,
      FS_INTENT_KIND.deletePermanent,
    ]) {
      expect(labelForKind(kind).label).not.toContain("_");
      expect(labelForKind(kind).label).not.toContain("trash");
    }
  });

  it("maps both create_file and create_folder to 'created'", () => {
    expect(labelForKind("create_file").label).toBe("created");
    expect(labelForKind("create_folder").label).toBe("created");
  });

  it("maps transfer and transfer_ok to 'transferred'", () => {
    expect(labelForKind("transfer").label).toBe("transferred");
    expect(labelForKind("transfer_ok").label).toBe("transferred");
  });

  it("maps sync and sync_ok to 'synced'", () => {
    expect(labelForKind("sync").label).toBe("synced");
    expect(labelForKind("sync_ok").label).toBe("synced");
  });

  it("falls through unknown kinds with underscores replaced by spaces", () => {
    const { label } = labelForKind("future_operation");
    expect(label).toBe("future operation");
  });

  it("returns a neutral tone class for unknown kinds", () => {
    const { tone } = labelForKind("future_operation");
    expect(tone).toContain("white");
  });

  it("is pure and idempotent for repeated calls", () => {
    const a = labelForKind("edit_text");
    const b = labelForKind("edit_text");
    expect(a.label).toBe(b.label);
    expect(a.tone).toBe(b.tone);
  });
});

describe("formatRelativeTime", () => {
  // Pin "now" to 2026-04-11T12:00:00Z across every test so the
  // relative math is deterministic. All reference times are in
  // the past.
  const NOW_MS = Date.parse("2026-04-11T12:00:00Z");

  it("returns seconds for sub-minute deltas (ISO format)", () => {
    const tenSecondsAgo = new Date(NOW_MS - 10_000).toISOString();
    expect(formatRelativeTime(tenSecondsAgo, NOW_MS)).toBe("10s");
  });

  it("returns minutes for sub-hour deltas", () => {
    const fiveMinutesAgo = new Date(NOW_MS - 5 * 60 * 1000).toISOString();
    expect(formatRelativeTime(fiveMinutesAgo, NOW_MS)).toBe("5m");
  });

  it("returns hours for sub-day deltas", () => {
    const threeHoursAgo = new Date(NOW_MS - 3 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(threeHoursAgo, NOW_MS)).toBe("3h");
  });

  it("returns days for multi-day deltas", () => {
    const fiveDaysAgo = new Date(NOW_MS - 5 * 24 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(fiveDaysAgo, NOW_MS)).toBe("5d");
  });

  it("parses SQLite space-separated UTC format", () => {
    // Equivalent to 2026-04-11T11:55:00Z = 5 minutes ago.
    expect(formatRelativeTime("2026-04-11 11:55:00", NOW_MS)).toBe("5m");
  });

  it("falls back to the raw input when parsing fails", () => {
    const bad = "not-a-timestamp";
    expect(formatRelativeTime(bad, NOW_MS)).toBe(bad);
  });

  it("is deterministic when `now` is pinned (test stability)", () => {
    const a = formatRelativeTime("2026-04-11T11:58:00Z", NOW_MS);
    const b = formatRelativeTime("2026-04-11T11:58:00Z", NOW_MS);
    expect(a).toBe(b);
  });

  it("handles the minute boundary exactly", () => {
    // 60 seconds ago → Math.round → 60s → 60 / 60 = 1m (promotes).
    const sixtySecondsAgo = new Date(NOW_MS - 60_000).toISOString();
    expect(formatRelativeTime(sixtySecondsAgo, NOW_MS)).toBe("1m");
  });

  it("handles the hour boundary exactly", () => {
    const sixtyMinutesAgo = new Date(NOW_MS - 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(sixtyMinutesAgo, NOW_MS)).toBe("1h");
  });

  it("handles the day boundary exactly", () => {
    const twentyFourHoursAgo = new Date(
      NOW_MS - 24 * 60 * 60 * 1000,
    ).toISOString();
    expect(formatRelativeTime(twentyFourHoursAgo, NOW_MS)).toBe("1d");
  });
});

describe("formatRelativeTime — withSuffix style", () => {
  const NOW_MS = Date.parse("2026-04-11T12:00:00Z");

  it("appends 'ago' to seconds", () => {
    const ts = new Date(NOW_MS - 10_000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withSuffix")).toBe("10s ago");
  });

  it("appends 'ago' to minutes", () => {
    const ts = new Date(NOW_MS - 5 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withSuffix")).toBe("5m ago");
  });

  it("appends 'ago' to hours", () => {
    const ts = new Date(NOW_MS - 3 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withSuffix")).toBe("3h ago");
  });

  it("appends 'ago' to days", () => {
    const ts = new Date(NOW_MS - 5 * 24 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withSuffix")).toBe("5d ago");
  });

  it("falls back to raw input on parse failure", () => {
    expect(formatRelativeTime("bad", NOW_MS, "withSuffix")).toBe("bad");
  });

  it("parses SQLite format", () => {
    expect(formatRelativeTime("2026-04-11 11:55:00", NOW_MS, "withSuffix")).toBe("5m ago");
  });
});

describe("formatRelativeTime — withJustNow style", () => {
  const NOW_MS = Date.parse("2026-04-11T12:00:00Z");

  it("returns 'just now' for deltas under 5 seconds", () => {
    const ts = new Date(NOW_MS - 3_000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withJustNow")).toBe("just now");
  });

  it("returns seconds with suffix for 5+ seconds", () => {
    const ts = new Date(NOW_MS - 10_000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withJustNow")).toBe("10s ago");
  });

  it("returns minutes with suffix", () => {
    const ts = new Date(NOW_MS - 5 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withJustNow")).toBe("5m ago");
  });

  it("returns hours with suffix", () => {
    const ts = new Date(NOW_MS - 3 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withJustNow")).toBe("3h ago");
  });

  it("returns days with suffix", () => {
    const ts = new Date(NOW_MS - 2 * 24 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withJustNow")).toBe("2d ago");
  });

  it("boundary: exactly 5 seconds is NOT 'just now'", () => {
    const ts = new Date(NOW_MS - 5_000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "withJustNow")).toBe("5s ago");
  });
});

describe("formatRelativeTime — prose style", () => {
  const NOW_MS = Date.parse("2026-04-11T12:00:00Z");

  it("returns 'Just now' for sub-minute deltas", () => {
    const ts = new Date(NOW_MS - 30_000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("Just now");
  });

  it("returns singular minute", () => {
    const ts = new Date(NOW_MS - 60_000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("1 minute ago");
  });

  it("returns plural minutes", () => {
    const ts = new Date(NOW_MS - 5 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("5 minutes ago");
  });

  it("returns singular hour", () => {
    const ts = new Date(NOW_MS - 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("1 hour ago");
  });

  it("returns plural hours", () => {
    const ts = new Date(NOW_MS - 3 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("3 hours ago");
  });

  it("returns 'Yesterday' for exactly 1 day", () => {
    const ts = new Date(NOW_MS - 24 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("Yesterday");
  });

  it("returns plural days for 2-6 days", () => {
    const ts = new Date(NOW_MS - 3 * 24 * 60 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS, "prose")).toBe("3 days ago");
  });

  it("returns locale date string for 7+ days", () => {
    const ts = new Date(NOW_MS - 10 * 24 * 60 * 60 * 1000).toISOString();
    const result = formatRelativeTime(ts, NOW_MS, "prose");
    expect(result).not.toContain("ago");
    expect(result).not.toBe("Just now");
  });

  it("falls back to raw input on parse failure", () => {
    expect(formatRelativeTime("nope", NOW_MS, "prose")).toBe("nope");
  });
});

describe("formatRelativeTime — default style is compact", () => {
  const NOW_MS = Date.parse("2026-04-11T12:00:00Z");

  it("omitting style matches explicit compact", () => {
    const ts = new Date(NOW_MS - 5 * 60 * 1000).toISOString();
    expect(formatRelativeTime(ts, NOW_MS)).toBe(
      formatRelativeTime(ts, NOW_MS, "compact"),
    );
  });

  it("compact has no suffix", () => {
    const ts = new Date(NOW_MS - 10_000).toISOString();
    const result = formatRelativeTime(ts, NOW_MS, "compact");
    expect(result).toBe("10s");
    expect(result).not.toContain("ago");
  });
});
