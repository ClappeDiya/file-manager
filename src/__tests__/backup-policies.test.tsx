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
import { hookBeforeMode, newBackupJob, type BackupSchedule } from "@/stores/backup-store";

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
