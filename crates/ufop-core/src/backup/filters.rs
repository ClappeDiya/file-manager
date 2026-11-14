//! Source selection: ordered include/exclude rules (exact path, wildcard,
//! regex, predefined groups) plus attribute, size, marker-file and
//! cloud-placeholder exclusions.
//!
//! Rules are evaluated in order and the first match wins; entries matching
//! no rule are included. Directory paths are matched with a trailing `/`, so
//! `/data/cache/` only matches the folder while `/data/cache` matches either.
//! An excluded directory prunes traversal, so rules for its descendants are
//! never consulted.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::Path;

// The rule engine is shared with the sync planner (`crate::filter_rules`).
pub use crate::filter_rules::{parse_filter_file, FilterAction, FilterKind, FilterRule, RuleSet, FILTER_GROUPS};

/// Compiled source filter with every selection option.
///
/// Precedence: the ordered [`FilterRule`]s decide first (an explicit
/// include re-includes what an ignore file excludes); entries no rule
/// matches are then checked against the ignore files (see
/// [`super::ignore`]); the attribute, size and marker exclusions apply to
/// everything, including explicitly included entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceFilter {
    pub rules: Vec<FilterRule>,
    pub exclude_hidden: bool,
    pub exclude_system: bool,
    pub exclude_temporary: bool,
    /// Skip files larger than this (bytes).
    pub max_file_size: Option<u64>,
    /// Skip directories containing any of these file names. `CACHEDIR.TAG`
    /// only counts when it carries the standard signature.
    pub exclusion_markers: Vec<String>,
    pub exclude_empty_folders: bool,
    /// Skip offline / recall-on-access cloud placeholders (Windows).
    pub exclude_cloud_placeholders: bool,
    /// Ignore-file names read in every folder (gitignore syntax).
    pub ignore_files: Vec<String>,
    /// Also honour `.gitignore` files and `.git/info/exclude` of
    /// repositories inside the sources.
    pub respect_gitignore: bool,
    #[serde(skip)]
    compiled: RuleSet,
}

impl Default for SourceFilter {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            exclude_hidden: false,
            exclude_system: false,
            exclude_temporary: false,
            max_file_size: None,
            exclusion_markers: Vec::new(),
            exclude_empty_folders: false,
            exclude_cloud_placeholders: false,
            ignore_files: vec![super::ignore::DEFAULT_IGNORE_FILE.to_string()],
            respect_gitignore: false,
            compiled: RuleSet::default(),
        }
    }
}

const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

impl SourceFilter {
    /// Validate and compile the rules. Must be called before `evaluate`.
    pub fn compile(mut self) -> Result<Self, AppError> {
        self.compiled = RuleSet::compile(&self.rules)?;
        Ok(self)
    }

    /// First matching rule's action; `None` when no rule matches.
    pub fn evaluate(&self, path: &str, is_dir: bool) -> Option<FilterAction> {
        self.compiled.evaluate(path, is_dir)
    }

    /// Like [`evaluate`](Self::evaluate) for `path` found under the source
    /// folder `root`: rclone rules (imported `--filter-from` files) are
    /// matched on the path inside `root`, so `/Photos/**` is the source's
    /// own `Photos` folder; the other rule kinds keep matching the full path.
    pub fn evaluate_under(&self, path: &Path, root: Option<&Path>, is_dir: bool) -> Option<FilterAction> {
        let rel = root.and_then(|r| path.strip_prefix(r).ok()).map(|r| r.to_string_lossy());
        self.compiled.evaluate_rooted(&path.to_string_lossy(), rel.as_deref(), is_dir)
    }

    /// Whether the entry is selected, applying rules and attribute checks.
    pub fn accepts(&self, path: &Path, md: &Metadata) -> bool {
        self.accepts_under(path, None, md)
    }

    /// [`accepts`](Self::accepts) for an entry under the source folder
    /// `root` (see [`evaluate_under`](Self::evaluate_under)).
    pub fn accepts_under(&self, path: &Path, root: Option<&Path>, md: &Metadata) -> bool {
        if self.evaluate_under(path, root, md.is_dir()) == Some(FilterAction::Exclude) {
            return false;
        }
        if self.exclude_hidden && is_hidden(path, md) {
            return false;
        }
        if (self.exclude_system && is_system(md)) || (self.exclude_temporary && is_temporary(md)) {
            return false;
        }
        if self.exclude_cloud_placeholders && is_cloud_placeholder(md) {
            return false;
        }
        if md.is_file() {
            if let Some(max) = self.max_file_size {
                if md.len() > max {
                    return false;
                }
            }
        }
        if md.is_dir() && self.has_marker(path) {
            return false;
        }
        true
    }

    /// Which ignore files the scanner reads.
    pub fn ignore_config(&self) -> super::ignore::IgnoreConfig {
        super::ignore::IgnoreConfig { files: self.ignore_files.clone(), gitignore: self.respect_gitignore }
    }

    fn has_marker(&self, dir: &Path) -> bool {
        self.exclusion_markers.iter().any(|m| {
            let p = dir.join(m);
            if !p.is_file() {
                return false;
            }
            if m == "CACHEDIR.TAG" {
                return std::fs::read(&p).map(|d| d.starts_with(CACHEDIR_SIGNATURE)).unwrap_or(false);
            }
            true
        })
    }
}

fn is_hidden(path: &Path, _md: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if _md.file_attributes() & 0x2 != 0 {
            return true;
        }
    }
    path.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false)
}

#[allow(unused_variables)]
fn win_attr(md: &Metadata, mask: u32) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return md.file_attributes() & mask != 0;
    }
    #[allow(unreachable_code)]
    false
}

fn is_system(md: &Metadata) -> bool {
    win_attr(md, 0x4)
}

fn is_temporary(md: &Metadata) -> bool {
    win_attr(md, 0x100)
}

fn is_cloud_placeholder(md: &Metadata) -> bool {
    // FILE_ATTRIBUTE_OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS
    win_attr(md, 0x1000 | 0x40000 | 0x400000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(action: FilterAction, kind: FilterKind, p: &str) -> FilterRule {
        FilterRule { action, kind, pattern: p.into() }
    }

    #[test]
    fn ordered_first_match_wins() {
        let f = SourceFilter {
            rules: vec![
                rule(FilterAction::Include, FilterKind::Wildcard, "*/keep.log"),
                rule(FilterAction::Exclude, FilterKind::Wildcard, "*.log"),
            ],
            ..Default::default()
        }
        .compile()
        .unwrap();
        assert_eq!(f.evaluate("/a/keep.log", false), Some(FilterAction::Include));
        assert_eq!(f.evaluate("/a/other.log", false), Some(FilterAction::Exclude));
        assert_eq!(f.evaluate("/a/x.txt", false), None);
    }

    #[test]
    fn exact_file_vs_directory() {
        let f = SourceFilter { rules: vec![rule(FilterAction::Exclude, FilterKind::Exact, "/d/cache/")], ..Default::default() }
            .compile()
            .unwrap();
        assert_eq!(f.evaluate("/d/cache", true), Some(FilterAction::Exclude));
        assert_eq!(f.evaluate("/d/cache", false), None);
    }

    #[test]
    fn regex_and_groups() {
        let f = SourceFilter {
            rules: vec![
                rule(FilterAction::Exclude, FilterKind::Regex, r"\.bak\d+$"),
                rule(FilterAction::Exclude, FilterKind::Group, "os_metadata"),
            ],
            ..Default::default()
        }
        .compile()
        .unwrap();
        assert_eq!(f.evaluate("/x/a.bak12", false), Some(FilterAction::Exclude));
        assert_eq!(f.evaluate("/x/.DS_Store", false), Some(FilterAction::Exclude));
        assert!(SourceFilter { rules: vec![rule(FilterAction::Exclude, FilterKind::Group, "nope")], ..Default::default() }
            .compile()
            .is_err());
        assert!(SourceFilter { rules: vec![rule(FilterAction::Exclude, FilterKind::Regex, "(")], ..Default::default() }
            .compile()
            .is_err());
    }

    #[test]
    fn size_hidden_and_markers() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("big"), vec![0u8; 100]).unwrap();
        std::fs::write(d.path().join(".hid"), b"x").unwrap();
        std::fs::create_dir(d.path().join("c")).unwrap();
        std::fs::write(d.path().join("c/CACHEDIR.TAG"), b"bogus").unwrap();
        std::fs::create_dir(d.path().join("n")).unwrap();
        std::fs::write(d.path().join("n/.nobackup"), b"").unwrap();
        let f = SourceFilter {
            max_file_size: Some(50),
            exclude_hidden: true,
            exclusion_markers: vec![".nobackup".into(), "CACHEDIR.TAG".into()],
            ..Default::default()
        }
        .compile()
        .unwrap();
        let md = |n: &str| std::fs::metadata(d.path().join(n)).unwrap();
        assert!(!f.accepts(&d.path().join("big"), &md("big")));
        assert!(!f.accepts(&d.path().join(".hid"), &md(".hid")));
        assert!(!f.accepts(&d.path().join("n"), &md("n")));
        // Unsigned CACHEDIR.TAG does not count.
        assert!(f.accepts(&d.path().join("c"), &md("c")));
        std::fs::write(d.path().join("c/CACHEDIR.TAG"), CACHEDIR_SIGNATURE).unwrap();
        assert!(!f.accepts(&d.path().join("c"), &md("c")));
    }
}
