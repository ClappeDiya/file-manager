//! Reusable notification profiles: a named channel (with its result
//! filter, templates and certificate policy) kept once and referenced by
//! jobs and policy templates through `notification_profile_ids`.
//!
//! Profiles live in the config table like policy templates; their secrets
//! (password, token, webhook URL, secret header values) stay in the
//! credential store under the channel's credential ids. The organization's
//! console can deliver read-only profiles (`org:<id>`), whose secrets name
//! console credentials (`central:<id>`).
//!
//! A job sends to its own `notifications` plus every referenced profile;
//! the same channel listed twice is sent once (result filters merged).

use super::Notification;
use crate::backup_engine::templates::MANAGED_PREFIX;
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};

/// Config-table key of the local profiles.
pub const PROFILES_KEY: &str = "backup.notification_profiles";

/// Lowest result a profile sends (overrides the channel's `send_on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Success,
    Warning,
    Error,
}

impl Severity {
    pub fn send_on(self) -> Vec<String> {
        let all = ["success", "warning", "error"];
        all[self as usize..].iter().map(|s| s.to_string()).collect()
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The channel, its filter, message templates and certificate policy.
    pub notification: Notification,
    /// When set, results at least this severe are sent (instead of `send_on`).
    #[serde(default)]
    pub min_severity: Option<Severity>,
    /// Delivered by the organization's console (read-only here).
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// Write-only: the channel secret to store with this save (never kept in the profile).
    #[serde(default, skip_serializing)]
    pub secret: Option<String>,
}

impl NotificationProfile {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() || self.name.chars().count() > 200 {
            return Err(AppError::validation("Give the notification profile a name (up to 200 characters)"));
        }
        self.notification.validate()
    }

    /// The channel as sent: `min_severity` applied.
    pub fn effective(&self) -> Notification {
        let mut n = self.notification.clone();
        if let Some(s) = self.min_severity {
            n.send_on = s.send_on();
        }
        n
    }

    /// A console profile (`ufop_core::agent::ManagedNotificationProfile`).
    pub fn from_managed(m: &ufop_core::agent::ManagedNotificationProfile) -> Result<Self, AppError> {
        m.validate().map_err(AppError::validation)?;
        let mut v = m.notification.clone();
        if let Some(o) = v.as_object_mut() {
            // Normalize console credential ids (`central:x`).
            if let Some(c) = o.get("credential_id").and_then(|c| c.as_str()).and_then(ufop_core::agent::policy::central_credential_id) {
                o.insert("credential_id".into(), c.into());
            }
        }
        let notification: Notification = serde_json::from_value(v).map_err(|e| AppError::validation(format!("Organization notification profile '{}': {e}", m.name)))?;
        let p = Self {
            id: format!("{MANAGED_PREFIX}{}", m.id),
            name: m.name.clone(),
            description: m.description.clone(),
            notification,
            min_severity: m.min_severity.as_deref().and_then(Severity::parse),
            managed: true,
            created_at: String::new(),
            updated_at: String::new(),
            secret: None,
        };
        p.validate().map_err(|e| AppError::validation(format!("Organization notification profile '{}': {}", m.name, e.user_message())))?;
        Ok(p)
    }
}

/// Identity of a channel's destination (for de-duplication): the channel
/// settings, its credential and certificate policy, ignoring the filter.
fn identity(n: &Notification) -> String {
    let mut k = n.clone();
    k.send_on.clear();
    serde_json::to_string(&k).unwrap_or_default()
}

/// Inline channels plus referenced profiles, de-duplicated (same channel
/// twice: sent once, result filters merged). Missing profiles are skipped
/// with a warning.
pub fn resolve(inline: &[Notification], ids: &[String], profiles: &[NotificationProfile]) -> (Vec<Notification>, Vec<String>) {
    let mut out: Vec<Notification> = Vec::new();
    let mut warnings = Vec::new();
    let mut push = |n: Notification| match out.iter_mut().find(|x| identity(x) == identity(&n)) {
        Some(x) => {
            for s in n.send_on {
                if !x.send_on.contains(&s) {
                    x.send_on.push(s);
                }
            }
        }
        None => out.push(n),
    };
    for n in inline {
        push(n.clone());
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        match profiles.iter().find(|p| &p.id == id) {
            Some(p) => push(p.effective()),
            None => warnings.push(format!("Notification profile {id} no longer exists; it is skipped")),
        }
    }
    (out, warnings)
}

/// Refuse unknown profile ids.
pub fn validate_refs(ids: &[String], profiles: &[NotificationProfile]) -> Result<(), AppError> {
    match ids.iter().find(|id| !profiles.iter().any(|p| &p.id == *id)) {
        Some(id) => Err(AppError::validation(format!("Notification profile {id} does not exist"))),
        None => Ok(()),
    }
}

/// What a forced delete changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDeleteOutcome {
    /// Jobs and policy templates the reference was removed from.
    pub removed_from: Vec<String>,
    pub warnings: Vec<String>,
}

/// A sample run for tests and previews.
pub fn sample_vars(result: &str) -> std::collections::BTreeMap<String, String> {
    [
        ("name", "Documents"),
        ("operation", "backup"),
        ("result", result),
        ("machine", "this computer"),
        ("backup_id", "sample"),
        ("job_id", "sample"),
        ("duration", "42s"),
        ("files", "1284"),
        ("bytes", "73400320"),
        ("errors", if result == "error" { "Could not reach the destination" } else { "" }),
        ("previous_result", "success"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify_hub::Channel;

    fn tg(chat: &str, send_on: &[&str]) -> Notification {
        Notification {
            channel: Channel::Telegram { chat_id: chat.into(), api_base: String::new() },
            send_on: send_on.iter().map(|s| s.to_string()).collect(),
            credential_id: format!("c-{chat}"),
            template: String::new(),
            cert: Default::default(),
        }
    }

    fn profile(id: &str, n: Notification) -> NotificationProfile {
        NotificationProfile { id: id.into(), name: id.into(), description: String::new(), notification: n, min_severity: None, managed: false, created_at: String::new(), updated_at: String::new(), secret: None }
    }

    #[test]
    fn severity_maps_to_results() {
        assert_eq!(Severity::Success.send_on(), vec!["success", "warning", "error"]);
        assert_eq!(Severity::Warning.send_on(), vec!["warning", "error"]);
        assert_eq!(Severity::Error.send_on(), vec!["error"]);
        let p = NotificationProfile { min_severity: Some(Severity::Error), ..profile("a", tg("1", &["success"])) };
        assert_eq!(p.effective().send_on, vec!["error"]);
    }

    #[test]
    fn resolution_merges_and_deduplicates() {
        let profiles = vec![profile("p1", tg("1", &["error"])), profile("p2", tg("2", &["warning"]))];
        // Inline channel equal to p1's destination: sent once, filters merged.
        let (list, warns) = resolve(&[tg("1", &["success"])], &["p1".into(), "p2".into(), "p2".into(), "gone".into()], &profiles);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].send_on, vec!["success", "error"]);
        assert!(matches!(&list[1].channel, Channel::Telegram { chat_id, .. } if chat_id == "2"));
        assert_eq!(warns.len(), 1);
        assert!(validate_refs(&["p1".into()], &profiles).is_ok());
        assert!(validate_refs(&["zz".into()], &profiles).is_err());
    }

    #[test]
    fn managed_profiles_need_console_credentials() {
        let m = ufop_core::agent::ManagedNotificationProfile {
            id: "ops".into(),
            name: "Ops".into(),
            notification: serde_json::json!({ "type": "telegram", "chat_id": "-100", "credential_id": "central:tg" }),
            min_severity: Some("warning".into()),
            ..Default::default()
        };
        let p = NotificationProfile::from_managed(&m).unwrap();
        assert_eq!(p.id, "org:ops");
        assert!(p.managed);
        assert_eq!(p.notification.credential_id, "central:tg");
        assert_eq!(p.effective().send_on, vec!["warning", "error"]);
        let bad = ufop_core::agent::ManagedNotificationProfile { notification: serde_json::json!({ "type": "telegram", "chat_id": "" }), ..m.clone() };
        assert!(NotificationProfile::from_managed(&bad).is_err(), "channel validation applies");
        let plain = ufop_core::agent::ManagedNotificationProfile { notification: serde_json::json!({ "type": "telegram", "chat_id": "1", "credential_id": "mine" }), ..m };
        assert!(NotificationProfile::from_managed(&plain).is_err());
    }

    #[test]
    fn secret_is_write_only() {
        let p = NotificationProfile { secret: Some("hunter2".into()), ..profile("a", tg("1", &["error"])) };
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("hunter2"));
        let back: NotificationProfile = serde_json::from_str(&json.replace("\"managed\"", "\"secret\":\"x\",\"managed\"")).unwrap();
        assert_eq!(back.secret.as_deref(), Some("x"));
    }
}
