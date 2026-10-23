/** Workflow graph model: connections, loops, layout, validation and run status (mirrors the Rust engine's rules). */
import { describe, expect, it } from "vitest";
import { addNode, autoLayout, connect, defaultKind, removeNode, runStatus, setEdgeOn, validateWorkflow, workflowActions } from "@/lib/workflow-model";
import type { ConditionNode, RuleAction, Workflow } from "@/stores/automation-store";

const cond: ConditionNode = { type: "condition", condition: { type: "extension_is", extensions: ["pdf"] } };
const notify: RuleAction = { type: "notify", title: "t", body: "b" };

function build(): Workflow {
  const [w1, c] = addNode({ nodes: [], edges: [] }, defaultKind("condition", notify, cond));
  const [w2, a] = addNode(w1, defaultKind("action", notify, cond));
  const [w3, b] = addNode(w2, defaultKind("action", { type: "create_folder", path: "/x" }, cond));
  const [w4, j] = addNode(w3, defaultKind("join", notify, cond));
  let wf = w4;
  for (const [f, t] of [[c, a], [c, b], [a, j], [b, j]]) {
    const r = connect(wf, f, t);
    if (typeof r === "string") throw new Error(r);
    wf = r;
  }
  return wf;
}

describe("workflow model", () => {
  it("connects conditions by true/false and actions on success", () => {
    const wf = build();
    expect(wf.edges.map((e) => `${e.from}>${e.to}:${e.on}`)).toEqual(["n1>n2:true", "n1>n3:true", "n2>n4:success", "n3>n4:success"]);
    // A second connection between the same condition and step takes the other branch.
    const again = connect(wf, "n1", "n2");
    expect(typeof again === "string" ? again : again.edges.at(-1)).toEqual({ from: "n1", to: "n2", on: "false" });
    expect(connect(wf, "n4", "n1")).toBe("That connection would make a loop.");
    expect(connect(wf, "n2", "n2")).toBe("A step cannot connect to itself.");
    expect(connect(wf, "n2", "n4")).toBe("These steps are already connected that way.");
    expect(connect(wf, "n1", "n4", "success")).toBe("Connections from a condition are true/false.");
    expect(setEdgeOn(wf, wf.edges[1], "false").edges[1].on).toBe("false");
  });

  it("lays out by depth, validates and removes", () => {
    const wf = autoLayout(build());
    expect(wf.nodes.map((n) => [n.id, n.y])).toEqual([["n1", 0], ["n2", 120], ["n3", 120], ["n4", 240]]);
    expect(wf.nodes[1].x).toBeLessThan(wf.nodes[2].x);
    expect(validateWorkflow(wf)).toEqual([]);
    expect(validateWorkflow({ nodes: [], edges: [] })).toEqual(["Add at least one step to the workflow."]);
    const looped = { ...wf, edges: [...wf.edges, { from: "n4", to: "n1", on: "success" as const }] };
    expect(validateWorkflow(looped)).toContain("The workflow has a loop.");
    const [withSub] = addNode(wf, defaultKind("sub_rule", notify, cond));
    expect(validateWorkflow(withSub)).toEqual(["Nested rule: choose the rule to run."]);
    const removed = removeNode(wf, "n2");
    expect(removed.nodes.map((n) => n.id)).toEqual(["n1", "n3", "n4"]);
    expect(removed.edges.some((e) => e.from === "n2" || e.to === "n2")).toBe(false);
    expect(workflowActions(wf).map((a) => a.type)).toEqual(["notify", "create_folder"]);
  });

  it("derives node status from a run", () => {
    const st = runStatus({
      id: "l", rule_id: "r", rule_name: "R", triggered_at: "", completed_at: null, trigger_event: "", status: "failed", files_affected: [], error_message: null,
      steps: [
        { step: 0, action: "condition", file: null, ok: true, attempts: 1, message: "1 match", node: "n1" },
        { step: 1, action: "notify", file: "/a", ok: true, attempts: 1, message: null, node: "n2" },
        { step: 2, action: "create_folder", file: null, ok: true, attempts: 0, message: "skipped", node: "n3" },
        { step: 3, action: "join", file: "/a", ok: false, attempts: 1, message: "x", node: "n4" },
        { step: 3, action: "join", file: "/b", ok: true, attempts: 1, message: null, node: "n4" },
      ],
    });
    expect(st).toEqual({ n1: "ok", n2: "ok", n3: "skipped", n4: "failed" });
  });
});
