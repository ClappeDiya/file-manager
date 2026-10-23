//! App settings an organization manages from the console: update channel,
//! automatic checks and a blackout window, the proxy, and AI. A `None`
//! section (or field) leaves that setting to the user.

use serde::{Deserialize, Serialize};

pub const CHANNELS: [&str; 2] = ["stable", "beta"];
pub const PROXY_MODES: [&str; 4] = ["system", "none", "manual", "pac"];
/// rclone VFS cache modes, from none to whole files kept for offline use.
pub const MOUNT_CACHE_MODES: [&str; 4] = ["off", "minimal", "writes", "full"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedSettings {
    /// Increases with every change in the console.
    pub version: u64,
    pub updates: Option<ManagedUpdates>,
    pub proxy: Option<ManagedProxy>,
    pub ai: Option<ManagedAi>,
    /// Machine-wide caps for backups and restores.
    pub transfers: Option<ManagedTransfers>,
    /// Rules for drive mounts (the mount profile).
    pub mounts: Option<ManagedMounts>,
    /// Automation rules (workflows) the organization owns on its desktops.
    pub workflows: Option<Vec<ManagedWorkflow>>,
    /// Backup policy templates the organization offers (or enforces) on its desktops.
    pub backup_templates: Option<Vec<ManagedBackupTemplate>>,
    /// Accounts and per-job access rules for `ufop-server` nodes (read-only there).
    pub server_access: Option<ManagedServerAccess>,
}

/// Most organization accounts and access rules per server.
pub const MAX_SERVER_USERS: usize = 500;
pub const MAX_SERVER_RULES: usize = 2000;
pub const SERVER_ROLES: [&str; 3] = ["admin", "operator", "viewer"];
pub const SERVER_PERMISSIONS: [&str; 5] = ["none", "read", "run", "write", "full"];

/// Server accounts and access rules the organization provisions on its
/// `ufop-server` nodes. Passwords travel only as Argon2 hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedServerAccess {
    pub users: Vec<ManagedServerUser>,
    pub rules: Vec<ManagedAclRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedServerUser {
    pub username: String,
    /// Argon2 PHC string (`$argon2id$v=19$…`), e.g. from `ufop-server hash-password`.
    pub password_hash: String,
    /// `admin`, `operator` or `viewer`.
    pub role: String,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedAclRule {
    /// A user name, `role:<role>` or `*`.
    pub principal: String,
    /// A job id, or a glob (`*`, `?`) over job ids and names.
    pub job_pattern: String,
    /// `none`, `read`, `run`, `write` or `full`.
    pub permission: String,
}

impl ManagedServerAccess {
    pub fn validate(&self) -> Result<(), String> {
        if self.users.len() > MAX_SERVER_USERS || self.rules.len() > MAX_SERVER_RULES {
            return Err(format!("At most {MAX_SERVER_USERS} server accounts and {MAX_SERVER_RULES} access rules"));
        }
        let mut names = std::collections::HashSet::new();
        for u in &self.users {
            let n = u.username.trim();
            if n.is_empty() || n.len() > 64 || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@-".contains(&b)) {
                return Err(format!("Server account name '{n}' must be 1–64 letters, digits, '.', '_', '@' or '-'"));
            }
            if !names.insert(n.to_ascii_lowercase()) {
                return Err(format!("Two server accounts are named '{n}'"));
            }
            if !u.password_hash.starts_with("$argon2") {
                return Err(format!("Server account '{n}' needs an Argon2 password hash ($argon2id$…), not a password"));
            }
            if !SERVER_ROLES.contains(&u.role.as_str()) {
                return Err(format!("Server account '{n}' has an unknown role '{}' (admin, operator or viewer)", u.role));
            }
        }
        for r in &self.rules {
            if r.principal.trim().is_empty() || r.job_pattern.trim().is_empty() {
                return Err("Every access rule needs a principal and a job pattern".into());
            }
            if !SERVER_PERMISSIONS.contains(&r.permission.as_str()) {
                return Err(format!("Unknown access permission '{}' (none, read, run, write or full)", r.permission));
            }
        }
        Ok(())
    }
}

/// Most organization backup templates, and the largest one (JSON bytes).
pub const MAX_BACKUP_TEMPLATES: usize = 50;
pub const MAX_BACKUP_TEMPLATE_BYTES: usize = 64 * 1024;

/// A backup policy template owned by the organization: read-only on each
/// desktop. Jobs may choose an ordinary one; an `enforced` one applies to
/// every job and wins over the job's own settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedBackupTemplate {
    /// Stable id (letters, digits, `-`, `_`).
    pub id: String,
    pub name: String,
    pub description: String,
    pub enforced: bool,
    /// The settings as the desktop app exports a template's `settings`
    /// (snake_case keys; checked in full by the desktop).
    pub settings: serde_json::Value,
}

impl ManagedBackupTemplate {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > 64 || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("Backup template id '{}' must be 1–64 letters, digits, '-' or '_'", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 200 {
            return Err(format!("Backup template {} needs a name (up to 200 characters)", self.id));
        }
        if !self.settings.is_object() {
            return Err(format!("Backup template '{}' settings must be an object (export a template from the desktop app)", self.name));
        }
        if serde_json::to_vec(&self.settings).map(|v| v.len()).unwrap_or(usize::MAX) > MAX_BACKUP_TEMPLATE_BYTES {
            return Err(format!("Backup template '{}' is larger than {} KB", self.name, MAX_BACKUP_TEMPLATE_BYTES / 1024));
        }
        // Scripts would run programs on every desktop: a decision for each desktop.
        if self.settings.get("scripts").is_some_and(|s| !s.is_null()) {
            return Err(format!("Backup template '{}' sets scripts; organization templates cannot", self.name));
        }
        Ok(())
    }
}

/// Most organization workflows per desktop, and the largest one (JSON bytes).
pub const MAX_WORKFLOWS: usize = 50;
pub const MAX_WORKFLOW_BYTES: usize = 256 * 1024;
/// Actions an organization workflow may not contain: running programs is
/// a decision for each desktop, not one pushed from the console.
pub const FORBIDDEN_WORKFLOW_ACTIONS: [&str; 2] = ["run_command", "execute_custom_command"];

/// An automation rule owned by the organization: installed read-only on
/// every desktop (people can run it, not change it) and removed when the
/// organization drops it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedWorkflow {
    /// Stable id (letters, digits, `-`, `_`).
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// The rule as the desktop app exports it (`AutomationRule` JSON).
    pub rule: serde_json::Value,
}

impl ManagedWorkflow {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > 64 || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("Workflow id '{}' must be 1–64 letters, digits, '-' or '_'", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 200 {
            return Err(format!("Workflow {} needs a name (up to 200 characters)", self.id));
        }
        if !self.rule.is_object() || self.rule.get("trigger").is_none() || self.rule.get("action").is_none() {
            return Err(format!("Workflow '{}' is not an automation rule (export it from the desktop app)", self.name));
        }
        if serde_json::to_vec(&self.rule).map(|v| v.len()).unwrap_or(usize::MAX) > MAX_WORKFLOW_BYTES {
            return Err(format!("Workflow '{}' is larger than {} KB", self.name, MAX_WORKFLOW_BYTES / 1024));
        }
        if let Some(t) = forbidden_action(&self.rule) {
            return Err(format!("Workflow '{}' runs a program ({t}); organization workflows cannot", self.name));
        }
        Ok(())
    }
}

/// The first forbidden action type anywhere in a rule's JSON.
fn forbidden_action(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(t) = m.get("type").and_then(|t| t.as_str()).filter(|t| FORBIDDEN_WORKFLOW_ACTIONS.contains(t)) {
                return Some(t.to_string());
            }
            m.values().find_map(forbidden_action)
        }
        serde_json::Value::Array(a) => a.iter().find_map(forbidden_action),
        _ => None,
    }
}

/// The organization's mount profile: what may be mounted and how.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedMounts {
    /// Protocols that may be mounted (`None` = any the app supports).
    pub allowed_protocols: Option<Vec<String>>,
    /// Mount everything read-only.
    pub read_only: Option<bool>,
    /// Cache mode every mount uses (`off`, `minimal`, `writes`, `full`).
    pub cache_mode: Option<String>,
    /// Largest cache per mount, in MB.
    pub max_cache_mb: Option<u64>,
    /// Cached files older than this are dropped, in hours.
    pub max_cache_age_hours: Option<u32>,
    /// Mount points must be inside this folder (`~` = the user's home).
    pub mount_root: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedTransfers {
    /// KiB/s for all uploads together; `None` = the machine's own setting, 0 = unlimited.
    pub max_upload_kbps: Option<u64>,
    pub max_download_kbps: Option<u64>,
    /// Operations that may run at the same time (1–8).
    pub max_concurrent_runs: Option<u32>,
}

pub const MAX_CONCURRENT_RUNS: u32 = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedUpdates {
    /// `stable` or `beta`.
    pub channel: Option<String>,
    pub auto_check: Option<bool>,
    /// No update is downloaded or installed inside this window.
    pub blackout: Option<BlackoutWindow>,
}

/// A weekly window in the machine's local time, e.g. weekdays 08:00–18:00.
/// `end` before `start` spans midnight (the day is the one it starts on).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BlackoutWindow {
    /// `HH:MM`.
    pub start: String,
    pub end: String,
    /// 0 = Sunday … 6 = Saturday; empty = every day.
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedProxy {
    /// `system`, `none`, `manual` or `pac`.
    pub mode: String,
    pub url: String,
    pub pac_url: String,
    pub no_proxy: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedAi {
    /// `false` turns every AI feature off.
    pub enabled: Option<bool>,
    /// `false` keeps AI on local models only.
    pub allow_cloud: Option<bool>,
}

fn minutes(hhmm: &str) -> Option<u16> {
    let (h, m) = hhmm.split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && hhmm.len() == 5).then_some(h * 60 + m)
}

impl BlackoutWindow {
    pub fn validate(&self) -> Result<(), String> {
        let (Some(s), Some(e)) = (minutes(&self.start), minutes(&self.end)) else {
            return Err("Blackout times must be HH:MM".into());
        };
        if s == e {
            return Err("The blackout window must not start and end at the same time".into());
        }
        if self.days.iter().any(|d| *d > 6) {
            return Err("Blackout days are 0 (Sunday) to 6 (Saturday)".into());
        }
        Ok(())
    }

    /// Whether `weekday` (0 = Sunday) at `minute` of the day is inside.
    pub fn contains(&self, weekday: u8, minute: u16) -> bool {
        let (Some(s), Some(e)) = (minutes(&self.start), minutes(&self.end)) else { return false };
        let day_ok = |d: u8| self.days.is_empty() || self.days.contains(&d);
        if s < e {
            day_ok(weekday) && (s..e).contains(&minute)
        } else {
            // Overnight: the evening part belongs to today, the morning part to yesterday.
            (day_ok(weekday) && minute >= s) || (day_ok((weekday + 6) % 7) && minute < e)
        }
    }

    /// Whether the local time now is inside.
    pub fn contains_now(&self) -> bool {
        use chrono::{Datelike, Timelike};
        let t = chrono::Local::now();
        self.contains(t.weekday().num_days_from_sunday() as u8, (t.hour() * 60 + t.minute()) as u16)
    }
}

impl ManagedSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(u) = &self.updates {
            if let Some(c) = &u.channel {
                if !CHANNELS.contains(&c.as_str()) {
                    return Err(format!("Unknown update channel '{c}'"));
                }
            }
            if let Some(b) = &u.blackout {
                b.validate()?;
            }
        }
        if let Some(t) = &self.transfers {
            if t.max_concurrent_runs.is_some_and(|n| !(1..=MAX_CONCURRENT_RUNS).contains(&n)) {
                return Err(format!("Concurrent operations must be 1 to {MAX_CONCURRENT_RUNS}"));
            }
        }
        if let Some(m) = &self.mounts {
            if let Some(c) = &m.cache_mode {
                if !MOUNT_CACHE_MODES.contains(&c.as_str()) {
                    return Err(format!("Unknown mount cache mode '{c}'"));
                }
            }
            if m.max_cache_mb == Some(0) || m.max_cache_age_hours == Some(0) {
                return Err("Mount cache limits must be at least 1".into());
            }
            if m.mount_root.as_deref().is_some_and(|r| r.trim().is_empty()) {
                return Err("The mount root must be a folder".into());
            }
        }
        if let Some(w) = &self.workflows {
            if w.len() > MAX_WORKFLOWS {
                return Err(format!("At most {MAX_WORKFLOWS} organization workflows"));
            }
            let mut ids = std::collections::HashSet::new();
            for wf in w {
                wf.validate()?;
                if !ids.insert(wf.id.as_str()) {
                    return Err(format!("Two workflows share the id '{}'", wf.id));
                }
            }
        }
        if let Some(t) = &self.backup_templates {
            if t.len() > MAX_BACKUP_TEMPLATES {
                return Err(format!("At most {MAX_BACKUP_TEMPLATES} organization backup templates"));
            }
            let mut ids = std::collections::HashSet::new();
            for bt in t {
                bt.validate()?;
                if !ids.insert(bt.id.as_str()) {
                    return Err(format!("Two backup templates share the id '{}'", bt.id));
                }
            }
        }
        if let Some(a) = &self.server_access {
            a.validate()?;
        }
        if let Some(p) = &self.proxy {
            if !PROXY_MODES.contains(&p.mode.as_str()) {
                return Err(format!("Unknown proxy mode '{}'", p.mode));
            }
            if p.mode == "manual" && p.url.trim().is_empty() {
                return Err("A manual proxy needs a URL".into());
            }
            if p.mode == "pac" && p.pac_url.trim().is_empty() {
                return Err("A PAC proxy needs the PAC file URL".into());
            }
        }
        Ok(())
    }

    /// No section managed.
    pub fn is_empty(&self) -> bool {
        self.updates.is_none() && self.proxy.is_none() && self.ai.is_none() && self.transfers.is_none() && self.mounts.is_none() && self.workflows.is_none() && self.backup_templates.is_none() && self.server_access.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_access_validates() {
        let s: ManagedSettings = serde_json::from_value(serde_json::json!({"serverAccess": {
            "users": [{"username": "ops", "passwordHash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaA", "role": "operator"}],
            "rules": [{"principal": "ops", "jobPattern": "prod-*", "permission": "run"}]
        }})).unwrap();
        assert!(s.validate().is_ok() && !s.is_empty());
        let a = s.server_access.clone().unwrap();
        let bad = |f: &dyn Fn(&mut ManagedServerAccess)| {
            let mut x = a.clone();
            f(&mut x);
            ManagedSettings { server_access: Some(x), ..Default::default() }.validate().unwrap_err()
        };
        assert!(bad(&|x| x.users[0].password_hash = "plain".into()).contains("Argon2"));
        assert!(bad(&|x| x.users[0].role = "root".into()).contains("unknown role"));
        assert!(bad(&|x| x.users[0].username = "has space".into()).contains("letters"));
        assert!(bad(&|x| x.users.push(x.users[0].clone())).contains("Two server accounts"));
        assert!(bad(&|x| x.rules[0].permission = "all".into()).contains("Unknown access permission"));
    }

    #[test]
    fn workflows_validate() {
        let rule = serde_json::json!({"name": "x", "trigger": {"type": "manual"}, "action": {"type": "notify", "title": "a", "body": "b"}});
        let wf = |id: &str, rule: serde_json::Value| ManagedWorkflow { id: id.into(), name: "Intake".into(), enabled: true, rule };
        let s: ManagedSettings = serde_json::from_value(serde_json::json!({"workflows": [{"id": "intake", "name": "Intake", "enabled": true, "rule": rule}]})).unwrap();
        assert!(s.validate().is_ok() && !s.is_empty());
        let bad = |w: ManagedWorkflow| ManagedSettings { workflows: Some(vec![w]), ..Default::default() }.validate().unwrap_err();
        assert!(bad(wf("has space", rule.clone())).contains("letters"));
        assert!(bad(wf("x", serde_json::json!({"trigger": {}}))).contains("not an automation rule"));
        let cmd = serde_json::json!({"trigger": {"type": "manual"}, "action": {"type": "notify"}, "workflow": {"nodes": [{"id": "n1", "kind": "action", "action": {"type": "run_command", "program": "sh"}}]}});
        assert!(bad(wf("x", cmd)).contains("runs a program (run_command)"));
        let dup = ManagedSettings { workflows: Some(vec![wf("a", rule.clone()), wf("a", rule)]), ..Default::default() };
        assert!(dup.validate().unwrap_err().contains("share the id"));
    }

    #[test]
    fn backup_templates_validate() {
        let s: ManagedSettings = serde_json::from_value(serde_json::json!({"backupTemplates": [{"id": "std", "name": "Standard", "enforced": true, "settings": {"retention": {"type": "keep_count", "count": 5}}}]})).unwrap();
        assert!(s.validate().is_ok() && !s.is_empty());
        assert!(s.backup_templates.as_ref().unwrap()[0].enforced);
        let t = |id: &str, settings: serde_json::Value| ManagedBackupTemplate { id: id.into(), name: "X".into(), settings, ..Default::default() };
        let bad = |b: ManagedBackupTemplate| ManagedSettings { backup_templates: Some(vec![b]), ..Default::default() }.validate().unwrap_err();
        assert!(bad(t("a b", serde_json::json!({}))).contains("letters"));
        assert!(bad(t("a", serde_json::json!([]))).contains("object"));
        assert!(bad(t("a", serde_json::json!({"scripts": {"run_before": "rm"}}))).contains("scripts"));
        let dup = ManagedSettings { backup_templates: Some(vec![t("a", serde_json::json!({})), t("a", serde_json::json!({}))]), ..Default::default() };
        assert!(dup.validate().unwrap_err().contains("share the id"));
    }

    #[test]
    fn mount_profile_validates_and_parses_camel_case() {
        let s: ManagedSettings = serde_json::from_str(r#"{"version":3,"mounts":{"allowedProtocols":["rclone"],"readOnly":true,"cacheMode":"full","maxCacheMb":2048,"maxCacheAgeHours":72,"mountRoot":"~/Drives"}}"#).unwrap();
        let m = s.mounts.clone().unwrap();
        assert_eq!((m.read_only, m.cache_mode.as_deref(), m.max_cache_mb, m.mount_root.as_deref()), (Some(true), Some("full"), Some(2048), Some("~/Drives")));
        assert!(s.validate().is_ok() && !s.is_empty());
        let bad = ManagedSettings { mounts: Some(ManagedMounts { cache_mode: Some("everything".into()), ..Default::default() }), ..Default::default() };
        assert!(bad.validate().unwrap_err().contains("cache mode"));
        let zero = ManagedSettings { mounts: Some(ManagedMounts { max_cache_mb: Some(0), ..Default::default() }), ..Default::default() };
        assert!(zero.validate().is_err());
    }

    fn w(start: &str, end: &str, days: &[u8]) -> BlackoutWindow {
        BlackoutWindow { start: start.into(), end: end.into(), days: days.to_vec() }
    }

    #[test]
    fn daytime_window_on_weekdays() {
        let b = w("08:00", "18:00", &[1, 2, 3, 4, 5]);
        assert!(b.validate().is_ok());
        assert!(b.contains(1, 8 * 60));
        assert!(b.contains(5, 17 * 60 + 59));
        assert!(!b.contains(5, 18 * 60), "end is exclusive");
        assert!(!b.contains(0, 12 * 60), "Sunday is outside");
        assert!(!b.contains(3, 7 * 60 + 59));
    }

    #[test]
    fn overnight_window_belongs_to_its_start_day() {
        let b = w("22:00", "06:00", &[5]); // Friday night
        assert!(b.contains(5, 23 * 60));
        assert!(b.contains(6, 5 * 60), "Saturday early morning continues Friday's window");
        assert!(!b.contains(6, 23 * 60), "Saturday night is not in the window");
        assert!(!b.contains(5, 5 * 60), "Friday early morning belongs to Thursday");
        assert!(w("22:00", "06:00", &[]).contains(0, 60));
    }

    #[test]
    fn validation() {
        assert!(w("8:00", "18:00", &[]).validate().is_err());
        assert!(w("24:00", "01:00", &[]).validate().is_err());
        assert!(w("10:00", "10:00", &[]).validate().is_err());
        assert!(w("10:00", "11:00", &[7]).validate().is_err());
        let mut s = ManagedSettings { version: 1, updates: Some(ManagedUpdates { channel: Some("nightly".into()), ..Default::default() }), ..Default::default() };
        assert!(s.validate().is_err());
        s.updates = Some(ManagedUpdates { channel: Some("beta".into()), auto_check: Some(true), blackout: Some(w("09:00", "17:00", &[1])) });
        s.proxy = Some(ManagedProxy { mode: "manual".into(), ..Default::default() });
        assert!(s.validate().unwrap_err().contains("URL"));
        s.proxy = Some(ManagedProxy { mode: "pac".into(), pac_url: "https://wpad/proxy.pac".into(), ..Default::default() });
        assert!(s.validate().is_ok());
        assert!(!s.is_empty());
        assert!(ManagedSettings::default().is_empty());
        s.transfers = Some(ManagedTransfers { max_concurrent_runs: Some(9), ..Default::default() });
        assert!(s.validate().unwrap_err().contains("Concurrent"));
        s.transfers = Some(ManagedTransfers { max_concurrent_runs: Some(2), max_upload_kbps: Some(0), max_download_kbps: None });
        assert!(s.validate().is_ok());
    }

    #[test]
    fn wire_format_is_camel_case() {
        let s: ManagedSettings = serde_json::from_value(serde_json::json!({
            "version": 3, "updates": { "autoCheck": false, "blackout": { "start": "08:00", "end": "18:00", "days": [1] } },
            "proxy": { "mode": "pac", "pacUrl": "https://wpad/p.pac", "noProxy": ["intranet"] }, "ai": { "allowCloud": false }
        }))
        .unwrap();
        assert_eq!(s.updates.as_ref().unwrap().auto_check, Some(false));
        assert_eq!(s.proxy.as_ref().unwrap().no_proxy, vec!["intranet"]);
        assert_eq!(s.ai.as_ref().unwrap().allow_cloud, Some(false));
        assert_eq!(s.ai.as_ref().unwrap().enabled, None);
    }
}
