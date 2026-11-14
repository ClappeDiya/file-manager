//! Result notifications of sync pairs and automation rules.
//!
//! A pair or rule names a notification profile (a channel set up once in the
//! backup settings) and says which results send. The decision is a pure
//! function so every engine applies the same rules: errors always notify
//! when asked for, and a size threshold only mutes *successful* runs that
//! moved little.

use serde::{Deserialize, Serialize};

/// How a run ended, as notifications see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunResult {
    Success,
    /// Finished, but with skipped files, conflicts or verification differences.
    Warning,
    Error,
}

impl RunResult {
    /// The word channels and filters use (`send_on`, `{{result}}`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Results a profile filter can name.
pub const RESULTS: [&str; 3] = ["success", "warning", "error"];

/// Notification settings of one sync pair or automation rule. Every field
/// defaults to "no notification", so stored pairs and rules are unaffected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNotify {
    /// The notification profile to send to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_profile_id: Option<String>,
    /// Results that send (`success`, `warning`, `error`); empty = whatever
    /// the profile itself is set to send.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notify_on: Vec<String>,
    /// Do not report a successful run that moved fewer bytes than this
    /// (errors and warnings are always reported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_bytes: Option<u64>,
}

impl RunNotify {
    /// The profile id, when one is chosen.
    pub fn profile(&self) -> Option<&str> {
        self.notify_profile_id.as_deref().map(str::trim).filter(|p| !p.is_empty())
    }

    /// Whether a run that ended with `result` after moving `bytes` is reported.
    /// `bytes` is `None` when the engine cannot tell (the threshold is then
    /// not applied: better a notification too many than a missed one).
    pub fn wants(&self, result: RunResult, bytes: Option<u64>) -> bool {
        if self.profile().is_none() {
            return false;
        }
        if !self.notify_on.is_empty() && !self.notify_on.iter().any(|r| r == result.as_str()) {
            return false;
        }
        if result == RunResult::Success {
            if let (Some(min), Some(moved)) = (self.min_bytes, bytes) {
                return moved >= min;
            }
        }
        true
    }

    /// The settings the editor may save.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(bad) = self.notify_on.iter().find(|r| !RESULTS.contains(&r.as_str())) {
            return Err(format!("Notify on \"{bad}\" is not a result; use success, warning or error"));
        }
        if self.profile().is_none() && (!self.notify_on.is_empty() || self.min_bytes.is_some()) {
            return Err("Choose a notification profile to send to".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(profile: Option<&str>, on: &[&str], min: Option<u64>) -> RunNotify {
        RunNotify { notify_profile_id: profile.map(Into::into), notify_on: on.iter().map(|s| s.to_string()).collect(), min_bytes: min }
    }

    #[test]
    fn nothing_without_a_profile() {
        assert!(!RunNotify::default().wants(RunResult::Error, Some(0)));
        assert!(!n(Some("  "), &[], None).wants(RunResult::Error, None));
    }

    #[test]
    fn notify_on_filters_results_and_empty_leaves_it_to_the_profile() {
        let only_errors = n(Some("p"), &["error"], None);
        assert!(only_errors.wants(RunResult::Error, None));
        assert!(!only_errors.wants(RunResult::Success, Some(10)));
        assert!(!only_errors.wants(RunResult::Warning, None));
        let any = n(Some("p"), &[], None);
        assert!([RunResult::Success, RunResult::Warning, RunResult::Error].iter().all(|r| any.wants(*r, Some(1))));
    }

    #[test]
    fn size_threshold_mutes_small_successes_only() {
        let t = n(Some("p"), &["success", "error", "warning"], Some(1_000));
        assert!(!t.wants(RunResult::Success, Some(999)));
        assert!(t.wants(RunResult::Success, Some(1_000)));
        assert!(!t.wants(RunResult::Success, Some(0)), "a run that moved nothing is below any threshold");
        assert!(t.wants(RunResult::Error, Some(0)), "errors always notify");
        assert!(t.wants(RunResult::Warning, Some(0)), "warnings always notify");
        assert!(t.wants(RunResult::Success, None), "unknown size: do not mute");
    }

    #[test]
    fn validation_and_serde_defaults() {
        assert!(n(Some("p"), &["success", "error"], Some(5)).validate().is_ok());
        assert!(n(Some("p"), &["fatal"], None).validate().unwrap_err().contains("fatal"));
        assert!(n(None, &["error"], None).validate().is_err());
        assert!(n(None, &[], Some(5)).validate().is_err());
        assert!(RunNotify::default().validate().is_ok());
        // "Nobody" in the editor: the profile cleared together with what depended on it.
        assert!(n(None, &[], None).validate().is_ok());
        // Absent fields read as "no notification"; none are written when unset.
        let back: RunNotify = serde_json::from_str("{}").unwrap();
        assert_eq!(back, RunNotify::default());
        assert_eq!(serde_json::to_string(&RunNotify::default()).unwrap(), "{}");
        let json = serde_json::to_value(n(Some("p"), &["error"], Some(7))).unwrap();
        assert_eq!(json, serde_json::json!({ "notify_profile_id": "p", "notify_on": ["error"], "min_bytes": 7 }));
    }
}
