/**
 * Workflow graph editing (pure; mirrors `automation_engine::workflow`):
 * nodes, connections and their rules, layout, validation, and the status
 * of each node in a run.
 */
import type { AutomationLog, ConditionNode, EdgeOn, RuleAction, Workflow, WorkflowEdge, WorkflowNode, WorkflowNodeKind } from "@/stores/automation-store";

export const MAX_NODES = 200;
export const MAX_DELAY_SECS = 24 * 3600;

export type NodeType = WorkflowNodeKind["kind"];

export const NODE_TYPES: { kind: NodeType; label: string }[] = [
  { kind: "action", label: "Action" },
  { kind: "condition", label: "Condition" },
  { kind: "delay", label: "Delay" },
  { kind: "join", label: "Join" },
  { kind: "sub_rule", label: "Nested rule" },
];

export const EDGE_LABELS: Record<EdgeOn, string> = { success: "on success", failure: "on failure", always: "always", true: "if true", false: "if false" };

const nextId = (wf: Workflow) => {
  let i = wf.nodes.length + 1;
  while (wf.nodes.some((n) => n.id === `n${i}`)) i++;
  return `n${i}`;
};

export function defaultKind(kind: NodeType, action: RuleAction, condition: ConditionNode): WorkflowNodeKind {
  switch (kind) {
    case "action":
      return { kind, action };
    case "condition":
      return { kind, condition };
    case "delay":
      return { kind, seconds: 60 };
    case "join":
      return { kind, all: false };
    case "sub_rule":
      return { kind, rule_id: "" };
  }
}

/** Add a node below the lowest one; returns the workflow and the new id. */
export function addNode(wf: Workflow, kind: WorkflowNodeKind): [Workflow, string] {
  const id = nextId(wf);
  const y = wf.nodes.reduce((m, n) => Math.max(m, n.y), -120) + 120;
  return [{ ...wf, nodes: [...wf.nodes, { ...kind, id, label: "", x: 0, y } as WorkflowNode] }, id];
}

export function removeNode(wf: Workflow, id: string): Workflow {
  return { nodes: wf.nodes.filter((n) => n.id !== id), edges: wf.edges.filter((e) => e.from !== id && e.to !== id) };
}

export function updateNode(wf: Workflow, id: string, patch: Partial<WorkflowNode>): Workflow {
  return { ...wf, nodes: wf.nodes.map((n) => (n.id === id ? ({ ...n, ...patch } as WorkflowNode) : n)) };
}

/** Whether `to` can reach `from` (adding from → to would close a loop). */
function reaches(wf: Workflow, start: string, target: string): boolean {
  const seen = new Set<string>();
  const stack = [start];
  while (stack.length) {
    const n = stack.pop()!;
    if (n === target) return true;
    if (seen.has(n)) continue;
    seen.add(n);
    wf.edges.filter((e) => e.from === n).forEach((e) => stack.push(e.to));
  }
  return false;
}

/** Connect two nodes; conditions branch on true (then false), others on success. */
export function connect(wf: Workflow, from: string, to: string, on?: EdgeOn): Workflow | string {
  if (from === to) return "A step cannot connect to itself.";
  if (reaches(wf, to, from)) return "That connection would make a loop.";
  const src = wf.nodes.find((n) => n.id === from);
  if (!src || !wf.nodes.some((n) => n.id === to)) return "Unknown step.";
  const isCond = src.kind === "condition";
  const used = wf.edges.filter((e) => e.from === from && e.to === to).map((e) => e.on);
  const choice = on ?? (isCond ? (used.includes("true") ? "false" : "true") : "success");
  if (isCond !== (choice === "true" || choice === "false")) return "Connections from a condition are true/false.";
  if (used.includes(choice)) return "These steps are already connected that way.";
  return { ...wf, edges: [...wf.edges, { from, to, on: choice }] };
}

export function setEdgeOn(wf: Workflow, edge: WorkflowEdge, on: EdgeOn): Workflow {
  return { ...wf, edges: wf.edges.map((e) => (e === edge || (e.from === edge.from && e.to === edge.to && e.on === edge.on) ? { ...e, on } : e)) };
}

export function removeEdge(wf: Workflow, edge: WorkflowEdge): Workflow {
  return { ...wf, edges: wf.edges.filter((e) => !(e.from === edge.from && e.to === edge.to && e.on === edge.on)) };
}

/** Lay nodes out in rows by depth (longest path from a start step). */
export function autoLayout(wf: Workflow): Workflow {
  const depth = new Map<string, number>();
  const visit = (id: string, d: number, path: Set<string>) => {
    if (path.has(id)) return;
    if ((depth.get(id) ?? -1) >= d) return;
    depth.set(id, d);
    const next = new Set(path).add(id);
    wf.edges.filter((e) => e.from === id).forEach((e) => visit(e.to, d + 1, next));
  };
  wf.nodes.filter((n) => !wf.edges.some((e) => e.to === n.id)).forEach((n) => visit(n.id, 0, new Set()));
  const rows = new Map<number, string[]>();
  wf.nodes.forEach((n) => {
    const d = depth.get(n.id) ?? 0;
    rows.set(d, [...(rows.get(d) ?? []), n.id]);
  });
  return {
    ...wf,
    nodes: wf.nodes.map((n) => {
      const d = depth.get(n.id) ?? 0;
      const row = rows.get(d)!;
      return { ...n, x: (row.indexOf(n.id) - (row.length - 1) / 2) * 240, y: d * 120 };
    }),
  };
}

/** Result names saved by the steps upstream of `id` (the ones its fields may use as {steps.<name>.…}). */
export function earlierWorkflowResults(wf: Workflow, id: string): string[] {
  const upstream = new Set<string>();
  const stack = [id];
  while (stack.length) {
    const cur = stack.pop()!;
    for (const e of wf.edges) {
      if (e.to === cur && !upstream.has(e.from)) {
        upstream.add(e.from);
        stack.push(e.from);
      }
    }
  }
  return wf.nodes.filter((n) => upstream.has(n.id)).map((n) => n.output_as?.trim() ?? "").filter(Boolean);
}

/** Problems that would make the engine refuse the workflow. */
export function validateWorkflow(wf: Workflow): string[] {
  const errors: string[] = [];
  if (wf.nodes.length === 0) errors.push("Add at least one step to the workflow.");
  if (wf.nodes.length > MAX_NODES) errors.push(`A workflow can have at most ${MAX_NODES} steps.`);
  const ids = new Set<string>();
  for (const n of wf.nodes) {
    if (ids.has(n.id)) errors.push(`Two steps share the id ${n.id}.`);
    ids.add(n.id);
    if (n.kind === "delay" && (n.seconds < 0 || n.seconds > MAX_DELAY_SECS)) errors.push(`${nodeTitle(n)}: a delay is 0–${MAX_DELAY_SECS} seconds.`);
    if (n.kind === "sub_rule" && !n.rule_id) errors.push(`${nodeTitle(n)}: choose the rule to run.`);
  }
  for (const e of wf.edges) {
    if (!ids.has(e.from) || !ids.has(e.to)) errors.push(`A connection refers to a missing step (${e.from} → ${e.to}).`);
  }
  // Kahn: any node left over sits on a loop.
  const indeg = new Map(wf.nodes.map((n) => [n.id, 0]));
  wf.edges.forEach((e) => indeg.set(e.to, (indeg.get(e.to) ?? 0) + 1));
  const q = wf.nodes.filter((n) => indeg.get(n.id) === 0).map((n) => n.id);
  let seen = 0;
  while (q.length) {
    const id = q.shift()!;
    seen++;
    wf.edges.filter((e) => e.from === id).forEach((e) => {
      const d = (indeg.get(e.to) ?? 0) - 1;
      indeg.set(e.to, d);
      if (d === 0) q.push(e.to);
    });
  }
  if (seen < wf.nodes.length) errors.push("The workflow has a loop.");
  return errors;
}

export function nodeTitle(n: WorkflowNode, actionLabel?: (a: RuleAction) => string): string {
  if (n.label.trim()) return n.label;
  switch (n.kind) {
    case "action":
      return actionLabel ? actionLabel(n.action) : n.action.type;
    case "condition":
      return "Condition";
    case "delay":
      return `Wait ${n.seconds}s`;
    case "join":
      return n.all ? "Join (all)" : "Join";
    case "sub_rule":
      return "Nested rule";
  }
}

export type NodeStatus = "ok" | "failed" | "skipped";

/** Status of each node in a run (from its step results). */
export function runStatus(log: AutomationLog | null | undefined): Record<string, NodeStatus> {
  const out: Record<string, NodeStatus> = {};
  for (const s of log?.steps ?? []) {
    if (!s.node) continue;
    const st: NodeStatus = s.message === "skipped" ? "skipped" : s.ok ? "ok" : "failed";
    if (out[s.node] !== "failed") out[s.node] = st === "skipped" && out[s.node] ? out[s.node] : st;
  }
  return out;
}

/** Every action a workflow (or linear rule) runs. */
export function workflowActions(wf: Workflow): RuleAction[] {
  return wf.nodes.flatMap((n) => (n.kind === "action" ? [n.action] : []));
}
