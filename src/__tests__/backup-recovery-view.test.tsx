/**
 * Recovery tool and database tools: the UI drives the Rust commands with
 * the argument names the handlers expect and shows their reports.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupRecoveryView } from "@/components/backup/backup-recovery-view";
import { BackupSettingsView } from "@/components/backup/backup-settings-view";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("recovery tool", () => {
  it("downloads, indexes, lists and restores", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "backup_recovery_download":
          return { downloaded: 5, skipped_existing: 1, bytes: 2048, failed: ["v1: decryption failed"] };
        case "backup_recovery_index":
          return { block_volumes: 2, blocks: 40, damaged: [] };
        case "backup_recovery_list":
          return [{ version: 0, timestamp: "2026-10-21T10:00:00+00:00", partial: false, files: 3, volume: "ufop-x.dlist" }];
        case "backup_recovery_restore":
          return { version: "x", files_restored: 2, files_partial: ["/a/b.bin"], folders_created: 1, bytes_written: 100, errors: [] };
        default:
          return null;
      }
    });
    render(<BackupRecoveryView />);
    fireEvent.change(screen.getByTestId("recovery-source"), { target: { value: "/mnt/b" } });
    fireEvent.change(screen.getByTestId("recovery-folder"), { target: { value: "/tmp/rec" } });
    fireEvent.click(screen.getByTestId("recovery-download"));
    expect(await screen.findByText(/Downloaded 5 volume/)).toBeInTheDocument();
    expect(screen.getByText("Failed: v1: decryption failed")).toBeInTheDocument();
    const dl = mockInvoke.mock.calls.find((c) => c[0] === "backup_recovery_download")![1];
    expect(dl).toMatchObject({ folder: "/tmp/rec", prefix: "ufop", options: { threads: 4, resume: true, max_bytes_per_sec: 0 } });

    fireEvent.click(screen.getByTestId("recovery-index"));
    expect(await screen.findByText(/Indexed 40 block/)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("recovery-target"), { target: { value: "/tmp/out" } });
    fireEvent.click(screen.getByTestId("recovery-restore"));
    expect(await screen.findByText("Partial: /a/b.bin")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("backup_recovery_restore", { folder: "/tmp/rec", version: 0, paths: [], target: "/tmp/out" });
  });
});

describe("database tools", () => {
  it("lists databases and deletes orphans", async () => {
    let inv = [
      { job_id: "j1", job_name: "Docs", info: { path: "", schema_version: 2, latest_schema: 2, versions: 3, volumes: 4, blocks: 5, files: 6, size_bytes: 4096, integrity_ok: true, integrity: "ok" }, error: null },
      { job_id: "deadbeef00", job_name: null, info: null, error: "unreadable" },
    ];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "backup_database_inventory") return inv;
      if (cmd === "backup_cleanup_databases") {
        inv = inv.slice(0, 1);
        return { orphans_deleted: ["deadbeef00"], vacuumed: 1, migrated: 0, bytes_before: 8192, bytes_after: 4096, errors: [] };
      }
      if (cmd === "backup_get_global_defaults") return { options: {}, throttle: null, retry: null };
      return [];
    });
    render(<BackupSettingsView />);
    fireEvent.click(screen.getByTestId("db-inventory"));
    expect(await screen.findByText(/orphan deadbeef/)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("db-cleanup"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_cleanup_databases", { deleteOrphans: true, vacuum: true }));
    expect(await screen.findByText(/1 orphan\(s\) deleted/)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText(/orphan deadbeef/)).not.toBeInTheDocument());
  });
});
