//! Sync selection on the shared rule engine (`crate::filter_rules`).
//!
//! A pair's [`SyncFilter`] has ordered include/exclude rules (`rules`,
//! first match wins, the same engine backup jobs use) and the older lists
//! (include/exclude globs and regexes, size limits, file types). The rules
//! decide first: an exclude rule drops the file, an include rule keeps it
//! regardless of the pattern lists. Files no rule matches go through the
//! lists exactly as before — every list must agree — and the size and
//! file-type limits apply to every file.

use crate::error::AppError;
use crate::filter_rules::{FilterAction, FilterKind, FilterRule, RuleSet};
use crate::sync_types::SyncFilter;
use std::path::Path;

/// A [`SyncFilter`] compiled once per plan.
#[derive(Debug, Clone)]
pub struct CompiledSyncFilter {
    rules: RuleSet,
    /// Exclude globs and regexes: any match drops the file.
    excludes: RuleSet,
    /// Include globs / regexes: when listed, a file must match one of each.
    include_globs: Option<RuleSet>,
    include_regex: Option<RuleSet>,
    min_size: u64,
    max_size: u64,
    file_types: Vec<String>,
    exclude_file_types: Vec<String>,
}

/// The legacy lists treat a pattern that does not compile as matching
/// nothing; keep that by leaving such patterns out.
fn valid_rules(action: FilterAction, kind: FilterKind, patterns: &[String]) -> Vec<FilterRule> {
    patterns
        .iter()
        .map(|p| FilterRule { action, kind, pattern: p.clone() })
        .filter(|r| RuleSet::compile(std::slice::from_ref(r)).is_ok())
        .collect()
}

/// Sync globs have always been case-sensitive on every platform.
fn compile_list(rules: &[FilterRule]) -> RuleSet {
    RuleSet::compile(rules).unwrap_or_default().with_case_sensitive(true)
}

impl CompiledSyncFilter {
    /// Compile `filter`. Fails only on an invalid ordered rule.
    pub fn new(filter: &SyncFilter) -> Result<Self, AppError> {
        let mut excludes = valid_rules(FilterAction::Exclude, FilterKind::Wildcard, &filter.exclude_patterns);
        excludes.extend(valid_rules(FilterAction::Exclude, FilterKind::Regex, &filter.exclude_regex));
        let include = |kind, patterns: &[String]| {
            (!patterns.is_empty()).then(|| compile_list(&valid_rules(FilterAction::Include, kind, patterns)))
        };
        Ok(Self {
            rules: RuleSet::compile(&filter.rules)?.with_case_sensitive(true),
            excludes: compile_list(&excludes),
            include_globs: include(FilterKind::Wildcard, &filter.include_patterns),
            include_regex: include(FilterKind::Regex, &filter.include_regex),
            min_size: filter.min_size,
            max_size: filter.max_size,
            file_types: filter.file_types.clone(),
            exclude_file_types: filter.exclude_file_types.clone(),
        })
    }

    /// Whether the file at `relative_path` (of `size` bytes) is synced.
    pub fn accepts(&self, relative_path: &str, size: u64) -> bool {
        let explicit = self.rules.evaluate(relative_path, false);
        if explicit == Some(FilterAction::Exclude) {
            return false;
        }
        if explicit.is_none() && !self.lists_accept(relative_path) {
            return false;
        }

        // Size filters
        if self.min_size > 0 && size < self.min_size {
            return false;
        }
        if self.max_size > 0 && size > self.max_size {
            return false;
        }

        let ext = Path::new(relative_path).extension().and_then(|e| e.to_str()).unwrap_or("");
        if !self.file_types.is_empty() && !self.file_types.iter().any(|t| t.eq_ignore_ascii_case(ext)) {
            return false;
        }
        if self.exclude_file_types.iter().any(|t| t.eq_ignore_ascii_case(ext)) {
            return false;
        }
        true
    }

    fn lists_accept(&self, relative_path: &str) -> bool {
        let included = |set: &Option<RuleSet>| match set {
            Some(s) => s.evaluate(relative_path, false).is_some(),
            None => true,
        };
        included(&self.include_globs)
            && self.excludes.evaluate(relative_path, false).is_none()
            && included(&self.include_regex)
    }
}

/// Check the ordered rules of a pair's filter (create / update).
pub fn validate(filter: &SyncFilter) -> Result<(), AppError> {
    CompiledSyncFilter::new(filter).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(action: FilterAction, kind: FilterKind, p: &str) -> FilterRule {
        FilterRule { action, kind, pattern: p.into() }
    }

    #[test]
    fn ordered_rules_come_first() {
        let f = SyncFilter {
            rules: vec![
                rule(FilterAction::Include, FilterKind::Wildcard, "logs/keep.log"),
                rule(FilterAction::Exclude, FilterKind::Regex, r"^cache/"),
            ],
            exclude_patterns: vec!["*.log".into()],
            max_size: 100,
            ..Default::default()
        };
        let c = CompiledSyncFilter::new(&f).unwrap();
        assert!(c.accepts("logs/keep.log", 10), "an include rule beats the exclude list");
        assert!(!c.accepts("logs/other.log", 10));
        assert!(!c.accepts("cache/a.txt", 10));
        assert!(c.accepts("a.txt", 10));
        assert!(!c.accepts("logs/keep.log", 1000), "size limits still apply");
        let bad = SyncFilter { rules: vec![rule(FilterAction::Exclude, FilterKind::Regex, "(")], ..Default::default() };
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn legacy_lists_keep_their_meaning() {
        // Both include lists must match; an invalid pattern matches nothing.
        let f = SyncFilter {
            include_patterns: vec!["*.txt".into(), "[".into()],
            include_regex: vec!["^docs/".into()],
            exclude_regex: vec!["secret".into(), "(".into()],
            ..Default::default()
        };
        let c = CompiledSyncFilter::new(&f).unwrap();
        assert!(c.accepts("docs/a.txt", 1));
        assert!(!c.accepts("other/a.txt", 1));
        assert!(!c.accepts("docs/a.pdf", 1));
        assert!(!c.accepts("docs/secret.txt", 1));
        let only_invalid = SyncFilter { include_patterns: vec!["[".into()], ..Default::default() };
        assert!(!CompiledSyncFilter::new(&only_invalid).unwrap().accepts("a.txt", 1));
        // Globs stay case-sensitive.
        let f = SyncFilter { exclude_patterns: vec!["*.TMP".into()], ..Default::default() };
        assert!(CompiledSyncFilter::new(&f).unwrap().accepts("a.tmp", 1));
    }
}
