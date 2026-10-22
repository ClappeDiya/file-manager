//! App settings an organization manages from the console: update channel,
//! automatic checks and a blackout window, the proxy, and AI. A `None`
//! section (or field) leaves that setting to the user.

use serde::{Deserialize, Serialize};

pub const CHANNELS: [&str; 2] = ["stable", "beta"];
pub const PROXY_MODES: [&str; 4] = ["system", "none", "manual", "pac"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedSettings {
    /// Increases with every change in the console.
    pub version: u64,
    pub updates: Option<ManagedUpdates>,
    pub proxy: Option<ManagedProxy>,
    pub ai: Option<ManagedAi>,
}

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
        self.updates.is_none() && self.proxy.is_none() && self.ai.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
