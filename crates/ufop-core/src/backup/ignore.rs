//! Ignore files (`.ufopignore`, optionally `.gitignore` and
//! `.git/info/exclude`) with gitignore semantics.
//!
//! Each file's rules apply to the folder it is in and everything below it.
//! Patterns follow `gitignore(5)`:
//!
//! - blank lines and lines starting with `#` are skipped (`\#` is a literal `#`);
//!   trailing spaces are dropped unless escaped (`\ `);
//! - `!pattern` re-includes what an earlier pattern excluded (`\!` is a literal `!`);
//! - a trailing `/` matches folders only;
//! - a pattern with a `/` at the start or in the middle is anchored to the
//!   ignore file's folder; otherwise it matches a name at any depth;
//! - `*` and `?` never match `/`, `[a-z]`/`[!a-z]` are character classes,
//!   `\x` escapes `x`; a leading `**/` matches in every folder, a trailing
//!   `/**` matches everything inside, `/**/` matches zero or more folders.
//!
//! Precedence: within one folder, `.git/info/exclude` comes first, then
//! `.gitignore`, then the configured ignore files in order; a later line
//! overrides an earlier one. A file deeper in the tree overrides the files
//! of the folders above it. A file inside an excluded folder is never
//! read, so (as in git) nothing below an excluded folder can be re-included.

use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default ignore-file name.
pub const DEFAULT_IGNORE_FILE: &str = ".ufopignore";

/// One compiled pattern.
#[derive(Debug, Clone)]
pub struct IgnoreRule {
    pub pattern: String,
    pub negated: bool,
    pub dir_only: bool,
    re: Regex,
}

impl IgnoreRule {
    /// Whether the rule matches `rel` (relative to the ignore file's folder,
    /// `/`-separated).
    pub fn matches(&self, rel: &str, is_dir: bool) -> bool {
        (!self.dir_only || is_dir) && self.re.is_match(rel)
    }
}

/// Parse an ignore file. Invalid patterns (an unclosed `[`) are skipped,
/// like git does.
pub fn parse(text: &str) -> Vec<IgnoreRule> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<IgnoreRule> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = trim_trailing_spaces(line);
    let (negated, body) = match line.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let (dir_only, body) = match body.strip_suffix('/') {
        Some(b) => (true, b),
        None => (false, body),
    };
    if body.is_empty() {
        return None;
    }
    let anchored = contains_unescaped_slash(body);
    let body = body.strip_prefix('/').unwrap_or(body);
    let re = glob_to_regex(body, anchored)?;
    Some(IgnoreRule { pattern: line.to_string(), negated, dir_only, re })
}

fn trim_trailing_spaces(s: &str) -> &str {
    let mut end = s.len();
    let b = s.as_bytes();
    while end > 0 && b[end - 1] == b' ' {
        // An escaped space stays.
        let backslashes = b[..end - 1].iter().rev().take_while(|c| **c == b'\\').count();
        if backslashes % 2 == 1 {
            break;
        }
        end -= 1;
    }
    &s[..end]
}

fn contains_unescaped_slash(s: &str) -> bool {
    let mut esc = false;
    for c in s.chars() {
        match c {
            _ if esc => esc = false,
            '\\' => esc = true,
            '/' => return true,
            _ => {}
        }
    }
    false
}

/// Translate a gitignore glob into an anchored regular expression.
fn glob_to_regex(glob: &str, anchored: bool) -> Option<Regex> {
    let chars: Vec<char> = glob.chars().collect();
    let mut re = String::from("^");
    if !anchored {
        re.push_str("(?:.*/)?");
    }
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                let at_start = i == 0 || chars[i - 1] == '/';
                let next = chars.get(i + 2);
                if at_start && next == Some(&'/') {
                    // `**/` (leading or `/**/`): zero or more folders.
                    re.push_str("(?:.*/)?");
                    i += 3;
                } else if at_start && next.is_none() {
                    // Trailing `/**` (or a lone `**`): everything inside.
                    re.push_str(".*");
                    i += 2;
                } else {
                    // Any other `**` is an ordinary `*`.
                    re.push_str("[^/]*");
                    i += 2;
                }
                continue;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '[' => {
                let (class, used) = char_class(&chars[i..])?;
                re.push_str(&class);
                i += used;
                continue;
            }
            '\\' => {
                i += 1;
                if let Some(e) = chars.get(i) {
                    re.push_str(&regex::escape(&e.to_string()));
                }
            }
            _ => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    regex::RegexBuilder::new(&re).case_insensitive(cfg!(windows)).build().ok()
}

/// `[...]` at the start of `chars` → (regex class, characters consumed).
fn char_class(chars: &[char]) -> Option<(String, usize)> {
    let mut i = 1;
    let mut out = String::from("[");
    if matches!(chars.get(i), Some('!') | Some('^')) {
        out.push('^');
        i += 1;
    }
    let start = i;
    loop {
        let c = *chars.get(i)?;
        if c == ']' && i > start {
            break;
        }
        match c {
            '\\' => {
                i += 1;
                let e = *chars.get(i)?;
                out.push('\\');
                out.push(e);
            }
            '-' => out.push('-'),
            '[' | ']' | '^' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    if out.starts_with("[^") {
        // `/` never matches a class.
        out.push('/');
    }
    out.push(']');
    Some((out, i + 1))
}

/// The rules that apply in one folder, in increasing precedence.
#[derive(Debug, Default)]
pub struct IgnoreFrame {
    pub rules: Vec<IgnoreRule>,
}

impl IgnoreFrame {
    /// `Some(true)` = ignored, `Some(false)` = re-included, `None` = no rule matched.
    pub fn decide(&self, rel: &str, is_dir: bool) -> Option<bool> {
        self.rules.iter().rev().find(|r| r.matches(rel, is_dir)).map(|r| !r.negated)
    }
}

/// Which ignore files are read.
#[derive(Debug, Clone, Default)]
pub struct IgnoreConfig {
    /// File names read in every folder (e.g. `.ufopignore`).
    pub files: Vec<String>,
    /// Also `.gitignore`, and `.git/info/exclude` at repository roots.
    pub gitignore: bool,
}

impl IgnoreConfig {
    pub fn is_active(&self) -> bool {
        self.gitignore || self.files.iter().any(|f| !f.trim().is_empty())
    }
}

/// Lazily loaded ignore rules for the folders of one scan.
///
/// `read` maps a logical path to where it can be read (a snapshot), so the
/// scanner keeps logical paths while the rules come from the snapshot.
pub struct IgnoreMatcher<'a> {
    cfg: IgnoreConfig,
    roots: Vec<PathBuf>,
    read: Box<dyn Fn(&Path) -> PathBuf + 'a>,
    frames: HashMap<PathBuf, Option<Arc<IgnoreFrame>>>,
}

impl<'a> IgnoreMatcher<'a> {
    pub fn new(cfg: IgnoreConfig, read: impl Fn(&Path) -> PathBuf + 'a) -> Self {
        Self { cfg, roots: Vec::new(), read: Box::new(read), frames: HashMap::new() }
    }

    /// Ignore files are only read inside source roots (and at the roots).
    pub fn add_root(&mut self, root: &Path) {
        if !self.roots.iter().any(|r| r == root) {
            self.roots.push(root.to_path_buf());
            self.roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
        }
    }

    pub fn is_active(&self) -> bool {
        self.cfg.is_active()
    }

    fn frame(&mut self, dir: &Path) -> Option<Arc<IgnoreFrame>> {
        if let Some(f) = self.frames.get(dir) {
            return f.clone();
        }
        let real = (self.read)(dir);
        let mut rules = Vec::new();
        let mut load = |p: PathBuf| {
            if let Ok(text) = std::fs::read_to_string(&p) {
                rules.extend(parse(&text));
            }
        };
        if self.cfg.gitignore {
            if real.join(".git").is_dir() {
                load(real.join(".git").join("info").join("exclude"));
            }
            load(real.join(".gitignore"));
        }
        for name in self.cfg.files.iter().map(|f| f.trim()).filter(|f| !f.is_empty()) {
            if !(self.cfg.gitignore && name == ".gitignore") {
                load(real.join(name));
            }
        }
        let f = (!rules.is_empty()).then(|| Arc::new(IgnoreFrame { rules }));
        self.frames.insert(dir.to_path_buf(), f.clone());
        f
    }

    /// Whether an entry is excluded by the ignore files above it.
    pub fn is_ignored(&mut self, path: &Path, is_dir: bool) -> bool {
        if !self.is_active() {
            return false;
        }
        let Some(root) = self.roots.iter().find(|r| path.starts_with(r) && path != r.as_path()).cloned() else { return false };
        // Deepest folder first: the first file with a matching rule decides.
        let mut dir = path.parent();
        while let Some(d) = dir {
            if let Some(frame) = self.frame(d) {
                if let Ok(rel) = path.strip_prefix(d) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if let Some(ignored) = frame.decide(&rel, is_dir) {
                        return ignored;
                    }
                }
            }
            if d == root.as_path() {
                break;
            }
            dir = d.parent();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(text: &str) -> IgnoreFrame {
        IgnoreFrame { rules: parse(text) }
    }

    #[test]
    fn basename_and_anchored_patterns() {
        let f = frame("*.log\n/build\ndocs/*.tmp\n");
        assert_eq!(f.decide("a.log", false), Some(true));
        assert_eq!(f.decide("deep/er/a.log", false), Some(true));
        assert_eq!(f.decide("build", true), Some(true));
        // Anchored: only at the top.
        assert_eq!(f.decide("src/build", true), None);
        assert_eq!(f.decide("docs/x.tmp", false), Some(true));
        assert_eq!(f.decide("a/docs/x.tmp", false), None);
        // `*` does not cross folders.
        assert_eq!(f.decide("docs/sub/x.tmp", false), None);
    }

    #[test]
    fn double_star_forms() {
        let f = frame("**/logs\nfoo/**\na/**/b\n");
        assert_eq!(f.decide("logs", true), Some(true));
        assert_eq!(f.decide("x/y/logs", false), Some(true));
        assert_eq!(f.decide("foo/x/y", false), Some(true));
        assert_eq!(f.decide("foo", true), None);
        assert_eq!(f.decide("a/b", false), Some(true));
        assert_eq!(f.decide("a/x/y/b", false), Some(true));
        assert_eq!(f.decide("a/xb", false), None);
    }

    #[test]
    fn negation_and_directory_only() {
        let f = frame("*.txt\n!keep.txt\ncache/\n");
        assert_eq!(f.decide("a.txt", false), Some(true));
        assert_eq!(f.decide("sub/keep.txt", false), Some(false));
        assert_eq!(f.decide("cache", true), Some(true));
        assert_eq!(f.decide("cache", false), None, "a file named cache is not a folder");
        // A later line wins over an earlier one.
        let f = frame("!x\nx\n");
        assert_eq!(f.decide("x", false), Some(true));
    }

    #[test]
    fn escapes_comments_classes_and_spaces() {
        let f = frame("# comment\n\\#hash\n\\!bang\nspace\\ \ntrail   \nfile[0-9].c\nn[!a]t\n");
        assert_eq!(f.rules.len(), 6);
        assert_eq!(f.decide("#hash", false), Some(true));
        assert_eq!(f.decide("!bang", false), Some(true));
        assert_eq!(f.decide("space ", false), Some(true));
        assert_eq!(f.decide("trail", false), Some(true));
        assert_eq!(f.decide("file7.c", false), Some(true));
        assert_eq!(f.decide("filex.c", false), None);
        assert_eq!(f.decide("nut", false), Some(true));
        assert_eq!(f.decide("nat", false), None);
        assert_eq!(f.decide("a?b", false), None);
        // `?` never matches a separator.
        assert_eq!(frame("a?b").decide("a/b", false), None);
        // Unclosed class: the line is skipped.
        assert!(parse("bad[").is_empty());
    }

    #[test]
    fn deeper_files_override_and_scope_to_subtree() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        crate::backup::test_util::write(root, ".ufopignore", b"*.log\nsecret/\n");
        crate::backup::test_util::write(root, "app/.ufopignore", b"!important.log\n");
        crate::backup::test_util::write(root, "other/.ufopignore", b"*.md\n");
        let mut m = IgnoreMatcher::new(IgnoreConfig { files: vec![DEFAULT_IGNORE_FILE.into()], gitignore: false }, |p| p.to_path_buf());
        m.add_root(root);
        assert!(m.is_ignored(&root.join("a.log"), false));
        assert!(m.is_ignored(&root.join("app/debug.log"), false));
        assert!(!m.is_ignored(&root.join("app/important.log"), false));
        assert!(m.is_ignored(&root.join("x/secret"), true));
        // other/.ufopignore only applies inside other/.
        assert!(m.is_ignored(&root.join("other/readme.md"), false));
        assert!(!m.is_ignored(&root.join("readme.md"), false));
        // The root itself is never ignored.
        assert!(!m.is_ignored(root, true));
    }

    #[test]
    fn gitignore_and_info_exclude() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        crate::backup::test_util::write(root, "repo/.git/info/exclude", b"*.o\nlocal.cfg\n");
        crate::backup::test_util::write(root, "repo/.gitignore", b"target/\n!keep.o\n");
        let gi = |gitignore| IgnoreConfig { files: vec![DEFAULT_IGNORE_FILE.into()], gitignore };
        let mut m = IgnoreMatcher::new(gi(true), |p| p.to_path_buf());
        m.add_root(root);
        assert!(m.is_ignored(&root.join("repo/a.o"), false));
        // .gitignore overrides .git/info/exclude.
        assert!(!m.is_ignored(&root.join("repo/src/keep.o"), false));
        assert!(m.is_ignored(&root.join("repo/local.cfg"), false));
        assert!(m.is_ignored(&root.join("repo/target"), true));
        let mut off = IgnoreMatcher::new(gi(false), |p| p.to_path_buf());
        off.add_root(root);
        assert!(!off.is_ignored(&root.join("repo/a.o"), false));
    }
}
