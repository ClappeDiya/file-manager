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

/// Which entries a compiled rule is tried on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applies {
    Any,
    /// An rclone file pattern: as in rclone it never excludes a folder, so
    /// `+ *.jpg` / `- *` still finds JPEGs in subfolders.
    Files,
    /// A folder an rclone include pattern needs to look into (`Photos/`
    /// for `+ /Photos/*.jpg`).
    Dirs,
}

#[derive(Debug, Clone)]
struct Compiled {
    action: FilterAction,
    matcher: Matcher,
    applies: Applies,
    /// rclone patterns name paths from the top of what is being walked
    /// (see [`RuleSet::evaluate_rooted`]).
    relative: bool,
}

/// Compiled, ordered rules.
#[derive(Debug, Clone)]
pub struct RuleSet {
    compiled: Vec<Compiled>,
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
        let any = |action, matcher| Compiled { action, matcher, applies: Applies::Any, relative: false };
        for r in rules {
            match r.kind {
                FilterKind::Exact => compiled.push(any(r.action, Matcher::Exact(normalize(&r.pattern)))),
                FilterKind::Wildcard => compiled.push(any(r.action, Matcher::Glob(compile_glob(&r.pattern)?))),
                FilterKind::Regex => {
                    let re = Regex::new(&r.pattern).map_err(|e| AppError::validation(format!("Invalid regular expression '{}': {e}", r.pattern)))?;
                    compiled.push(any(r.action, Matcher::Regex(re)));
                }
                FilterKind::Rclone => {
                    let bad = |e: regex::Error| AppError::validation(format!("Invalid pattern '{}': {e}", r.pattern));
                    let re = Regex::new(&rclone_to_regex(&r.pattern)?).map_err(bad)?;
                    // As in rclone: a pattern ending in `/` or using `**`
                    // also names folders; any other pattern names files
                    // only, and an include one opens the folders above it.
                    let p = r.pattern.trim();
                    let file_rule = !p.ends_with('/') && !p.contains("**");
                    let applies = if file_rule { Applies::Files } else { Applies::Any };
                    compiled.push(Compiled { action: r.action, matcher: Matcher::Regex(re), applies, relative: true });
                    if file_rule && r.action == FilterAction::Include {
                        for dir in rclone_parent_dirs(p) {
                            // Exactly that folder: the file-pattern regex
                            // with the folder's trailing `/` before `$`.
                            let Some(re) = rclone_to_regex(&dir).ok().and_then(|re| Regex::new(&format!("{}/$", re.strip_suffix('$')?)).ok()) else {
                                continue;
                            };
                            compiled.push(Compiled { action: r.action, matcher: Matcher::Regex(re), applies: Applies::Dirs, relative: true });
                        }
                    }
                }
                FilterKind::Group => {
                    let pats = group_patterns(&r.pattern).ok_or_else(|| AppError::validation(format!("Unknown filter group '{}'", r.pattern)))?;
                    for p in pats.iter() {
                        compiled.push(any(r.action, Matcher::Glob(compile_glob(p)?)));
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
        self.evaluate_in(paths, None, is_dir)
    }

    /// Like [`evaluate`](Self::evaluate) for an entry at absolute `path`
    /// found under the folder being walked, `rel` being its path inside that
    /// folder. rclone rules are matched on `rel` (so `/Photos/**` means the
    /// folder's own `Photos`, and they never filter the folder itself);
    /// every other rule on `path`. With `rel` `None` all rules see `path`.
    pub fn evaluate_rooted(&self, path: &str, rel: Option<&str>, is_dir: bool) -> Option<FilterAction> {
        let rel = rel.map(|r| format!("/{}", normalize(r).trim_start_matches('/')));
        self.evaluate_in(&[path], rel.as_deref(), is_dir)
    }

    fn evaluate_in(&self, paths: &[&str], rel: Option<&str>, is_dir: bool) -> Option<FilterAction> {
        let spell = |path: &str| {
            let mut p = normalize(path);
            if is_dir && !p.ends_with('/') {
                p.push('/');
            }
            p
        };
        let paths: Vec<String> = paths.iter().map(|p| spell(p)).collect();
        let rel: Option<Vec<String>> = rel.map(|r| if r == "/" { Vec::new() } else { vec![spell(r)] });
        let opts = match_options(self.case_sensitive);
        for c in &self.compiled {
            if (c.applies == Applies::Files && is_dir) || (c.applies == Applies::Dirs && !is_dir) {
                continue;
            }
            let paths = match &rel {
                Some(rel) if c.relative => rel,
                _ => &paths,
            };
            let (action, m) = (&c.action, &c.matcher);
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

/// The folders above an rclone pattern's last segment, deepest first:
/// `/Photos/2024/*.jpg` gives `/Photos/2024` and `/Photos`.
fn rclone_parent_dirs(pattern: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut g = pattern;
    while let Some(i) = g.rfind('/') {
        if i == 0 {
            break;
        }
        g = &g[..i];
        out.push(g.to_string());
    }
    out
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

/// Read a `ufop backup` `--include` / `--exclude` value: `[regex]`,
/// `{group}`, a wildcard (has `*` or `?`) or an exact path.
pub fn parse_cli_filter(spec: &str, action: FilterAction) -> FilterRule {
    let (kind, pattern) = if let Some(r) = spec.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        (FilterKind::Regex, r.to_string())
    } else if let Some(g) = spec.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        (FilterKind::Group, g.to_string())
    } else if spec.contains('*') || spec.contains('?') {
        (FilterKind::Wildcard, spec.to_string())
    } else {
        (FilterKind::Exact, spec.to_string())
    };
    FilterRule { action, kind, pattern }
}

/// Read a `ufop backup` `--filter` value: one rclone `+ pattern` /
/// `- pattern` line (an rclone glob, matched inside each source folder as
/// in the app).
pub fn parse_cli_filter_line(line: &str) -> Result<FilterRule, AppError> {
    match parse_filter_file(line)?.as_slice() {
        [rule] => Ok(rule.clone()),
        _ => Err(AppError::validation(format!("--filter \"{line}\" must be one \"+ pattern\" or \"- pattern\" rule"))),
    }
}

/// The rules of a `ufop backup` command line, in the order it applies
/// them: every `--include`, then every `--exclude`, then the `--filter`
/// lines in order.
pub fn cli_rules(include: &[String], exclude: &[String], filter: &[String]) -> Result<Vec<FilterRule>, AppError> {
    let mut rules: Vec<FilterRule> = include.iter().map(|s| parse_cli_filter(s, FilterAction::Include)).collect();
    rules.extend(exclude.iter().map(|s| parse_cli_filter(s, FilterAction::Exclude)));
    for line in filter {
        rules.push(parse_cli_filter_line(line)?);
    }
    Ok(rules)
}

/// The `ufop backup` command-line flag and value for `rule`, read back by
/// [`parse_cli_filter`] (`--include` / `--exclude`) or
/// [`parse_cli_filter_line`] (`--filter`, for rclone globs, which keep
/// their own syntax and their source-relative matching).
pub fn cli_filter_arg(rule: &FilterRule) -> (&'static str, String) {
    let flag = match rule.action {
        FilterAction::Include => "--include",
        FilterAction::Exclude => "--exclude",
    };
    match rule.kind {
        FilterKind::Regex => (flag, format!("[{}]", rule.pattern)),
        FilterKind::Group => (flag, format!("{{{}}}", rule.pattern)),
        FilterKind::Rclone => {
            let sign = if rule.action == FilterAction::Include { '+' } else { '-' };
            ("--filter", format!("{sign} {}", rule.pattern))
        }
        FilterKind::Exact | FilterKind::Wildcard => (flag, rule.pattern.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(action: FilterAction, kind: FilterKind, p: &str) -> FilterRule {
        FilterRule { action, kind, pattern: p.into() }
    }

    #[test]
    fn cli_filter_args_read_back_as_the_same_rules() {
        let rules = vec![
            rule(FilterAction::Include, FilterKind::Exact, "/a/b"),
            rule(FilterAction::Exclude, FilterKind::Wildcard, "*.log"),
            rule(FilterAction::Exclude, FilterKind::Regex, "\\.tmp$"),
            rule(FilterAction::Exclude, FilterKind::Group, "cache_files"),
            rule(FilterAction::Include, FilterKind::Rclone, "/Photos/**"),
            rule(FilterAction::Exclude, FilterKind::Rclone, "*"),
        ];
        let (mut inc, mut exc, mut fil) = (vec![], vec![], vec![]);
        for r in &rules {
            let (flag, value) = cli_filter_arg(r);
            match flag {
                "--include" => inc.push(value),
                "--exclude" => exc.push(value),
                _ => fil.push(value),
            }
        }
        assert_eq!(fil, vec!["+ /Photos/**".to_string(), "- *".to_string()]);
        assert_eq!(cli_rules(&inc, &exc, &fil).unwrap(), rules);
        assert!(parse_cli_filter_line("+ a\n- b").is_err());
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
    fn rclone_rules_match_inside_the_walked_folder() {
        let set = RuleSet::compile(&parse_filter_file("+ /Photos/2024/*.jpg\n- *").unwrap()).unwrap();
        let at = |abs: &str, rel: &str, dir| set.evaluate_rooted(abs, Some(rel), dir);
        assert_eq!(at("/home/u/src/Photos/2024/a.jpg", "Photos/2024/a.jpg", false), Some(FilterAction::Include));
        // The folders on the way are opened; others are left to the walk.
        assert_eq!(at("/home/u/src/Photos", "Photos", true), Some(FilterAction::Include));
        assert_eq!(at("/home/u/src/Photos/2024", "Photos/2024", true), Some(FilterAction::Include));
        assert_eq!(at("/home/u/src/Music", "Music", true), None);
        assert_eq!(at("/home/u/src/Music/b.mp3", "Music/b.mp3", false), Some(FilterAction::Exclude));
        // The walked folder itself is never filtered by rclone rules.
        assert_eq!(at("/home/u/src", "", true), None);
        // Other kinds keep matching the full path.
        let set = RuleSet::compile(&[rule(FilterAction::Exclude, FilterKind::Exact, "/home/u/src/a")]).unwrap();
        assert_eq!(set.evaluate_rooted("/home/u/src/a", Some("a"), false), Some(FilterAction::Exclude));
        assert_eq!(rclone_parent_dirs("/Photos/2024/*.jpg"), vec!["/Photos/2024", "/Photos"]);
        assert_eq!(rclone_parent_dirs("docs/*.txt"), vec!["docs"]);
        assert!(rclone_parent_dirs("*.jpg").is_empty());
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
