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

/// What failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// A file could not be opened, read or written (or its metadata read).
    File,
    /// A folder could not be listed or created.
    Dir,
    /// An entry of an unsupported kind.
    UnknownEntry,
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
    state: Mutex<State>,
    stop: AtomicBool,
}

impl ErrorTracker {
    pub fn new(policy: ErrorPolicy) -> Self {
        Self { policy, state: Mutex::default(), stop: AtomicBool::new(false) }
    }

    pub fn policy(&self) -> &ErrorPolicy {
        &self.policy
    }

    /// The outcome the policy gives a problem of this class.
    pub fn classify(&self, class: ErrorClass) -> Outcome {
        let fail = |a: ErrorAction| if a == ErrorAction::Fail { Outcome::Error } else { Outcome::Warning };
        match class {
            ErrorClass::File => fail(self.policy.on_file_error),
            ErrorClass::Dir => fail(self.policy.on_dir_error),
            ErrorClass::UnknownEntry => match self.policy.on_unknown_entry {
                UnknownEntryAction::Ignore => Outcome::Ignored,
                UnknownEntryAction::Warn => Outcome::Warning,
                UnknownEntryAction::Fail => Outcome::Error,
            },
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
            self.policy.max_errors.filter(|m| s.counted > *m).map(|m| format!("Stopped after more than {m} read error(s)"))
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
    fn policy_serde_defaults() {
        let p: ErrorPolicy = serde_json::from_str(r#"{"on_file_error":"fail","fail_fast":true}"#).unwrap();
        assert_eq!(p.on_file_error, ErrorAction::Fail);
        assert_eq!(p.on_unknown_entry, UnknownEntryAction::Ignore);
        assert_eq!(p.max_errors, None);
    }
}
