//! Filename policy: which names may be written where.
//!
//! A [`PolicySet`] holds rules scoped to folders. For a target path, every
//! rule whose scope contains it applies (parents first), unless a deeper
//! rule says `inherit: false`, which drops everything above it. A name is
//! allowed only if every applicable rule allows it. The same check runs for
//! the safety interlock (copy/move/rename), sync planning, automation and
//! fleet-pushed policies, so a rule is enforced the same way everywhere.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One rule, applying to `scope` and (unless `recursive` is false) below it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilenameRule {
    pub id: String,
    pub name: String,
    /// Folder the rule covers; empty means everywhere.
    pub scope: String,
    pub recursive: bool,
    /// When false, rules from parent scopes stop applying below `scope`.
    pub inherit: bool,
    /// If set, only these extensions are allowed (case-insensitive, no dot).
    pub allowed_extensions: Option<Vec<String>>,
    /// Extensions that are never allowed.
    pub blocked_extensions: Vec<String>,
    /// Names must match this regex (e.g. a naming convention).
    pub required_pattern: Option<String>,
    /// Names must not match this regex.
    pub forbidden_pattern: Option<String>,
    /// Maximum file-name length in bytes.
    pub max_name_bytes: Option<usize>,
    /// Shown to the user when the rule blocks a name.
    pub message: Option<String>,
    pub enabled: bool,
}

impl Default for FilenameRule {
    /// Enabled, recursive and inheriting: a rule saved with only an id and
    /// a constraint does what it says.
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            scope: String::new(),
            recursive: true,
            inherit: true,
            allowed_extensions: None,
            blocked_extensions: Vec::new(),
            required_pattern: None,
            forbidden_pattern: None,
            max_name_bytes: None,
            message: None,
            enabled: true,
        }
    }
}

impl FilenameRule {
    pub fn new(id: impl Into<String>, scope: impl Into<String>) -> Self {
        Self { id: id.into(), scope: scope.into(), ..Default::default() }
    }
}

/// Why a name was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub rule_id: String,
    pub rule_name: String,
    pub path: String,
    pub reason: String,
}

impl std::fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let who = if self.rule_name.is_empty() { &self.rule_id } else { &self.rule_name };
        write!(f, "{}: {} (policy '{who}')", self.path, self.reason)
    }
}

/// Invalid rule configuration (reported when the rule is saved).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Policy '{rule}': {reason}")]
pub struct RuleError {
    pub rule: String,
    pub reason: String,
}

struct Compiled {
    rule: FilenameRule,
    scope: PathBuf,
    required: Option<regex::Regex>,
    forbidden: Option<regex::Regex>,
}

/// A validated, ready-to-evaluate set of rules.
#[derive(Default)]
pub struct PolicySet {
    rules: Vec<Compiled>,
}

fn norm_ext(e: &str) -> String {
    e.trim().trim_start_matches('.').to_lowercase()
}

impl PolicySet {
    /// Compile `rules`, rejecting invalid regexes up front.
    pub fn new(rules: Vec<FilenameRule>) -> Result<Self, RuleError> {
        let mut compiled = Vec::new();
        for rule in rules.into_iter().filter(|r| r.enabled) {
            let re = |p: &Option<String>| -> Result<Option<regex::Regex>, RuleError> {
                p.as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| regex::Regex::new(s).map_err(|e| RuleError { rule: rule.id.clone(), reason: format!("invalid pattern: {e}") }))
                    .transpose()
            };
            let required = re(&rule.required_pattern)?;
            let forbidden = re(&rule.forbidden_pattern)?;
            let scope = PathBuf::from(rule.scope.trim_end_matches(['/', '\\']));
            compiled.push(Compiled { rule, scope, required, forbidden });
        }
        // Shallow scopes first, so `inherit: false` can reset what came before.
        compiled.sort_by_key(|c| c.scope.components().count());
        Ok(Self { rules: compiled })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn applies(c: &Compiled, dir: &Path) -> bool {
        if c.rule.scope.is_empty() {
            return true;
        }
        if c.rule.recursive {
            dir.starts_with(&c.scope)
        } else {
            dir == c.scope
        }
    }

    /// Rules that apply to a file written into `dir`, outermost first.
    pub fn rules_for(&self, dir: &Path) -> Vec<&FilenameRule> {
        let mut out: Vec<&FilenameRule> = Vec::new();
        for c in self.rules.iter().filter(|c| Self::applies(c, dir)) {
            if !c.rule.inherit {
                out.clear();
            }
            out.push(&c.rule);
        }
        out
    }

    /// Check a file that is about to exist at `path`.
    pub fn check(&self, path: &Path) -> Result<(), PolicyViolation> {
        let dir = path.parent().unwrap_or(Path::new(""));
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let ext = path.extension().map(|e| norm_ext(&e.to_string_lossy())).unwrap_or_default();
        let mut applicable: Vec<&Compiled> = Vec::new();
        for c in self.rules.iter().filter(|c| Self::applies(c, dir)) {
            if !c.rule.inherit {
                applicable.clear();
            }
            applicable.push(c);
        }
        for c in applicable {
            let r = &c.rule;
            let fail = |reason: String| PolicyViolation {
                rule_id: r.id.clone(),
                rule_name: r.name.clone(),
                path: path.to_string_lossy().to_string(),
                reason: r.message.clone().unwrap_or(reason),
            };
            if r.blocked_extensions.iter().any(|b| norm_ext(b) == ext && !ext.is_empty()) {
                return Err(fail(format!(".{ext} files are not allowed here")));
            }
            if let Some(allowed) = &r.allowed_extensions {
                if !allowed.iter().any(|a| norm_ext(a) == ext) {
                    let list = allowed.iter().map(|a| format!(".{}", norm_ext(a))).collect::<Vec<_>>().join(", ");
                    return Err(fail(format!("only {list} files are allowed here")));
                }
            }
            if let Some(re) = &c.required {
                if !re.is_match(&name) {
                    return Err(fail(format!("'{name}' does not follow the naming rule {}", re.as_str())));
                }
            }
            if let Some(re) = &c.forbidden {
                if re.is_match(&name) {
                    return Err(fail(format!("'{name}' matches the forbidden pattern {}", re.as_str())));
                }
            }
            if let Some(max) = r.max_name_bytes {
                if name.len() > max {
                    return Err(fail(format!("name is {} bytes; the limit is {max}", name.len())));
                }
            }
        }
        Ok(())
    }

    /// Check many targets; returns every violation (empty = all allowed).
    pub fn check_all<P: AsRef<Path>>(&self, paths: &[P]) -> Vec<PolicyViolation> {
        paths.iter().filter_map(|p| self.check(p.as_ref()).err()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, scope: &str) -> FilenameRule {
        FilenameRule { name: id.to_uppercase(), ..FilenameRule::new(id, scope) }
    }

    #[test]
    fn blocked_and_allowed_extensions() {
        let set = PolicySet::new(vec![
            FilenameRule { blocked_extensions: vec!["EXE".into(), ".bat".into()], ..rule("no-exe", "") },
            FilenameRule { allowed_extensions: Some(vec!["pdf".into(), "csv".into()]), ..rule("finance", "/data/finance") },
        ])
        .unwrap();
        assert!(set.check(Path::new("/tmp/setup.exe")).unwrap_err().reason.contains(".exe"));
        assert!(set.check(Path::new("/tmp/run.BAT")).is_err());
        assert!(set.check(Path::new("/tmp/notes.txt")).is_ok());
        assert!(set.check(Path::new("/data/finance/q3.PDF")).is_ok());
        let v = set.check(Path::new("/data/finance/2026/q3.docx")).unwrap_err();
        assert_eq!(v.rule_id, "finance");
        assert!(v.reason.contains(".pdf, .csv"));
        assert!(set.check(Path::new("/data/finance-old/q3.docx")).is_ok(), "sibling prefix is out of scope");
    }

    #[test]
    fn naming_conventions_and_length() {
        let set = PolicySet::new(vec![FilenameRule {
            required_pattern: Some(r"^\d{4}-\d{2}-\d{2}_".into()),
            forbidden_pattern: Some(r"(?i)final.*final".into()),
            max_name_bytes: Some(40),
            message: None,
            ..rule("reports", "/reports")
        }])
        .unwrap();
        assert!(set.check(Path::new("/reports/2026-10-22_board.pdf")).is_ok());
        assert!(set.check(Path::new("/reports/board.pdf")).unwrap_err().reason.contains("naming rule"));
        assert!(set.check(Path::new("/reports/2026-10-22_final_FINAL.pdf")).unwrap_err().reason.contains("forbidden"));
        assert!(set.check(Path::new(&format!("/reports/2026-10-22_{}.pdf", "x".repeat(40)))).unwrap_err().reason.contains("limit"));
    }

    #[test]
    fn inheritance_and_non_recursive_scopes() {
        let set = PolicySet::new(vec![
            FilenameRule { blocked_extensions: vec!["zip".into()], ..rule("root", "/share") },
            FilenameRule { inherit: false, ..rule("sandbox", "/share/sandbox") },
            FilenameRule { allowed_extensions: Some(vec!["png".into()]), recursive: false, ..rule("icons", "/share/icons") },
        ])
        .unwrap();
        assert!(set.check(Path::new("/share/a.zip")).is_err());
        assert!(set.check(Path::new("/share/sandbox/a.zip")).is_ok(), "inherit:false drops parent rules");
        assert!(set.check(Path::new("/share/icons/a.jpg")).is_err());
        assert!(set.check(Path::new("/share/icons/sub/a.jpg")).is_ok(), "non-recursive rule ends at its folder");
        let names: Vec<_> = set.rules_for(Path::new("/share/sandbox/x")).iter().map(|r| r.id.clone()).collect();
        assert_eq!(names, vec!["sandbox"]);
    }

    #[test]
    fn custom_messages_disabled_rules_and_bad_regex() {
        let set = PolicySet::new(vec![
            FilenameRule { blocked_extensions: vec!["mp3".into()], message: Some("No music on the file server".into()), ..rule("m", "") },
            FilenameRule { blocked_extensions: vec!["txt".into()], enabled: false, ..rule("off", "") },
        ])
        .unwrap();
        assert_eq!(set.check(Path::new("/a/b.mp3")).unwrap_err().reason, "No music on the file server");
        assert!(set.check(Path::new("/a/b.txt")).is_ok());
        let err = PolicySet::new(vec![FilenameRule { required_pattern: Some("(".into()), ..rule("bad", "") }]).err().unwrap();
        assert!(err.reason.contains("invalid pattern"));
        assert_eq!(set.check_all(&["/x/a.mp3", "/x/b.wav"]).len(), 1);
    }

    #[test]
    fn rules_round_trip_with_defaults() {
        let r: FilenameRule = serde_json::from_str(r#"{"id":"x","blocked_extensions":["exe"]}"#).unwrap();
        assert!(r.blocked_extensions == vec!["exe"] && r.enabled && r.recursive && r.inherit, "missing fields take the defaults");
    }
}
