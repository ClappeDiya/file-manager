//! Tauri commands for AI features (T-043..T-045).
//!
//! Provides IPC handlers for error explanations, chat, suggestions,
//! natural language job creation, and safety controls.

use crate::ai_engine::{
    AiAssistant, AiAuditEntry, AiChatMessage, AiExecutionResult, AiFeatureToggles, AiSuggestion,
    ErrorExplanation, ParsedJobConfig, SuggestionContext,
};
use crate::automation_engine::AutomationManager;
use crate::core::error::AppError;
use crate::storage::Repository;
use tauri::State;

// ── T-043: Error Explanations ──

/// Explain an error in plain language.
#[tauri::command]
pub async fn ai_explain_error(
    error_type: String,
    message: String,
    advice: String,
    assistant: State<'_, AiAssistant>,
) -> Result<ErrorExplanation, AppError> {
    let explanation = assistant.explain_error_message(&error_type, &message, &advice);
    Ok(explanation)
}

// ── T-043: Chat ──

/// Send a chat message to the AI assistant.
#[tauri::command]
pub async fn ai_chat(
    message: String,
    assistant: State<'_, AiAssistant>,
) -> Result<AiChatMessage, AppError> {
    assistant.chat(&message).await
}

/// Get chat history.
#[tauri::command]
pub async fn ai_get_chat_history(
    assistant: State<'_, AiAssistant>,
) -> Result<Vec<AiChatMessage>, AppError> {
    Ok(assistant.get_chat_history().await)
}

/// Clear chat history.
#[tauri::command]
pub async fn ai_clear_chat(
    assistant: State<'_, AiAssistant>,
) -> Result<(), AppError> {
    assistant.clear_chat_history().await;
    Ok(())
}

// ── T-043: Suggestions ──

/// Generate contextual suggestions.
#[tauri::command]
pub async fn ai_generate_suggestions(
    context_json: String,
    assistant: State<'_, AiAssistant>,
) -> Result<Vec<AiSuggestion>, AppError> {
    let context: SuggestionContext = serde_json::from_str(&context_json).map_err(|e| {
        AppError::State {
            message: format!("Invalid suggestion context: {}", e),
            advice: "Provide valid JSON context.".to_string(),
        }
    })?;
    Ok(assistant.generate_suggestions(&context).await)
}

/// Get active suggestions.
#[tauri::command]
pub async fn ai_get_suggestions(
    assistant: State<'_, AiAssistant>,
) -> Result<Vec<AiSuggestion>, AppError> {
    Ok(assistant.get_suggestions().await)
}

/// Dismiss a suggestion.
#[tauri::command]
pub async fn ai_dismiss_suggestion(
    suggestion_id: String,
    assistant: State<'_, AiAssistant>,
) -> Result<(), AppError> {
    assistant.dismiss_suggestion(&suggestion_id).await
}

/// Accept a suggestion (returns action payload).
#[tauri::command]
pub async fn ai_accept_suggestion(
    suggestion_id: String,
    assistant: State<'_, AiAssistant>,
) -> Result<String, AppError> {
    assistant.accept_suggestion(&suggestion_id).await
}

// ── T-044: Natural Language Job Creation ──

/// Parse natural language into a job configuration.
#[tauri::command]
pub async fn ai_parse_natural_language(
    input: String,
    context_path: Option<String>,
    assistant: State<'_, AiAssistant>,
) -> Result<ParsedJobConfig, AppError> {
    assistant
        .parse_natural_language_with_context(&input, context_path.as_deref())
        .await
}

// ── Intent Bar: Ollama Integration ──

/// Probe whether Ollama is available locally.
#[tauri::command]
pub async fn ai_probe_ollama(
    assistant: State<'_, AiAssistant>,
) -> Result<bool, AppError> {
    Ok(assistant.probe_ollama().await)
}

/// Execute a parsed intent (the bridge from NL → engine actions).
#[tauri::command]
pub async fn ai_execute_parsed_job(
    config_json: String,
    assistant: State<'_, AiAssistant>,
    automation_mgr: State<'_, AutomationManager>,
    repo: State<'_, Repository>,
) -> Result<AiExecutionResult, AppError> {
    let config: ParsedJobConfig = serde_json::from_str(&config_json).map_err(|e| {
        AppError::State {
            message: format!("Invalid parsed job config: {e}"),
            advice: "The intent configuration is malformed.".to_string(),
        }
    })?;

    let audit_id = uuid::Uuid::new_v4().to_string();

    let result = match config.intent.as_str() {
        "automation" => {
            // Build an automation rule from the parsed config
            let rule =
                crate::ai_engine::parsed_job_to_automation_rule(&config, &config_json);
            let rule_name = rule.name.clone();
            let rule_id = rule.id.clone();
            automation_mgr.save_rule(&repo, &rule).await?;
            tracing::info!("Intent Bar: created automation rule '{rule_name}' ({rule_id})");

            AiExecutionResult {
                success: true,
                action_taken: format!("Created automation rule: {rule_name}"),
                entity_id: Some(rule_id),
                audit_id: audit_id.clone(),
            }
        }
        "sync" | "backup" => {
            // Return a preview result — actual sync pair creation requires
            // connection selection which needs user interaction
            let source = config
                .source_path
                .clone()
                .unwrap_or_else(|| "not specified".to_string());
            let dest = config
                .dest_connector
                .clone()
                .or(config.dest_path.clone())
                .unwrap_or_else(|| "not specified".to_string());
            let schedule_desc = config
                .schedule_human
                .clone()
                .unwrap_or_else(|| "manual".to_string());

            AiExecutionResult {
                success: true,
                action_taken: format!("sync:{source}→{dest}|schedule:{schedule_desc}"),
                entity_id: config.preview_config.clone(),
                audit_id: audit_id.clone(),
            }
        }
        "transfer" => {
            let source = config
                .source_path
                .clone()
                .unwrap_or_else(|| "not specified".to_string());
            let dest = config
                .dest_connector
                .clone()
                .or(config.dest_path.clone())
                .unwrap_or_else(|| "not specified".to_string());

            AiExecutionResult {
                success: true,
                action_taken: format!("transfer:{source}→{dest}"),
                entity_id: config.preview_config.clone(),
                audit_id: audit_id.clone(),
            }
        }
        "filter" | "navigate" => {
            // These are frontend-only actions — return info for the UI to act on
            let path = config.source_path.clone();
            let filter_info = serde_json::json!({
                "intent": config.intent,
                "path": path,
                "filter_patterns": config.filter_patterns,
                "exclude_patterns": config.exclude_patterns,
            });

            AiExecutionResult {
                success: true,
                action_taken: config.intent.clone(),
                entity_id: Some(filter_info.to_string()),
                audit_id: audit_id.clone(),
            }
        }
        "vault" => {
            let source = config.source_path.clone();
            AiExecutionResult {
                success: true,
                action_taken: "vault".to_string(),
                entity_id: source,
                audit_id: audit_id.clone(),
            }
        }
        _ => {
            AiExecutionResult {
                success: true,
                action_taken: format!("{}:{}", config.intent, config.source_path.clone().unwrap_or_default()),
                entity_id: config.preview_config.clone(),
                audit_id: audit_id.clone(),
            }
        }
    };

    // Log execution to audit
    assistant
        .log_ai_action_public(
            "intent_executed",
            &config.intent,
            &result.action_taken,
            true,
            false,
        )
        .await;

    Ok(result)
}

/// Get Ollama model name.
#[tauri::command]
pub async fn ai_get_ollama_model(
    assistant: State<'_, AiAssistant>,
) -> Result<String, AppError> {
    Ok(assistant.get_ollama_model().await)
}

/// Set Ollama model name.
#[tauri::command]
pub async fn ai_set_ollama_model(
    model: String,
    assistant: State<'_, AiAssistant>,
) -> Result<(), AppError> {
    assistant.set_ollama_model(model).await;
    Ok(())
}

// ── T-045: Safety Controls ──

/// Confirm a destructive action.
#[tauri::command]
pub async fn ai_confirm_action(
    action_id: String,
    assistant: State<'_, AiAssistant>,
) -> Result<bool, AppError> {
    assistant.confirm_destructive_action(&action_id).await
}

/// Check if an action requires confirmation.
#[tauri::command]
pub async fn ai_check_confirmation_needed(
    action_payload: String,
    assistant: State<'_, AiAssistant>,
) -> Result<bool, AppError> {
    Ok(assistant.requires_confirmation(&action_payload).await)
}

/// Get AI audit log.
#[tauri::command]
pub async fn ai_get_audit_log(
    limit: usize,
    offset: usize,
    assistant: State<'_, AiAssistant>,
) -> Result<Vec<AiAuditEntry>, AppError> {
    Ok(assistant.get_audit_log(limit, offset).await)
}

/// Get AI feature toggles.
#[tauri::command]
pub async fn ai_get_feature_toggles(
    assistant: State<'_, AiAssistant>,
) -> Result<AiFeatureToggles, AppError> {
    Ok(assistant.get_feature_toggles().await)
}

/// Update AI feature toggles.
#[tauri::command]
pub async fn ai_set_feature_toggles(
    toggles_json: String,
    assistant: State<'_, AiAssistant>,
) -> Result<(), AppError> {
    let toggles: AiFeatureToggles = serde_json::from_str(&toggles_json).map_err(|e| {
        AppError::Configuration {
            message: format!("Invalid feature toggles: {}", e),
            advice: "Provide valid toggle configuration.".to_string(),
        }
    })?;
    assistant.set_feature_toggles(toggles).await;
    Ok(())
}
