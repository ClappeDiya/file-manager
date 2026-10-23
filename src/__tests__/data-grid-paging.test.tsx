/**
 * K122: sortable, paged tables — the generic DataGrid (client and server
 * paging, page sizes, virtualized "All", keyboard), remembered views in the
 * UI store, the paged IPC hook and the backup tables built on them.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import type { ColumnDef } from "@tanstack/react-table";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { DataGrid, type GridQuery } from "@/components/data-grid";
import { migrateUIState, useUIStore } from "@/stores/ui-store";
import { usePagedListing } from "@/hooks/use-paged-listing";
import { BackupRunHistory } from "@/components/backup/backup-run-history";
import { useBackupStore } from "@/stores/backup-store";
import { runDuration } from "@/lib/backup-time";

type Item = { id: number; name: string; size: number };
const items: Item[] = Array.from({ length: 120 }, (_, i) => ({ id: i, name: `file-${String(i).padStart(3, "0")}`, size: (i * 37) % 101 }));
const columns: ColumnDef<Item>[] = [
  { id: "name", header: "Name", accessorFn: (r) => r.name },
  { id: "size", header: "Size", accessorFn: (r) => r.size },
];
const grid = (extra: Partial<Parameters<typeof DataGrid<Item>>[0]> = {}) => (
  <DataGrid<Item> data={items} columns={columns} label="Things" pagination getRowId={(r) => String(r.id)} {...extra} />
);
const bodyRows = () => screen.getAllByRole("row").filter((r) => r.querySelector("td[role='gridcell']"));
const firstCell = () => bodyRows()[0].querySelector("td")!.textContent;

beforeEach(() => {
  localStorage.clear();
  mockInvoke.mockReset();
  useUIStore.setState({ tableViews: {} });
});

describe("DataGrid paging and sorting", () => {
  it("pages 50 rows by default and moves between pages", () => {
    render(grid());
    expect(bodyRows()).toHaveLength(50);
    expect(screen.getByTestId("grid-range")).toHaveTextContent("1–50 of 120");
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(screen.getByTestId("grid-range")).toHaveTextContent("51–100 of 120");
    expect(firstCell()).toBe("file-050");
    fireEvent.click(screen.getByRole("button", { name: "Last page" }));
    expect(bodyRows()).toHaveLength(20);
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "First page" }));
    expect(firstCell()).toBe("file-000");
  });

  it("sorts by a header click and sets aria-sort, descending on the second click", () => {
    render(grid());
    const size = screen.getByRole("columnheader", { name: /Size/ });
    expect(size).toHaveAttribute("aria-sort", "none");
    fireEvent.click(size);
    expect(size).toHaveAttribute("aria-sort", "ascending");
    expect(bodyRows()[0].querySelectorAll("td")[1].textContent).toBe("0");
    fireEvent.click(size);
    expect(size).toHaveAttribute("aria-sort", "descending");
    expect(bodyRows()[0].querySelectorAll("td")[1].textContent).toBe("100");
  });

  it("is keyboard accessible: Enter on a header sorts, Enter on a row activates it", () => {
    const onRowActivate = vi.fn();
    render(grid({ onRowActivate }));
    const name = screen.getByRole("columnheader", { name: /Name/ });
    expect(name).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(name, { key: "Enter" });
    expect(name).toHaveAttribute("aria-sort", "ascending");
    fireEvent.keyDown(name, { key: " " });
    expect(name).toHaveAttribute("aria-sort", "descending");
    fireEvent.keyDown(bodyRows()[0], { key: "Enter" });
    expect(onRowActivate).toHaveBeenCalledWith(items[119]);
  });

  it("changes the page size and virtualizes All", () => {
    render(grid());
    fireEvent.change(screen.getByTestId("grid-page-size"), { target: { value: "25" } });
    expect(bodyRows()).toHaveLength(25);
    // jsdom has no layout: give the scroll area a height so the virtualizer has a window.
    const h = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get: () => 448 });
    try {
      fireEvent.change(screen.getByTestId("grid-page-size"), { target: { value: "all" } });
    } finally {
      if (h) Object.defineProperty(HTMLElement.prototype, "offsetHeight", h);
    }
    expect(screen.getByTestId("grid-range")).toHaveTextContent("1–120 of 120");
    expect(screen.queryByRole("button", { name: "Next page" })).toBeNull();
    const shown = bodyRows().length;
    expect(shown).toBeGreaterThan(0);
    expect(shown).toBeLessThan(120);
  });

  it("goes back to the first page when the reset key changes", () => {
    const { rerender } = render(grid({ resetKey: "a" }));
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    rerender(grid({ resetKey: "b" }));
    expect(screen.getByTestId("grid-range")).toHaveTextContent("1–50 of 120");
  });
});

describe("remembered table views", () => {
  it("persists page size and sort per table and restores them", () => {
    const { unmount } = render(grid({ prefsKey: "things" }));
    fireEvent.change(screen.getByTestId("grid-page-size"), { target: { value: "100" } });
    fireEvent.click(screen.getByRole("columnheader", { name: /Size/ }));
    expect(useUIStore.getState().tableViews.things).toEqual({ pageSize: 100, sort: { id: "size", desc: false } });
    const stored = JSON.parse(localStorage.getItem("ufop-ui-state") || "{}");
    expect(stored.state.tableViews.things.pageSize).toBe(100);
    unmount();
    render(grid({ prefsKey: "things" }));
    expect(bodyRows()).toHaveLength(100);
    expect(screen.getByRole("columnheader", { name: /Size/ })).toHaveAttribute("aria-sort", "ascending");
    // Another table keeps its own view.
    expect(useUIStore.getState().tableViews.other).toBeUndefined();
  });

  it("the migration drops broken stored views", () => {
    const out = migrateUIState(
      { tableViews: { ok: { pageSize: 25, sort: null }, bad: { pageSize: 7, sort: null }, worse: "x", sorted: { pageSize: "all", sort: { id: "a", desc: true } } } },
      1,
    );
    expect(out.tableViews).toEqual({ ok: { pageSize: 25, sort: null }, sorted: { pageSize: "all", sort: { id: "a", desc: true } } });
  });
});

describe("server paging", () => {
  it("asks for the shown page and sort, and shows the server total", () => {
    const queries: GridQuery[] = [];
    const page = items.slice(0, 50);
    render(<DataGrid<Item> data={page} columns={columns} label="Things" pagination server={{ total: 1000, onQueryChange: (q) => queries.push(q) }} />);
    expect(screen.getByTestId("grid-range")).toHaveTextContent("1–50 of 1,000");
    expect(queries.at(-1)).toEqual({ offset: 0, limit: 50, sort: null });
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(queries.at(-1)).toEqual({ offset: 50, limit: 50, sort: null });
    fireEvent.click(screen.getByRole("columnheader", { name: /Size/ }));
    expect(queries.at(-1)).toEqual({ offset: 0, limit: 50, sort: { id: "size", desc: false } });
    // Server rows are shown as given (already sorted by the backend).
    expect(firstCell()).toBe("file-000");
    fireEvent.change(screen.getByTestId("grid-page-size"), { target: { value: "all" } });
    expect(queries.at(-1)).toEqual({ offset: 0, limit: null, sort: { id: "size", desc: false } });
  });

  it("usePagedListing sends the page and reads totals, or filters a plain array itself", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    mockInvoke.mockResolvedValueOnce({ items: [{ id: 1 }], total: 9, offset: 0, limit: 25 });
    const view = { pageSize: 25 as const, sort: { id: "size", desc: true } };
    const { result } = renderHook(() => usePagedListing<{ id: number }>("backup_list_files", { id: "j" }, { view, filter: " x ", textOf: () => "" }));
    await waitFor(() => expect(result.current.total).toBe(9));
    expect(mockInvoke).toHaveBeenCalledWith("backup_list_files", { id: "j", page: { offset: 0, limit: 25, sort: "size", descending: true, filter: "x" } });
    expect(result.current.server?.total).toBe(9);

    mockInvoke.mockResolvedValueOnce([{ n: "alpha" }, { n: "beta" }, { n: "Alphabet" }]);
    const arr = renderHook(() => usePagedListing<{ n: string }>("backup_remote_list", {}, { view, filter: "ALPHA", textOf: (r) => r.n }));
    await waitFor(() => expect(arr.result.current.total).toBe(2));
    expect(arr.result.current.server).toBeUndefined();
  });
});

describe("run history", () => {
  it("loads pages of runs from the backend, sorts there and opens a run's log", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    useBackupStore.setState({ jobs: [{ id: "j1", name: "Docs" } as never] });
    const run = (i: number) => ({ id: `r${i}`, job_id: "j1", operation: "backup", status: i % 2 ? "warning" : "success", started_at: "2026-10-01T10:00:00Z", finished_at: "2026-10-01T10:02:05Z", summary: `run ${i}`, report: {}, log: [{ level: "info", message: `log ${i}` }] });
    mockInvoke.mockImplementation(async (cmd: string, args: { page?: { offset: number; limit: number } }) => {
      if (cmd !== "backup_run_history") return null;
      const off = args.page?.offset ?? 0;
      return { items: Array.from({ length: 3 }, (_, k) => run(off + k)), total: 230, offset: off, limit: args.page?.limit ?? 50 };
    });
    render(<BackupRunHistory />);
    expect(await screen.findByText("run 0")).toBeInTheDocument();
    expect(screen.getByTestId("grid-range")).toHaveTextContent("of 230");
    expect(within(screen.getByTestId("run-row-r0")).getByText("2 min")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("columnheader", { name: /Result/ }));
    await waitFor(() => expect(mockInvoke).toHaveBeenLastCalledWith("backup_run_history", expect.objectContaining({ jobId: null, page: expect.objectContaining({ sort: "status", descending: false, offset: 0 }) })));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    });
    await waitFor(() => expect(screen.getByText("run 50")).toBeInTheDocument());
    fireEvent.click(screen.getByText("run 50"));
    expect(screen.getByTestId("run-details")).toHaveTextContent("log 50");
  });

  it("formats durations", () => {
    expect(runDuration("2026-01-01T00:00:00Z", null)).toBe("—");
    expect(runDuration("2026-01-01T00:00:00Z", "2026-01-01T00:00:09Z")).toBe("9 s");
    expect(runDuration("2026-01-01T00:00:00Z", "2026-01-01T01:05:00Z")).toBe("1 h 5 min");
  });
});
