import { describe, it, expect, vi } from "vitest";
import { PreviewView } from "@/components/sync-panel";
import { render, screen, fireEvent } from "@testing-library/react";
import { SyncComparison } from "@/components/sync-comparison";
import { buildComparisonRows, summarizeRows, type PreviewEntry, type PreviewLike } from "@/lib/sync-compare";

const entry = (path: string, action: string, src: number | null, dst: number | null): PreviewEntry => ({
  relative_path: path,
  action,
  source_size: src,
  dest_size: dst,
  source_modified: src === null ? null : "2026-01-01T00:00:00Z",
  dest_modified: dst === null ? null : "2025-01-01T00:00:00Z",
  reason: "r",
});

const preview: PreviewLike = {
  additions: [entry("docs/new.txt", "add", 10, null)],
  modifications: [entry("docs/changed.txt", "modify", 20, 5)],
  deletions: [entry("old/gone.txt", "delete", null, 7)],
  skipped: [entry("tmp.log", "skip", 3, null)],
  conflicts: [],
  dirs_to_create: ["empty"],
  dirs_to_remove: ["old"],
};

describe("buildComparisonRows", () => {
  it("groups files under folders, folders first", () => {
    const rows = buildComparisonRows(preview);
    expect(rows.map((r) => r.path)).toEqual([
      "docs",
      "docs/changed.txt",
      "docs/new.txt",
      "empty",
      "old",
      "old/gone.txt",
    ]);
    expect(rows.find((r) => r.path === "docs")!.change).toBe("folder");
    expect(rows.find((r) => r.path === "old")!.change).toBe("rmdir");
    expect(rows.find((r) => r.path === "empty")!.change).toBe("mkdir");
  });

  it("puts each side's data in the right column", () => {
    const rows = buildComparisonRows(preview);
    const added = rows.find((r) => r.path === "docs/new.txt")!;
    expect(added.source?.size).toBe(10);
    expect(added.dest).toBeNull();
    const deleted = rows.find((r) => r.path === "old/gone.txt")!;
    expect(deleted.source).toBeNull();
    expect(deleted.dest?.size).toBe(7);
  });

  it("includes skipped files only on request, and normalises Windows paths", () => {
    expect(buildComparisonRows(preview).some((r) => r.path === "tmp.log")).toBe(false);
    expect(buildComparisonRows(preview, true).some((r) => r.path === "tmp.log")).toBe(true);
    const win = buildComparisonRows({ ...preview, additions: [entry("a\\b.txt", "add", 1, null)] });
    expect(win.some((r) => r.path === "a/b.txt" && r.depth === 1)).toBe(true);
  });

  it("summarises counts, counting removed folders as deletions", () => {
    expect(summarizeRows(buildComparisonRows(preview))).toEqual({
      add: 1,
      modify: 1,
      delete: 2,
      conflict: 0,
      mkdir: 1,
    });
  });
});

describe("SyncComparison", () => {
  it("renders both columns and toggles skipped rows", () => {
    render(<SyncComparison preview={preview} />);
    expect(screen.getByText("Source")).toBeInTheDocument();
    expect(screen.getByText("Destination")).toBeInTheDocument();
    expect(screen.getAllByText("Delete").length).toBe(1);
    expect(screen.queryByText("tmp.log")).toBeNull();
    fireEvent.click(screen.getByLabelText("Show skipped"));
    expect(screen.getByText("tmp.log")).toBeInTheDocument();
  });

  it("says so when nothing will change", () => {
    render(
      <SyncComparison
        preview={{ additions: [], modifications: [], deletions: [], skipped: [], conflicts: [] }}
      />,
    );
    expect(screen.getByText(/both sides already match/)).toBeInTheDocument();
  });
});


describe("PreviewView delete confirmation", () => {
  const full = {
    pair_id: "p",
    pair_name: "Docs",
    computed_at: "2026-01-01T00:00:00Z",
    duration_ms: 5,
    path_warnings: [],
    total_additions: 1,
    total_modifications: 1,
    total_deletions: 1,
    total_skipped: 1,
    total_conflicts: 0,
    total_bytes: 30,
    ...preview,
  } as any;

  it("blocks Proceed until deletions are acknowledged", () => {
    const onProceed = vi.fn();
    render(<PreviewView preview={full} onProceed={onProceed} onCancel={vi.fn()} onExport={vi.fn()} loading={false} />);
    const proceed = screen.getByRole("button", { name: "Proceed" });
    expect(proceed).toBeDisabled();
    expect(screen.getByText(/2 items will be deleted/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: /will be deleted/ }));
    expect(proceed).toBeEnabled();
    fireEvent.click(proceed);
    expect(onProceed).toHaveBeenCalled();
  });

  it("switches to the side-by-side tab", () => {
    render(<PreviewView preview={full} onProceed={vi.fn()} onCancel={vi.fn()} onExport={vi.fn()} loading={false} />);
    fireEvent.click(screen.getByRole("tab", { name: "Side by side" }));
    expect(screen.getByTestId("sync-comparison")).toBeInTheDocument();
  });

  it("needs no confirmation when nothing is deleted", () => {
    const safe = { ...full, deletions: [], total_deletions: 0, dirs_to_remove: [] };
    render(<PreviewView preview={safe} onProceed={vi.fn()} onCancel={vi.fn()} onExport={vi.fn()} loading={false} />);
    expect(screen.getByRole("button", { name: "Proceed" })).toBeEnabled();
  });
});
