/** Operations drawer: active ops with progress, cancel, recent events, indicator. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { OperationsDrawer, OperationsIndicator } from "@/components/operations-drawer";
import { formatEta, useOperationsStore, type OpItem } from "@/stores/operations-store";

const op = (over: Partial<OpItem>): OpItem => ({
  engine: "transfer", id: "t1", title: "Copy big.iso", detail: "→ /backup", state: "running", fraction: 0.5, bytes_done: 512, bytes_total: 1024,
  items_done: 0, items_total: 1, started_at: null, bytes_per_sec: 256, eta_secs: 2, cancellable: true, ...over,
});

beforeEach(() => {
  useOperationsStore.setState({ open: false });
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    if (cmd === "operations_snapshot")
      return {
        active: [op({}), op({ engine: "index", id: "/home/me", title: "Index me", fraction: null, bytes_total: 0, items_done: 40, items_total: 0, bytes_per_sec: 0, eta_secs: null }), op({ engine: "backup", id: "k", title: "Backup Docs", state: "queued", fraction: null })],
        recent: [{ id: "e1", occurred_at: "2026-10-22T10:00:00Z", engine: "fs", kind: "move", status: "ok", summary: "move: a.txt → /b", subject_path: null }],
      };
    if (cmd === "operations_cancel") return true;
    return null;
  });
});

describe("OperationsDrawer", () => {
  it("shows running and queued work with progress, and recent events", async () => {
    render(<OperationsDrawer />);
    expect(screen.queryByRole("complementary", { name: "Operations" })).toBeNull();
    act(() => useOperationsStore.getState().setOpen(true));
    const row = await screen.findByRole("listitem", { name: "Copy big.iso" });
    expect(row.textContent).toContain("50%");
    expect(row.textContent).toContain("2s left");
    expect(screen.getByRole("progressbar", { name: "Copy big.iso progress" }).getAttribute("aria-valuenow")).toBe("50");
    expect(screen.getByRole("listitem", { name: "Index me" }).textContent).toContain("Working");
    expect(screen.getByRole("listitem", { name: "Backup Docs" }).textContent).toContain("Waiting");
    expect(screen.getByText("move: a.txt → /b")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel Copy big.iso" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("operations_cancel", { engine: "transfer", id: "t1" }));
    fireEvent.click(screen.getByRole("button", { name: "Close operations" }));
    expect(useOperationsStore.getState().open).toBe(false);
  });
});

describe("OperationsIndicator", () => {
  it("counts running operations and toggles the drawer", async () => {
    render(<OperationsIndicator />);
    const b = await screen.findByRole("button", { name: /2 operations running/ });
    expect(mockInvoke).toHaveBeenCalledWith("operations_snapshot", { recentLimit: 0 });
    fireEvent.click(b);
    expect(useOperationsStore.getState().open).toBe(true);
  });
});

describe("formatEta", () => {
  it("formats", () => {
    expect([formatEta(null), formatEta(5), formatEta(65), formatEta(3_725)]).toEqual([null, "5s", "1m 05s", "1h 02m"]);
  });
});
