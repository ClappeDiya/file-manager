/**
 * Workflow builder and its editor integration: steps and connections are
 * edited through the inspector and connect form (the canvas renders the same
 * graph), conversion to and from a list, the engine's JSON shape on save,
 * revisions, and the read-only run graph.
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { WorkflowBuilder } from "@/components/workflow-builder";
import { AutomationRuleEditor } from "@/components/automation-rule-editor";
import { useAutomationStore, type AutomationRule, type RuleAction, type Workflow } from "@/stores/automation-store";
import { useUIStore } from "@/stores/ui-store";
import { linearSteps } from "@/lib/automation-rule-model";

beforeAll(() => {
  // React Flow measures its container and reads the zoom from a matrix.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  class Matrix {
    m22 = 1;
    constructor(t?: string) {
      const scale = t?.match(/scale\(([^)]+)\)/);
      if (scale) this.m22 = Number(scale[1]);
    }
  }
  (window as unknown as Record<string, unknown>).DOMMatrixReadOnly ??= Matrix;
});

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

function Harness({ initial, onChange }: { initial: Workflow; onChange: (wf: Workflow) => void }) {
  const [wf, setWf] = useState(initial);
  return <WorkflowBuilder workflow={wf} onChange={(w) => { onChange(w); setWf(w); }} rules={[{ id: "r2", name: "Archive" }]} newAction={() => ({ type: "notify", title: "t", body: "b" })} newCondition={() => ({ type: "all", nodes: [] })} />;
}

describe("WorkflowBuilder", () => {
  it("adds steps, connects them, edits steps and connections, and validates", () => {
    const changes = vi.fn();
    render(<Harness initial={{ nodes: [], edges: [] }} onChange={changes} />);
    expect(screen.getByLabelText("Workflow problems")).toHaveTextContent("Add at least one step");
    fireEvent.click(screen.getByText("Add step"));
    fireEvent.change(screen.getByLabelText("Step type"), { target: { value: "delay" } });
    fireEvent.click(screen.getByText("Add step"));
    fireEvent.change(screen.getByLabelText("Delay seconds"), { target: { value: "30" } });
    fireEvent.change(screen.getByLabelText("Step type"), { target: { value: "sub_rule" } });
    fireEvent.click(screen.getByText("Add step"));
    expect(screen.getByLabelText("Workflow problems")).toHaveTextContent("Nested rule: choose the rule to run.");
    fireEvent.change(screen.getByLabelText("Nested rule"), { target: { value: "r2" } });
    expect(screen.queryByLabelText("Workflow problems")).toBeNull();

    fireEvent.change(screen.getByLabelText("Connect from"), { target: { value: "n1" } });
    fireEvent.change(screen.getByLabelText("Connect to"), { target: { value: "n2" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    fireEvent.change(screen.getByLabelText("Connect from"), { target: { value: "n2" } });
    fireEvent.change(screen.getByLabelText("Connect to"), { target: { value: "n3" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    // A loop is refused with a message.
    fireEvent.change(screen.getByLabelText("Connect from"), { target: { value: "n3" } });
    fireEvent.change(screen.getByLabelText("Connect to"), { target: { value: "n1" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(screen.getByRole("status")).toHaveTextContent("would make a loop");

    const last = changes.mock.calls.at(-1)![0] as Workflow;
    expect(last.nodes.map((n) => [n.id, n.kind])).toEqual([["n1", "action"], ["n2", "delay"], ["n3", "sub_rule"]]);
    expect(last.nodes[1]).toMatchObject({ kind: "delay", seconds: 30 });
    expect(last.nodes[2]).toMatchObject({ kind: "sub_rule", rule_id: "r2" });
    expect(last.edges).toEqual([{ from: "n1", to: "n2", on: "success" }, { from: "n2", to: "n3", on: "success" }]);
    expect(screen.getByLabelText("Workflow steps")).toHaveTextContent("→ n2 (on success)");

    // Delete a step: its connections go too.
    fireEvent.click(screen.getByLabelText(/^Step n2:/));
    fireEvent.click(screen.getByText("Delete step"));
    expect((changes.mock.calls.at(-1)![0] as Workflow).edges).toEqual([]);
  });

  it("shows a run read-only with step statuses", () => {
    const wf: Workflow = { nodes: [{ id: "n1", kind: "action", action: { type: "notify", title: "a", body: "b" }, label: "Tell", x: 0, y: 0 }, { id: "n2", kind: "join", all: false, label: "", x: 0, y: 120 }], edges: [{ from: "n1", to: "n2", on: "success" }] };
    render(<WorkflowBuilder workflow={wf} status={{ n1: "ok", n2: "skipped" }} />);
    expect(screen.queryByText("Add step")).toBeNull();
    const steps = screen.getByLabelText("Workflow steps");
    expect(within(steps).getByText("ok")).toBeInTheDocument();
    expect(within(steps).getByText("skipped")).toBeInTheDocument();
  });

  it("turns a chain back into a list, or refuses when it branches", () => {
    const a = { type: "notify" as const, title: "1", body: "" };
    const chain: Workflow = { nodes: ["n1", "n2", "n3"].map((id, i) => ({ id, kind: "action" as const, action: { ...a, title: String(i) }, label: "", x: 0, y: 0 })), edges: [{ from: "n1", to: "n2", on: "success" }, { from: "n2", to: "n3", on: "always" }] };
    expect(linearSteps(chain)!.map((s) => [s.action.type === "notify" && s.action.title, s.continue_on_error])).toEqual([["0", false], ["1", true], ["2", false]]);
    expect(linearSteps({ ...chain, edges: [{ from: "n1", to: "n2", on: "success" }, { from: "n1", to: "n3", on: "success" }] })).toBeNull();
    expect(linearSteps({ ...chain, edges: [{ from: "n1", to: "n2", on: "always" }, { from: "n2", to: "n3", on: "success" }] })).toBeNull();
  });
});

describe("rule editor with a workflow", () => {
  let saved: AutomationRule | null = null;
  beforeEach(() => {
    saved = null;
    useUIStore.setState({ appMode: "advanced" });
    useAutomationStore.setState({ editorOpen: false, editorRule: null, rules: [] });
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "automation_convert_to_workflow": {
          const r = args!.rule as AutomationRule;
          return { nodes: [{ id: "n1", kind: "action", action: r.action, label: "", x: 0, y: 0 }], edges: [] };
        }
        case "create_automation_rule":
        case "update_automation_rule":
          saved = args!.rule as AutomationRule;
          return saved;
        case "automation_rule_revisions":
          return [{ revision: 1, saved_at: "2026-10-01T10:00:00Z", name: "Before", workflow: false, actions: 1, rule: { ...(args as object), id: "r1" } }];
        case "automation_restore_revision":
          return { id: "r1", name: "Before", enabled: false, trigger: { type: "manual" }, conditions: [], action: { type: "create_folder", path: "/old" }, created_at: "", updated_at: "", last_triggered: null, trigger_count: 0, error_count: 0, last_error: null };
        default:
          return [];
      }
    });
  });

  it("converts the actions to a workflow and saves the graph", async () => {
    render(<AutomationRuleEditor />);
    act(() => useAutomationStore.getState().openEditor());
    fireEvent.change(await screen.findByLabelText("Trigger"), { target: { value: "manual" } });
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "create_folder" } });
    fireEvent.change(screen.getByLabelText("Folder", { selector: "input" }), { target: { value: "/out" } });
    fireEvent.click(screen.getByText("Edit as a workflow (graph)"));
    expect(await screen.findByTestId("workflow-builder")).toBeInTheDocument();
    expect(screen.queryByLabelText("Action 1")).toBeNull();
    fireEvent.change(screen.getByLabelText("Step type"), { target: { value: "delay" } });
    fireEvent.click(screen.getByText("Add step"));
    fireEvent.change(screen.getByLabelText("Connect from"), { target: { value: "n1" } });
    fireEvent.change(screen.getByLabelText("Connect to"), { target: { value: "n2" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    fireEvent.click(screen.getByText("Create rule"));
    await waitFor(() => expect(saved).not.toBeNull());
    expect(saved!.workflow!.nodes.map((n) => n.kind)).toEqual(["action", "delay"]);
    expect(saved!.workflow!.edges).toEqual([{ from: "n1", to: "n2", on: "success" }]);
    expect(saved!.action).toEqual({ type: "create_folder", path: "/out" });
    expect(saved!.steps).toEqual([]);
  });

  it("refuses to save an invalid workflow and goes back to a list", async () => {
    render(<AutomationRuleEditor />);
    act(() => useAutomationStore.getState().openEditor());
    fireEvent.change(await screen.findByLabelText("Trigger"), { target: { value: "manual" } });
    fireEvent.change(screen.getByLabelText("Action 1"), { target: { value: "create_folder" } });
    fireEvent.change(screen.getByLabelText("Folder", { selector: "input" }), { target: { value: "/out" } });
    fireEvent.click(screen.getByText("Edit as a workflow (graph)"));
    await screen.findByTestId("workflow-builder");
    fireEvent.change(screen.getByLabelText("Step type"), { target: { value: "sub_rule" } });
    fireEvent.click(screen.getByText("Add step"));
    fireEvent.click(screen.getByText("Create rule"));
    expect(await screen.findAllByText(/choose the rule to run/)).not.toHaveLength(0);
    expect(saved).toBeNull();
    fireEvent.click(screen.getByLabelText(/^Step n2:/));
    fireEvent.click(screen.getByText("Delete step"));
    fireEvent.click(screen.getByText("Use a list of actions instead"));
    expect(screen.getByLabelText("Action 1")).toBeInTheDocument();
  });

  it("lists earlier versions of a saved rule and restores one", async () => {
    const rule: AutomationRule = { id: "r1", name: "Now", enabled: false, trigger: { type: "manual" }, conditions: [], action: { type: "create_folder", path: "/new" }, created_at: "", updated_at: "", last_triggered: null, trigger_count: 0, error_count: 0, last_error: null };
    useAutomationStore.setState({ rules: [rule] });
    render(<AutomationRuleEditor />);
    act(() => useAutomationStore.getState().openEditor(rule));
    const history = await screen.findByTestId("rule-history");
    expect(history).toHaveTextContent("History (1 earlier version)");
    fireEvent.click(within(history).getByText("Restore"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("automation_restore_revision", { ruleId: "r1", revision: 1 }));
    await waitFor(() => expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Before"));
  });
});

describe("WorkflowBuilder results and risk", () => {
  const wf: Workflow = {
    nodes: [
      { id: "n1", kind: "action", action: { type: "list_files", path: "/a", output: "/o.csv", format: "csv", recursive: true, include_dirs: false, overwrite: true }, label: "", x: 0, y: 0, output_as: "listing" },
      { id: "n2", kind: "action", action: { type: "purge", path: "/tmp/x", max_files: null }, label: "", x: 0, y: 120 },
    ],
    edges: [{ from: "n1", to: "n2", on: "success" }],
  };

  it("badges each step with its risk and names a step's result along the edges", () => {
    const changes = vi.fn();
    const riskOf = (a: RuleAction) => (a.type === "purge" ? { level: "high" as const, reason: "Removes a folder and everything in it" } : { level: "low" as const, reason: "Writes a file listing" });
    function Wf() {
      const [w, setW] = useState(wf);
      return <WorkflowBuilder workflow={w} onChange={(x) => { changes(x); setW(x); }} riskOf={riskOf} />;
    }
    render(<Wf />);
    const list = screen.getByLabelText("Workflow steps");
    expect(within(list).getAllByTestId("risk-badge").map((b) => b.getAttribute("data-risk"))).toEqual(["low", "high"]);
    // The second step sees the first step's result name.
    fireEvent.click(screen.getByLabelText(/^Step n2:/));
    expect(screen.getByLabelText("Selected step")).toHaveTextContent("available here: listing");
    fireEvent.change(screen.getByLabelText("Result name"), { target: { value: "purged" } });
    expect((changes.mock.calls.at(-1)![0] as Workflow).nodes[1]).toMatchObject({ id: "n2", output_as: "purged" });
    // The first step sees none (nothing upstream).
    fireEvent.click(screen.getByLabelText(/^Step n1:/));
    expect(screen.getByLabelText("Selected step")).not.toHaveTextContent("available here");
  });
});
