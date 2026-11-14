//! Batch validation: does a delivered set of files have the members it
//! should, and do their contents look right?
//!
//! A batch is the files directly inside one folder. **Member rules** say
//! which files must be there (a name pattern, how many, how large).
//! **Content rules** check files matching a pattern: not empty, valid
//! JSON, CSV with required columns and a minimum number of rows, or
//! SHA-256 sums listed in a manifest (`sha256sum` format). The outcome is a
//! [`ValidationReport`], which the `validate_batch` action can write next
//! to the batch and use to fail the step (so a workflow can quarantine the
//! batch or notify someone).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Largest file read for a content check.
const MAX_CHECK_BYTES: u64 = 256 * 1024 * 1024;

/// Files matching `pattern` (case-insensitive glob on the file name) must
/// number `min..=max` and each be at least `min_bytes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemberRule {
    pub pattern: String,
    #[serde(default = "one")]
    pub min: u32,
    #[serde(default)]
    pub max: Option<u32>,
    #[serde(default)]
    pub min_bytes: Option<u64>,
}

fn one() -> u32 {
    1
}

/// A check on the contents of files matching `pattern`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContentRule {
    pub pattern: String,
    pub check: ContentCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentCheck {
    NotEmpty,
    Json,
    Csv {
        /// Header names that must be present (case-insensitive).
        #[serde(default)]
        required_columns: Vec<String>,
        /// Data rows (after the header).
        #[serde(default)]
        min_rows: Option<u64>,
        /// Default `,`.
        #[serde(default)]
        delimiter: Option<String>,
    },
    /// The file is a manifest of `<sha256>  <name>` lines; every listed
    /// file must exist in the batch and match.
    Sha256Manifest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BatchRules {
    #[serde(default)]
    pub members: Vec<MemberRule>,
    #[serde(default)]
    pub content: Vec<ContentRule>,
    /// Files no member rule names are a failure.
    #[serde(default)]
    pub no_extra_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    /// The rule (`members: *.csv`, `csv: orders_*.csv`…).
    pub rule: String,
    /// The file it is about, when there is one.
    pub file: Option<String>,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationReport {
    pub folder: String,
    pub checked_at: String,
    pub ok: bool,
    pub files: usize,
    pub findings: Vec<Finding>,
}

impl ValidationReport {
    pub fn failures(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| !f.ok)
    }

    /// One line for the run history.
    pub fn summary(&self) -> String {
        let bad: Vec<String> = self.failures().map(|f| f.message.clone()).collect();
        if bad.is_empty() {
            format!("Batch valid: {} files, {} checks passed", self.files, self.findings.len())
        } else {
            format!("Batch invalid ({} of {} checks failed): {}", bad.len(), self.findings.len(), bad.join("; "))
        }
    }

    /// Human-readable report (written as `.txt` beside the JSON one).
    pub fn to_text(&self) -> String {
        let mut s = format!("Validation of {}\nChecked {}\nResult: {}\n\n", self.folder, self.checked_at, if self.ok { "VALID" } else { "INVALID" });
        for f in &self.findings {
            s.push_str(&format!("[{}] {}{}: {}\n", if f.ok { "ok" } else { "FAIL" }, f.rule, f.file.as_deref().map(|n| format!(" ({n})")).unwrap_or_default(), f.message));
        }
        s
    }
}

impl BatchRules {
    pub fn validate(&self) -> Result<(), String> {
        if self.members.is_empty() && self.content.is_empty() && !self.no_extra_files {
            return Err("Add at least one member or content rule".into());
        }
        for p in self.members.iter().map(|m| &m.pattern).chain(self.content.iter().map(|c| &c.pattern)) {
            matcher(p)?;
        }
        for m in &self.members {
            if m.max.is_some_and(|max| max < m.min) {
                return Err(format!("'{}': the most files allowed is below the fewest required", m.pattern));
            }
        }
        Ok(())
    }
}

fn matcher(pattern: &str) -> Result<glob::Pattern, String> {
    if pattern.trim().is_empty() || pattern.contains(['/', '\\']) {
        return Err(format!("'{pattern}' must be a file name pattern like *.csv"));
    }
    glob::Pattern::new(&pattern.to_lowercase()).map_err(|e| format!("'{pattern}' is not a valid pattern: {e}"))
}

fn matches(p: &glob::Pattern, name: &str) -> bool {
    p.matches(&name.to_lowercase())
}

/// Validate the files directly inside `folder`.
pub fn validate_batch(folder: &Path, rules: &BatchRules) -> Result<ValidationReport, String> {
    rules.validate()?;
    let mut files: Vec<(String, PathBuf, u64)> = std::fs::read_dir(folder)
        .map_err(|e| format!("Cannot read {}: {e}", folder.display()))?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            // Our own reports and hidden files aren't batch members.
            (m.is_file() && !name.starts_with('.') && !name.ends_with(".validation.json") && !name.ends_with(".validation.txt")).then(|| (name, e.path(), m.len()))
        })
        .collect();
    files.sort();
    let mut findings = Vec::new();

    for m in &rules.members {
        let p = matcher(&m.pattern)?;
        let hits: Vec<&(String, PathBuf, u64)> = files.iter().filter(|(n, ..)| matches(&p, n)).collect();
        let n = hits.len() as u32;
        let count_ok = n >= m.min && m.max.map_or(true, |max| n <= max);
        let wanted = match m.max {
            Some(max) if max == m.min => format!("{max}"),
            Some(max) => format!("{}–{max}", m.min),
            None => format!("at least {}", m.min),
        };
        findings.push(Finding { rule: format!("members: {}", m.pattern), file: None, ok: count_ok, message: format!("{n} file(s) match {} (expected {wanted})", m.pattern) });
        if let Some(min_bytes) = m.min_bytes {
            for (name, _, size) in &hits {
                if *size < min_bytes {
                    findings.push(Finding { rule: format!("members: {}", m.pattern), file: Some(name.clone()), ok: false, message: format!("{name} is {size} bytes, below {min_bytes}") });
                }
            }
        }
    }
    if rules.no_extra_files {
        let patterns: Vec<glob::Pattern> = rules.members.iter().map(|m| matcher(&m.pattern)).collect::<Result<_, _>>()?;
        for (name, ..) in files.iter().filter(|(n, ..)| !patterns.iter().any(|p| matches(p, n))) {
            findings.push(Finding { rule: "no extra files".into(), file: Some(name.clone()), ok: false, message: format!("{name} is not an expected member") });
        }
    }

    for c in &rules.content {
        let p = matcher(&c.pattern)?;
        let hits: Vec<&(String, PathBuf, u64)> = files.iter().filter(|(n, ..)| matches(&p, n)).collect();
        if hits.is_empty() {
            findings.push(Finding { rule: format!("{}: {}", check_name(&c.check), c.pattern), file: None, ok: false, message: format!("No file matches {} to check", c.pattern) });
        }
        for (name, path, size) in hits {
            let (ok, message) = match check(&c.check, path, *size, folder) {
                Ok(m) => (true, m),
                Err(m) => (false, m),
            };
            findings.push(Finding { rule: format!("{}: {}", check_name(&c.check), c.pattern), file: Some(name.clone()), ok, message: format!("{name}: {message}") });
        }
    }

    Ok(ValidationReport {
        folder: folder.to_string_lossy().into_owned(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        ok: findings.iter().all(|f| f.ok),
        files: files.len(),
        findings,
    })
}

fn check_name(c: &ContentCheck) -> &'static str {
    match c {
        ContentCheck::NotEmpty => "not empty",
        ContentCheck::Json => "json",
        ContentCheck::Csv { .. } => "csv",
        ContentCheck::Sha256Manifest => "sha256 manifest",
    }
}

fn read(path: &Path, size: u64) -> Result<Vec<u8>, String> {
    if size > MAX_CHECK_BYTES {
        return Err(format!("larger than {} MB, not checked", MAX_CHECK_BYTES / 1024 / 1024));
    }
    std::fs::read(path).map_err(|e| format!("cannot be read: {e}"))
}

/// `Ok(description)` when the file passes.
fn check(c: &ContentCheck, path: &Path, size: u64, folder: &Path) -> Result<String, String> {
    match c {
        ContentCheck::NotEmpty => (size > 0).then(|| format!("{size} bytes")).ok_or_else(|| "is empty".into()),
        ContentCheck::Json => {
            let data = read(path, size)?;
            serde_json::from_slice::<serde_json::Value>(&data).map(|_| "valid JSON".to_string()).map_err(|e| format!("is not valid JSON ({e})"))
        }
        ContentCheck::Csv { required_columns, min_rows, delimiter } => {
            let data = read(path, size)?;
            let delim = match delimiter.as_deref().unwrap_or(",") {
                "\\t" | "tab" => b'\t',
                d if d.len() == 1 => d.as_bytes()[0],
                d => return Err(format!("the delimiter '{d}' must be one character")),
            };
            let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data);
            let mut r = csv::ReaderBuilder::new().delimiter(delim).flexible(false).from_reader(data);
            let headers: Vec<String> = r.headers().map_err(|e| format!("has no readable header ({e})"))?.iter().map(|h| h.trim().to_lowercase()).collect();
            let missing: Vec<&String> = required_columns.iter().filter(|c| !headers.contains(&c.trim().to_lowercase())).collect();
            if !missing.is_empty() {
                return Err(format!("is missing column(s) {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let mut rows = 0u64;
            for rec in r.records() {
                rec.map_err(|e| format!("has a malformed row ({e})"))?;
                rows += 1;
            }
            if let Some(min) = min_rows.filter(|m| rows < *m) {
                return Err(format!("has {rows} data row(s), expected at least {min}"));
            }
            Ok(format!("{rows} rows, {} columns", headers.len()))
        }
        ContentCheck::Sha256Manifest => {
            use sha2::Digest;
            let data = read(path, size)?;
            let text = String::from_utf8_lossy(&data);
            let mut checked = 0;
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let (hash, name) = line.split_once(char::is_whitespace).ok_or_else(|| format!("has a line that isn't '<sha256>  <name>': {line}"))?;
                let name = name.trim().trim_start_matches('*');
                if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("lists '{name}' with an invalid SHA-256"));
                }
                if name.contains(['/', '\\']) || name == ".." {
                    return Err(format!("lists '{name}', which is outside the batch"));
                }
                let target = folder.join(name);
                let meta = std::fs::metadata(&target).map_err(|_| format!("lists {name}, which is missing"))?;
                let bytes = read(&target, meta.len()).map_err(|e| format!("{name} {e}"))?;
                if hex::encode(sha2::Sha256::digest(&bytes)) != hash.to_lowercase() {
                    return Err(format!("{name} does not match its SHA-256"));
                }
                checked += 1;
            }
            if checked == 0 {
                return Err("lists no files".into());
            }
            Ok(format!("{checked} file(s) match"))
        }
    }
}

/// Write `report` as `<base>.validation.json` and `.txt` into `dir`; returns the JSON path.
pub fn write_report(report: &ValidationReport, dir: &Path, base: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let json = dir.join(format!("{base}.validation.json"));
    let text = dir.join(format!("{base}.validation.txt"));
    std::fs::write(&json, serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?).map_err(|e| format!("Cannot write {}: {e}", json.display()))?;
    std::fs::write(&text, report.to_text()).map_err(|e| format!("Cannot write {}: {e}", text.display()))?;
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    fn batch() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("orders_1.csv"), "\u{feff}Order,Amount,Customer\n1,10,a\n2,20,b\n").unwrap();
        std::fs::write(d.path().join("orders_2.CSV"), "order;amount\n3;30\n").unwrap();
        std::fs::write(d.path().join("meta.json"), r#"{"batch": 7}"#).unwrap();
        std::fs::write(d.path().join(".DS_Store"), "x").unwrap();
        let sum = hex::encode(sha2::Sha256::digest(std::fs::read(d.path().join("meta.json")).unwrap()));
        std::fs::write(d.path().join("SHA256SUMS"), format!("{sum}  meta.json\n")).unwrap();
        d
    }

    #[test]
    fn a_complete_batch_passes_and_reports() {
        let d = batch();
        let rules = BatchRules {
            members: vec![
                MemberRule { pattern: "orders_*.csv".into(), min: 2, max: Some(5), min_bytes: Some(10) },
                MemberRule { pattern: "meta.json".into(), min: 1, max: Some(1), min_bytes: None },
                MemberRule { pattern: "SHA256SUMS".into(), min: 1, max: None, min_bytes: None },
            ],
            content: vec![
                ContentRule { pattern: "orders_1.csv".into(), check: ContentCheck::Csv { required_columns: vec!["order".into(), "AMOUNT".into()], min_rows: Some(2), delimiter: None } },
                ContentRule { pattern: "orders_2.csv".into(), check: ContentCheck::Csv { required_columns: vec!["amount".into()], min_rows: None, delimiter: Some(";".into()) } },
                ContentRule { pattern: "*.json".into(), check: ContentCheck::Json },
                ContentRule { pattern: "sha256sums".into(), check: ContentCheck::Sha256Manifest },
            ],
            no_extra_files: true,
        };
        let r = validate_batch(d.path(), &rules).unwrap();
        assert!(r.ok, "{}", r.to_text());
        assert_eq!(r.files, 4, "hidden files are not members");
        assert!(r.summary().starts_with("Batch valid: 4 files"));
        let json = write_report(&r, d.path(), "batch").unwrap();
        let back: ValidationReport = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
        assert_eq!(back, r);
        assert!(std::fs::read_to_string(d.path().join("batch.validation.txt")).unwrap().contains("Result: VALID"));
        // The reports themselves don't count as members next time.
        assert!(validate_batch(d.path(), &rules).unwrap().ok);
    }

    #[test]
    fn each_kind_of_problem_is_reported() {
        let d = batch();
        std::fs::write(d.path().join("orders_3.csv"), "order,amount\n4\n").unwrap();
        std::fs::write(d.path().join("empty.txt"), "").unwrap();
        std::fs::write(d.path().join("meta.json"), "{broken").unwrap();
        let rules = BatchRules {
            members: vec![
                MemberRule { pattern: "orders_*.csv".into(), min: 1, max: Some(2), min_bytes: Some(19) },
                MemberRule { pattern: "invoice.pdf".into(), min: 1, max: None, min_bytes: None },
            ],
            content: vec![
                ContentRule { pattern: "orders_3.csv".into(), check: ContentCheck::Csv { required_columns: vec![], min_rows: None, delimiter: None } },
                ContentRule { pattern: "orders_1.csv".into(), check: ContentCheck::Csv { required_columns: vec!["vat".into()], min_rows: Some(5), delimiter: None } },
                ContentRule { pattern: "*.txt".into(), check: ContentCheck::NotEmpty },
                ContentRule { pattern: "meta.json".into(), check: ContentCheck::Json },
                ContentRule { pattern: "SHA256SUMS".into(), check: ContentCheck::Sha256Manifest },
                ContentRule { pattern: "*.xml".into(), check: ContentCheck::NotEmpty },
            ],
            no_extra_files: true,
        };
        let r = validate_batch(d.path(), &rules).unwrap();
        assert!(!r.ok);
        let failed: Vec<&str> = r.failures().map(|f| f.message.as_str()).collect();
        let has = |s: &str| assert!(failed.iter().any(|m| m.contains(s)), "missing '{s}' in {failed:#?}");
        has("3 file(s) match orders_*.csv (expected 1–2)");
        has("orders_2.CSV is 18 bytes, below 19");
        has("0 file(s) match invoice.pdf");
        has("orders_3.csv: has a malformed row");
        has("is missing column(s) vat");
        has("empty.txt: is empty");
        has("meta.json: is not valid JSON");
        has("meta.json does not match its SHA-256");
        has("No file matches *.xml");
        has("empty.txt is not an expected member");
        assert!(r.summary().starts_with("Batch invalid"));
    }

    #[test]
    fn rules_and_manifests_are_checked() {
        assert!(BatchRules::default().validate().is_err());
        let bad = |p: &str| BatchRules { members: vec![MemberRule { pattern: p.into(), min: 1, max: None, min_bytes: None }], ..Default::default() }.validate().is_err();
        assert!(bad("") && bad("../*.csv") && bad("[") && !bad("*.csv"));
        assert!(BatchRules { members: vec![MemberRule { pattern: "*".into(), min: 3, max: Some(2), min_bytes: None }], ..Default::default() }.validate().is_err());
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("m.sha256"), format!("{}  ../etc/passwd\n", "0".repeat(64))).unwrap();
        let rules = BatchRules { content: vec![ContentRule { pattern: "*.sha256".into(), check: ContentCheck::Sha256Manifest }], ..Default::default() };
        let r = validate_batch(d.path(), &rules).unwrap();
        assert!(r.failures().any(|f| f.message.contains("outside the batch")));
        assert!(validate_batch(&d.path().join("missing"), &rules).is_err());
    }
}
