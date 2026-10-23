/**
 * Backup panel + store: IPC contract (command names and argument shapes
 * matching the Rust handlers), filter text round-trip, and the main
 * create → run → restore UI flows against a mocked backend.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

import { BackupPanel } from "@/components/backup/backup-panel";
import { rulesToText, textToRules } from "@/lib/backup-helpers";
import { describeRetention, describeSchedule, newBackupJob, useBackupStore, type BackupJob } from "@/stores/backup-store";

const STATUS = { running: true, paused: false, paused_until: null, tasks: [], next_runs: [] };

function savedJob(): BackupJob {
  return {
    ...newBackupJob(),
    id: "job-1",
    name: "Documents",
    sources: ["/home/me/Docs"],
    destination: { type: "local", path: "/mnt/backup", alternate_paths: [], marker_file: null },
    stats: { ...newBackupJob().stats, version_count: 2, last_result: "success" },
  };
}

function backend(overrides: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  const jobs: BackupJob[] = [savedJob()];
  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    backup_list_jobs: () => jobs,
    backup_status: () => STATUS,
    backup_capabilities: () => ({ filter_groups: [{ name: "cache_files", description: "Caches", patterns: ["*/.cache/"] }], connection_protocols: ["sftp", "s3"], default_no_compress_extensions: [] }),
    list_connections: () => [],
    backup_run: () => "task-1",
    backup_delete_job: () => null,
    backup_get_desktop_settings: () => ({ run_in_background: false, start_at_login: false }),
    backup_set_desktop_settings: (a) => a.settings,
    backup_pause: () => ({ ...STATUS, paused: true }),
    backup_save_job: (a) => ({ ...(a.job as BackupJob), id: "job-2" }),
    backup_list_versions: () => [
      { id: 2, timestamp: "2026-10-20T02:00:00Z", volume_name: "v2", partial: false, file_count: 2, total_size: 10 },
      { id: 1, timestamp: "2026-10-19T02:00:00Z", volume_name: "v1", partial: false, file_count: 1, total_size: 5 },
    ],
    backup_list_files: () => [
      { path: "/home/me/Docs", kind: "dir", size: 0, mtime: null, mode: null, hash: null, blocks: [], symlink_target: null },
      { path: "/home/me/Docs/a.txt", kind: "file", size: 10, mtime: 1, mode: null, hash: "h", blocks: ["b"], symlink_target: null },
    ],
    backup_execute: () => ({ run_id: "r", status: "success", summary: "1 files restored", report: { files_restored: 1, bytes_restored: 10, files_skipped_matching: 0, files_skipped_conflict: 0, folders_created: 0, volumes_downloaded: 1, planned: [], warnings: [], errors: [], version: "x" }, log: [] }),
    ...overrides,
  };
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    const h = handlers[cmd];
    if (!h) throw { message: `unexpected command ${cmd}` };
    return h(args ?? {});
  });
}

beforeEach(() => {
  // Make `isTauriAvailable()` true so calls reach the mocked `invoke`.
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset();
  useBackupStore.setState({ jobs: [], status: STATUS, history: [], capabilities: null, loading: false, error: null });
});

describe("backup helpers", () => {
  it("round-trips filter rules through the text format", () => {
    const rules = textToRules("-*.tmp\n-{cache_files}\n-[\\.bak\\d+$]\n+/home/me/keep.log\nnot a rule");
    expect(rules).toEqual([
      { action: "exclude", kind: "wildcard", pattern: "*.tmp" },
      { action: "exclude", kind: "group", pattern: "cache_files" },
      { action: "exclude", kind: "regex", pattern: "\\.bak\\d+$" },
      { action: "include", kind: "exact", pattern: "/home/me/keep.log" },
    ]);
    expect(textToRules(rulesToText(rules))).toEqual(rules);
  });

  it("describes schedules and retention", () => {
    expect(describeSchedule({ enabled: false, start_time: "02:00", interval_minutes: 1440, weekdays: [], skip_on_battery: false, calendar_ids: [] })).toBe("Manual only");
    expect(describeSchedule({ enabled: true, start_time: "02:00", interval_minutes: 1440, weekdays: [0, 4], skip_on_battery: false, calendar_ids: [] })).toBe("daily from 02:00 on Mon, Fri");
    expect(describeSchedule({ enabled: true, start_time: "00:30", interval_minutes: 60, weekdays: [], skip_on_battery: false, calendar_ids: [] })).toBe("every 1 h from 00:30");
    expect(describeRetention({ type: "keep_count", count: 5 })).toBe("Keep 5 version(s)");
  });

  it("new jobs mirror the Rust defaults that the backend validates", () => {
    const j = newBackupJob();
    expect(j.id).toBe("");
    expect(j.encrypted).toBe(true);
    expect(j.options.block_size).toBe(102400);
    expect(j.retention).toEqual({ type: "keep_all" });
    expect(j.destination.type).toBe("local");
  });
});

describe("backup store", () => {
  it("sends camelCase IPC args the Tauri handlers expect", async () => {
    backend();
    await useBackupStore.getState().runJob("job-1", true);
    expect(mockInvoke).toHaveBeenCalledWith("backup_run", { id: "job-1", dryRun: true });
    await useBackupStore.getState().deleteJob("job-1", true, false);
    expect(mockInvoke).toHaveBeenCalledWith("backup_delete_job", { id: "job-1", deleteLocalDb: true, deleteRemote: false });
    await useBackupStore.getState().pause(30);
    expect(mockInvoke).toHaveBeenCalledWith("backup_pause", { minutes: 30 });
  });

  it("keeps the stored passphrase when none is given", async () => {
    backend();
    await useBackupStore.getState().saveJob(savedJob());
    expect(mockInvoke).toHaveBeenCalledWith("backup_save_job", expect.objectContaining({ passphrase: null }));
  });

  it("surfaces backend errors with their advice", async () => {
    backend({ backup_list_jobs: () => Promise.reject({ message: "Database error", advice: "Restart the app." }) });
    await useBackupStore.getState().loadJobs();
    expect(useBackupStore.getState().error).toBe("Database error Restart the app.");
  });
});

describe("BackupPanel", () => {
  it("lists jobs from the backend and starts a run", async () => {
    backend();
    render(<BackupPanel />);
    expect(await screen.findByText("Documents")).toBeTruthy();
    expect(screen.getByText("2 version(s)")).toBeTruthy();
    fireEvent.click(screen.getByTestId("backup-run-now"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_run", { id: "job-1", dryRun: false }));
  });

  it("toggles tray mode and start at login", async () => {
    backend();
    render(<BackupPanel />);
    const box = await screen.findByLabelText("Start at login");
    fireEvent.click(box);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_set_desktop_settings", { settings: { run_in_background: false, start_at_login: true } }),
    );
  });

  it("creates a job through the wizard", async () => {
    backend();
    render(<BackupPanel />);
    await screen.findByText("Documents");
    fireEvent.click(screen.getByTestId("backup-new"));
    fireEvent.change(screen.getByTestId("backup-name"), { target: { value: "Photos" } });
    fireEvent.change(screen.getByTestId("backup-passphrase"), { target: { value: "correct horse battery staple" } });
    fireEvent.click(screen.getByText("Sources"));
    fireEvent.change(screen.getByTestId("backup-sources"), { target: { value: "/home/me/Photos" } });
    fireEvent.click(screen.getByText("Destination"));
    fireEvent.change(screen.getByTestId("backup-dest-path"), { target: { value: "/mnt/usb" } });
    fireEvent.click(screen.getByTestId("backup-save"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "backup_save_job",
        expect.objectContaining({
          passphrase: "correct horse battery staple",
          job: expect.objectContaining({ name: "Photos", sources: ["/home/me/Photos"], destination: { type: "local", path: "/mnt/usb", alternate_paths: [], marker_file: null } }),
        }),
      ),
    );
  });

  it("read-only jobs show a badge, cannot run, and the editor toggles the flag", async () => {
    backend({ backup_list_jobs: () => [{ ...savedJob(), read_only: true }] });
    render(<BackupPanel />);
    expect(await screen.findByTestId("read-only-badge")).toBeTruthy();
    expect(screen.getByTestId("backup-run-now")).toBeDisabled();
    fireEvent.click(screen.getByTestId("backup-new"));
    fireEvent.change(screen.getByTestId("backup-name"), { target: { value: "Viewer" } });
    fireEvent.change(screen.getByTestId("backup-passphrase"), { target: { value: "correct horse battery staple" } });
    fireEvent.click(screen.getByText("Sources"));
    fireEvent.change(screen.getByTestId("backup-sources"), { target: { value: "/home/me/Photos" } });
    fireEvent.click(screen.getByText("Destination"));
    fireEvent.change(screen.getByTestId("backup-dest-path"), { target: { value: "/mnt/usb" } });
    fireEvent.click(screen.getByLabelText(/Read-only \(restore and browse only\)/));
    fireEvent.click(screen.getByTestId("backup-save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_save_job", expect.objectContaining({ job: expect.objectContaining({ read_only: true }) })));
  });

  it("restores selected files from a chosen version", async () => {
    backend();
    render(<BackupPanel />);
    await screen.findByText("Documents");
    fireEvent.click(screen.getByTestId("backup-open-restore"));
    const list = await screen.findByTestId("backup-file-list");
    await waitFor(() => expect(list.textContent).toContain("a.txt"));
    fireEvent.change(screen.getByTestId("backup-version-select"), { target: { value: "1" } });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_list_files", { id: "job-1", version: 1, prefix: null }));
    const box = (await screen.findAllByRole("checkbox")).find((c) => c.closest("li")?.textContent?.includes("a.txt"))!;
    fireEvent.click(box);
    fireEvent.click(screen.getByTestId("backup-restore-run"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_execute", {
        id: "job-1",
        operation: {
          type: "restore",
          options: expect.objectContaining({ version: 1, paths: ["/home/me/Docs/a.txt"], target: null, conflict: "overwrite" }),
          overrides: { download_bps: null, retries: null, network_timeout_secs: null },
        },
      }),
    );
    expect(await screen.findByText(/Restored 1 file/)).toBeTruthy();
  });
});
