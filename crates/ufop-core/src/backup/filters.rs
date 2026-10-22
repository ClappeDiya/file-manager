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
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterAction {
    Include,
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    /// Literal path.
    Exact,
    /// `*` (any run, including separators) and `?` (one char).
    Wildcard,
    Regex,
    /// A predefined group name (see [`FILTER_GROUPS`]).
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRule {
    pub action: FilterAction,
    pub kind: FilterKind,
    pub pattern: String,
}

/// Predefined exclusion groups: (name, description, wildcard patterns).
pub const FILTER_GROUPS: &[(&str, &str, &[&str])] = &[
    (
        "temporary_files",
        "Editor swap files, partial downloads and temp folders",
        &["*.tmp", "*.temp", "*~", "*.swp", "*.swo", "*.part", "*.crdownload", "*/tmp/", "*/Temp/", "*/.Trash-*/"],
    ),
    (
        "cache_files",
        "Application and browser caches",
        &["*/.cache/", "*/Cache/", "*/Caches/", "*/node_modules/.cache/", "*/__pycache__/", "*.pyc", "*/Library/Caches/"],
    ),
    (
        "os_metadata",
        "Desktop/Finder metadata files",
        &["*/.DS_Store", "*/Thumbs.db", "*/desktop.ini", "*/.Spotlight-V100/", "*/.fseventsd/", "*/.TemporaryItems/", "*/$RECYCLE.BIN/", "*/.Trashes/"],
    ),
    (
        "system_files",
        "Operating-system files that cannot be restored meaningfully",
        &["*/pagefile.sys", "*/hiberfil.sys", "*/swapfile.sys", "*/System Volume Information/", "/proc/", "/sys/", "/dev/", "/run/", "/private/var/vm/"],
    ),
    (
        "build_artifacts",
        "Rebuildable developer output",
        &["*/node_modules/", "*/target/debug/", "*/target/release/", "*/.gradle/", "*/build/intermediates/"],
    ),
];

#[derive(Debug, Clone)]
enum Matcher {
    Exact(String),
    Glob(glob::Pattern),
    Regex(Regex),
}

/// Compiled source filter with every selection option.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(skip)]
    compiled: Vec<(FilterAction, Matcher)>,
}

const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

impl SourceFilter {
    /// Validate and compile the rules. Must be called before `evaluate`.
    pub fn compile(mut self) -> Result<Self, AppError> {
        let mut compiled = Vec::new();
        for r in &self.rules {
            match r.kind {
                FilterKind::Exact => compiled.push((r.action, Matcher::Exact(normalize(&r.pattern)))),
                FilterKind::Wildcard => compiled.push((r.action, Matcher::Glob(compile_glob(&r.pattern)?))),
                FilterKind::Regex => {
                    let re = Regex::new(&r.pattern).map_err(|e| {
                        AppError::validation(format!("Invalid regular expression '{}': {e}", r.pattern))
                    })?;
                    compiled.push((r.action, Matcher::Regex(re)));
                }
                FilterKind::Group => {
                    let (_, _, pats) = FILTER_GROUPS
                        .iter()
                        .find(|(n, _, _)| *n == r.pattern)
                        .ok_or_else(|| AppError::validation(format!("Unknown filter group '{}'", r.pattern)))?;
                    for p in pats.iter() {
                        compiled.push((r.action, Matcher::Glob(compile_glob(p)?)));
                    }
                }
            }
        }
        self.compiled = compiled;
        Ok(self)
    }

    /// First matching rule's action; `None` when no rule matches.
    pub fn evaluate(&self, path: &str, is_dir: bool) -> Option<FilterAction> {
        let mut p = normalize(path);
        if is_dir && !p.ends_with('/') {
            p.push('/');
        }
        let opts = glob::MatchOptions { case_sensitive: !cfg!(windows), require_literal_separator: false, require_literal_leading_dot: false };
        for (action, m) in &self.compiled {
            let hit = match m {
                Matcher::Exact(e) => {
                    if e.ends_with('/') {
                        is_dir && p == *e
                    } else {
                        p == *e || (is_dir && p.trim_end_matches('/') == e)
                    }
                }
                Matcher::Glob(g) => g.matches_with(&p, opts),
                Matcher::Regex(r) => r.is_match(&p),
            };
            if hit {
                return Some(*action);
            }
        }
        None
    }

    /// Whether the entry is selected, applying rules and attribute checks.
    pub fn accepts(&self, path: &Path, md: &Metadata) -> bool {
        let s = path.to_string_lossy();
        if self.evaluate(&s, md.is_dir()) == Some(FilterAction::Exclude) {
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

fn compile_glob(p: &str) -> Result<glob::Pattern, AppError> {
    glob::Pattern::new(&normalize(p)).map_err(|e| AppError::validation(format!("Invalid wildcard '{p}': {e}")))
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
