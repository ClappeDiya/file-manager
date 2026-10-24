//! The one include/exclude rule engine: ordered rules (exact path, wildcard,
//! regex, predefined groups, rclone-style globs), first match wins.
//!
//! Used by backup source selection (`backup::filters::SourceFilter`) and by
//! the sync planner (`sync::filter`, which also maps the older sync filter
//! lists onto it). Paths are matched with `/` separators; directories with a
//! trailing `/`, so `cache/` only matches the folder while `cache` matches
//! either.
//!
//! Rules can be written in rclone's `--filter-from` format
//! ([`parse_filter_file`]): one `+ pattern` (include) or `- pattern`
//! (exclude) per line, `#` / `;` comments, and `!` to clear the rules so far.

use crate::error::AppError;
use regex::Regex;
use serde::{Deserialize, Serialize};

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
    /// rclone glob: `*` stays inside one path segment, `**` crosses them, a
    /// leading `/` anchors at the root (otherwise any folder matches), a
    /// trailing `/` matches a folder and everything in it, `{a,b}`
    /// alternatives and `{{regex}}` pieces.
    Rclone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRule {
    pub action: FilterAction,
    pub kind: FilterKind,
    pub pattern: String,
}

/// Predefined groups: (name, description, wildcard patterns). Media and
/// document groups list lower- and upper-case extensions (matching is
/// case-sensitive outside Windows).
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
    (
        "images",
        "Photos and pictures",
        &[
            "*.jpg", "*.JPG", "*.jpeg", "*.JPEG", "*.png", "*.PNG", "*.gif", "*.GIF", "*.bmp", "*.BMP", "*.tif", "*.TIF", "*.tiff", "*.TIFF",
            "*.webp", "*.WEBP", "*.heic", "*.HEIC", "*.heif", "*.HEIF", "*.raw", "*.RAW", "*.cr2", "*.CR2", "*.cr3", "*.CR3", "*.nef", "*.NEF",
            "*.arw", "*.ARW", "*.dng", "*.DNG", "*.svg", "*.SVG", "*.psd", "*.PSD",
        ],
    ),
    (
        "video",
        "Video files",
        &[
            "*.mp4", "*.MP4", "*.m4v", "*.M4V", "*.mov", "*.MOV", "*.avi", "*.AVI", "*.mkv", "*.MKV", "*.wmv", "*.WMV", "*.flv", "*.FLV",
            "*.webm", "*.WEBM", "*.mpg", "*.MPG", "*.mpeg", "*.MPEG", "*.3gp", "*.3GP", "*.mts", "*.MTS", "*.m2ts", "*.M2TS",
        ],
    ),
    (
        "audio",
        "Music and sound files",
        &[
            "*.mp3", "*.MP3", "*.wav", "*.WAV", "*.flac", "*.FLAC", "*.aac", "*.AAC", "*.m4a", "*.M4A", "*.ogg", "*.OGG", "*.opus", "*.OPUS",
            "*.wma", "*.WMA", "*.aiff", "*.AIFF", "*.aif", "*.AIF", "*.alac", "*.ALAC", "*.mid", "*.MID", "*.midi", "*.MIDI",
        ],
    ),
    (
        "documents",
        "Office documents, PDFs and text",
        &[
            "*.pdf", "*.PDF", "*.doc", "*.DOC", "*.docx", "*.DOCX", "*.xls", "*.XLS", "*.xlsx", "*.XLSX", "*.ppt", "*.PPT", "*.pptx", "*.PPTX",
            "*.odt", "*.ODT", "*.ods", "*.ODS", "*.odp", "*.ODP", "*.rtf", "*.RTF", "*.txt", "*.TXT", "*.md", "*.MD", "*.csv", "*.CSV",
            "*.pages", "*.numbers", "*.key", "*.epub", "*.EPUB",
        ],
    ),
    (
        "archives",
        "Compressed archives and disk images",
        &[
            "*.zip", "*.ZIP", "*.7z", "*.7Z", "*.rar", "*.RAR", "*.tar", "*.TAR", "*.gz", "*.GZ", "*.tgz", "*.TGZ", "*.bz2", "*.BZ2",
            "*.xz", "*.XZ", "*.zst", "*.ZST", "*.iso", "*.ISO", "*.dmg", "*.DMG", "*.cab", "*.CAB",
        ],
    ),
];

/// The wildcard patterns of a predefined group.
pub fn group_patterns(name: &str) -> Option<&'static [&'static str]> {
    FILTER_GROUPS.iter().find(|(n, _, _)| *n == name).map(|(_, _, p)| *p)
}

#[derive(Debug, Clone)]
enum Matcher {
    Exact(String),
    Glob(glob::Pattern),
    Regex(Regex),
}

/// Compiled, ordered rules.
#[derive(Debug, Clone)]
pub struct RuleSet {
    compiled: Vec<(FilterAction, Matcher)>,
    /// Wildcards are case-sensitive except on Windows unless overridden.
    case_sensitive: bool,
}

impl Default for RuleSet {
    fn default() -> Self {
        Self { compiled: Vec::new(), case_sensitive: !cfg!(windows) }
    }
}

pub(crate) fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

fn compile_glob(p: &str) -> Result<glob::Pattern, AppError> {
    glob::Pattern::new(&normalize(p)).map_err(|e| AppError::validation(format!("Invalid wildcard '{p}': {e}")))
}

fn match_options(case_sensitive: bool) -> glob::MatchOptions {
    glob::MatchOptions { case_sensitive, require_literal_separator: false, require_literal_leading_dot: false }
}

/// Whether wildcard `pattern` matches `path` (`*` also crosses `/`). An
/// invalid pattern matches nothing.
pub fn wildcard_matches(pattern: &str, path: &str, case_sensitive: bool) -> bool {
    compile_glob(pattern).map(|g| g.matches_with(&normalize(path), match_options(case_sensitive))).unwrap_or(false)
}

impl RuleSet {
    /// Validate and compile `rules` (in order).
    pub fn compile(rules: &[FilterRule]) -> Result<Self, AppError> {
        let mut compiled = Vec::new();
        for r in rules {
            match r.kind {
                FilterKind::Exact => compiled.push((r.action, Matcher::Exact(normalize(&r.pattern)))),
                FilterKind::Wildcard => compiled.push((r.action, Matcher::Glob(compile_glob(&r.pattern)?))),
                FilterKind::Regex => {
                    let re = Regex::new(&r.pattern).map_err(|e| AppError::validation(format!("Invalid regular expression '{}': {e}", r.pattern)))?;
                    compiled.push((r.action, Matcher::Regex(re)));
                }
                FilterKind::Rclone => {
                    let re = Regex::new(&rclone_to_regex(&r.pattern)?)
                        .map_err(|e| AppError::validation(format!("Invalid pattern '{}': {e}", r.pattern)))?;
                    compiled.push((r.action, Matcher::Regex(re)));
                }
                FilterKind::Group => {
                    let pats = group_patterns(&r.pattern).ok_or_else(|| AppError::validation(format!("Unknown filter group '{}'", r.pattern)))?;
                    for p in pats.iter() {
                        compiled.push((r.action, Matcher::Glob(compile_glob(p)?)));
                    }
                }
            }
        }
        Ok(Self { compiled, ..Self::default() })
    }

    /// Match wildcards case-sensitively (or not) regardless of platform.
    pub fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.compiled.is_empty()
    }

    /// First matching rule's action; `None` when no rule matches.
    pub fn evaluate(&self, path: &str, is_dir: bool) -> Option<FilterAction> {
        self.evaluate_any(&[path], is_dir)
    }

    /// Like [`evaluate`](Self::evaluate) for a path that has several
    /// spellings: the first rule that matches any of `paths` decides.
    pub fn evaluate_any(&self, paths: &[&str], is_dir: bool) -> Option<FilterAction> {
        let paths: Vec<String> = paths
            .iter()
            .map(|path| {
                let mut p = normalize(path);
                if is_dir && !p.ends_with('/') {
                    p.push('/');
                }
                p
            })
            .collect();
        let opts = match_options(self.case_sensitive);
        for (action, m) in &self.compiled {
            let hit = |p: &String| match m {
                Matcher::Exact(e) => {
                    if e.ends_with('/') {
                        is_dir && p == e
                    } else {
                        p == e || (is_dir && p.trim_end_matches('/') == e)
                    }
                }
                Matcher::Glob(g) => g.matches_with(p, opts),
                Matcher::Regex(r) => r.is_match(p),
            };
            if paths.iter().any(hit) {
                return Some(*action);
            }
        }
        None
    }
}

/// Translate an rclone filter glob into a regular expression over
/// `/`-separated paths (relative, or absolute with a leading `/`).
pub fn rclone_to_regex(pattern: &str) -> Result<String, AppError> {
    let bad = |why: &str| AppError::validation(format!("Invalid pattern '{pattern}': {why}"));
    let mut p = pattern.trim();
    if p.is_empty() {
        return Err(bad("empty pattern"));
    }
    let anchored = p.starts_with('/');
    if anchored {
        p = &p[1..];
    }
    let dir_only = p.ends_with('/') && p.len() > 1;
    if dir_only {
        p = &p[..p.len() - 1];
    }
    let mut re = String::from(if anchored { "^/?" } else { "(^|/)" });
    let chars: Vec<char> = p.chars().collect();
    let mut i = 0;
    let mut braces = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let next = chars.get(i + 1).ok_or_else(|| bad("ends with a backslash"))?;
                re.push_str(&regex::escape(&next.to_string()));
                i += 1;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                re.push_str(".*");
                i += 1;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '[' => {
                let end = chars[i + 1..].iter().position(|&c| c == ']').ok_or_else(|| bad("unclosed ["))? + i + 1;
                let mut class: String = chars[i + 1..end].iter().collect();
                if let Some(rest) = class.strip_prefix('!') {
                    class = format!("^{rest}");
                }
                re.push('[');
                re.push_str(&class.replace('\\', "\\\\"));
                re.push(']');
                i = end;
            }
            '{' if chars.get(i + 1) == Some(&'{') => {
                // Raw regex up to the closing `}}`; in `{{\d{4}}}` the
                // regex's own `}` comes first, so take the last of a run.
                let mut j = i + 2;
                let close = loop {
                    if j + 1 >= chars.len() {
                        return Err(bad("unclosed {{"));
                    }
                    if chars[j] == '}' && chars[j + 1] == '}' && chars.get(j + 2) != Some(&'}') {
                        break j;
                    }
                    j += 1;
                };
                let raw: String = chars[i + 2..close].iter().collect();
                re.push_str(&format!("(?:{raw})"));
                i = close + 1;
            }
            '{' => {
                braces += 1;
                re.push_str("(?:");
            }
            ',' if braces > 0 => re.push('|'),
            '}' if braces > 0 => {
                braces -= 1;
                re.push(')');
            }
            _ => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    if braces > 0 {
        return Err(bad("unclosed {"));
    }
    // A folder pattern (trailing `/`) matches the folder and everything in
    // it; any other pattern matches whole paths only, as in rclone.
    re.push_str(if dir_only { "/" } else { "$" });
    Ok(re)
}

/// Read rules in rclone's `--filter-from` format: `+ pattern` includes,
/// `- pattern` excludes, `#` and `;` start comments, `!` clears the rules
/// read so far. Patterns are rclone globs ([`FilterKind::Rclone`]).
pub fn parse_filter_file(text: &str) -> Result<Vec<FilterRule>, AppError> {
    let mut rules = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line == "!" {
            rules.clear();
            continue;
        }
        let (action, rest) = match line.split_at(1) {
            ("+", rest) => (FilterAction::Include, rest),
            ("-", rest) => (FilterAction::Exclude, rest),
            _ => {
                return Err(AppError::validation(format!(
                    "Line {}: \"{line}\" must start with \"+ \" (include) or \"- \" (exclude)",
                    n + 1
                )))
            }
        };
        let pattern = rest.strip_prefix(' ').unwrap_or(rest).trim_end();
        if pattern.is_empty() {
            return Err(AppError::validation(format!("Line {}: missing pattern", n + 1)));
        }
        rclone_to_regex(pattern).map_err(|e| AppError::validation(format!("Line {}: {}", n + 1, e.user_message())))?;
        rules.push(FilterRule { action, kind: FilterKind::Rclone, pattern: pattern.to_string() });
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(action: FilterAction, kind: FilterKind, p: &str) -> FilterRule {
        FilterRule { action, kind, pattern: p.into() }
    }

    #[test]
    fn rclone_globs() {
        let set = |p: &str| RuleSet::compile(&[rule(FilterAction::Exclude, FilterKind::Rclone, p)]).unwrap();
        let hit = |s: &RuleSet, path: &str, dir: bool| s.evaluate(path, dir) == Some(FilterAction::Exclude);
        // Unanchored: any folder; `*` stays in one segment.
        let s = set("*.jpg");
        assert!(hit(&s, "a.jpg", false) && hit(&s, "x/y/a.jpg", false) && hit(&s, "/home/u/a.jpg", false));
        assert!(!hit(&s, "a.jpgx", false));
        let s = set("docs/*.txt");
        assert!(hit(&s, "docs/a.txt", false) && hit(&s, "p/docs/a.txt", false));
        assert!(!hit(&s, "docs/sub/a.txt", false));
        // `**` crosses folders; leading `/` anchors at the root.
        let s = set("/docs/**");
        assert!(hit(&s, "docs/sub/a.txt", false) && hit(&s, "/docs/a", false));
        assert!(!hit(&s, "x/docs/a", false));
        // Trailing `/`: the folder and everything in it.
        let s = set("node_modules/");
        assert!(hit(&s, "app/node_modules", true) && hit(&s, "app/node_modules/x/y.js", false));
        assert!(!hit(&s, "app/node_modules", false));
        // Alternatives, character classes, raw regex and escapes.
        let s = set("*.{jpg,png}");
        assert!(hit(&s, "a.png", false) && !hit(&s, "a.gif", false));
        let s = set("file[0-9].txt");
        assert!(hit(&s, "file7.txt", false) && !hit(&s, "fileA.txt", false));
        let s = set("{{\\d{4}}}-report.pdf");
        assert!(hit(&s, "2024-report.pdf", false) && !hit(&s, "abcd-report.pdf", false));
        let s = set("weird\\*name");
        assert!(hit(&s, "weird*name", false) && !hit(&s, "weirdXname", false));
        assert!(rclone_to_regex("a{b").is_err() && rclone_to_regex("[ab").is_err());
    }

    #[test]
    fn filter_file_format() {
        let text = "# photos only\n; also a comment\n\n- *.tmp\n+ /Photos/**\n- *\n";
        let rules = parse_filter_file(text).unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[1], rule(FilterAction::Include, FilterKind::Rclone, "/Photos/**"));
        let set = RuleSet::compile(&rules).unwrap();
        assert_eq!(set.evaluate("Photos/2024/a.jpg", false), Some(FilterAction::Include));
        assert_eq!(set.evaluate("Photos/a.tmp", false), Some(FilterAction::Exclude));
        assert_eq!(set.evaluate("Music/a.mp3", false), Some(FilterAction::Exclude));
        // `!` clears what came before.
        assert_eq!(parse_filter_file("- a\n!\n+ b").unwrap(), vec![rule(FilterAction::Include, FilterKind::Rclone, "b")]);
        let err = parse_filter_file("- ok\nnope").unwrap_err();
        assert!(err.to_string().contains("Line 2"), "{err}");
        assert!(parse_filter_file("+ a{b").is_err());
    }

    #[test]
    fn media_and_document_groups() {
        let set = |g: &str| RuleSet::compile(&[rule(FilterAction::Exclude, FilterKind::Group, g)]).unwrap();
        for (g, yes, no) in [
            ("images", "x/IMG_1.JPG", "x/a.mp4"),
            ("video", "clip.mov", "song.mp3"),
            ("audio", "song.flac", "doc.pdf"),
            ("documents", "Report.DOCX", "pic.png"),
            ("archives", "backup.tar.gz", "notes.txt"),
        ] {
            let s = set(g).with_case_sensitive(true);
            assert_eq!(s.evaluate(yes, false), Some(FilterAction::Exclude), "{g}: {yes}");
            assert_eq!(s.evaluate(no, false), None, "{g}: {no}");
        }
    }

    #[test]
    fn wildcards_case_and_validity() {
        assert!(wildcard_matches("*.TXT", "a/b.TXT", true));
        assert!(!wildcard_matches("*.TXT", "a/b.txt", true));
        assert!(wildcard_matches("*.TXT", "a/b.txt", false));
        assert!(!wildcard_matches("[", "x", true), "invalid patterns match nothing");
    }
}
