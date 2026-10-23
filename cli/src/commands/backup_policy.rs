//! Policy templates, effective settings and schedule previews of the
//! signed-in ufop-server (`ufop server login`), like `backup run-all`:
//! jobs and templates live on the server, so these commands call its REST API.
//!
//! - `ufop backup template list|show|create|update|delete|apply|export|import`
//! - `ufop backup effective <job>`: each setting and where it comes from
//! - `ufop backup schedule-preview --cron …`: validate and list next runs

use super::server::{show, urlencode, Client};
use crate::output;
use crate::OutputFormat;
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum TemplateCommands {
    /// List templates (local, then the organization's)
    List,
    /// Show one template (id or name)
    Show { template: String },
    /// Create a template from a JSON file (`{"name": …, "settings": {…}}`) or flags
    Create {
        /// Template JSON (an exported template works too)
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Settings JSON (the template's `settings` object)
        #[arg(long)]
        settings: Option<String>,
    },
    /// Replace a template's name, description or settings
    Update {
        template: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        settings: Option<String>,
    },
    /// Delete a template no job uses
    Delete { template: String },
    /// Base a job on a template too (it wins over the job's earlier templates)
    Apply {
        template: String,
        /// Job id or name
        #[arg(long)]
        job: String,
    },
    /// Write templates as a JSON file (default: all, to standard output)
    Export {
        /// Only these templates (id or name, repeatable)
        #[arg(long = "template")]
        templates: Vec<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Import a template file (same id or name updates)
    Import { file: PathBuf },
}

fn read_json(path: &PathBuf) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", path.display()))
}

/// A template body from `--file` and the flags (flags win).
fn template_body(base: Value, file: Option<&PathBuf>, name: Option<String>, description: Option<String>, settings: Option<String>) -> Result<Value, String> {
    let mut t = match file {
        Some(f) => {
            let v = read_json(f)?;
            // An export file holds a list: take its only template.
            match v.get("templates").and_then(|t| t.as_array()) {
                Some(list) if list.len() == 1 => list[0].clone(),
                Some(_) => return Err("The file holds several templates; use `template import`".into()),
                None => v,
            }
        }
        None => base,
    };
    if !t.is_object() {
        t = json!({});
    }
    if let Some(n) = name {
        t["name"] = json!(n);
    }
    if let Some(d) = description {
        t["description"] = json!(d);
    }
    if let Some(s) = settings {
        t["settings"] = serde_json::from_str(&s).map_err(|e| format!("--settings is not JSON: {e}"))?;
    }
    Ok(t)
}

/// A template's id from its id or name.
async fn template_id(c: &mut Client, id_or_name: &str) -> Result<String, String> {
    let list = c.call(Method::GET, "/backup-templates", None).await?;
    list.as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"] == id_or_name || t["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(id_or_name)))
        .and_then(|t| t["id"].as_str().map(String::from))
        .ok_or_else(|| format!("No template named or with id '{id_or_name}'"))
}

/// Human table of effective settings: path, source, value.
pub fn effective_text(v: &Value) -> String {
    let mut out = String::new();
    for w in v["warnings"].as_array().into_iter().flatten() {
        out.push_str(&format!("warning: {}\n", w.as_str().unwrap_or_default()));
    }
    out.push_str(&format!("{:<36}  {:<24}  {}\n", "Setting", "From", "Value"));
    for s in v["settings"].as_array().into_iter().flatten() {
        let src = &s["source"];
        let from = match src["type"].as_str().unwrap_or_default() {
            "built_in" => "built-in default".to_string(),
            "global" => "global defaults".to_string(),
            "job" => "this job".to_string(),
            "template" => format!("template {}", src["name"].as_str().unwrap_or_default()),
            "managed" => format!("organization {}", src["name"].as_str().unwrap_or_default()),
            other => other.to_string(),
        };
        let mut value = s["value"].to_string();
        if value.chars().count() > 60 {
            value = format!("{}…", value.chars().take(59).collect::<String>());
        }
        out.push_str(&format!("{:<36}  {:<24}  {value}\n", s["field_path"].as_str().unwrap_or_default(), from));
    }
    out
}

fn print(format: &OutputFormat, v: &Value) {
    show(format, v)
}

pub async fn template(cmd: TemplateCommands, format: &OutputFormat) -> Result<u8, String> {
    let mut c = Client::new()?;
    let v = match cmd {
        TemplateCommands::List => {
            let list = c.call(Method::GET, "/backup-templates", None).await?;
            if matches!(format, OutputFormat::Human) {
                for t in list.as_array().into_iter().flatten() {
                    let flags = match (t["managed"].as_bool(), t["enforced"].as_bool()) {
                        (Some(true), Some(true)) => " [organization, enforced]",
                        (Some(true), _) => " [organization]",
                        _ => "",
                    };
                    println!("{}  {}{flags}", t["id"].as_str().unwrap_or_default(), t["name"].as_str().unwrap_or_default());
                }
                return Ok(0);
            }
            list
        }
        TemplateCommands::Show { template } => {
            let id = template_id(&mut c, &template).await?;
            c.call(Method::GET, &format!("/backup-templates/{}", urlencode(&id)), None).await?
        }
        TemplateCommands::Create { file, name, description, settings } => {
            let mut body = template_body(json!({}), file.as_ref(), name, description, settings)?;
            if let Some(o) = body.as_object_mut() {
                o.remove("id");
            }
            c.call(Method::POST, "/backup-templates", Some(body)).await?
        }
        TemplateCommands::Update { template, file, name, description, settings } => {
            let id = template_id(&mut c, &template).await?;
            let current = c.call(Method::GET, &format!("/backup-templates/{}", urlencode(&id)), None).await?;
            let body = template_body(current, file.as_ref(), name, description, settings)?;
            c.call(Method::PUT, &format!("/backup-templates/{}", urlencode(&id)), Some(body)).await?
        }
        TemplateCommands::Delete { template } => {
            let id = template_id(&mut c, &template).await?;
            c.call(Method::DELETE, &format!("/backup-templates/{}", urlencode(&id)), None).await?
        }
        TemplateCommands::Apply { template, job } => {
            let id = template_id(&mut c, &template).await?;
            let job_id = c.job_id(&job).await?;
            c.call(Method::POST, &format!("/backup-templates/{}/apply", urlencode(&id)), Some(json!({ "job_id": job_id }))).await?
        }
        TemplateCommands::Export { templates, out } => {
            let mut ids = Vec::new();
            for t in &templates {
                ids.push(template_id(&mut c, t).await?);
            }
            let r = c.raw(Method::GET, &format!("/backup-templates/export?ids={}", urlencode(&ids.join(",")))).send().await.map_err(|e| format!("Server unreachable: {e}"))?;
            if !r.status().is_success() {
                return Err(format!("Export failed: {}", r.status()));
            }
            let text = r.text().await.map_err(|e| e.to_string())?;
            match out {
                Some(p) => {
                    std::fs::write(&p, &text).map_err(|e| format!("Cannot write {}: {e}", p.display()))?;
                    output::print_success(format, &format!("Templates written to {}", p.display()));
                }
                None => println!("{text}"),
            }
            return Ok(0);
        }
        TemplateCommands::Import { file } => {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("Cannot read {}: {e}", file.display()))?;
            let r = c.raw(Method::POST, "/backup-templates/import").header("Content-Type", "application/json").body(text).send().await.map_err(|e| format!("Server unreachable: {e}"))?;
            let ok = r.status().is_success();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            if !ok {
                return Err(v["message"].as_str().unwrap_or("Import failed").to_string());
            }
            v
        }
    };
    print(format, &v);
    Ok(0)
}

pub async fn effective(job: &str, format: &OutputFormat) -> Result<u8, String> {
    let mut c = Client::new()?;
    let id = c.job_id(job).await?;
    let v = c.call(Method::GET, &format!("/jobs/{}/effective-settings", urlencode(&id)), None).await?;
    match format {
        OutputFormat::Human => print!("{}", effective_text(&v)),
        _ => output::print_data(format, &v),
    }
    Ok(0)
}

/// `ufop backup hardening <job> [--probe]`.
pub async fn hardening(job: &str, probe: bool, format: &OutputFormat) -> Result<u8, String> {
    let mut c = Client::new()?;
    let id = c.job_id(job).await?;
    let v = if probe {
        c.call(Method::POST, &format!("/jobs/{}/hardening/probe", urlencode(&id)), None).await?
    } else {
        c.call(Method::GET, &format!("/jobs/{}/hardening", urlencode(&id)), None).await?
    };
    match format {
        OutputFormat::Human => print!("{}", hardening_text(&v)),
        _ => output::print_data(format, &v),
    }
    Ok(hardening_exit(&v))
}

/// 0 = every check passes, 1 = warnings or unchecked, 2 = a failure.
pub fn hardening_exit(v: &Value) -> u8 {
    let st: Vec<&str> = v["checks"].as_array().into_iter().flatten().filter_map(|c| c["status"].as_str()).collect();
    if st.contains(&"fail") {
        2
    } else if st.iter().any(|s| *s == "warn" || *s == "unknown") {
        1
    } else {
        0
    }
}

pub fn hardening_text(v: &Value) -> String {
    let mut out = format!("Protection: {}/{}\n", v["passed"].as_u64().unwrap_or(0), v["total"].as_u64().unwrap_or(0));
    for c in v["checks"].as_array().into_iter().flatten() {
        let mark = match c["status"].as_str().unwrap_or("") {
            "pass" => "PASS",
            "warn" => "WARN",
            "fail" => "FAIL",
            "unknown" => "  ? ",
            _ => "info",
        };
        out.push_str(&format!("[{mark}] {}: {}\n", c["title"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or("")));
        if let Some(a) = c["advice"].as_str().filter(|a| !a.is_empty() && c["status"] != "pass") {
            out.push_str(&format!("       → {a} ({})\n", c["doc"].as_str().unwrap_or("")));
        }
    }
    if let Some(l) = v["probe"]["leftover"].as_str() {
        out.push_str(&format!("Probe object left at the destination: {l}\n"));
    }
    out
}

/// Body of `POST /jobs/{id}/point-in-time`; the time must be RFC 3339.
pub fn open_at_body(point_in_time: &str, name: Option<String>) -> Result<Value, String> {
    let at = chrono::DateTime::parse_from_rfc3339(point_in_time.trim()).map_err(|e| format!("--point-in-time must be an RFC 3339 time such as 2026-10-01T08:00:00Z: {e}"))?;
    Ok(json!({ "at": at.with_timezone(&chrono::Utc).to_rfc3339(), "name": name }))
}

/// `ufop backup open-at`: a read-only job reading the job's S3 bucket as of a time.
pub async fn open_at(job: &str, point_in_time: &str, name: Option<String>, format: &OutputFormat) -> Result<u8, String> {
    let body = open_at_body(point_in_time, name)?;
    let mut c = Client::new()?;
    let id = c.job_id(job).await?;
    let v = c.call(Method::POST, &format!("/jobs/{}/point-in-time", urlencode(&id)), Some(body)).await?;
    match format {
        OutputFormat::Human => println!(
            "Created read-only job \"{}\" ({}) with {} version(s) as of {}. Restore from it with the server or the app.",
            v["name"].as_str().unwrap_or(""),
            v["id"].as_str().unwrap_or(""),
            v["stats"]["version_count"].as_u64().unwrap_or(0),
            point_in_time.trim()
        ),
        _ => output::print_data(format, &v),
    }
    Ok(0)
}

/// Body of a schedule preview: cron expressions, or an interval schedule.
pub fn preview_body(cron: &[String], start_time: Option<&str>, interval_minutes: Option<u32>, timezone: Option<&str>, count: usize) -> Value {
    let mut s = json!({ "enabled": true, "cron": cron });
    if let Some(z) = timezone {
        s["timezone"] = json!(z);
    }
    if let Some(t) = start_time {
        s["start_time"] = json!(t);
    }
    if let Some(i) = interval_minutes {
        s["interval_minutes"] = json!(i);
    }
    json!({ "schedule": s, "count": count })
}

pub async fn schedule_preview(cron: &[String], start_time: Option<&str>, interval_minutes: Option<u32>, timezone: Option<&str>, count: usize, format: &OutputFormat) -> Result<u8, String> {
    let mut c = Client::new()?;
    let v = c.call(Method::POST, "/backup-schedule/preview", Some(preview_body(cron, start_time, interval_minutes, timezone, count))).await?;
    match format {
        OutputFormat::Human => {
            for r in v.as_array().into_iter().flatten() {
                let line = preview_line(r, timezone.is_some_and(|z| !z.trim().is_empty() && !z.trim().eq_ignore_ascii_case("local")));
                println!("{line}");
            }
        }
        _ => output::print_data(format, &v),
    }
    Ok(0)
}

/// One "next run" line: the time in the schedule's zone (and this
/// computer's equivalent when the zone is not local), plus a skipping calendar.
fn preview_line(r: &Value, other_zone: bool) -> String {
    let at = r["at"].as_str().unwrap_or_default();
    let local = chrono::DateTime::parse_from_rfc3339(at).map(|d| d.with_timezone(&chrono::Local).format("%a %Y-%m-%d %H:%M").to_string()).unwrap_or_else(|_| at.to_string());
    let mut line = match r["zoned"].as_str().and_then(|z| chrono::DateTime::parse_from_rfc3339(z).ok()) {
        Some(z) if other_zone => format!("{}  (this computer: {local})", z.format("%a %Y-%m-%d %H:%M %:z")),
        _ => local,
    };
    if let Some(c) = r["skipped_by"].as_str() {
        line.push_str(&format!("  (skipped: {c})"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardening_output_and_exit_codes() {
        let v = json!({"passed": 1, "total": 2, "checks": [
            {"id": "encryption", "title": "Backups are encrypted", "status": "pass", "detail": "Yes.", "advice": "", "doc": "g#a"},
            {"id": "credentials", "title": "The backup credentials cannot delete", "status": "unknown", "detail": "Not checked yet.", "advice": "Run the checks.", "doc": "g#b"},
            {"id": "legal_hold", "title": "Legal hold", "status": "info", "detail": "No legal hold.", "advice": "Place one.", "doc": "g#c"}
        ], "probe": {"leftover": "ufop-hardening-probe-1.tmp"}});
        let t = hardening_text(&v);
        assert!(t.starts_with("Protection: 1/2\n"));
        assert!(t.contains("[PASS] Backups are encrypted: Yes."));
        assert!(t.contains("→ Run the checks. (g#b)"));
        assert!(t.contains("left at the destination: ufop-hardening-probe-1.tmp"));
        assert_eq!(hardening_exit(&v), 1);
        let mut f = v.clone();
        f["checks"][1]["status"] = json!("fail");
        assert_eq!(hardening_exit(&f), 2);
        f["checks"][1]["status"] = json!("pass");
        assert_eq!(hardening_exit(&f), 0, "info does not count");
    }

    #[test]
    fn template_bodies_merge_file_and_flags() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.json");
        std::fs::write(&f, r#"{"format":"ufop-backup-templates","version":1,"templates":[{"id":"x","name":"From file","settings":{"retention":{"type":"keep_all"}}}]}"#).unwrap();
        let b = template_body(json!({}), Some(&f), Some("Renamed".into()), None, Some(r#"{"options":{"compression_level":3}}"#.into())).unwrap();
        assert_eq!(b["name"], "Renamed");
        assert_eq!(b["settings"]["options"]["compression_level"], 3);
        assert!(template_body(json!({}), None, None, None, Some("not json".into())).is_err());
        let b = template_body(json!({"id": "a", "name": "Old", "settings": {}}), None, None, Some("New text".into()), None).unwrap();
        assert_eq!((b["id"].as_str(), b["description"].as_str()), (Some("a"), Some("New text")));
    }

    #[test]
    fn open_at_body_needs_rfc3339() {
        let b = open_at_body("2026-10-01T10:00:00+02:00", Some("Before attack".into())).unwrap();
        assert_eq!(b["at"], "2026-10-01T08:00:00+00:00");
        assert_eq!(b["name"], "Before attack");
        assert!(open_at_body("yesterday", None).unwrap_err().contains("RFC 3339"));
    }

    #[test]
    fn effective_table_names_sources() {
        let v = json!({"warnings": ["Policy template x no longer exists"], "settings": [
            {"field_path": "retention", "value": {"type": "keep_count", "count": 4}, "source": {"type": "template", "id": "t", "name": "Servers"}},
            {"field_path": "options.compression_level", "value": 7, "source": {"type": "job"}},
            {"field_path": "options.hash_threads", "value": 8, "source": {"type": "managed", "id": "org:x", "name": "Org"}}]});
        let t = effective_text(&v);
        assert!(t.contains("warning: Policy template x"));
        assert!(t.contains("template Servers"));
        assert!(t.contains("this job"));
        assert!(t.contains("organization Org"));
    }

    #[test]
    fn preview_bodies() {
        let b = preview_body(&["30 2 * * *".into()], None, None, None, 5);
        assert_eq!(b["schedule"]["cron"][0], "30 2 * * *");
        assert_eq!(b["count"], 5);
        assert!(b["schedule"].get("timezone").is_none());
        let b = preview_body(&[], Some("03:00"), Some(60), Some("+02:00"), 3);
        assert_eq!((b["schedule"]["start_time"].as_str(), b["schedule"]["interval_minutes"].as_u64()), (Some("03:00"), Some(60)));
        assert_eq!(b["schedule"]["timezone"], "+02:00");
    }

    #[test]
    fn preview_lines_show_the_zone_time() {
        let r = json!({"at": "2026-03-17T00:30:00+00:00", "zoned": "2026-03-17T02:30:00+02:00", "skipped_by": "Holidays"});
        let line = preview_line(&r, true);
        assert!(line.starts_with("Tue 2026-03-17 02:30 +02:00  (this computer: "), "{line}");
        assert!(line.ends_with("(skipped: Holidays)"), "{line}");
        let plain = preview_line(&json!({"at": "2026-03-17T00:30:00+00:00", "skipped_by": null}), false);
        assert!(!plain.contains("this computer") && !plain.contains("skipped"), "{plain}");
    }
}
