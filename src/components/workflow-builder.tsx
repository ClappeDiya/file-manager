/**
 * Visual workflow editor (Advanced mode): the rule's actions as a graph on a
 * React Flow canvas, with an inspector for the selected step or connection,
 * a keyboard-friendly step list and connect form, and validation. Read-only
 * with `status` it draws a run: each step coloured by how it ended.
 */
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { applyNodeChanges, Background, Controls, Handle, Position, ReactFlow, type Connection, type Edge, type Node, type NodeChange, type NodeProps } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { addNode, autoLayout, connect, defaultKind, earlierWorkflowResults, EDGE_LABELS, NODE_TYPES, nodeTitle, removeEdge, removeNode, setEdgeOn, updateNode, validateWorkflow, type NodeStatus, type NodeType } from "@/lib/workflow-model";
import { RiskBadge } from "@/components/step-risk";
import { RESULT_FIELDS } from "@/lib/automation-rule-model";
import type { ConditionNode, EdgeOn, RuleAction, StepRisk, Workflow, WorkflowEdge, WorkflowNode } from "@/stores/automation-store";

const field = "w-full min-w-0 rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";
const btn = "px-2 py-1 text-xs rounded border border-[var(--color-border)] hover:bg-[var(--color-hover)] disabled:opacity-50";
const STATUS_CLASS: Record<NodeStatus, string> = { ok: "border-green-500 bg-green-50 dark:bg-green-900/30", failed: "border-red-500 bg-red-50 dark:bg-red-900/30", skipped: "border-dashed opacity-60" };
const KIND_TAG: Record<NodeType, string> = { action: "Action", condition: "Condition", delay: "Delay", join: "Join", sub_rule: "Nested rule" };

interface StepData extends Record<string, unknown> {
  title: string;
  kind: NodeType;
  status?: NodeStatus;
  selected: boolean;
  risk?: StepRisk;
}

function StepNode({ data }: NodeProps<Node<StepData>>) {
  return (
    <div className={`rounded border-2 px-3 py-2 text-xs bg-[var(--color-surface)] min-w-[140px] ${data.status ? STATUS_CLASS[data.status] : "border-[var(--color-border)]"} ${data.selected ? "ring-2 ring-blue-500" : ""}`} data-testid="wf-node">
      <Handle type="target" position={Position.Top} />
      <div className="text-[10px] uppercase tracking-wide text-[var(--color-text-muted)]">{KIND_TAG[data.kind]}</div>
      <div className="font-medium truncate max-w-[180px]">{data.title}</div>
      <RiskBadge risk={data.risk} />
      {data.status && <div className="text-[10px]">{data.status}</div>}
      <Handle type="source" position={Position.Bottom} />
    </div>
  );
}

const nodeTypes = { step: StepNode };
const edgeKey = (e: WorkflowEdge) => `${e.from}->${e.to}:${e.on}`;

export interface WorkflowBuilderProps {
  workflow: Workflow;
  /** Omit for a read-only view (a run graph). */
  onChange?: (wf: Workflow) => void;
  status?: Record<string, NodeStatus>;
  rules?: { id: string; name: string }[];
  actionLabel?: (a: RuleAction) => string;
  renderAction?: (a: RuleAction, onChange: (a: RuleAction) => void, idx: number) => ReactNode;
  renderCondition?: (c: ConditionNode, onChange: (c: ConditionNode) => void) => ReactNode;
  newAction?: () => RuleAction;
  /** Risk of an action, shown as a badge on its step (Advanced). */
  riskOf?: (a: RuleAction) => StepRisk | undefined;
  newCondition?: () => ConditionNode;
}

export function WorkflowBuilder({ workflow, onChange, status, rules = [], actionLabel, renderAction, renderCondition, newAction, newCondition, riskOf }: WorkflowBuilderProps) {
  const readOnly = !onChange;
  const [selected, setSelected] = useState<string | null>(null);
  const [selectedEdge, setSelectedEdge] = useState<string | null>(null);
  const [addKind, setAddKind] = useState<NodeType>("action");
  const [link, setLink] = useState<{ from: string; to: string }>({ from: "", to: "" });
  const [message, setMessage] = useState<string | null>(null);
  const errors = useMemo(() => (readOnly ? [] : validateWorkflow(workflow)), [workflow, readOnly]);

  const toNodes = useCallback(
    (wf: Workflow): Node<StepData>[] => wf.nodes.map((n) => ({ id: n.id, type: "step", position: { x: n.x, y: n.y }, draggable: !readOnly, data: { title: nodeTitle(n, actionLabel), kind: n.kind, status: status?.[n.id], selected: n.id === selected, risk: n.kind === "action" ? riskOf?.(n.action) : undefined } })),
    [actionLabel, readOnly, riskOf, selected, status],
  );
  const [rfNodes, setRfNodes] = useState<Node<StepData>[]>(() => toNodes(workflow));
  useEffect(() => setRfNodes(toNodes(workflow)), [workflow, toNodes]);
  const rfEdges: Edge[] = useMemo(
    () =>
      workflow.edges.map((e) => ({
        id: edgeKey(e),
        source: e.from,
        target: e.to,
        label: EDGE_LABELS[e.on],
        animated: e.on === "always",
        style: { stroke: e.on === "failure" ? "#dc2626" : e.on === "false" ? "#9ca3af" : undefined, strokeWidth: edgeKey(e) === selectedEdge ? 3 : 1.5 },
      })),
    [workflow.edges, selectedEdge],
  );

  const change = (wf: Workflow | string) => {
    if (typeof wf === "string") {
      setMessage(wf);
      return;
    }
    setMessage(null);
    onChange?.(wf);
  };
  const onNodesChange = (changes: NodeChange<Node<StepData>>[]) => setRfNodes((ns) => applyNodeChanges(changes, ns));
  const onConnect = (c: Connection) => c.source && c.target && change(connect(workflow, c.source, c.target));
  const node = workflow.nodes.find((n) => n.id === selected) ?? null;
  const edge = workflow.edges.find((e) => edgeKey(e) === selectedEdge) ?? null;
  const fromCondition = (e: WorkflowEdge) => workflow.nodes.find((n) => n.id === e.from)?.kind === "condition";
  const patchNode = (patch: Partial<WorkflowNode>) => node && change(updateNode(workflow, node.id, patch));

  return (
    <div className="space-y-2" data-testid="workflow-builder">
      {!readOnly && (
        <div className="flex flex-wrap items-center gap-1">
          <select aria-label="Step type" className={`${field} w-auto`} value={addKind} onChange={(e) => setAddKind(e.target.value as NodeType)}>
            {NODE_TYPES.map((t) => (
              <option key={t.kind} value={t.kind}>
                {t.label}
              </option>
            ))}
          </select>
          <button
            type="button"
            className={btn}
            onClick={() => {
              const [wf, id] = addNode(workflow, defaultKind(addKind, newAction?.() ?? { type: "notify", title: "{rule}", body: "{filename}" }, newCondition?.() ?? { type: "all", nodes: [] }));
              change(wf);
              setSelected(id);
              setSelectedEdge(null);
            }}
          >
            Add step
          </button>
          <button type="button" className={btn} onClick={() => change(autoLayout(workflow))}>
            Arrange
          </button>
        </div>
      )}
      <div className="h-80 rounded border border-[var(--color-border)]" aria-label="Workflow canvas">
        <ReactFlow
          nodes={rfNodes}
          edges={rfEdges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onNodeDragStop={(_, n) => !readOnly && change(updateNode(workflow, n.id, { x: Math.round(n.position.x), y: Math.round(n.position.y) }))}
          onConnect={readOnly ? undefined : onConnect}
          onNodeClick={(_, n) => {
            setSelected(n.id);
            setSelectedEdge(null);
          }}
          onEdgeClick={(_, e) => {
            setSelectedEdge(e.id);
            setSelected(null);
          }}
          nodesConnectable={!readOnly}
          fitView
          proOptions={{ hideAttribution: true }}
        >
          <Background />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>

      {/* The same graph as a list (keyboard and screen readers). */}
      <ol className="text-xs space-y-0.5" aria-label="Workflow steps">
        {workflow.nodes.map((n) => (
          <li key={n.id} className="flex gap-2 items-center">
            <button type="button" className={`text-left hover:underline ${n.id === selected ? "font-semibold" : ""}`} onClick={() => setSelected(n.id)} aria-label={`Step ${n.id}: ${nodeTitle(n, actionLabel)}`}>
              {n.id}. {nodeTitle(n, actionLabel)}
            </button>
            {n.kind === "action" && <RiskBadge risk={riskOf?.(n.action)} />}
            {status?.[n.id] && <span className="text-[10px] text-[var(--color-text-muted)]">{status[n.id]}</span>}
            <span className="text-[10px] text-[var(--color-text-muted)]">
              {workflow.edges
                .filter((e) => e.from === n.id)
                .map((e) => `→ ${e.to} (${EDGE_LABELS[e.on]})`)
                .join(", ")}
            </span>
          </li>
        ))}
      </ol>

      {!readOnly && workflow.nodes.length > 1 && (
        <div className="flex flex-wrap items-center gap-1 text-xs">
          <span>Connect</span>
          <select aria-label="Connect from" className={`${field} w-auto`} value={link.from} onChange={(e) => setLink({ ...link, from: e.target.value })}>
            <option value="">step…</option>
            {workflow.nodes.map((n) => (
              <option key={n.id} value={n.id}>
                {n.id}
              </option>
            ))}
          </select>
          <span>to</span>
          <select aria-label="Connect to" className={`${field} w-auto`} value={link.to} onChange={(e) => setLink({ ...link, to: e.target.value })}>
            <option value="">step…</option>
            {workflow.nodes.map((n) => (
              <option key={n.id} value={n.id}>
                {n.id}
              </option>
            ))}
          </select>
          <button type="button" className={btn} disabled={!link.from || !link.to} onClick={() => change(connect(workflow, link.from, link.to))}>
            Connect
          </button>
        </div>
      )}

      {!readOnly && node && (
        <section className="rounded border border-[var(--color-border)] p-2 space-y-2" aria-label="Selected step">
          <div className="flex items-center gap-2">
            <b className="text-xs">
              {KIND_TAG[node.kind]} {node.id}
            </b>
            <input aria-label="Step label" className={field} placeholder={nodeTitle({ ...node, label: "" }, actionLabel)} value={node.label} onChange={(e) => patchNode({ label: e.target.value })} />
            <button
              type="button"
              className={btn}
              onClick={() => {
                change(removeNode(workflow, node.id));
                setSelected(null);
              }}
            >
              Delete step
            </button>
          </div>
          {node.kind === "action" && renderAction?.(node.action, (action) => patchNode({ action } as Partial<WorkflowNode>), workflow.nodes.indexOf(node) + 100)}
          {(node.kind === "action" || node.kind === "sub_rule") && (
            <div className="space-y-1">
              {node.kind === "action" && <RiskBadge risk={riskOf?.(node.action)} />}
              <label className="text-xs block">
                Save this step&apos;s result as (optional)
                <input aria-label="Result name" className={`${field} font-mono`} placeholder="e.g. listing" value={node.output_as ?? ""} onChange={(e) => patchNode({ output_as: e.target.value || null })} />
              </label>
              <p className="text-[11px] text-[var(--color-text-muted)]">
                Later steps can use {RESULT_FIELDS.map((f) => `{steps.${node.output_as?.trim() || "name"}.${f}}`).join(" ")}
                {earlierWorkflowResults(workflow, node.id).length > 0 && <> · available here: {earlierWorkflowResults(workflow, node.id).join(", ")}</>}
              </p>
            </div>
          )}
          {node.kind === "condition" && renderCondition?.(node.condition, (condition) => patchNode({ condition } as Partial<WorkflowNode>))}
          {node.kind === "delay" && (
            <label className="text-xs block">
              Wait (seconds)
              <input aria-label="Delay seconds" type="number" min={0} max={86400} className={field} value={node.seconds} onChange={(e) => patchNode({ seconds: Number(e.target.value) } as Partial<WorkflowNode>)} />
            </label>
          )}
          {node.kind === "join" && (
            <label className="text-xs flex items-center gap-1">
              <input type="checkbox" checked={node.all} onChange={(e) => patchNode({ all: e.target.checked } as Partial<WorkflowNode>)} />
              Wait for every incoming branch to succeed (otherwise any one)
            </label>
          )}
          {node.kind === "sub_rule" && (
            <label className="text-xs block">
              Rule to run
              <select aria-label="Nested rule" className={field} value={node.rule_id} onChange={(e) => patchNode({ rule_id: e.target.value } as Partial<WorkflowNode>)}>
                <option value="">Choose…</option>
                {rules.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.name}
                  </option>
                ))}
              </select>
            </label>
          )}
        </section>
      )}

      {!readOnly && edge && (
        <section className="rounded border border-[var(--color-border)] p-2 flex items-center gap-2 text-xs" aria-label="Selected connection">
          <span>
            {edge.from} → {edge.to}
          </span>
          <select aria-label="Connection fires" className={`${field} w-auto`} value={edge.on} onChange={(e) => change(setEdgeOn(workflow, edge, e.target.value as EdgeOn))}>
            {(fromCondition(edge) ? (["true", "false"] as EdgeOn[]) : (["success", "failure", "always"] as EdgeOn[])).map((o) => (
              <option key={o} value={o}>
                {EDGE_LABELS[o]}
              </option>
            ))}
          </select>
          <button
            type="button"
            className={btn}
            onClick={() => {
              change(removeEdge(workflow, edge));
              setSelectedEdge(null);
            }}
          >
            Delete connection
          </button>
        </section>
      )}

      {message && (
        <p className="text-xs text-amber-600" role="status">
          {message}
        </p>
      )}
      {errors.length > 0 && (
        <ul className="text-xs text-red-600" aria-label="Workflow problems">
          {errors.map((e) => (
            <li key={e}>{e}</li>
          ))}
        </ul>
      )}
      {!readOnly && (
        <p className="text-[11px] text-[var(--color-text-muted)]">
          Branches run in parallel. A step waits for its inputs and runs when one of them fires (a join set to “every” needs all). Files flow along connections; moving, renaming or downloading hands the new file on.
        </p>
      )}
    </div>
  );
}
