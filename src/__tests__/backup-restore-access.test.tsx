/**
 * Restore and access features (Kopia parity WP5): restore as of a time,
 * mirror restore with its confirmation flow, "Download as ZIP…", file
 * preview, "Open as drive" and the volume cache settings — against a mocked
 * backend, checking the command names and argument shapes the Rust handlers take.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a), convertFileSrc: (p: string) => `asset://${p}` }));

import { BackupRestoreView } from "@/components/backup/backup-restore-view";
import { BackupCacheSettings } from "@/components/backup/backup-cache-settings";
import { archiveFileName, deletionSummary, versionArgs } from "@/stores/backup-access";
import { newBackupJob, useBackupStore, type BackupJob } from "@/stores/backup-store";
import { useUIStore } from "@/stores/ui-store";

const STATUS = { running: true, paused: false, paused_until: null, tasks: [], next_runs: [] };
const job: BackupJob = { ...newBackupJob(), id: "job-1", name: "My Docs", sources: ["/home/me/Docs"] };
const REPORT = { version: "x", files_restored: 1, folders_created: 0, files_skipped_matching: 0, files_skipped_conflict: 0, files_skipped_duplicate: 0, bytes_restored: 5, volumes_downloaded: 1, blocks_from_local: 0, planned: [], planned_deletions: [], files_deleted: 0, folders_deleted: 0, owners_restored: 0, sparse_files: 0, stop_reason: null, version_index: 1, archive: null, warnings: [], errors: [] };

type Handler = (a: Record<string, unknown>) => unknown;
function backend(extra: Record<string, Handler> = {}) {
  const handlers: Record<string, Handler> = {
    backup_list_jobs: () => [job],
    backup_status: () => STATUS,
    backup_list_sources: () => [],
    backup_list_drives: () => [],
    backup_list_versions: () => [
      { id: 3, timestamp: "2026-10-20T12:00:00Z", volume_name: "v3", partial: false, file_count: 2, total_size: 10, tags: {}, pinned: false },
      { id: 2, timestamp: "2026-10-19T12:00:00Z", volume_name: "v2", partial: false, file_count: 2, total_size: 10, tags: {}, pinned: false },
      { id: 1, timestamp: "2026-10-18T12:00:00Z", volume_name: "v1", partial: false, file_count: 1, total_size: 5, tags: {}, pinned: false },
    ],
    backup_list_files: () => [
      { path: "/home/me/Docs", kind: "dir", size: 0, mtime: null, mode: null, hash: null, blocks: [], symlink_target: null },
      { path: "/home/me/Docs/a.txt", kind: "file", size: 5, mtime: 1, mode: null, hash: "h", blocks: ["b"], symlink_target: null },
    ],
    backup_execute: () => ({ run_id: "r", status: "success", summary: "", report: REPORT, log: [] }),
    ...extra,
  };
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    const h = handlers[cmd];
    if (!h) throw { message: `unexpected command ${cmd}` };
    return h(args ?? {});
  });
}

const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1] as Record<string, unknown>);

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset();
  useUIStore.setState({ backupAdvanced: false });
  useBackupStore.setState({ jobs: [job], status: STATUS, history: [], capabilities: null, loading: false, error: null });
});

async function open() {
  render(<BackupRestoreView job={job} />);
  const list = await screen.findByTestId("backup-file-list");
  await waitFor(() => expect(list.textContent).toContain("a.txt"));
}

describe("helpers", () => {
  it("summarises deletions, names archives and builds version args", () => {
    expect(deletionSummary(["/a", "/b", "/d/"])).toBe("2 files and 1 folder");
    expect(deletionSummary(["/a"])).toBe("1 file");
    expect(deletionSummary(["/d/", "/e/"])).toBe("2 folders");
    expect(archiveFileName("My Docs", "2026-10-20T12:00:00+00:00")).toBe("My_Docs-2026-10-20_12-00-00.zip");
    expect(versionArgs(2)).toEqual({ version: 2, selector: null });
    expect(versionArgs({ type: "latest" })).toEqual({ version: null, selector: { type: "latest" } });
  });
});

describe("restore view", () => {
  it("restores as it was at a time and says which version that is", async () => {
    backend();
    await open();
    fireEvent.change(screen.getByTestId("restore-as-of-input"), { target: { value: "2026-10-19T23:00" } });
    expect(await screen.findByTestId("restore-as-of-result")).toHaveTextContent(/Uses the version from/);
    await waitFor(() => expect(calls("backup_list_files").some((a) => a.version === 1)).toBe(true));
    fireEvent.click(screen.getByTestId("backup-restore-run"));
    await waitFor(() => expect(calls("backup_execute")).toHaveLength(1));
    const op = calls("backup_execute")[0].operation as { options: Record<string, unknown> };
    expect(op.options).toMatchObject({ version: 1, selector: { type: "as_of", time: new Date("2026-10-19T23:00").toISOString() } });
    expect(await screen.findByText(/Restored 1 file/)).toBeInTheDocument();
    // Too early: no version.
    fireEvent.change(screen.getByTestId("restore-as-of-input"), { target: { value: "2020-01-01T00:00" } });
    expect(await screen.findByTestId("restore-as-of-result")).toHaveTextContent(/No complete version/);
  });

  it("runs a mirror restore only after the deletion list is confirmed", async () => {
    let n = 0;
    backend({
      backup_execute: (a) => {
        const op = a.operation as { options: { dry_run: boolean } };
        n++;
        return { run_id: "r", status: "success", summary: "", report: op.options.dry_run ? { ...REPORT, planned: ["x"], planned_deletions: ["/r/Docs/junk.tmp", "/r/Docs/old/"] } : { ...REPORT, files_deleted: 1, folders_deleted: 1 }, log: [] };
      },
      request_destructive_confirmation: (a) => ({ token: "tok-1", operation: a.operation, expires_in_seconds: 30 }),
      safety_assess_intent: () => ({ level: "low", reasons: [], baseline_sample_size: 0, baseline_median_files: 0, baseline_median_bytes: 0, intent_hash: "h", requires_confirmation: false }),
    });
    await open();
    fireEvent.click(screen.getByLabelText("Another folder"));
    fireEvent.change(screen.getByTestId("restore-target"), { target: { value: "/r" } });
    fireEvent.click(screen.getByLabelText(/Mirror: delete files/));
    fireEvent.click(screen.getByTestId("backup-restore-run"));
    const box = await screen.findByTestId("mirror-confirm");
    expect(box).toHaveTextContent("1 file and 1 folder will be deleted");
    expect(box).toHaveTextContent("/r/Docs/junk.tmp");
    expect(n).toBe(1);
    const dry = calls("backup_execute")[0];
    expect((dry.operation as { options: Record<string, unknown> }).options).toMatchObject({ dry_run: true, delete_extraneous: true, target: "/r" });
    expect(dry.confirmationToken).toBeNull();
    fireEvent.click(screen.getByTestId("mirror-confirm-delete"));
    await waitFor(() => expect(calls("backup_execute")).toHaveLength(2));
    expect(calls("request_destructive_confirmation")[0]).toMatchObject({ operation: "backup_mirror_restore" });
    const real = calls("backup_execute")[1];
    expect(real.confirmationToken).toBe("tok-1");
    expect(real.operation).toMatchObject({ options: { dry_run: false, delete_extraneous: true }, overrides: { confirm_delete: true } });
    expect(await screen.findByText(/deleted 1 file\(s\) and 1 folder/)).toBeInTheDocument();
  });

  it("cancelling the mirror list deletes nothing", async () => {
    backend({ backup_execute: () => ({ run_id: "r", status: "success", summary: "", report: { ...REPORT, planned: ["x"], planned_deletions: ["/r/x"] }, log: [] }) });
    await open();
    fireEvent.click(screen.getByLabelText("Another folder"));
    fireEvent.change(screen.getByTestId("restore-target"), { target: { value: "/r" } });
    fireEvent.click(screen.getByLabelText(/Mirror: delete files/));
    fireEvent.click(screen.getByTestId("backup-restore-run"));
    await screen.findByTestId("mirror-confirm");
    fireEvent.click(screen.getByText("Cancel"));
    await waitFor(() => expect(screen.queryByTestId("mirror-confirm")).toBeNull());
    expect(calls("backup_execute")).toHaveLength(1);
    expect(calls("request_destructive_confirmation")).toHaveLength(0);
  });

  it("downloads the selection as a ZIP through the save dialog", async () => {
    backend({ pick_save_file: () => "/tmp/out.zip", backup_execute: () => ({ run_id: "r", status: "success", summary: "", report: { ...REPORT, archive: "/tmp/out.zip" }, log: [] }) });
    await open();
    fireEvent.click(screen.getByTestId("backup-download-archive"));
    await waitFor(() => expect(calls("backup_execute")).toHaveLength(1));
    expect(calls("pick_save_file")[0]).toMatchObject({ defaultName: "My_Docs-2026-10-20_12-00-00.zip" });
    expect(calls("backup_execute")[0].operation).toMatchObject({ type: "restore", options: { archive: { path: "/tmp/out.zip", format: null }, version: 0, paths: [] } });
    expect(await screen.findByText(/Saved the archive \/tmp\/out.zip/)).toBeInTheDocument();
  });

  it("previews a backed-up file in the preview pane", async () => {
    backend({
      backup_preview_file: () => ({ path: "/home/me/Docs/a.txt", local_path: "/cache/p/a.txt", bytes: 5, version: 0, timestamp: "2026-10-20T12:00:00Z" }),
      preview_file: () => ({ path: "/cache/p/a.txt", file_name: "a.txt", file_size: 5, mime_type: "text/plain", preview_type: "text", content: "hello", metadata: { size: 5, size_human: "5 B", created: null, modified: null, accessed: null, is_readonly: false, is_symlink: false, permissions: null, exif: null }, is_metadata_only: false, render_hint: "" }),
    });
    await open();
    fireEvent.click(screen.getByTestId("backup-file-preview-open"));
    expect(await screen.findByTestId("backup-file-preview")).toBeInTheDocument();
    await waitFor(() => expect(calls("backup_preview_file")[0]).toMatchObject({ id: "job-1", path: "/home/me/Docs/a.txt", version: 0, selector: null }));
    await waitFor(() => expect(calls("preview_file")[0]).toMatchObject({ path: "/cache/p/a.txt" }));
    fireEvent.click(screen.getByTestId("backup-file-preview-close"));
    await waitFor(() => expect(screen.queryByTestId("backup-file-preview")).toBeNull());
  });

  it("opens a version as a drive and unmounts it", async () => {
    const drive = { id: "d1", job_id: "job-1", job_name: "My Docs", url: "http://127.0.0.1:4000/tok/", username: "ufop", password: "tok", version: 0, timestamp: "2026-10-20T12:00:00Z", all_versions: false, mount_point: null, mounted_with: null, mount_error: "gio is not installed", started_at: "", idle_timeout_minutes: 0 };
    let open_ = false;
    backend({
      backup_open_drive: () => {
        open_ = true;
        return drive;
      },
      backup_list_drives: () => (open_ ? [drive] : []),
      backup_close_drive: () => {
        open_ = false;
        return true;
      },
    });
    await open();
    fireEvent.click(screen.getByTestId("backup-open-drive"));
    const list = await screen.findByTestId("backup-open-drives");
    expect(list).toHaveTextContent("http://127.0.0.1:4000/tok/");
    expect(list).toHaveTextContent("gio is not installed");
    expect(calls("backup_open_drive")[0]).toMatchObject({ id: "job-1", request: { version: 0, all_versions: false } });
    fireEvent.click(screen.getByTestId("backup-unmount-d1"));
    await waitFor(() => expect(screen.queryByTestId("backup-open-drives")).toBeNull());
    expect(calls("backup_close_drive")[0]).toEqual({ driveId: "d1" });
  });

  it("sends owner, error policy and sparse settings in Advanced mode", async () => {
    useUIStore.setState({ backupAdvanced: true });
    backend();
    await open();
    fireEvent.change(screen.getByLabelText("Restore file owners"), { target: { value: "never" } });
    fireEvent.change(screen.getByLabelText("Sparse files"), { target: { value: "off" } });
    fireEvent.change(screen.getByLabelText("Data is missing or damaged"), { target: { value: "continue" } });
    fireEvent.change(screen.getByLabelText("Stop after more than N problems"), { target: { value: "5" } });
    fireEvent.click(screen.getByLabelText(/fail fast/));
    fireEvent.click(screen.getByTestId("backup-restore-run"));
    await waitFor(() => expect(calls("backup_execute")).toHaveLength(1));
    expect((calls("backup_execute")[0].operation as { options: Record<string, unknown> }).options).toMatchObject({
      restore_owner: "never",
      sparse: "off",
      error_policy: { on_missing_data: "continue", on_write_error: "fail", max_errors: 5, fail_fast: true },
    });
  });
});

describe("volume cache settings", () => {
  it("shows usage, clears and saves the limit", async () => {
    let entries = 3;
    backend({
      backup_cache_stats: () => ({ dir: "/c", max_bytes: 2 * 1024 ** 3, entries, bytes: entries * 1024 ** 2, hits: 3, misses: 1, corrupt: 0, evictions: 0, hit_rate: 0.75 }),
      backup_get_cache_settings: () => ({ dir: null, max_bytes: 2 * 1024 ** 3, use_for_restore: false }),
      backup_clear_cache: () => {
        entries = 0;
        return 3 * 1024 ** 2;
      },
      backup_set_cache_settings: (a) => ({ dir: "/c", max_bytes: (a.settings as { max_bytes: number }).max_bytes, entries: 0, bytes: 0, hits: 0, misses: 0, corrupt: 0, evictions: 0, hit_rate: 0 }),
    });
    useUIStore.setState({ backupAdvanced: true });
    render(<BackupCacheSettings />);
    expect(await screen.findByTestId("backup-cache-usage")).toHaveTextContent(/3 volume\(s\) · 75% served from the cache/);
    fireEvent.click(screen.getByTestId("backup-cache-clear"));
    expect(await screen.findByText(/Freed 3/)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("backup-cache-size"), { target: { value: "1" } });
    fireEvent.click(screen.getByTestId("backup-cache-save"));
    await waitFor(() => expect(calls("backup_set_cache_settings")[0]).toEqual({ settings: { dir: null, max_bytes: 1024 ** 3, use_for_restore: false } }));
  });
});
