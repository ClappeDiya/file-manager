/** WS-I rows 72, 73, 93: direction chips, per-file list of a running sync, date-ranged transfer history, failed-transfer retention. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { TransferPanel } from "@/components/transfer-panel";
import { TransferHistoryPanel } from "@/components/transfer-history-panel";
import { FailedRetentionSettings } from "@/components/failed-retention-settings";
import { jobMatches, transferDirection } from "@/lib/transfer-direction";

const job = (id: string, source_path: string, dest_path: string, status = "active") => ({
  id, source_path, dest_path, total_bytes: 1000, transferred_bytes: 500, status, priority: "normal", sort_order: 0, created_at: "2026-10-01T10:00:00Z",
  updated_at: "2026-10-01T10:00:00Z", error_message: null, retry_count: 0, speed_bps: 0, eta_seconds: null, verify_checksum: false,
});

const JOBS = [
  job("up", "/home/me/a.bin", "sftp://nas/backups/a.bin"),
  job("down", "s3://bucket/b.bin", "/home/me/b.bin"),
  job("loc", "/home/me/c.bin", "/mnt/usb/c.bin"),
];
const SYNC = { engine: "sync", id: "pair-1", title: "Sync Photos", detail: "copying · a.jpg", state: "running", fraction: 0.5, bytes_done: 5, bytes_total: 10, items_done: 5, items_total: 10, started_at: "2026-10-01T10:00:00Z", bytes_per_sec: 0, eta_secs: null, cancellable: true };
const LIVE = {
  files: [
    { path: "photos/new.jpg", state: "active", action: null, bytes: 0, error: null },
    { path: "photos/bad.jpg", state: "failed", action: "failed", bytes: 3, error: "access denied" },
    { path: "photos/old.jpg", state: "done", action: "copied", bytes: 2048, error: null },
  ],
  active_total: 1, finished_total: 2, failed_total: 1, truncated: false,
};

let handlers: Record<string, (a: Record<string, unknown>) => unknown>;
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => (a ?? {}) as Record<string, unknown>);

beforeEach(() => {
  mockInvoke.mockReset();
  localStorage.clear();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  handlers = {
    list_transfers: () => JOBS,
    operations_snapshot: () => ({ active: [SYNC], recent: [] }),
    get_sync_live_files: () => LIVE,
    get_throttle_config: () => ({ global_limit_bps: 0, per_connection_limit_bps: 0 }),
    get_failed_transfer_retention: () => ({ max_age_days: 30, max_total_kb: 4096 }),
    search_transfer_history: () => [],
    operations_cancel: () => true,
  };
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    const h = handlers[cmd];
    return h ? h(args ?? {}) : null;
  });
});

async function openFull() {
  render(<TransferPanel />);
  fireEvent.click(await screen.findByLabelText(/active transfers, click to expand/));
  await screen.findByRole("group", { name: "Transfer direction" });
}

const shownJobs = () => screen.queryAllByText(/\.bin/).length;

describe("transfer direction", () => {
  it("tells uploads, downloads and local copies apart", () => {
    expect(transferDirection("/a/b", "sftp://host/x")).toBe("upload");
    expect(transferDirection("s3://bkt/k", "/a/b")).toBe("download");
    expect(transferDirection("/a/b", "/c/d")).toBe("local");
    expect(transferDirection("C:\\a", "D:\\b")).toBe("local");
    expect(transferDirection("sftp://a/x", "s3://b/y")).toBe("remote");
    expect(jobMatches("all", "/a", "/b")).toBe(true);
    expect(jobMatches("sync", "/a", "sftp://h/b")).toBe(false);
  });
});

describe("transfer panel direction chips", () => {
  it("filters the queue by direction and shows running syncs under All and Sync", async () => {
    await openFull();
    expect(await screen.findByTestId("sync-group")).toBeInTheDocument();
    expect(shownJobs()).toBeGreaterThanOrEqual(3);

    fireEvent.click(screen.getByRole("button", { name: "Upload" }));
    expect(screen.queryByTestId("sync-group")).toBeNull();
    expect(screen.getAllByText(/a\.bin/).length).toBeGreaterThan(0);
    expect(screen.queryAllByText(/b\.bin/)).toHaveLength(0);
    expect(screen.queryAllByText(/c\.bin/)).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    expect(screen.getAllByText(/b\.bin/).length).toBeGreaterThan(0);
    expect(screen.queryAllByText(/a\.bin/)).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: "Local" }));
    expect(screen.getAllByText(/c\.bin/).length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Sync" }));
    expect(await screen.findByTestId("sync-group")).toBeInTheDocument();
    expect(screen.queryAllByText(/\.bin/)).toHaveLength(0);
  });

  it("says what is missing for a chip with nothing in it", async () => {
    handlers.list_transfers = () => [JOBS[2]];
    handlers.operations_snapshot = () => ({ active: [], recent: [] });
    await openFull();
    fireEvent.click(screen.getByRole("button", { name: "Upload" }));
    expect(await screen.findByText("No upload transfers")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sync" }));
    expect(await screen.findByText("No sync is running")).toBeInTheDocument();
  });
});

describe("running sync group", () => {
  it("opens to the files it is working on, with failures and their reason", async () => {
    await openFull();
    const group = await screen.findByTestId("sync-group");
    expect(calls("get_sync_live_files")).toEqual([]);
    fireEvent.click(within(group).getByRole("button", { name: /Show files of Sync Photos/ }));
    const list = await screen.findByRole("listbox", { name: "Files of this sync" });
    await waitFor(() => expect(within(list).getByText(/bad\.jpg/)).toBeInTheDocument());
    expect(within(list).getByText(/access denied/)).toBeInTheDocument();
    expect(within(list).getByText(/new\.jpg/)).toBeInTheDocument();
    expect(screen.getByText(/1 in progress · 2 finished · 1 failed/)).toBeInTheDocument();
    expect(calls("get_sync_live_files")[0]).toEqual({ pairId: "pair-1", limit: 200 });
    fireEvent.click(within(group).getByRole("button", { name: /Hide files of Sync Photos/ }));
    expect(screen.queryByRole("listbox", { name: "Files of this sync" })).toBeNull();
  });

  it("says when the run has ended and cancels a running sync", async () => {
    await openFull();
    const group = await screen.findByTestId("sync-group");
    fireEvent.click(within(group).getByRole("button", { name: /Show files/ }));
    handlers.get_sync_live_files = () => null;
    expect(await screen.findByText(/This sync has finished/, undefined, { timeout: 3000 })).toBeInTheDocument();
    fireEvent.click(within(group).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(calls("operations_cancel")).toEqual([{ engine: "sync", id: "pair-1" }]));
  });
});

describe("transfer history", () => {
  it("searches within the chosen dates and leaves all-time searches without dates", async () => {
    render(<TransferHistoryPanel />);
    await waitFor(() => expect(calls("search_transfer_history").length).toBeGreaterThan(0));
    expect(calls("search_transfer_history")[0]).not.toHaveProperty("dateFrom");
    fireEvent.click(screen.getByRole("button", { name: "Last 30 days" }));
    await waitFor(() => expect(calls("search_transfer_history").some((a) => typeof a.dateFrom === "string")).toBe(true));
    const last = calls("search_transfer_history").at(-1)!;
    expect(Date.parse(String(last.dateFrom))).toBeLessThanOrEqual(Date.now() - 28 * 86400_000);
  });

  it("exports with the same dates and shows times in local time", async () => {
    handlers.search_transfer_history = () => [
      { id: "h1", source_path: "/home/me/docs/a.txt", dest_path: "/mnt/usb/a.txt", total_bytes: 10, transferred_bytes: 10, status: "completed", started_at: "2026-10-01T10:00:00Z", completed_at: null, duration_ms: 1000, speed_bps: 10, error_message: null, connection_id: null, checksum_verified: false, checksum_match: null, conflict_policy: null, conflict_resolution: null, retry_count: 0 },
    ];
    handlers.export_transfer_history = () => "[]";
    (URL as unknown as { createObjectURL: () => string }).createObjectURL = () => "blob:x";
    (URL as unknown as { revokeObjectURL: () => void }).revokeObjectURL = () => undefined;
    render(<TransferHistoryPanel />);
    expect(await screen.findByText(new Date("2026-10-01T10:00:00Z").toLocaleString())).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Yesterday" }));
    await waitFor(() => expect(calls("search_transfer_history").some((a) => a.dateTo)).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "Export JSON" }));
    await waitFor(() => expect(calls("export_transfer_history").length).toBe(1));
    expect(calls("export_transfer_history")[0]).toMatchObject({ format: "json" });
    expect(typeof calls("export_transfer_history")[0].dateFrom).toBe("string");
  });

  it("opens the folders of a transfer", async () => {
    handlers.search_transfer_history = () => [
      { id: "h1", source_path: "/home/me/docs/a.txt", dest_path: "sftp://nas/a.txt", total_bytes: 10, transferred_bytes: 10, status: "completed", started_at: "2026-10-01T10:00:00Z", completed_at: null, duration_ms: 1000, speed_bps: 10, error_message: null, connection_id: null, checksum_verified: false, checksum_match: null, conflict_policy: null, conflict_resolution: null, retry_count: 0 },
    ];
    handlers.reveal_in_os = () => null;
    render(<TransferHistoryPanel />);
    fireEvent.click(await screen.findByLabelText("Open source of a.txt"));
    await waitFor(() => expect(calls("reveal_in_os")).toEqual([{ path: "/home/me/docs" }]));
    // A server path cannot be revealed in the system file manager.
    expect(screen.getByLabelText("Open destination of a.txt")).toBeDisabled();
  });
});

describe("failed transfer retention", () => {
  it("loads, saves and reports the limits", async () => {
    handlers.get_failed_transfer_retention = () => ({ max_age_days: 14, max_total_kb: 8192 });
    handlers.set_failed_transfer_retention = (a) => ({ max_age_days: a.maxAgeDays, max_total_kb: a.maxTotalKb });
    render(<FailedRetentionSettings />);
    const days = screen.getByLabelText("Keep failed transfers for days") as HTMLInputElement;
    const mb = screen.getByLabelText("Keep failed transfers up to MB") as HTMLInputElement;
    await waitFor(() => expect(days.value).toBe("14"));
    expect(mb.value).toBe("8");
    fireEvent.change(days, { target: { value: "7" } });
    fireEvent.change(mb, { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls("set_failed_transfer_retention")).toEqual([{ maxAgeDays: 7, maxTotalKb: 2048 }]));
    expect(await screen.findByRole("status")).toHaveTextContent("Saved");
  });

  it("shows why saving failed", async () => {
    handlers.set_failed_transfer_retention = () => {
      throw { message: "Could not save the setting.", advice: "Try again." };
    };
    render(<FailedRetentionSettings />);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save the setting.");
  });
});
