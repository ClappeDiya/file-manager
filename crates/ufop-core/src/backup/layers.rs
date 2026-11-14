//! Layered job settings: which value each setting of a backup job runs
//! with, and where it came from.
//!
//! A setting is addressed by its *field path*: `options.<name>` for each
//! backup option, or a whole section (`filter`, `retention`, `schedule`,
//! `maintenance`, …; [`SECTIONS`]). Layers are applied in order, each
//! replacing the settings it sets:
//!
//! ```text
//! built-in defaults < lower layers (globals, templates in order) < the job's own settings < managed (enforced) layers
//! ```
//!
//! The desktop app and `ufop-server` (policy templates, global defaults)
//! and the standalone `ufop agent` host (organization templates on managed
//! jobs) resolve settings with [`resolve_paths`], so a job inherits the same
//! values wherever it runs. Values are JSON, as the job serializes them.

use super::BackupOptions;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Job sections a template can set as a whole.
pub const SECTIONS: &[&str] = &["filter", "retention", "schedule", "maintenance", "scripts", "notifications", "notification_profile_ids", "notify_on", "throttle", "retry", "prevent_sleep"];

/// Backup options that belong to one job (repository layout, per-run
/// inputs): never inherited, always the job's own.
pub const PER_JOB_OPTIONS: &[&str] = &["prefix", "block_size", "block_hash", "file_hash", "dry_run", "task_config", "changed_paths", "deleted_paths"];

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

/// One layer: the settings it sets, as field path → value.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub source: SettingSource,
    pub paths: Vec<(String, Value)>,
}

/// Every field path of a job: each backup option, then [`SECTIONS`].
pub fn all_paths() -> Vec<String> {
    let opts = serde_json::to_value(BackupOptions::default()).ok();
    let mut v: Vec<String> = opts.as_ref().and_then(|o| o.as_object()).map(|o| o.keys().map(|k| format!("options.{k}")).collect()).unwrap_or_default();
    v.sort();
    v.extend(SECTIONS.iter().map(|s| s.to_string()));
    v
}

/// Whether a field path is a per-job option (never inherited).
pub fn is_per_job(path: &str) -> bool {
    path.strip_prefix("options.").is_some_and(|k| PER_JOB_OPTIONS.contains(&k))
}

/// The value at a field path (`options.x` or a section).
pub fn get_path<'v>(job: &'v Value, path: &str) -> Option<&'v Value> {
    match path.split_once('.') {
        Some((a, b)) => job.get(a)?.get(b),
        None => job.get(path),
    }
}

/// Set the value at a field path (the parent object must exist).
pub fn set_path(job: &mut Value, path: &str, value: Value) {
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

/// A partial settings object (a template's `settings`: an `options` map
/// plus whole sections) as field path → value; absent and `null` sections
/// are not set.
pub fn patch_paths(settings: &Value) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> =
        settings.get("options").and_then(|o| o.as_object()).map(|o| o.iter().map(|(k, v)| (format!("options.{k}"), v.clone())).collect()).unwrap_or_default();
    for s in SECTIONS {
        if let Some(x) = settings.get(*s).filter(|x| !x.is_null()) {
            out.push((s.to_string(), x.clone()));
        }
    }
    out
}

/// The settings a job sets itself: `overrides` when it has the list, else
/// every path whose value differs from `builtin`. Per-job options that
/// differ always count.
pub fn explicit_paths(builtin: &Value, own: &Value, overrides: Option<&[String]>, paths: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in paths {
        let differs = get_path(own, p) != get_path(builtin, p);
        let mine = match overrides {
            Some(list) => list.contains(p),
            None => differs,
        };
        if mine || (is_per_job(p) && differs) {
            out.insert(p.clone());
        }
    }
    out
}

/// Resolve `paths` of a job: built-in value < `below` layers (in order) <
/// the job's own value (when `explicit`) < `managed` layers. Returns the job
/// JSON (`own` with every path replaced by its effective value) and each
/// setting with its source. Per-job options never inherit.
pub fn resolve_paths(builtin: &Value, own: &Value, paths: &[String], explicit: &BTreeSet<String>, below: &[Layer], managed: &[Layer]) -> (Value, Vec<EffectiveSetting>) {
    let find = |layer: &Layer, p: &str| layer.paths.iter().rev().find(|(k, _)| k == p).map(|(_, v)| v.clone());
    let mut out = own.clone();
    let mut settings = Vec::with_capacity(paths.len());
    for p in paths {
        let mut value = get_path(builtin, p).cloned().unwrap_or(Value::Null);
        let mut source = SettingSource::BuiltIn;
        let per_job = is_per_job(p);
        if !per_job {
            for l in below {
                if let Some(v) = find(l, p) {
                    value = v;
                    source = l.source.clone();
                }
            }
        }
        let under = Inherited { value: value.clone(), source: source.clone() };
        let mut inherited = None;
        if explicit.contains(p) {
            value = get_path(own, p).cloned().unwrap_or(Value::Null);
            source = SettingSource::Job;
            inherited = Some(under.clone());
        }
        if !per_job {
            for l in managed {
                if let Some(v) = find(l, p) {
                    value = v;
                    source = l.source.clone();
                    inherited.get_or_insert_with(|| under.clone());
                }
            }
        }
        set_path(&mut out, p, value.clone());
        settings.push(EffectiveSetting { field_path: p.clone(), value, source, inherited });
    }
    (out, settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tpl(id: &str, paths: Vec<(&str, Value)>) -> Layer {
        Layer { source: SettingSource::Template { id: id.into(), name: id.to_uppercase() }, paths: paths.into_iter().map(|(k, v)| (k.to_string(), v)).collect() }
    }

    #[test]
    fn layers_apply_in_order_and_the_job_and_managed_layers_win() {
        let builtin = json!({ "options": { "compression_level": 6, "block_size": 1, "threads": 0 }, "schedule": null, "retention": "keep_all" });
        let own = json!({ "options": { "compression_level": 6, "block_size": 4, "threads": 2 }, "schedule": null, "retention": "keep_all" });
        let paths: Vec<String> = ["options.compression_level", "options.block_size", "options.threads", "schedule", "retention"].iter().map(|s| s.to_string()).collect();
        let explicit = explicit_paths(&builtin, &own, None, &paths);
        assert_eq!(explicit.iter().cloned().collect::<Vec<_>>(), vec!["options.block_size".to_string(), "options.threads".to_string()]);
        let below = [
            tpl("a", vec![("options.compression_level", json!(9)), ("schedule", json!({"cron": ["0 3 * * *"]})), ("options.block_size", json!(64))]),
            tpl("b", vec![("options.compression_level", json!(3)), ("options.threads", json!(8))]),
        ];
        let managed = [Layer { source: SettingSource::Managed { id: "org:x".into(), name: "X".into() }, paths: vec![("retention".into(), json!({"keep_last": 3}))] }];
        let (job, settings) = resolve_paths(&builtin, &own, &paths, &explicit, &below, &managed);
        // Later template wins; the job's own value wins over both.
        assert_eq!(job["options"]["compression_level"], 3);
        assert_eq!(job["options"]["threads"], 2);
        // Per-job options never inherit.
        assert_eq!(job["options"]["block_size"], 4);
        assert_eq!(job["schedule"]["cron"][0], "0 3 * * *");
        assert_eq!(job["retention"]["keep_last"], 3);
        let s = |p: &str| settings.iter().find(|s| s.field_path == p).unwrap().clone();
        assert_eq!(s("options.compression_level").source, SettingSource::Template { id: "b".into(), name: "B".into() });
        assert_eq!(s("options.threads").inherited.unwrap().value, json!(8));
        assert_eq!(s("retention").inherited.unwrap().source, SettingSource::BuiltIn);
        // An overrides list replaces the "differs from built-in" rule.
        let only = explicit_paths(&builtin, &own, Some(&["schedule".to_string()]), &paths);
        assert!(only.contains("schedule") && only.contains("options.block_size") && !only.contains("options.threads"));
    }

    #[test]
    fn patch_paths_lists_options_then_sections() {
        let p = patch_paths(&json!({ "options": { "threads": 2 }, "schedule": { "enabled": true }, "filter": null }));
        assert_eq!(p, vec![("options.threads".to_string(), json!(2)), ("schedule".to_string(), json!({ "enabled": true }))]);
        assert!(all_paths().contains(&"options.compression_level".to_string()) && all_paths().ends_with(&["prevent_sleep".to_string()]));
        assert!(is_per_job("options.prefix") && !is_per_job("schedule"));
    }
}
