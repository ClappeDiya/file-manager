/**
 * Sources and runs (WP4: K26/K28/K12/K29/K11/K6/K10/K54/K31/K14/K5): the new
 * option fields map onto the Rust serde shapes, the editors change them,
 * Estimate and Back up all now call the right IPC commands, and the
 * summaries read well.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SourcesAndRunsOptions, StdinCommandEditor } from "@/components/backup/backup-run-options";
import { BackupAdvancedOptions } from "@/components/backup/backup-advanced-options";
import { EstimatePanel, RunAllButton } from "@/components/backup/backup-estimate";
import { useEstimate } from "@/components/backup/use-estimate";
import { describeEstimate, describeRunAll } from "@/lib/backup-helpers";
import { newBackupJob, useBackupStore, type BackupEstimate, type BackupOptions } from "@/stores/backup-store";

const EST: BackupEstimate = {
  files: 10,
  folders: 2,
  bytes_examined: 5_000_000,
  new_files: 2,
  changed_files: 1,
  unchanged_files: 7,
  deleted_files: 1,
  new_bytes_at_most: 3 * 1024 * 1024,
  new_bytes_at_least: 1024 * 1024,
  upper_bound: true,
  has_previous_version: true,
  bytes_per_sec: 1024 * 1024,
  speed_source: "history",
  estimated_duration_secs: 125,
  largest: [{ path: "/data/big.iso", size: 2 * 1024 * 1024, status: "new" }],
  warnings: [],
};

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: true });
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("defaults match the Rust serde shapes", () => {
  it("new jobs carry every new field with the engine defaults", () => {
    const j = newBackupJob();
    expect(j.filter).toMatchObject({ ignore_files: [".ufopignore"], respect_gitignore: false });
    expect(j.options).toMatchObject({
      one_file_system: false,
      error_policy: { on_file_error: "continue", on_dir_error: "continue", on_unknown_entry: "ignore", fail_fast: false, max_errors: null },
      rehash_percent: 0,
      max_upload_bytes_per_run: null,
      parallel_file_threshold: 256 * 1024 * 1024,
    });
    expect(j.options.snapshot.provider).toBe("auto");
    expect(j.stdin_command).toBeNull();
  });
});

describe("SourcesAndRunsOptions", () => {
  it("edits the error policy, rehash, upload cap and parallel threshold", () => {
    const onChange = vi.fn();
    const o = newBackupJob().options;
    const { rerender } = render(<SourcesAndRunsOptions options={o} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText(/Stay on one filesystem/));
    expect(onChange).toHaveBeenLastCalledWith({ one_file_system: true });
    fireEvent.change(screen.getByTestId("opt-on-file-error"), { target: { value: "fail" } });
    expect(onChange).toHaveBeenLastCalledWith({ error_policy: { ...o.error_policy, on_file_error: "fail" } });
    fireEvent.change(screen.getByTestId("opt-on-unknown"), { target: { value: "warn" } });
    expect(onChange).toHaveBeenLastCalledWith({ error_policy: { ...o.error_policy, on_unknown_entry: "warn" } });
    fireEvent.click(screen.getByLabelText(/fail fast/));
    expect(onChange).toHaveBeenLastCalledWith({ error_policy: { ...o.error_policy, fail_fast: true } });
    fireEvent.change(screen.getByTestId("opt-max-errors"), { target: { value: "5" } });
    expect(onChange).toHaveBeenLastCalledWith({ error_policy: { ...o.error_policy, max_errors: 5 } });
    fireEvent.change(screen.getByTestId("opt-rehash"), { target: { value: "250" } });
    expect(onChange).toHaveBeenLastCalledWith({ rehash_percent: 100 });
    fireEvent.change(screen.getByTestId("opt-max-upload"), { target: { value: "2" } });
    expect(onChange).toHaveBeenLastCalledWith({ max_upload_bytes_per_run: 2 * 1024 ** 3 });
    rerender(<SourcesAndRunsOptions options={{ ...o, max_upload_bytes_per_run: 2 * 1024 ** 3 }} onChange={onChange} />);
    expect((screen.getByTestId("opt-max-upload") as HTMLInputElement).value).toBe("2");
    fireEvent.change(screen.getByTestId("opt-max-upload"), { target: { value: "" } });
    expect(onChange).toHaveBeenLastCalledWith({ max_upload_bytes_per_run: null });
    fireEvent.change(screen.getByTestId("opt-parallel-threshold"), { target: { value: "0" } });
    expect(onChange).toHaveBeenLastCalledWith({ parallel_file_threshold: 0 });
  });

  it("copes with settings saved before these fields existed", () => {
    const o = { ...newBackupJob().options } as Partial<BackupOptions>;
    delete o.error_policy;
    delete o.parallel_file_threshold;
    render(<SourcesAndRunsOptions options={o as BackupOptions} onChange={() => {}} />);
    expect((screen.getByTestId("opt-parallel-threshold") as HTMLInputElement).value).toBe("256");
  });

  it("is part of the advanced options, next to the snapshot method", () => {
    const onChange = vi.fn();
    const o = { ...newBackupJob().options, snapshot_policy: "auto" as const };
    render(<BackupAdvancedOptions options={o} locked={false} onChange={onChange} />);
    expect(screen.getByTestId("opt-on-file-error")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("opt-snapshot-provider"), { target: { value: "zfs" } });
    expect(onChange).toHaveBeenLastCalledWith({ snapshot: { ...o.snapshot, provider: "zfs" } });
  });
});

describe("StdinCommandEditor", () => {
  it("sets and clears the command and shows the stored path", () => {
    const onChange = vi.fn();
    const { rerender } = render(<StdinCommandEditor value={null} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("stdin-command"), { target: { value: "pg_dump mydb" } });
    expect(onChange).toHaveBeenLastCalledWith({ command: "pg_dump mydb", path: "", timeout_secs: 0 });
    rerender(<StdinCommandEditor value={{ command: "pg_dump mydb", path: "mydb.sql", timeout_secs: 0 }} onChange={onChange} />);
    expect(screen.getByText("Stored as /stdin/mydb.sql")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("stdin-timeout"), { target: { value: "600" } });
    expect(onChange).toHaveBeenLastCalledWith({ command: "pg_dump mydb", path: "mydb.sql", timeout_secs: 600 });
    rerender(<StdinCommandEditor value={{ command: "x", path: "", timeout_secs: 0 }} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("stdin-command"), { target: { value: "" } });
    expect(onChange).toHaveBeenLastCalledWith(null);
  });
});

describe("estimate", () => {
  it("summarises at most / at least and the time", () => {
    expect(describeEstimate(EST)).toBe("2 new, 1 changed of 10 files: at most 3.0 MB to back up, about 2 min 5 s");
    expect(describeEstimate({ ...EST, new_files: 0, changed_files: 0, deleted_files: 0 })).toBe("No new or changed files (10 checked)");
    expect(describeEstimate({ ...EST, upper_bound: false, estimated_duration_secs: null })).toBe("2 new, 1 changed of 10 files: 3.0 MB to back up");
  });

  it("the Estimate button calls backup_estimate and shows the largest files", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      expect(cmd).toBe("backup_estimate");
      expect(args).toEqual({ jobId: "j1", assumedBytesPerSec: null });
      return EST;
    });
    function Card() {
      const e = useEstimate("j1");
      return (
        <div>
          {e.button}
          {e.panel}
        </div>
      );
    }
    render(<Card />);
    fireEvent.click(screen.getByTestId("backup-estimate-btn"));
    await waitFor(() => expect(screen.getByTestId("backup-estimate")).toBeInTheDocument());
    expect(screen.getByText("/data/big.iso")).toBeInTheDocument();
    expect(screen.getByTestId("backup-estimate").textContent).toContain("from recent backups");
  });

  it("explains an assumed speed", () => {
    render(<EstimatePanel estimate={{ ...EST, speed_source: "assumed" }} onClose={() => {}} />);
    expect(screen.getByTestId("backup-estimate").textContent).toContain("assumed: no backup history yet");
  });
});

describe("Back up all now", () => {
  it("queues every job and reports skipped ones", async () => {
    useBackupStore.setState({ jobs: [{ ...newBackupJob(), id: "a", name: "A" }, { ...newBackupJob(), id: "b", name: "B" }] });
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_run_all") {
        expect(args).toEqual({ filter: { job_ids: [], scheduled_only: false, dry_run: false } });
        return { started: [{ job_id: "a", job_name: "A", task_id: "t" }], skipped: [{ job_id: "b", job_name: "B", reason: "read-only (restore and browse only)" }] };
      }
      if (cmd === "backup_status") return { running: true, paused: false, paused_until: null, tasks: [], next_runs: [] };
      throw new Error(`unexpected ${cmd}`);
    });
    render(<RunAllButton />);
    fireEvent.click(screen.getByTestId("backup-run-all"));
    await waitFor(() => expect(screen.getByText(/1 backup\(s\) queued; skipped B \(read-only/)).toBeInTheDocument());
    expect(describeRunAll({ started: [], skipped: [] })).toBe("0 backup(s) queued");
  });
});
