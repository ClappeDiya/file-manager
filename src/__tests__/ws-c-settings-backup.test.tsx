/** WS-C: backup duplicate + shared rule tools in the backup editor, Settings › sync defaults and bandwidth timetable. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupPanel } from "@/components/backup/backup-panel";
import { SyncDefaultsSection } from "@/components/sync-defaults-section";
import { BandwidthTimetableSection } from "@/components/bandwidth-timetable-section";
import { FilterPresetPicker, appendRules } from "@/components/filter-rules-editor";
import { rulesToText, textToRules } from "@/lib/backup-helpers";
import { DEFAULT_COPY_OPTIONS } from "@/lib/copy-options";
import { newBackupJob, type BackupJob, type FilterRule } from "@/stores/backup-store";
import { useUIStore } from "@/stores/ui-store";

const STATUS = { running: true, paused: false, paused_until: null, tasks: [], next_runs: [] };
const job: BackupJob = {
  ...newBackupJob(),
  id: "job-1",
  name: "Documents",
  sources: ["/home/me/Docs"],
  destination: { type: "local", path: "/mnt/backup", alternate_paths: [], marker_file: null },
};

function backend(extra: Record<string, (a: Record<string, unknown>) => unknown> = {}) {
  const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
    backup_list_jobs: () => [job],
    backup_status: () => STATUS,
    backup_capabilities: () => ({ filter_groups: [], connection_protocols: [], default_no_compress_extensions: [] }),
    list_connections: () => [],
    backup_get_desktop_settings: () => ({ run_in_background: false, start_at_login: false }),
    duplicate_backup_job: () => ({ ...job, id: "job-2", name: "Documents (copy)", schedule: { ...job.schedule, enabled: false } }),
    list_user_filter_presets: () => [],
    save_user_filter_preset: (a) => ({ id: "u9", name: a.name, rules: a.rules, created_at: "x" }),
    delete_user_filter_preset: () => true,
    get_sync_default_options: () => ({ ...DEFAULT_COPY_OPTIONS, retries: 3 }),
    set_sync_default_options: (a) => a.options,
    get_bandwidth_schedule: () => [],
    set_bandwidth_schedule: (a) => a.windows,
    ...extra,
  };
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    const h = handlers[cmd];
    return h ? h(args ?? {}) : cmd.startsWith("list") ? [] : null;
  });
}

const called = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useUIStore.setState({ appMode: "simple" });
});

describe("backup: duplicate and shared filter tools", () => {
  it("duplicates a job and opens the copy in the editor", async () => {
    backend();
    render(<BackupPanel />);
    fireEvent.click(await screen.findByTestId("backup-duplicate"));
    await waitFor(() => expect(called("duplicate_backup_job")).toEqual([{ id: "job-1" }]));
    expect(await screen.findByDisplayValue("Documents (copy)")).toBeInTheDocument();
  });

  it("keeps rclone rules in the text format", () => {
    const rules: FilterRule[] = [{ action: "exclude", kind: "rclone", pattern: "*.{jpg,png}" }];
    expect(rulesToText(rules)).toBe("-rclone:*.{jpg,png}");
    expect(textToRules(rulesToText(rules))).toEqual(rules);
    expect(appendRules(rules, [...rules, { action: "include", kind: "wildcard", pattern: "a" }])).toHaveLength(2);
  });

  it("saves the current rules as a preset, and deletes presets in the Advanced view", async () => {
    let presets: unknown[] = [];
    backend({
      list_user_filter_presets: () => presets,
      save_user_filter_preset: (a) => {
        presets = [{ id: "u9", name: a.name, rules: a.rules, created_at: "x" }];
        return presets[0];
      },
      delete_user_filter_preset: () => {
        presets = [];
        return true;
      },
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    useUIStore.setState({ appMode: "advanced" });
    const rules: FilterRule[] = [{ action: "exclude", kind: "wildcard", pattern: "*.tmp" }];
    const onApply = vi.fn();
    render(<FilterPresetPicker rules={rules} onApply={onApply} />);
    fireEvent.click(screen.getByRole("button", { name: "Save as preset…" }));
    fireEvent.change(screen.getByLabelText("Preset name"), { target: { value: "Mine" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(called("save_user_filter_preset")).toEqual([{ id: null, name: "Mine", rules }]));
    fireEvent.change(await screen.findByLabelText("Apply a saved filter preset"), { target: { value: "u9" } });
    await waitFor(() => expect(onApply).toHaveBeenCalledWith(rules));
    fireEvent.click(screen.getByLabelText("Delete preset Mine"));
    await waitFor(() => expect(called("delete_user_filter_preset")).toEqual([{ id: "u9" }]));
  });
});

describe("Settings: sync defaults and bandwidth timetable", () => {
  it("edits and saves the global sync defaults", async () => {
    backend();
    render(<SyncDefaultsSection />);
    expect(await screen.findByTestId("sync-defaults")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Reliability & speed"));
    fireEvent.click(screen.getByRole("button", { name: "Save sync defaults" }));
    await waitFor(() => expect(called("set_sync_default_options")).toHaveLength(1));
    expect((called("set_sync_default_options")[0].options as { retries: number }).retries).toBe(3);
    expect(await screen.findByText(/Saved\./)).toBeInTheDocument();
  });

  it("adds a window and saves the timetable", async () => {
    backend();
    render(<BandwidthTimetableSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Add window" }));
    fireEvent.change(screen.getByLabelText("Upload limit"), { target: { value: "256" } });
    fireEvent.click(screen.getByLabelText("Sat"));
    fireEvent.click(screen.getByRole("button", { name: "Save timetable" }));
    await waitFor(() => expect(called("set_bandwidth_schedule")).toHaveLength(1));
    expect(called("set_bandwidth_schedule")[0].windows).toEqual([
      { days: [0, 1, 2, 3, 4, 5], start: "08:00", end: "18:00", upload_bps: 256 * 1024, download_bps: 0 },
    ]);
    expect(await screen.findByText("Saved and applied.")).toBeInTheDocument();
  });
});
