//! What to do when entries cannot be read: per-kind actions, fail-fast and
//! an error budget. [`ErrorPolicy`] is the setting; [`ErrorTracker`]
//! records each problem, files it as a warning or an error and says when
//! the operation should stop. Backup uses it for scanning and reading;
//! restore can use the same tracker for writing.
//!
//! - `Continue`: the problem is a warning and the entry is skipped (a backup
//!   keeps the previous version's copy of an unreadable file).
//! - `Fail`: the problem is an error; the run still finishes the rest (and
//!   ends with status "error") unless `fail_fast` is on, which stops it at
//!   the first error. A stopped backup writes a partial version of what was
//!   finished.
//! - `max_errors`: stop once more than this many problems (warnings and
//!   errors from unreadable entries) were recorded.
//!
//! Restore uses the same tracker with [`RestoreErrorPolicy`]: files that
//! cannot be written, folders that cannot be created, data that is missing
//! from the destination and metadata (permissions, owner, attributes) that
//! cannot be applied each get their own action.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Action for an unreadable file or folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ErrorAction {
    /// Warn, skip the entry and carry on.
    #[default]
    Continue,
    /// Record an error (and stop at once with `fail_fast`).
    Fail,
}

/// Action for entries that are neither files, folders nor links
/// (sockets, FIFOs, device nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnknownEntryAction {
    /// Skip them silently.
    #[default]
    Ignore,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ErrorPolicy {
    pub on_file_error: ErrorAction,
    pub on_dir_error: ErrorAction,
    pub on_unknown_entry: UnknownEntryAction,
    /// Stop at the first error instead of finishing the run.
    pub fail_fast: bool,
    /// Stop once more than this many problems were recorded.
    pub max_errors: Option<u32>,
}

/// What a restore does when something goes wrong. The defaults match how
/// restores always behaved: a file that cannot be written or whose data is
/// missing is an error (the restore still finishes the other files), and
/// metadata that cannot be applied is a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreErrorPolicy {
    /// A file or link cannot be written (`continue` = only a warning).
    pub on_write_error: ErrorAction,
    /// A folder cannot be created.
    pub on_dir_error: ErrorAction,
    /// A file's data is missing or damaged at the destination.
    pub on_missing_data: ErrorAction,
    /// Permissions, owner, times or attributes cannot be applied.
    pub on_metadata_error: UnknownEntryAction,
    pub fail_fast: bool,
    pub max_errors: Option<u32>,
}

impl Default for RestoreErrorPolicy {
    fn default() -> Self {
        Self {
            on_write_error: ErrorAction::Fail,
            on_dir_error: ErrorAction::Fail,
            on_missing_data: ErrorAction::Fail,
            on_metadata_error: UnknownEntryAction::Warn,
            fail_fast: false,
            max_errors: None,
        }
    }
}

/// What failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// A file could not be opened, read or written (or its metadata read).
    File,
    /// A folder could not be listed or created.
    Dir,
    /// An entry of an unsupported kind.
    UnknownEntry,
    /// Restore: metadata could not be applied.
    Metadata,
    /// Restore: data is missing or damaged at the destination.
    MissingData,
}

/// How a recorded problem was filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Warning,
    Error,
}

#[derive(Debug, Default)]
struct State {
    warnings: Vec<String>,
    errors: Vec<String>,
    counted: u32,
    stop_reason: Option<String>,
}

/// Records problems under an [`ErrorPolicy`]; shared by worker threads.
#[derive(Debug, Default)]
pub struct ErrorTracker {
    policy: ErrorPolicy,
    /// Restore-only classes (metadata, missing data); `None` for backups.
    restore: Option<RestoreErrorPolicy>,
    state: Mutex<State>,
    stop: AtomicBool,
}

impl ErrorTracker {
    pub fn new(policy: ErrorPolicy) -> Self {
        Self { policy, restore: None, state: Mutex::default(), stop: AtomicBool::new(false) }
    }

    /// Tracker for a restore: `File` = writing a file, `Dir` = creating a
    /// folder, plus `Metadata` and `MissingData`.
    pub fn for_restore(p: RestoreErrorPolicy) -> Self {
        let policy = ErrorPolicy { on_file_error: p.on_write_error, on_dir_error: p.on_dir_error, on_unknown_entry: UnknownEntryAction::Warn, fail_fast: p.fail_fast, max_errors: p.max_errors };
        Self { policy, restore: Some(p), state: Mutex::default(), stop: AtomicBool::new(false) }
    }

    pub fn policy(&self) -> &ErrorPolicy {
        &self.policy
    }

    /// The outcome the policy gives a problem of this class.
    pub fn classify(&self, class: ErrorClass) -> Outcome {
        let fail = |a: ErrorAction| if a == ErrorAction::Fail { Outcome::Error } else { Outcome::Warning };
        let level = |a: UnknownEntryAction| match a {
            UnknownEntryAction::Ignore => Outcome::Ignored,
            UnknownEntryAction::Warn => Outcome::Warning,
            UnknownEntryAction::Fail => Outcome::Error,
        };
        let r = self.restore.unwrap_or_default();
        match class {
            ErrorClass::File => fail(self.policy.on_file_error),
            ErrorClass::Dir => fail(self.policy.on_dir_error),
            ErrorClass::UnknownEntry => level(self.policy.on_unknown_entry),
            ErrorClass::Metadata => level(r.on_metadata_error),
            ErrorClass::MissingData => fail(r.on_missing_data),
        }
    }

    /// Record a problem; returns how it was filed. Sets the stop flag when
    /// fail-fast or the error budget says so.
    pub fn record(&self, class: ErrorClass, message: impl Into<String>) -> Outcome {
        let outcome = self.classify(class);
        if outcome == Outcome::Ignored {
            return outcome;
        }
        let message = message.into();
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.counted += 1;
        let reason = if outcome == Outcome::Error && self.policy.fail_fast {
            Some(format!("Stopped at the first error (fail-fast): {message}"))
        } else {
            let what = if self.restore.is_some() { "problem" } else { "read error" };
            self.policy.max_errors.filter(|m| s.counted > *m).map(|m| format!("Stopped after more than {m} {what}(s)"))
        };
        match outcome {
            Outcome::Error => s.errors.push(message),
            _ => s.warnings.push(message),
        }
        if let Some(r) = reason {
            if s.stop_reason.is_none() {
                s.stop_reason = Some(r);
            }
            self.stop.store(true, Ordering::SeqCst);
        }
        outcome
    }

    /// Whether the operation should stop now.
    pub fn should_stop(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    pub fn stop_reason(&self) -> Option<String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).stop_reason.clone()
    }

    /// Problems recorded so far (warnings + errors).
    pub fn count(&self) -> u32 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).counted
    }

    /// Drain the recorded (warnings, errors).
    pub fn take(&self) -> (Vec<String>, Vec<String>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (std::mem::take(&mut s.warnings), std::mem::take(&mut s.errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_warns_and_never_stops() {
        let t = ErrorTracker::new(ErrorPolicy::default());
        assert_eq!(t.record(ErrorClass::File, "a"), Outcome::Warning);
        assert_eq!(t.record(ErrorClass::Dir, "b"), Outcome::Warning);
        assert_eq!(t.record(ErrorClass::UnknownEntry, "fifo"), Outcome::Ignored);
        assert!(!t.should_stop());
        assert_eq!(t.take(), (vec!["a".to_string(), "b".to_string()], vec![]));
        assert_eq!(t.count(), 2);
    }

    #[test]
    fn per_kind_actions_fail_fast_and_budget() {
        let p = ErrorPolicy { on_dir_error: ErrorAction::Fail, on_unknown_entry: UnknownEntryAction::Warn, ..Default::default() };
        let t = ErrorTracker::new(p);
        assert_eq!(t.record(ErrorClass::Dir, "d"), Outcome::Error);
        assert!(!t.should_stop(), "without fail-fast the run goes on");
        assert_eq!(t.record(ErrorClass::UnknownEntry, "s"), Outcome::Warning);

        let t = ErrorTracker::new(ErrorPolicy { fail_fast: true, ..p });
        t.record(ErrorClass::File, "only a warning");
        assert!(!t.should_stop());
        t.record(ErrorClass::Dir, "cannot list /x");
        assert!(t.should_stop());
        assert!(t.stop_reason().unwrap().contains("cannot list /x"));

        let t = ErrorTracker::new(ErrorPolicy { max_errors: Some(2), ..Default::default() });
        t.record(ErrorClass::File, "1");
        t.record(ErrorClass::File, "2");
        assert!(!t.should_stop());
        t.record(ErrorClass::File, "3");
        assert!(t.should_stop());
        assert!(t.stop_reason().unwrap().contains("more than 2"));
    }

    #[test]
    fn restore_policy_classes() {
        let t = ErrorTracker::for_restore(RestoreErrorPolicy::default());
        assert_eq!(t.record(ErrorClass::File, "write"), Outcome::Error, "writing failures stay errors by default");
        assert_eq!(t.record(ErrorClass::MissingData, "gone"), Outcome::Error);
        assert_eq!(t.record(ErrorClass::Metadata, "chmod"), Outcome::Warning);
        assert!(!t.should_stop());
        let p = RestoreErrorPolicy { on_write_error: ErrorAction::Continue, on_metadata_error: UnknownEntryAction::Ignore, on_missing_data: ErrorAction::Fail, fail_fast: true, ..Default::default() };
        let t = ErrorTracker::for_restore(p);
        assert_eq!(t.record(ErrorClass::File, "w"), Outcome::Warning);
        assert_eq!(t.record(ErrorClass::Metadata, "m"), Outcome::Ignored);
        assert!(!t.should_stop());
        t.record(ErrorClass::MissingData, "block x");
        assert!(t.should_stop() && t.stop_reason().unwrap().contains("block x"));
        let t = ErrorTracker::for_restore(RestoreErrorPolicy { max_errors: Some(1), ..Default::default() });
        t.record(ErrorClass::Metadata, "1");
        t.record(ErrorClass::Metadata, "2");
        assert!(t.stop_reason().unwrap().contains("more than 1 problem"));
        let d: RestoreErrorPolicy = serde_json::from_str(r#"{"fail_fast":true}"#).unwrap();
        assert_eq!(d.on_write_error, ErrorAction::Fail);
        assert!(d.fail_fast);
    }

    #[test]
    fn policy_serde_defaults() {
        let p: ErrorPolicy = serde_json::from_str(r#"{"on_file_error":"fail","fail_fast":true}"#).unwrap();
        assert_eq!(p.on_file_error, ErrorAction::Fail);
        assert_eq!(p.on_unknown_entry, UnknownEntryAction::Ignore);
        assert_eq!(p.max_errors, None);
    }
}
