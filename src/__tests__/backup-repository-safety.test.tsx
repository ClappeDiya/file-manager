/**
 * Repository safety UI (K115/K118/K9/K70/K81): lock status line and the
 * guarded Break lock, the one-click local database encryption, the
 * read-only toggle and badge, and the new option fields mapping onto the
 * Rust `BackupOptions` / `BackupJob` serde shapes.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupRepositorySafety } from "@/components/backup/backup-repository-safety";
import { BackupAdvancedOptions } from "@/components/backup/backup-advanced-options";
import { lockSummary, newBackupJob, type RepoLockEntry, type RepoLockStatus } from "@/stores/backup-store";

const LOCK: RepoLockEntry = {
  file: "ufop-lock.json",
  owner_id: "abc",
  hostname: "laptop",
  username: "me",
  pid: 42,
  app: "ufop-cli",
  version: "0.1.0",
  operation: "backup",
  kind: "exclusive",
  acquired_at: "2026-10-23T10:00:00Z",
  expires_at: "2026-10-23T10:30:00Z",
  stale: false,
  damaged: false,
};
const busy: RepoLockStatus = { locks: [LOCK], exclusive: true, readers: 0, checked_at: "2026-10-23T10:05:00Z" };
const free: RepoLockStatus = { locks: [], exclusive: false, readers: 0, checked_at: "2026-10-23T10:05:00Z" };

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: true });
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("lockSummary", () => {
  it("names the live holder, prefers the writer, ignores stale locks", () => {
    expect(lockSummary(null)).toBeNull();
    expect(lockSummary(free)).toBeNull();
    expect(lockSummary(busy)).toMatch(/^Repository in use by laptop \(backup\) since /);
    const reader = { ...LOCK, file: "ufop-lock-r-1.json", kind: "shared" as const, operation: "restore", hostname: "nas" };
    expect(lockSummary({ ...busy, locks: [reader, LOCK] })).toMatch(/laptop \(backup\).* and 1 other/);
    expect(lockSummary({ ...busy, locks: [{ ...LOCK, stale: true }] })).toBeNull();
  });
});

describe("BackupRepositorySafety", () => {
  it("shows who holds the lock and breaks it only after confirmation", async () => {
    const job = { ...newBackupJob(), id: "j1" };
    let status = busy;
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_lock_status") return status;
      if (cmd === "backup_break_lock") {
        expect(args).toEqual({ jobId: "j1", confirm: true, ownerId: "abc" });
        status = free;
        return { removed: ["ufop-lock.json"], holders: ["laptop (backup)"] };
      }
      throw new Error(`unexpected ${cmd}`);
    });
    render(<BackupRepositorySafety job={job} />);
    await waitFor(() => expect(screen.getByTestId("backup-lock-status").textContent).toContain("Repository in use by laptop (backup)"));
    fireEvent.click(screen.getByTestId("backup-break-lock"));
    expect(mockInvoke).not.toHaveBeenCalledWith("backup_break_lock", expect.anything());
    fireEvent.click(screen.getByTestId("backup-break-lock-confirm"));
    await waitFor(() => expect(screen.getByText("Removed 1 lock file(s).")).toBeInTheDocument());
    expect(screen.getByTestId("backup-lock-status").textContent).toContain("Not in use");
  });

  it("hides Break lock in Simple mode and for read-only jobs", async () => {
    mockInvoke.mockResolvedValue(busy);
    useUIStore.setState({ backupAdvanced: false });
    const { unmount } = render(<BackupRepositorySafety job={{ ...newBackupJob(), id: "j1" }} />);
    await waitFor(() => expect(screen.getByTestId("backup-lock-status").textContent).toContain("laptop"));
    expect(screen.queryByTestId("backup-break-lock")).toBeNull();
    unmount();
    useUIStore.setState({ backupAdvanced: true });
    render(<BackupRepositorySafety job={{ ...newBackupJob(), id: "j1", read_only: true }} />);
    await waitFor(() => expect(screen.getByTestId("backup-read-only-note")).toBeInTheDocument());
    expect(screen.queryByTestId("backup-break-lock")).toBeNull();
  });

  it("encrypts the local database in one click", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_lock_status") return free;
      if (cmd === "backup_set_database_encryption") {
        expect(args).toEqual({ jobId: "j1", enabled: true });
        return { encrypted: true, files: 10, settings: 0 };
      }
      if (cmd === "backup_list_jobs") return [];
      if (cmd === "backup_status") return { paused: false, paused_until: null, tasks: [], next_runs: [] };
      return null;
    });
    render(<BackupRepositorySafety job={{ ...newBackupJob(), id: "j1", encrypt_local_database: false }} />);
    expect(screen.getByTestId("backup-db-encryption").textContent).toContain("not encrypted");
    fireEvent.click(screen.getByTestId("backup-encrypt-db"));
    await waitFor(() => expect(screen.getByText("The local database is now encrypted.")).toBeInTheDocument());
  });
});

describe("options", () => {
  it("defaults match the Rust defaults", () => {
    const j = newBackupJob();
    expect(j).toMatchObject({ read_only: false, encrypt_local_database: null });
    expect(j.options).toMatchObject({ lock_ttl_secs: 1800, clock_skew_warn_secs: 300, clock_skew_abort_secs: 3600, checkpoint_interval_secs: 900, checkpoint_interval_bytes: 0 });
  });

  it("advanced fields map minutes and MB onto seconds and bytes", () => {
    const onChange = vi.fn();
    render(<BackupAdvancedOptions options={newBackupJob().options} locked={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("opt-checkpoint-secs"), { target: { value: "5" } });
    expect(onChange).toHaveBeenLastCalledWith({ checkpoint_interval_secs: 300 });
    fireEvent.change(screen.getByTestId("opt-checkpoint-bytes"), { target: { value: "100" } });
    expect(onChange).toHaveBeenLastCalledWith({ checkpoint_interval_bytes: 100 * 1048576 });
    fireEvent.change(screen.getByTestId("opt-skew-abort"), { target: { value: "0" } });
    expect(onChange).toHaveBeenLastCalledWith({ clock_skew_abort_secs: 0 });
    fireEvent.change(screen.getByTestId("opt-lock-ttl"), { target: { value: "10" } });
    expect(onChange).toHaveBeenLastCalledWith({ lock_ttl_secs: 600 });
  });
});
