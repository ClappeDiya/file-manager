/** WS-I row 73: sync run history shows what started a run, filters by date, lists the changed files and opens the folders. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SyncPanel } from "@/components/sync-panel";
import { DEFAULT_COPY_OPTIONS } from "@/lib/copy-options";
import type { SyncPair } from "@/stores/sync-store";

const pair: SyncPair = {
  id: "p1",
  name: "Photos nightly",
  source_path: "/home/me/Pictures",
  dest_path: "/mnt/nas/photos",
  mode: "one_way",
  enabled: true,
  last_run: null,
  trigger: "manual",
  filter: { include_patterns: [], exclude_patterns: [], include_regex: [], exclude_regex: [], min_size: 0, max_size: 0, file_types: [], exclude_file_types: [], rules: [] },
  conflict_policy: "create_conflict_copy",
  verify_mode: "fast",
  checksum_enabled: false,
  created_at: "2026-10-01T00:00:00Z",
  time_offset_secs: null,
  copy_options: { ...DEFAULT_COPY_OPTIONS },
  run_options: { settle_secs: 0, completion_marker: null, version_template: null },
  option_overrides: [],
};

const run = (id: string, trigger: string | undefined, extra: Record<string, unknown> = {}) => ({
  id, pair_id: "p1", pair_name: "Photos nightly", started_at: "2026-10-01T02:00:00Z", ended_at: "2026-10-01T02:01:00Z", duration_ms: 60000,
  files_added: 2, files_modified: 0, files_deleted: 0, files_skipped: 0, conflicts_resolved: 0, errors: 1, bytes_transferred: 1024,
  status: "partial_success", health: "yellow", error_messages: [], resumed: false, exit_code: 9, ...(trigger ? { trigger } : {}), ...extra,
});

let history: (a: Record<string, unknown>) => unknown;

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  history = () => [run("r1", "scheduled"), run("r2", "api"), run("r3", undefined)];
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "list_sync_pairs": return [pair];
      case "get_sync_health": return "green";
      case "get_sync_run_history": return history(args ?? {});
      case "get_sync_run_items":
        return { items: [{ rel_path: "a.jpg", action: "copied", bytes: 2048, error: null }, { rel_path: "b.jpg", action: "failed", bytes: 10, error: "access denied" }], total: 2, truncated: false };
      case "reveal_in_os": return null;
      default: return cmd.startsWith("list") || cmd.startsWith("get_sync_conflicts") ? [] : null;
    }
  });
});

const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

async function openHistory() {
  render(<SyncPanel />);
  fireEvent.click(screen.getByLabelText("Run history"));
  await screen.findAllByTestId("run-trigger");
}

describe("sync run history detail", () => {
  it("shows what started each run, and Manual for runs stored before triggers existed", async () => {
    await openHistory();
    expect(screen.getAllByTestId("run-trigger").map((e) => e.textContent)).toEqual(["Started: Scheduled", "Started: API", "Started: Manual"]);
  });

  it("filters by date range and leaves the all-time call as it was", async () => {
    await openHistory();
    expect(calls("get_sync_run_history")[0]).toEqual({ pairId: null, limit: 200 });
    fireEvent.click(screen.getByRole("button", { name: "Last 7 days" }));
    await waitFor(() => expect(calls("get_sync_run_history").length).toBe(2));
    const args = calls("get_sync_run_history")[1] as Record<string, unknown>;
    expect(args.pairId).toBeNull();
    expect(args.dateTo).toBeUndefined();
    expect(Date.parse(String(args.dateFrom))).toBeLessThanOrEqual(Date.now() - 5 * 86400_000);
    // Clearing the chip goes back to all time.
    fireEvent.click(screen.getByRole("button", { name: "Last 7 days" }));
    await waitFor(() => expect(calls("get_sync_run_history").length).toBe(3));
    expect(calls("get_sync_run_history")[2]).toEqual({ pairId: null, limit: 200 });
  });

  it("explains an empty range instead of showing nothing", async () => {
    await openHistory();
    history = () => [];
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    expect(await screen.findByText("No sync runs in this date range.")).toBeInTheDocument();
    // The chips stay so the range can be changed.
    expect(screen.getByRole("button", { name: "Today" })).toHaveAttribute("aria-pressed", "true");
  });

  it("lists the files a run changed, including why one failed", async () => {
    await openHistory();
    fireEvent.click(screen.getByLabelText("Changed files of report r1"));
    const list = await screen.findByRole("listbox", { name: "Changed files" });
    await waitFor(() => expect(within(list).getByText(/b\.jpg/)).toBeInTheDocument());
    expect(calls("get_sync_run_items")[0]).toMatchObject({ runId: "r1", offset: 0 });
    expect(within(list).getByText(/access denied/)).toBeInTheDocument();
    // Filter to the failures.
    fireEvent.click(screen.getByRole("button", { name: "failed 1" }));
    await waitFor(() => expect(within(screen.getByRole("listbox", { name: "Changed files" })).queryByText(/a\.jpg/)).toBeNull());
  });

  it("reveals the pair's source and destination folders", async () => {
    await openHistory();
    fireEvent.click(screen.getByLabelText("Open source of report r1"));
    await waitFor(() => expect(calls("reveal_in_os")).toEqual([{ path: "/home/me/Pictures" }]));
    fireEvent.click(screen.getByLabelText("Open destination of report r1"));
    await waitFor(() => expect(calls("reveal_in_os")[1]).toEqual({ path: "/mnt/nas/photos" }));
  });

  it("shows times in the computer's time zone", async () => {
    await openHistory();
    expect(screen.getAllByText(new RegExp(new Date("2026-10-01T02:00:00Z").toLocaleString().replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))).length).toBeGreaterThan(0);
  });
});
