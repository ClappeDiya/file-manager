/**
 * Named schedules and holiday calendars in the editors: sync pairs and
 * automation rules store `schedule:<id>`, backup schedules calendar ids;
 * the sync cron field previews real next runs (regression: it showed every
 * valid expression as invalid).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SyncPanel } from "@/components/sync-panel";
import { AutomationRuleEditor } from "@/components/automation-rule-editor";
import { HolidayCalendarPicker } from "@/components/schedule-pickers";
import { useScheduleStore } from "@/stores/schedule-store";
import { useAutomationStore, type AutomationRule } from "@/stores/automation-store";
import { useUIStore } from "@/stores/ui-store";

const sched = { id: "s1", name: "Weekday mornings", description: "", cron_expr: "0 0 9 * * Mon-Fri", calendar_ids: ["c1"], updated_at: "" };
const cal = { id: "c1", name: "UK bank holidays", dates: [], annual: ["12-25"], skip_weekends: false };

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
        if (args?.cronExpr === "bad") throw { message: "Invalid cron expression 'bad'", advice: "" };
        return [
          { at: "2026-12-24T09:00:00Z", skipped_by: null },
          { at: "2026-12-25T09:00:00Z", skipped_by: "UK bank holidays" },
          { at: "2026-12-28T09:00:00Z", skipped_by: null },
        ];
      case "create_automation_rule":
        return args?.rule;
      default:
        return [];
    }
  });
});

describe("sync pair schedule", () => {
  it("shows the next runs of a valid cron and errors for an invalid one", async () => {
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Create new sync pair"));
    fireEvent.change(screen.getByDisplayValue("Manual"), { target: { value: "scheduled" } });
    expect(await screen.findByText("Valid - Next 3 runs:", {}, { timeout: 2000 })).toBeInTheDocument();
    expect(screen.getByText(/\(skipped: UK bank holidays\)/)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("0 0 */6 * * * *"), { target: { value: "bad" } });
    expect(await screen.findByText(/Invalid cron expression 'bad'/, {}, { timeout: 2000 })).toBeInTheDocument();
  });

  it("can use a named schedule", async () => {
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Create new sync pair"));
    fireEvent.change(screen.getByDisplayValue("Manual"), { target: { value: "scheduled" } });
    const picker = await screen.findByLabelText("Named schedule");
    fireEvent.change(picker, { target: { value: "s1" } });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("schedule_preview", { cronExpr: "schedule:s1", count: 3 }));
    expect(screen.getByPlaceholderText("0 0 */6 * * * *").closest("label")!.className).toContain("hidden");
    fireEvent.change(picker, { target: { value: "" } });
    await waitFor(() => expect((screen.getByPlaceholderText("0 0 */6 * * * *") as HTMLInputElement).value).toBe("0 0 */6 * * * *"));
  });
});

describe("automation rule schedule", () => {
  it("stores a named schedule reference (advanced mode)", async () => {
    useUIStore.setState({ appMode: "advanced" });
    useAutomationStore.setState({ editorOpen: false, editorRule: null, rules: [] });
    render(<AutomationRuleEditor />);
    act(() => useAutomationStore.getState().openEditor());
    fireEvent.change(await screen.findByLabelText("Trigger"), { target: { value: "schedule" } });
    fireEvent.change(await screen.findByLabelText("Named schedule"), { target: { value: "s1" } });
    expect(screen.queryByLabelText("Cron expression")).toBeNull();
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "create_folder" } });
    fireEvent.change(screen.getByLabelText("Folder", { selector: "input" }), { target: { value: "/out" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "create_automation_rule")).toBe(true));
    const rule = mockInvoke.mock.calls.find(([c]) => c === "create_automation_rule")![1].rule as AutomationRule;
    expect(rule.trigger).toEqual({ type: "schedule", cron_expr: "schedule:s1", cron_human: null });
  });
});

describe("backup holiday calendars", () => {
  it("toggles calendar ids", async () => {
    const onChange = vi.fn();
    function Harness() {
      const [v, setV] = useState<string[]>([]);
      return <HolidayCalendarPicker value={v} onChange={(ids) => { onChange(ids); setV(ids); }} />;
    }
    render(<Harness />);
    fireEvent.click(await screen.findByLabelText("UK bank holidays"));
    expect(onChange).toHaveBeenLastCalledWith(["c1"]);
    fireEvent.click(screen.getByLabelText("UK bank holidays"));
    expect(onChange).toHaveBeenLastCalledWith([]);
  });
});
