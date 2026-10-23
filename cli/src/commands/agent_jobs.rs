//! Settings and schedules of the `ufop agent` host's managed jobs.
//!
//! A fleet policy's job template is resolved the way the desktop app and
//! `ufop-server` resolve a job (`ufop_core::backup::layers`): built-in
//! defaults < the organization templates the job names (`template_ids`,
//! `org:<id>`, in order) < the policy's own settings < enforced
//! organization templates. The CLI runs the backup options, filter,
//! retention, backup schedule and maintenance schedules; other sections
//! (scripts, notifications, throttling, …) are reported as ignored.
//!
//! Schedules are `ufop_core::backup::schedule::BackupSchedule`: interval or
//! cron mode, in the schedule's time zone, with catch-up of a missed slot,
//! exactly as on the desktop. Holiday calendars are not delivered to CLI
//! machines, so calendar references are dropped with a warning.

use serde_json::{json, Map, Value};
use ufop_core::agent::ManagedBackupTemplate;
use ufop_core::backup::layers::{self, Layer, SettingSource};
use ufop_core::backup::schedule::{BackupSchedule, MaintenanceSchedule, MAINTENANCE_TASKS};
use ufop_core::backup::{BackupOptions, RetentionPolicy, SourceFilter};
use ufop_core::error::AppError;

/// Id prefix of organization templates in `template_ids` (as on the desktop).
pub const ORG_PREFIX: &str = "org:";

/// Job sections the CLI agent runs (besides `options.*`).
const CLI_SECTIONS: &[&str] = &["filter", "retention", "schedule", "maintenance"];

/// What a managed job runs with.
#[derive(Debug, Clone, Default)]
pub struct Effective {
    pub options: BackupOptions,
    pub filter: SourceFilter,
    pub retention: RetentionPolicy,
    pub schedule: BackupSchedule,
    pub maintenance: MaintenanceSchedule,
    pub template_ids: Vec<String>,
    pub warnings: Vec<String>,
}

fn builtin() -> Result<Value, AppError> {
    Ok(json!({
        "options": serde_json::to_value(BackupOptions::default())?,
        "filter": serde_json::to_value(SourceFilter::default())?,
        "retention": serde_json::to_value(RetentionPolicy::default())?,
        "schedule": serde_json::to_value(BackupSchedule::default())?,
        "maintenance": serde_json::to_value(MaintenanceSchedule::default())?,
    }))
}

fn paths() -> Vec<String> {
    layers::all_paths().into_iter().filter(|p| p.starts_with("options.") || CLI_SECTIONS.contains(&p.as_str())).collect()
}

fn field<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Result<T, AppError> {
    serde_json::from_value(v.get(k).cloned().unwrap_or(Value::Null)).map_err(|e| AppError::validation(format!("Policy field '{k}': {e}")))
}

/// The policy's own value of a section, on top of the built-in one (an
/// `options` object only names the options it sets).
fn own_settings(base: &Value, fields: &Map<String, Value>) -> Value {
    let mut own = base.clone();
    if let Some(o) = fields.get("options").and_then(|o| o.as_object()) {
        for (k, v) in o {
            layers::set_path(&mut own, &format!("options.{k}"), v.clone());
        }
    }
    for s in CLI_SECTIONS {
        if let Some(v) = fields.get(*s).filter(|v| !v.is_null()) {
            own[*s] = v.clone();
        }
    }
    own
}

fn ignored_sections(keys: impl Iterator<Item = String>, warnings: &mut Vec<String>, from: &str) {
    for k in keys {
        if layers::SECTIONS.contains(&k.as_str()) && !CLI_SECTIONS.contains(&k.as_str()) {
            let w = format!("{from} sets '{k}', which the ufop CLI agent does not support; it is ignored");
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
    }
}

/// Holiday calendars are not available to the CLI agent: drop references.
fn drop_calendars(s: &mut BackupSchedule, what: &str, warnings: &mut Vec<String>) {
    let missing: Vec<String> = s.calendar_ids.iter().filter(|id| !ufop_core::calendar::registry().contains(id)).cloned().collect();
    if !missing.is_empty() {
        warnings.push(format!("The {what} schedule skips the holiday calendars {}, which the ufop CLI agent does not have; it runs on those days", missing.join(", ")));
        s.calendar_ids.retain(|id| !missing.contains(id));
    }
}

/// Resolve a policy's job fields with the organization's templates.
pub fn resolve(fields: &Map<String, Value>, templates: &[ManagedBackupTemplate]) -> Result<Effective, AppError> {
    let base = builtin()?;
    let own = own_settings(&base, fields);
    let paths = paths();
    let overrides: Option<Vec<String>> = match fields.get("overrides") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| AppError::validation(format!("Policy field 'overrides': {e}")))?),
    };
    let explicit = layers::explicit_paths(&base, &own, overrides.as_deref(), &paths);
    let template_ids: Vec<String> = match fields.get("template_ids") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| AppError::validation(format!("Policy field 'template_ids': {e}")))?,
    };
    let mut warnings = Vec::new();
    ignored_sections(fields.keys().cloned(), &mut warnings, "The policy");
    let layer = |t: &ManagedBackupTemplate, warnings: &mut Vec<String>| -> Result<Layer, AppError> {
        t.validate().map_err(AppError::validation)?;
        ignored_sections(t.settings.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default().into_iter(), warnings, &format!("Template '{}'", t.name));
        Ok(Layer { source: SettingSource::Template { id: format!("{ORG_PREFIX}{}", t.id), name: t.name.clone() }, paths: layers::patch_paths(&t.settings) })
    };
    let mut below = Vec::new();
    for id in &template_ids {
        match id.strip_prefix(ORG_PREFIX).and_then(|i| templates.iter().find(|t| t.id == i && !t.enforced)) {
            Some(t) => below.push(layer(t, &mut warnings)?),
            None if id.starts_with(ORG_PREFIX) => warnings.push(format!("Organization template {id} is not available; its settings are not applied")),
            None => warnings.push(format!("Template {id} is a desktop template; the ufop CLI agent applies organization templates only")),
        }
    }
    let mut managed = Vec::new();
    for t in templates.iter().filter(|t| t.enforced) {
        let mut l = layer(t, &mut warnings)?;
        l.source = SettingSource::Managed { id: format!("{ORG_PREFIX}{}", t.id), name: t.name.clone() };
        managed.push(l);
    }
    let (eff, _) = layers::resolve_paths(&base, &own, &paths, &explicit, &below, &managed);
    let mut schedule: BackupSchedule = field(&eff, "schedule")?;
    let mut maintenance: MaintenanceSchedule = field(&eff, "maintenance")?;
    drop_calendars(&mut schedule, "backup", &mut warnings);
    for task in MAINTENANCE_TASKS {
        let s = match task {
            "compact" => maintenance.compact.as_mut(),
            "verify" => maintenance.verify.as_mut().map(|v| &mut v.schedule),
            _ => maintenance.retention.as_mut(),
        };
        if let Some(s) = s {
            drop_calendars(s, task, &mut warnings);
        }
    }
    if schedule.enabled {
        schedule.validate()?;
    }
    maintenance.validate()?;
    let filter: SourceFilter = field(&eff, "filter")?;
    filter.clone().compile()?;
    let retention: RetentionPolicy = field(&eff, "retention")?;
    retention.validate()?;
    Ok(Effective { options: field(&eff, "options")?, filter, retention, schedule, maintenance, template_ids, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tpl(id: &str, enforced: bool, settings: Value) -> ManagedBackupTemplate {
        ManagedBackupTemplate { id: id.into(), name: id.to_uppercase(), enforced, settings, ..Default::default() }
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn templates_fill_in_what_the_policy_leaves_and_enforced_ones_win() {
        let templates = vec![
            tpl("night", false, json!({ "schedule": { "enabled": true, "cron": ["30 1 * * *"], "timezone": "UTC" }, "options": { "compression_level": 9 },
                "maintenance": { "compact": { "enabled": true, "cron": ["0 4 * * Sun"], "timezone": "UTC" } }, "notifications": [] })),
            tpl("keep", true, json!({ "retention": { "type": "keep_count", "count": 7 } })),
        ];
        let fields = obj(json!({ "template_ids": ["org:night", "org:gone", "local-1"], "options": { "upload_concurrency": 2 }, "retention": { "type": "keep_days", "days": 3 } }));
        let e = resolve(&fields, &templates).unwrap();
        assert_eq!(e.schedule.cron, vec!["30 1 * * *".to_string()]);
        assert_eq!(e.schedule.timezone.as_deref(), Some("UTC"));
        assert_eq!(e.options.compression_level, 9);
        assert!(e.maintenance.scheduled("compact"));
        // The enforced template wins over the policy's own retention.
        assert_eq!(serde_json::to_value(&e.retention).unwrap(), json!({ "type": "keep_count", "count": 7 }));
        let w = e.warnings.join("\n");
        assert!(w.contains("org:gone") && w.contains("local-1") && w.contains("'notifications'"), "{w}");

        // The policy's own schedule wins over the template's.
        let fields = obj(json!({ "template_ids": ["org:night"], "schedule": { "enabled": true, "interval_minutes": 120, "start_time": "03:00" } }));
        let e = resolve(&fields, &templates).unwrap();
        assert!(!e.schedule.is_cron() && e.schedule.interval_minutes == 120);
    }

    #[test]
    fn invalid_schedules_are_refused_and_unknown_calendars_dropped() {
        let bad = obj(json!({ "schedule": { "enabled": true, "cron": ["* * * * *"] } }));
        assert!(resolve(&bad, &[]).unwrap_err().user_message().contains("more often"));
        let tz = obj(json!({ "schedule": { "enabled": true, "timezone": "Mars/Base" } }));
        assert!(resolve(&tz, &[]).is_err());
        let cal = obj(json!({ "schedule": { "enabled": true, "calendar_ids": ["cal-x"] } }));
        let e = resolve(&cal, &[]).unwrap();
        assert!(e.schedule.calendar_ids.is_empty() && e.warnings[0].contains("cal-x"));
    }
}
