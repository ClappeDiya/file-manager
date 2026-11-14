//! Named step outputs: a step declares `output_as: "name"`, later steps use
//! `{steps.name.files}`, `{steps.name.count}`, `{steps.name.bytes}` and
//! `{steps.name.path}` in any template field.
//!
//! The values are served through the same token engine as every other
//! `{token}` (`ufop_core::naming`): they enter a step's template context as
//! extra values named `steps.<name>.<field>`, so filters (`|upper`) and the
//! unknown-token check apply unchanged. [`validate_rule`] rejects a rule
//! whose references cannot resolve before it ever runs: a name no earlier
//! step (or, in a graph, no upstream step) declares, or an unknown field.

use super::workflow::{NodeKind, Workflow};
use super::{AutomationRule, RuleAction};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

/// The fields a step output has.
pub const FIELDS: [&str; 4] = ["files", "count", "bytes", "path"];

/// Longest output name.
pub const MAX_NAME: usize = 40;

/// What a finished step produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepOutput {
    /// The files it produced or worked on.
    pub files: Vec<String>,
    pub count: u64,
    /// Total size of those files that exist.
    pub bytes: u64,
    /// The first of them.
    pub path: String,
}

impl StepOutput {
    /// The output of a step that produced `paths` (sizes read from disk).
    pub fn from_paths(paths: &[String]) -> Self {
        let bytes = paths.iter().filter_map(|p| std::fs::metadata(Path::new(p)).ok()).filter(|m| m.is_file()).map(|m| m.len()).sum();
        Self { files: paths.to_vec(), count: paths.len() as u64, bytes, path: paths.first().cloned().unwrap_or_default() }
    }

    /// Several outputs of one step (a graph node runs once per input file).
    pub fn merge(parts: Vec<StepOutput>) -> Self {
        let mut out = StepOutput::default();
        for p in parts {
            out.files.extend(p.files);
            out.count += p.count;
            out.bytes += p.bytes;
            if out.path.is_empty() {
                out.path = p.path;
            }
        }
        out
    }

    /// One field's text: `files` is one path per line.
    pub fn field(&self, name: &str) -> Option<String> {
        Some(match name {
            "files" => self.files.join("\n"),
            "count" => self.count.to_string(),
            "bytes" => self.bytes.to_string(),
            "path" => self.path.clone(),
            _ => return None,
        })
    }
}

/// Outputs of the steps of one run, by name.
#[derive(Debug, Default)]
pub struct Outputs(Mutex<HashMap<String, StepOutput>>);

impl Outputs {
    pub fn set(&self, name: &str, out: StepOutput) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), out);
    }

    /// Forget everything (each run of a rule on a file starts empty).
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Template values `steps.<name>.<field>` for every output so far.
    pub fn template_values(&self) -> HashMap<String, String> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().flat_map(|(name, o)| FIELDS.iter().filter_map(move |f| o.field(f).map(|v| (format!("steps.{name}.{f}"), v)))).collect()
    }
}

/// Whether `name` can name an output: a letter or `_`, then letters,
/// digits, `_` or `-`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_NAME && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Every string anywhere in an action's configuration.
fn strings_of(action: &RuleAction) -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::to_value(action) {
        walk(&v, &mut out);
    }
    out
}

/// `steps.<name>.<field>` references in an action's templates, as
/// (name, field) with an empty field for a reference without one.
pub fn references(action: &RuleAction) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for s in strings_of(action) {
        for token in ufop_core::naming::tokens::tokens_in(&s) {
            if let Some(rest) = token.strip_prefix("steps.") {
                let (name, field) = rest.split_once('.').unwrap_or((rest, ""));
                let r = (name.to_string(), field.to_string());
                if !found.contains(&r) {
                    found.push(r);
                }
            }
        }
    }
    found
}

/// Check one step's references against the names its predecessors declare.
fn check_refs(label: &str, action: &RuleAction, visible: &HashSet<&str>) -> Result<(), String> {
    for (name, field) in references(action) {
        if !visible.contains(name.as_str()) {
            return Err(format!("{label} uses {{steps.{name}.{}}}, but no earlier step is named \"{name}\" (name a step in its \"Save result as\" box)", if field.is_empty() { "…" } else { &field }));
        }
        if !FIELDS.contains(&field.as_str()) {
            return Err(format!("{label} uses {{steps.{name}.{field}}}: a step result has the fields {}", FIELDS.join(", ")));
        }
    }
    Ok(())
}

fn check_name(label: &str, name: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("{label}: \"{name}\" is not a valid result name (letters, digits, _ and -, starting with a letter)"));
    }
    if !seen.insert(name.to_string()) {
        return Err(format!("{label}: the result name \"{name}\" is used by another step"));
    }
    Ok(())
}

/// Reject a rule whose step results cannot resolve. Linear rules: a step
/// may use results of earlier steps. Graphs: of steps upstream of it along
/// the connections.
pub fn validate_rule(rule: &AutomationRule) -> Result<(), String> {
    let mut seen = HashSet::new();
    match &rule.workflow {
        None => {
            let steps: Vec<(&RuleAction, Option<&str>)> = std::iter::once((&rule.action, rule.output_as.as_deref()))
                .chain(rule.steps.iter().map(|s| (&s.action, s.output_as.as_deref())))
                .collect();
            let mut visible: HashSet<&str> = HashSet::new();
            for (i, (action, out)) in steps.iter().enumerate() {
                let label = format!("Action {}", i + 1);
                check_refs(&label, action, &visible)?;
                if let Some(n) = out.map(str::trim).filter(|n| !n.is_empty()) {
                    check_name(&label, n, &mut seen)?;
                    visible.insert(n);
                }
            }
            Ok(())
        }
        Some(wf) => validate_workflow(wf),
    }
}

fn validate_workflow(wf: &Workflow) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut names: HashMap<&str, &str> = HashMap::new(); // node id → output name
    for n in &wf.nodes {
        if let Some(name) = n.output_as.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
            if !matches!(n.kind, NodeKind::Action { .. } | NodeKind::SubRule { .. }) {
                return Err(format!("Step {}: only action steps can save a result", n.id));
            }
            check_name(&format!("Step {}", n.id), name, &mut seen)?;
            names.insert(n.id.as_str(), name);
        }
    }
    for n in &wf.nodes {
        let NodeKind::Action { action } = &n.kind else { continue };
        if references(action).is_empty() {
            continue;
        }
        let upstream = wf.ancestors(&n.id);
        let visible: HashSet<&str> = upstream.iter().filter_map(|id| names.get(id.as_str()).copied()).collect();
        check_refs(&format!("Step {}", n.id), action, &visible)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_engine::workflow::{EdgeOn, WorkflowEdge, WorkflowNode};
    use crate::automation_engine::{ActionStep, RuleTrigger};

    fn notify(body: &str) -> RuleAction {
        RuleAction::Notify { title: "t".into(), body: body.into() }
    }

    fn rule(first: RuleAction, first_out: Option<&str>, rest: Vec<(RuleAction, Option<&str>)>) -> AutomationRule {
        let mut r = AutomationRule::new("r".into(), RuleTrigger::Manual, first);
        r.output_as = first_out.map(Into::into);
        r.steps = rest.into_iter().map(|(action, o)| ActionStep { action, continue_on_error: false, output_as: o.map(Into::into) }).collect();
        r
    }

    #[test]
    fn names() {
        for ok in ["a", "dl", "step_1", "My-List", "_x"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "1a", "a b", "a.b", "a/b", "é", &"x".repeat(41)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn fields_and_template_values() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a.txt"), t.path().join("b.txt"));
        std::fs::write(&a, "12345").unwrap();
        std::fs::write(&b, "123").unwrap();
        let o = StepOutput::from_paths(&[a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()]);
        assert_eq!((o.count, o.bytes), (2, 8));
        assert_eq!(o.field("path").unwrap(), a.to_string_lossy());
        assert_eq!(o.field("files").unwrap().lines().count(), 2);
        assert!(o.field("nope").is_none());
        let store = Outputs::default();
        store.set("list", o);
        let v = store.template_values();
        assert_eq!(v["steps.list.count"], "2");
        assert_eq!(v["steps.list.bytes"], "8");
        assert_eq!(v.len(), 4);
        store.clear();
        assert!(store.template_values().is_empty());
        let merged = StepOutput::merge(vec![StepOutput { files: vec!["x".into()], count: 1, bytes: 2, path: "x".into() }, StepOutput { files: vec!["y".into()], count: 3, bytes: 4, path: "y".into() }]);
        assert_eq!((merged.count, merged.bytes, merged.path.as_str(), merged.files.len()), (4, 6, "x", 2));
    }

    #[test]
    fn references_are_found_in_any_field_and_through_filters() {
        let a = RuleAction::SendMessage {
            notification: crate::notify_hub::Notification { channel: crate::notify_hub::Channel::Slack {}, send_on: vec![], credential_id: String::new(), template: "{steps.l.count}".into(), cert: Default::default() },
            title: "{steps.l.path|upper}".into(),
            body: "{{steps.fake.count}} {steps.d.bytes}".into(),
        };
        let r = references(&a);
        assert!(r.contains(&("l".into(), "count".into())) && r.contains(&("l".into(), "path".into())) && r.contains(&("d".into(), "bytes".into())));
        assert!(!r.iter().any(|(n, _)| n == "fake"), "escaped braces are literal");
    }

    #[test]
    fn linear_rules_resolve_only_earlier_results() {
        let ok = rule(RuleAction::CreateFolder { path: "/x".into() }, Some("mk"), vec![(notify("{steps.mk.path} {steps.mk.count}"), None)]);
        assert!(validate_rule(&ok).is_ok());
        // A later step's result is not available yet.
        let early = rule(notify("{steps.later.path}"), None, vec![(RuleAction::CreateFolder { path: "/x".into() }, Some("later"))]);
        let e = validate_rule(&early).unwrap_err();
        assert!(e.contains("Action 1") && e.contains("\"later\""), "{e}");
        // Unknown name, unknown field, missing field.
        let unknown = rule(RuleAction::CreateFolder { path: "/x".into() }, Some("a"), vec![(notify("{steps.zz.path}"), None)]);
        assert!(validate_rule(&unknown).unwrap_err().contains("\"zz\""));
        let field = rule(RuleAction::CreateFolder { path: "/x".into() }, Some("a"), vec![(notify("{steps.a.size}"), None)]);
        assert!(validate_rule(&field).unwrap_err().contains("fields files, count, bytes, path"));
        let bare = rule(RuleAction::CreateFolder { path: "/x".into() }, Some("a"), vec![(notify("{steps.a}"), None)]);
        assert!(validate_rule(&bare).is_err());
        // Names must be valid and unique.
        let bad_name = rule(RuleAction::CreateFolder { path: "/x".into() }, Some("a b"), vec![]);
        assert!(validate_rule(&bad_name).unwrap_err().contains("not a valid result name"));
        let dup = rule(RuleAction::CreateFolder { path: "/x".into() }, Some("a"), vec![(RuleAction::CreateFolder { path: "/y".into() }, Some("a"))]);
        assert!(validate_rule(&dup).unwrap_err().contains("used by another step"));
        // A rule without any reference is untouched.
        assert!(validate_rule(&rule(notify("plain {name}"), None, vec![])).is_ok());
    }

    fn node(id: &str, action: RuleAction, out: Option<&str>) -> WorkflowNode {
        WorkflowNode { id: id.into(), kind: NodeKind::Action { action: Box::new(action) }, label: String::new(), x: 0.0, y: 0.0, output_as: out.map(Into::into) }
    }

    #[test]
    fn graph_rules_resolve_only_upstream_results() {
        let edge = |a: &str, b: &str| WorkflowEdge { from: a.into(), to: b.into(), on: EdgeOn::Success };
        let mk = |wf: Workflow| {
            let mut r = AutomationRule::new("g".into(), RuleTrigger::Manual, notify("x"));
            r.workflow = Some(wf);
            r
        };
        // list → (copy | tell): tell is downstream of list.
        let ok = mk(Workflow {
            nodes: vec![node("list", RuleAction::CreateFolder { path: "/l".into() }, Some("l")), node("a", notify("{steps.l.count}"), None), node("b", notify("{steps.l.path}"), None)],
            edges: vec![edge("list", "a"), edge("list", "b")],
        });
        assert!(validate_rule(&ok).is_ok());
        // Sibling branches cannot see each other's results (they run in parallel).
        let sibling = mk(Workflow {
            nodes: vec![node("root", RuleAction::CreateFolder { path: "/r".into() }, None), node("a", RuleAction::CreateFolder { path: "/a".into() }, Some("A")), node("b", notify("{steps.A.path}"), None)],
            edges: vec![edge("root", "a"), edge("root", "b")],
        });
        assert!(validate_rule(&sibling).unwrap_err().contains("Step b"));
        // Indirect ancestors count.
        let chain = mk(Workflow {
            nodes: vec![node("a", RuleAction::CreateFolder { path: "/a".into() }, Some("A")), node("m", RuleAction::CreateFolder { path: "/m".into() }, None), node("c", notify("{steps.A.bytes}"), None)],
            edges: vec![edge("a", "m"), edge("m", "c")],
        });
        assert!(validate_rule(&chain).is_ok());
        // A name used twice, or saved by a non-action node.
        let dup = mk(Workflow { nodes: vec![node("a", RuleAction::CreateFolder { path: "/a".into() }, Some("x")), node("b", RuleAction::CreateFolder { path: "/b".into() }, Some("x"))], edges: vec![] });
        assert!(validate_rule(&dup).is_err());
        let mut delay = node("d", notify("x"), Some("y"));
        delay.kind = NodeKind::Delay { seconds: 1 };
        assert!(validate_rule(&mk(Workflow { nodes: vec![delay], edges: vec![] })).unwrap_err().contains("only action steps"));
    }
}
