//! The files a running sync is working on and has finished, for the
//! expandable per-file list in the transfer panel. Bounded: the newest
//! [`KEEP_FINISHED`] finished files are kept (failures and conflicts are kept
//! longer than copies), so a run over millions of files costs a fixed amount
//! of memory; the totals still count everything.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use ufop_core::sync::executor::FileEvent;
use ufop_core::sync_types::{SyncItemAction, SyncRunItem};

/// Finished files remembered per run.
pub const KEEP_FINISHED: usize = 2_000;
/// Failures and conflicts remembered per run, beyond the newest finished ones.
pub const KEEP_PROBLEMS: usize = 500;
/// Most rows one call returns.
pub const MAX_ROWS: usize = 1_000;

/// One file in the live list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveFile {
    pub path: String,
    /// `active`, `done`, `failed` or `conflict`.
    pub state: String,
    /// What the run did to a finished file (`copied`, `updated`, `deleted`…).
    pub action: Option<String>,
    pub bytes: u64,
    pub error: Option<String>,
}

/// The bounded answer for one running sync.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LiveFiles {
    /// Files being worked on now, then the newest finished ones, problems first.
    pub files: Vec<LiveFile>,
    pub active_total: u64,
    /// Files finished so far, including those no longer listed.
    pub finished_total: u64,
    pub failed_total: u64,
    /// More finished files exist than are listed.
    pub truncated: bool,
}

/// What one run keeps while it is going.
#[derive(Debug, Default)]
pub struct LiveRun {
    active: BTreeSet<String>,
    finished: VecDeque<SyncRunItem>,
    problems: VecDeque<SyncRunItem>,
    finished_total: u64,
    failed_total: u64,
}

impl LiveRun {
    pub fn apply(&mut self, event: &FileEvent) {
        match event {
            FileEvent::Started(path) => {
                self.active.insert(path.clone());
            }
            FileEvent::Finished { path, item } => {
                self.active.remove(path);
                let Some(item) = item else { return };
                self.finished_total += 1;
                let problem = matches!(item.action, SyncItemAction::Failed | SyncItemAction::Conflict);
                if item.action == SyncItemAction::Failed {
                    self.failed_total += 1;
                }
                if problem {
                    self.problems.push_back(item.clone());
                    if self.problems.len() > KEEP_PROBLEMS {
                        self.problems.pop_front();
                    }
                }
                self.finished.push_back(item.clone());
                if self.finished.len() > KEEP_FINISHED {
                    self.finished.pop_front();
                }
            }
        }
    }

    /// At most `limit` rows: active files, then problems, then the newest
    /// finished files.
    pub fn snapshot(&self, limit: usize) -> LiveFiles {
        let limit = limit.clamp(1, MAX_ROWS);
        let mut files: Vec<LiveFile> = self
            .active
            .iter()
            .map(|p| LiveFile { path: p.clone(), state: "active".into(), action: None, bytes: 0, error: None })
            .take(limit)
            .collect();
        let row = |i: &SyncRunItem| LiveFile {
            path: i.rel_path.clone(),
            state: match i.action {
                SyncItemAction::Failed => "failed",
                SyncItemAction::Conflict => "conflict",
                _ => "done",
            }
            .into(),
            action: Some(i.action.as_str().into()),
            bytes: i.bytes,
            error: i.error.clone(),
        };
        let mut listed = std::collections::HashSet::new();
        for i in self.problems.iter().rev() {
            if files.len() >= limit {
                break;
            }
            if listed.insert(i.rel_path.clone()) {
                files.push(row(i));
            }
        }
        for i in self.finished.iter().rev() {
            if files.len() >= limit {
                break;
            }
            if !matches!(i.action, SyncItemAction::Failed | SyncItemAction::Conflict) {
                files.push(row(i));
            }
        }
        LiveFiles {
            truncated: self.finished_total as usize > files.iter().filter(|f| f.state != "active").count(),
            files,
            active_total: self.active.len() as u64,
            finished_total: self.finished_total,
            failed_total: self.failed_total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(path: &str, a: SyncItemAction) -> FileEvent {
        FileEvent::Finished { path: path.into(), item: Some(SyncRunItem::done(path, a, 10)) }
    }

    #[test]
    fn active_files_move_to_finished() {
        let mut run = LiveRun::default();
        run.apply(&FileEvent::Started("a".into()));
        run.apply(&FileEvent::Started("b".into()));
        assert_eq!(run.snapshot(10).active_total, 2);
        run.apply(&done("a", SyncItemAction::Copied));
        let snap = run.snapshot(10);
        assert_eq!((snap.active_total, snap.finished_total), (1, 1));
        assert_eq!(snap.files.iter().map(|f| (f.path.as_str(), f.state.as_str())).collect::<Vec<_>>(), vec![("b", "active"), ("a", "done")]);
        assert!(!snap.truncated);
    }

    #[test]
    fn a_file_left_alone_just_stops_being_active() {
        let mut run = LiveRun::default();
        run.apply(&FileEvent::Started("a".into()));
        run.apply(&FileEvent::Finished { path: "a".into(), item: None });
        let snap = run.snapshot(10);
        assert!(snap.files.is_empty());
        assert_eq!((snap.active_total, snap.finished_total), (0, 0));
    }

    #[test]
    fn the_list_is_bounded_and_problems_come_first() {
        let mut run = LiveRun::default();
        for i in 0..(KEEP_FINISHED + 50) {
            run.apply(&done(&format!("f{i}"), SyncItemAction::Copied));
        }
        run.apply(&FileEvent::Finished { path: "bad".into(), item: Some(SyncRunItem::failed("bad", 3, "denied".into())) });
        let snap = run.snapshot(MAX_ROWS + 500);
        assert!(snap.files.len() <= MAX_ROWS, "{}", snap.files.len());
        assert_eq!(snap.files[0].path, "bad");
        assert_eq!((snap.files[0].state.as_str(), snap.files[0].error.as_deref()), ("failed", Some("denied")));
        assert_eq!((snap.finished_total, snap.failed_total), ((KEEP_FINISHED + 51) as u64, 1));
        assert!(snap.truncated);
        // Newest finished first after the problems.
        assert_eq!(snap.files[1].path, format!("f{}", KEEP_FINISHED + 49));
        assert_eq!(run.snapshot(5).files.len(), 5);
    }

    #[test]
    fn memory_is_bounded_whatever_the_run_size() {
        let mut run = LiveRun::default();
        for i in 0..(KEEP_FINISHED * 3) {
            run.apply(&done(&format!("f{i}"), SyncItemAction::Copied));
            run.apply(&FileEvent::Finished { path: format!("x{i}"), item: Some(SyncRunItem::failed(&format!("x{i}"), 0, "e".into())) });
        }
        assert!(run.finished.len() <= KEEP_FINISHED && run.problems.len() <= KEEP_PROBLEMS);
        assert_eq!(run.snapshot(10).failed_total as usize, KEEP_FINISHED * 3);
    }
}
