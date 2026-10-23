//! Condition evaluation for automation rules.
//!
//! Rules carry either a flat list of conditions (all must match) or an
//! AND/OR/NOT tree. A `NameRegex` condition that matches also provides the
//! capture groups the rule's templates use as `{1}`…`{9}`.

use super::{AutomationRule, ConditionNode, RuleCondition};
use glob::Pattern;
use std::path::Path;
use std::time::SystemTime;

/// Evaluate all conditions against a file path. Returns true if ALL match (AND).
/// If conditions is empty, always returns true.
pub fn evaluate_conditions(conditions: &[RuleCondition], file_path: &str) -> bool {
    if conditions.is_empty() {
        return true;
    }

    let path = Path::new(file_path);
    let metadata = std::fs::metadata(file_path).ok();

    for condition in conditions {
        if !evaluate_single(condition, path, &metadata) {
            return false;
        }
    }

    true
}

/// Result of matching a rule against a file.
#[derive(Debug, Default, PartialEq)]
pub struct RuleMatch {
    pub matched: bool,
    /// Captures of the first matching `NameRegex` (index 0 = whole match).
    pub captures: Vec<String>,
}

/// Match a rule's conditions (tree if present, else the flat list).
pub fn evaluate_rule(rule: &AutomationRule, file_path: &str) -> RuleMatch {
    let path = Path::new(file_path);
    let metadata = std::fs::metadata(file_path).ok();
    let mut captures = Vec::new();
    let matched = match &rule.condition_tree {
        Some(tree) => eval_node(tree, path, &metadata, &mut captures),
        None => rule.conditions.iter().all(|c| eval_leaf(c, path, &metadata, &mut captures)),
    };
    RuleMatch { matched, captures: if matched { captures } else { Vec::new() } }
}

/// Whether `file_path` satisfies a condition tree (workflow condition steps).
pub fn evaluate_tree(tree: &ConditionNode, file_path: &str) -> bool {
    let path = Path::new(file_path);
    let metadata = std::fs::metadata(file_path).ok();
    eval_node(tree, path, &metadata, &mut Vec::new())
}

fn eval_node(node: &ConditionNode, path: &Path, md: &Option<std::fs::Metadata>, caps: &mut Vec<String>) -> bool {
    match node {
        ConditionNode::All { nodes } => nodes.iter().all(|n| eval_node(n, path, md, caps)),
        ConditionNode::Any { nodes } => nodes.is_empty() || nodes.iter().any(|n| eval_node(n, path, md, caps)),
        ConditionNode::Not { node } => {
            // Captures from inside a negation never apply.
            let mut ignored = Vec::new();
            !eval_node(node, path, md, &mut ignored)
        }
        ConditionNode::Condition { condition } => eval_leaf(condition, path, md, caps),
    }
}

fn eval_leaf(condition: &RuleCondition, path: &Path, md: &Option<std::fs::Metadata>, caps: &mut Vec<String>) -> bool {
    if let RuleCondition::NameRegex { pattern } = condition {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        return match regex::Regex::new(pattern).ok().and_then(|re| {
            re.captures(name).map(|c| c.iter().map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default()).collect::<Vec<_>>())
        }) {
            Some(c) => {
                if caps.is_empty() {
                    *caps = c;
                }
                true
            }
            None => false,
        };
    }
    evaluate_single(condition, path, md)
}

/// Seconds since the file was last modified, if known.
fn age_secs(metadata: &Option<std::fs::Metadata>) -> Option<u64> {
    metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs())
}

fn evaluate_single(
    condition: &RuleCondition,
    path: &Path,
    metadata: &Option<std::fs::Metadata>,
) -> bool {
    match condition {
        RuleCondition::OlderThan { seconds } => age_secs(metadata).is_some_and(|a| a > *seconds),
        RuleCondition::ModifiedWithin { seconds } => age_secs(metadata).is_some_and(|a| a <= *seconds),
        RuleCondition::NameRegex { pattern } => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            regex::Regex::new(pattern).map(|re| re.is_match(name)).unwrap_or(false)
        }
        RuleCondition::ExtensionIs { extensions } => {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            extensions
                .iter()
                .any(|e| e.to_lowercase().trim_start_matches('.') == ext)
        }

        RuleCondition::NameMatches { pattern } => {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            Pattern::new(pattern)
                .map(|p| p.matches(name))
                .unwrap_or(false)
        }

        RuleCondition::SizeGreaterThan { bytes } => metadata
            .as_ref()
            .map(|m| m.len() > *bytes)
            .unwrap_or(false),

        RuleCondition::SizeLessThan { bytes } => metadata
            .as_ref()
            .map(|m| m.len() < *bytes)
            .unwrap_or(false),

        RuleCondition::CreatedWithin { seconds } => {
            metadata
                .as_ref()
                .and_then(|m| m.created().ok())
                .map(|created| {
                    SystemTime::now()
                        .duration_since(created)
                        .map(|d| d.as_secs() <= *seconds)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }

        RuleCondition::PathContains { substring } => {
            path.to_str()
                .map(|p| p.contains(substring.as_str()))
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_engine::{RuleAction, RuleTrigger};

    fn rule_with_tree(tree: ConditionNode) -> AutomationRule {
        let mut r = AutomationRule::new("t".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/x".into() });
        r.condition_tree = Some(tree);
        r
    }

    fn leaf(c: RuleCondition) -> ConditionNode {
        ConditionNode::Condition { condition: c }
    }

    #[test]
    fn condition_tree_any_all_not_and_captures() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("INV-2026-07.pdf");
        std::fs::write(&f, b"x").unwrap();
        let p = f.to_str().unwrap();

        let tree = ConditionNode::All {
            nodes: vec![
                ConditionNode::Any {
                    nodes: vec![
                        leaf(RuleCondition::ExtensionIs { extensions: vec!["docx".into()] }),
                        leaf(RuleCondition::ExtensionIs { extensions: vec!["pdf".into()] }),
                    ],
                },
                ConditionNode::Not { node: Box::new(leaf(RuleCondition::NameMatches { pattern: "*draft*".into() })) },
                leaf(RuleCondition::NameRegex { pattern: r"^INV-(\d{4})-(\d{2})".into() }),
            ],
        };
        let m = evaluate_rule(&rule_with_tree(tree.clone()), p);
        assert!(m.matched);
        assert_eq!(m.captures, vec!["INV-2026-07".to_string(), "2026".into(), "07".into()]);

        let draft = dir.path().join("INV-2026-07-draft.pdf");
        std::fs::write(&draft, b"x").unwrap();
        assert!(!evaluate_rule(&rule_with_tree(tree), draft.to_str().unwrap()).matched);
    }

    #[test]
    fn age_conditions() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("fresh.log");
        std::fs::write(&f, b"x").unwrap();
        let p = f.to_str().unwrap();
        assert!(evaluate_conditions(&[RuleCondition::ModifiedWithin { seconds: 60 }], p));
        assert!(!evaluate_conditions(&[RuleCondition::OlderThan { seconds: 60 }], p));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86_400);
        std::fs::File::options().write(true).open(&f).unwrap().set_modified(old).unwrap();
        assert!(evaluate_conditions(&[RuleCondition::OlderThan { seconds: 2 * 86_400 }], p));
        assert!(!evaluate_conditions(&[RuleCondition::OlderThan { seconds: 2 * 86_400 }], "/no/such/file"), "missing files never match");
    }

    #[test]
    fn flat_list_still_works_through_evaluate_rule() {
        let mut r = AutomationRule::new("t".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/x".into() });
        r.conditions = vec![RuleCondition::PathContains { substring: "reports".into() }];
        assert!(evaluate_rule(&r, "/data/reports/a.csv").matched);
        assert!(!evaluate_rule(&r, "/data/other/a.csv").matched);
        r.conditions.clear();
        assert!(evaluate_rule(&r, "/anything").matched);
    }
    use std::io::Write;

    #[test]
    fn test_empty_conditions_always_matches() {
        assert!(evaluate_conditions(&[], "/any/path.txt"));
    }

    #[test]
    fn test_extension_match() {
        let conditions = vec![RuleCondition::ExtensionIs {
            extensions: vec!["pdf".to_string(), "doc".to_string()],
        }];
        assert!(evaluate_conditions(&conditions, "/test/report.pdf"));
        assert!(evaluate_conditions(&conditions, "/test/report.PDF")); // case insensitive
        assert!(!evaluate_conditions(&conditions, "/test/report.txt"));
    }

    #[test]
    fn test_extension_with_dot_prefix() {
        let conditions = vec![RuleCondition::ExtensionIs {
            extensions: vec![".pdf".to_string()],
        }];
        assert!(evaluate_conditions(&conditions, "/test/report.pdf"));
    }

    #[test]
    fn test_name_glob_pattern() {
        let conditions = vec![RuleCondition::NameMatches {
            pattern: "report_*".to_string(),
        }];
        assert!(evaluate_conditions(&conditions, "/test/report_2024.pdf"));
        assert!(!evaluate_conditions(&conditions, "/test/invoice_2024.pdf"));
    }

    #[test]
    fn test_path_contains() {
        let conditions = vec![RuleCondition::PathContains {
            substring: "Downloads".to_string(),
        }];
        assert!(evaluate_conditions(&conditions, "/Users/me/Downloads/file.pdf"));
        assert!(!evaluate_conditions(&conditions, "/Users/me/Documents/file.pdf"));
    }

    #[test]
    fn test_size_conditions_with_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(&[0u8; 500]).unwrap(); // 500 bytes

        let path_str = file_path.to_str().unwrap();

        assert!(evaluate_conditions(
            &[RuleCondition::SizeGreaterThan { bytes: 100 }],
            path_str,
        ));
        assert!(!evaluate_conditions(
            &[RuleCondition::SizeGreaterThan { bytes: 1000 }],
            path_str,
        ));
        assert!(evaluate_conditions(
            &[RuleCondition::SizeLessThan { bytes: 1000 }],
            path_str,
        ));
    }

    #[test]
    fn test_and_logic_multiple_conditions() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("report.pdf");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(&[0u8; 500]).unwrap();

        let path_str = file_path.to_str().unwrap();

        // Both conditions must match
        let conditions = vec![
            RuleCondition::ExtensionIs {
                extensions: vec!["pdf".to_string()],
            },
            RuleCondition::SizeGreaterThan { bytes: 100 },
        ];
        assert!(evaluate_conditions(&conditions, path_str));

        // One fails -> whole thing fails
        let conditions_fail = vec![
            RuleCondition::ExtensionIs {
                extensions: vec!["txt".to_string()],
            },
            RuleCondition::SizeGreaterThan { bytes: 100 },
        ];
        assert!(!evaluate_conditions(&conditions_fail, path_str));
    }
}
