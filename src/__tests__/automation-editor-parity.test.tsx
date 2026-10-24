/**
 * Rule editor additions (Advanced): searchable templates, per-step risk
 * badges with a confirmation before saving High-risk steps, result names,
 * the new steps (verify, list, tidy, purge), download straight to a remote,
 * and result notifications.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { AutomationRuleEditor } from "@/components/automation-rule-editor";
import { RiskBadge, RiskConfirm, highRiskSteps, fetchRisks } from "@/components/step-risk";
import { useAutomationStore, type AutomationRule, type RuleAction, type StepRisk } from "@/stores/automation-store";
import { useUIStore } from "@/stores/ui-store";

let lastSaved: AutomationRule | null = null;
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

/** What Rust answers for the actions the tests use. */
const RISKS: Record<string, StepRisk> = {
  delete_file: { level: "high", reason: "Deletes files permanently" },
  purge: { level: "high", reason: "Removes a folder and everything in it" },
  copy_file: { level: "low", reason: "Copies files; nothing is removed" },
  move_file: { level: "medium", reason: "Moves files out of their folder" },
};

beforeEach(() => {
  mockInvoke.mockReset();
  lastSaved = null;
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useAutomationStore.setState({ editorOpen: false, editorRule: null, rules: [] });
  useUIStore.setState({ appMode: "advanced" });
  mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "create_automation_rule":
      case "update_automation_rule":
        lastSaved = args?.rule as AutomationRule;
        return lastSaved;
      case "automation_action_risks":
        return (args?.actions as RuleAction[]).map((a) => RISKS[a.type] ?? { level: "low", reason: "" });
      case "list_sync_pairs":
        return [{ id: "pair-1", name: "Photos" }];
      case "list_connections":
        return [{ id: "conn-1", name: "Office SFTP", protocol: "sftp" }];
      case "backup_list_notification_profiles":
        return [{ id: "prof-1", name: "Ops Slack", description: "", notification: { type: "slack", credential_id: "c", send_on: [], template: "", cert: {} }, min_severity: null, managed: false, created_at: "", updated_at: "" }];
      default:
        return [];
    }
  });
});

const open = (rule?: Partial<AutomationRule>) => act(() => useAutomationStore.getState().openEditor(rule));
const manual = () => fireEvent.change(screen.getByLabelText("Trigger"), { target: { value: "manual" } });
const watch = () => fireEvent.change(screen.getByLabelText("Folder to watch", { selector: "input" }), { target: { value: "/in" } });

describe("step risk helpers", () => {
  it("draws a badge with the reason and picks out the High steps", () => {
    render(<RiskBadge risk={RISKS.delete_file} />);
    const badge = screen.getByTestId("risk-badge");
    expect(badge.textContent).toBe("High risk");
    expect(badge.getAttribute("title")).toBe("Deletes files permanently");
    expect(highRiskSteps([{ type: "delete_file", to_trash: false }, { type: "create_folder", path: "/x" }, { type: "purge", path: "/y", max_files: null }], [RISKS.delete_file, RISKS.copy_file, RISKS.purge]).map((s) => s.step)).toEqual([1, 3]);
  });

  it("asks Rust for risks only when there are actions, and ignores a mismatched answer", async () => {
    expect(await fetchRisks([])).toEqual([]);
    expect(mockInvoke).not.toHaveBeenCalled();
    mockInvoke.mockResolvedValueOnce([RISKS.copy_file]);
    expect(await fetchRisks([{ type: "delete_file", to_trash: true }, { type: "delete_file", to_trash: true }])).toEqual([]);
  });

  it("the confirmation names every step and answers both ways", () => {
    const yes = vi.fn();
    const no = vi.fn();
    render(<RiskConfirm steps={[{ step: 2, risk: RISKS.purge }]} onConfirm={yes} onCancel={no} />);
    expect(screen.getByRole("alertdialog").textContent).toContain("Action 2: Removes a folder and everything in it");
    fireEvent.click(screen.getByText("Go back"));
    fireEvent.click(screen.getByText("Save with high-risk steps"));
    expect([no.mock.calls.length, yes.mock.calls.length]).toEqual([1, 1]);
  });
});

describe("rule editor in Advanced mode", () => {
  it("shows a risk badge on each step", async () => {
    render(<AutomationRuleEditor />);
    open();
    watch();
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "delete_file" } });
    await waitFor(() => expect(screen.getByTestId("step-0").querySelector('[data-risk="high"]')).not.toBeNull());
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "copy_file" } });
    await waitFor(() => expect(screen.getByTestId("step-0").querySelector('[data-risk="low"]')).not.toBeNull());
  });

  it("holds a rule with a High-risk step until it is confirmed in the page, not a browser dialog", async () => {
    const browserConfirm = vi.spyOn(window, "confirm");
    render(<AutomationRuleEditor />);
    open();
    watch();
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "delete_file" } });
    fireEvent.click(screen.getByLabelText(/Move to the folder's .ufop_trash/)); // permanent delete
    fireEvent.click(screen.getByText("Create rule"));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Deletes files permanently");
    expect(calls("create_automation_rule")).toHaveLength(0);
    // Go back: nothing saved.
    fireEvent.click(screen.getByText("Go back"));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls("create_automation_rule")).toHaveLength(0);
    // Confirm: saved.
    fireEvent.click(screen.getByText("Create rule"));
    fireEvent.click(await screen.findByText("Save with high-risk steps"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(lastSaved!.action).toEqual({ type: "delete_file", to_trash: false });
    expect(browserConfirm).not.toHaveBeenCalled();
  });

  it("does not ask for a Low or Medium risk rule", async () => {
    render(<AutomationRuleEditor />);
    open();
    watch();
    fireEvent.change(screen.getByLabelText("Destination folder", { selector: "input" }), { target: { value: "/out" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("asks nothing and shows no badges in Standard mode", async () => {
    useUIStore.setState({ appMode: "simple" });
    render(<AutomationRuleEditor />);
    open();
    watch();
    fireEvent.change(screen.getByLabelText("Destination folder", { selector: "input" }), { target: { value: "/out" } });
    expect(screen.queryByTestId("risk-badge")).toBeNull();
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(calls("automation_action_risks")).toHaveLength(0);
  });

  it("starts from a searched template (Advanced) and keeps the plain list in Standard", async () => {
    render(<AutomationRuleEditor />);
    open({ trigger: { type: "file_appears", watch_path: "/in", recursive: false } } as Partial<AutomationRule>);
    const box = await screen.findByRole("combobox", { name: "Start from a template" });
    fireEvent.change(box, { target: { value: "antivirus" } });
    fireEvent.click(screen.getByRole("option", { name: /Scan arrivals for malware/ }));
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Scan new files for malware");
    expect((screen.getByLabelText("Action 1") as HTMLSelectElement).value).toBe("scan_for_malware");
  });

  it("saves result names and refuses an unknown reference's name clash before saving", async () => {
    render(<AutomationRuleEditor />);
    open();
    watch();
    fireEvent.change(screen.getByLabelText("Destination folder", { selector: "input" }), { target: { value: "/out" } });
    fireEvent.click(screen.getByText("Add action"));
    fireEvent.change(screen.getByLabelText("Save this step's result as (optional)", { selector: "#step-0-out" }), { target: { value: "moved" } });
    expect(screen.getByTestId("step-1").textContent).toContain("{steps.moved.…}");
    fireEvent.change(screen.getByLabelText("Save this step's result as (optional)", { selector: "#step-1-out" }), { target: { value: "moved" } });
    fireEvent.click(screen.getByText("Create rule"));
    expect((await screen.findByRole("alert")).textContent).toContain('The result name "moved" is used by more than one step.');
    fireEvent.change(screen.getByLabelText("Save this step's result as (optional)", { selector: "#step-1-out" }), { target: { value: "note" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(lastSaved!.output_as).toBe("moved");
    expect(lastSaved!.steps![0]).toMatchObject({ output_as: "note" });
  });

  it("edits the new steps and saves them in the engine's shape", async () => {
    render(<AutomationRuleEditor />);
    open();
    manual();
    // verify_pair
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "verify_pair" } });
    fireEvent.change(await screen.findByLabelText("Sync pair"), { target: { value: "pair-1" } });
    fireEvent.click(screen.getByLabelText(/Compare file contents/));
    // list_files
    fireEvent.click(screen.getByText("Add action"));
    fireEvent.change(screen.getByLabelText("Action 2"), { target: { value: "list_files" } });
    fireEvent.change(screen.getByLabelText(/Folder to list/, { selector: "input" }), { target: { value: "/data" } });
    fireEvent.change(screen.getByLabelText("Save the listing as"), { target: { value: "/r/list.json" } });
    fireEvent.change(screen.getByLabelText("Format"), { target: { value: "json" } });
    // remove_empty_dirs
    fireEvent.click(screen.getByText("Add action"));
    fireEvent.change(screen.getByLabelText("Action 3"), { target: { value: "remove_empty_dirs" } });
    fireEvent.change(screen.getByLabelText(/^Folder \(empty/, { selector: "input" }), { target: { value: "/data" } });
    // purge
    fireEvent.click(screen.getByText("Add action"));
    fireEvent.change(screen.getByLabelText("Action 4"), { target: { value: "purge" } });
    fireEvent.change(screen.getByLabelText("Folder to purge", { selector: "input" }), { target: { value: "/data/tmp" } });
    fireEvent.change(screen.getByLabelText(/Stop if it holds more than/), { target: { value: "50" } });
    fireEvent.click(screen.getByText("Create rule"));
    fireEvent.click(await screen.findByText("Save with high-risk steps")); // purge is High
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(lastSaved!.action).toEqual({ type: "verify_pair", sync_pair_id: "pair-1", full: false });
    expect(lastSaved!.steps!.map((s) => s.action)).toEqual([
      { type: "list_files", path: "/data", output: "/r/list.json", format: "json", recursive: true, include_dirs: false, overwrite: true },
      { type: "remove_empty_dirs", path: "/data", remove_root: false },
      { type: "purge", path: "/data/tmp", max_files: 50 },
    ]);
  });

  it("validates the new steps", async () => {
    render(<AutomationRuleEditor />);
    open();
    manual();
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "verify_pair" } });
    fireEvent.click(screen.getByText("Create rule"));
    expect((await screen.findByRole("alert")).textContent).toContain("Action 1: choose a sync pair.");
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "purge" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Action 1: choose the folder to purge."));
    expect(calls("create_automation_rule")).toHaveLength(0);
  });

  it("sends a download straight to a remote connection", async () => {
    render(<AutomationRuleEditor />);
    open();
    manual();
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "download_url" } });
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "https://example.com/a.csv" } });
    await screen.findByText(/Office SFTP/);
    fireEvent.change(screen.getByLabelText("Send it to"), { target: { value: "conn-1" } });
    expect(screen.queryByLabelText("Save into folder", { selector: "input" })).toBeNull();
    fireEvent.click(screen.getByText("Create rule"));
    expect((await screen.findByRole("alert")).textContent).toContain("enter the remote folder to upload into");
    fireEvent.change(screen.getByLabelText("Remote folder"), { target: { value: "/incoming/{yyyy}" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(lastSaved!.action).toMatchObject({ type: "download_url", destination_connection_id: "conn-1", destination_protocol: "sftp", remote_path: "/incoming/{yyyy}" });
  });

  it("saves the result notification settings with the rule's options", async () => {
    render(<AutomationRuleEditor />);
    open();
    watch();
    fireEvent.change(screen.getByLabelText("Destination folder", { selector: "input" }), { target: { value: "/out" } });
    await screen.findByText(/Ops Slack/);
    fireEvent.change(screen.getByTestId("run-notify-profile"), { target: { value: "prof-1" } });
    fireEvent.click(screen.getByLabelText("Errors"));
    fireEvent.change(screen.getByLabelText(/Don’t report successful runs below/), { target: { value: "5" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(lastSaved!.options).toMatchObject({ notify_profile_id: "prof-1", notify_on: ["error"], min_bytes: 5 * 1048576 });
  });
});
