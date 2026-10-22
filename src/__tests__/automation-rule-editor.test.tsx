/**
 * Automation rule editor: renders when opened from the store, saves the
 * rule the engine expects, validates before saving, previews templates via
 * Rust, runs a dry run, and gates command rules behind an approval.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { AutomationRuleEditor } from "@/components/automation-rule-editor";
import { useAutomationStore, type AutomationRule } from "@/stores/automation-store";
import { useUIStore } from "@/stores/ui-store";

let lastSaved: AutomationRule | null = null;
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useAutomationStore.setState({ editorOpen: false, editorRule: null, rules: [] });
  useUIStore.setState({ appMode: "simple" });
  mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "create_automation_rule":
      case "update_automation_rule":
        lastSaved = args?.rule as AutomationRule;
        return lastSaved;
      case "list_automation_rules":
        return [];
      case "naming_preview":
        return `/out/${args?.template === "/out/{ext}" ? "pdf" : "x"}`;
      case "test_automation_rule":
        return { id: "l", rule_id: "r", rule_name: "R", status: "success", trigger_event: "manual", files_affected: ["/in/a.pdf"], error_message: "dry run: would move", executed_at: "", duration_ms: 1, attempts: 1, steps: [] };
      case "approve_automation_command":
        return { ...lastSaved!, options: { ...lastSaved!.options!, command_approval: "ok" } };
      default:
        return [];
    }
  });
});

const open = (rule?: Partial<AutomationRule>) => act(() => useAutomationStore.getState().openEditor(rule));

describe("AutomationRuleEditor", () => {
  it("is hidden until opened and closes with Escape", async () => {
    render(<AutomationRuleEditor />);
    expect(screen.queryByRole("dialog")).toBeNull();
    open();
    expect(await screen.findByRole("dialog", { name: "New rule" })).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("validates before saving", async () => {
    render(<AutomationRuleEditor />);
    open();
    fireEvent.click(await screen.findByText("Create rule"));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Choose the folder to watch.");
    expect(alert.textContent).toContain("Action 1: choose a destination.");
    expect(calls("create_automation_rule")).toHaveLength(0);
  });

  it("creates a rule from a preset in the engine's shape", async () => {
    render(<AutomationRuleEditor />);
    open({ trigger: { type: "file_appears", watch_path: "/in", recursive: false } } as Partial<AutomationRule>);
    fireEvent.change(await screen.findByLabelText("Start from a template"), { target: { value: "unzip-arrivals" } });
    fireEvent.change(screen.getByLabelText("Folder to watch", { selector: "input" }), { target: { value: "/in" } });
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    const rule = calls("create_automation_rule")[0].rule as AutomationRule;
    expect(rule.trigger).toEqual({ type: "file_appears", watch_path: "/in", recursive: false });
    expect(rule.conditions).toEqual([{ type: "extension_is", extensions: ["zip"] }]);
    expect(rule.action).toMatchObject({ type: "extract_archive", delete_archive: false });
    expect(rule.enabled).toBe(false);
    expect(rule.options).toMatchObject({ overwrite: "rename", settle_secs: 3, retry: { retries: 3 } });
    // Saved: the button now edits the same rule.
    expect(await screen.findByText("Save changes")).toBeTruthy();
  });

  it("previews destination templates with Rust's renderer", async () => {
    render(<AutomationRuleEditor />);
    open();
    fireEvent.change(await screen.findByLabelText("Destination folder", { selector: "input" }), { target: { value: "/out/{ext}" } });
    fireEvent.change(screen.getByLabelText("Preview templates with this file"), { target: { value: "/in/a.pdf" } });
    expect((await screen.findByTestId("template-preview", {}, { timeout: 2000 })).textContent).toBe("→ /out/pdf");
    expect(calls("naming_preview")[0]).toEqual({ template: "/out/{ext}", samplePath: "/in/a.pdf", matchRegex: null, asPath: true });
  });

  it("dry-runs the unsaved form and shows the outcome", async () => {
    render(<AutomationRuleEditor />);
    open();
    fireEvent.change(screen.getByLabelText("Folder to watch", { selector: "input" }), { target: { value: "/in" } });
    fireEvent.change(screen.getByLabelText("Destination folder", { selector: "input" }), { target: { value: "/out" } });
    fireEvent.click(screen.getByText("Dry run"));
    await waitFor(() => expect(calls("test_automation_rule")).toHaveLength(1));
    expect(await screen.findByText("/in/a.pdf")).toBeTruthy();
    expect(calls("create_automation_rule")).toHaveLength(0);
  });

  it("asks to approve the exact command lines of a saved rule", async () => {
    useUIStore.setState({ appMode: "advanced" });
    render(<AutomationRuleEditor />);
    open();
    fireEvent.change(screen.getByLabelText("Folder to watch", { selector: "input" }), { target: { value: "/in" } });
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "run_command" } });
    fireEvent.change(screen.getByLabelText("Program"), { target: { value: "/usr/bin/lp" } });
    fireEvent.click(screen.getByText("Create rule"));
    const box = await screen.findByRole("region", { name: "Command approval" });
    expect(box.textContent).toContain("/usr/bin/lp {path}");
    fireEvent.click(screen.getByText("Approve commands"));
    await waitFor(() => expect(calls("approve_automation_command")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Command approval" })).toBeNull());
  });

  it("saves a URL download step in the engine's shape (advanced mode)", async () => {
    useUIStore.setState({ appMode: "advanced" });
    render(<AutomationRuleEditor />);
    open();
    fireEvent.change(screen.getByLabelText("Folder to watch", { selector: "input" }), { target: { value: "/in" } });
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "download_url" } });
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "https://example.com/{name}.csv" } });
    fireEvent.change(screen.getByLabelText("Save into folder", { selector: "input" }), { target: { value: "/downloads" } });
    fireEvent.change(screen.getByLabelText("File name (optional)"), { target: { value: "{name}-latest.csv" } });
    fireEvent.change(screen.getByLabelText("Size limit (MB)"), { target: { value: "25" } });
    fireEvent.click(screen.getByLabelText(/Allow intranet addresses/));
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(calls("create_automation_rule")).toHaveLength(1));
    expect(lastSaved!.action).toEqual({ type: "download_url", url: "https://example.com/{name}.csv", dest_path: "/downloads", file_name: "{name}-latest.csv", overwrite: false, max_mb: 25, sha256: null, allow_private: true });
  });

  it("hides the webhook trigger in simple mode", async () => {
    render(<AutomationRuleEditor />);
    open();
    const trigger = (await screen.findByLabelText("Trigger")) as HTMLSelectElement;
    expect([...trigger.options].map((o) => o.value)).not.toContain("webhook");
    act(() => useUIStore.setState({ appMode: "advanced" }));
    expect([...(screen.getByLabelText("Trigger") as HTMLSelectElement).options].map((o) => o.value)).toContain("webhook");
  });
});
