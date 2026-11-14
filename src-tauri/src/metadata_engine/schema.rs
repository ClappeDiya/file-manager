//! Metadata schemas: typed fields a folder's files carry (Files.com-style
//! "metadata requirements").
//!
//! - A schema belongs to one folder; `recursive` schemas also govern its
//!   subfolders, and the nearest schema wins.
//! - Values set on a file under a schema are validated (and normalized) for
//!   the schema's fields; a `strict` schema refuses keys it does not define.
//! - Required fields cannot be removed; `compliance` lists files missing
//!   required fields or holding values that no longer validate (set before the
//!   schema existed or changed).

use super::{dir_prefix, now, MetadataStore};
use crate::core::error::AppError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const MAX_FIELDS: usize = 64;
pub const MAX_COMPLIANCE_FILES: usize = 20_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Text,
    Number,
    Integer,
    /// `YYYY-MM-DD`.
    Date,
    /// `true` / `false`.
    Boolean,
    /// One of `options`.
    Choice,
    Email,
    Url,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MetadataField {
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    /// Allowed values (choice).
    #[serde(default)]
    pub options: Vec<String>,
    /// Regular expression the whole value must match (text).
    #[serde(default)]
    pub pattern: Option<String>,
    /// Bounds: value (number, integer) or length (text).
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetadataSchema {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Absolute folder path.
    pub folder: String,
    pub fields: Vec<MetadataField>,
    /// Refuse keys the schema does not define.
    #[serde(default)]
    pub strict: bool,
    /// Also govern subfolders (unless a nearer schema exists).
    #[serde(default = "yes")]
    pub recursive: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComplianceIssue {
    pub path: String,
    pub key: String,
    /// `missing`, `invalid` or `not_allowed` (strict schema).
    pub problem: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComplianceReport {
    pub schema: Option<MetadataSchema>,
    pub files_checked: usize,
    pub compliant: usize,
    pub issues: Vec<ComplianceIssue>,
    /// More files existed than were checked.
    pub truncated: bool,
}

impl MetadataField {
    fn name(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.key
        } else {
            &self.label
        }
    }

    /// Validate `value`; returns it normalized (trimmed, canonical booleans/numbers/dates).
    pub fn validate(&self, value: &str) -> Result<String, String> {
        let v = value.trim();
        let name = self.name();
        if v.is_empty() {
            return if self.required { Err(format!("{name} is required")) } else { Ok(String::new()) };
        }
        let bounds = |n: f64, what: &str| -> Result<(), String> {
            if let Some(min) = self.min.filter(|m| n < *m) {
                return Err(format!("{name}: {what} must be at least {min}"));
            }
            if let Some(max) = self.max.filter(|m| n > *m) {
                return Err(format!("{name}: {what} must be at most {max}"));
            }
            Ok(())
        };
        Ok(match self.kind {
            FieldKind::Text => {
                bounds(v.chars().count() as f64, "length")?;
                if let Some(p) = self.pattern.as_deref().filter(|p| !p.is_empty()) {
                    let re = regex::Regex::new(&format!("^(?:{p})$")).map_err(|e| format!("{name}: invalid pattern: {e}"))?;
                    if !re.is_match(v) {
                        return Err(format!("{name}: '{v}' does not match the required format"));
                    }
                }
                v.to_string()
            }
            FieldKind::Number => {
                let n: f64 = v.parse().ok().filter(|n: &f64| n.is_finite()).ok_or_else(|| format!("{name}: '{v}' is not a number"))?;
                bounds(n, "value")?;
                v.to_string()
            }
            FieldKind::Integer => {
                let n: i64 = v.parse().map_err(|_| format!("{name}: '{v}' is not a whole number"))?;
                bounds(n as f64, "value")?;
                n.to_string()
            }
            FieldKind::Date => chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").map_err(|_| format!("{name}: '{v}' is not a date (YYYY-MM-DD)"))?.format("%Y-%m-%d").to_string(),
            FieldKind::Boolean => match v.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => "true".into(),
                "false" | "no" | "0" => "false".into(),
                _ => return Err(format!("{name}: '{v}' is not true or false")),
            },
            FieldKind::Choice => self.options.iter().find(|o| o.eq_ignore_ascii_case(v)).cloned().ok_or_else(|| format!("{name}: '{v}' is not one of {}", self.options.join(", ")))?,
            FieldKind::Email => {
                let ok = v.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.') && !v.contains(char::is_whitespace));
                if !ok {
                    return Err(format!("{name}: '{v}' is not an email address"));
                }
                v.to_string()
            }
            FieldKind::Url => {
                let ok = (v.starts_with("https://") || v.starts_with("http://")) && v.len() > 8 && !v.contains(char::is_whitespace);
                if !ok {
                    return Err(format!("{name}: '{v}' is not an http(s) URL"));
                }
                v.to_string()
            }
        })
    }
}

impl MetadataSchema {
    pub fn field(&self, key: &str) -> Option<&MetadataField> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Check the definition itself.
    pub fn check(&mut self) -> Result<(), AppError> {
        self.name = self.name.trim().to_string();
        self.folder = self.folder.trim().to_string();
        if self.name.is_empty() {
            return Err(AppError::validation("Give the schema a name"));
        }
        if self.folder.is_empty() {
            return Err(AppError::validation("Choose the folder the schema applies to"));
        }
        if self.fields.is_empty() || self.fields.len() > MAX_FIELDS {
            return Err(AppError::validation(format!("A schema needs 1–{MAX_FIELDS} fields")));
        }
        let mut seen = HashSet::new();
        for f in &mut self.fields {
            f.key = super::validate_key(&f.key)?;
            f.label = f.label.trim().to_string();
            if !seen.insert(f.key.to_lowercase()) {
                return Err(AppError::validation(format!("The field '{}' appears twice", f.key)));
            }
            if let (Some(a), Some(b)) = (f.min, f.max) {
                if a > b {
                    return Err(AppError::validation(format!("{}: the minimum is above the maximum", f.key)));
                }
            }
            match f.kind {
                FieldKind::Choice => {
                    f.options = f.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
                    if f.options.is_empty() {
                        return Err(AppError::validation(format!("{}: a choice field needs options", f.key)));
                    }
                }
                FieldKind::Text => {
                    if let Some(p) = f.pattern.as_deref().filter(|p| !p.is_empty()) {
                        regex::Regex::new(p).map_err(|e| AppError::validation(format!("{}: invalid pattern: {e}", f.key)))?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether this schema governs files in `dir`.
    fn governs(&self, dir: &str) -> bool {
        let same = dir_prefix(dir) == dir_prefix(&self.folder);
        same || (self.recursive && dir_prefix(dir).starts_with(&dir_prefix(&self.folder)))
    }

    /// Problems with one file's metadata.
    pub fn issues(&self, path: &str, have: &BTreeMap<String, String>) -> Vec<ComplianceIssue> {
        let issue = |key: &str, problem: &str, message: String| ComplianceIssue { path: path.into(), key: key.into(), problem: problem.into(), message };
        let mut out = Vec::new();
        for f in &self.fields {
            match have.get(&f.key).filter(|v| !v.trim().is_empty()) {
                None if f.required => out.push(issue(&f.key, "missing", format!("{} is required", f.name()))),
                Some(v) => {
                    if let Err(e) = f.validate(v) {
                        out.push(issue(&f.key, "invalid", e));
                    }
                }
                None => {}
            }
        }
        if self.strict {
            for k in have.keys().filter(|k| self.field(k).is_none()) {
                out.push(issue(k, "not_allowed", format!("'{k}' is not a field of {}", self.name)));
            }
        }
        out
    }
}

fn parent_dir(path: &str) -> &str {
    let t = path.trim_end_matches(['/', '\\']);
    match t.rfind(['/', '\\']) {
        Some(0) => &t[..1],
        Some(i) => &t[..i],
        None => "",
    }
}

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MetadataSchema> {
    let fields: String = r.get(3)?;
    Ok(MetadataSchema {
        id: r.get(0)?,
        name: r.get(1)?,
        folder: r.get(2)?,
        fields: serde_json::from_str(&fields).unwrap_or_default(),
        strict: r.get::<_, i64>(4)? != 0,
        recursive: r.get::<_, i64>(5)? != 0,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const COLS: &str = "id, name, folder, fields_json, strict, recursive, created_at, updated_at";

impl MetadataStore {
    pub async fn list_schemas(&self) -> Result<Vec<MetadataSchema>, AppError> {
        self.repo
            .pool()
            .execute(|conn| {
                let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM metadata_schemas ORDER BY folder"))?;
                let rows = stmt.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Create (empty id) or replace a schema. One schema per folder.
    pub async fn save_schema(&self, mut s: MetadataSchema) -> Result<MetadataSchema, AppError> {
        s.check()?;
        let fields = serde_json::to_string(&s.fields).map_err(AppError::from)?;
        if s.id.is_empty() {
            s.id = uuid::Uuid::new_v4().to_string();
            s.created_at = now();
        }
        s.updated_at = now();
        let out = s.clone();
        self.repo
            .pool()
            .execute(move |conn| {
                let other: Option<String> = conn.query_row("SELECT id FROM metadata_schemas WHERE folder = ?1 AND id != ?2", params![s.folder, s.id], |r| r.get(0)).optional()?;
                if other.is_some() {
                    return Err(AppError::validation(format!("{} already has a schema; edit that one", s.folder)));
                }
                let created: Option<String> = conn.query_row("SELECT created_at FROM metadata_schemas WHERE id = ?1", params![s.id], |r| r.get(0)).optional()?;
                conn.execute(
                    "INSERT INTO metadata_schemas (id, name, folder, fields_json, strict, recursive, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(id) DO UPDATE SET name = excluded.name, folder = excluded.folder, fields_json = excluded.fields_json, strict = excluded.strict, recursive = excluded.recursive, updated_at = excluded.updated_at",
                    params![s.id, s.name, s.folder, fields, s.strict as i64, s.recursive as i64, created.unwrap_or(s.created_at.clone()), s.updated_at],
                )?;
                Ok(())
            })
            .await?;
        Ok(out)
    }

    pub async fn delete_schema(&self, id: &str) -> Result<(), AppError> {
        let id = id.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute("DELETE FROM metadata_schemas WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await
    }

    /// The schema governing files in `dir` (the nearest one).
    pub async fn schema_for_dir(&self, dir: &str) -> Result<Option<MetadataSchema>, AppError> {
        Ok(self.list_schemas().await?.into_iter().filter(|s| s.governs(dir)).max_by_key(|s| dir_prefix(&s.folder).len()))
    }

    /// The schema governing a file.
    pub async fn schema_for(&self, path: &str) -> Result<Option<MetadataSchema>, AppError> {
        self.schema_for_dir(parent_dir(path)).await
    }

    /// Validate a value about to be set; returns the value to store.
    pub(super) async fn check_value(&self, path: &str, key: &str, value: &str) -> Result<String, AppError> {
        let Some(schema) = self.schema_for(path).await? else { return Ok(value.to_string()) };
        match schema.field(key) {
            Some(f) => f.validate(value).map_err(AppError::validation),
            None if schema.strict => Err(AppError::validation(format!("'{key}' is not a field of the {} schema ({})", schema.name, schema.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>().join(", ")))),
            None => Ok(value.to_string()),
        }
    }

    /// Refuse removing a field the governing schema requires.
    pub(super) async fn check_removal(&self, path: &str, key: &str) -> Result<(), AppError> {
        if let Some(s) = self.schema_for(path).await? {
            if s.field(key).is_some_and(|f| f.required) {
                return Err(AppError::validation(format!("'{key}' is required by the {} schema", s.name)));
            }
        }
        Ok(())
    }

    /// Check the files in `folder` (and subfolders when `recursive`) against their schemas.
    pub async fn compliance(&self, folder: &str, recursive: bool) -> Result<ComplianceReport, AppError> {
        let schemas = self.list_schemas().await?;
        let root = std::path::PathBuf::from(folder);
        if !root.is_dir() {
            return Err(AppError::validation(format!("{folder} is not a folder")));
        }
        let mut files = Vec::new();
        let mut truncated = false;
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if recursive {
                        stack.push(e.path());
                    }
                } else if ft.is_file() {
                    if files.len() >= MAX_COMPLIANCE_FILES {
                        truncated = true;
                        break;
                    }
                    files.push(e.path().to_string_lossy().into_owned());
                }
            }
        }
        let snap = self.snapshot(&[folder.to_string()]).await?;
        let empty = BTreeMap::new();
        let mut issues = Vec::new();
        let mut compliant = 0;
        for f in &files {
            let dir = parent_dir(f);
            let governing = schemas.iter().filter(|s| s.governs(dir)).max_by_key(|s| dir_prefix(&s.folder).len());
            let found = governing.map(|s| s.issues(f, snap.metadata.get(f).unwrap_or(&empty))).unwrap_or_default();
            if found.is_empty() {
                compliant += 1;
            }
            issues.extend(found);
        }
        Ok(ComplianceReport { schema: self.schema_for_dir(folder).await?, files_checked: files.len(), compliant, issues, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Repository;

    fn field(key: &str, kind: FieldKind) -> MetadataField {
        MetadataField { key: key.into(), kind, ..Default::default() }
    }

    #[test]
    fn validates_and_normalizes_each_kind() {
        let mut text = field("code", FieldKind::Text);
        text.pattern = Some("[A-Z]{3}-\\d+".into());
        text.max = Some(8.0);
        assert_eq!(text.validate(" INV-42 ").unwrap(), "INV-42");
        assert!(text.validate("inv-42").unwrap_err().contains("format"));
        assert!(text.validate("ABC-123456").unwrap_err().contains("at most 8"));
        let mut n = field("amount", FieldKind::Number);
        n.min = Some(0.0);
        assert_eq!(n.validate("12.5").unwrap(), "12.5");
        assert!(n.validate("-1").is_err() && n.validate("NaN").is_err() && n.validate("abc").is_err());
        assert_eq!(field("n", FieldKind::Integer).validate("007").unwrap(), "7");
        assert!(field("n", FieldKind::Integer).validate("1.5").is_err());
        assert_eq!(field("d", FieldKind::Date).validate("2026-02-28").unwrap(), "2026-02-28");
        assert!(field("d", FieldKind::Date).validate("2026-02-30").is_err());
        assert_eq!(field("b", FieldKind::Boolean).validate("Yes").unwrap(), "true");
        let mut c = field("status", FieldKind::Choice);
        c.options = vec!["Draft".into(), "Final".into()];
        assert_eq!(c.validate("final").unwrap(), "Final");
        assert!(c.validate("Other").unwrap_err().contains("Draft, Final"));
        assert!(field("e", FieldKind::Email).validate("a@b.co").is_ok() && field("e", FieldKind::Email).validate("a@b").is_err());
        assert!(field("u", FieldKind::Url).validate("https://x.io/a").is_ok() && field("u", FieldKind::Url).validate("ftp://x").is_err());
        let mut req = field("client", FieldKind::Text);
        req.required = true;
        req.label = "Client".into();
        assert_eq!(req.validate("  ").unwrap_err(), "Client is required");
    }

    #[test]
    fn checks_definitions() {
        let mut s = MetadataSchema { id: String::new(), name: "Invoices".into(), folder: "/d".into(), fields: vec![field("a", FieldKind::Text), field("A", FieldKind::Text)], strict: false, recursive: true, created_at: String::new(), updated_at: String::new() };
        assert!(s.check().unwrap_err().to_string().contains("twice"));
        s.fields = vec![field("status", FieldKind::Choice)];
        assert!(s.check().unwrap_err().to_string().contains("options"));
        s.fields = vec![MetadataField { key: "x".into(), pattern: Some("(".into()), ..Default::default() }];
        assert!(s.check().unwrap_err().to_string().contains("pattern"));
        s.fields = vec![MetadataField { key: "x".into(), kind: FieldKind::Number, min: Some(5.0), max: Some(1.0), ..Default::default() }];
        assert!(s.check().unwrap_err().to_string().contains("minimum"));
    }

    #[tokio::test]
    async fn schemas_govern_values_removal_and_compliance() {
        let repo = Repository::open_in_memory().await.unwrap();
        let store = MetadataStore::new(repo);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("invoices");
        std::fs::create_dir_all(root.join("2026/q1")).unwrap();
        std::fs::create_dir_all(dir.path().join("other")).unwrap();
        let (a, b, c, o) = (root.join("a.pdf"), root.join("2026/q1/b.pdf"), root.join("2026/q1/c.pdf"), dir.path().join("other/o.pdf"));
        for p in [&a, &b, &c, &o] {
            std::fs::write(p, "x").unwrap();
        }
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        // Set before any schema: free-form.
        store.set_metadata(&s(&b), "status", "whatever").await.unwrap();

        let mut status = field("status", FieldKind::Choice);
        status.options = vec!["Draft".into(), "Paid".into()];
        status.required = true;
        let schema = store
            .save_schema(MetadataSchema { id: String::new(), name: "Invoices".into(), folder: s(&root), fields: vec![status, field("amount", FieldKind::Number)], strict: true, recursive: true, created_at: String::new(), updated_at: String::new() })
            .await
            .unwrap();
        assert!(!schema.id.is_empty());
        assert_eq!(store.schema_for(&s(&b)).await.unwrap().unwrap().id, schema.id, "subfolders inherit");
        assert!(store.schema_for(&s(&o)).await.unwrap().is_none());
        // A sibling folder whose name starts the same is not governed.
        assert!(store.schema_for_dir(&format!("{}-old", s(&root))).await.unwrap().is_none());

        assert_eq!(store.set_metadata(&s(&a), "status", "paid").await.unwrap().metadata["status"], "Paid");
        assert!(store.set_metadata(&s(&a), "status", "Lost").await.unwrap_err().to_string().contains("Draft, Paid"));
        assert!(store.set_metadata(&s(&a), "amount", "ten").await.is_err());
        assert!(store.set_metadata(&s(&a), "notes", "x").await.unwrap_err().to_string().contains("not a field"));
        assert!(store.remove_metadata(&s(&a), "status").await.unwrap_err().to_string().contains("required"));
        store.set_metadata(&s(&o), "notes", "free").await.unwrap();

        let r = store.compliance(&s(&root), true).await.unwrap();
        assert_eq!((r.files_checked, r.compliant), (3, 1));
        let probs: Vec<(String, &str, &str)> = r.issues.iter().map(|i| (std::path::Path::new(&i.path).file_name().unwrap().to_string_lossy().into_owned(), i.key.as_str(), i.problem.as_str())).collect();
        assert!(probs.contains(&("b.pdf".into(), "status", "invalid")), "{probs:?}");
        assert!(probs.contains(&("c.pdf".into(), "status", "missing")), "{probs:?}");
        assert_eq!(store.compliance(&s(&root), false).await.unwrap().files_checked, 1);

        // A nearer, non-strict schema on the subfolder wins there.
        store
            .save_schema(MetadataSchema { id: String::new(), name: "Q1".into(), folder: s(&root.join("2026/q1")), fields: vec![field("amount", FieldKind::Integer)], strict: false, recursive: false, created_at: String::new(), updated_at: String::new() })
            .await
            .unwrap();
        store.set_metadata(&s(&c), "notes", "ok here").await.unwrap();
        assert!(store.remove_metadata(&s(&b), "status").await.is_ok());
        let dup = store.save_schema(MetadataSchema { id: String::new(), name: "Again".into(), folder: s(&root), fields: vec![field("x", FieldKind::Text)], strict: false, recursive: true, created_at: String::new(), updated_at: String::new() }).await;
        assert!(dup.unwrap_err().to_string().contains("already has a schema"));
        store.delete_schema(&schema.id).await.unwrap();
        store.set_metadata(&s(&a), "anything", "goes").await.unwrap();
    }

    #[test]
    fn parent_dirs() {
        assert_eq!(parent_dir("/a/b/c.txt"), "/a/b");
        assert_eq!(parent_dir("/c.txt"), "/");
        assert_eq!(parent_dir(r"C:\d\e.txt"), r"C:\d");
    }
}
