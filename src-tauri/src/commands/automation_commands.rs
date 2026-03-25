//! Tauri commands for the automation engine (Quickflows).

use crate::automation_engine::{AutomationLog, AutomationManager, AutomationRule};
use crate::core::error::AppError;
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
