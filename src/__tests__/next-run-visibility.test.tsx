/**
 * "Next run" for scheduled sync pairs and automation rules: the formatting
 * helper, and the lines in the automation and sync panels (from the single
 * `upcoming_runs` call).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { act } from "react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { AutomationPanel } from "@/components/automation-panel";
import { SyncPanel } from "@/components/sync-panel";
import { formatNextRun, nextRunsById, type UpcomingRun } from "@/lib/upcoming-runs";
import { useAutomationStore, type AutomationRule } from "@/stores/automation-store";
import { useUIStore } from "@/stores/ui-store";

const inHours = (h: number) => new Date(Date.now() + h * 3_600_000).toISOString();
const UPCOMING: UpcomingRun[] = [
  { kind: "automation", id: "rule-1", name: "Nightly", next_run_at: inHours(5) },
  { kind: "sync", id: "pair-1", name: "Photos", next_run_at: inHours(2) },
];

const rule = (id: string, name: string, trigger: AutomationRule["trigger"], enabled = true) =>
  ({ id, name, enabled, trigger, conditions: [], action: { type: "notify", title: "t", body: "b" }, steps: [], created_at: "", updated_at: "", last_triggered: null, trigger_count: 0, error_count: 0 }) as unknown as AutomationRule;

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useUIStore.setState({ appMode: "simple" });
});

describe("formatNextRun", () => {
  const now = new Date("2026-10-24T10:00:00Z");
  it("says how soon for the next hour, and the day and time after that", () => {
    expect(formatNextRun("2026-10-24T10:05:00Z", now)).toBe("in 5 min");
    expect(formatNextRun("2026-10-24T09:59:00Z", now)).toBe("due now");
    const later = formatNextRun("2026-10-25T02:00:00Z", now);
    expect(later).toMatch(/\d/);
    expect(later).not.toMatch(/^in /);
    expect(formatNextRun("not a date", now)).toBe("");
  });

  it("splits the backend list by kind", () => {
    expect(Object.keys(nextRunsById(UPCOMING, "sync"))).toEqual(["pair-1"]);
    expect(Object.keys(nextRunsById(UPCOMING, "automation"))).toEqual(["rule-1"]);
  });
});

describe("automation panel", () => {
  it("shows the next run of an enabled scheduled rule, not of others", async () => {
    const rules = [
      rule("rule-1", "Nightly", { type: "schedule", cron_expr: "0 0 2 * * * *", cron_human: "Every day at 02:00" }),
      rule("rule-2", "Watcher", { type: "file_appears", watch_path: "/in", recursive: false }),
      rule("rule-3", "Paused", { type: "schedule", cron_expr: "0 0 2 * * * *", cron_human: "Every day at 02:00" }, false),
    ];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_automation_rules") return rules;
      if (cmd === "upcoming_runs") return [...UPCOMING, { kind: "automation", id: "rule-3", name: "Paused", next_run_at: inHours(1) }];
      return [];
    });
    useAutomationStore.setState({ panelOpen: true, rules: [], nextRuns: {} });
    render(<AutomationPanel />);
    const line = await screen.findByTestId("next-run-rule-1");
    expect(line.textContent).toContain("Next run:");
    expect(screen.queryByTestId("next-run-rule-2")).toBeNull();
    expect(screen.queryByTestId("next-run-rule-3")).toBeNull(); // disabled
    expect(useAutomationStore.getState().nextRuns).toHaveProperty("rule-1");
  });
});

describe("sync panel", () => {
  it("shows the next run of a scheduled pair", async () => {
    const pair = { id: "pair-1", name: "Photos", source_path: "/a", dest_path: "/b", mode: "one_way", enabled: true, last_run: null, trigger: { scheduled: { cron_expr: "0 0 */6 * * * *" } } };
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_sync_pairs") return [pair, { ...pair, id: "pair-2", name: "Manual", trigger: "manual" }];
      if (cmd === "get_sync_health") return "green";
      if (cmd === "upcoming_runs") return UPCOMING;
      return [];
    });
    render(<SyncPanel />);
    expect((await screen.findByTestId("sync-next-run-pair-1")).textContent).toContain("Next run:");
    expect(screen.queryByTestId("sync-next-run-pair-2")).toBeNull();
  });

  it("saves a pair's notification settings through update_sync_pair", async () => {
    const pair = { id: "pair-1", name: "Photos", source_path: "/a", dest_path: "/b", mode: "one_way", enabled: true, last_run: null, trigger: "manual", run_options: { settle_secs: 0, completion_marker: null, version_template: null } };
    const updates: unknown[] = [];
    mockInvoke.mockImplementation(async (cmd: string, args?: { pairJson?: string }) => {
      if (cmd === "list_sync_pairs") return [pair];
      if (cmd === "get_sync_health") return "green";
      if (cmd === "update_sync_pair") {
        updates.push(JSON.parse(args!.pairJson!));
        return pair;
      }
      if (cmd === "backup_list_notification_profiles") return [{ id: "prof-1", name: "Ops Slack", description: "", notification: { type: "slack", credential_id: "c", send_on: [], template: "", cert: {} }, min_severity: null, managed: false, created_at: "", updated_at: "" }];
      return [];
    });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Notifications for Photos"));
    await screen.findByText(/Ops Slack/);
    await act(async () => {
      fireEvent.change(screen.getByTestId("run-notify-profile"), { target: { value: "prof-1" } });
    });
    await waitFor(() => expect(updates).toHaveLength(1), { timeout: 3000 });
    expect((updates[0] as { run_options: unknown }).run_options).toEqual({ settle_secs: 0, completion_marker: null, version_template: null, notify_profile_id: "prof-1" });
  });
  const notifyHarness = (initialNotify: Record<string, unknown> = {}) => {
    const pair = { id: "pair-1", name: "Photos", source_path: "/a", dest_path: "/b", mode: "one_way", enabled: true, last_run: null, trigger: "manual", run_options: { settle_secs: 0, completion_marker: null, version_template: null, ...initialNotify } };
    const updates: { run_options: Record<string, unknown> }[] = [];
    mockInvoke.mockImplementation(async (cmd: string, args?: { pairJson?: string }) => {
      if (cmd === "list_sync_pairs") return [pair];
      if (cmd === "get_sync_health") return "green";
      if (cmd === "update_sync_pair") {
        const sent = JSON.parse(args!.pairJson!);
        // What the backend's RunNotify::validate enforces.
        const o = sent.run_options;
        if (!o.notify_profile_id && ((o.notify_on ?? []).length > 0 || o.min_bytes != null)) throw new Error("Choose a notification profile to send to");
        updates.push(sent);
        return pair;
      }
      if (cmd === "backup_list_notification_profiles") return [{ id: "prof-1", name: "Ops Slack", description: "", notification: { type: "slack", credential_id: "c", send_on: [], template: "", cert: {} }, min_severity: null, managed: false, created_at: "", updated_at: "" }];
      return [];
    });
    return updates;
  };

  it("turning notifications off after choosing results saves a valid pair", async () => {
    const updates = notifyHarness({ notify_profile_id: "prof-1", notify_on: ["error"] });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Notifications for Photos"));
    await screen.findByText(/Ops Slack/);
    await act(async () => {
      fireEvent.change(screen.getByTestId("run-notify-profile"), { target: { value: "" } });
    });
    await waitFor(() => expect(updates).toHaveLength(1), { timeout: 3000 });
    expect(screen.queryByText(/Choose a notification profile/)).toBeNull();
    const o = updates[0].run_options;
    expect(o.notify_profile_id ?? null).toBeNull();
    expect(o.notify_on).toEqual([]);
    expect(o.min_bytes ?? null).toBeNull();
  });

  it("typing a threshold saves once, after typing pauses", async () => {
    useUIStore.setState({ appMode: "advanced" });
    const updates = notifyHarness({ notify_profile_id: "prof-1" });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Notifications for Photos"));
    await screen.findByText(/Ops Slack/);
    const input = screen.getByLabelText(/Don’t report successful runs below/);
    for (const v of ["1", "10", "100"]) fireEvent.change(input, { target: { value: v } });
    expect(updates).toHaveLength(0);
    await waitFor(() => expect(updates).toHaveLength(1), { timeout: 3000 });
    expect(updates[0].run_options.min_bytes).toBe(100 * 1024 * 1024);
  });
});
