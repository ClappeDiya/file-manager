/**
 * Versions UI (K7, K22, K113, K69, K23, K20, K76, K4): version table with
 * pins, notes, storage added and tag filters; the retention editor's GFS
 * preset, count editor and live preview; copying versions; the source
 * picker and sharing section. IPC names and argument shapes match the Rust
 * handlers and serde types.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupVersionTable } from "@/components/backup/backup-version-list";
import { RetentionEditor } from "@/components/backup/backup-retention-editor";
import { BackupRestoreView } from "@/components/backup/backup-restore-view";
import { BackupSharing } from "@/components/backup/backup-sharing";
import {
  describeCounts,
  describeRetention,
  GFS_PRESET,
  matchesTags,
  newBackupJob,
  parseTags,
  useBackupStore,
  type BackupJob,
  type Fileset,
  type RetentionPolicy,
} from "@/stores/backup-store";

function version(id: number, extra: Partial<Fileset> = {}): Fileset {
  return {
    id,
    timestamp: `2026-10-${10 + id}T02:00:00Z`,
    volume_name: `v${id}`,
    partial: false,
    file_count: id,
    total_size: 1000 * id,
    source: "s1",
    note: null,
    tags: {},
    pinned: false,
    pin_reason: null,
    identity: { hostname: "laptop", username: "ann", job_id: "j1", source_label: "Docs" },
    stats: { bytes_added: 2048, new_blocks: 3, files_added: 1, files_changed: 0, files_deleted: 0 },
    ...extra,
  };
}

const job: BackupJob = { ...newBackupJob(), id: "j1", name: "Docs" };
const ok = (report: unknown) => ({ run_id: "r", status: "success", summary: "", report, log: [] });

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: false });
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useBackupStore.setState({ jobs: [job, { ...newBackupJob(), id: "j2", name: "Archive" }] });
});

describe("version helpers", () => {
  it("parses and matches tags like the Rust filter", () => {
    expect(parseTags("env=prod, release\nbad key=x, team = ops")).toEqual({ env: "prod", release: "", team: "ops" });
    expect(matchesTags({ env: "prod" }, { env: "" })).toBe(true);
    expect(matchesTags({ env: "prod" }, { env: "dev" })).toBe(false);
    expect(matchesTags({}, {})).toBe(true);
  });

  it("describes count-based retention", () => {
    expect(describeRetention(GFS_PRESET)).toBe("Keep 7 daily, 4 weekly, 12 monthly, 3 yearly");
    const zero: RetentionPolicy = { type: "counts", keep_latest: 0, keep_hourly: 0, keep_daily: 0, keep_weekly: 0, keep_monthly: 0, keep_yearly: 0 };
    expect(describeRetention(zero)).toBe("Keep all versions (no limits set)");
    expect(describeCounts({ ...(zero as Extract<RetentionPolicy, { type: "counts" }>), keep_latest: 2 })).toBe("Keep 2 latest");
  });
});

describe("BackupVersionTable", () => {
  it("pins and annotates versions and shows — for versions without statistics", async () => {
    const versions = [version(2), version(1, { stats: null, pinned: true, pin_reason: "audit" })];
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_set_version_metadata") return ok({ ...versions[args.version as number], ...(args.patch as object) });
      throw new Error(`unexpected ${cmd}`);
    });
    const changed = vi.fn(async () => undefined);
    render(<BackupVersionTable job={job} versions={versions} selected={0} onSelect={() => undefined} source={null} onChanged={changed} />);
    expect(screen.getByTestId("version-row-0").textContent).toContain("2.0 KB");
    expect(screen.getByTestId("version-row-1").textContent).toContain("—");
    expect(screen.getByTestId("version-pin-1").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByTestId("version-pin-0"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_set_version_metadata", { id: "j1", version: 0, source: null, patch: { note: null, tags: null, pinned: true, pin_reason: null } }),
    );
    expect(changed).toHaveBeenCalled();
    // Standard view: the note is editable.
    fireEvent.click(screen.getAllByText("Add note…")[0]);
    const input = screen.getByLabelText("Version note");
    fireEvent.change(input, { target: { value: "before upgrade" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_set_version_metadata", expect.objectContaining({ version: 0, patch: expect.objectContaining({ note: "before upgrade" }) })));
    // Tag filters and copying are Advanced only.
    expect(screen.queryByTestId("copy-versions-open")).toBeNull();
  });

  it("filters by tag chips and copies versions to another job (Advanced)", async () => {
    useUIStore.setState({ backupAdvanced: true });
    const versions = [version(3, { tags: { env: "prod" } }), version(2, { tags: { env: "dev" }, identity: { hostname: "old-pc", username: "ann", job_id: "j1", source_label: "Docs" } }), version(1, { tags: { env: "prod" } })];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "backup_execute") return ok({ dry_run: false, versions_copied: 2, versions_skipped: 0, files: 2, blocks_copied: 5, blocks_reused: 0, bytes_copied: 10, bytes_uploaded: 4096, rechunked: false, copied: [], warnings: [] });
      if (cmd === "backup_list_jobs" || cmd === "backup_status") return cmd === "backup_list_jobs" ? [job] : { running: true, paused: false, paused_until: null, tasks: [], next_runs: [] };
      throw new Error(`unexpected ${cmd}`);
    });
    render(<BackupVersionTable job={job} versions={versions} selected={0} onSelect={() => undefined} source={null} onChanged={async () => undefined} />);
    expect(screen.getByTestId("version-row-1").textContent).toContain("ann@old-pc");
    fireEvent.click(screen.getByRole("button", { name: "env=prod" }));
    expect(screen.queryByTestId("version-row-1")).toBeNull();
    expect(screen.getByTestId("version-row-2")).toBeTruthy();
    fireEvent.click(screen.getByText("Copy the 2 shown…"));
    expect(screen.getByTestId("copy-target-job")).toHaveProperty("value", "j2");
    fireEvent.click(screen.getByTestId("copy-versions-run"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "copy_versions", versions: [0, 2], source: null, target: { type: "job", job_id: "j2" }, dry_run: false } }),
    );
    expect(await screen.findByText(/Copied 2 version\(s\)/)).toBeTruthy();
  });
});

describe("RetentionEditor", () => {
  it("offers the GFS preset in Standard mode and previews the unsaved policy", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_preview_retention")
        return {
          policy: args.policy,
          legal_hold: false,
          keep: 1,
          delete: 1,
          versions: [
            { id: 2, timestamp: "2026-10-20T02:00:00Z", keep: true, reasons: ["newest version", "daily 7"], summary: "kept: newest version, daily 7" },
            { id: 1, timestamp: "2026-09-01T02:00:00Z", keep: false, reasons: ["not selected by any rule"], summary: "deleted: not selected by any rule" },
          ],
        };
      throw new Error(`unexpected ${cmd}`);
    });
    let current: RetentionPolicy = { type: "keep_all" };
    const onChange = vi.fn((r: RetentionPolicy) => (current = r));
    const { rerender } = render(<RetentionEditor job={{ ...job, retention: current }} isNew={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("backup-retention"), { target: { value: "counts" } });
    expect(onChange).toHaveBeenCalledWith(GFS_PRESET);
    rerender(<RetentionEditor job={{ ...job, retention: current }} isNew={false} onChange={onChange} />);
    expect(screen.getByTestId("retention-counts-summary").textContent).toContain("Keep 7 daily, 4 weekly");
    expect(screen.queryByLabelText("Keep daily")).toBeNull();
    const preview = await screen.findByTestId("retention-preview", {}, { timeout: 2000 });
    expect(preview.textContent).toContain("kept: newest version, daily 7");
    expect(preview.textContent).toContain("deleted: not selected by any rule");
    expect(mockInvoke).toHaveBeenCalledWith("backup_preview_retention", { id: "j1", policy: GFS_PRESET, source: null });
  });

  it("edits the counts in Advanced mode", () => {
    useUIStore.setState({ backupAdvanced: true });
    mockInvoke.mockResolvedValue({ policy: GFS_PRESET, legal_hold: false, keep: 0, delete: 0, versions: [] });
    const onChange = vi.fn();
    render(<RetentionEditor job={{ ...job, retention: GFS_PRESET }} isNew onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Keep daily"), { target: { value: "14" } });
    expect(onChange).toHaveBeenCalledWith({ ...GFS_PRESET, keep_daily: 14 });
  });
});

describe("shared repositories", () => {
  it("browses another source's versions in Advanced mode", async () => {
    useUIStore.setState({ backupAdvanced: true });
    const sources = [
      { id: "s1", identity: null, label: "ann@laptop (Docs)", versions: 1, newest: null, oldest: null, newest_size: 0, own: true },
      { id: "s2", identity: null, label: "bob@nas (Photos)", versions: 2, newest: null, oldest: null, newest_size: 0, own: false },
    ];
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_list_sources") return sources;
      if (cmd === "backup_list_versions") return args.source === "s2" ? [version(5, { source: "s2" }), version(4, { source: "s2" })] : [version(1)];
      if (cmd === "backup_list_files") return [];
      throw new Error(`unexpected ${cmd}`);
    });
    render(<BackupRestoreView job={job} />);
    const picker = await screen.findByLabelText("Source");
    fireEvent.change(picker, { target: { value: "s2" } });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_list_versions", { id: "j1", source: "s2" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_list_files", { id: "j1", version: 0, prefix: null, source: "s2" }));
    expect(screen.getByTestId("version-row-1")).toBeTruthy();
  });

  it("shares the repository from Maintenance", async () => {
    let shared = false;
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_execute") {
        const op = args.operation as { type: string };
        if (op.type === "sync_sources") return ok({ shared, sources: [{ id: "s1", identity: null, label: "ann@laptop (Docs)", versions: 3, newest: null, oldest: null, newest_size: 0, own: true }] });
        if (op.type === "enable_sharing") {
          shared = true;
          return ok({ already_shared: false, versions_assigned: 3, source: "s1" });
        }
      }
      if (cmd === "backup_list_jobs") return [job];
      if (cmd === "backup_status") return { running: true, paused: false, paused_until: null, tasks: [], next_runs: [] };
      throw new Error(`unexpected ${cmd}`);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<BackupSharing job={job} />);
    fireEvent.click(screen.getByTestId("sharing-refresh"));
    expect((await screen.findByTestId("sharing-sources")).textContent).toContain("ann@laptop");
    fireEvent.click(screen.getByTestId("sharing-enable"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "enable_sharing", claim_all: true } }));
    expect(await screen.findByText(/3 existing version\(s\) now belong to this job/)).toBeTruthy();
    expect(screen.getByText("This repository is shared.")).toBeTruthy();
  });
});
