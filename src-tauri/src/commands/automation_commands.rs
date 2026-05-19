//! Tauri commands for the automation engine (Quickflows).

use crate::automation_engine::executor::execute_rule_action;
use crate::automation_engine::pinner::{build_pinned_rule, build_retry_rule};
use crate::automation_engine::suggester::{LedgerSuggester, QuickflowSuggestion};
use crate::automation_engine::{AutomationLog, AutomationManager, AutomationRule};
use crate::core::error::AppError;
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::storage::Repository;
use tauri::State;

// ── CRUD ──

#[tauri::command]
pub async fn create_automation_rule(
    rule: AutomationRule,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<AutomationRule, AppError> {
    mgr.save_rule(&repo, &rule).await?;
    tracing::info!("Created automation rule '{}' ({})", rule.name, rule.id);
    Ok(rule)
}

#[tauri::command]
pub async fn update_automation_rule(
    rule: AutomationRule,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<AutomationRule, AppError> {
    mgr.save_rule(&repo, &rule).await?;
    tracing::info!("Updated automation rule '{}' ({})", rule.name, rule.id);
    Ok(rule)
}

#[tauri::command]
pub async fn delete_automation_rule(
    rule_id: String,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<(), AppError> {
    mgr.delete_rule(&repo, &rule_id).await?;
    tracing::info!("Deleted automation rule {}", rule_id);
    Ok(())
}

#[tauri::command]
pub async fn list_automation_rules(
    mgr: State<'_, AutomationManager>,
) -> Result<Vec<AutomationRule>, AppError> {
    Ok(mgr.list_rules().await)
}

#[tauri::command]
pub async fn get_automation_rule(
    rule_id: String,
    mgr: State<'_, AutomationManager>,
) -> Result<AutomationRule, AppError> {
    mgr.get_rule(&rule_id).await.ok_or_else(|| AppError::State {
        message: format!("Automation rule '{}' not found", rule_id),
        advice: "Check the rule ID.".to_string(),
    })
}

// ── Control ──

#[tauri::command]
pub async fn enable_automation_rule(
    rule_id: String,
    enabled: bool,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<(), AppError> {
    mgr.set_enabled(&repo, &rule_id, enabled).await?;
    tracing::info!(
        "Automation rule {} {}",
        rule_id,
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

#[tauri::command]
pub async fn run_automation_rule(
    rule_id: String,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<AutomationLog, AppError> {
    let log = mgr.run_rule(&repo, &rule_id).await?;
    Ok(log)
}

#[tauri::command]
pub async fn test_automation_rule(
    rule: AutomationRule,
    mgr: State<'_, AutomationManager>,
) -> Result<AutomationLog, AppError> {
    let log = mgr.test_rule(&rule).await?;
    Ok(log)
}

// ── Logs ──

#[tauri::command]
pub async fn list_automation_logs(
    rule_id: Option<String>,
    limit: Option<u32>,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<Vec<AutomationLog>, AppError> {
    let limit = limit.unwrap_or(50);
    mgr.list_logs(&repo, rule_id.as_deref(), limit).await
}

#[tauri::command]
pub async fn clear_automation_logs(
    rule_id: Option<String>,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<(), AppError> {
    mgr.clear_logs(&repo, rule_id.as_deref()).await
}

// ── NL Bridge ──

#[tauri::command]
pub async fn parse_automation_nl(
    input: String,
    ai: State<'_, crate::ai_engine::AiAssistant>,
) -> Result<AutomationRule, AppError> {
    let parsed = ai.parse_natural_language(&input).await?;
    let rule = crate::ai_engine::parsed_job_to_automation_rule(&parsed, &input);
    Ok(rule)
}

// ── Quickflow Suggester ──
//
// Bridges the unified operation ledger and the automation engine. Surfaces
// single-click "Save as Quickflow" suggestions whenever the user has
// demonstrated the same source-dir → dest-dir move/copy pattern at least
// three times. Pure derivation over existing data — no new storage, no
// new background tasks.

/// List the current pattern suggestions, freshly computed from the ledger.
#[tauri::command]
pub async fn quickflow_suggestions_list(
    suggester: State<'_, LedgerSuggester>,
) -> Result<Vec<QuickflowSuggestion>, AppError> {
    suggester.suggestions().await
}

/// Dismiss a suggestion for the rest of the session. Idempotent.
#[tauri::command]
pub async fn quickflow_suggestion_dismiss(
    id: String,
    suggester: State<'_, LedgerSuggester>,
) -> Result<(), AppError> {
    suggester.dismiss(&id).await;
    Ok(())
}

/// Convert a suggestion into a saved (but disabled-by-default) Quickflow.
/// The user must explicitly enable it from the automation panel before any
/// real-world side effects occur — keeping the safety story unchanged.
#[tauri::command]
pub async fn quickflow_suggestion_accept(
    id: String,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
    suggester: State<'_, LedgerSuggester>,
) -> Result<AutomationRule, AppError> {
    let suggestions = suggester.suggestions().await?;
    let suggestion = suggestions
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::State {
            message: format!("Quickflow suggestion '{id}' is no longer available"),
            advice: "Refresh the automation panel and try again.".to_string(),
        })?;
    let rule = LedgerSuggester::build_rule(&suggestion);
    mgr.save_rule(&repo, &rule).await?;
    // Dismiss so the same pattern doesn't immediately re-surface.
    suggester.dismiss(&id).await;
    tracing::info!(
        "Promoted Quickflow suggestion '{}' → rule '{}'",
        id,
        rule.id
    );
    Ok(rule)
}

// ── Operation Pin ──
//
// Symmetric to the Quickflow Suggester, but for a single specific past
// event rather than a recurring pattern. Looks up the correlation group
// in the unified ledger, builds a `Manual`-trigger rule with a
// `RuleAction::ReplayOp` action, and saves it disabled-by-default.
//
// The user enables it from the existing automation panel before any
// real-world side effects can occur — keeping the safety story intact.

/// Pin a single past ledger event as a re-runnable manual Quickflow.
/// The created rule is disabled until the user explicitly enables it
/// from the automation panel.
#[tauri::command]
pub async fn pin_ledger_event(
    correlation_id: String,
    ledger: State<'_, OperationLedger>,
    repo: State<'_, Repository>,
    mgr: State<'_, AutomationManager>,
) -> Result<AutomationRule, AppError> {
    let rule = build_pinned_rule(&correlation_id, &ledger).await?;
    mgr.save_rule(&repo, &rule).await?;
    tracing::info!(
        "Pinned ledger event '{}' → manual rule '{}' ({})",
        correlation_id,
        rule.name,
        rule.id
    );
    Ok(rule)
}

// ── Operation Retry ──
//
// Failure-side mirror of Operation Pin. Looks up the *failed* (or
// cancelled) correlation group, builds an ephemeral `ReplayOp` rule,
// executes it directly through the existing executor, and records the
// outcome to the unified ledger so the retry attempt appears on the
// activity timeline alongside the original failure.
//
// The rule is NOT persisted to the automation table — retries are
// one-shot operations, and persisting them would clutter the user's
// saved-rules list with disposable artifacts. The new ledger row
// (engine `automation`, kind `retry`) is the only durable record.

/// Re-attempt a failed (or cancelled) fs.copy / fs.move operation in
/// place, without persisting the rule. Records the outcome to the
/// ledger so the retry attempt is visible on the activity timeline.
///
/// Returns the [`AutomationLog`] from the executor so the caller can
/// surface a precise success / failure toast with files-affected counts
/// and the underlying error message.
#[tauri::command]
pub async fn retry_failed_event(
    correlation_id: String,
    ledger: State<'_, OperationLedger>,
) -> Result<AutomationLog, AppError> {
    let rule = build_retry_rule(&correlation_id, &ledger).await?;
    let log = execute_rule_action(&rule, None, false).await;

    // Record the retry outcome to the ledger so the activity timeline
    // captures the attempt. Engine="automation" + kind="retry" makes the
    // row immediately filterable; correlating to the *original* failed
    // group ties the retry to its cause without inventing a new field.
    let status = match log.status {
        crate::automation_engine::AutomationStatus::Success => LedgerStatus::Ok,
        crate::automation_engine::AutomationStatus::Failed => LedgerStatus::Failed,
        crate::automation_engine::AutomationStatus::Skipped => LedgerStatus::Skipped,
        crate::automation_engine::AutomationStatus::Running => LedgerStatus::Ok,
    };
    let summary = match (&log.error_message, log.files_affected.len()) {
        (Some(err), _) => format!("retry failed: {err}"),
        (None, 0) => "retry produced no files".to_string(),
        (None, 1) => "retry succeeded (1 file)".to_string(),
        (None, n) => format!("retry succeeded ({n} files)"),
    };
    let mut ev = RecordEvent::new(LedgerEngine::Automation, "retry")
        .status(status)
        .correlation(&correlation_id)
        .summary(summary);
    if let Some(first) = log.files_affected.first() {
        ev = ev.target(first.clone());
    }
    ledger.record(ev).await;

    tracing::info!(
        "Retried ledger event '{}' → status={:?}, files={}",
        correlation_id,
        log.status,
        log.files_affected.len()
    );
    Ok(log)
}
