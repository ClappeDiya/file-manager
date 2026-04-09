//! IPC commands for the Safety Interlock.
//!
//! Thin adapter layer that exposes [`SafetyInterlock`] to the frontend.
//! All real logic lives in the `safety` module — these handlers only
//! forward arguments and results.

use crate::safety::{OperationIntent, RiskAssessment, RiskLevel, SafetyInterlock};
use tauri::State;

/// Assess an operation's risk level against the user's history.
///
/// Fail-open: the backend never returns an error from this command. If
/// anything goes wrong internally, the interlock yields a `Low`
/// assessment with a diagnostic reason and the frontend is free to
/// proceed. This mirrors the behavior of the ledger itself and ensures
/// a broken safety layer can never block legitimate work.
#[tauri::command]
pub async fn safety_assess_intent(
    intent: OperationIntent,
    interlock: State<'_, SafetyInterlock>,
) -> Result<RiskAssessment, ()> {
    Ok(interlock.assess(&intent).await)
}

/// Record the user's explicit confirmation of a previously-assessed
/// medium/high-risk intent. Writes a `safety.confirmed` event to the
/// operation ledger so the confirmation is itself auditable.
///
/// Returns `true` on success. Fail-open: never errors.
#[tauri::command]
pub async fn safety_confirm_intent(
    intent: OperationIntent,
    intent_hash: String,
    level: RiskLevel,
    interlock: State<'_, SafetyInterlock>,
) -> Result<bool, ()> {
    interlock.confirm(&intent, &intent_hash, level).await;
    Ok(true)
}
