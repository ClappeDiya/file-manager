//! Named policy templates and the settings resolution every job runs with.
//!
//! A [`PolicyTemplate`] is a named, partial set of job settings (a
//! [`SettingsPatch`]): only the fields it sets apply. Jobs list the templates
//! they are based on (`template_ids`, in order). What a job runs with is
//! decided per setting by [`resolve`], the one place that knows the order:
//!
//! ```text
//! built-in defaults < global defaults < templates (in order) < the job's own settings < managed (console)
//! ```
//!
//! A setting is addressed by its *field path*: `options.<name>` for each
//! backup option, or a whole section (`filter`, `retention`, `schedule`,
//! `maintenance`, `scripts`, `notifications`, `notify_on`, `throttle`,
//! `retry`, `prevent_sleep`). Which settings the job sets itself is its
//! `overrides` list; jobs from before templates have none, and every
//! setting that differs from the built-in default counts as theirs.
//!
//! Templates the organization delivers through the console's managed
//! settings are read-only here; an *enforced* one is the managed layer and
//! wins over everything else.

use super::defaults::GlobalDefaults;
use super::schedule::{BackupSchedule, MaintenanceSchedule};
use super::{hooks, notify, BackupJob};
use chrono::Utc;
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use ufop_core::backup::backend::{RetryPolicy, Throttle};
use ufop_core::backup::{BackupOptions, RetentionPolicy, SourceFilter};

/// Config-table key of the local templates.
pub const TEMPLATES_KEY: &str = "backup.policy_templates";

/// Job sections a template can set as a whole.
pub const SECTIONS: &[&str] = &["filter", "retention", "schedule", "maintenance", "scripts", "notifications", "notify_on", "throttle", "retry", "prevent_sleep"];

/// Backup options that belong to one job (repository layout, per-run
/// inputs): never inherited, always the job's own.
pub const PER_JOB_OPTIONS: &[&str] = &["prefix", "block_size", "block_hash", "file_hash", "dry_run", "task_config", "changed_paths", "deleted_paths"];

/// A partial set of job settings: `None` / absent = not set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsPatch {
    /// Backup options (snake_case names, engine JSON values), like the global defaults.
    pub options: Map<String, Value>,
    pub filter: Option<SourceFilter>,
    pub retention: Option<RetentionPolicy>,
    pub schedule: Option<BackupSchedule>,
    pub maintenance: Option<MaintenanceSchedule>,
    pub scripts: Option<hooks::ScriptConfig>,
    pub notifications: Option<Vec<notify::Notification>>,
    pub notify_on: Option<Vec<String>>,
    pub throttle: Option<Throttle>,
    pub retry: Option<RetryPolicy>,
    pub prevent_sleep: Option<bool>,
}

/// Compared by value (the filter holds compiled patterns).
impl PartialEq for SettingsPatch {
    fn eq(&self, o: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(o).ok()
    }
}

impl SettingsPatch {
    /// The patch as field path → value.
    pub fn paths(&self) -> Result<Vec<(String, Value)>, AppError> {
        let mut out: Vec<(String, Value)> = self.options.iter().map(|(k, v)| (format!("options.{k}"), v.clone())).collect();
        let v = serde_json::to_value(self)?;
        for s in SECTIONS {
            if let Some(x) = v.get(*s).filter(|x| !x.is_null()) {
                out.push((s.to_string(), x.clone()));
            }
        }
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        self.paths().map(|p| p.is_empty()).unwrap_or(true)
    }

    /// Reject unknown or per-job options and values the job cannot take.
    pub fn validate(&self) -> Result<(), AppError> {
        validate_options(&self.options)?;
        let mut job = BackupJob::default();
        apply_paths(&mut job, &self.paths()?)?;
        if let Some(s) = &self.schedule {
            s.validate()?;
        }
        if let Some(m) = &self.maintenance {
            m.validate()?;
        }
        if let Some(r) = &self.retention {
            r.validate()?;
        }
        if let Some(f) = &self.filter {
            f.clone().compile()?;
        }
        Ok(())
    }
}

/// Shared by global defaults and templates: option names and values.
pub fn validate_options(options: &Map<String, Value>) -> Result<(), AppError> {
    let base = serde_json::to_value(BackupOptions::default())?;
    let known = base.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    for k in options.keys() {
        if !known.contains(k) {
            return Err(AppError::validation(format!("Unknown backup option '{k}'")));
        }
        if PER_JOB_OPTIONS.contains(&k.as_str()) {
            return Err(AppError::validation(format!("'{k}' is per job and cannot be inherited")));
        }
    }
    let mut merged = base;
    if let Some(o) = merged.as_object_mut() {
        o.extend(options.clone());
    }
    serde_json::from_value::<BackupOptions>(merged).map_err(|e| AppError::validation(format!("Invalid option value: {e}")))?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub settings: SettingsPatch,
    pub updated_at: String,
    /// Delivered by the organization's console (read-only here).
    pub managed: bool,
    /// Managed only: applies to every job, over the job's own settings.
    pub enforced: bool,
}

impl PolicyTemplate {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() {
            return Err(AppError::validation("Give the template a name"));
        }
        self.settings.validate()
    }

    /// A console template (`ufop_core::agent::ManagedBackupTemplate`).
    pub fn from_managed(m: &ufop_core::agent::ManagedBackupTemplate) -> Result<Self, AppError> {
        let settings: SettingsPatch =
            serde_json::from_value(m.settings.clone()).map_err(|e| AppError::validation(format!("Organization template '{}': {e}", m.name)))?;
        let t = Self { id: format!("{MANAGED_PREFIX}{}", m.id), name: m.name.clone(), description: m.description.clone(), settings, updated_at: String::new(), managed: true, enforced: m.enforced };
        t.validate().map_err(|e| AppError::validation(format!("Organization template '{}': {}", m.name, e.user_message())))?;
        Ok(t)
    }
}

/// Id prefix of console-delivered templates.
pub const MANAGED_PREFIX: &str = "org:";

/// Where an effective setting came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingSource {
    BuiltIn,
    Global,
    Template { id: String, name: String },
    Job,
    Managed { id: String, name: String },
}

/// The value a setting would have without the job's own (or managed) value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inherited {
    pub value: Value,
    pub source: SettingSource,
}

/// One setting as the job runs with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveSetting {
    pub field_path: String,
    pub value: Value,
    pub source: SettingSource,
    /// Set when the job (or the console) overrides an inherited value: what
    /// "reset to inherited" gives.
    pub inherited: Option<Inherited>,
}

/// [`resolve`]'s result.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub job: BackupJob,
    pub settings: Vec<EffectiveSetting>,
    pub warnings: Vec<String>,
}

/// Every field path a job has.
pub fn all_paths() -> Vec<String> {
    let opts = serde_json::to_value(BackupOptions::default()).ok();
    let mut v: Vec<String> = opts.as_ref().and_then(|o| o.as_object()).map(|o| o.keys().map(|k| format!("options.{k}")).collect()).unwrap_or_default();
    v.sort();
    v.extend(SECTIONS.iter().map(|s| s.to_string()));
    v
}

fn get<'v>(job: &'v Value, path: &str) -> Option<&'v Value> {
    match path.split_once('.') {
        Some((a, b)) => job.get(a)?.get(b),
        None => job.get(path),
    }
}

fn set(job: &mut Value, path: &str, value: Value) {
    match path.split_once('.') {
        Some((a, b)) => {
            if let Some(o) = job.get_mut(a).and_then(|x| x.as_object_mut()) {
                o.insert(b.to_string(), value);
            }
        }
        None => {
            if let Some(o) = job.as_object_mut() {
                o.insert(path.to_string(), value);
            }
        }
    }
}

fn apply_paths(job: &mut BackupJob, paths: &[(String, Value)]) -> Result<(), AppError> {
    let mut v = serde_json::to_value(&*job)?;
    for (p, x) in paths {
        set(&mut v, p, x.clone());
    }
    *job = serde_json::from_value(v).map_err(|e| AppError::validation(format!("Invalid setting: {e}")))?;
    Ok(())
}

fn is_per_job(path: &str) -> bool {
    path.strip_prefix("options.").is_some_and(|k| PER_JOB_OPTIONS.contains(&k))
}

/// The settings `job` sets itself: its `overrides`, or (older jobs) every
/// setting that differs from the built-in default. Per-job options always count.
pub fn explicit_paths(job: &BackupJob) -> Result<BTreeSet<String>, AppError> {
    let builtin = serde_json::to_value(BackupJob::default())?;
    let jv = serde_json::to_value(job)?;
    let mut out = BTreeSet::new();
    for p in all_paths() {
        let own = match &job.overrides {
            Some(list) => list.contains(&p),
            None => get(&jv, &p) != get(&builtin, &p),
        };
        if own || (is_per_job(&p) && get(&jv, &p) != get(&builtin, &p)) {
            out.insert(p);
        }
    }
    Ok(out)
}

/// Check a job's template ids and override paths.
pub fn validate_job_refs(job: &BackupJob, templates: &[PolicyTemplate]) -> Result<(), AppError> {
    for id in &job.template_ids {
        match templates.iter().find(|t| &t.id == id) {
            None => return Err(AppError::validation(format!("Policy template {id} does not exist"))),
            Some(t) if t.enforced => return Err(AppError::validation(format!("'{}' applies to every job already; it cannot be chosen", t.name))),
            Some(_) => {}
        }
    }
    if let Some(list) = &job.overrides {
        let known = all_paths();
        if let Some(p) = list.iter().find(|p| !known.contains(p)) {
            return Err(AppError::validation(format!("Unknown setting '{p}'")));
        }
    }
    Ok(())
}

/// What `job` runs with, setting by setting: built-in defaults < global
/// defaults < the job's templates (in order) < the job's own settings <
/// enforced managed templates. `templates` holds every known template
/// (local and managed); missing ones are skipped with a warning.
pub fn resolve(job: &BackupJob, globals: &GlobalDefaults, templates: &[PolicyTemplate]) -> Result<Resolution, AppError> {
    let builtin = serde_json::to_value(BackupJob::default())?;
    let jv = serde_json::to_value(job)?;
    let mut warnings = Vec::new();

    // Layers below the job, in order.
    let mut layers: Vec<(SettingSource, Vec<(String, Value)>)> = Vec::new();
    let mut g: Vec<(String, Value)> = globals.options.iter().map(|(k, v)| (format!("options.{k}"), v.clone())).collect();
    if let Some(t) = globals.throttle {
        g.push(("throttle".into(), serde_json::to_value(t)?));
    }
    if let Some(r) = globals.retry {
        g.push(("retry".into(), serde_json::to_value(r)?));
    }
    layers.push((SettingSource::Global, g));
    for id in &job.template_ids {
        match templates.iter().find(|t| &t.id == id && !t.enforced) {
            Some(t) => layers.push((SettingSource::Template { id: t.id.clone(), name: t.name.clone() }, t.settings.paths()?)),
            None => warnings.push(format!("Policy template {id} no longer exists; its settings are not applied")),
        }
    }
    let managed: Vec<(SettingSource, Vec<(String, Value)>)> = templates
        .iter()
        .filter(|t| t.managed && t.enforced)
        .map(|t| Ok((SettingSource::Managed { id: t.id.clone(), name: t.name.clone() }, t.settings.paths()?)))
        .collect::<Result<_, AppError>>()?;
    let explicit = explicit_paths(job)?;

    let mut out = jv.clone();
    let mut settings = Vec::new();
    for p in all_paths() {
        let mut value = get(&builtin, &p).cloned().unwrap_or(Value::Null);
        let mut source = SettingSource::BuiltIn;
        if !is_per_job(&p) {
            for (src, paths) in &layers {
                if let Some((_, v)) = paths.iter().rev().find(|(k, _)| *k == p) {
                    value = v.clone();
                    source = src.clone();
                }
            }
        }
        let below = Inherited { value: value.clone(), source: source.clone() };
        let mut inherited = None;
        if explicit.contains(&p) {
            value = get(&jv, &p).cloned().unwrap_or(Value::Null);
            source = SettingSource::Job;
            inherited = Some(below.clone());
        }
        if !is_per_job(&p) {
            for (src, paths) in &managed {
                if let Some((_, v)) = paths.iter().rev().find(|(k, _)| *k == p) {
                    value = v.clone();
                    source = src.clone();
                    inherited.get_or_insert_with(|| below.clone());
                }
            }
        }
        set(&mut out, &p, value.clone());
        settings.push(EffectiveSetting { field_path: p, value, source, inherited });
    }
    let mut resolved: BackupJob = serde_json::from_value(out).map_err(|e| AppError::validation(format!("Inherited settings do not fit this job: {e}")))?;
    resolved.masked_values = job.masked_values.clone();
    Ok(Resolution { job: resolved, settings, warnings })
}

/// Export file format for templates.
const FORMAT: &str = "ufop-backup-templates";

#[derive(Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    templates: Vec<PolicyTemplate>,
}

/// Templates as a JSON file (managed ones are exported as ordinary templates).
pub fn export(templates: &[PolicyTemplate]) -> Result<String, AppError> {
    let templates = templates.iter().map(|t| PolicyTemplate { managed: false, enforced: false, ..t.clone() }).collect();
    Ok(serde_json::to_string_pretty(&ExportFile { format: FORMAT.into(), version: 1, templates })?)
}

/// Parse an export file, a bare list or a single template.
pub fn import(json: &str) -> Result<Vec<PolicyTemplate>, AppError> {
    let v: Value = serde_json::from_str(json).map_err(|e| AppError::validation(format!("Not a template file: {e}")))?;
    let list: Vec<PolicyTemplate> = if v.get("format").and_then(|f| f.as_str()) == Some(FORMAT) {
        serde_json::from_value(v["templates"].clone())?
    } else if v.is_array() {
        serde_json::from_value(v)?
    } else {
        vec![serde_json::from_value(v)?]
    };
    let list: Vec<PolicyTemplate> = list.into_iter().map(|t| PolicyTemplate { managed: false, enforced: false, ..t }).collect();
    for t in &list {
        t.validate()?;
    }
    Ok(list)
}

/// A job's settings with where each came from (the effective-settings view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveSettings {
    pub job_id: String,
    pub template_ids: Vec<String>,
    /// `None`: an older job (settings differing from the defaults are its own).
    pub overrides: Option<Vec<String>>,
    pub settings: Vec<EffectiveSetting>,
    pub warnings: Vec<String>,
}

impl super::BackupManager {
    async fn local_templates(&self) -> Result<Vec<PolicyTemplate>, AppError> {
        Ok(self.config_get(TEMPLATES_KEY).await?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    /// Local templates, then the organization's (read-only), by name.
    pub async fn list_templates(&self) -> Result<Vec<PolicyTemplate>, AppError> {
        let mut v = self.local_templates().await?;
        v.extend(self.inner.managed_templates.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned());
        v.sort_by_key(|t| (t.managed, t.name.to_lowercase()));
        Ok(v)
    }

    pub async fn get_template(&self, id: &str) -> Result<PolicyTemplate, AppError> {
        self.list_templates().await?.into_iter().find(|t| t.id == id).ok_or_else(|| AppError::not_found(format!("Policy template {id} not found")))
    }

    /// Create (empty id) or update a local template.
    pub async fn save_template(&self, mut t: PolicyTemplate) -> Result<PolicyTemplate, AppError> {
        if t.managed || t.id.starts_with(MANAGED_PREFIX) {
            return Err(AppError::PermissionDenied { message: "Organization templates are managed by the console".into(), advice: "Copy it into a new template to change it.".into() });
        }
        t.name = t.name.trim().to_string();
        t.enforced = false;
        t.validate()?;
        let mut list = self.local_templates().await?;
        if list.iter().any(|x| x.id != t.id && x.name.eq_ignore_ascii_case(&t.name)) {
            return Err(AppError::validation(format!("A template named '{}' already exists", t.name)));
        }
        if t.id.is_empty() {
            t.id = uuid::Uuid::new_v4().to_string();
        }
        t.updated_at = Utc::now().to_rfc3339();
        match list.iter_mut().find(|x| x.id == t.id) {
            Some(x) => *x = t.clone(),
            None => list.push(t.clone()),
        }
        self.config_set(TEMPLATES_KEY, serde_json::to_string(&list)?).await?;
        Ok(t)
    }

    /// Delete a local template no job uses.
    pub async fn delete_template(&self, id: &str) -> Result<(), AppError> {
        if id.starts_with(MANAGED_PREFIX) {
            return Err(AppError::PermissionDenied { message: "Organization templates are managed by the console".into(), advice: "Ask your administrator to remove it.".into() });
        }
        let users: Vec<String> = self.list_jobs().await.into_iter().filter(|j| j.template_ids.iter().any(|t| t == id)).map(|j| j.name).collect();
        if !users.is_empty() {
            return Err(AppError::state(format!("The template is used by {}", users.join(", ")), "Remove it from those jobs first."));
        }
        let mut list = self.local_templates().await?;
        let before = list.len();
        list.retain(|t| t.id != id);
        if list.len() == before {
            return Err(AppError::not_found(format!("Policy template {id} not found")));
        }
        self.config_set(TEMPLATES_KEY, serde_json::to_string(&list)?).await
    }

    /// Base a job on one more template (appended: it wins over earlier ones).
    pub async fn apply_template(&self, template_id: &str, job_id: &str) -> Result<BackupJob, AppError> {
        self.get_template(template_id).await?;
        let mut job = self.get_job(job_id).await?;
        if !job.template_ids.iter().any(|t| t == template_id) {
            job.template_ids.push(template_id.to_string());
        }
        self.save_job(job, None).await
    }

    /// Templates as a JSON file (`ids` empty = all).
    pub async fn export_templates(&self, ids: &[String]) -> Result<String, AppError> {
        let all = self.list_templates().await?;
        let chosen: Vec<PolicyTemplate> = all.into_iter().filter(|t| ids.is_empty() || ids.contains(&t.id)).collect();
        export(&chosen)
    }

    /// Import templates: same id or name updates, anything else is added.
    pub async fn import_templates(&self, json: &str) -> Result<Vec<PolicyTemplate>, AppError> {
        let existing = self.local_templates().await?;
        let mut saved = Vec::new();
        for mut t in import(json)? {
            t.id = existing.iter().find(|x| x.id == t.id || x.name.eq_ignore_ascii_case(t.name.trim())).map(|x| x.id.clone()).unwrap_or_default();
            saved.push(self.save_template(t).await?);
        }
        Ok(saved)
    }

    /// Replace the console's templates; returns the ones that were refused.
    pub fn set_managed_templates(&self, list: Option<&[ufop_core::agent::ManagedBackupTemplate]>) -> Vec<String> {
        let mut ok = Vec::new();
        let mut problems = Vec::new();
        for m in list.unwrap_or_default() {
            match PolicyTemplate::from_managed(m) {
                Ok(t) => ok.push(t),
                Err(e) => problems.push(e.user_message()),
            }
        }
        *self.inner.managed_templates.lock().unwrap_or_else(|e| e.into_inner()) = ok;
        problems
    }

    /// Resolve a job's settings (see [`resolve`]).
    pub async fn resolve_job(&self, job: &BackupJob) -> Result<Resolution, AppError> {
        resolve(job, &self.global_defaults().await?, &self.list_templates().await?)
    }

    /// Each setting of a saved job (`job_id`) or of an unsaved one (`job`), with its source.
    pub async fn effective_settings(&self, job_id: Option<&str>, job: Option<BackupJob>) -> Result<EffectiveSettings, AppError> {
        let job = match (job, job_id) {
            (Some(j), _) => j,
            (None, Some(id)) => self.get_job(id).await?,
            (None, None) => return Err(AppError::validation("Name a job or pass one")),
        };
        let r = self.resolve_job(&job).await?;
        Ok(EffectiveSettings { job_id: job.id.clone(), template_ids: job.template_ids.clone(), overrides: job.overrides.clone(), settings: r.settings, warnings: r.warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tpl(id: &str, patch: SettingsPatch) -> PolicyTemplate {
        PolicyTemplate { id: id.into(), name: format!("T {id}"), settings: patch, ..Default::default() }
    }

    fn opt(k: &str, v: Value) -> SettingsPatch {
        let mut p = SettingsPatch::default();
        p.options.insert(k.into(), v);
        p
    }

    fn source_of<'a>(r: &'a Resolution, path: &str) -> &'a EffectiveSetting {
        r.settings.iter().find(|s| s.field_path == path).unwrap()
    }

    #[test]
    fn precedence_builtin_global_templates_job_managed() {
        let mut g = GlobalDefaults::default();
        g.options.insert("compression_level".into(), 9.into());
        g.options.insert("upload_concurrency".into(), 6.into());
        g.throttle = Some(Throttle { upload_bps: 1000, download_bps: 0 });
        let a = tpl("a", SettingsPatch { retention: Some(RetentionPolicy::KeepCount { count: 5 }), ..opt("compression_level", 3.into()) });
        let b = tpl("b", SettingsPatch { retention: Some(RetentionPolicy::KeepDays { days: 30 }), ..opt("hash_threads", 2.into()) });
        let mut org = tpl("org:x", opt("hash_threads", 8.into()));
        org.managed = true;
        org.enforced = true;
        let all = vec![a, b, org.clone()];

        let mut job = BackupJob { template_ids: vec!["a".into(), "b".into()], overrides: Some(vec!["options.upload_concurrency".into()]), ..Default::default() };
        job.options.upload_concurrency = 2;
        job.options.compression_level = 1; // not in overrides: ignored
        let r = resolve(&job, &g, &all).unwrap();
        // Template a beats the global default.
        assert_eq!(r.job.options.compression_level, 3);
        assert_eq!(source_of(&r, "options.compression_level").source, SettingSource::Template { id: "a".into(), name: "T a".into() });
        // Later template wins.
        assert_eq!(r.job.retention, RetentionPolicy::KeepDays { days: 30 });
        // The job's own setting beats the global default.
        assert_eq!(r.job.options.upload_concurrency, 2);
        let s = source_of(&r, "options.upload_concurrency");
        assert_eq!(s.source, SettingSource::Job);
        assert_eq!(s.inherited.as_ref().unwrap().value, Value::from(6));
        assert_eq!(s.inherited.as_ref().unwrap().source, SettingSource::Global);
        // Managed beats the template.
        assert_eq!(r.job.options.hash_threads, 8);
        assert!(matches!(source_of(&r, "options.hash_threads").source, SettingSource::Managed { .. }));
        // Global and built-in values show where they came from.
        assert_eq!(r.job.throttle.upload_bps, 1000);
        assert_eq!(source_of(&r, "throttle").source, SettingSource::Global);
        assert_eq!(source_of(&r, "options.volume_size").source, SettingSource::BuiltIn);
        assert!(r.warnings.is_empty());

        // A deleted template is skipped with a warning.
        let r = resolve(&BackupJob { template_ids: vec!["gone".into()], ..Default::default() }, &g, &all).unwrap();
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn older_jobs_keep_their_non_default_settings() {
        let mut g = GlobalDefaults::default();
        g.options.insert("compression_level".into(), 9.into());
        g.options.insert("upload_concurrency".into(), 6.into());
        let t = tpl("t", SettingsPatch { schedule: Some(BackupSchedule::cron(&["0 1 * * *"])), ..opt("upload_concurrency", 3.into()) });
        let mut job = BackupJob { template_ids: vec!["t".into()], ..Default::default() };
        job.options.compression_level = 1;
        let r = resolve(&job, &g, &[t]).unwrap();
        assert_eq!(r.job.options.compression_level, 1, "set by the job (differs from built-in)");
        assert_eq!(r.job.options.upload_concurrency, 3, "left at built-in: inherited");
        assert!(r.job.schedule.is_cron());
        let ex = explicit_paths(&job).unwrap();
        assert!(ex.contains("options.compression_level") && !ex.contains("schedule"));
        // Same result as the older GlobalDefaults::apply without templates.
        let mut old = job.clone();
        old.template_ids.clear();
        let r = resolve(&old, &g, &[]).unwrap();
        let mut applied = old.clone();
        g.apply(&mut applied).unwrap();
        assert_eq!(serde_json::to_value(&r.job.options).unwrap(), serde_json::to_value(&applied.options).unwrap());
    }

    #[test]
    fn per_job_options_are_never_inherited() {
        let mut job = BackupJob { overrides: Some(vec![]), ..Default::default() };
        job.options.prefix = "mine".into();
        job.options.block_size = 8192;
        let r = resolve(&job, &GlobalDefaults::default(), &[]).unwrap();
        assert_eq!(r.job.options.prefix, "mine");
        assert_eq!(r.job.options.block_size, 8192);
        assert_eq!(source_of(&r, "options.prefix").source, SettingSource::Job);
        assert!(tpl("x", opt("block_size", 1.into())).validate().is_err());
        assert!(tpl("x", opt("nope", 1.into())).validate().is_err());
        assert!(tpl("x", opt("usn_policy", "sometimes".into())).validate().is_err());
        assert!(tpl("x", SettingsPatch { schedule: Some(BackupSchedule::cron(&["bad"])), ..Default::default() }).validate().is_err());
        assert!(PolicyTemplate { name: " ".into(), ..Default::default() }.validate().is_err());
    }

    #[test]
    fn job_references_and_portability() {
        let mut org = tpl("org:e", SettingsPatch::default());
        org.managed = true;
        org.enforced = true;
        let all = vec![tpl("a", opt("compression_level", 2.into())), org];
        assert!(validate_job_refs(&BackupJob { template_ids: vec!["a".into()], ..Default::default() }, &all).is_ok());
        assert!(validate_job_refs(&BackupJob { template_ids: vec!["zz".into()], ..Default::default() }, &all).is_err());
        assert!(validate_job_refs(&BackupJob { template_ids: vec!["org:e".into()], ..Default::default() }, &all).is_err());
        assert!(validate_job_refs(&BackupJob { overrides: Some(vec!["options.nope".into()]), ..Default::default() }, &all).is_err());

        let json = export(&all).unwrap();
        let back = import(&json).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.iter().all(|t| !t.managed && !t.enforced));
        assert_eq!(back[0].settings, all[0].settings);
        assert_eq!(import(&serde_json::to_string(&all[0]).unwrap()).unwrap().len(), 1);
        assert!(import("{\"name\":\"\"}").is_err());
        assert!(SettingsPatch::default().is_empty());
        let paths = all[0].settings.paths().unwrap();
        assert_eq!(paths, vec![("options.compression_level".to_string(), Value::from(2))]);
    }
}
