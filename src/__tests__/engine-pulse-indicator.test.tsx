/**
 * EnginePulseIndicator — drill-in behaviour.
 *
 * Locks the contract that each colored engine dot is a button that
 * opens the Activity Timeline pre-filtered to that engine via the
 * `openWithPreset` primitive — and that the error badge continues to
 * open with `failedOnly: true`. Both flows MUST pass the
 * sibling-clearing intent (`failedOnly: false` / `engineFilter: "all"`,
 * `correlationFilter: null`) so the user sees the whole per-axis
 * slice rather than the intersection with any prior narrowing.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { EnginePulseIndicator } from "@/components/engine-pulse-indicator";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";
import * as pulseModule from "@/lib/pulse";

beforeEach(() => {
  useActivityTimelineStore.setState({
    panelOpen: false,
    pendingPreset: null,
  });
  vi.restoreAllMocks();
});

describe("EnginePulseIndicator: drill-in", () => {
  it("renders one button per active engine and one for the error badge", async () => {
    vi.spyOn(pulseModule, "getEnginePulse").mockResolvedValue({
      since: "2026-05-14T00:00:00Z",
      total: 9,
      by_engine: { fs: 4, transfer: 3, sync: 2 },
      by_status: { ok: 7, failed: 2 },
    });
    render(<EnginePulseIndicator />);
    await waitFor(() => {
      expect(screen.getByTestId("engine-pulse-dot-fs")).toBeInTheDocument();
    });
    expect(screen.getByTestId("engine-pulse-dot-transfer")).toBeInTheDocument();
    expect(screen.getByTestId("engine-pulse-dot-sync")).toBeInTheDocument();
    expect(screen.getByTestId("engine-pulse-error-badge")).toBeInTheDocument();
  });

  it("clicking an engine dot opens the timeline filtered to that engine", async () => {
    vi.spyOn(pulseModule, "getEnginePulse").mockResolvedValue({
      since: "2026-05-14T00:00:00Z",
      total: 5,
      by_engine: { transfer: 5 },
      by_status: { ok: 5 },
    });
    render(<EnginePulseIndicator />);
    const dot = await screen.findByTestId("engine-pulse-dot-transfer");
    fireEvent.click(dot);

    const state = useActivityTimelineStore.getState();
    expect(state.panelOpen).toBe(true);
    expect(state.pendingPreset).toEqual({
      engineFilter: "transfer",
      failedOnly: false,
      correlationFilter: null,
    });
  });

  it("clicking the error badge opens the timeline filtered to all failures", async () => {
    vi.spyOn(pulseModule, "getEnginePulse").mockResolvedValue({
      since: "2026-05-14T00:00:00Z",
      total: 5,
      by_engine: { fs: 5 },
      by_status: { ok: 2, failed: 3 },
    });
    render(<EnginePulseIndicator />);
    const badge = await screen.findByTestId("engine-pulse-error-badge");
    fireEvent.click(badge);

    expect(useActivityTimelineStore.getState().pendingPreset).toEqual({
      failedOnly: true,
      engineFilter: "all",
      correlationFilter: null,
    });
  });

  it("dots are buttons with accessible labels carrying the engine name and op count", async () => {
    vi.spyOn(pulseModule, "getEnginePulse").mockResolvedValue({
      since: "2026-05-14T00:00:00Z",
      total: 1,
      by_engine: { fs: 1 },
      by_status: { ok: 1 },
    });
    render(<EnginePulseIndicator />);
    const dot = await screen.findByTestId("engine-pulse-dot-fs");
    expect(dot.tagName).toBe("BUTTON");
    expect(dot.getAttribute("aria-label")).toMatch(/Files.*1 op/);
  });

  it("renders no dots and no error badge when there is no recent activity", async () => {
    vi.spyOn(pulseModule, "getEnginePulse").mockResolvedValue({
      since: "2026-05-14T00:00:00Z",
      total: 0,
      by_engine: {},
      by_status: {},
    });
    render(<EnginePulseIndicator />);
    // The indicator renders "Idle" when there are no ops.
    await screen.findByText("Idle");
    expect(screen.queryByTestId("engine-pulse-error-badge")).toBeNull();
    // No engine dots should be rendered.
    expect(
      screen.queryAllByText("", { selector: "[data-testid^='engine-pulse-dot-']" }).length,
    ).toBe(0);
  });
});
