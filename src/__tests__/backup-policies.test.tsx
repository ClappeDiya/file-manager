/**
 * Policies, schedules and maintenance (WP6: K17/K18/K33/K116/K119/K34):
 * cron schedules with the next-run preview, hook modes, policy templates,
 * the effective-settings view and verification options.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ScheduleEditor } from "@/components/backup/backup-schedule-editor";
import { cronLines, isCronSchedule } from "@/lib/backup-helpers";
import { hookBeforeMode, newBackupJob, newVerifyOptions, type BackupSchedule, type VerifyOptions } from "@/stores/backup-store";
import { changedPaths, describeSource, patchFromJob, patchPaths, setPath, settingPaths, usePolicyStore, type EffectiveSettings, type PolicyTemplate } from "@/stores/backup-policy";
import { CustomVerify, EffectiveSettingsPanel, MaintenanceScheduleEditor, TemplatePicker, TemplatesManager, VerifyOptionsEditor } from "@/components/backup/backup-policies";

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: true });
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("cron schedules (K33)", () => {
  it("parses cron lines and detects cron mode", () => {
    expect(cronLines("30 2 * * Mon-Fri\n\n# comment\n 0 18 * * Sat ")).toEqual(["30 2 * * Mon-Fri", "0 18 * * Sat"]);
    const s = newBackupJob().schedule;
    expect(s.cron).toEqual([]);
    expect(isCronSchedule(s)).toBe(false);
    expect(isCronSchedule({ ...s, cron: [" "] })).toBe(false);
    expect(isCronSchedule({ ...s, cron: ["0 2 * * *"] })).toBe(true);
  });

  it("switches to Custom (cron) and previews the next runs", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "backup_schedule_preview") return [{ at: "2026-03-16T02:30:00Z", skipped_by: null }, { at: "2026-03-17T02:30:00Z", skipped_by: "Holidays" }];
      return [];
    });
    let value: BackupSchedule = { ...newBackupJob().schedule, enabled: true };
    const onChange = vi.fn((s: BackupSchedule) => {
      value = s;
    });
    const { rerender } = render(<ScheduleEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("schedule-type"), { target: { value: "cron" } });
    expect(value.cron).toEqual(["0 2 * * *"]);
    rerender(<ScheduleEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("schedule-cron"), { target: { value: "30 2 * * Mon-Fri\n0 18 * * Sat" } });
    expect(value.cron).toEqual(["30 2 * * Mon-Fri", "0 18 * * Sat"]);
    rerender(<ScheduleEditor value={value} onChange={onChange} />);
    await waitFor(() => expect(screen.getByTestId("schedule-preview").textContent).toContain("skipped: Holidays"));
    const call = mockInvoke.mock.calls.find((c) => c[0] === "backup_schedule_preview");
    expect(call?.[1]).toMatchObject({ schedule: { cron: ["30 2 * * Mon-Fri", "0 18 * * Sat"] }, count: 5 });
  });

  it("shows a friendly error for an invalid expression", async () => {
    mockInvoke.mockRejectedValue({ message: "Invalid cron expression '61 * * * *'", advice: "" });
    render(<ScheduleEditor value={{ ...newBackupJob().schedule, enabled: true, cron: ["61 * * * *"] }} onChange={() => {}} />);
    await waitFor(() => expect(screen.getByTestId("schedule-preview").textContent).toContain("Invalid cron expression"));
  });

  it("hides the cron type in Simple mode for interval schedules", () => {
    useUIStore.setState({ backupAdvanced: false });
    render(<ScheduleEditor value={newBackupJob().schedule} onChange={() => {}} />);
    expect(screen.queryByTestId("schedule-type")).toBeNull();
  });
});

describe("hook modes (K34)", () => {
  it("maps the older required flag", () => {
    const s = newBackupJob().scripts;
    expect(hookBeforeMode(s)).toBe("optional");
    expect(hookBeforeMode({ ...s, run_before_required: true })).toBe("required");
    expect(hookBeforeMode({ ...s, run_before_required: true, before_mode: "async" })).toBe("async");
  });
});

const TPL: PolicyTemplate = { id: "t1", name: "Laptops", description: "", settings: { retention: { type: "keep_count", count: 5 }, options: { compression_level: 3 } }, updated_at: "", managed: false, enforced: false };
const ORG: PolicyTemplate = { ...TPL, id: "org:std", name: "Org standard", managed: true, enforced: true };

describe("policy templates (K17)", () => {
  beforeEach(() => usePolicyStore.setState({ templates: [], error: null }));

  it("paths, diffs and patches match the Rust field paths", () => {
    const j = newBackupJob();
    const paths = settingPaths(j);
    expect(paths).toContain("options.compression_level");
    expect(paths).toContain("retention");
    expect(paths).toContain("maintenance");
    const k = setPath(setPath(j, "options.compression_level", 9), "retention", { type: "keep_days", days: 3 });
    expect(changedPaths(j, k).sort()).toEqual(["options.compression_level", "retention"]);
    expect(patchPaths(TPL.settings).sort()).toEqual(["options.compression_level", "retention"]);
    const p = patchFromJob(k, ["options.compression_level", "options.prefix", "retention"]);
    expect(p).toEqual({ options: { compression_level: 9 }, retention: { type: "keep_days", days: 3 } });
    expect(describeSource({ type: "template", id: "t", name: "X" })).toBe("Template: X");
  });

  it("manager lists, saves and validates templates", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: { template?: PolicyTemplate }) => {
      if (cmd === "backup_list_templates") return [TPL, ORG];
      if (cmd === "backup_save_template") return { ...args.template, id: "t2" };
      return null;
    });
    render(<TemplatesManager />);
    await waitFor(() => expect(screen.getByTestId("templates-list").textContent).toContain("Laptops"));
    expect(screen.getByTestId("templates-list").textContent).toContain("Org standard");
    fireEvent.click(screen.getByTestId("template-new"));
    fireEvent.change(screen.getByTestId("template-name"), { target: { value: "Servers" } });
    fireEvent.change(screen.getByTestId("template-settings"), { target: { value: "{not json" } });
    fireEvent.click(screen.getByTestId("template-save"));
    await waitFor(() => expect(document.body.textContent).toContain("not valid JSON"));
    fireEvent.change(screen.getByTestId("template-settings"), { target: { value: '{"retention":{"type":"keep_all"}}' } });
    fireEvent.click(screen.getByTestId("template-save"));
    await waitFor(() => expect(document.body.textContent).toContain("Saved Servers"));
    const save = mockInvoke.mock.calls.find((c) => c[0] === "backup_save_template");
    expect(save?.[1]).toMatchObject({ template: { name: "Servers", settings: { retention: { type: "keep_all" } } } });
  });

  it("picker adds, orders and removes templates; enforced ones cannot be picked", async () => {
    usePolicyStore.setState({ templates: [TPL, { ...TPL, id: "t3", name: "Servers" }, ORG] });
    mockInvoke.mockResolvedValue([TPL, { ...TPL, id: "t3", name: "Servers" }, ORG]);
    const onChange = vi.fn();
    const { rerender } = render(<TemplatePicker value={[]} onChange={onChange} />);
    const add = screen.getByTestId("template-add") as HTMLSelectElement;
    expect([...add.options].map((o) => o.value)).not.toContain("org:std");
    fireEvent.change(add, { target: { value: "t1" } });
    expect(onChange).toHaveBeenLastCalledWith(["t1"]);
    rerender(<TemplatePicker value={["t1", "t3"]} onChange={onChange} />);
    fireEvent.click(screen.getAllByText("↓")[0]);
    expect(onChange).toHaveBeenLastCalledWith(["t3", "t1"]);
    expect(document.body.textContent).toContain("Your organization enforces: Org standard");
  });
});

describe("effective settings (K18)", () => {
  it("shows sources and resets a job override to the inherited value", async () => {
    const eff: EffectiveSettings = {
      job_id: "j",
      template_ids: ["t1"],
      overrides: null,
      warnings: ["Policy template gone no longer exists; its settings are not applied"],
      settings: [
        { field_path: "retention", value: { type: "keep_count", count: 5 }, source: { type: "template", id: "t1", name: "Laptops" }, inherited: null },
        { field_path: "options.compression_level", value: 7, source: { type: "job" }, inherited: { value: 3, source: { type: "template", id: "t1", name: "Laptops" } } },
        { field_path: "options.hash_threads", value: 8, source: { type: "managed", id: "org:std", name: "Org standard" }, inherited: { value: 0, source: { type: "built_in" } } },
      ],
    };
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "backup_effective_settings" ? eff : null));
    const onReset = vi.fn();
    const job = { ...newBackupJob(), id: "j" };
    render(<EffectiveSettingsPanel job={job} onReset={onReset} />);
    await waitFor(() => expect(screen.getByTestId("effective-retention").textContent).toContain("Template: Laptops"));
    expect(screen.getByTestId("effective-options.hash_threads").textContent).toContain("Organization: Org standard");
    expect(document.body.textContent).toContain("no longer exists");
    expect(screen.queryByTestId("reset-options.hash_threads")).toBeNull();
    fireEvent.click(screen.getByTestId("reset-options.compression_level"));
    expect(onReset).toHaveBeenCalledWith("options.compression_level", 3, ["options.compression_level"]);
    const call = mockInvoke.mock.calls.find((c) => c[0] === "backup_effective_settings");
    expect(call?.[1]).toMatchObject({ id: null, job: { id: "j" } });
  });
});

describe("maintenance schedule (K116) and verification options (K119)", () => {
  it("turns tasks on with their own schedules", () => {
    let m = newBackupJob().maintenance;
    const onChange = vi.fn((x) => {
      m = x;
    });
    const { rerender } = render(<MaintenanceScheduleEditor value={m} onChange={onChange} autoCompactDays={7} />);
    expect(document.body.textContent).toContain("at most every 7 day(s)");
    fireEvent.click(screen.getByLabelText("Compact on its own schedule"));
    expect(m.compact?.cron).toEqual(["0 3 * * Sun"]);
    rerender(<MaintenanceScheduleEditor value={m} onChange={onChange} autoCompactDays={7} />);
    fireEvent.click(screen.getByLabelText("Verify on a schedule"));
    expect(m.verify?.options.mode).toEqual({ type: "sample_percent", percent: 10 });
    expect(m.verify?.schedule.enabled).toBe(true);
  });

  it("edits scope, sampling and limits", () => {
    let v: VerifyOptions = newVerifyOptions();
    const onChange = vi.fn((x: VerifyOptions) => {
      v = x;
    });
    const { rerender } = render(<VerifyOptionsEditor value={v} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("verify-mode"), { target: { value: "sample_percent" } });
    expect(v.mode).toEqual({ type: "sample_percent", percent: 10 });
    rerender(<VerifyOptionsEditor value={v} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("verify-percent"), { target: { value: "25" } });
    rerender(<VerifyOptionsEditor value={v} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("verify-files-percent"), { target: { value: "5" } });
    rerender(<VerifyOptionsEditor value={v} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("verify-paths"), { target: { value: "*/Documents/*\n\n/home/me/Photos" } });
    rerender(<VerifyOptionsEditor value={v} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("verify-versions"), { target: { value: "latest\n2" } });
    expect(v).toMatchObject({ mode: { type: "sample_percent", percent: 25 }, files_percent: 5, scope: { paths: ["*/Documents/*", "/home/me/Photos"], versions: ["latest", "2"] } });
  });

  it("custom check runs a verify operation with the options", () => {
    const onRun = vi.fn();
    render(<CustomVerify busy={false} onRun={onRun} />);
    fireEvent.click(screen.getByTestId("verify-custom-run"));
    expect(onRun).toHaveBeenCalledWith({ type: "verify", full: false, options: expect.objectContaining({ mode: { type: "sample_percent", percent: 10 }, files_percent: 5 }) });
  });
});
