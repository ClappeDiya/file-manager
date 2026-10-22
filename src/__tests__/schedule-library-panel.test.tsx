/** Schedule library: schedules, calendars, preview and ICS import call the Rust commands with their shapes. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));
vi.mock("@/lib/folder-picker", () => ({ pickFiles: vi.fn(async () => ["/tmp/holidays.ics"]), pickFolder: vi.fn() }));

import { ScheduleLibraryPanel } from "@/components/schedule-library-panel";
import { refId, scheduleRef, useScheduleStore } from "@/stores/schedule-store";

const cal = { id: "c1", name: "UK bank holidays", dates: ["2026-12-28"], annual: ["12-25"], skip_weekends: true };
const sched = { id: "s1", name: "Weekday mornings", description: "", cron_expr: "0 0 9 * * Mon-Fri", calendar_ids: ["c1"], updated_at: "" };

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useScheduleStore.setState({ schedules: [], calendars: [], loaded: false });
  mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "schedules_list":
        return [sched];
      case "calendars_list":
        return [cal];
      case "schedule_preview":
        return [
          { at: "2026-12-24T09:00:00Z", skipped_by: null },
          { at: "2026-12-25T09:00:00Z", skipped_by: "UK bank holidays" },
        ];
      case "schedule_save":
        return { ...(args!.schedule as object), id: "s2" };
      case "calendar_save":
        return args!.calendar;
      case "calendar_import_ics":
        return [{ ...cal, dates: ["2026-12-28", "2027-01-01"] }, 1];
      case "schedule_delete":
        throw { message: "The schedule is used by sync pair 'Docs'; change those first", advice: "" };
      default:
        return null;
    }
  });
});

describe("schedule refs", () => {
  it("round-trip", () => {
    expect(refId(scheduleRef("abc"))).toBe("abc");
    expect(refId("0 0 * * * *")).toBeNull();
  });
});

describe("ScheduleLibraryPanel", () => {
  it("lists schedules with their calendars and previews skipped runs", async () => {
    render(<ScheduleLibraryPanel />);
    expect(await screen.findByText("Weekday mornings")).toBeInTheDocument();
    expect(screen.getByText("skips UK bank holidays")).toBeInTheDocument();
    expect(screen.getByText(/1 date\(s\), 1 yearly, weekends/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("New schedule"));
    fireEvent.change(screen.getByLabelText("Schedule name"), { target: { value: "Nightly" } });
    fireEvent.change(screen.getByLabelText("Cron expression"), { target: { value: "0 0 2 * * *" } });
    fireEvent.click(screen.getByLabelText("UK bank holidays"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("schedule_preview", { cronExpr: "0 0 2 * * *", calendarIds: ["c1"], count: 8 }));
    expect(await screen.findByText(/skipped \(UK bank holidays\)/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("Save schedule"));
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "schedule_save")).toBe(true));
    const saved = mockInvoke.mock.calls.find(([c]) => c === "schedule_save")![1].schedule;
    expect(saved).toMatchObject({ id: "", name: "Nightly", cron_expr: "0 0 2 * * *", calendar_ids: ["c1"] });
  });

  it("saves calendars, imports .ics and reports refusals", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<ScheduleLibraryPanel />);
    await screen.findByText("UK bank holidays");
    fireEvent.click(screen.getByText("New calendar"));
    fireEvent.change(screen.getByLabelText("Calendar name"), { target: { value: "Office" } });
    fireEvent.change(screen.getByLabelText("Dates"), { target: { value: "2026-04-03\n\n2026-04-06" } });
    fireEvent.change(screen.getByLabelText("Yearly dates"), { target: { value: "12-24, 12-31" } });
    fireEvent.click(screen.getByText("Save calendar"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("calendar_save", { calendar: { id: "", name: "Office", dates: ["2026-04-03", "2026-04-06"], annual: ["12-24", "12-31"], skip_weekends: false } }));

    fireEvent.click(screen.getAllByText("Edit")[1]);
    fireEvent.click(screen.getByText("Import .ics…"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("calendar_import_ics", { id: "c1", name: "UK bank holidays", path: "/tmp/holidays.ics" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Imported 1 date(s).");

    fireEvent.click(screen.getByLabelText("Delete Weekday mornings"));
    expect(await screen.findByRole("alert")).toHaveTextContent("used by sync pair 'Docs'");
  });
});
