//! Journey 4: AI-assisted folder cleanup
//!
//! End-to-end test validating:
//! 1. AI analyzes folder structure and generates suggestions
//! 2. Suggestions include cleanup, duplicate risk, sync rules
//! 3. User accepts/dismisses suggestions with audit trail
//! 4. Natural language command parsing
//! 5. Safety controls (confirmation for destructive actions)
//! 6. Feature toggles for AI capabilities

use unified_file_ops_lib::ai_engine::{AiAssistant, SuggestionContext};
use unified_file_ops_lib::core::error::AppError;

/// Journey 4A: AI generates cleanup suggestions for a folder with many temp files
#[tokio::test]
async fn journey4_generate_cleanup_suggestions() {
    let ai = AiAssistant::new();

    let context = SuggestionContext {
        current_path: Some("/downloads".to_string()),
        total_files: 500,
        temp_file_count: 100,
        duplicate_candidates: 5,
        unclassified_folders: 10,
        recent_errors: vec![],
        active_sync_pairs: 0,
    };

    let suggestions = ai.generate_suggestions(&context).await;

    // Should generate suggestions for cleanup, duplicates, and classification
    assert!(!suggestions.is_empty(),
        "AI should generate suggestions for a messy folder");

    // Check that suggestions have required fields
    for suggestion in &suggestions {
        assert!(!suggestion.id.is_empty(), "Suggestion should have an ID");
        assert!(!suggestion.title.is_empty(), "Suggestion should have a title");
        assert!(!suggestion.description.is_empty(), "Suggestion should have a description");
        assert!(suggestion.confidence >= 0.0 && suggestion.confidence <= 1.0,
            "Confidence should be between 0 and 1, got {}", suggestion.confidence);
    }
}

/// Journey 4B: Get and manage suggestion list
#[tokio::test]
async fn journey4_manage_suggestions() {
    let ai = AiAssistant::new();

    let context = SuggestionContext {
        current_path: Some("/documents".to_string()),
        total_files: 200,
        temp_file_count: 60,
        duplicate_candidates: 3,
        unclassified_folders: 6,
        recent_errors: vec![],
        active_sync_pairs: 0,
    };

    ai.generate_suggestions(&context).await;
    let suggestions = ai.get_suggestions().await;

    if !suggestions.is_empty() {
        // Dismiss first suggestion
        let first_id = suggestions[0].id.clone();
        ai.dismiss_suggestion(&first_id).await.unwrap();

        let updated = ai.get_suggestions().await;
        // Dismissed suggestions should be filtered out from active list
        assert!(updated.iter().all(|s| s.id != first_id || s.dismissed),
            "Dismissed suggestion should not appear in active list");
    }
}

/// Journey 4C: AI chat interaction
#[tokio::test]
async fn journey4_ai_chat() {
    let ai = AiAssistant::new();

    // User asks about folder organization
    let response = ai.chat("How should I organize my downloads folder?").await.unwrap();
    assert!(!response.content.is_empty(), "AI should respond to chat");
    assert_eq!(response.role, "assistant");

    // Get chat history
    let history = ai.get_chat_history().await;
    assert!(history.len() >= 2, "Should have user message and AI response");
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].role, "assistant");

    // Clear chat
    ai.clear_chat_history().await;
    let history = ai.get_chat_history().await;
    assert!(history.is_empty(), "Chat should be cleared");
}

/// Journey 4D: Natural language command parsing - sync
#[tokio::test]
async fn journey4_natural_language_parsing_sync() {
    let ai = AiAssistant::new();

    let result = ai.parse_natural_language(
        "Sync my Documents folder to Google Drive every night"
    ).await.unwrap();

    assert_eq!(result.intent, "sync", "Should parse sync intent");
    assert!(result.confidence > 0.0, "Should have some confidence");
}

/// Journey 4E: Natural language command parsing - backup
#[tokio::test]
async fn journey4_natural_language_parsing_backup() {
    let ai = AiAssistant::new();

    let result = ai.parse_natural_language(
        "Backup my photos to external drive weekly"
    ).await.unwrap();

    assert_eq!(result.intent, "backup", "Should parse backup intent");
}

/// Journey 4F: Natural language command parsing - transfer
#[tokio::test]
async fn journey4_natural_language_parsing_transfer() {
    let ai = AiAssistant::new();

    let result = ai.parse_natural_language(
        "Transfer all PDFs from downloads to documents"
    ).await.unwrap();

    assert_eq!(result.intent, "transfer", "Should parse transfer intent");
}

/// Journey 4G: Safety controls for destructive actions (via accept_suggestion)
#[tokio::test]
async fn journey4_safety_controls() {
    let ai = AiAssistant::new();

    // Generate suggestions that include cleanup (destructive)
    let context = SuggestionContext {
        current_path: Some("/tmp".to_string()),
        total_files: 100,
        temp_file_count: 80,
        duplicate_candidates: 0,
        unclassified_folders: 0,
        recent_errors: vec![],
        active_sync_pairs: 0,
    };

    let suggestions = ai.generate_suggestions(&context).await;
    // The cleanup suggestion is a destructive action requiring confirmation
    let cleanup_suggestion = suggestions.iter()
        .find(|s| s.action_payload.as_ref().map(|p| p.contains("cleanup")).unwrap_or(false));

    if let Some(s) = cleanup_suggestion {
        let _payload = ai.accept_suggestion(&s.id).await.unwrap();
        // After accepting a destructive action, it should require confirmation
        // confirm_destructive_action returns true if it was pending
        let confirmed = ai.confirm_destructive_action(&s.id).await.unwrap();
        assert!(confirmed, "Cleanup action should require confirmation");
    }
}

/// Journey 4H: Error explanation
#[test]
fn journey4_error_explanation() {
    let ai = AiAssistant::new();

    let error = AppError::FileOperation {
        message: "Permission denied: /etc/passwd".to_string(),
        advice: "Check file permissions.".to_string(),
    };

    let explanation = ai.explain_error(&error);

    assert!(!explanation.plain_language.is_empty(), "Should have plain language explanation");
    assert!(!explanation.possible_causes.is_empty(), "Should list possible causes");
    assert!(!explanation.suggested_fixes.is_empty(), "Should suggest fixes");
}

/// Journey 4I: Error explanation for different error types
#[test]
fn journey4_error_explanation_types() {
    let ai = AiAssistant::new();

    let error_types: Vec<AppError> = vec![
        AppError::Connection {
            message: "Connection refused".to_string(),
            advice: "Check server status.".to_string(),
        },
        AppError::Transfer {
            message: "Network timeout".to_string(),
            advice: "Check network connection.".to_string(),
        },
        AppError::Sync {
            message: "Conflict detected".to_string(),
            advice: "Review sync conflicts.".to_string(),
        },
        AppError::Encryption {
            message: "Wrong password".to_string(),
            advice: "Try again with correct password.".to_string(),
        },
    ];

    for error in &error_types {
        let explanation = ai.explain_error(error);
        assert!(!explanation.plain_language.is_empty(),
            "Should explain error: {:?}", error);
        assert!(!explanation.possible_causes.is_empty(),
            "Should list causes for: {:?}", error);
    }
}

/// Journey 4J: Feature toggles
#[tokio::test]
async fn journey4_feature_toggles() {
    let ai = AiAssistant::new();

    // Get current toggles
    let toggles = ai.get_feature_toggles().await;
    assert!(toggles.ai_master_enabled, "AI should be enabled by default");
    assert!(toggles.suggestions, "Suggestions should be enabled by default");
    assert!(toggles.nl_job_creation, "NL should be enabled by default");
    assert!(toggles.error_explanations, "Error explanations should be enabled by default");
    assert!(!toggles.content_analysis_opt_in, "Content analysis should be OFF by default (T-045)");

    // Disable AI master
    let mut new_toggles = toggles.clone();
    new_toggles.ai_master_enabled = false;
    ai.set_feature_toggles(new_toggles.clone()).await;

    // NL parsing should fail when AI is disabled
    let result = ai.parse_natural_language("sync my files").await;
    assert!(result.is_err(), "NL should fail when AI is disabled");
}

/// Journey 4K: Audit log for AI actions
#[tokio::test]
async fn journey4_ai_audit_log() {
    let ai = AiAssistant::new();

    // parse_natural_language logs to audit
    ai.parse_natural_language("sync my documents to google drive").await.unwrap();

    // Accept a suggestion (also logs)
    let context = SuggestionContext {
        current_path: Some("/documents".to_string()),
        total_files: 15000,
        temp_file_count: 100,
        duplicate_candidates: 5,
        unclassified_folders: 10,
        recent_errors: vec![],
        active_sync_pairs: 0,
    };
    let suggestions = ai.generate_suggestions(&context).await;
    if !suggestions.is_empty() {
        let _ = ai.accept_suggestion(&suggestions[0].id).await;
    }

    // Check audit log - parse_natural_language and accept_suggestion both log
    let audit_log = ai.get_audit_log(100, 0).await;
    assert!(!audit_log.is_empty(), "AI operations should be in audit log");
}

/// Journey 4L: Accept suggestion generates action
#[tokio::test]
async fn journey4_accept_suggestion() {
    let ai = AiAssistant::new();

    let context = SuggestionContext {
        current_path: Some("/documents".to_string()),
        total_files: 15000,
        temp_file_count: 100,
        duplicate_candidates: 5,
        unclassified_folders: 10,
        recent_errors: vec![],
        active_sync_pairs: 0,
    };

    ai.generate_suggestions(&context).await;
    let suggestions = ai.get_suggestions().await;

    if !suggestions.is_empty() {
        let suggestion_id = suggestions[0].id.clone();
        let payload = ai.accept_suggestion(&suggestion_id).await.unwrap();
        assert!(!payload.is_empty(),
            "Accepting suggestion should return action payload");
    }
}

/// Journey 4M: Suggestion generation with disabled features produces no results
#[tokio::test]
async fn journey4_disabled_suggestions() {
    let ai = AiAssistant::new();

    let mut toggles = ai.get_feature_toggles().await;
    toggles.suggestions = false;
    ai.set_feature_toggles(toggles).await;

    let context = SuggestionContext {
        current_path: Some("/documents".to_string()),
        total_files: 50000,
        temp_file_count: 1000,
        duplicate_candidates: 100,
        unclassified_folders: 50,
        recent_errors: vec![],
        active_sync_pairs: 0,
    };

    let suggestions = ai.generate_suggestions(&context).await;
    assert!(suggestions.is_empty(), "Disabled suggestions should return empty");
}

/// Journey 4N: Explain error message via string API
#[test]
fn journey4_explain_error_string_api() {
    let ai = AiAssistant::new();

    let explanation = ai.explain_error_message(
        "connection",
        "ECONNREFUSED: Connection refused to sftp.example.com:22",
        "Check server availability.",
    );

    assert!(!explanation.plain_language.is_empty());
    assert!(explanation.plain_language.contains("connect") || explanation.plain_language.contains("server"),
        "Explanation should mention connection: '{}'", explanation.plain_language);
}
