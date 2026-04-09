//! File Lineage / Provenance Graph — answers "what is the complete life
//! story of this file?" using only the unified [`crate::ledger::OperationLedger`].
//!
//! # The idea in one sentence
//!
//! Every rename, move, copy, sync, transfer, and automation fire is already
//! stored in the ledger as a `(subject_path, target_path)` pair sharing a
//! correlation id with the other rows of the same operation. Given a file's
//! *current* path, walking those pairs backward in a breadth-first search
//! recovers every prior name the file ever had — and every event that
//! touched any of those names — without any new stored data.
//!
//! # Example
//!
//! Say the user did:
//!   1. `rename /a.txt → /b.txt`
//!   2. `move /b.txt → /docs/b.txt`
//!   3. `sync /docs/b.txt → /backup/b.txt`
//!
//! Each of these is one or more ledger rows. Starting from `/docs/b.txt`,
//! [`compute_lineage`] finds the move row (subject=`/b.txt`, target=`/docs/b.txt`),
//! adds `/b.txt` to the frontier, then finds the rename row (subject=`/a.txt`,
//! target=`/b.txt`), adds `/a.txt`. Done. The user gets the full causal chain.
//!
//! # Design goals
//!
//! - **Pure read**: never writes to the ledger. Zero state added.
//! - **DRY**: reuses [`crate::ledger::OperationLedger::events_touching_path`]
//!   as its one and only primitive. No duplicate SQL, no new tables.
//! - **Bounded**: BFS depth is capped (`max_depth`) and each hop is capped
//!   by the ledger helper's own `limit`. Cycles are impossible (visited set).
//! - **Fail-soft**: paths that have never been touched return an empty
//!   lineage, not an error. The caller can surface "no history yet" cleanly.

use crate::core::error::AppError;
use crate::ledger::{LedgerEvent, OperationLedger};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

/// Result of a lineage query. All fields are populated from the ledger only.
///
/// - `root_path` — what the caller asked about.
/// - `aliases` — every other path the file was ever known by, sorted.
/// - `events` — every ledger event that touched any alias, newest-first.
/// - `correlation_ids` — the distinct correlation ids across `events`,
///   suitable for handing to an existing correlation-trace UI.
/// - `truncated` — true if BFS hit the depth cap before the frontier was
///   exhausted. The frontend can surface a "history may be incomplete" hint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLineage {
    pub root_path: String,
    pub aliases: Vec<String>,
    pub events: Vec<LedgerEvent>,
    pub correlation_ids: Vec<String>,
    pub truncated: bool,
}

/// Per-hop row cap. A single file rarely touches more than a handful of
/// events, but a directory that was renamed could carry many. 500 is well
/// under the ledger helper's hard ceiling of 1000 and generous enough that
/// realistic workloads never truncate.
const PER_PATH_LIMIT: u32 = 500;

/// Default BFS depth cap. Pathological rename chains deeper than this are
/// extremely unusual; the cap protects against accidental O(N) walks over
/// the entire ledger if a bug ever makes `subject_path == target_path`.
pub const DEFAULT_MAX_DEPTH: u32 = 16;

/// Walk the ledger from `root_path` outward, collecting every event that
/// touches any alias the file has ever been known by.
///
/// - `max_depth` — maximum BFS depth. Clamped to `[1, 64]`. Pass
///   [`DEFAULT_MAX_DEPTH`] if unsure.
///
/// Fail-soft: an unknown path returns an empty lineage.
pub async fn compute_lineage(
    ledger: &OperationLedger,
    root_path: String,
    max_depth: u32,
) -> Result<FileLineage, AppError> {
    let max_depth = max_depth.clamp(1, 64);

    // Visited sets. `visited_paths` prevents infinite loops on self-touches;
    // `visited_event_ids` dedupes events across hops (a rename row matches
    // both its subject and its target, so it would otherwise be seen twice).
    let mut visited_paths: BTreeSet<String> = BTreeSet::new();
    let mut visited_event_ids: HashSet<String> = HashSet::new();
    let mut collected: Vec<LedgerEvent> = Vec::new();

    let mut frontier: Vec<String> = vec![root_path.clone()];
    visited_paths.insert(root_path.clone());

    let mut depth: u32 = 0;
    let mut truncated = false;

    while !frontier.is_empty() {
        if depth >= max_depth {
            truncated = true;
            break;
        }
        let mut next_frontier: Vec<String> = Vec::new();

        for path in frontier.drain(..) {
            let rows = ledger
                .events_touching_path(path, PER_PATH_LIMIT)
                .await?;
            for row in rows {
                if !visited_event_ids.insert(row.id.clone()) {
                    continue;
                }
                // Any new path mentioned by this row becomes a frontier
                // member for the next hop.
                for side in [row.subject_path.as_deref(), row.target_path.as_deref()]
                {
                    if let Some(p) = side {
                        if !p.is_empty() && visited_paths.insert(p.to_string()) {
                            next_frontier.push(p.to_string());
                        }
                    }
                }
                collected.push(row);
            }
        }
        frontier = next_frontier;
        depth += 1;
    }

    // Sort events newest-first to match the Activity Timeline's native
    // ordering — the frontend can drop them straight into the existing
    // TimelineRow renderer without re-sorting.
    collected.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at).then(b.id.cmp(&a.id)));

    // Distinct correlation ids, ordered to match the event order so the
    // "trace" surface can be driven by this list directly.
    let mut seen_corr: HashSet<String> = HashSet::new();
    let mut correlation_ids: Vec<String> = Vec::new();
    for ev in &collected {
        if let Some(cid) = ev.correlation_id.as_deref() {
            if seen_corr.insert(cid.to_string()) {
                correlation_ids.push(cid.to_string());
            }
        }
    }

    // Aliases exclude the root so the UI can display "also known as" cleanly.
    let aliases: Vec<String> = visited_paths
        .into_iter()
        .filter(|p| p != &root_path)
        .collect();

    Ok(FileLineage {
        root_path,
        aliases,
        events: collected,
        correlation_ids,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{LedgerEngine, OperationLedger, RecordEvent};
    use crate::storage::Repository;

    async fn fresh_ledger() -> OperationLedger {
        let repo = Repository::open_in_memory().await.unwrap();
        OperationLedger::new(repo.pool().clone())
    }

    /// A freshly-created path with no history returns an empty lineage
    /// rather than an error. The frontend wants to render "no history yet"
    /// instead of surfacing an error toast.
    #[tokio::test]
    async fn unknown_path_returns_empty_lineage() {
        let ledger = fresh_ledger().await;
        let lin = compute_lineage(&ledger, "/nope".into(), DEFAULT_MAX_DEPTH)
            .await
            .unwrap();
        assert_eq!(lin.root_path, "/nope");
        assert!(lin.aliases.is_empty());
        assert!(lin.events.is_empty());
        assert!(lin.correlation_ids.is_empty());
        assert!(!lin.truncated);
    }

    /// The classic case: rename chain A → B → C. Asking about C must find
    /// events touching all three names.
    #[tokio::test]
    async fn walks_rename_chain_backward() {
        let ledger = fresh_ledger().await;

        // Oldest op first so `occurred_at` orders them naturally.
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "rename")
                    .correlation("r1")
                    .subject("/a.txt")
                    .target("/b.txt")
                    .summary("rename a to b"),
            )
            .await;
        // A gap so SQLite's seconds-resolution timestamps separate the rows.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "move")
                    .correlation("r2")
                    .subject("/b.txt")
                    .target("/docs/b.txt")
                    .summary("move b into docs"),
            )
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Sync, "sync.file")
                    .correlation("r3")
                    .subject("/docs/b.txt")
                    .target("/backup/b.txt")
                    .summary("sync docs b to backup"),
            )
            .await;

        let lin = compute_lineage(&ledger, "/backup/b.txt".into(), DEFAULT_MAX_DEPTH)
            .await
            .unwrap();

        assert_eq!(lin.events.len(), 3);
        // Aliases: /a.txt, /b.txt, /docs/b.txt (root excluded)
        assert_eq!(
            lin.aliases,
            vec![
                "/a.txt".to_string(),
                "/b.txt".to_string(),
                "/docs/b.txt".to_string()
            ]
        );
        // Three distinct correlations, newest-first.
        assert_eq!(lin.correlation_ids, vec!["r3", "r2", "r1"]);
        assert!(!lin.truncated);
    }

    /// Lineage must dedupe events that match both subject AND target filters
    /// (a rename row where subject == target would otherwise be counted
    /// twice). Also tests cycle safety.
    #[tokio::test]
    async fn dedupes_self_touching_events() {
        let ledger = fresh_ledger().await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "touch")
                    .correlation("r1")
                    .subject("/x")
                    .target("/x")
                    .summary("noop"),
            )
            .await;
        let lin = compute_lineage(&ledger, "/x".into(), 4).await.unwrap();
        assert_eq!(lin.events.len(), 1);
        assert!(lin.aliases.is_empty());
    }

    /// Depth cap must trigger the `truncated` flag without over-running.
    #[tokio::test]
    async fn depth_cap_sets_truncated_flag() {
        let ledger = fresh_ledger().await;
        // Build a chain longer than the cap: /p0 → /p1 → /p2 → /p3
        for i in 0..4 {
            ledger
                .record(
                    RecordEvent::new(LedgerEngine::Fs, "rename")
                        .correlation(format!("r{i}"))
                        .subject(format!("/p{i}"))
                        .target(format!("/p{}", i + 1))
                        .summary("chain"),
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Query end of chain with depth=1 → only sees the last hop.
        let lin = compute_lineage(&ledger, "/p4".into(), 1).await.unwrap();
        // One BFS hop: finds the /p3→/p4 row and surfaces /p3 as an alias,
        // but hits the depth cap before it can walk from /p3.
        assert!(lin.truncated);
        assert_eq!(lin.events.len(), 1);
        assert_eq!(lin.aliases, vec!["/p3".to_string()]);
    }

    /// A completely separate file must NOT show up in the lineage of
    /// another file with a similar but different path.
    #[tokio::test]
    async fn isolates_unrelated_paths() {
        let ledger = fresh_ledger().await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "rename")
                    .correlation("target")
                    .subject("/a")
                    .target("/b"),
            )
            .await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "rename")
                    .correlation("unrelated")
                    .subject("/x")
                    .target("/y"),
            )
            .await;

        let lin = compute_lineage(&ledger, "/b".into(), DEFAULT_MAX_DEPTH)
            .await
            .unwrap();
        assert_eq!(lin.events.len(), 1);
        assert_eq!(lin.correlation_ids, vec!["target".to_string()]);
        assert!(!lin.aliases.contains(&"/x".to_string()));
        assert!(!lin.aliases.contains(&"/y".to_string()));
    }

    /// A diamond: two different ops both touch the same path; both must be
    /// collected without infinite looping.
    #[tokio::test]
    async fn collects_multiple_ops_on_same_path() {
        let ledger = fresh_ledger().await;
        // Two copies landing at the same destination from two sources.
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "copy")
                    .correlation("c1")
                    .subject("/src1/f")
                    .target("/dst/f"),
            )
            .await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "copy")
                    .correlation("c2")
                    .subject("/src2/f")
                    .target("/dst/f"),
            )
            .await;

        let lin = compute_lineage(&ledger, "/dst/f".into(), DEFAULT_MAX_DEPTH)
            .await
            .unwrap();
        assert_eq!(lin.events.len(), 2);
        assert_eq!(
            lin.aliases,
            vec!["/src1/f".to_string(), "/src2/f".to_string()]
        );
        assert!(lin.correlation_ids.contains(&"c1".to_string()));
        assert!(lin.correlation_ids.contains(&"c2".to_string()));
    }

    /// `events_touching_path` ledger helper — narrow unit check of the
    /// primitive that lineage depends on. Keeps the lineage contract
    /// defensible from a single-level test.
    #[tokio::test]
    async fn ledger_helper_matches_either_side() {
        let ledger = fresh_ledger().await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "rename")
                    .subject("/from")
                    .target("/to"),
            )
            .await;
        let by_subj = ledger
            .events_touching_path("/from".into(), 10)
            .await
            .unwrap();
        assert_eq!(by_subj.len(), 1);
        let by_targ = ledger
            .events_touching_path("/to".into(), 10)
            .await
            .unwrap();
        assert_eq!(by_targ.len(), 1);
        let by_other = ledger
            .events_touching_path("/other".into(), 10)
            .await
            .unwrap();
        assert_eq!(by_other.len(), 0);
    }
}
