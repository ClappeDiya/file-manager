//! Notification profiles of the signed-in ufop-server (`ufop server login`),
//! like `backup template`: profiles live on the server, so these commands
//! call its REST API.
//!
//! - `ufop backup notify-profile list|show|create|update|delete|test`
//!
//! Secrets (SMTP password, bot token, webhook URL or token) are sent once
//! with `--secret` / `--secret-env` and stored in the server's credential
//! store; they are never printed or kept in the profile.

use super::server::{show, urlencode, Client};
use crate::OutputFormat;
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum NotifyProfileCommands {
    /// List profiles (local, then the organization's)
    List,
    /// Show one profile (id or name)
    Show { profile: String },
    /// Create a profile from a JSON file, a channel JSON (`--notification`) or a webhook preset
    Create {
        #[command(flatten)]
        fields: ProfileFields,
        /// Webhook preset: pushover, ntfy, discord, pagerduty, gotify or generic
        #[arg(long)]
        preset: Option<String>,
        /// Preset server URL (ntfy, Gotify, generic; optional for the others)
        #[arg(long)]
        server: Option<String>,
        /// ntfy topic
        #[arg(long)]
        topic: Option<String>,
        /// Pushover user or group key
        #[arg(long)]
        user: Option<String>,
        /// ntfy: send the secret as a bearer token
        #[arg(long)]
        bearer: bool,
    },
    /// Change a profile's name, description, channel, severity or secret
    Update {
        profile: String,
        #[command(flatten)]
        fields: ProfileFields,
    },
    /// Delete a profile (`--force` also removes it from the jobs and templates using it)
    Delete {
        profile: String,
        #[arg(long)]
        force: bool,
    },
    /// Send a test message through a profile
    Test { profile: String },
}

#[derive(clap::Args, Debug, Default)]
pub struct ProfileFields {
    /// Profile JSON (`{"name": …, "notification": {…}}`)
    #[arg(long)]
    pub file: Option<PathBuf>,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// Channel JSON (e.g. `{"type":"email","host":…,"from":…,"to":[…]}`)
    #[arg(long)]
    pub notification: Option<String>,
    /// Send results at least this severe: success, warning or error (`none` clears it)
    #[arg(long)]
    pub min_severity: Option<String>,
    /// Channel secret to store (prefer --secret-env: arguments show in process lists)
    #[arg(long)]
    pub secret: Option<String>,
    /// Read the channel secret from this environment variable
    #[arg(long)]
    pub secret_env: Option<String>,
}

const PRESETS: &[&str] = &["generic", "pushover", "ntfy", "discord", "pagerduty", "gotify"];

/// A profile body from `base`, `--file` and the flags (flags win).
pub fn profile_body(base: Value, f: &ProfileFields, env: impl Fn(&str) -> Option<String>) -> Result<Value, String> {
    let mut p = match &f.file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", path.display()))?
        }
        None => base,
    };
    if !p.is_object() {
        p = json!({});
    }
    if let Some(n) = &f.name {
        p["name"] = json!(n);
    }
    if let Some(d) = &f.description {
        p["description"] = json!(d);
    }
    if let Some(n) = &f.notification {
        let mut n: Value = serde_json::from_str(n).map_err(|e| format!("--notification is not JSON: {e}"))?;
        // New email channels send HTML unless told otherwise (like the desktop app).
        if n["type"] == "email" && n.get("html").is_none() {
            n["html"] = json!(true);
        }
        p["notification"] = n;
    }
    if let Some(s) = &f.min_severity {
        p["min_severity"] = match s.as_str() {
            "none" | "" => Value::Null,
            "success" | "warning" | "error" => json!(s),
            other => return Err(format!("Unknown severity '{other}' (success, warning, error or none)")),
        };
    }
    let secret = match (&f.secret, &f.secret_env) {
        (Some(s), _) => Some(s.clone()),
        (None, Some(var)) => Some(env(var).ok_or_else(|| format!("Environment variable {var} is not set"))?),
        _ => None,
    };
    if let Some(s) = secret {
        p["secret"] = json!(s);
    }
    Ok(p)
}

async fn profile_id(c: &mut Client, id_or_name: &str) -> Result<String, String> {
    let list = c.call(Method::GET, "/notification-profiles", None).await?;
    list.as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"] == id_or_name || p["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(id_or_name)))
        .and_then(|p| p["id"].as_str().map(String::from))
        .ok_or_else(|| format!("No notification profile named or with id '{id_or_name}'"))
}

/// One line per profile: id, name, channel, filter.
pub fn list_text(list: &Value) -> String {
    let mut out = String::new();
    for p in list.as_array().into_iter().flatten() {
        let n = &p["notification"];
        let mut kind = n["type"].as_str().unwrap_or_default().to_string();
        if let Some(pr) = n["preset"].as_str().filter(|p| *p != "generic") {
            kind = format!("{kind}/{pr}");
        }
        let filter = match p["min_severity"].as_str() {
            Some(s) => format!("≥ {s}"),
            None => n["send_on"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")).unwrap_or_default(),
        };
        let org = if p["managed"].as_bool() == Some(true) { " [organization]" } else { "" };
        out.push_str(&format!("{}  {}  ({kind}; {filter}){org}\n", p["id"].as_str().unwrap_or_default(), p["name"].as_str().unwrap_or_default()));
    }
    out
}

pub async fn notify_profile(cmd: NotifyProfileCommands, format: &OutputFormat) -> Result<u8, String> {
    let mut c = Client::new()?;
    let env = |k: &str| std::env::var(k).ok();
    let v = match cmd {
        NotifyProfileCommands::List => {
            let list = c.call(Method::GET, "/notification-profiles", None).await?;
            if matches!(format, OutputFormat::Human) {
                print!("{}", list_text(&list));
                return Ok(0);
            }
            list
        }
        NotifyProfileCommands::Show { profile } => {
            let id = profile_id(&mut c, &profile).await?;
            c.call(Method::GET, &format!("/notification-profiles/{}", urlencode(&id)), None).await?
        }
        NotifyProfileCommands::Create { fields, preset, server, topic, user, bearer } => {
            let mut base = json!({});
            if let Some(p) = preset {
                if !PRESETS.contains(&p.as_str()) {
                    return Err(format!("Unknown preset '{p}' ({})", PRESETS.join(", ")));
                }
                let params = json!({ "server": server.unwrap_or_default(), "topic": topic.unwrap_or_default(), "user": user.unwrap_or_default(), "bearer": bearer });
                base["notification"] = c.call(Method::POST, "/notifications/webhook-preset", Some(json!({ "preset": p, "params": params }))).await?;
            }
            let mut body = profile_body(base, &fields, env)?;
            if let Some(o) = body.as_object_mut() {
                o.remove("id");
            }
            if body.get("notification").is_none() {
                return Err("Give the channel with --notification, --file or --preset".into());
            }
            c.call(Method::POST, "/notification-profiles", Some(body)).await?
        }
        NotifyProfileCommands::Update { profile, fields } => {
            let id = profile_id(&mut c, &profile).await?;
            let current = c.call(Method::GET, &format!("/notification-profiles/{}", urlencode(&id)), None).await?;
            let body = profile_body(current, &fields, env)?;
            c.call(Method::PUT, &format!("/notification-profiles/{}", urlencode(&id)), Some(body)).await?
        }
        NotifyProfileCommands::Delete { profile, force } => {
            let id = profile_id(&mut c, &profile).await?;
            let r = c.call(Method::DELETE, &format!("/notification-profiles/{}?force={force}", urlencode(&id)), None).await?;
            if matches!(format, OutputFormat::Human) {
                for w in r["warnings"].as_array().into_iter().flatten() {
                    eprintln!("warning: {}", w.as_str().unwrap_or_default());
                }
                println!("Deleted notification profile {id}");
                return Ok(0);
            }
            r
        }
        NotifyProfileCommands::Test { profile } => {
            let id = profile_id(&mut c, &profile).await?;
            c.call(Method::POST, &format!("/notification-profiles/{}/test", urlencode(&id)), None).await?;
            if matches!(format, OutputFormat::Human) {
                println!("Test message sent through {profile}");
                return Ok(0);
            }
            json!({ "ok": true })
        }
    };
    show(format, &v);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_from_flags_and_secret_env() {
        let f = ProfileFields {
            name: Some("Ops".into()),
            notification: Some(r#"{"type":"telegram","chat_id":"-1"}"#.into()),
            min_severity: Some("warning".into()),
            secret_env: Some("TOKEN_VAR".into()),
            ..Default::default()
        };
        let b = profile_body(json!({ "id": "x", "description": "old" }), &f, |k| (k == "TOKEN_VAR").then(|| "123:ABC".to_string())).unwrap();
        assert_eq!(b, json!({ "id": "x", "description": "old", "name": "Ops", "notification": { "type": "telegram", "chat_id": "-1" }, "min_severity": "warning", "secret": "123:ABC" }));
        let none = profile_body(b.clone(), &ProfileFields { min_severity: Some("none".into()), ..Default::default() }, |_| None).unwrap();
        assert!(none["min_severity"].is_null());
        assert!(profile_body(json!({}), &ProfileFields { min_severity: Some("info".into()), ..Default::default() }, |_| None).is_err());
        assert!(profile_body(json!({}), &ProfileFields { secret_env: Some("MISSING".into()), ..Default::default() }, |_| None).is_err());
        assert!(profile_body(json!({}), &ProfileFields { notification: Some("{bad".into()), ..Default::default() }, |_| None).is_err());
        let mail = profile_body(json!({}), &ProfileFields { notification: Some(r#"{"type":"email","host":"h"}"#.into()), ..Default::default() }, |_| None).unwrap();
        assert_eq!(mail["notification"]["html"], true);
        let plain = profile_body(json!({}), &ProfileFields { notification: Some(r#"{"type":"email","html":false}"#.into()), ..Default::default() }, |_| None).unwrap();
        assert_eq!(plain["notification"]["html"], false);
    }

    #[test]
    fn list_lines() {
        let list = json!([
            { "id": "a", "name": "Pager", "min_severity": "error", "managed": false, "notification": { "type": "webhook", "preset": "pagerduty", "send_on": ["success"] } },
            { "id": "org:b", "name": "Mail", "min_severity": null, "managed": true, "notification": { "type": "email", "send_on": ["warning", "error"] } }
        ]);
        assert_eq!(list_text(&list), "a  Pager  (webhook/pagerduty; ≥ error)\norg:b  Mail  (email; warning,error) [organization]\n");
    }
}
