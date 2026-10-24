/**
 * Folder compare window: compares on open, status chips (OR filters with
 * counts), folder roll-ups, checkbox + shift-range selection → plan →
 * Safety Interlock → apply → re-compared rows, Undo, delete confirmation,
 * cancel, and the Advanced-only navigation.
 */
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { FolderCompareDialog } from "@/components/folder-compare-dialog";
import { useCompareStore } from "@/stores/compare-store";
import { useUIStore } from "@/stores/ui-store";
import type { ApplyOutcome, CompareEntry, CompareResult, CompareStatus } from "@/lib/folder-compare";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => mockInvoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

function entry(path: string, status: CompareStatus, over: Partial<CompareEntry> = {}): CompareEntry {
  const isDir = over.is_dir ?? false;
  return {
    path,
    name: path.split("/").pop() ?? path,
    is_dir: isDir,
    status,
    in_left: status !== "right_only",
    in_right: status !== "left_only",
    left_size: isDir || status === "right_only" ? null : 10,
    right_size: isDir || status === "left_only" ? null : 10,
    left_modified: null,
    right_modified: null,
    reason: status === "different" ? "Size differs" : null,
    error: null,
    children_skipped: false,
    ...over,
  };
}
const dir = (path: string, status: CompareStatus) => entry(path, status, { is_dir: true });

const totals = (files: number, folders = 0) => ({ files, folders, left_bytes: 0, right_bytes: 0 });
const SUMMARY = {
  left_only: totals(1),
  right_only: totals(2),
  same: totals(2, 1),
  different: totals(2, 3),
  error: totals(0),
  duration_ms: 1200,
};

const ENTRIES: CompareEntry[] = [
  dir("docs", "different"),
  entry("docs/a.txt", "same"),
  entry("docs/b.txt", "different", { right_size: 400 }),
  entry("docs/new.txt", "left_only", { left_size: 5 }),
  dir("media", "different"),
  dir("media/big", "different"),
  entry("media/big/x.bin", "different", { left_size: 1, right_size: 5000 }),
  entry("media/big/y.bin", "right_only", { right_size: 20 }),
  dir("same", "same"),
  entry("same/s.txt", "same"),
  entry("zzz.txt", "right_only", { right_size: 30 }),
];

function result(taskId: string): CompareResult {
  return {
    task_id: taskId,
    left: { type: "local", path: "/home/me/a" },
    right: { type: "local", path: "/home/me/b" },
    left_label: "/home/me/a",
    right_label: "/home/me/b",
    options: { filter: {} as never, compare_mode: "size_mtime", max_depth: 0, time_tolerance: false },
    entries: ENTRIES,
    summary: SUMMARY,
    notes: [],
    computed_at: "",
  };
}

const LOW = { level: "low", reasons: [], baseline_sample_size: 0, baseline_median_files: 0, baseline_median_bytes: 0, intent_hash: "h", requires_confirmation: false };

function plan(action: string, paths: string[]) {
  const steps = paths
    .filter((p) => p !== "docs/a.txt")
    .map((p) => (action.startsWith("delete") ? { type: "delete", side: "right", path: p, is_dir: false, size: 30 } : { type: "copy", from: "left", to: "right", path: p, size: 10, modified: null, replace: p === "docs/b.txt" }));
  const skipped = paths.includes("docs/a.txt") ? [{ path: "docs/a.txt", reason: "Already the same on both sides." }] : [];
  return {
    plan: { action, steps, skipped, files: steps.length, folders: 0, bytes: steps.length * 10, replaced: 0 },
    intent: { engine: "fs", kind: action.startsWith("delete") ? "delete_trash" : "copy", affected_files: steps.length, total_bytes: 10, subject_path: "/home/me/b", summary: "s" },
  };
}

function outcome(args: { paths: string[]; action: string; taskId: string }): ApplyOutcome {
  return {
    task_id: args.taskId,
    action: args.action as ApplyOutcome["action"],
    done: args.paths,
    failed: [],
    skipped: [],
    kept: [],
    notes: [],
    cancelled: false,
    undo_id: args.action.startsWith("copy") ? "undo-1" : null,
    changed: args.action.startsWith("copy") ? args.paths.map((p) => entry(p, "same")) : [],
    removed: args.action.startsWith("delete") ? args.paths : [],
    summary: SUMMARY,
  };
}

let compareResolver: ((v: unknown) => void) | null = null;
let holdCompare = false;

function backend(cmd: string, args: Record<string, unknown>): unknown {
  switch (cmd) {
    case "compare_folders":
      if (holdCompare) return new Promise((r) => (compareResolver = r));
      return result(args.taskId as string);
    case "compare_plan_action":
      return plan(args.action as string, args.paths as string[]);
    case "safety_assess_intent":
      return LOW;
    case "compare_apply":
      return outcome(args as { paths: string[]; action: string; taskId: string });
    case "list_user_filter_presets":
    case "get_filter_groups":
      return [];
    case "undo_by_correlation":
      return { success: true, summary: "Undid copy", correlation_id: "undo-1", kind: "copy", item_count: 1 };
    default:
      return true;
  }
}

const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1] as Record<string, unknown>);

const sizeDescriptors = {
  h: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight"),
  w: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth"),
};

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  // The virtualised tree renders the rows that fit its (jsdom-less) height.
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get: () => 600 });
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get: () => 900 });
});

afterAll(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  if (sizeDescriptors.h) Object.defineProperty(HTMLElement.prototype, "offsetHeight", sizeDescriptors.h);
  if (sizeDescriptors.w) Object.defineProperty(HTMLElement.prototype, "offsetWidth", sizeDescriptors.w);
});

beforeEach(() => {
  holdCompare = false;
  compareResolver = null;
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd: string, args: Record<string, unknown>) => Promise.resolve(backend(cmd, args ?? {})));
  useUIStore.setState({ appMode: "simple" });
  useCompareStore.setState({ request: null });
});

async function openCompare() {
  render(<FolderCompareDialog />);
  act(() => useCompareStore.getState().open({ type: "local", path: "/home/me/a" }, { type: "local", path: "/home/me/b" }));
  await screen.findByTestId("compare-row-docs");
}

const row = (path: string) => screen.queryByTestId(`compare-row-${path}`);

describe("FolderCompareDialog", () => {
  it("compares on open and shows status chips with counts and folder roll-ups", async () => {
    await openCompare();
    expect(calls("compare_folders")).toEqual([
      expect.objectContaining({
        left: { type: "local", path: "/home/me/a" },
        right: { type: "local", path: "/home/me/b" },
        options: expect.objectContaining({ compare_mode: "size_mtime", max_depth: 0 }),
        taskId: expect.any(String),
      }),
    ]);
    expect(screen.getByTestId("compare-chip-left_only")).toHaveTextContent("Left only 1");
    expect(screen.getByTestId("compare-chip-right_only")).toHaveTextContent("Right only 2");
    expect(screen.getByTestId("compare-chip-different")).toHaveTextContent("Different 5");
    const rollup = screen.getByTestId("compare-rollup-docs");
    expect(rollup).toHaveTextContent("1 different");
    expect(rollup).toHaveTextContent("1 left only");
    expect(screen.getByTestId("compare-status-docs/b.txt")).toHaveTextContent("Different");
    // Matching folders start collapsed.
    expect(row("same")).toBeInTheDocument();
    expect(row("same/s.txt")).toBeNull();
    fireEvent.click(screen.getByTestId("compare-toggle-same"));
    expect(row("same/s.txt")).toBeInTheDocument();
  });

  it("status chips are OR-combined filters", async () => {
    await openCompare();
    fireEvent.click(screen.getByTestId("compare-chip-left_only"));
    expect(screen.getByTestId("compare-chip-left_only")).toHaveAttribute("aria-pressed", "true");
    expect(row("docs/new.txt")).toBeInTheDocument();
    expect(row("docs/b.txt")).toBeNull();
    expect(row("zzz.txt")).toBeNull();
    fireEvent.click(screen.getByTestId("compare-chip-right_only"));
    expect(row("docs/new.txt")).toBeInTheDocument();
    expect(row("zzz.txt")).toBeInTheDocument();
    expect(row("media/big/y.bin")).toBeInTheDocument();
    expect(row("media/big/x.bin")).toBeNull();
    fireEvent.click(screen.getByTestId("compare-chip-clear"));
    expect(row("docs/b.txt")).toBeInTheDocument();
  });

  it("shift-range selection → plan → interlock → apply → re-compared rows, with Undo", async () => {
    await openCompare();
    fireEvent.click(screen.getByTestId("compare-select-docs/b.txt"));
    fireEvent.click(screen.getByTestId("compare-select-docs/new.txt"), { shiftKey: true });
    expect(row("docs/b.txt")).toHaveAttribute("aria-selected", "true");
    expect(row("docs/new.txt")).toHaveAttribute("aria-selected", "true");
    expect(row("docs/a.txt")).toHaveAttribute("aria-selected", "false");

    fireEvent.click(screen.getByTestId("compare-action-copy_to_right"));
    await screen.findByText(/Copy → right: 2 done/);
    const taskId = calls("compare_folders")[0].taskId;
    expect(calls("compare_plan_action")).toEqual([
      expect.objectContaining({ taskId, paths: ["docs/b.txt", "docs/new.txt"], action: "copy_to_right", permanent: false, targetLabel: "/home/me/b" }),
    ]);
    expect(calls("safety_assess_intent")).toEqual([{ intent: expect.objectContaining({ engine: "fs", kind: "copy" }) }]);
    expect(calls("compare_apply")).toEqual([
      expect.objectContaining({ taskId, paths: ["docs/b.txt", "docs/new.txt"], action: "copy_to_right", permanent: false, applyId: expect.any(String) }),
    ]);
    // The re-compared entries replace the old ones; the selection is cleared.
    expect(screen.getByTestId("compare-status-docs/b.txt")).toHaveTextContent("Same");
    expect(screen.getByTestId("compare-status-docs/new.txt")).toHaveTextContent("Same");
    expect(row("docs/b.txt")).toHaveAttribute("aria-selected", "false");

    fireEvent.click(screen.getByTestId("compare-undo"));
    await waitFor(() => expect(calls("undo_by_correlation")).toEqual([{ correlationId: "undo-1" }]));
    // Undo compares again.
    await waitFor(() => expect(calls("compare_folders")).toHaveLength(2));
  });

  it("selecting a folder sends the folder, not each file below it", async () => {
    await openCompare();
    fireEvent.click(screen.getByTestId("compare-select-docs/b.txt"));
    fireEvent.click(screen.getByTestId("compare-select-docs"));
    fireEvent.click(screen.getByTestId("compare-action-copy_to_left"));
    await waitFor(() => expect(calls("compare_plan_action")).toHaveLength(1));
    expect(calls("compare_plan_action")[0]).toEqual(expect.objectContaining({ paths: ["docs"], action: "copy_to_left", targetLabel: "/home/me/a" }));
  });

  it("does nothing (and says why) when only entries that are already the same are selected", async () => {
    await openCompare();
    fireEvent.click(screen.getByTestId("compare-select-docs/a.txt"));
    fireEvent.click(screen.getByTestId("compare-action-copy_to_right"));
    await screen.findByText(/already the same on both sides/);
    expect(calls("compare_apply")).toEqual([]);
    expect(calls("safety_assess_intent")).toEqual([]);
  });

  it("asks before deleting, then deletes through the interlock", async () => {
    await openCompare();
    fireEvent.click(screen.getByTestId("compare-select-zzz.txt"));
    fireEvent.click(screen.getByTestId("compare-permanent"));
    fireEvent.click(screen.getByTestId("compare-action-delete_right"));
    const confirm = await screen.findByTestId("compare-confirm-delete");
    expect(confirm).toHaveTextContent(/Delete 1 file .* on the right permanently/);
    expect(calls("compare_apply")).toEqual([]);
    fireEvent.click(within(confirm).getByTestId("compare-confirm-delete-yes"));
    await screen.findByText(/Delete on right: 1 done/);
    expect(calls("safety_assess_intent")[0]).toEqual({ intent: expect.objectContaining({ kind: "delete_trash" }) });
    expect(calls("compare_apply")[0]).toEqual(expect.objectContaining({ paths: ["zzz.txt"], action: "delete_right", permanent: true }));
    expect(row("zzz.txt")).toBeNull();
  });

  it("cancels a running comparison by its task id", async () => {
    holdCompare = true;
    render(<FolderCompareDialog />);
    act(() => useCompareStore.getState().open({ type: "local", path: "/a" }, { type: "remote", connection_id: "c1", protocol: "sftp", path: "/b" }));
    const cancel = await screen.findByTestId("compare-cancel");
    // A remote side turns on the 2-second tolerance by default.
    expect(calls("compare_folders")[0]).toEqual(expect.objectContaining({ options: expect.objectContaining({ time_tolerance: true }) }));
    fireEvent.click(cancel);
    await waitFor(() => expect(calls("compare_cancel")).toEqual([{ taskId: calls("compare_folders")[0].taskId }]));
    await act(async () => compareResolver?.(result("x")));
  });

  it("closing releases the results", async () => {
    await openCompare();
    fireEvent.click(screen.getByTestId("compare-close"));
    expect(useCompareStore.getState().request).toBeNull();
    expect(calls("compare_release")).toEqual([{ taskId: calls("compare_folders")[0].taskId }]);
  });

  it("navigation and size-difference sorting are Advanced only", async () => {
    await openCompare();
    expect(screen.queryByTestId("compare-advanced-nav")).toBeNull();
    act(() => useUIStore.setState({ appMode: "advanced" }));
    expect(screen.getByTestId("compare-advanced-nav")).toBeInTheDocument();

    // Collapse media, then jump to the largest difference by size: media/big opens up.
    fireEvent.click(screen.getByTestId("compare-toggle-media"));
    expect(row("media/big")).toBeNull();
    fireEvent.click(screen.getByTestId("compare-largest-size"));
    expect(row("media/big")).toBeInTheDocument();
    expect(row("media/big")!.className).toContain("hover-bg");

    fireEvent.click(screen.getByTestId("compare-next-diff"));
    expect(row("media/big/x.bin")!.className).toContain("hover-bg");

    // Sort by size difference: media (5,019 B apart) comes before docs.
    fireEvent.change(screen.getByTestId("compare-sort"), { target: { value: "size_delta" } });
    const order = screen.getAllByRole("row").map((r) => r.getAttribute("data-testid"));
    expect(order.indexOf("compare-row-media")).toBeLessThan(order.indexOf("compare-row-docs"));
  });

  it("sends include/exclude patterns and ordered filter rules (the sync filter engine) when comparing again", async () => {
    act(() => useUIStore.setState({ appMode: "advanced" }));
    await openCompare();
    fireEvent.change(screen.getByTestId("compare-include"), { target: { value: "*.jpg, docs/*" } });
    fireEvent.change(screen.getByTestId("compare-exclude"), { target: { value: "*.tmp" } });
    fireEvent.change(screen.getByTestId("compare-mode"), { target: { value: "checksum" } });
    fireEvent.change(screen.getByTestId("compare-depth"), { target: { value: "2" } });
    fireEvent.click(screen.getByTestId("compare-rules-toggle"));
    const rules = screen.getByTestId("compare-rules");
    fireEvent.click(within(rules).getByText("Add rule"));
    fireEvent.change(within(rules).getByRole("textbox"), { target: { value: "*.bak" } });
    fireEvent.click(screen.getByTestId("compare-run"));
    await waitFor(() => expect(calls("compare_folders")).toHaveLength(2));
    expect(calls("compare_folders")[1].options).toEqual(
      expect.objectContaining({
        compare_mode: "checksum",
        max_depth: 2,
        filter: expect.objectContaining({
          include_patterns: ["*.jpg", "docs/*"],
          exclude_patterns: ["*.tmp"],
          rules: [{ action: "exclude", kind: "wildcard", pattern: "*.bak" }],
        }),
      }),
    );
  });
});

