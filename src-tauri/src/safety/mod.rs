//! Safety Interlock — context-aware anomaly detection for user operations.
//!
//! The Safety Interlock is a thin, zero-cost layer that sits between the user
//! and any potentially destructive or large-scale operation (transfer, sync,
//! delete, move, batch rename, etc.). Before such an operation runs, the
//! caller describes its *intent* (what engine, what kind, how many files, how
//! many bytes) and the interlock compares that intent against the user's own
//! recent history — sourced entirely from the existing [`OperationLedger`].
//!
//! Key design properties:
//!
//! - **Zero new infrastructure.** No new tables, no new daemons, no new
//!   dependencies. The interlock reads the ledger and writes back to the
//!   ledger. All data it needs already exists.
//! - **Zero cognitive load.** The interlock is *silent* for operations that
//!   match the user's typical behavior. It only surfaces a confirmation
//!   dialog when the intent is statistically anomalous — e.g. deleting 10×
//!   the usual file count, transferring 20× the usual byte volume, or
//!   performing a destructive operation with no precedent at all.
//! - **Per-user, per-engine baseline.** "Normal" is computed from the user's
//!   own last N ledger events for the same engine+kind, so the interlock
//!   adapts automatically to power users without nagging them.
//! - **Fail-open.** If the ledger is unavailable or the assessment errors,
//!   the interlock returns `RiskLevel::Low` with a diagnostic reason. It
//!   never blocks legitimate work because of an internal failure.
//!
//! The Safety Interlock composes existing engines (ledger, AI audit trail)
//! rather than adding a new subsystem. It is the smallest possible
//! accretive safety feature the platform could ship.

use crate::ledger::{LedgerEngine, LedgerEvent, LedgerQuery, LedgerStatus, OperationLedger, RecordEvent};
use serde::{Deserialize, Serialize};

/// Absolute floor for "destructive" file counts. Any delete/overwrite that
/// touches fewer than this number of files never escalates to Medium on
/// volume alone — beneath this threshold we assume the user has eyes on
/// what they're doing.
const DESTRUCTIVE_VOLUME_FLOOR: u64 = 25;

/// Absolute ceiling that always trips a High risk regardless of history.
/// A single-shot operation touching more files than this deserves
/// confirmation even for a user whose baseline is also large, because the
/// blast radius of a mistake at this scale is catastrophic.
const DESTRUCTIVE_VOLUME_HARD_CAP: u64 = 5_000;

/// Absolute ceiling for raw bytes (10 GiB) that always trips High on
/// transfer-shaped operations, even if the user has done larger transfers
/// historically. Large transfers are expensive mistakes.
const BYTES_HARD_CAP: u64 = 10 * 1024 * 1024 * 1024;

/// How many historical events of the same (engine, kind) to inspect when
/// building the baseline. Small enough to stay cheap, large enough to
/// smooth over noise.
const BASELINE_SAMPLE_SIZE: u32 = 50;

/// Multiplier over the historical median that trips Medium risk.
const MEDIUM_MULTIPLIER: f64 = 5.0;

/// Multiplier over the historical median that trips High risk.
const HIGH_MULTIPLIER: f64 = 10.0;

/// Kinds of operations the interlock recognises as inherently destructive.
/// A destructive kind with no baseline at all still trips Medium, because
/// "first time the user has ever deleted anything this large" is exactly
/// the moment a confirmation is most valuable.
const DESTRUCTIVE_KINDS: &[&str] = &[
    "delete", "trash", "purge", "mass_delete",
    "mirror", "overwrite", "rollback", "wipe",
    "replace", "move", "rename_batch",
];

/// A description of an operation the user is about to run. The caller fills
/// this in from whatever data it has at the moment it's about to invoke an
/// engine command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationIntent {
    /// Engine that will execute the operation (matches [`LedgerEngine`] names
    /// in snake_case: `"transfer"`, `"sync"`, `"fs"`, `"automation"`, …).
    pub engine: String,
    /// The operation kind. Matches the `kind` written to the ledger. The
    /// interlock pulls baseline from prior events of the same `(engine, kind)`.
    pub kind: String,
    /// Number of files/items the operation will touch. `0` means "unknown".
    pub affected_files: u64,
    /// Total bytes the operation will move or delete. `0` means "unknown".
    pub total_bytes: u64,
    /// Optional subject path (source root). Used purely for display in the
    /// confirmation dialog.
    pub subject_path: Option<String>,
    /// Optional human-readable one-line summary for the dialog.
    pub summary: Option<String>,
}

/// The severity of an assessed intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Within the user's usual pattern. Operation may proceed without
    /// confirmation.
    Low,
    /// Outside the user's usual pattern but not catastrophic. The UI should
    /// show a confirmation dialog but default the action to "Proceed".
    Medium,
    /// Far outside normal. The UI must show a confirmation dialog and the
    /// default action is "Cancel". The user has to deliberately override.
    High,
}

impl RiskLevel {
    /// Whether the UI should show a confirmation dialog for this level.
    pub fn requires_confirmation(self) -> bool {
        matches!(self, RiskLevel::Medium | RiskLevel::High)
    }
}

/// The result of assessing an [`OperationIntent`] against historical baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub level: RiskLevel,
    /// Human-readable reasons why this risk level was chosen. Always at
    /// least one entry; always safe to render directly to the user.
    pub reasons: Vec<String>,
    /// How many historical events the baseline was computed from. `0`
    /// means "no precedent — first time doing this".
    pub baseline_sample_size: u32,
    /// Median file count across the baseline sample. `0` if unknown.
    pub baseline_median_files: u64,
    /// Median byte count across the baseline sample. `0` if unknown.
    pub baseline_median_bytes: u64,
    /// Stable hash of the intent — the UI passes this to
    /// [`SafetyInterlock::confirm`] to record the user's explicit approval.
    pub intent_hash: String,
    /// Whether the UI should block and show a confirmation dialog.
    /// Mirror of `level.requires_confirmation()` for convenience on the
    /// frontend.
    pub requires_confirmation: bool,
}

/// Context-aware anomaly detection over the [`OperationLedger`].
///
/// Cloning is cheap — internally just clones the underlying ledger, which
/// itself wraps an `Arc<Mutex<Connection>>`.
#[derive(Clone)]
pub struct SafetyInterlock {
    ledger: OperationLedger,
}

impl SafetyInterlock {
    pub fn new(ledger: OperationLedger) -> Self {
        Self { ledger }
    }

    /// Assess an operation's risk level against the user's own history.
    ///
    /// Fail-open: any internal error is converted into a `Low` assessment
    /// with a diagnostic reason, so a broken ledger never blocks real work.
    pub async fn assess(&self, intent: &OperationIntent) -> RiskAssessment {
        let intent_hash = compute_intent_hash(intent);

        // Pull baseline (last N events of the same engine+kind). The ledger's
        // own `query` is already paginated and bounded.
        let baseline = match self
            .ledger
            .query(LedgerQuery {
                engine: Some(intent.engine.clone()),
                kind: Some(intent.kind.clone()),
                subject_path: None,
                correlation_id: None,
                limit: Some(BASELINE_SAMPLE_SIZE),
            })
            .await
        {
            Ok(events) => events,
            Err(e) => {
                // Fail-open: allow the operation but note the degradation.
                return RiskAssessment {
                    level: RiskLevel::Low,
                    reasons: vec![format!(
                        "safety: baseline unavailable ({}), defaulting to low risk",
                        e
                    )],
                    baseline_sample_size: 0,
                    baseline_median_files: 0,
                    baseline_median_bytes: 0,
                    intent_hash,
                    requires_confirmation: false,
                };
            }
        };

        self.classify(intent, &baseline, intent_hash)
    }

    /// Record the user's explicit approval of a previously-assessed intent.
    /// Written to the ledger as a [`LedgerEngine::System`] event with kind
    /// `"safety.confirmed"` so the confirmation is itself auditable and
    /// surfaces in the cross-engine timeline.
    ///
    /// Fail-open: never returns an error; logs internally if the ledger
    /// write fails.
    pub async fn confirm(&self, intent: &OperationIntent, intent_hash: &str, level: RiskLevel) {
        let details = serde_json::json!({
            "intent_hash": intent_hash,
            "engine": intent.engine,
            "kind": intent.kind,
            "affected_files": intent.affected_files,
            "total_bytes": intent.total_bytes,
            "level": match level {
                RiskLevel::Low => "low",
                RiskLevel::Medium => "medium",
                RiskLevel::High => "high",
            },
        })
        .to_string();

        let mut record = RecordEvent::new(LedgerEngine::System, "safety.confirmed")
            .status(LedgerStatus::Ok)
            .summary(format!(
                "User confirmed {} {} operation ({} files, {} bytes)",
                intent.engine, intent.kind, intent.affected_files, intent.total_bytes
            ))
            .details_json(details);

        if let Some(subject) = &intent.subject_path {
            record = record.subject(subject.clone());
        }

        self.ledger.record(record).await;
    }

    /// Core classification — pure function over the intent and a slice of
    /// historical events. Kept separate so it is directly testable without a
    /// database.
    fn classify(
        &self,
        intent: &OperationIntent,
        baseline: &[LedgerEvent],
        intent_hash: String,
    ) -> RiskAssessment {
        let sample_size = baseline.len() as u32;
        let (median_files, median_bytes) = compute_medians(baseline);

        let mut reasons: Vec<String> = Vec::new();
        let mut level = RiskLevel::Low;

        let is_destructive = DESTRUCTIVE_KINDS.iter().any(|k| *k == intent.kind);

        // Rule 1: absolute hard caps — always trip High, regardless of history.
        if intent.affected_files >= DESTRUCTIVE_VOLUME_HARD_CAP {
            reasons.push(format!(
                "Operation would touch {} files — above the absolute safety ceiling ({}).",
                intent.affected_files, DESTRUCTIVE_VOLUME_HARD_CAP
            ));
            level = RiskLevel::High;
        }
        if intent.total_bytes >= BYTES_HARD_CAP {
            reasons.push(format!(
                "Operation would move or delete {:.1} GiB — above the absolute safety ceiling.",
                intent.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            ));
            level = RiskLevel::High;
        }

        // Rule 2: destructive operation with no precedent at all ⇒ at least Medium.
        if is_destructive
            && sample_size == 0
            && intent.affected_files >= DESTRUCTIVE_VOLUME_FLOOR
            && level == RiskLevel::Low
        {
            reasons.push(format!(
                "First-ever {} of this kind on this device, and it touches {} files.",
                intent.kind, intent.affected_files
            ));
            level = RiskLevel::Medium;
        }

        // Rule 3: ratio against historical median.
        if sample_size > 0 && median_files > 0 && intent.affected_files >= DESTRUCTIVE_VOLUME_FLOOR {
            let ratio = intent.affected_files as f64 / median_files as f64;
            if ratio >= HIGH_MULTIPLIER && level != RiskLevel::High {
                reasons.push(format!(
                    "This operation touches {} files — {:.0}× your usual {} (median {} files over last {} runs).",
                    intent.affected_files, ratio, intent.kind, median_files, sample_size
                ));
                level = RiskLevel::High;
            } else if ratio >= MEDIUM_MULTIPLIER && level == RiskLevel::Low {
                reasons.push(format!(
                    "This operation touches {} files — {:.0}× your usual {} (median {} files).",
                    intent.affected_files, ratio, intent.kind, median_files
                ));
                level = RiskLevel::Medium;
            }
        }

        // Rule 4: byte-volume ratio — only for operations the ledger actually
        // tracks byte counts on.
        if sample_size > 0 && median_bytes > 0 && intent.total_bytes > 0 {
            let ratio = intent.total_bytes as f64 / median_bytes as f64;
            if ratio >= HIGH_MULTIPLIER && level != RiskLevel::High {
                reasons.push(format!(
                    "Byte volume is {:.0}× your usual {} (median {}).",
                    ratio,
                    intent.kind,
                    format_bytes(median_bytes)
                ));
                level = RiskLevel::High;
            } else if ratio >= MEDIUM_MULTIPLIER && level == RiskLevel::Low {
                reasons.push(format!(
                    "Byte volume is {:.0}× your usual {} (median {}).",
                    ratio,
                    intent.kind,
                    format_bytes(median_bytes)
                ));
                level = RiskLevel::Medium;
            }
        }

        if reasons.is_empty() {
            reasons.push(if sample_size == 0 {
                "No precedent yet — operation is within default safety bounds.".to_string()
            } else {
                format!(
                    "Within your usual pattern for {} operations ({} prior runs in baseline).",
                    intent.kind, sample_size
                )
            });
        }

        RiskAssessment {
            requires_confirmation: level.requires_confirmation(),
            level,
            reasons,
            baseline_sample_size: sample_size,
            baseline_median_files: median_files,
            baseline_median_bytes: median_bytes,
            intent_hash,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Stable, cheap, deterministic hash of an intent. Used as an opaque token
/// that the UI passes back to `confirm()` so the backend can audit which
/// exact assessment the user approved. Not a cryptographic hash — a
/// collision would just mean the audit log loses the ability to correlate
/// two identical intents, which is harmless.
fn compute_intent_hash(intent: &OperationIntent) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    intent.engine.hash(&mut h);
    intent.kind.hash(&mut h);
    intent.affected_files.hash(&mut h);
    intent.total_bytes.hash(&mut h);
    intent.subject_path.hash(&mut h);
    format!("intent-{:016x}", h.finish())
}

/// Compute the median file count and median byte count across a baseline of
/// ledger events. Ledger events don't carry a direct "affected_files" count,
/// so we approximate as follows:
///
/// - **Byte median** uses the `bytes` column directly (skipping `None`).
/// - **File count median** inspects `details_json` for a numeric field named
///   `files` or `count` or `affected_files`. If none of these are present
///   we treat each event as a single file. This is the right default
///   because most ledger events represent a single operation on a single
///   file.
///
/// Returns `(0, 0)` if the baseline is empty.
fn compute_medians(events: &[LedgerEvent]) -> (u64, u64) {
    if events.is_empty() {
        return (0, 0);
    }

    let mut bytes: Vec<u64> = events
        .iter()
        .filter_map(|e| e.bytes.and_then(|b| if b >= 0 { Some(b as u64) } else { None }))
        .collect();
    bytes.sort_unstable();
    let median_bytes = if bytes.is_empty() {
        0
    } else {
        bytes[bytes.len() / 2]
    };

    let mut files: Vec<u64> = events
        .iter()
        .map(|e| extract_file_count(&e.details_json))
        .collect();
    files.sort_unstable();
    let median_files = files[files.len() / 2];

    (median_files, median_bytes)
}

/// Pull a file-count-like integer out of a ledger event's `details_json`,
/// defaulting to `1` (one file per event) when no such field is present.
fn extract_file_count(details: &str) -> u64 {
    let parsed: serde_json::Value = match serde_json::from_str(details) {
        Ok(v) => v,
        Err(_) => return 1,
    };
    for key in ["affected_files", "files", "count", "file_count", "total_files"] {
        if let Some(n) = parsed.get(key).and_then(|v| v.as_u64()) {
            return n;
        }
    }
    1
}

/// Pretty-print a byte count for human-readable reasons.
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pool::DbPool;

    fn make_event(engine: &str, kind: &str, bytes: Option<i64>, details: &str) -> LedgerEvent {
        LedgerEvent {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
            engine: engine.to_string(),
            kind: kind.to_string(),
            status: "ok".to_string(),
            subject_path: None,
            target_path: None,
            bytes,
            correlation_id: None,
            summary: String::new(),
            details_json: details.to_string(),
            undo_token: None,
        }
    }

    fn make_interlock() -> SafetyInterlock {
        // We never call ledger.query() in classify() tests, so an in-memory
        // pool-backed ledger is sufficient and the tests remain hermetic.
        let pool = DbPool::open_in_memory().expect("in-memory pool");
        let ledger = OperationLedger::new(pool);
        SafetyInterlock::new(ledger)
    }

    #[test]
    fn risk_level_requires_confirmation_behaves() {
        assert!(!RiskLevel::Low.requires_confirmation());
        assert!(RiskLevel::Medium.requires_confirmation());
        assert!(RiskLevel::High.requires_confirmation());
    }

    #[test]
    fn median_of_empty_is_zero() {
        let (f, b) = compute_medians(&[]);
        assert_eq!(f, 0);
        assert_eq!(b, 0);
    }

    #[test]
    fn median_defaults_to_one_file_per_event() {
        let events = vec![
            make_event("transfer", "upload", Some(100), "{}"),
            make_event("transfer", "upload", Some(200), "{}"),
            make_event("transfer", "upload", Some(300), "{}"),
        ];
        let (files, bytes) = compute_medians(&events);
        assert_eq!(files, 1, "each event with no count field counts as 1");
        assert_eq!(bytes, 200);
    }

    #[test]
    fn median_reads_affected_files_from_details() {
        let events = vec![
            make_event("sync", "mirror", None, r#"{"affected_files": 10}"#),
            make_event("sync", "mirror", None, r#"{"affected_files": 20}"#),
            make_event("sync", "mirror", None, r#"{"affected_files": 30}"#),
        ];
        let (files, _) = compute_medians(&events);
        assert_eq!(files, 20);
    }

    #[test]
    fn extract_file_count_falls_back_to_one() {
        assert_eq!(extract_file_count("{}"), 1);
        assert_eq!(extract_file_count("not json"), 1);
        assert_eq!(extract_file_count(r#"{"files": 42}"#), 42);
        assert_eq!(extract_file_count(r#"{"count": 7}"#), 7);
    }

    #[test]
    fn classify_low_when_within_baseline() {
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "transfer".into(),
            kind: "upload".into(),
            affected_files: 3,
            total_bytes: 150,
            subject_path: None,
            summary: None,
        };
        let baseline = vec![
            make_event("transfer", "upload", Some(100), "{}"),
            make_event("transfer", "upload", Some(200), "{}"),
            make_event("transfer", "upload", Some(300), "{}"),
        ];
        let a = si.classify(&intent, &baseline, "h".into());
        assert_eq!(a.level, RiskLevel::Low);
        assert!(!a.requires_confirmation);
    }

    #[test]
    fn classify_high_on_absolute_file_cap() {
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "fs".into(),
            kind: "delete".into(),
            affected_files: DESTRUCTIVE_VOLUME_HARD_CAP + 1,
            total_bytes: 0,
            subject_path: None,
            summary: None,
        };
        let a = si.classify(&intent, &[], "h".into());
        assert_eq!(a.level, RiskLevel::High);
        assert!(a.requires_confirmation);
    }

    #[test]
    fn classify_high_on_absolute_bytes_cap() {
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "transfer".into(),
            kind: "upload".into(),
            affected_files: 1,
            total_bytes: BYTES_HARD_CAP + 1,
            subject_path: None,
            summary: None,
        };
        let a = si.classify(&intent, &[], "h".into());
        assert_eq!(a.level, RiskLevel::High);
    }

    #[test]
    fn classify_medium_on_destructive_first_run() {
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "fs".into(),
            kind: "delete".into(),
            affected_files: 100,
            total_bytes: 0,
            subject_path: None,
            summary: None,
        };
        let a = si.classify(&intent, &[], "h".into());
        assert_eq!(a.level, RiskLevel::Medium);
    }

    #[test]
    fn classify_low_on_destructive_below_floor_even_without_history() {
        // Deleting 3 files with no history is normal behavior — don't nag.
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "fs".into(),
            kind: "delete".into(),
            affected_files: 3,
            total_bytes: 0,
            subject_path: None,
            summary: None,
        };
        let a = si.classify(&intent, &[], "h".into());
        assert_eq!(a.level, RiskLevel::Low);
    }

    #[test]
    fn classify_high_on_10x_ratio() {
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "sync".into(),
            kind: "mirror".into(),
            affected_files: 500,
            total_bytes: 0,
            subject_path: None,
            summary: None,
        };
        // Baseline median is 20 files; 500/20 = 25× ⇒ High.
        let baseline = vec![
            make_event("sync", "mirror", None, r#"{"affected_files": 10}"#),
            make_event("sync", "mirror", None, r#"{"affected_files": 20}"#),
            make_event("sync", "mirror", None, r#"{"affected_files": 30}"#),
        ];
        let a = si.classify(&intent, &baseline, "h".into());
        assert_eq!(a.level, RiskLevel::High);
    }

    #[test]
    fn classify_medium_on_5x_ratio() {
        let si = make_interlock();
        let intent = OperationIntent {
            engine: "sync".into(),
            kind: "mirror".into(),
            affected_files: 120,
            total_bytes: 0,
            subject_path: None,
            summary: None,
        };
        // Baseline median 20; 120/20 = 6× ⇒ Medium (< 10× High threshold).
        let baseline = vec![
            make_event("sync", "mirror", None, r#"{"affected_files": 10}"#),
            make_event("sync", "mirror", None, r#"{"affected_files": 20}"#),
            make_event("sync", "mirror", None, r#"{"affected_files": 30}"#),
        ];
        let a = si.classify(&intent, &baseline, "h".into());
        assert_eq!(a.level, RiskLevel::Medium);
    }

    #[test]
    fn intent_hash_is_stable_and_distinct() {
        let a = OperationIntent {
            engine: "transfer".into(),
            kind: "upload".into(),
            affected_files: 10,
            total_bytes: 1000,
            subject_path: Some("/a".into()),
            summary: None,
        };
        let b = OperationIntent {
            affected_files: 11,
            ..a.clone()
        };
        assert_eq!(compute_intent_hash(&a), compute_intent_hash(&a));
        assert_ne!(compute_intent_hash(&a), compute_intent_hash(&b));
    }

    #[test]
    fn format_bytes_ranges() {
        assert_eq!(format_bytes(512), "512 B");
        assert!(format_bytes(2048).ends_with("KiB"));
        assert!(format_bytes(5 * 1024 * 1024).ends_with("MiB"));
        assert!(format_bytes(3 * 1024 * 1024 * 1024).ends_with("GiB"));
    }
}
