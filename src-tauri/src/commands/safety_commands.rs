//! IPC commands for the Safety Interlock.
//!
//! Thin adapter layer that exposes [`SafetyInterlock`] to the frontend.
//! All real logic lives in the `safety` module — these handlers only
//! forward arguments and results.

use crate::core::error::AppError;
use crate::safety::{OperationIntent, RiskAssessment, RiskLevel, SafetyInterlock};
use tauri::State;

/// Assess an operation's risk level against the user's history.
///
/// Fail-open: the backend currently never produces an `Err`. If anything
/// goes wrong internally, the interlock yields a `Low` assessment with
/// a diagnostic reason and the frontend is free to proceed. The
/// `Result<_, AppError>` shape is required by Tauri for async commands
/// with reference parameters and aligns this handler with the rest of
/// the IPC surface — should a future error path land (e.g. ledger write
/// failures), it can produce a structured `AppError` without rewriting
/// the signature.
#[tauri::command]
pub async fn safety_assess_intent(
    intent: OperationIntent,
    interlock: State<'_, SafetyInterlock>,
) -> Result<RiskAssessment, AppError> {
    Ok(interlock.assess(&intent).await)
}

/// Record the user's explicit confirmation of a previously-assessed
/// medium/high-risk intent. Writes a `safety.confirmed` event to the
/// operation ledger so the confirmation is itself auditable.
///
/// Returns `Ok(true)` on success. Currently fail-open (no error path
/// produced), but the canonical `Result<_, AppError>` shape leaves room
/// for a future fail-closed variant without a breaking signature change.
#[tauri::command]
pub async fn safety_confirm_intent(
    intent: OperationIntent,
    intent_hash: String,
    level: RiskLevel,
    interlock: State<'_, SafetyInterlock>,
) -> Result<bool, AppError> {
    interlock.confirm(&intent, &intent_hash, level).await;
    Ok(true)
}
