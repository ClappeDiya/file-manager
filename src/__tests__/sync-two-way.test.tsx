/** WS-C sync UI: two-way history, preview directions, compare modes and the Advanced-only preset. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { TwoWayHistory } from "@/components/sync-two-way-history";
import { PreviewView } from "@/components/sync-panel";
import { CopyPresetPicker, RobocopyOptionsSection } from "@/components/robocopy-options";
import { DEFAULT_COPY_OPTIONS } from "@/lib/copy-options";
import { buildComparisonRows } from "@/lib/sync-compare";
import { SyncComparison } from "@/components/sync-comparison";
import { useUIStore } from "@/stores/ui-store";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useUIStore.setState({ appMode: "simple" });
});

const entry = (path: string, action: string, src: number | null, dst: number | null, direction?: "to_dest" | "to_source") => ({
  relative_path: path,
  action,
  source_size: src,
  dest_size: dst,
  source_modified: null,
  dest_modified: null,
  reason: "r",
  path_length_warning: null,
  direction,
});

describe("TwoWayHistory", () => {
  it("is hidden in the Standard view", () => {
    render(<TwoWayHistory pairId="p1" />);
    expect(screen.queryByTestId("two-way-history")).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("shows the baseline and resets it in the Advanced view", async () => {
    useUIStore.setState({ appMode: "advanced" });
    let info: unknown = { files: 12, recorded_at: "2026-10-01T02:00:00Z", run_id: "r1" };
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_sync_baseline_info") return info;
      if (cmd === "reset_sync_baseline") {
        info = null;
        return null;
      }
      return null;
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<TwoWayHistory pairId="p1" />);
    expect(await screen.findByText(/Sync history: 12 files/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reset history" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("reset_sync_baseline", { pairId: "p1" }));
    expect(await screen.findByText(/No sync history yet/)).toBeInTheDocument();
  });
});

describe("two-way preview", () => {
  const preview = {
    pair_id: "p",
    pair_name: "Docs",
    computed_at: "2026-01-01T00:00:00Z",
    duration_ms: 5,
    additions: [entry("from-dest.txt", "add", null, 4, "to_source")],
    modifications: [],
    deletions: [entry("gone-in-dest.txt", "delete", 3, null, "to_source"), entry("gone-in-source.txt", "delete", null, 3, "to_dest")],
    skipped: [],
    conflicts: [],
    path_warnings: [],
    total_additions: 1,
    total_modifications: 0,
    total_deletions: 2,
    total_skipped: 0,
    total_conflicts: 0,
    total_bytes: 4,
    notes: ["First two-way run: files are combined in both directions and nothing is deleted."],
  };

  it("says which side deletions happen on and shows the planner's notes", () => {
    render(<PreviewView preview={preview} onProceed={vi.fn()} onCancel={vi.fn()} onExport={vi.fn()} loading={false} />);
    expect(screen.getByText(/2 items will be deleted/)).toHaveTextContent("1 from the source, 1 from the destination");
    expect(screen.getByTestId("sync-preview-note")).toHaveTextContent("First two-way run");
  });

  it("strikes deleted files on the side they are deleted from", () => {
    const rows = buildComparisonRows(preview);
    expect(rows.find((r) => r.path === "gone-in-dest.txt")!.direction).toBe("to_source");
    render(<SyncComparison preview={preview} />);
    const sourceCopy = screen.getByText("gone-in-dest.txt").closest("span.line-through");
    expect(sourceCopy).not.toBeNull();
    expect(screen.getAllByText("← New").length).toBe(1);
  });
});

describe("compare mode and presets", () => {
  it("offers the compare mode only in the Advanced view", () => {
    const onChange = vi.fn();
    const { rerender } = render(<RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={onChange} mode="one_way" windows={false} />);
    expect(screen.queryByLabelText("Compare files by")).toBeNull();
    useUIStore.setState({ appMode: "advanced" });
    rerender(<RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={onChange} mode="one_way" windows={false} />);
    fireEvent.change(screen.getByLabelText("Compare files by"), { target: { value: "checksum" } });
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ compare_mode: "checksum" }));
  });

  it("shows the two-way delete limit for two-way pairs", () => {
    const onChange = vi.fn();
    render(<RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={onChange} mode="two_way" windows={false} />);
    expect(screen.getByText(/Stop if a run would delete more than/)).toBeInTheDocument();
  });

  it("keeps the Ignore existing preset for the Advanced view", () => {
    const onPick = vi.fn();
    const { rerender } = render(<CopyPresetPicker activeId={null} onPick={onPick} />);
    expect(screen.queryByRole("button", { name: "Ignore existing" })).toBeNull();
    useUIStore.setState({ appMode: "advanced" });
    rerender(<CopyPresetPicker activeId={null} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: "Ignore existing" }));
    expect(onPick).toHaveBeenCalledWith(
      expect.objectContaining({ options: { exclude_older: true, exclude_newer: true, exclude_changed: true } }),
    );
  });
});
