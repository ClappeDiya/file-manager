/**
 * K123: schedule time zones — the zone helpers, the editor's time zone
 * picker (Advanced) and the next-run preview in the chosen zone plus local.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ScheduleEditor } from "@/components/backup/backup-schedule-editor";
import { describeSchedule, newBackupJob, type BackupSchedule } from "@/stores/backup-store";
import { isLocalZone, offsetMinutesOf, offsetName, ZONE_CHOICES, zoneValue, zoneWallClock } from "@/lib/schedule-zone";

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue([]);
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("zone helpers", () => {
  it("names offsets and reads them back from RFC 3339 times", () => {
    expect(offsetName(120)).toBe("+02:00");
    expect(offsetName(-330)).toBe("-05:30");
    expect(offsetMinutesOf("2026-03-17T02:30:00+02:00")).toBe(120);
    expect(offsetMinutesOf("2026-03-17T02:30:00-05:30")).toBe(-330);
    expect(offsetMinutesOf("2026-03-17T02:30:00Z")).toBe(0);
    expect(offsetMinutesOf("garbage")).toBeNull();
  });

  it("treats empty and 'local' as this computer's zone", () => {
    expect(isLocalZone(null)).toBe(true);
    expect(isLocalZone(" Local ")).toBe(true);
    expect(isLocalZone("UTC")).toBe(false);
    expect(zoneValue(null)).toBe("local");
    expect(zoneValue("+02:00")).toBe("+02:00");
    expect(ZONE_CHOICES[0].value).toBe("local");
    expect(ZONE_CHOICES.map((z) => z.value)).toEqual(expect.arrayContaining(["UTC", "-12:00", "+05:45", "+14:00"]));
  });

  it("shows the wall-clock time of the zone", () => {
    expect(zoneWallClock("2026-03-17T02:30:00+02:00", "en-US")).toMatch(/Tue, Mar 17, 2026.*02:30.*\(UTC\+02:00\)$/);
    expect(zoneWallClock("2026-03-17T02:30:00+00:00", "en-US")).toMatch(/02:30.*\(UTC\)$/);
  });

  it("describes a schedule with its zone", () => {
    const s: BackupSchedule = { ...newBackupJob().schedule, enabled: true, timezone: "UTC" };
    expect(describeSchedule(s)).toBe("daily from 02:00 (UTC)");
    expect(describeSchedule({ ...s, timezone: "local" })).toBe("daily from 02:00");
  });
});

describe("schedule editor time zone", () => {
  it("is Advanced only", () => {
    useUIStore.setState({ appMode: "simple", backupAdvanced: false });
    render(<ScheduleEditor value={newBackupJob().schedule} onChange={() => {}} />);
    expect(screen.queryByTestId("schedule-timezone")).toBeNull();
  });

  it("picks a zone and previews runs there and on this computer", async () => {
    useUIStore.setState({ backupAdvanced: true });
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "backup_schedule_preview" ? [{ at: "2026-03-17T00:30:00+00:00", zoned: "2026-03-17T02:30:00+02:00", skipped_by: null }] : [],
    );
    let value: BackupSchedule = { ...newBackupJob().schedule, enabled: true, start_time: "02:30" };
    const onChange = vi.fn((s: BackupSchedule) => (value = s));
    const { rerender } = render(<ScheduleEditor value={value} onChange={onChange} />);
    const picker = screen.getByTestId("schedule-timezone") as HTMLSelectElement;
    expect(picker.value).toBe("local");
    fireEvent.change(picker, { target: { value: "+02:00" } });
    expect(value.timezone).toBe("+02:00");
    rerender(<ScheduleEditor value={value} onChange={onChange} />);
    await waitFor(() => expect(screen.getByTestId("schedule-preview").textContent).toContain("(UTC+02:00)"));
    expect(screen.getByTestId("schedule-preview").textContent).toContain("on this computer");
    const call = mockInvoke.mock.calls.filter((c) => c[0] === "backup_schedule_preview").at(-1)!;
    expect((call[1] as { schedule: BackupSchedule }).schedule.timezone).toBe("+02:00");
    // Back to local: stored as null, preview in local time only.
    fireEvent.change(screen.getByTestId("schedule-timezone"), { target: { value: "local" } });
    expect(value.timezone).toBeNull();
  });

  it("keeps a stored zone that is not in the list", () => {
    useUIStore.setState({ backupAdvanced: true });
    render(<ScheduleEditor value={{ ...newBackupJob().schedule, timezone: "UTC+1" }} onChange={() => {}} />);
    expect((screen.getByTestId("schedule-timezone") as HTMLSelectElement).value).toBe("UTC+1");
  });
});
