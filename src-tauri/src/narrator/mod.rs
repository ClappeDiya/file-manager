//! Operation Narrator — turns a correlation-id'd group of ledger events
//! into a plain-language cross-engine story.
//!
//! The Operation Ledger is already the source of truth for everything that
//! happens in the platform, and the Activity Timeline already lets a user
//! filter the timeline down to a single correlation-id trace. What was
//! missing was a way to answer the question every user actually asks:
//!
//!     "OK but what actually happened in this operation?"
//!
//! The raw event list answers that technically — N rows, each with a
//! timestamp and an engine-specific summary — but not intuitively. The
//! Operation Narrator reads the same rows and produces a short,
//! human-readable story describing outcomes, counts, durations, failures,
//! and the engines that participated.
//!
//! Design properties:
//!
//! - **Zero new infrastructure.** Reads only from [`OperationLedger`].
//!   No new tables, no new migrations, no new dependencies, no new
//!   background work.
//! - **Pure and deterministic.** The core `summarize()` function is a
//!   pure function over a slice of ledger events. No I/O, no side
//!   effects, fully testable without a database.
//! - **Zero cognitive overload.** Only runs when a user explicitly asks
//!   for a narrative ("Explain this operation" button on a correlation
//!   trace). Never fires automatically, never adds dashboard chrome.
//! - **Always works.** Does not require the AI engine, the network, or
//!   any optional component. A deterministic narrative is always
//!   returned, even for events with minimal metadata.
//! - **DRY composition.** Reuses `OperationLedger::query` with a
//!   correlation_id filter — exactly the same primitive the Activity
//!   Timeline's click-to-trace feature already uses.

use crate::core::error::AppError;
use crate::ledger::{LedgerEvent, LedgerQuery, OperationLedger};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single human-readable fact derived from the ledger events.
///
/// Facts are short, stand-alone sentences the UI can render as bullet
/// points beneath the one-line story.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrativeFact {
    /// Short category tag, lowercase snake_case (e.g. `"outcome"`,
    /// `"engine"`, `"volume"`, `"failure"`). The UI may choose icons
    /// based on this tag.
    pub tag: String,
    /// The fact text. Always a complete sentence, never truncated.
    pub text: String,
}

/// The narrative produced for one correlation-id trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationNarrative {
    pub correlation_id: String,
    /// Number of ledger events included in the narrative.
    pub event_count: usize,
    /// Engines that participated, ordered deterministically.
    pub engines_involved: Vec<String>,
    /// Span of the operation in milliseconds. `0` when the operation
    /// contains only a single event or timestamps cannot be parsed.
    pub duration_ms: u64,
    /// One-sentence "headline" summary, safe to render in a toast or
    /// list row. Always non-empty.
    pub headline: String,
    /// Short multi-sentence story that stitches the facts together in
    /// paragraph form. Always non-empty.
    pub story: String,
    /// Structured facts the UI can render as bullet points below the
    /// story. Ordered by importance (outcome first, then volume, then
    /// failures, then engines).
    pub facts: Vec<NarrativeFact>,
}

/// Reads correlation traces out of the operation ledger and narrates them.
///
/// Cloning is cheap — the underlying [`OperationLedger`] clones as an
/// `Arc<Mutex<Connection>>` reference.
#[derive(Clone)]
pub struct OperationNarrator {
    ledger: OperationLedger,
}

impl OperationNarrator {
    pub fn new(ledger: OperationLedger) -> Self {
        Self { ledger }
    }

    /// Fetch every event carrying `correlation_id` and produce a narrative.
    ///
    /// Returns an empty-but-valid narrative when no events are found, so
    /// the UI never has to handle a special "not found" branch.
    pub async fn narrate(&self, correlation_id: &str) -> Result<OperationNarrative, AppError> {
        let events = self
            .ledger
            .query(LedgerQuery {
                engine: None,
                kind: None,
                subject_path: None,
                correlation_id: Some(correlation_id.to_string()),
                limit: Some(1000),
            })
            .await?;

        Ok(summarize(correlation_id, &events))
    }
}

// ── Pure summarization ──────────────────────────────────────────────────

/// Pure function over a slice of ledger events. Exposed for direct unit
/// testing — the real `narrate()` method delegates to this after fetching
/// events.
///
/// Contract:
/// - Always returns a non-empty `headline` and `story`.
/// - Always preserves `correlation_id` in the result, even when no events
///   match (empty trace).
/// - Facts list is deterministic and ordered: outcome → volume → failures
///   → engines.
pub fn summarize(correlation_id: &str, events: &[LedgerEvent]) -> OperationNarrative {
    if events.is_empty() {
        return OperationNarrative {
            correlation_id: correlation_id.to_string(),
            event_count: 0,
            engines_involved: Vec::new(),
            duration_ms: 0,
            headline: format!("No ledger events recorded for operation {correlation_id}."),
            story: "This correlation id has no events in the ledger. It may have been pruned, or the operation never emitted a trace.".to_string(),
            facts: Vec::new(),
        };
    }

    // Aggregate counts by status and engine.
    let mut status_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut engine_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_bytes: u64 = 0;
    let mut failure_messages: Vec<String> = Vec::new();

    for event in events {
        *status_counts.entry(event.status.as_str()).or_insert(0) += 1;
        *engine_counts.entry(event.engine.clone()).or_insert(0) += 1;
        if let Some(b) = event.bytes {
            if b > 0 {
                total_bytes += b as u64;
            }
        }
        if event.status == "failed" && !event.summary.is_empty() {
            failure_messages.push(event.summary.clone());
        }
    }

    let event_count = events.len();
    let engines_involved: Vec<String> = engine_counts.keys().cloned().collect();
    let duration_ms = compute_span_ms(events);

    let ok = *status_counts.get("ok").unwrap_or(&0);
    let failed = *status_counts.get("failed").unwrap_or(&0);
    let skipped = *status_counts.get("skipped").unwrap_or(&0);
    let cancelled = *status_counts.get("cancelled").unwrap_or(&0);

    // Headline — a single tight sentence the UI can show in a row.
    let primary_engine = events
        .first()
        .map(|e| e.engine.clone())
        .unwrap_or_else(|| "operation".to_string());
    let headline_verb = outcome_verb(ok, failed, cancelled);
    let headline = format!(
        "{primary} {verb} {count} event{plural}{bytes}{duration}.",
        primary = primary_engine,
        verb = headline_verb,
        count = event_count,
        plural = if event_count == 1 { "" } else { "s" },
        bytes = if total_bytes > 0 {
            format!(" covering {}", format_bytes(total_bytes))
        } else {
            String::new()
        },
        duration = if duration_ms >= 1000 {
            format!(" over {}", format_duration(duration_ms))
        } else {
            String::new()
        },
    );

    // Story — a short paragraph that expands the headline.
    let mut story_parts: Vec<String> = Vec::new();

    let engines_sentence = if engines_involved.len() == 1 {
        format!("The operation ran on the {} engine.", engines_involved[0])
    } else {
        format!(
            "The operation crossed {} engines ({}).",
            engines_involved.len(),
            engines_involved.join(", ")
        )
    };
    story_parts.push(engines_sentence);

    if ok > 0 || failed > 0 || skipped > 0 || cancelled > 0 {
        let mut outcome_parts: Vec<String> = Vec::new();
        if ok > 0 {
            outcome_parts.push(format!("{ok} succeeded"));
        }
        if failed > 0 {
            outcome_parts.push(format!("{failed} failed"));
        }
        if skipped > 0 {
            outcome_parts.push(format!("{skipped} skipped"));
        }
        if cancelled > 0 {
            outcome_parts.push(format!("{cancelled} cancelled"));
        }
        story_parts.push(format!("Of {event_count} events: {}.", outcome_parts.join(", ")));
    }

    if total_bytes > 0 {
        story_parts.push(format!(
            "Total volume moved or touched: {}.",
            format_bytes(total_bytes)
        ));
    }

    if duration_ms >= 1000 {
        story_parts.push(format!("Wall-clock duration: {}.", format_duration(duration_ms)));
    }

    if !failure_messages.is_empty() {
        let shown = failure_messages.len().min(2);
        story_parts.push(format!(
            "First failure reason: \"{}\"{}.",
            failure_messages[0],
            if shown < failure_messages.len() {
                format!(" ({} more)", failure_messages.len() - shown)
            } else {
                String::new()
            }
        ));
    }

    let story = story_parts.join(" ");

    // Facts — ordered: outcome, volume, duration, failures, engines.
    let mut facts: Vec<NarrativeFact> = Vec::new();
    facts.push(NarrativeFact {
        tag: "outcome".to_string(),
        text: format!(
            "{ok} ok, {failed} failed, {skipped} skipped, {cancelled} cancelled across {event_count} events."
        ),
    });
    if total_bytes > 0 {
        facts.push(NarrativeFact {
            tag: "volume".to_string(),
            text: format!("Touched {} of data.", format_bytes(total_bytes)),
        });
    }
    if duration_ms >= 1000 {
        facts.push(NarrativeFact {
            tag: "duration".to_string(),
            text: format!("Took {} end-to-end.", format_duration(duration_ms)),
        });
    }
    for msg in failure_messages.iter().take(3) {
        facts.push(NarrativeFact {
            tag: "failure".to_string(),
            text: msg.clone(),
        });
    }
    for (engine, count) in engine_counts.iter() {
        facts.push(NarrativeFact {
            tag: "engine".to_string(),
            text: format!(
                "{engine}: {count} event{plural}",
                plural = if *count == 1 { "" } else { "s" }
            ),
        });
    }

    OperationNarrative {
        correlation_id: correlation_id.to_string(),
        event_count,
        engines_involved,
        duration_ms,
        headline,
        story,
        facts,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn outcome_verb(ok: usize, failed: usize, cancelled: usize) -> &'static str {
    if failed > 0 && ok == 0 {
        "failed across"
    } else if failed > 0 {
        "completed with failures across"
    } else if cancelled > 0 && ok == 0 {
        "was cancelled across"
    } else {
        "completed"
    }
}

/// Compute the millisecond span between the first and last event in the
/// slice. Returns `0` when there are fewer than two events or the
/// timestamps cannot be parsed.
fn compute_span_ms(events: &[LedgerEvent]) -> u64 {
    if events.len() < 2 {
        return 0;
    }

    let mut stamps: Vec<i64> = events
        .iter()
        .filter_map(|e| parse_ledger_timestamp(&e.occurred_at))
        .collect();

    if stamps.len() < 2 {
        return 0;
    }

    stamps.sort_unstable();
    let first = stamps.first().copied().unwrap_or(0);
    let last = stamps.last().copied().unwrap_or(0);
    let delta = last.saturating_sub(first);
    if delta < 0 {
        0
    } else {
        delta as u64
    }
}

/// Ledger timestamps are either RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`) or the
/// SQLite default (`YYYY-MM-DD HH:MM:SS`). Accept both.
fn parse_ledger_timestamp(raw: &str) -> Option<i64> {
    let rfc = if raw.contains('T') {
        raw.to_string()
    } else {
        format!("{}Z", raw.replace(' ', "T"))
    };
    DateTime::parse_from_rfc3339(&rfc)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;
    let b = bytes as f64;
    if b >= TIB {
        format!("{:.1} TiB", b / TIB)
    } else if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration(ms: u64) -> String {
    if ms >= 3_600_000 {
        let hours = ms / 3_600_000;
        let mins = (ms % 3_600_000) / 60_000;
        format!("{hours}h {mins}m")
    } else if ms >= 60_000 {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1_000;
        format!("{mins}m {secs}s")
    } else if ms >= 1_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn evt(engine: &str, status: &str, bytes: Option<i64>, summary: &str, ts: &str) -> LedgerEvent {
        LedgerEvent {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: ts.to_string(),
            engine: engine.to_string(),
            kind: "test_kind".to_string(),
            status: status.to_string(),
            subject_path: None,
            target_path: None,
            bytes,
            correlation_id: Some("corr-1".to_string()),
            summary: summary.to_string(),
            details_json: "{}".to_string(),
            undo_token: None,
        }
    }

    #[test]
    fn empty_events_returns_valid_empty_narrative() {
        let n = summarize("corr-xyz", &[]);
        assert_eq!(n.correlation_id, "corr-xyz");
        assert_eq!(n.event_count, 0);
        assert!(!n.headline.is_empty());
        assert!(!n.story.is_empty());
        assert!(n.facts.is_empty());
        assert_eq!(n.engines_involved.len(), 0);
    }

    #[test]
    fn single_ok_event_produces_simple_narrative() {
        let events = vec![evt("sync", "ok", Some(1024), "Mirrored 1 file", "2026-04-01T12:00:00Z")];
        let n = summarize("corr-1", &events);
        assert_eq!(n.event_count, 1);
        assert_eq!(n.engines_involved, vec!["sync".to_string()]);
        assert_eq!(n.duration_ms, 0); // single event has no span
        assert!(n.headline.contains("sync"));
        assert!(n.story.contains("sync"));
        // Outcome fact always present.
        assert!(n.facts.iter().any(|f| f.tag == "outcome"));
    }

    #[test]
    fn mixed_outcomes_are_summarised_correctly() {
        let events = vec![
            evt("transfer", "ok", Some(1000), "Uploaded part 1", "2026-04-01T12:00:00Z"),
            evt("transfer", "ok", Some(2000), "Uploaded part 2", "2026-04-01T12:00:05Z"),
            evt("transfer", "failed", Some(0), "403 forbidden", "2026-04-01T12:00:10Z"),
            evt("transfer", "skipped", Some(0), "Already exists", "2026-04-01T12:00:12Z"),
        ];
        let n = summarize("corr-2", &events);
        assert_eq!(n.event_count, 4);
        assert!(n.story.contains("2 succeeded"));
        assert!(n.story.contains("1 failed"));
        assert!(n.story.contains("1 skipped"));
        assert!(n.story.contains("403 forbidden"));
        assert!(n.facts.iter().any(|f| f.tag == "failure" && f.text.contains("403")));
        assert!(n.facts.iter().any(|f| f.tag == "volume"));
    }

    #[test]
    fn cross_engine_events_are_listed_once_each() {
        let events = vec![
            evt("sync", "ok", None, "Plan computed", "2026-04-01T12:00:00Z"),
            evt("fs", "ok", Some(500), "File written", "2026-04-01T12:00:01Z"),
            evt("transfer", "ok", Some(10_000), "Uploaded", "2026-04-01T12:00:02Z"),
            evt("fs", "ok", Some(500), "File written 2", "2026-04-01T12:00:03Z"),
        ];
        let n = summarize("corr-3", &events);
        assert_eq!(n.engines_involved.len(), 3);
        assert!(n.story.contains("3 engines"));
        // One engine fact per distinct engine.
        let engine_facts: Vec<&NarrativeFact> =
            n.facts.iter().filter(|f| f.tag == "engine").collect();
        assert_eq!(engine_facts.len(), 3);
    }

    #[test]
    fn duration_only_shown_when_meaningful() {
        // Sub-second spans are suppressed in narrative prose (they're noise).
        let events = vec![
            evt("sync", "ok", None, "start", "2026-04-01T12:00:00.000Z"),
            evt("sync", "ok", None, "end", "2026-04-01T12:00:00.500Z"),
        ];
        let n = summarize("corr-4", &events);
        assert!(!n.story.contains("Wall-clock"));
    }

    #[test]
    fn duration_shown_for_multi_second_ops() {
        let events = vec![
            evt("sync", "ok", None, "start", "2026-04-01T12:00:00Z"),
            evt("sync", "ok", None, "end", "2026-04-01T12:00:15Z"),
        ];
        let n = summarize("corr-5", &events);
        assert_eq!(n.duration_ms, 15_000);
        assert!(n.story.contains("Wall-clock"));
        assert!(n.facts.iter().any(|f| f.tag == "duration"));
    }

    #[test]
    fn format_bytes_ranges() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert!(format_bytes(5_000).ends_with("KiB"));
        assert!(format_bytes(5_000_000).ends_with("MiB"));
        assert!(format_bytes(5_000_000_000).ends_with("GiB"));
        assert!(format_bytes(2_000_000_000_000).ends_with("TiB"));
    }

    #[test]
    fn format_duration_ranges() {
        assert!(format_duration(500).ends_with("ms"));
        assert!(format_duration(5_500).ends_with("s"));
        assert!(format_duration(125_000).contains("m"));
        assert!(format_duration(3_800_000).contains("h"));
    }

    #[test]
    fn parse_ledger_timestamp_accepts_both_formats() {
        assert!(parse_ledger_timestamp("2026-04-01T12:00:00Z").is_some());
        assert!(parse_ledger_timestamp("2026-04-01 12:00:00").is_some());
        assert!(parse_ledger_timestamp("not a timestamp").is_none());
    }

    #[test]
    fn all_failed_events_produce_failure_headline() {
        let events = vec![
            evt("transfer", "failed", None, "err1", "2026-04-01T12:00:00Z"),
            evt("transfer", "failed", None, "err2", "2026-04-01T12:00:01Z"),
        ];
        let n = summarize("corr-6", &events);
        assert!(n.headline.contains("failed"));
    }

    #[test]
    fn headline_is_always_nonempty() {
        let cases: Vec<Vec<LedgerEvent>> = vec![
            vec![],
            vec![evt("x", "ok", None, "", "2026-04-01T12:00:00Z")],
            vec![evt("x", "failed", None, "oops", "2026-04-01T12:00:00Z")],
        ];
        for (i, events) in cases.iter().enumerate() {
            let n = summarize(&format!("corr-{i}"), events);
            assert!(!n.headline.is_empty(), "headline was empty for case {i}");
            assert!(!n.story.is_empty(), "story was empty for case {i}");
        }
    }

    #[test]
    fn engines_involved_is_deterministic() {
        let events = vec![
            evt("transfer", "ok", None, "", "2026-04-01T12:00:00Z"),
            evt("sync", "ok", None, "", "2026-04-01T12:00:01Z"),
            evt("fs", "ok", None, "", "2026-04-01T12:00:02Z"),
        ];
        let n1 = summarize("x", &events);
        let n2 = summarize("x", &events);
        assert_eq!(n1.engines_involved, n2.engines_involved);
        // BTreeMap ordering ⇒ alphabetical.
        assert_eq!(n1.engines_involved, vec!["fs", "sync", "transfer"]);
    }
}
