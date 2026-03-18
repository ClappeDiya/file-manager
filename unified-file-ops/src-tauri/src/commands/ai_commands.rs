//! Tauri commands for AI features (T-043..T-045).
//!
//! Provides IPC handlers for error explanations, chat, suggestions,
//! natural language job creation, and safety controls.

use crate::ai_engine::{
    AiAssistant, AiAuditEntry, AiChatMessage, AiFeatureToggles, AiSuggestion,
    ErrorExplanation, ParsedJobConfig, SuggestionContext,
};
use crate::core::error::AppError;
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
    assistant: State<'_, AiAssistant>,
) -> Result<ParsedJobConfig, AppError> {
    assistant.parse_natural_language(&input).await
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
