/**
 * Defaults & destinations: only changed, non-per-job options are sent as
 * global defaults; the library add/remove calls hit the Rust commands.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupSettingsView } from "@/components/backup/backup-settings-view";
import { diffOptions } from "@/lib/backup-helpers";
import { newBackupJob } from "@/stores/backup-store";

describe("backup settings", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    (window as unknown as Record<string, unknown>).__TAURI__ = {};
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  it("diffOptions keeps changed shared options only", () => {
    const b = newBackupJob().options;
    const d = diffOptions(b, { ...b, upload_concurrency: 7, prefix: "other", block_size: 1 });
    expect(d).toEqual({ upload_concurrency: 7 });
  });

  it("saves defaults and manages the library", async () => {
    let lib: unknown[] = [];
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_get_global_defaults") return { options: { upload_concurrency: 4 }, throttle: null, retry: null };
      if (cmd === "backup_list_destinations") return lib;
      if (cmd === "backup_save_destination") {
        lib = [{ ...(args.destination as object), id: "d1" }];
        return lib[0];
      }
      if (cmd === "backup_delete_destination") {
        lib = [];
        return null;
      }
      return args.defaults ?? null;
    });
    render(<BackupSettingsView />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_get_global_defaults", undefined));
    fireEvent.click(await screen.findByTestId("backup-defaults-save"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_set_global_defaults", { defaults: { options: { upload_concurrency: 4 }, throttle: null, retry: null } }),
    );

    fireEvent.change(screen.getByTestId("library-name"), { target: { value: "NAS" } });
    fireEvent.change(screen.getByTestId("library-path"), { target: { value: "/mnt/nas" } });
    fireEvent.click(screen.getByTestId("library-add"));
    expect(await screen.findByText("NAS")).toBeInTheDocument();
    const saved = mockInvoke.mock.calls.find((c) => c[0] === "backup_save_destination")![1] as { destination: { destination: { path: string } } };
    expect(saved.destination.destination.path).toBe("/mnt/nas");
    fireEvent.click(screen.getByText("Remove"));
    await waitFor(() => expect(screen.getByText("No saved destinations yet.")).toBeInTheDocument());
  });
});
