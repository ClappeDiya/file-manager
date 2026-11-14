/** The sync panel's History view reads the stored run history. */
import { beforeEach, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SyncPanel } from "@/components/sync-panel";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

it("shows stored runs of every pair", async () => {
  const run = {
    id: "r1", pair_id: "p1", pair_name: "Photos nightly", started_at: "2026-10-01T02:00:00Z", ended_at: "2026-10-01T02:01:00Z", duration_ms: 60000,
    files_added: 3, files_modified: 0, files_deleted: 0, files_skipped: 0, conflicts_resolved: 0, errors: 0, bytes_transferred: 1024,
    status: "success", health: "healthy", error_messages: [], resumed: false, exit_code: 1,
  };
  mockInvoke.mockImplementation(async (cmd: string) => (cmd === "get_sync_run_history" ? [run] : cmd.startsWith("list") || cmd.startsWith("get_sync_conflicts") ? [] : null));
  render(<SyncPanel />);
  fireEvent.click(screen.getByLabelText("Run history"));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("get_sync_run_history", { pairId: null, limit: 200 }));
  expect(await screen.findByText("Photos nightly")).toBeInTheDocument();
});
