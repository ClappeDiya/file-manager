//! Chat-bot remote control (Telegram, then Slack): start, stop and list
//! backup, sync and automation jobs from a chat.
//!
//! Off by default and switched on in Settings → Advanced. The bot only
//! answers the user ids on its allow-list, maps a fixed set of commands
//! (`/jobs`, `/run <job>`, `/stop <job>`, `/status`, `/help`) onto the
//! app's own managers (never a shell or a program), writes every accepted
//! command to the tamper-evident audit log before running it, and rate-limits
//! each sender. Bot tokens live in the credential store.
//!
//! Layout: [`core`] holds the pure rules (parser, allow-list, rate limit,
//! name resolution); [`handler`] ties them to a backend and an audit sink;
//! [`telegram`] (long polling) and [`slack`] (Socket Mode over [`ws`]) are the
//! transports; [`backend`] is the real backend; [`service`] starts and stops it all.

pub mod backend;
pub mod core;
pub mod handler;
pub mod service;
pub mod slack;
pub mod telegram;
pub mod ws;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// Config key of the settings.
pub const SETTINGS_KEY: &str = "chat_control.settings";
/// Credential-store keys of the secrets.
pub const TELEGRAM_TOKEN_KEY: &str = "chat_control.telegram.bot_token";
pub const SLACK_BOT_TOKEN_KEY: &str = "chat_control.slack.bot_token";
pub const SLACK_APP_TOKEN_KEY: &str = "chat_control.slack.app_token";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramSettings {
    pub enabled: bool,
    /// Telegram user ids (numbers) allowed to command the bot.
    pub allowed_user_ids: Vec<String>,
    /// Bot API base; empty = `https://api.telegram.org`.
    pub api_base: String,
}

impl Default for TelegramSettings {
    fn default() -> Self {
        Self { enabled: false, allowed_user_ids: Vec::new(), api_base: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackSettings {
    pub enabled: bool,
    /// Slack member ids (`U…`) allowed to command the bot.
    pub allowed_user_ids: Vec<String>,
    /// Web API base; empty = `https://slack.com/api`.
    pub api_base: String,
}

impl Default for SlackSettings {
    fn default() -> Self {
        Self { enabled: false, allowed_user_ids: Vec::new(), api_base: String::new() }
    }
}

/// Settings → Advanced → Remote control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatControlSettings {
    /// Master switch; nothing runs while it is off.
    pub enabled: bool,
    pub telegram: TelegramSettings,
    pub slack: SlackSettings,
    /// Commands one sender may send per minute.
    pub rate_limit_per_min: u32,
}

impl Default for ChatControlSettings {
    fn default() -> Self {
        Self { enabled: false, telegram: TelegramSettings::default(), slack: SlackSettings::default(), rate_limit_per_min: 10 }
    }
}

/// An endpoint override is only accepted over https, or http to this computer (tests, relays).
fn valid_base(base: &str) -> bool {
    base.trim().is_empty()
        || url::Url::parse(base.trim()).is_ok_and(|u| u.scheme() == "https" || (u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
}

impl ChatControlSettings {
    /// What may be saved: an enabled channel needs an allow-list of
    /// well-formed ids; endpoint overrides must be https (or loopback).
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=60).contains(&self.rate_limit_per_min) {
            return Err("Allow between 1 and 60 commands per minute".into());
        }
        if !valid_base(&self.telegram.api_base) || !valid_base(&self.slack.api_base) {
            return Err("The service address must start with https://".into());
        }
        if self.telegram.enabled {
            if self.telegram.allowed_user_ids.is_empty() {
                return Err("Add at least one Telegram user id that may command the bot".into());
            }
            if let Some(bad) = self.telegram.allowed_user_ids.iter().find(|i| !core::valid_telegram_id(i)) {
                return Err(format!("\"{bad}\" is not a Telegram user id (a number)"));
            }
        }
        if self.slack.enabled {
            if self.slack.allowed_user_ids.is_empty() {
                return Err("Add at least one Slack member id that may command the bot".into());
            }
            if let Some(bad) = self.slack.allowed_user_ids.iter().find(|i| !core::valid_slack_id(i)) {
                return Err(format!("\"{bad}\" is not a Slack member id (like U012ABCDEF)"));
            }
        }
        Ok(())
    }
}

/// Live state of one channel, shared with the UI.
#[derive(Debug, Default)]
pub struct ChannelStatus {
    pub connected: AtomicBool,
    /// Commands answered since the channel started.
    pub handled: AtomicU64,
    pub last_error: Mutex<Option<String>>,
}

impl ChannelStatus {
    pub fn set_error(&self, e: impl Into<String>) {
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.into());
        self.connected.store(false, Ordering::SeqCst);
    }

    pub fn clear_error(&self) {
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.connected.store(true, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> ChannelState {
        ChannelState { connected: self.connected.load(Ordering::SeqCst), handled: self.handled.load(Ordering::SeqCst), last_error: self.last_error.lock().unwrap_or_else(|p| p.into_inner()).clone() }
    }
}

/// What the UI shows for a channel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelState {
    pub connected: bool,
    pub handled: u64,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_and_nothing_runs_without_an_allow_list() {
        let s = ChatControlSettings::default();
        assert!(!s.enabled && !s.telegram.enabled && !s.slack.enabled);
        assert!(s.validate().is_ok());
        let mut t = s.clone();
        t.telegram.enabled = true;
        assert!(t.validate().unwrap_err().contains("at least one Telegram user id"));
        t.telegram.allowed_user_ids = vec!["12345".into()];
        assert!(t.validate().is_ok());
        t.telegram.allowed_user_ids.push("@someone".into());
        assert!(t.validate().unwrap_err().contains("@someone"));
        let mut sl = s.clone();
        sl.slack.enabled = true;
        assert!(sl.validate().unwrap_err().contains("Slack member id"));
        sl.slack.allowed_user_ids = vec!["U0ABC123".into()];
        assert!(sl.validate().is_ok());
        sl.slack.allowed_user_ids = vec!["general".into()];
        assert!(sl.validate().is_err());
    }

    #[test]
    fn limits_and_endpoints_are_checked() {
        let mut s = ChatControlSettings::default();
        for bad in [0, 61] {
            s.rate_limit_per_min = bad;
            assert!(s.validate().is_err());
        }
        s.rate_limit_per_min = 5;
        s.telegram.api_base = "http://example.com".into();
        assert!(s.validate().unwrap_err().contains("https"), "tokens never go over plain http to a remote host");
        s.telegram.api_base = "http://127.0.0.1:9999".into();
        assert!(s.validate().is_ok());
        s.telegram.api_base = "https://bots.example.org/".into();
        assert!(s.validate().is_ok());
        s.slack.api_base = "not a url".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn settings_read_with_missing_fields_and_status_tracks_errors() {
        let s: ChatControlSettings = serde_json::from_str(r#"{"enabled":true,"telegram":{"enabled":true}}"#).unwrap();
        assert!(s.enabled && s.telegram.enabled && s.telegram.allowed_user_ids.is_empty() && s.rate_limit_per_min == 10 && !s.slack.enabled);
        let st = ChannelStatus::default();
        st.clear_error();
        assert!(st.snapshot().connected);
        st.set_error("boom");
        let snap = st.snapshot();
        assert_eq!((snap.connected, snap.last_error.as_deref()), (false, Some("boom")));
    }
}
