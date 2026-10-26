/** WS-I row 73: the trigger of automation, sync and backup runs. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { AutomationPanel } from "@/components/automation-panel";
import { automationTrigger, triggerLabel } from "@/lib/run-trigger";
import { useAutomationStore, type AutomationLog } from "@/stores/automation-store";

const log = (id: string, trigger_event: string): AutomationLog =>
  ({ id, rule_id: "r", rule_name: `Rule ${id}`, triggered_at: "2026-10-01T10:00:00Z", completed_at: null, trigger_event, status: "success", files_affected: [], error_message: null, attempts: 1, steps: [] }) as unknown as AutomationLog;

describe("trigger labels", () => {
  it("names the trigger of a sync or backup run, Manual when unknown", () => {
    expect([undefined, null, "manual", "scheduled", "watcher", "api", "automation", "???"].map(triggerLabel)).toEqual(["Manual", "Manual", "Manual", "Scheduled", "Watcher", "API", "Automation", "Manual"]);
  });

  it("maps the events that fire automation rules and keeps unknown ones as they are", () => {
    expect(automationTrigger("manual")).toEqual({ label: "Manual", detail: null });
    expect(automationTrigger("schedule")).toEqual({ label: "Scheduled", detail: null });
    expect(automationTrigger("file: /in/a.pdf")).toEqual({ label: "Watcher", detail: "/in/a.pdf" });
    expect(automationTrigger("file: C:\\in\\a.pdf")).toEqual({ label: "Watcher", detail: "C:\\in\\a.pdf" });
    expect(automationTrigger("webhook")).toEqual({ label: "API", detail: null });
    expect(automationTrigger("")).toEqual({ label: "Manual", detail: null });
    expect(automationTrigger("usb-inserted")).toEqual({ label: "usb-inserted", detail: null });
  });
});

const LOGS = [log("1", "manual"), log("2", "schedule"), log("3", "file: /in/a.pdf")];

describe("automation logs", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "list_automation_logs" ? LOGS : []));
    (window as unknown as Record<string, unknown>).__TAURI__ = {};
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  it("shows what fired each run, and the file for watcher runs", async () => {
    useAutomationStore.setState({ rules: [], logs: LOGS, loading: false, panelOpen: true });
    render(<AutomationPanel />);
    fireEvent.click(await screen.findByRole("button", { name: /Logs \(3\)/ }));
    const labels = screen.getAllByTestId("log-trigger").map((e) => e.textContent?.replace("· ", ""));
    expect(labels).toEqual(["Manual", "Scheduled", "Watcher"]);
    expect(within(screen.getByText("Rule 3").closest("div")!.parentElement!).getByText("/in/a.pdf")).toBeInTheDocument();
  });
});
