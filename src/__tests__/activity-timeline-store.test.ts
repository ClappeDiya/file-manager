/**
 * activity-timeline-store
 *
 * Covers the panel-open + drill-in preset contract:
 *   - successive `openWithPreset` calls overwrite, never queue
 *   - `consumePendingPreset` is atomic read-and-clear
 *   - a consumed preset never re-applies on its own
 *   - close-then-reopen does not resurrect a consumed preset
 *
 * The dedicated test guards against a future regression where a
 * caller leaks a stale preset across openings — which would silently
 * re-filter the timeline whenever the user re-opens the panel.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";

beforeEach(() => {
  // Each test starts from a closed, preset-less state. Zustand stores
  // are module-singletons, so without an explicit reset the prior
  // test's tail state would leak in.
  useActivityTimelineStore.setState({
    panelOpen: false,
    pendingPreset: null,
  });
});

describe("activity-timeline-store: open/close basics", () => {
  it("starts closed with no pending preset", () => {
    const s = useActivityTimelineStore.getState();
    expect(s.panelOpen).toBe(false);
    expect(s.pendingPreset).toBeNull();
  });

  it("openPanel sets panelOpen without touching preset", () => {
    useActivityTimelineStore.getState().openPanel();
    const s = useActivityTimelineStore.getState();
    expect(s.panelOpen).toBe(true);
    expect(s.pendingPreset).toBeNull();
  });

  it("closePanel does not touch a pending preset", () => {
    useActivityTimelineStore
      .getState()
      .openWithPreset({ failedOnly: true });
    useActivityTimelineStore.getState().closePanel();
    const s = useActivityTimelineStore.getState();
    expect(s.panelOpen).toBe(false);
    // Pending preset persists across a close — the next openPanel
    // (or any panel render) consumes it via consumePendingPreset.
    expect(s.pendingPreset).toEqual({ failedOnly: true });
  });

  it("togglePanel flips state without touching preset", () => {
    const t = useActivityTimelineStore.getState().togglePanel;
    t();
    expect(useActivityTimelineStore.getState().panelOpen).toBe(true);
    t();
    expect(useActivityTimelineStore.getState().panelOpen).toBe(false);
    expect(useActivityTimelineStore.getState().pendingPreset).toBeNull();
  });
});

describe("activity-timeline-store: drill-in preset", () => {
  it("openWithPreset opens the panel and stages the preset", () => {
    useActivityTimelineStore.getState().openWithPreset({
      failedOnly: true,
      engineFilter: "fs",
    });
    const s = useActivityTimelineStore.getState();
    expect(s.panelOpen).toBe(true);
    expect(s.pendingPreset).toEqual({
      failedOnly: true,
      engineFilter: "fs",
    });
  });

  it("consumePendingPreset returns the preset and clears it atomically", () => {
    useActivityTimelineStore
      .getState()
      .openWithPreset({ correlationFilter: "abc-123" });
    const consumed = useActivityTimelineStore
      .getState()
      .consumePendingPreset();
    expect(consumed).toEqual({ correlationFilter: "abc-123" });
    expect(useActivityTimelineStore.getState().pendingPreset).toBeNull();
  });

  it("consumePendingPreset returns null when nothing is staged", () => {
    expect(
      useActivityTimelineStore.getState().consumePendingPreset(),
    ).toBeNull();
  });

  it("a consumed preset does not re-apply on subsequent reads", () => {
    useActivityTimelineStore
      .getState()
      .openWithPreset({ failedOnly: true });
    const first = useActivityTimelineStore
      .getState()
      .consumePendingPreset();
    const second = useActivityTimelineStore
      .getState()
      .consumePendingPreset();
    expect(first).toEqual({ failedOnly: true });
    expect(second).toBeNull();
  });

  it("successive openWithPreset calls overwrite (do not queue)", () => {
    const api = useActivityTimelineStore.getState();
    api.openWithPreset({ failedOnly: true });
    api.openWithPreset({ engineFilter: "transfer" });
    expect(useActivityTimelineStore.getState().pendingPreset).toEqual({
      engineFilter: "transfer",
    });
  });

  it("close-then-reopen does not resurrect a consumed preset", () => {
    const api = useActivityTimelineStore.getState();
    api.openWithPreset({ failedOnly: true });
    api.consumePendingPreset();
    api.closePanel();
    api.openPanel();
    expect(useActivityTimelineStore.getState().pendingPreset).toBeNull();
  });

  it("supports null correlationFilter to explicitly clear an active trace", () => {
    // The engine-pulse error drill-in passes correlationFilter: null
    // so the user sees every failure, not just failures inside
    // whatever correlation trace happened to be active. The store
    // must distinguish "leave alone" (undefined) from "explicitly
    // clear" (null) — this test locks that distinction.
    useActivityTimelineStore.getState().openWithPreset({
      failedOnly: true,
      engineFilter: "all",
      correlationFilter: null,
    });
    expect(
      useActivityTimelineStore.getState().pendingPreset,
    ).toEqual({
      failedOnly: true,
      engineFilter: "all",
      correlationFilter: null,
    });
  });

  it("carries pathFilter through openWithPreset", () => {
    // "Show Folder Activity" passes pathFilter to narrow the timeline
    // to events under a specific directory. The store must round-trip
    // it just like the other preset fields — same three-state
    // semantics (string = set, null = clear, undefined = leave alone).
    useActivityTimelineStore.getState().openWithPreset({
      pathFilter: "/Users/me/docs",
      engineFilter: "all",
      failedOnly: false,
      correlationFilter: null,
    });
    expect(
      useActivityTimelineStore.getState().pendingPreset,
    ).toEqual({
      pathFilter: "/Users/me/docs",
      engineFilter: "all",
      failedOnly: false,
      correlationFilter: null,
    });
  });

  it("supports null pathFilter to explicitly clear an active folder filter", () => {
    useActivityTimelineStore.getState().openWithPreset({
      pathFilter: null,
    });
    expect(
      useActivityTimelineStore.getState().pendingPreset,
    ).toEqual({
      pathFilter: null,
    });
  });
});
