//! Workflows: a rule's actions as a directed acyclic graph instead of a list.
//!
//! - **Nodes**: an action, a condition (splits files into a true and a false
//!   branch), a delay, a join, or another rule (nested workflow).
//! - **Edges** carry files from one node to the next. An edge fires on
//!   success (default), failure, always, or a condition's true/false branch.
//! - Branches run in parallel. A node waits for all its inputs to finish and
//!   then runs when at least one incoming edge fired (a `join` in `all` mode
//!   needs every one); otherwise it is skipped, and skipping propagates.
//! - Files flow along edges: a node receives the union of what its fired
//!   inputs carry (aggregation), an action runs once per file, and moving,
//!   renaming or downloading hands the new path on (other actions pass their
//!   input through).
//! - The same executor runs every action; each node's outcome is recorded
//!   with its id, so the run can be drawn on the graph.
//!
//! A linear rule (`action` + `steps`) converts to a workflow with
//! [`Workflow::from_rule`].

use super::executor::{action_kind, dispatch_step, hands_on_path, is_transient, ExecEnv};
use super::outputs::StepOutput;
use super::{AutomationRule, ConditionNode, RuleAction, StepResult};
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

pub const MAX_NODES: usize = 200;
pub const MAX_DELAY_SECS: u64 = 24 * 3600;
pub const MAX_NESTING: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Workflow {
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(flatten)]
    pub kind: NodeKind,
    #[serde(default)]
    pub label: String,
    /// Name under which an action step's result is saved for the steps
    /// downstream of it (`{steps.<name>.path}`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_as: Option<String>,
    /// Canvas position (the editor's; ignored by the engine).
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    Action {
        action: Box<RuleAction>,
    },
    /// Files matching go down `true` edges, the others down `false` edges.
    Condition {
        condition: ConditionNode,
    },
    Delay {
        seconds: u64,
    },
    /// Merge branches: `all` waits for every input to fire, otherwise any.
    Join {
        #[serde(default)]
        all: bool,
    },
    /// Run another rule's actions (or workflow) on the files.
    SubRule {
        rule_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum EdgeOn {
    #[default]
    Success,
    Failure,
    Always,
    True,
    False,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub on: EdgeOn,
}

/// How a node ended.
#[derive(Debug, Clone, PartialEq)]
enum NodeState {
    Done { ok: bool, out: Vec<String>, yes: Vec<String>, no: Vec<String> },
    Skipped,
}

impl NodeKind {
    pub fn label(&self) -> String {
        match self {
            NodeKind::Action { action } => action_kind(action).to_string(),
            NodeKind::Condition { .. } => "condition".into(),
            NodeKind::Delay { seconds } => format!("delay {seconds}s"),
            NodeKind::Join { all } => if *all { "join (all)" } else { "join" }.into(),
            NodeKind::SubRule { .. } => "sub_rule".into(),
        }
    }
}

impl Workflow {
    /// The workflow equivalent of a linear rule: one node per step, each
    /// following the previous on success (always when it may continue on error).
    pub fn from_rule(rule: &AutomationRule) -> Self {
        let steps = rule.all_steps();
        let outputs: Vec<Option<String>> = std::iter::once(rule.output_as.clone()).chain(rule.steps.iter().map(|s| s.output_as.clone())).collect();
        let nodes: Vec<WorkflowNode> = steps
            .iter()
            .enumerate()
            .map(|(i, (a, _))| WorkflowNode { id: format!("n{}", i + 1), kind: NodeKind::Action { action: Box::new((*a).clone()) }, label: String::new(), output_as: outputs[i].clone(), x: 0.0, y: i as f64 * 120.0 })
            .collect();
        let edges = steps
            .windows(2)
            .enumerate()
            .map(|(i, w)| WorkflowEdge { from: format!("n{}", i + 1), to: format!("n{}", i + 2), on: if w[0].1 { EdgeOn::Always } else { EdgeOn::Success } })
            .collect();
        Self { nodes, edges }
    }

    /// Structural checks: unique ids, known endpoints, no cycles, limits.
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("The workflow has no steps".into());
        }
        if self.nodes.len() > MAX_NODES {
            return Err(format!("A workflow can have at most {MAX_NODES} steps"));
        }
        let mut ids = HashSet::new();
        for n in &self.nodes {
            if n.id.trim().is_empty() || !ids.insert(n.id.as_str()) {
                return Err(format!("Step ids must be unique and non-empty ('{}')", n.id));
            }
            match &n.kind {
                NodeKind::Delay { seconds } if *seconds > MAX_DELAY_SECS => return Err(format!("A delay can be at most {MAX_DELAY_SECS} seconds")),
                NodeKind::SubRule { rule_id } if rule_id.trim().is_empty() => return Err("Choose the rule a nested step runs".into()),
                _ => {}
            }
        }
        let mut seen = HashSet::new();
        for e in &self.edges {
            if !ids.contains(e.from.as_str()) || !ids.contains(e.to.as_str()) {
                return Err(format!("A connection refers to a missing step ({} → {})", e.from, e.to));
            }
            if e.from == e.to {
                return Err(format!("Step {} connects to itself", e.from));
            }
            if !seen.insert((e.from.as_str(), e.to.as_str(), e.on)) {
                return Err(format!("Duplicate connection {} → {}", e.from, e.to));
            }
            let from_condition = self.nodes.iter().any(|n| n.id == e.from && matches!(n.kind, NodeKind::Condition { .. }));
            if from_condition != matches!(e.on, EdgeOn::True | EdgeOn::False) {
                return Err(format!("Connections from a condition use true/false; others success/failure/always ({} → {})", e.from, e.to));
            }
        }
        self.order().map(|_| ())
    }

    /// Ids of every step upstream of `id`: the steps it receives files or
    /// results from, directly or through other steps.
    pub fn ancestors(&self, id: &str) -> HashSet<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            for e in self.edges.iter().filter(|e| e.to == cur) {
                if seen.insert(e.from.clone()) {
                    stack.push(e.from.clone());
                }
            }
        }
        seen
    }

    /// Topological order (Kahn), or an error naming a cycle member.
    fn order(&self) -> Result<Vec<String>, String> {
        let mut indeg: HashMap<&str, usize> = self.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
        for e in &self.edges {
            *indeg.entry(e.to.as_str()).or_default() += 1;
        }
        let mut q: VecDeque<&str> = self.nodes.iter().filter(|n| indeg[n.id.as_str()] == 0).map(|n| n.id.as_str()).collect();
        let mut out = Vec::new();
        while let Some(id) = q.pop_front() {
            out.push(id.to_string());
            for e in self.edges.iter().filter(|e| e.from == id) {
                let d = indeg.get_mut(e.to.as_str()).expect("validated");
                *d -= 1;
                if *d == 0 {
                    q.push_back(e.to.as_str());
                }
            }
        }
        if out.len() != self.nodes.len() {
            let stuck = self.nodes.iter().find(|n| !out.contains(&n.id)).map(|n| n.id.clone()).unwrap_or_default();
            return Err(format!("The workflow has a loop (through step {stuck})"));
        }
        Ok(out)
    }
}

/// Files an edge carries, if it fired.
fn fired(state: &NodeState, on: EdgeOn) -> Option<Vec<String>> {
    match state {
        NodeState::Skipped => None,
        NodeState::Done { ok, out, yes, no } => match on {
            EdgeOn::Always => Some(out.clone()),
            EdgeOn::Success => ok.then(|| out.clone()),
            EdgeOn::Failure => (!ok).then(|| out.clone()),
            // A condition with no files at all takes the false branch.
            EdgeOn::True => (!yes.is_empty()).then(|| yes.clone()),
            EdgeOn::False => (!no.is_empty() || (yes.is_empty() && no.is_empty())).then(|| no.clone()),
        },
    }
}

/// Run `wf` on `start` (the triggering or selected file, if any). Returns
/// node results (with node ids), the files affected, and the first error.
pub async fn run(wf: &Workflow, rule: &AutomationRule, start: Option<String>, env: &ExecEnv<'_>) -> (Vec<StepResult>, Vec<String>, Option<String>) {
    let mut results: Vec<StepResult> = Vec::new();
    let mut affected: Vec<String> = Vec::new();
    if let Err(e) = wf.validate() {
        return (results, affected, Some(e));
    }
    let index: HashMap<&str, usize> = wf.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut state: HashMap<String, NodeState> = HashMap::new();
    let mut started: HashSet<String> = HashSet::new();
    let mut first_error: Option<String> = None;
    let mut running = FuturesUnordered::new();
    let seed: Vec<String> = start.into_iter().collect();

    loop {
        // Start every node whose inputs have all finished.
        for n in &wf.nodes {
            if started.contains(&n.id) {
                continue;
            }
            let incoming: Vec<&WorkflowEdge> = wf.edges.iter().filter(|e| e.to == n.id).collect();
            if !incoming.iter().all(|e| state.contains_key(&e.from)) {
                continue;
            }
            started.insert(n.id.clone());
            let inputs: Vec<Option<Vec<String>>> = incoming.iter().map(|e| fired(&state[&e.from], e.on)).collect();
            let need_all = matches!(n.kind, NodeKind::Join { all: true });
            let runs = if incoming.is_empty() { true } else if need_all { inputs.iter().all(Option::is_some) } else { inputs.iter().any(Option::is_some) };
            if !runs {
                state.insert(n.id.clone(), NodeState::Skipped);
                results.push(StepResult { step: index[n.id.as_str()], action: n.kind.label(), file: None, ok: true, attempts: 0, message: Some("skipped".into()), node: Some(n.id.clone()) });
                continue;
            }
            let files: Vec<String> = if incoming.is_empty() {
                seed.clone()
            } else {
                inputs.into_iter().flatten().flatten().collect::<BTreeSet<_>>().into_iter().collect()
            };
            let id = n.id.clone();
            running.push(async move { (id.clone(), run_node(n, &files, rule, env).await) });
        }
        if running.is_empty() {
            if state.len() == wf.nodes.len() {
                break;
            }
            // Newly skipped nodes may have unblocked others: look again.
            continue;
        }
        let Some((id, (st, mut node_results))) = running.next().await else { break };
        for r in &mut node_results {
            r.step = index[id.as_str()];
            r.node = Some(id.clone());
            if !r.ok {
                let label = r.file.as_deref().map(|f| format!("{f}: ")).unwrap_or_default();
                first_error.get_or_insert(format!("{label}{}", r.message.clone().unwrap_or_default()));
            }
        }
        if let NodeState::Done { out, .. } = &st {
            if matches!(wf.nodes[index[id.as_str()]].kind, NodeKind::Action { .. } | NodeKind::SubRule { .. }) {
                affected.extend(out.iter().cloned());
            }
        }
        results.extend(node_results);
        state.insert(id, st);
    }
    affected.dedup();
    (results, affected, first_error)
}

async fn run_node(n: &WorkflowNode, files: &[String], rule: &AutomationRule, env: &ExecEnv<'_>) -> (NodeState, Vec<StepResult>) {
    let label = n.kind.label();
    let res = |file: Option<String>, ok: bool, attempts: u32, message: Option<String>| StepResult { step: 0, action: label.clone(), file, ok, attempts, message, node: None };
    match &n.kind {
        NodeKind::Delay { seconds } => {
            tokio::time::sleep(std::time::Duration::from_secs((*seconds).min(MAX_DELAY_SECS))).await;
            (NodeState::Done { ok: true, out: files.to_vec(), yes: vec![], no: vec![] }, vec![res(None, true, 1, Some(format!("waited {seconds}s")))])
        }
        NodeKind::Join { .. } => (NodeState::Done { ok: true, out: files.to_vec(), yes: vec![], no: vec![] }, vec![res(None, true, 1, Some(format!("{} file(s)", files.len())))]),
        NodeKind::Condition { condition } => {
            let (yes, no): (Vec<String>, Vec<String>) = files.iter().cloned().partition(|f| super::evaluator::evaluate_tree(condition, f));
            let msg = format!("{} match, {} do not", yes.len(), no.len());
            (NodeState::Done { ok: true, out: files.to_vec(), yes, no }, vec![res(None, true, 1, Some(msg))])
        }
        NodeKind::Action { action } => {
            let targets: Vec<Option<String>> = if files.is_empty() { vec![None] } else { files.iter().cloned().map(Some).collect() };
            let (mut ok, mut out, mut results, mut produced) = (true, Vec::new(), Vec::new(), Vec::new());
            for file in targets {
                let retried = ufop_core::retry::retry_async(
                    &rule.options.retry,
                    |_| dispatch_step(action, file.as_deref(), env),
                    |e: &String| is_transient(action, e),
                    |attempt, e, d| tracing::warn!("Workflow '{}' step {} failed (attempt {}), retrying in {d:?}: {e}", rule.name, n.id, attempt + 1),
                )
                .await;
                match retried.result {
                    Ok(step) => {
                        if hands_on_path(action) && !step.paths.is_empty() {
                            out.extend(step.paths.iter().cloned());
                        } else if let Some(f) = &file {
                            out.push(f.clone());
                        }
                        produced.push(step.into_output());
                        results.push(res(file, true, retried.attempts, None));
                    }
                    Err(e) => {
                        ok = false;
                        if let Some(f) = &file {
                            out.push(f.clone());
                        }
                        results.push(res(file, false, retried.attempts, Some(e)));
                    }
                }
            }
            // The step's result for the steps downstream of it.
            if let Some(name) = n.output_as.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                env.outputs.set(name, StepOutput::merge(produced));
            }
            (NodeState::Done { ok, out, yes: vec![], no: vec![] }, results)
        }
        NodeKind::SubRule { rule_id } => {
            let fail = |m: String| (NodeState::Done { ok: false, out: files.to_vec(), yes: vec![], no: vec![] }, vec![res(None, false, 1, Some(m))]);
            if env.call_stack.len() >= MAX_NESTING {
                return fail(format!("Nested rules are limited to {MAX_NESTING} levels"));
            }
            if env.call_stack.iter().any(|r| r == rule_id) || *rule_id == rule.id {
                return fail(format!("Rule {rule_id} would run itself"));
            }
            let sub = match env.services {
                Some(s) => s.rule(rule_id).await,
                None => None,
            };
            let Some(sub) = sub else {
                return fail(format!("Rule {rule_id} not found"));
            };
            let mut child = env.child(&sub);
            child.call_stack.push(rule.id.clone());
            let targets: Vec<Option<String>> = if files.is_empty() { vec![None] } else { files.iter().cloned().map(Some).collect() };
            let (mut ok, mut out, mut results) = (true, Vec::new(), Vec::new());
            for file in targets {
                let o = Box::pin(super::executor::run_actions(&sub, file.clone(), &child)).await;
                let sub_ok = o.error.is_none();
                ok &= sub_ok;
                // The nested rule hands on what it produced, else the input.
                out.extend(if o.affected.is_empty() { file.clone().into_iter().collect() } else { o.affected });
                results.push(res(file, sub_ok, o.attempts, Some(o.error.unwrap_or_else(|| format!("ran '{}'", sub.name)))));
            }
            out.dedup();
            if let Some(name) = n.output_as.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                env.outputs.set(name, StepOutput::from_paths(&out));
            }
            (NodeState::Done { ok, out, yes: vec![], no: vec![] }, results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_engine::{RuleCondition, RuleTrigger};

    fn node(id: &str, kind: NodeKind) -> WorkflowNode {
        WorkflowNode { id: id.into(), kind, label: String::new(), output_as: None, x: 0.0, y: 0.0 }
    }
    fn edge(from: &str, to: &str, on: EdgeOn) -> WorkflowEdge {
        WorkflowEdge { from: from.into(), to: to.into(), on }
    }

    #[test]
    fn validation_and_conversion() {
        let a = |p: &str| NodeKind::Action { action: Box::new(RuleAction::CreateFolder { path: p.into() }) };
        let ok = Workflow { nodes: vec![node("a", a("/x")), node("b", a("/y"))], edges: vec![edge("a", "b", EdgeOn::Success)] };
        assert!(ok.validate().is_ok());
        let cyc = Workflow { nodes: ok.nodes.clone(), edges: vec![edge("a", "b", EdgeOn::Success), edge("b", "a", EdgeOn::Always)] };
        assert!(cyc.validate().unwrap_err().contains("loop"));
        let dup = Workflow { nodes: vec![node("a", a("/x")), node("a", a("/y"))], edges: vec![] };
        assert!(dup.validate().is_err());
        let dangling = Workflow { nodes: ok.nodes.clone(), edges: vec![edge("a", "zz", EdgeOn::Success)] };
        assert!(dangling.validate().unwrap_err().contains("missing"));
        let cond = Workflow { nodes: vec![node("c", NodeKind::Condition { condition: ConditionNode::All { nodes: vec![] } }), node("b", a("/y"))], edges: vec![edge("c", "b", EdgeOn::Success)] };
        assert!(cond.validate().unwrap_err().contains("true/false"));
        assert!(Workflow::default().validate().is_err());
        assert!(Workflow { nodes: vec![node("d", NodeKind::Delay { seconds: MAX_DELAY_SECS + 1 })], edges: vec![] }.validate().is_err());

        let mut rule = AutomationRule::new("r".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/1".into() });
        rule.steps = vec![
            crate::automation_engine::ActionStep { action: RuleAction::CreateFolder { path: "/2".into() }, continue_on_error: true, output_as: None },
            crate::automation_engine::ActionStep { action: RuleAction::CreateFolder { path: "/3".into() }, continue_on_error: false, output_as: None },
        ];
        let wf = Workflow::from_rule(&rule);
        assert_eq!(wf.nodes.len(), 3);
        assert_eq!(wf.edges, vec![edge("n1", "n2", EdgeOn::Success), edge("n2", "n3", EdgeOn::Always)]);
        assert!(wf.validate().is_ok());
        // JSON shape the editor uses.
        let j = serde_json::to_value(&wf.nodes[0]).unwrap();
        assert_eq!(j["kind"], "action");
        assert_eq!(j["action"]["type"], "create_folder");
        let _ = RuleCondition::ExtensionIs { extensions: vec![] };
    }

    use crate::automation_engine::executor::execute_rule_action_with;
    use crate::automation_engine::services::mock::RecordingServices;
    use crate::automation_engine::AutomationStatus;

    fn rule_with(wf: Workflow) -> AutomationRule {
        let mut r = AutomationRule::new("wf".into(), RuleTrigger::Manual, RuleAction::Notify { title: "x".into(), body: "y".into() });
        r.workflow = Some(wf);
        r
    }
    fn act(a: RuleAction) -> NodeKind {
        NodeKind::Action { action: Box::new(a) }
    }
    fn is_ext(e: &str) -> NodeKind {
        NodeKind::Condition { condition: ConditionNode::Condition { condition: RuleCondition::ExtensionIs { extensions: vec![e.into()] } } }
    }
    fn status_of(log: &crate::automation_engine::AutomationLog, node: &str) -> Vec<(bool, Option<String>)> {
        log.steps.iter().filter(|s| s.node.as_deref() == Some(node)).map(|s| (s.ok, s.message.clone())).collect()
    }

    #[tokio::test]
    async fn branches_join_skip_and_hand_files_on() {
        let t = tempfile::tempdir().unwrap();
        let p = |s: &str| t.path().join(s).to_string_lossy().into_owned();
        for d in ["in", "txt", "other"] {
            std::fs::create_dir_all(t.path().join(d)).unwrap();
        }
        std::fs::write(t.path().join("in/a.txt"), "a").unwrap();
        std::fs::write(t.path().join("in/b.pdf"), "b").unwrap();
        let wf = Workflow {
            nodes: vec![
                node("is_txt", is_ext("txt")),
                node("move_txt", act(RuleAction::MoveFile { dest_path: p("txt") })),
                node("copy_other", act(RuleAction::CopyFile { dest_path: p("other") })),
                node("join", NodeKind::Join { all: false }),
                node("tell", act(RuleAction::Notify { title: "done".into(), body: "{filename}".into() })),
            ],
            edges: vec![
                edge("is_txt", "move_txt", EdgeOn::True),
                edge("is_txt", "copy_other", EdgeOn::False),
                edge("move_txt", "join", EdgeOn::Success),
                edge("copy_other", "join", EdgeOn::Success),
                edge("join", "tell", EdgeOn::Success),
            ],
        };
        let svc = RecordingServices::default();
        let rule = rule_with(wf);
        let log = execute_rule_action_with(&rule, Some(&p("in/a.txt")), false, Some(&svc)).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log);
        assert!(t.path().join("txt/a.txt").exists() && !t.path().join("in/a.txt").exists());
        assert_eq!(status_of(&log, "copy_other"), vec![(true, Some("skipped".into()))]);
        // The moved path is what reaches the notification.
        assert_eq!(svc.calls(), vec!["notify:done:a.txt".to_string()]);
        let log = execute_rule_action_with(&rule, Some(&p("in/b.pdf")), false, Some(&svc)).await;
        assert_eq!(log.status, AutomationStatus::Success);
        assert!(t.path().join("other/b.pdf").exists() && t.path().join("in/b.pdf").exists());
        assert_eq!(status_of(&log, "move_txt"), vec![(true, Some("skipped".into()))]);
        assert_eq!(svc.calls().len(), 2);
        // Every node shows up once in the run graph.
        let nodes: BTreeSet<_> = log.steps.iter().filter_map(|s| s.node.clone()).collect();
        assert_eq!(nodes.len(), 5);
    }

    #[tokio::test]
    async fn failure_edges_join_all_delay_and_parallel_aggregation() {
        let t = tempfile::tempdir().unwrap();
        let p = |s: &str| t.path().join(s).to_string_lossy().into_owned();
        std::fs::create_dir_all(t.path().join("a")).unwrap();
        std::fs::create_dir_all(t.path().join("b")).unwrap();
        std::fs::write(t.path().join("f.txt"), "f").unwrap();
        let wf = Workflow {
            nodes: vec![
                node("to_a", act(RuleAction::CopyFile { dest_path: p("a") })),
                node("to_b", act(RuleAction::CopyFile { dest_path: p("b") })),
                node("broken", act(RuleAction::CopyFile { dest_path: "/definitely/../not/allowed".into() })),
                node("wait", NodeKind::Delay { seconds: 1 }),
                node("both", NodeKind::Join { all: true }),
                node("all_three", NodeKind::Join { all: true }),
                node("on_error", act(RuleAction::Notify { title: "failed".into(), body: "{filename}".into() })),
                node("summary", act(RuleAction::Notify { title: "both".into(), body: "{filename}".into() })),
            ],
            edges: vec![
                edge("to_a", "wait", EdgeOn::Success),
                edge("wait", "both", EdgeOn::Success),
                edge("to_b", "both", EdgeOn::Success),
                edge("both", "summary", EdgeOn::Success),
                edge("broken", "on_error", EdgeOn::Failure),
                edge("to_a", "all_three", EdgeOn::Success),
                edge("broken", "all_three", EdgeOn::Success),
            ],
        };
        let svc = RecordingServices::default();
        let started = std::time::Instant::now();
        let log = execute_rule_action_with(&rule_with(wf), Some(&p("f.txt")), false, Some(&svc)).await;
        assert!(started.elapsed() >= std::time::Duration::from_secs(1), "the delay ran");
        assert_eq!(log.status, AutomationStatus::Failed, "one step failed");
        assert!(log.error_message.as_deref().unwrap_or("").contains("traversal"), "{:?}", log.error_message);
        assert!(t.path().join("a/f.txt").exists() && t.path().join("b/f.txt").exists());
        let mut calls = svc.calls();
        calls.sort();
        // The failure edge fired; the join of a failed branch was skipped; the
        // two copies' join ran once on the original file (deduplicated).
        assert_eq!(calls, vec!["notify:both:f.txt".to_string(), "notify:failed:f.txt".to_string()]);
        assert_eq!(status_of(&log, "all_three"), vec![(true, Some("skipped".into()))]);
    }

    #[tokio::test]
    async fn nested_rules_run_and_loops_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let svc = RecordingServices::default();
        let mut child = AutomationRule::new("child".into(), RuleTrigger::Manual, RuleAction::Notify { title: "child".into(), body: "{rule}".into() });
        child.id = "child".into();
        let mut a = rule_with(Workflow { nodes: vec![node("sub", NodeKind::SubRule { rule_id: "child".into() })], edges: vec![] });
        a.id = "a".into();
        svc.rules.lock().unwrap().push(child.clone());
        let log = execute_rule_action_with(&a, None, false, Some(&svc)).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log);
        assert_eq!(svc.calls(), vec!["notify:child:child".to_string()]);

        // child → a → child … is refused instead of recursing.
        child.workflow = Some(Workflow { nodes: vec![node("back", NodeKind::SubRule { rule_id: "a".into() })], edges: vec![] });
        svc.rules.lock().unwrap().clear();
        svc.rules.lock().unwrap().extend([child, a.clone()]);
        let log = execute_rule_action_with(&a, None, false, Some(&svc)).await;
        assert_eq!(log.status, AutomationStatus::Failed);
        assert!(log.error_message.unwrap().contains("would run itself"));
        let missing = rule_with(Workflow { nodes: vec![node("sub", NodeKind::SubRule { rule_id: "nope".into() })], edges: vec![] });
        assert!(execute_rule_action_with(&missing, None, false, Some(&svc)).await.error_message.unwrap().contains("not found"));
        drop(t);
    }

    #[tokio::test]
    async fn commands_in_workflows_need_approval_and_invalid_graphs_fail() {
        let cmd = RuleAction::RunCommand { program: "echo".into(), args: vec!["hi".into()], working_dir: None, timeout_secs: 5 };
        let mut rule = rule_with(Workflow { nodes: vec![node("c", act(cmd.clone()))], edges: vec![] });
        assert!(crate::automation_engine::command_approvals(&rule).is_some(), "workflow commands are listed for approval");
        let log = execute_rule_action_with(&rule, None, false, None).await;
        assert!(log.error_message.unwrap().contains("not approved"));
        rule.options.command_approval = crate::automation_engine::command_approvals(&rule);
        let log = execute_rule_action_with(&rule, None, false, None).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log);
        rule.workflow = Some(Workflow { nodes: vec![node("x", act(cmd.clone())), node("x", act(cmd))], edges: vec![] });
        assert!(execute_rule_action_with(&rule, None, false, None).await.error_message.unwrap().contains("unique"));
    }
}
