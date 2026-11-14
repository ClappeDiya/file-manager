//! Batch-rename pipeline shared by the desktop app and `ufop rename`:
//! token pattern → find/replace (literal or regex) → case transform, with
//! cross-platform name warnings.

use super::tokens::{self, render, sentence_case, title_case, TokenContext, UnknownTokens};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What to do to every file name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenameSpec {
    /// Token template (`{name}_{num:3}.{ext}`); see [`tokens`].
    pub pattern: Option<String>,
    pub find: Option<String>,
    /// Replacement for `find`; regex mode supports `$1` / `${name}`.
    pub replace: Option<String>,
    pub use_regex: bool,
    /// `upper`, `lower`, `title` or `sentence`.
    pub case_transform: Option<String>,
    pub counter_start: u64,
    pub counter_step: u64,
    pub counter_pad: usize,
}

/// The planned new name of one file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenamePlanEntry {
    pub original_path: String,
    pub original_name: String,
    pub new_name: String,
    pub changed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RenameError {
    #[error("Invalid regular expression: {0}")]
    Regex(String),
    #[error("Unknown case transform '{0}'. Use: upper, lower, title, sentence")]
    Case(String),
    #[error("Invalid pattern: {0}")]
    Template(#[from] tokens::TemplateError),
}

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Problems a name would have on common filesystems.
pub fn compat_warnings(name: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if name.is_empty() {
        warnings.push("Resulting filename would be empty".into());
        return warnings;
    }
    let stem_upper = name.split('.').next().unwrap_or("").to_uppercase();
    if WINDOWS_RESERVED.contains(&stem_upper.as_str()) {
        warnings.push(format!("'{name}' is a reserved name on Windows"));
    }
    for ch in ['<', '>', ':', '"', '|', '?', '*'] {
        if name.contains(ch) {
            warnings.push(format!("Character '{ch}' is invalid on Windows"));
        }
    }
    if name.contains('/') || name.contains('\\') {
        warnings.push("Names cannot contain path separators".into());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        warnings.push("Trailing dot/space not allowed on Windows".into());
    }
    if name.len() > 255 {
        warnings.push(format!("Filename exceeds 255 characters ({} chars)", name.len()));
    }
    warnings
}

/// Apply `spec` to `name` (already token-rendered), find/replace and case.
fn transform(mut name: String, spec: &RenameSpec, re: Option<&regex::Regex>) -> Result<String, RenameError> {
    if let Some(find) = spec.find.as_deref().filter(|f| !f.is_empty()) {
        let replacement = spec.replace.as_deref().unwrap_or("");
        name = match re {
            Some(re) => re.replace_all(&name, replacement).to_string(),
            None => name.replace(find, replacement),
        };
    }
    if let Some(case) = spec.case_transform.as_deref().filter(|c| !c.is_empty()) {
        name = match case.to_lowercase().as_str() {
            "upper" => name.to_uppercase(),
            "lower" => name.to_lowercase(),
            "title" => title_case(&name),
            "sentence" => sentence_case(&name),
            other => return Err(RenameError::Case(other.to_string())),
        };
    }
    Ok(name)
}

/// Plan new names for `files` in order (the counter advances per file).
pub fn plan<P: AsRef<Path>>(files: &[P], spec: &RenameSpec) -> Result<Vec<RenamePlanEntry>, RenameError> {
    let re = match (&spec.find, spec.use_regex) {
        (Some(f), true) if !f.is_empty() => Some(regex::Regex::new(f).map_err(|e| RenameError::Regex(e.to_string()))?),
        _ => None,
    };
    let step = if spec.counter_step == 0 { 1 } else { spec.counter_step };
    let mut counter = spec.counter_start;
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let path = file.as_ref();
        let ctx = TokenContext::for_path(path).with_counter(counter, spec.counter_pad);
        let original = ctx.file_name.clone();
        let mut name = original.clone();
        if let Some(pattern) = spec.pattern.as_deref().filter(|p| !p.is_empty()) {
            name = render(pattern, &ctx, UnknownTokens::Keep)?;
            // A pattern without a dot keeps the original extension.
            if !name.contains('.') && !ctx.ext.is_empty() {
                name = format!("{name}.{}", ctx.ext);
            }
        }
        let name = transform(name, spec, re.as_ref())?;
        out.push(RenamePlanEntry {
            original_path: path.to_string_lossy().to_string(),
            changed: name != original,
            warnings: compat_warnings(&name),
            original_name: original,
            new_name: name,
        });
        counter += step;
    }
    Ok(out)
}

/// Plan entries whose new name collides with another entry's new name or
/// with an existing, unrelated file in the same folder.
pub fn collisions(entries: &[RenamePlanEntry]) -> Vec<String> {
    use std::collections::HashMap;
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();
    let originals: std::collections::HashSet<PathBuf> = entries.iter().map(|e| PathBuf::from(&e.original_path)).collect();
    let mut problems = Vec::new();
    for e in entries.iter().filter(|e| e.changed) {
        let target = Path::new(&e.original_path).with_file_name(&e.new_name);
        if let Some(_prev) = seen.insert(target.clone(), 1) {
            problems.push(format!("Two files would be named '{}'", e.new_name));
        } else if target.exists() && !originals.contains(&target) {
            problems.push(format!("'{}' already exists", e.new_name));
        }
    }
    problems
}

/// Result of [`apply`]: `(from, to)` for every rename made.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApplyOutcome {
    pub renamed: Vec<(String, String)>,
    pub errors: Vec<String>,
}

/// Rename every changed entry, safely:
///
/// - refuses the whole batch if two entries target the same name or a
///   target exists outside the batch (`rename` would overwrite it);
/// - renames in two phases (each file to a unique temporary name, then to
///   its target), so chains and swaps (`a→b`, `b→a`) work;
/// - on any failure, moves everything already renamed back.
pub fn apply(entries: &[RenamePlanEntry]) -> Result<ApplyOutcome, String> {
    let problems = collisions(entries);
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let changed: Vec<&RenamePlanEntry> = entries.iter().filter(|e| e.changed).collect();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // (original, temp, target)

    let rollback = |staged: &[(PathBuf, PathBuf, PathBuf)], moved_to_target: usize| {
        for (i, (orig, temp, target)) in staged.iter().enumerate().rev() {
            let from = if i < moved_to_target { target } else { temp };
            let _ = std::fs::rename(from, orig);
        }
    };

    for (i, e) in changed.iter().enumerate() {
        let orig = PathBuf::from(&e.original_path);
        let temp = orig.with_file_name(format!(".ufop-rename-{token}-{i}"));
        let target = orig.with_file_name(&e.new_name);
        if let Err(err) = std::fs::rename(&orig, &temp) {
            rollback(&staged, 0);
            return Err(format!("{}: {err}", e.original_name));
        }
        staged.push((orig, temp, target));
    }
    for (i, (_, temp, target)) in staged.iter().enumerate() {
        if target.exists() {
            rollback(&staged, i);
            return Err(format!("'{}' appeared while renaming; nothing was changed", target.display()));
        }
        if let Err(err) = std::fs::rename(temp, target) {
            rollback(&staged, i);
            return Err(format!("{}: {err}", target.display()));
        }
    }
    Ok(ApplyOutcome {
        renamed: staged
            .into_iter()
            .map(|(o, _, t)| (o.to_string_lossy().to_string(), t.to_string_lossy().to_string()))
            .collect(),
        errors: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(names: &[&str]) -> (PathBuf, Vec<PathBuf>) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ufop-rename-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = names
            .iter()
            .map(|n| {
                let p = dir.join(n);
                std::fs::write(&p, b"x").unwrap();
                p
            })
            .collect();
        (dir, files)
    }

    #[test]
    fn pattern_counter_and_extension_rules() {
        let (dir, files) = dir_with(&["a.jpg", "b.jpg"]);
        let spec = RenameSpec { pattern: Some("photo_{num}".into()), counter_start: 1, counter_step: 1, counter_pad: 3, ..Default::default() };
        let plan = plan(&files, &spec).unwrap();
        assert_eq!(plan[0].new_name, "photo_001.jpg");
        assert_eq!(plan[1].new_name, "photo_002.jpg");
        assert!(plan.iter().all(|e| e.changed && e.warnings.is_empty()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn find_replace_and_case() {
        let (dir, files) = dir_with(&["IMG_2026 trip.JPG"]);
        let spec = RenameSpec {
            find: Some(r"IMG_(\d+)".into()),
            replace: Some("$1-shot".into()),
            use_regex: true,
            case_transform: Some("lower".into()),
            ..Default::default()
        };
        assert_eq!(plan(&files, &spec).unwrap()[0].new_name, "2026-shot trip.jpg");
        let literal = RenameSpec { find: Some(" ".into()), replace: Some("_".into()), ..Default::default() };
        assert_eq!(plan(&files, &literal).unwrap()[0].new_name, "IMG_2026_trip.JPG");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn invalid_inputs_are_errors_not_silent_fallbacks() {
        let (dir, files) = dir_with(&["x.txt"]);
        let bad_re = RenameSpec { find: Some("(".into()), use_regex: true, ..Default::default() };
        assert!(matches!(plan(&files, &bad_re), Err(RenameError::Regex(_))));
        let bad_case = RenameSpec { case_transform: Some("shout".into()), ..Default::default() };
        assert_eq!(plan(&files, &bad_case), Err(RenameError::Case("shout".into())));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn warnings_and_collisions() {
        assert!(compat_warnings("CON.txt")[0].contains("reserved"));
        assert!(compat_warnings("a:b").iter().any(|w| w.contains("':'")));
        assert!(compat_warnings("").iter().any(|w| w.contains("empty")));
        let (dir, files) = dir_with(&["one.txt", "two.txt", "taken.txt"]);
        let spec = RenameSpec { pattern: Some("same".into()), ..Default::default() };
        let p = plan(&files[..2], &spec).unwrap();
        assert_eq!(collisions(&p), vec!["Two files would be named 'same.txt'".to_string()]);
        let to_taken = RenameSpec { pattern: Some("taken".into()), ..Default::default() };
        assert_eq!(collisions(&plan(&files[..1], &to_taken).unwrap()), vec!["'taken.txt' already exists".to_string()]);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Regression: renaming onto an existing file overwrote it.
    #[test]
    fn apply_never_overwrites_and_handles_chains() {
        let (dir, files) = dir_with(&["a.txt", "b.txt", "keep.txt"]);
        std::fs::write(&files[0], b"A").unwrap();
        std::fs::write(&files[1], b"B").unwrap();
        std::fs::write(&files[2], b"KEEP").unwrap();

        // a→keep would clobber keep.txt: the whole batch is refused.
        let clobber = plan(&files[..1], &RenameSpec { pattern: Some("keep".into()), ..Default::default() }).unwrap();
        assert!(apply(&clobber).unwrap_err().contains("already exists"));
        assert_eq!(std::fs::read(&files[2]).unwrap(), b"KEEP");
        assert!(files[0].exists());

        // Swap a ↔ b in one batch.
        let mut swap = plan(&files[..2], &RenameSpec::default()).unwrap();
        swap[0].new_name = "b.txt".into();
        swap[0].changed = true;
        swap[1].new_name = "a.txt".into();
        swap[1].changed = true;
        let out = apply(&swap).unwrap();
        assert_eq!(out.renamed.len(), 2);
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"B");
        assert_eq!(std::fs::read(dir.join("b.txt")).unwrap(), b"A");
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".ufop-rename")));
        std::fs::remove_dir_all(dir).ok();
    }
}
