//! AI engine - error explanations, suggestions, natural language job creation,
//! safety controls, and audit logging.
//!
//! Provides `AiAssistant` for AI-powered features (T-043..T-045).

use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ── AI Types ──

/// Category of AI suggestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionCategory {
    SyncRule,
    Exclusion,
    Cleanup,
    FolderClassification,
    DuplicateRisk,
    Performance,
    Security,
}

/// An AI-generated suggestion card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSuggestion {
    pub id: String,
    pub category: SuggestionCategory,
    pub title: String,
    pub description: String,
    /// Machine-readable action payload (JSON) for Accept.
    pub action_payload: Option<String>,
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
    pub dismissed: bool,
}

/// An AI chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatMessage {
    pub id: String,
    pub role: String,       // "user" | "assistant" | "system"
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

/// Error explanation result from AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorExplanation {
    pub original_error: String,
    pub plain_language: String,
    pub possible_causes: Vec<String>,
    pub suggested_fixes: Vec<String>,
    pub related_docs: Vec<String>,
}

/// A parsed natural-language job configuration (T-044).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedJobConfig {
    pub intent: String,         // "sync" | "transfer" | "backup" | "exclusion" | "rename"
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub dest_connector: Option<String>,  // e.g., "google_drive", "sftp", "local"
    pub schedule: Option<String>,        // cron expression
    pub schedule_human: Option<String>,  // "every night" etc.
    pub direction: Option<String>,       // "one_way" | "bidirectional"
    pub filter_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub rename_pattern: Option<String>,
    pub confidence: f32,
    pub needs_clarification: bool,
    pub clarification_questions: Vec<String>,
    /// Raw generated sync pair JSON for preview.
    pub preview_config: Option<String>,
}

/// An AI audit log entry (T-045).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub action_type: String,          // "suggestion_accepted", "nl_job_created", "error_explained", etc.
    pub input_summary: String,        // What the user asked / what triggered it
    pub output_summary: String,       // What the AI produced
    pub user_confirmed: bool,         // Whether user explicitly confirmed
    pub destructive: bool,            // Whether the action was destructive
    pub metadata: Option<String>,     // Additional JSON metadata
}

/// Enterprise AI feature toggles (T-045).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiFeatureToggles {
    pub ai_master_enabled: bool,
    pub error_explanations: bool,
    pub suggestions: bool,
    pub nl_job_creation: bool,
    pub content_analysis_opt_in: bool,
    pub model_routing: String,   // "local" | "cloud" | "auto"
}

impl Default for AiFeatureToggles {
    fn default() -> Self {
        Self {
            ai_master_enabled: true,
            error_explanations: true,
            suggestions: true,
            nl_job_creation: true,
            content_analysis_opt_in: false, // Off by default per T-045
            model_routing: "local".to_string(),
        }
    }
}

// ── AI Assistant ──

/// AI assistant that provides contextual help, suggestions, NL job creation,
/// and safety controls.
pub struct AiAssistant {
    chat_history: Arc<RwLock<Vec<AiChatMessage>>>,
    suggestions: Arc<RwLock<Vec<AiSuggestion>>>,
    audit_log: Arc<RwLock<Vec<AiAuditEntry>>>,
    feature_toggles: Arc<RwLock<AiFeatureToggles>>,
    pending_confirmations: Arc<RwLock<Vec<String>>>,
}

impl AiAssistant {
    pub fn new() -> Self {
        Self {
            chat_history: Arc::new(RwLock::new(Vec::new())),
            suggestions: Arc::new(RwLock::new(Vec::new())),
            audit_log: Arc::new(RwLock::new(Vec::new())),
            feature_toggles: Arc::new(RwLock::new(AiFeatureToggles::default())),
            pending_confirmations: Arc::new(RwLock::new(Vec::new())),
        }
    }

    // ── T-043: Error Explanations ──

    /// Generate a user-friendly explanation for an error.
    pub fn explain_error(&self, error: &AppError) -> ErrorExplanation {
        match error {
            AppError::FileOperation { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("A file operation could not be completed: {message}"),
                possible_causes: vec![
                    "The file or folder may not exist".to_string(),
                    "You may not have permission to access this location".to_string(),
                    "The disk might be full or read-only".to_string(),
                ],
                suggested_fixes: vec![advice.clone()],
                related_docs: vec!["File Operations Guide".to_string()],
            },
            AppError::Database { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("The application database encountered a problem: {message}"),
                possible_causes: vec![
                    "The database file may be corrupted".to_string(),
                    "Disk space may be insufficient".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Try restarting the application".to_string(),
                ],
                related_docs: vec![],
            },
            AppError::Connection { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("Could not connect to the remote server: {message}"),
                possible_causes: vec![
                    "The server may be offline or unreachable".to_string(),
                    "Your credentials may have expired".to_string(),
                    "A firewall may be blocking the connection".to_string(),
                    "The network may be unavailable".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Check your network connection".to_string(),
                    "Verify the server address and port".to_string(),
                ],
                related_docs: vec!["Connection Troubleshooting".to_string()],
            },
            AppError::Transfer { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("The file transfer was interrupted: {message}"),
                possible_causes: vec![
                    "The network connection was lost during transfer".to_string(),
                    "The destination ran out of disk space".to_string(),
                    "The source file was modified during transfer".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Try resuming the transfer".to_string(),
                    "Check available disk space on the destination".to_string(),
                ],
                related_docs: vec!["Transfer Recovery Guide".to_string()],
            },
            AppError::Sync { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("The sync operation encountered an issue: {message}"),
                possible_causes: vec![
                    "Files may have been changed on both sides".to_string(),
                    "The sync destination may be unavailable".to_string(),
                    "File name conflicts across platforms".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Review sync conflicts in the Sync panel".to_string(),
                    "Run a dry-run to preview changes before syncing".to_string(),
                ],
                related_docs: vec!["Sync Engine Guide".to_string()],
            },
            AppError::PermissionDenied { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("Access was denied: {message}"),
                possible_causes: vec![
                    "Insufficient file system permissions".to_string(),
                    "The file may be locked by another application".to_string(),
                    "Administrative privileges may be required".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Check file permissions with your system settings".to_string(),
                ],
                related_docs: vec!["Permission Management".to_string()],
            },
            AppError::Configuration { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("There is a configuration issue: {message}"),
                possible_causes: vec![
                    "A required setting is missing or invalid".to_string(),
                    "The configuration file may be corrupted".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Reset settings to defaults if the problem persists".to_string(),
                ],
                related_docs: vec!["Configuration Reference".to_string()],
            },
            AppError::State { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("An application state error occurred: {message}"),
                possible_causes: vec![
                    "The application state may be out of sync".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Restart the application to reset state".to_string(),
                ],
                related_docs: vec![],
            },
            AppError::Encryption { message, advice } => ErrorExplanation {
                original_error: message.clone(),
                plain_language: format!("An encryption error occurred: {message}"),
                possible_causes: vec![
                    "The password may be incorrect".to_string(),
                    "The encrypted data may be corrupted".to_string(),
                    "The vault may be locked".to_string(),
                    "The encryption key may have been compromised".to_string(),
                ],
                suggested_fixes: vec![
                    advice.clone(),
                    "Verify your password and try again".to_string(),
                    "Check that the vault file is not corrupted".to_string(),
                ],
                related_docs: vec!["Encryption & Vault Guide".to_string()],
            },
            AppError::Internal { .. } => ErrorExplanation {
                original_error: "Internal error".to_string(),
                plain_language: "An unexpected internal error occurred. The application encountered a situation it was not prepared to handle.".to_string(),
                possible_causes: vec![
                    "This may be caused by a software bug".to_string(),
                    "System resources may be exhausted".to_string(),
                ],
                suggested_fixes: vec![
                    "Try restarting the application".to_string(),
                    "If the problem persists, check for application updates".to_string(),
                ],
                related_docs: vec!["Reporting Issues".to_string()],
            },
        }
    }

    /// Explain a raw error message string (for context menu "Explain this error").
    pub fn explain_error_message(&self, error_type: &str, message: &str, advice: &str) -> ErrorExplanation {
        let app_error = match error_type {
            "file_operation" => AppError::FileOperation {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            "connection" => AppError::Connection {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            "transfer" => AppError::Transfer {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            "sync" => AppError::Sync {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            "permission_denied" => AppError::PermissionDenied {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            "configuration" => AppError::Configuration {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            "encryption" => AppError::Encryption {
                message: message.to_string(),
                advice: advice.to_string(),
            },
            _ => AppError::Internal {
                message: message.to_string(),
            },
        };
        self.explain_error(&app_error)
    }

    // ── T-043: AI Suggestions ──

    /// Generate contextual suggestions based on current state.
    pub async fn generate_suggestions(
        &self,
        context: &SuggestionContext,
    ) -> Vec<AiSuggestion> {
        let toggles = self.feature_toggles.read().await;
        if !toggles.ai_master_enabled || !toggles.suggestions {
            return vec![];
        }
        drop(toggles);

        let mut suggestions = Vec::new();
        let now = Utc::now();

        // Analyze for sync rule suggestions
        if let Some(ref path) = context.current_path {
            if path.contains("Documents") || path.contains("documents") {
                suggestions.push(AiSuggestion {
                    id: Uuid::new_v4().to_string(),
                    category: SuggestionCategory::SyncRule,
                    title: "Set up automatic sync for Documents".to_string(),
                    description: "Your Documents folder could benefit from automatic syncing. Set up a sync pair to keep it backed up.".to_string(),
                    action_payload: Some(serde_json::json!({
                        "action": "create_sync_pair",
                        "source_path": path,
                        "mode": "one_way_source",
                        "trigger": "watch"
                    }).to_string()),
                    confidence: 0.75,
                    created_at: now,
                    dismissed: false,
                });
            }
        }

        // Check for large folders that could benefit from exclusions
        if context.total_files > 10000 {
            suggestions.push(AiSuggestion {
                id: Uuid::new_v4().to_string(),
                category: SuggestionCategory::Exclusion,
                title: "Consider adding exclusion patterns".to_string(),
                description: format!(
                    "This location contains {} files. Adding exclusion patterns for temporary or build files can speed up sync operations.",
                    context.total_files
                ),
                action_payload: Some(serde_json::json!({
                    "action": "add_exclusions",
                    "patterns": ["node_modules/", ".git/", "*.tmp", "*.log", "__pycache__/", "target/"]
                }).to_string()),
                confidence: 0.85,
                created_at: now,
                dismissed: false,
            });
        }

        // Cleanup suggestion for temp files
        if context.temp_file_count > 50 {
            suggestions.push(AiSuggestion {
                id: Uuid::new_v4().to_string(),
                category: SuggestionCategory::Cleanup,
                title: "Clean up temporary files".to_string(),
                description: format!(
                    "Found {} temporary files that can be safely cleaned up to free disk space.",
                    context.temp_file_count
                ),
                action_payload: Some(serde_json::json!({
                    "action": "cleanup",
                    "patterns": ["*.tmp", "*.bak", "*.swp", "~*"],
                    "requires_confirmation": true
                }).to_string()),
                confidence: 0.70,
                created_at: now,
                dismissed: false,
            });
        }

        // Duplicate risk detection
        if context.duplicate_candidates > 0 {
            suggestions.push(AiSuggestion {
                id: Uuid::new_v4().to_string(),
                category: SuggestionCategory::DuplicateRisk,
                title: "Potential duplicate files detected".to_string(),
                description: format!(
                    "Found {} files with identical names in different locations. Review to avoid sync conflicts.",
                    context.duplicate_candidates
                ),
                action_payload: Some(serde_json::json!({
                    "action": "review_duplicates",
                    "count": context.duplicate_candidates
                }).to_string()),
                confidence: 0.65,
                created_at: now,
                dismissed: false,
            });
        }

        // Folder classification suggestion
        if context.unclassified_folders > 5 {
            suggestions.push(AiSuggestion {
                id: Uuid::new_v4().to_string(),
                category: SuggestionCategory::FolderClassification,
                title: "Organize folders with auto-classification".to_string(),
                description: "Several folders could benefit from classification labels to improve sync and search.".to_string(),
                action_payload: Some(serde_json::json!({
                    "action": "classify_folders",
                    "count": context.unclassified_folders
                }).to_string()),
                confidence: 0.60,
                created_at: now,
                dismissed: false,
            });
        }

        // Store suggestions
        let mut stored = self.suggestions.write().await;
        stored.extend(suggestions.clone());

        suggestions
    }

    /// Get all active (non-dismissed) suggestions.
    pub async fn get_suggestions(&self) -> Vec<AiSuggestion> {
        let suggestions = self.suggestions.read().await;
        suggestions.iter().filter(|s| !s.dismissed).cloned().collect()
    }

    /// Dismiss a suggestion.
    pub async fn dismiss_suggestion(&self, suggestion_id: &str) -> Result<(), AppError> {
        let mut suggestions = self.suggestions.write().await;
        if let Some(s) = suggestions.iter_mut().find(|s| s.id == suggestion_id) {
            s.dismissed = true;
            Ok(())
        } else {
            Err(AppError::State {
                message: format!("Suggestion '{}' not found", suggestion_id),
                advice: "The suggestion may have already been dismissed.".to_string(),
            })
        }
    }

    /// Accept a suggestion (logs to audit trail, returns action payload).
    pub async fn accept_suggestion(&self, suggestion_id: &str) -> Result<String, AppError> {
        let mut suggestions = self.suggestions.write().await;
        let suggestion = suggestions
            .iter_mut()
            .find(|s| s.id == suggestion_id)
            .ok_or_else(|| AppError::State {
                message: format!("Suggestion '{}' not found", suggestion_id),
                advice: "The suggestion may have expired.".to_string(),
            })?;

        let payload = suggestion.action_payload.clone().unwrap_or_default();
        suggestion.dismissed = true;

        // Check if action is destructive and requires confirmation
        let is_destructive = payload.contains("\"requires_confirmation\":true")
            || payload.contains("\"action\":\"cleanup\"")
            || payload.contains("\"action\":\"delete\"");

        if is_destructive {
            // Store pending confirmation
            let mut pending = self.pending_confirmations.write().await;
            pending.push(suggestion_id.to_string());
        }

        // Log to audit trail
        self.log_ai_action(
            "suggestion_accepted",
            &suggestion.title,
            &payload,
            !is_destructive,
            is_destructive,
        ).await;

        Ok(payload)
    }

    // ── T-044: Natural Language Job Creation ──

    /// Parse a natural language command into a job configuration.
    pub async fn parse_natural_language(&self, input: &str) -> Result<ParsedJobConfig, AppError> {
        let toggles = self.feature_toggles.read().await;
        if !toggles.ai_master_enabled || !toggles.nl_job_creation {
            return Err(AppError::Configuration {
                message: "AI natural language processing is disabled".to_string(),
                advice: "Enable AI features in Settings > AI Controls.".to_string(),
            });
        }
        drop(toggles);

        let input_lower = input.to_lowercase();

        // Parse intent
        let intent = if input_lower.contains("sync") || input_lower.contains("synchronize") {
            "sync"
        } else if input_lower.contains("transfer") || input_lower.contains("send") || input_lower.contains("upload") {
            "transfer"
        } else if input_lower.contains("backup") || input_lower.contains("back up") {
            "backup"
        } else if input_lower.contains("exclude") || input_lower.contains("ignore") || input_lower.contains("skip") {
            "exclusion"
        } else if input_lower.contains("rename") || input_lower.contains("name pattern") {
            "rename"
        } else {
            "sync" // default intent
        };

        // Parse source path
        let source_path = self.extract_path(&input_lower, &["from", "my", "the"]);

        // Parse destination / connector
        let (dest_path, dest_connector) = self.extract_destination(&input_lower);

        // Parse schedule
        let (schedule, schedule_human) = self.extract_schedule(&input_lower);

        // Parse direction
        let direction = if input_lower.contains("bidirectional") || input_lower.contains("two-way") || input_lower.contains("both ways") {
            Some("bidirectional".to_string())
        } else {
            Some("one_way".to_string())
        };

        // Parse filter patterns
        let filter_patterns = self.extract_patterns(&input_lower, true);
        let exclude_patterns = self.extract_patterns(&input_lower, false);

        // Parse rename pattern
        let rename_pattern = if intent == "rename" {
            self.extract_rename_pattern(&input_lower)
        } else {
            None
        };

        // Determine if we need clarification
        let mut clarification_questions = Vec::new();
        let mut confidence: f32 = 0.85;

        if source_path.is_none() {
            clarification_questions.push("Which folder or files would you like to use as the source?".to_string());
            confidence -= 0.3;
        }
        if dest_connector.is_none() && dest_path.is_none() {
            clarification_questions.push("Where should the files be sent or synced to?".to_string());
            confidence -= 0.2;
        }
        if intent == "sync" && schedule.is_none() {
            clarification_questions.push("How often should this run? (e.g., every hour, daily, weekly)".to_string());
            confidence -= 0.1;
        }

        let needs_clarification = !clarification_questions.is_empty();

        // Build preview config
        let preview_config = if !needs_clarification || confidence > 0.5 {
            Some(serde_json::json!({
                "name": format!("{} job", intent),
                "source_path": source_path.clone().unwrap_or_default(),
                "dest_path": dest_path.clone().unwrap_or_default(),
                "mode": if direction.as_deref() == Some("bidirectional") { "bidirectional" } else { "one_way_source" },
                "trigger": if schedule.is_some() { "scheduled" } else { "manual" },
                "cron_expr": schedule.clone(),
                "filter": {
                    "include_patterns": filter_patterns,
                    "exclude_patterns": exclude_patterns
                }
            }).to_string())
        } else {
            None
        };

        // Log to audit trail
        self.log_ai_action(
            "nl_parse",
            input,
            &format!("Intent: {}, confidence: {:.0}%", intent, confidence * 100.0),
            false,
            false,
        ).await;

        Ok(ParsedJobConfig {
            intent: intent.to_string(),
            source_path,
            dest_path,
            dest_connector,
            schedule,
            schedule_human,
            direction,
            filter_patterns: vec![],
            exclude_patterns: vec![],
            rename_pattern,
            confidence,
            needs_clarification,
            clarification_questions,
            preview_config,
        })
    }

    // ── T-043: Chat ──

    /// Send a chat message and get a response.
    pub async fn chat(&self, user_message: &str) -> Result<AiChatMessage, AppError> {
        let toggles = self.feature_toggles.read().await;
        if !toggles.ai_master_enabled {
            return Err(AppError::Configuration {
                message: "AI features are disabled".to_string(),
                advice: "Enable AI in Settings > AI Controls.".to_string(),
            });
        }
        drop(toggles);

        let user_msg = AiChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content: user_message.to_string(),
            timestamp: Utc::now(),
        };

        // Store user message
        let mut history = self.chat_history.write().await;
        history.push(user_msg);

        // Generate response based on content analysis
        let response_content = self.generate_chat_response(user_message).await;

        let assistant_msg = AiChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: response_content,
            timestamp: Utc::now(),
        };

        history.push(assistant_msg.clone());

        Ok(assistant_msg)
    }

    /// Get chat history.
    pub async fn get_chat_history(&self) -> Vec<AiChatMessage> {
        let history = self.chat_history.read().await;
        history.clone()
    }

    /// Clear chat history.
    pub async fn clear_chat_history(&self) {
        let mut history = self.chat_history.write().await;
        history.clear();
    }

    // ── T-045: Safety Controls ──

    /// Confirm a destructive action.
    pub async fn confirm_destructive_action(&self, action_id: &str) -> Result<bool, AppError> {
        let mut pending = self.pending_confirmations.write().await;
        if let Some(pos) = pending.iter().position(|id| id == action_id) {
            pending.remove(pos);

            // Log confirmation
            self.log_ai_action(
                "destructive_confirmed",
                action_id,
                "User confirmed destructive action",
                true,
                true,
            ).await;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Check if an action requires confirmation.
    pub async fn requires_confirmation(&self, action_payload: &str) -> bool {
        action_payload.contains("\"requires_confirmation\":true")
            || action_payload.contains("cleanup")
            || action_payload.contains("delete")
            || action_payload.contains("rollback")
            || action_payload.contains("overwrite")
    }

    /// Get all AI audit log entries.
    pub async fn get_audit_log(&self, limit: usize, offset: usize) -> Vec<AiAuditEntry> {
        let log = self.audit_log.read().await;
        let start = offset.min(log.len());
        let end = (start + limit).min(log.len());
        log[start..end].to_vec()
    }

    /// Get feature toggles.
    pub async fn get_feature_toggles(&self) -> AiFeatureToggles {
        let toggles = self.feature_toggles.read().await;
        toggles.clone()
    }

    /// Update feature toggles.
    pub async fn set_feature_toggles(&self, toggles: AiFeatureToggles) {
        let mut current = self.feature_toggles.write().await;
        *current = toggles;
    }

    // ── Internal Helpers ──

    /// Log an AI action to the audit trail.
    async fn log_ai_action(
        &self,
        action_type: &str,
        input_summary: &str,
        output_summary: &str,
        user_confirmed: bool,
        destructive: bool,
    ) {
        let entry = AiAuditEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            action_type: action_type.to_string(),
            input_summary: input_summary.to_string(),
            output_summary: output_summary.to_string(),
            user_confirmed,
            destructive,
            metadata: None,
        };
        let mut log = self.audit_log.write().await;
        log.push(entry);
    }

    /// Generate a chat response using local context analysis.
    async fn generate_chat_response(&self, input: &str) -> String {
        let input_lower = input.to_lowercase();

        // Help with errors
        if input_lower.contains("error") || input_lower.contains("fail") || input_lower.contains("problem") {
            return "I can help you with that! Here are a few things to try:\n\n\
                    1. Check the error details panel for the specific error message\n\
                    2. Right-click any failed item and select \"Explain this error\" for a detailed breakdown\n\
                    3. Review the transfer/sync logs for additional context\n\n\
                    Would you like me to analyze a specific error?".to_string();
        }

        // Help with sync
        if input_lower.contains("sync") || input_lower.contains("synchronize") {
            return "I can help you set up sync! Here are your options:\n\n\
                    1. **Quick sync**: Type something like \"Sync my Documents to Google Drive every night\"\n\
                    2. **Manual setup**: Go to the Sync panel and click \"New Sync Pair\"\n\
                    3. **Review existing**: Check the Sync panel for current sync pairs and their health status\n\n\
                    What would you like to do?".to_string();
        }

        // Help with transfers
        if input_lower.contains("transfer") || input_lower.contains("copy") || input_lower.contains("move") {
            return "I can help with file transfers! Here are some tips:\n\n\
                    1. Drag files between panes for quick copy/move\n\
                    2. Use the Transfer panel to monitor progress\n\
                    3. Set bandwidth limits in Transfer settings if needed\n\n\
                    Need help with a specific transfer operation?".to_string();
        }

        // Help with connections
        if input_lower.contains("connect") || input_lower.contains("sftp") || input_lower.contains("ssh") || input_lower.contains("ftp") {
            return "To connect to a remote server:\n\n\
                    1. Click the Connection panel in the sidebar\n\
                    2. Click \"New Connection\" and select the protocol\n\
                    3. Enter your server details and credentials\n\
                    4. Test the connection before saving\n\n\
                    Your saved connections appear in the sidebar for quick access.".to_string();
        }

        // General help
        if input_lower.contains("help") || input_lower.contains("what can") || input_lower.contains("how do") {
            return "I can help with:\n\n\
                    - **Error explanations**: Right-click failed items for detailed help\n\
                    - **Sync setup**: Describe your sync needs in plain language\n\
                    - **Transfer guidance**: Tips for efficient file transfers\n\
                    - **Connection help**: Setting up SFTP, FTP, WebDAV connections\n\
                    - **Cleanup suggestions**: Finding temp files and duplicates\n\
                    - **Sync rules**: Recommending exclusion patterns and schedules\n\n\
                    Just describe what you need!".to_string();
        }

        // Default response with guidance
        "I understand you need help. Could you be more specific? For example:\n\n\
         - \"Sync my Documents to Google Drive every night\"\n\
         - \"Why did my transfer fail?\"\n\
         - \"Help me set up an SFTP connection\"\n\
         - \"Clean up temporary files in my project folder\"\n\n\
         I can also generate suggestions based on your current workspace.".to_string()
    }

    /// Extract a file path from natural language input.
    fn extract_path(&self, input: &str, prepositions: &[&str]) -> Option<String> {
        let well_known = [
            ("documents", "~/Documents"),
            ("downloads", "~/Downloads"),
            ("desktop", "~/Desktop"),
            ("pictures", "~/Pictures"),
            ("music", "~/Music"),
            ("videos", "~/Videos"),
            ("home", "~"),
            ("projects", "~/Projects"),
        ];

        for (keyword, path) in &well_known {
            if input.contains(keyword) {
                // Check if preceded by a preposition indicating source
                for prep in prepositions {
                    if input.contains(&format!("{} {}", prep, keyword)) {
                        return Some(path.to_string());
                    }
                }
                // Or just mentioned
                return Some(path.to_string());
            }
        }

        // Try to find quoted paths
        if let Some(start) = input.find('"') {
            if let Some(end) = input[start + 1..].find('"') {
                return Some(input[start + 1..start + 1 + end].to_string());
            }
        }

        // Try to find absolute paths
        for word in input.split_whitespace() {
            if word.starts_with('/') || word.starts_with("~/") {
                return Some(word.to_string());
            }
        }

        None
    }

    /// Extract destination information from natural language input.
    fn extract_destination(&self, input: &str) -> (Option<String>, Option<String>) {
        let connectors = [
            ("google drive", "google_drive"),
            ("gdrive", "google_drive"),
            ("dropbox", "dropbox"),
            ("onedrive", "onedrive"),
            ("s3", "aws_s3"),
            ("aws", "aws_s3"),
            ("azure", "azure_blob"),
            ("backblaze", "backblaze_b2"),
            ("b2", "backblaze_b2"),
            ("nas", "smb"),
            ("network drive", "smb"),
        ];

        for (keyword, connector) in &connectors {
            if input.contains(keyword) {
                return (None, Some(connector.to_string()));
            }
        }

        // Check for path-based destination after "to"
        if let Some(to_pos) = input.find(" to ") {
            let after_to = &input[to_pos + 4..];
            if let Some(path) = self.extract_path(after_to, &["to"]) {
                return (Some(path), Some("local".to_string()));
            }
            // Check for SFTP/SSH
            if after_to.contains("sftp") || after_to.contains("ssh") {
                return (None, Some("sftp".to_string()));
            }
            if after_to.contains("ftp") {
                return (None, Some("ftp".to_string()));
            }
            if after_to.contains("webdav") {
                return (None, Some("webdav".to_string()));
            }
        }

        (None, None)
    }

    /// Extract schedule from natural language input.
    fn extract_schedule(&self, input: &str) -> (Option<String>, Option<String>) {
        let schedules = [
            ("every hour", "0 0 * * * * *", "every hour"),
            ("hourly", "0 0 * * * * *", "every hour"),
            ("every night", "0 0 2 * * * *", "every night at 2 AM"),
            ("nightly", "0 0 2 * * * *", "every night at 2 AM"),
            ("every day", "0 0 2 * * * *", "every day at 2 AM"),
            ("daily", "0 0 2 * * * *", "every day at 2 AM"),
            ("every week", "0 0 2 * * 0 *", "every Sunday at 2 AM"),
            ("weekly", "0 0 2 * * 0 *", "every Sunday at 2 AM"),
            ("every month", "0 0 2 1 * * *", "first of every month at 2 AM"),
            ("monthly", "0 0 2 1 * * *", "first of every month at 2 AM"),
            ("every 6 hours", "0 0 */6 * * * *", "every 6 hours"),
            ("every 15 minutes", "0 */15 * * * * *", "every 15 minutes"),
            ("every 30 minutes", "0 */30 * * * * *", "every 30 minutes"),
            ("in real time", "watch", "real-time (file watcher)"),
            ("real-time", "watch", "real-time (file watcher)"),
            ("immediately", "watch", "real-time (file watcher)"),
            ("on change", "watch", "real-time (file watcher)"),
        ];

        for (keyword, cron, human) in &schedules {
            if input.contains(keyword) {
                if *cron == "watch" {
                    return (Some("watch".to_string()), Some(human.to_string()));
                }
                return (Some(cron.to_string()), Some(human.to_string()));
            }
        }

        (None, None)
    }

    /// Extract filter/exclusion patterns from natural language.
    fn extract_patterns(&self, input: &str, include: bool) -> Vec<String> {
        let mut patterns = Vec::new();

        if include {
            if input.contains("only images") || input.contains("just images") || input.contains("just photos") {
                patterns.extend(["*.jpg", "*.jpeg", "*.png", "*.gif", "*.webp", "*.svg"].iter().map(|s| s.to_string()));
            }
            if input.contains("only documents") || input.contains("just documents") {
                patterns.extend(["*.pdf", "*.doc", "*.docx", "*.xls", "*.xlsx", "*.ppt", "*.pptx", "*.txt"].iter().map(|s| s.to_string()));
            }
            if input.contains("only videos") || input.contains("just videos") {
                patterns.extend(["*.mp4", "*.mkv", "*.avi", "*.mov", "*.wmv"].iter().map(|s| s.to_string()));
            }
        } else if input.contains("exclude") || input.contains("ignore") || input.contains("skip") {
            if input.contains("node_modules") {
                patterns.push("node_modules/".to_string());
            }
            if input.contains("git") {
                patterns.push(".git/".to_string());
            }
            if input.contains("temp") || input.contains("tmp") {
                patterns.extend(["*.tmp", "*.temp", "~*"].iter().map(|s| s.to_string()));
            }
            if input.contains("log") {
                patterns.push("*.log".to_string());
            }
        }

        patterns
    }

    /// Extract rename pattern from natural language.
    fn extract_rename_pattern(&self, input: &str) -> Option<String> {
        if input.contains("date prefix") || input.contains("add date") {
            Some("{date}_{name}".to_string())
        } else if input.contains("lowercase") {
            Some("{lower}".to_string())
        } else if input.contains("uppercase") {
            Some("{upper}".to_string())
        } else if input.contains("number") || input.contains("sequential") {
            Some("{name}_{seq}".to_string())
        } else {
            None
        }
    }
}

/// Context information for generating suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct SuggestionContext {
    pub current_path: Option<String>,
    pub total_files: usize,
    pub temp_file_count: usize,
    pub duplicate_candidates: usize,
    pub unclassified_folders: usize,
    pub recent_errors: Vec<String>,
    pub active_sync_pairs: usize,
}


impl Default for AiAssistant {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explain_file_error() {
        let ai = AiAssistant::new();
        let err = AppError::file_op("Not found", "Check path");
        let explanation = ai.explain_error(&err);
        assert!(explanation.plain_language.contains("Not found"));
        assert!(!explanation.possible_causes.is_empty());
        assert!(!explanation.suggested_fixes.is_empty());
    }

    #[test]
    fn test_explain_internal_error() {
        let ai = AiAssistant::new();
        let err = AppError::Internal {
            message: "secret stuff".to_string(),
        };
        let explanation = ai.explain_error(&err);
        assert!(!explanation.plain_language.contains("secret"));
        assert!(explanation.plain_language.contains("unexpected"));
    }

    #[test]
    fn test_explain_error_message() {
        let ai = AiAssistant::new();
        let explanation = ai.explain_error_message("connection", "Timeout", "Try again");
        assert!(explanation.plain_language.contains("connect"));
    }

    #[tokio::test]
    async fn test_parse_sync_command() {
        let ai = AiAssistant::new();
        let result = ai
            .parse_natural_language("Sync my Documents to Google Drive every night")
            .await
            .unwrap();
        assert_eq!(result.intent, "sync");
        assert!(result.source_path.is_some());
        assert_eq!(result.dest_connector, Some("google_drive".to_string()));
        assert!(result.schedule.is_some());
        assert!(result.confidence > 0.5);
    }

    #[tokio::test]
    async fn test_parse_backup_command() {
        let ai = AiAssistant::new();
        let result = ai
            .parse_natural_language("Backup my Downloads to Dropbox weekly")
            .await
            .unwrap();
        assert_eq!(result.intent, "backup");
        assert!(result.source_path.is_some());
        assert_eq!(result.dest_connector, Some("dropbox".to_string()));
    }

    #[tokio::test]
    async fn test_parse_ambiguous_command() {
        let ai = AiAssistant::new();
        let result = ai
            .parse_natural_language("sync my files")
            .await
            .unwrap();
        assert!(result.needs_clarification);
        assert!(!result.clarification_questions.is_empty());
    }

    #[tokio::test]
    async fn test_chat_response() {
        let ai = AiAssistant::new();
        let response = ai.chat("How do I sync my files?").await.unwrap();
        assert_eq!(response.role, "assistant");
        assert!(!response.content.is_empty());
    }

    #[tokio::test]
    async fn test_suggestions_generation() {
        let ai = AiAssistant::new();
        let context = SuggestionContext {
            current_path: Some("/home/user/Documents".to_string()),
            total_files: 15000,
            temp_file_count: 100,
            duplicate_candidates: 5,
            unclassified_folders: 10,
            recent_errors: vec![],
            active_sync_pairs: 0,
        };
        let suggestions = ai.generate_suggestions(&context).await;
        assert!(suggestions.len() >= 3);
    }

    #[tokio::test]
    async fn test_dismiss_suggestion() {
        let ai = AiAssistant::new();
        let context = SuggestionContext {
            current_path: Some("/home/user/Documents".to_string()),
            ..Default::default()
        };
        let suggestions = ai.generate_suggestions(&context).await;
        assert!(!suggestions.is_empty());
        let id = suggestions[0].id.clone();
        ai.dismiss_suggestion(&id).await.unwrap();
        let active = ai.get_suggestions().await;
        assert!(active.iter().all(|s| s.id != id));
    }

    #[tokio::test]
    async fn test_feature_toggles_disable() {
        let ai = AiAssistant::new();
        let mut toggles = ai.get_feature_toggles().await;
        toggles.ai_master_enabled = false;
        ai.set_feature_toggles(toggles).await;

        let result = ai.parse_natural_language("sync my files").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_audit_logging() {
        let ai = AiAssistant::new();
        ai.chat("test message").await.unwrap();
        // Chat doesn't directly log, but NL parse does
        let _ = ai.parse_natural_language("sync my documents").await;
        let log = ai.get_audit_log(10, 0).await;
        assert!(!log.is_empty());
    }

    #[tokio::test]
    async fn test_content_analysis_off_by_default() {
        let ai = AiAssistant::new();
        let toggles = ai.get_feature_toggles().await;
        assert!(!toggles.content_analysis_opt_in);
    }
}
