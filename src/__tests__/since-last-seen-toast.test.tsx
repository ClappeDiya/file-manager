/**
 * SinceLastSeenToast — drill-in behaviour.
 *
 * Locks the contract that clicking the toast body opens the Activity
 * Timeline via `openWithPreset` with the right filter intent, and
 * that the X dismiss button continues to dismiss without drilling in.
 *
 * Why a dedicated test:
 *   - The toast text used to advertise "open the timeline for details"
 *     with no actual action. Future refactors of either the toast or
 *     the timeline store could silently re-orphan this affordance —
 *     this test makes that regression visible.
 *   - The drill-in MUST pass `engineFilter: "all"` and
 *     `correlationFilter: null` so the user sees the whole "while you
 *     were away" window, not just whatever filter was active before.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SinceLastSeenToast, type LedgerSinceSummary } from "@/components/since-last-seen-toast";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";

beforeEach(() => {
  useActivityTimelineStore.setState({
    panelOpen: false,
    pendingPreset: null,
  });
});

function makeSummary(overrides: Partial<LedgerSinceSummary> = {}): LedgerSinceSummary {
  return {
    since: "2026-05-14T00:00:00Z",
    total: 4,
    by_engine: { fs: 3, sync: 1 },
    by_status: { ok: 4 },
    ...overrides,
  };
}

describe("SinceLastSeenToast: drill-in", () => {
  it("renders an Open Timeline CTA when total > 0", () => {
    render(
      <SinceLastSeenToast summary={makeSummary()} onDismiss={() => {}} />,
    );
    const cta = screen.getByTestId("since-last-seen-open-timeline");
    expect(cta).toBeInTheDocument();
    expect(cta.textContent).toContain("Open Timeline");
  });

  it("clicking the CTA opens the timeline with engine and correlation cleared", () => {
    const onDismiss = vi.fn();
    render(
      <SinceLastSeenToast summary={makeSummary()} onDismiss={onDismiss} />,
    );
    fireEvent.click(screen.getByTestId("since-last-seen-open-timeline"));

    const state = useActivityTimelineStore.getState();
    expect(state.panelOpen).toBe(true);
    expect(state.pendingPreset).toEqual({
      failedOnly: false,
      engineFilter: "all",
      correlationFilter: null,
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("clicking the CTA with failed > 0 stages failedOnly:true and labels the CTA accordingly", () => {
    render(
      <SinceLastSeenToast
        summary={makeSummary({
          total: 5,
          by_status: { ok: 2, failed: 3 },
        })}
        onDismiss={() => {}}
      />,
    );
    const cta = screen.getByTestId("since-last-seen-open-timeline");
    expect(cta.textContent).toContain("3 failed");
    fireEvent.click(cta);
    expect(useActivityTimelineStore.getState().pendingPreset).toEqual({
      failedOnly: true,
      engineFilter: "all",
      correlationFilter: null,
    });
  });

  it("X dismiss button dismisses without drilling in", () => {
    const onDismiss = vi.fn();
    render(
      <SinceLastSeenToast summary={makeSummary()} onDismiss={onDismiss} />,
    );
    fireEvent.click(screen.getByLabelText("Dismiss"));
    expect(onDismiss).toHaveBeenCalledTimes(1);
    // Key contract: dismissing the toast must NOT stage a preset.
    expect(useActivityTimelineStore.getState().pendingPreset).toBeNull();
    expect(useActivityTimelineStore.getState().panelOpen).toBe(false);
  });

  it("renders nothing when summary is null or total is 0", () => {
    const { container, rerender } = render(
      <SinceLastSeenToast summary={null} onDismiss={() => {}} />,
    );
    expect(container.textContent).toBe("");
    rerender(
      <SinceLastSeenToast
        summary={makeSummary({ total: 0 })}
        onDismiss={() => {}}
      />,
    );
    expect(container.textContent).toBe("");
  });
});

describe("SinceLastSeenToast: bytes total surfacing", () => {
  it("renders the bytes total inline in the headline when present", () => {
    // After iteration 10, `LedgerSinceSummary` carries optional
    // `bytes_total` from the backend's COALESCE(SUM(bytes), 0)
    // aggregation. The toast headline should surface it so the user
    // sees data-volume scale, not just op count. Suppression-when-zero
    // / suppression-when-null is covered by the next two tests.
    render(
      <SinceLastSeenToast
        summary={makeSummary({ bytes_total: 1_500_000 })}
        onDismiss={() => {}}
      />,
    );
    expect(screen.getByText(/4 operations · /)).toBeInTheDocument();
    expect(
      screen.getByText(/ran while you were away/),
    ).toBeInTheDocument();
  });

  it("suppresses bytes when total is 0 (metadata-only window)", () => {
    // A window full of renames / folder creates / AI suggestions has
    // bytes_total === 0. The toast should NOT render "0 B" — the
    // headline stays clean.
    render(
      <SinceLastSeenToast
        summary={makeSummary({ bytes_total: 0 })}
        onDismiss={() => {}}
      />,
    );
    expect(screen.queryByText(/0 B/)).not.toBeInTheDocument();
    expect(
      screen.getByText(/4 operations ran while you were away/),
    ).toBeInTheDocument();
  });

  it("suppresses bytes when the field is null (first-run sentinel)", () => {
    // `ledger_since_last_seen` returns `bytes_total: null` on the
    // first run when there's no baseline timestamp. The toast must
    // tolerate that without rendering "null B".
    render(
      <SinceLastSeenToast
        summary={makeSummary({ bytes_total: null })}
        onDismiss={() => {}}
      />,
    );
    expect(screen.queryByText(/null/)).not.toBeInTheDocument();
    expect(
      screen.getByText(/4 operations ran while you were away/),
    ).toBeInTheDocument();
  });
});
