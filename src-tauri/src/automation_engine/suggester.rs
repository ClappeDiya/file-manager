//! Quickflow Suggester — bridges the operation ledger and the automation engine.
//!
//! Watches recent fs activity in the unified [`OperationLedger`] and surfaces
//! single-click "Save as Quickflow" suggestions when the user has demonstrated
//! the same `source_dir → dest_dir` move/copy pattern at least three times.
//!
//! # Why this exists
//!
//! The ledger already records every fs operation across the platform, and the
//! automation engine already knows how to execute Quickflows. The missing
//! piece — until now — was the bridge that turns observed user behavior into
//! a saved automation rule. This module is that bridge, and only that bridge.
//!
//! # Design constraints
//!
//! - **DRY**: pure composition of two existing engines. No new event store,
//!   no duplicate watcher, no redundant query path.
//! - **Cheap**: one bounded ledger query per panel open (≤500 events). Pattern
//!   detection is O(N) over the result, so a single panel render costs a few
//!   milliseconds even on a heavily-used machine.
//! - **No new storage**: the dismissed-suggestion set lives in memory for the
//!   session. A restart re-evaluates patterns from scratch, which is fine —
//!   the user can dismiss again if the pattern is still uninteresting.
//! - **Conservative**: only `move` and `copy` patterns are surfaced. Rename
//!   patterns are skipped because rename recipes (date-prefix, lowercase,
//!   regex) are too varied to infer from raw events.

use crate::automation_engine::{AutomationRule, RuleAction, RuleTrigger};
use crate::core::error::AppError;
use crate::ledger::{LedgerQuery, OperationLedger};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Minimum number of times a pattern must repeat before it becomes a
/// suggestion. Three is the smallest number that demonstrates intentionality
/// — two is "I might do this again", three is "I always do this".
const MIN_REPEATS: u32 = 3;

/// Maximum number of suggestions surfaced at a time. The user should never
/// face a wall of suggestions; show the strongest five and let the rest
/// emerge as those are accepted or dismissed.
const MAX_SUGGESTIONS: usize = 5;

/// Maximum number of recent fs events to scan for patterns. Bounded so the
/// query is cheap regardless of how active the ledger has been.
const SCAN_LIMIT: u32 = 500;

/// A pattern detected in the ledger that the user could promote to a
/// Quickflow with one click. Sent over IPC to the React automation panel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuickflowSuggestion {
    /// Stable hash of `(kind, source_dir, dest_dir)`. Used as the dismissal
    /// key, the React list key, and the Accept/Dismiss IPC argument.
    pub id: String,
    /// Operation kind: `"copy"` or `"move"`.
    pub kind: String,
    /// Directory the user keeps moving/copying files OUT of.
    pub source_dir: String,
    /// Directory the user keeps moving/copying files INTO.
    pub dest_dir: String,
    /// Number of ledger events that match this pattern within the scan window.
    pub hit_count: u32,
    /// ISO-8601 timestamp of the most recent matching event.
    pub last_seen: String,
    /// Up to three sample filenames that show the user what would have been
    /// auto-handled if this Quickflow had already existed.
    pub sample_files: Vec<String>,
    /// Pre-computed friendly rule name the user can keep or edit.
    pub suggested_name: String,
}

/// Bridges the unified ledger and the automation engine. Stateless except
/// for the in-memory dismissal set; cloning is cheap.
#[derive(Clone)]
pub struct LedgerSuggester {
    ledger: OperationLedger,
    dismissed: Arc<RwLock<HashSet<String>>>,
}

impl LedgerSuggester {
    pub fn new(ledger: OperationLedger) -> Self {
        Self {
            ledger,
            dismissed: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Compute the current set of pattern suggestions. One ledger query;
    /// O(N) grouping over the bounded result.
    pub async fn suggestions(&self) -> Result<Vec<QuickflowSuggestion>, AppError> {
        let events = self
            .ledger
            .query(LedgerQuery {
                engine: Some("fs".to_string()),
                kind: None,
                subject_path: None,
                correlation_id: None,
                limit: Some(SCAN_LIMIT),
            })
            .await?;

        let dismissed = self.dismissed.read().await;
        let mut groups: HashMap<String, GroupAccum> = HashMap::new();

        for ev in &events {
            if ev.status != "ok" {
                continue;
            }
            if ev.kind != "copy" && ev.kind != "move" {
                continue;
            }
            let Some(subject) = ev.subject_path.as_ref() else {
                continue;
            };
            let Some(target) = ev.target_path.as_ref() else {
                continue;
            };
            let Some(source_dir) = parent_dir(subject) else {
                continue;
            };
            let Some(dest_dir) = parent_dir(target) else {
                continue;
            };
            // Skip degenerate patterns (rename in place — same parent dir).
            if source_dir == dest_dir {
                continue;
            }
            let id = pattern_id(&ev.kind, &source_dir, &dest_dir);
            if dismissed.contains(&id) {
                continue;
            }
            let entry = groups.entry(id.clone()).or_insert_with(|| GroupAccum {
                id,
                kind: ev.kind.clone(),
                source_dir,
                dest_dir,
                hit_count: 0,
                last_seen: ev.occurred_at.clone(),
                sample_files: Vec::new(),
            });
            entry.hit_count += 1;
            if ev.occurred_at > entry.last_seen {
                entry.last_seen = ev.occurred_at.clone();
            }
            if entry.sample_files.len() < 3 {
                if let Some(name) = file_name(subject) {
                    if !entry.sample_files.contains(&name) {
                        entry.sample_files.push(name);
                    }
                }
            }
        }

        let mut suggestions: Vec<QuickflowSuggestion> = groups
            .into_values()
            .filter(|g| g.hit_count >= MIN_REPEATS)
            .map(|g| QuickflowSuggestion {
                suggested_name: format!(
                    "{} from {} to {}",
                    capitalize(&g.kind),
                    short_dir(&g.source_dir),
                    short_dir(&g.dest_dir),
                ),
                id: g.id,
                kind: g.kind,
                source_dir: g.source_dir,
                dest_dir: g.dest_dir,
                hit_count: g.hit_count,
                last_seen: g.last_seen,
                sample_files: g.sample_files,
            })
            .collect();

        suggestions.sort_by(|a, b| {
            b.hit_count
                .cmp(&a.hit_count)
                .then_with(|| b.last_seen.cmp(&a.last_seen))
        });
        suggestions.truncate(MAX_SUGGESTIONS);
        Ok(suggestions)
    }

    /// Dismiss a suggestion for the rest of the session. Idempotent.
    pub async fn dismiss(&self, id: &str) {
        self.dismissed.write().await.insert(id.to_string());
    }

    /// Convert a suggestion into a fully-formed (but disabled-by-default)
    /// `AutomationRule`. The caller is responsible for persisting it via
    /// `AutomationManager::save_rule` — keeping that one-line wiring out of
    /// this module so the suggester stays free of the repo dependency.
    pub fn build_rule(suggestion: &QuickflowSuggestion) -> AutomationRule {
        let trigger = RuleTrigger::FileAppears {
            watch_path: suggestion.source_dir.clone(),
            recursive: false,
        };
        let action = match suggestion.kind.as_str() {
            "move" => RuleAction::MoveFile {
                dest_path: suggestion.dest_dir.clone(),
            },
            _ => RuleAction::CopyFile {
                dest_path: suggestion.dest_dir.clone(),
            },
        };
        AutomationRule::new(suggestion.suggested_name.clone(), trigger, action)
    }
}

struct GroupAccum {
    id: String,
    kind: String,
    source_dir: String,
    dest_dir: String,
    hit_count: u32,
    last_seen: String,
    sample_files: Vec<String>,
}

fn parent_dir(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn file_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

fn pattern_id(kind: &str, source_dir: &str, dest_dir: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    source_dir.hash(&mut hasher);
    dest_dir.hash(&mut hasher);
    format!("qfs-{:016x}", hasher.finish())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn short_dir(dir: &str) -> String {
    Path::new(dir)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| dir.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{LedgerEngine, LedgerStatus, RecordEvent};
    use crate::storage::Repository;

    async fn fresh_ledger() -> OperationLedger {
        let repo = Repository::open_in_memory().await.unwrap();
        OperationLedger::new(repo.pool().clone())
    }

    async fn record_move(ledger: &OperationLedger, from: &str, to: &str) {
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "move")
                    .status(LedgerStatus::Ok)
                    .subject(from)
                    .target(to),
            )
            .await;
    }

    async fn record_copy(ledger: &OperationLedger, from: &str, to: &str) {
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "copy")
                    .status(LedgerStatus::Ok)
                    .subject(from)
                    .target(to),
            )
            .await;
    }

    #[tokio::test]
    async fn empty_ledger_returns_no_suggestions() {
        let ledger = fresh_ledger().await;
        let suggester = LedgerSuggester::new(ledger);
        assert!(suggester.suggestions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn two_hits_below_threshold() {
        let ledger = fresh_ledger().await;
        record_move(&ledger, "/in/a.txt", "/out/a.txt").await;
        record_move(&ledger, "/in/b.txt", "/out/b.txt").await;
        let suggester = LedgerSuggester::new(ledger);
        assert!(suggester.suggestions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn three_hits_yield_one_suggestion() {
        let ledger = fresh_ledger().await;
        record_move(&ledger, "/in/a.txt", "/out/a.txt").await;
        record_move(&ledger, "/in/b.txt", "/out/b.txt").await;
        record_move(&ledger, "/in/c.txt", "/out/c.txt").await;
        let suggester = LedgerSuggester::new(ledger);
        let suggestions = suggester.suggestions().await.unwrap();
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert_eq!(s.kind, "move");
        assert_eq!(s.source_dir, "/in");
        assert_eq!(s.dest_dir, "/out");
        assert_eq!(s.hit_count, 3);
        assert_eq!(s.sample_files.len(), 3);
        assert!(s.suggested_name.contains("Move"));
        assert!(s.id.starts_with("qfs-"));
    }

    #[tokio::test]
    async fn dismissed_pattern_disappears() {
        let ledger = fresh_ledger().await;
        for i in 0..3 {
            record_move(&ledger, &format!("/in/{i}.txt"), &format!("/out/{i}.txt")).await;
        }
        let suggester = LedgerSuggester::new(ledger);
        let suggestions = suggester.suggestions().await.unwrap();
        assert_eq!(suggestions.len(), 1);
        suggester.dismiss(&suggestions[0].id).await;
        assert!(suggester.suggestions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rename_in_place_skipped() {
        let ledger = fresh_ledger().await;
        for i in 0..3 {
            record_move(
                &ledger,
                &format!("/d/{i}-old.txt"),
                &format!("/d/{i}-new.txt"),
            )
            .await;
        }
        let suggester = LedgerSuggester::new(ledger);
        // Source and dest dir are both "/d" — must NOT surface.
        assert!(suggester.suggestions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn copy_pattern_yields_copy_action() {
        let ledger = fresh_ledger().await;
        for i in 0..3 {
            record_copy(&ledger, &format!("/in/{i}.txt"), &format!("/out/{i}.txt")).await;
        }
        let suggester = LedgerSuggester::new(ledger);
        let suggestions = suggester.suggestions().await.unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].kind, "copy");
        let rule = LedgerSuggester::build_rule(&suggestions[0]);
        assert!(matches!(rule.action, RuleAction::CopyFile { ref dest_path } if dest_path == "/out"));
    }

    #[tokio::test]
    async fn build_rule_sets_trigger_and_action_for_move() {
        let suggestion = QuickflowSuggestion {
            id: "qfs-test".to_string(),
            kind: "move".to_string(),
            source_dir: "/inbox".to_string(),
            dest_dir: "/archive".to_string(),
            hit_count: 4,
            last_seen: "2026-04-10T00:00:00Z".to_string(),
            sample_files: vec!["a.pdf".to_string()],
            suggested_name: "Move from inbox to archive".to_string(),
        };
        let rule = LedgerSuggester::build_rule(&suggestion);
        assert!(
            matches!(rule.trigger, RuleTrigger::FileAppears { ref watch_path, recursive: false } if watch_path == "/inbox")
        );
        assert!(
            matches!(rule.action, RuleAction::MoveFile { ref dest_path } if dest_path == "/archive")
        );
        assert!(!rule.enabled, "newly built rule must be disabled by default");
    }

    #[tokio::test]
    async fn failed_events_are_ignored() {
        let ledger = fresh_ledger().await;
        for i in 0..3 {
            ledger
                .record(
                    RecordEvent::new(LedgerEngine::Fs, "move")
                        .status(LedgerStatus::Failed)
                        .subject(format!("/in/{i}.txt"))
                        .target(format!("/out/{i}.txt")),
                )
                .await;
        }
        let suggester = LedgerSuggester::new(ledger);
        // Failed events should not contribute to a suggestion.
        assert!(suggester.suggestions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn distinct_patterns_yield_distinct_suggestions() {
        let ledger = fresh_ledger().await;
        for i in 0..3 {
            record_move(&ledger, &format!("/in/{i}.txt"), &format!("/out/{i}.txt")).await;
        }
        for i in 0..4 {
            record_copy(
                &ledger,
                &format!("/src/{i}.txt"),
                &format!("/dst/{i}.txt"),
            )
            .await;
        }
        let suggester = LedgerSuggester::new(ledger);
        let suggestions = suggester.suggestions().await.unwrap();
        assert_eq!(suggestions.len(), 2);
        // The 4-hit copy pattern should be ranked first.
        assert_eq!(suggestions[0].kind, "copy");
        assert_eq!(suggestions[0].hit_count, 4);
        assert_eq!(suggestions[1].kind, "move");
        assert_eq!(suggestions[1].hit_count, 3);
    }
}
