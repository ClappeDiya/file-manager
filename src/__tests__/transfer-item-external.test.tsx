/**
 * Transfer queue rows of connector transfers a folder comparison runs
 * ("external" jobs): the queue can cancel them, but has no way to pause,
 * retry or re-prioritise them, so those controls are not offered.
 */
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TransferItem } from "@/components/transfer-panel";

function job(status: string, external: boolean) {
  return {
    id: `${status}-${external}`,
    source_path: external ? "sftp://Server/a.txt" : "/a.txt",
    dest_path: "/b/a.txt",
    total_bytes: 10,
    transferred_bytes: 0,
    status,
    priority: "normal",
    sort_order: 0,
    created_at: "",
    updated_at: "",
    error_message: null,
    retry_count: 0,
    speed_bps: 0,
    eta_seconds: null,
    verify_checksum: false,
    external,
  };
}

function renderItem(status: string, external: boolean) {
  const handlers = { onPause: vi.fn(), onResume: vi.fn(), onCancel: vi.fn(), onRetry: vi.fn(), onSetPriority: vi.fn(), onSetConflictPolicy: vi.fn(), onReorder: vi.fn(), onResolveConflict: vi.fn() };
  render(<TransferItem job={job(status, external)} totalJobs={1} {...handlers} />);
  return handlers;
}

describe("TransferItem", () => {
  it("offers pause, priority and conflict policy for a queue job", () => {
    renderItem("active", false);
    expect(screen.getByLabelText("Pause transfer")).toBeInTheDocument();
    expect(screen.getByLabelText("Set priority")).toBeInTheDocument();
    expect(screen.getByLabelText("Conflict policy for this transfer")).toBeInTheDocument();
  });

  it("an active connector transfer can only be cancelled", () => {
    const h = renderItem("active", true);
    expect(screen.queryByLabelText("Pause transfer")).toBeNull();
    expect(screen.queryByLabelText("Set priority")).toBeNull();
    expect(screen.queryByLabelText("Conflict policy for this transfer")).toBeNull();
    fireEvent.click(screen.getByLabelText("Cancel transfer"));
    expect(h.onCancel).toHaveBeenCalledWith("active-true");
  });

  it("a failed connector transfer offers no retry; a failed queue job does", () => {
    renderItem("failed", true);
    expect(screen.queryByLabelText("Retry transfer")).toBeNull();
    renderItem("failed", false);
    expect(screen.getByLabelText("Retry transfer")).toBeInTheDocument();
  });
});
