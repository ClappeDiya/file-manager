pub mod ai_engine;
pub mod automation_engine;
pub mod commands;
pub mod compat_engine;
pub mod connectors;
pub mod core;
pub mod fs_engine;
pub mod governance;
pub mod ledger;
pub mod lineage;
pub mod mount_engine;
pub mod narrator;
pub mod safety;
pub mod security;
pub mod storage;
pub mod sync_engine;
pub mod spaces_engine;
pub mod transfer_engine;

use ai_engine::AiAssistant;
use automation_engine::AutomationManager;
use spaces_engine::SpacesManager;
use commands::{ai_commands, archive_commands, automation_commands, aws_commands, batch_rename_commands, confirmation_commands, connection_commands, connector_commands, custom_commands, drive_commands, editor_commands, encryption_commands, file_ops_commands, fs_commands, integrity_commands, ledger_commands, lineage_commands, log_commands, master_password_commands, mount_commands, narrator_commands, network_wizard_commands, notification_commands, peer_commands, preview_commands, s3_commands, safety_commands, settings_commands, space_commands, ssh_key_commands, state_commands, sync_commands, system_commands, terminal_commands, transfer_commands, undo_commands, url_handler_commands, version_commands};
use ledger::OperationLedger;
use narrator::OperationNarrator;
use safety::SafetyInterlock;
use commands::terminal_commands::TerminalManager;
use connectors::{ConnectionManager, ConnectorRegistry, GoogleDriveConnector, OneDriveConnector, PeerManager, S3Connector, ServerTransferManager};
use mount_engine::MountManager;
use core::traits::StorageOperations;
use security::confirmation::ConfirmationManager;
use security::CredentialStore;
use security::MasterPasswordManager;
use security::vault::VaultManager;
use storage::Repository;
use sync_engine::SyncManager;
use transfer_engine::history::TransferHistory;
use transfer_engine::TransferManager;

/// Application state initialized at startup.
struct AppState {
    repo: Repository,
    transfer_mgr: TransferManager,
    connection_mgr: ConnectionManager,
    transfer_history: TransferHistory,
    connector_registry: ConnectorRegistry,
    sync_mgr: SyncManager,
    peer_mgr: PeerManager,
    server_transfer_mgr: ServerTransferManager,
    ai_assistant: AiAssistant,
    terminal_mgr: TerminalManager,
    vault_mgr: VaultManager,
    master_pw_mgr: MasterPasswordManager,
    mount_mgr: MountManager,
    s3_connector: std::sync::Arc<S3Connector>,
    gdrive_connector: std::sync::Arc<GoogleDriveConnector>,
    onedrive_connector: std::sync::Arc<OneDriveConnector>,
    editor_state: std::sync::Arc<commands::editor_commands::EditorState>,
    automation_mgr: AutomationManager,
    spaces_mgr: SpacesManager,
    confirmation_mgr: ConfirmationManager,
    ledger: OperationLedger,
    safety_interlock: SafetyInterlock,
    narrator: OperationNarrator,
}

/// Initialize the database, transfer manager, and connection manager, restoring persisted state.
async fn initialize_app_state() -> Result<AppState, crate::core::error::AppError> {
    // Open database (creates if needed, runs migrations)
    let repo = Repository::open_default().await?;

    // Restore transfer queue from database
    let transfer_mgr = TransferManager::new();

    // Initialize the three-layer transfer journal
    transfer_mgr.init_journal(repo.pool().clone()).await;

    // Recover any active transfers from the journal (crash recovery)
    match transfer_mgr.recover_from_journal().await {
        Ok(count) => {
            if count > 0 {
                tracing::info!("Found {} transfer(s) in journal for recovery", count);
            }
        }
        Err(e) => {
            tracing::warn!("Journal recovery failed (non-fatal): {e}");
        }
    }

    let queued_jobs = StorageOperations::load_transfer_queue(&repo).await?;
    if !queued_jobs.is_empty() {
        tracing::info!(
            "Restoring {} interrupted transfer(s) from previous session",
            queued_jobs.len()
        );
        transfer_mgr.restore_jobs(queued_jobs).await;
    }

    // Initialize credential store and connection manager
    let credential_store = std::sync::Arc::new(CredentialStore::new());
    let connection_mgr = ConnectionManager::new(repo.clone(), credential_store);

    // Initialize transfer history
    let transfer_history = TransferHistory::new(repo.pool().clone());

    // Initialize connector registry (SFTP, FTP, WebDAV, SMB, NFS, Local)
    let connector_registry = ConnectorRegistry::new();

    // Create shared connector instances for advanced cloud commands
    let s3_connector = std::sync::Arc::new(S3Connector::new());
    let gdrive_connector = std::sync::Arc::new(GoogleDriveConnector::new());
    let onedrive_connector = std::sync::Arc::new(OneDriveConnector::new());

    // Initialize sync manager (T-039..T-042)
    let sync_mgr = SyncManager::new();

    // Initialize peer manager (T-031) and server transfer manager (T-033)
    let peer_mgr = PeerManager::new();
    let server_transfer_mgr = ServerTransferManager::new();

    // Initialize AI assistant (T-043..T-045)
    let ai_assistant = AiAssistant::new();

    // Initialize terminal manager (T-046)
    let terminal_mgr = TerminalManager::new();

    // Initialize vault manager for encrypted vaults (T-047)
    let vault_mgr = VaultManager::new();

    // Initialize master password manager
    let master_pw_mgr = MasterPasswordManager::new();

    // Initialize mount manager (Finder-mounted remote storage)
    let mount_mgr = MountManager::new();

    // Initialize editor state (external editor mappings and file watch)
    let editor_state = std::sync::Arc::new(commands::editor_commands::EditorState::new());

    // Operation Ledger — unified append-only event store across all engines.
    // Created BEFORE the automation manager so we can inject it and capture
    // autonomous rule fires in the cross-engine timeline.
    let ledger = OperationLedger::new(repo.pool().clone());
    // Best-effort startup prune (30-day retention). Non-fatal.
    if let Err(e) = ledger.prune(30).await {
        tracing::warn!("Operation ledger prune failed (non-fatal): {e}");
    }

    // Initialize automation manager (Quickflows)
    let mut automation_mgr = AutomationManager::new();
    automation_mgr.set_ledger(ledger.clone());
    if let Err(e) = automation_mgr.load_rules_from_db(&repo).await {
        tracing::warn!("Failed to load automation rules (non-fatal): {e}");
    }
    if let Err(e) = automation_mgr.start_all_watchers().await {
        tracing::warn!("Failed to start automation watchers (non-fatal): {e}");
    }

    // Initialize Smart Spaces manager
    let spaces_mgr = SpacesManager::new();
    if let Err(e) = spaces_mgr.load_from_db(&repo).await {
        tracing::warn!("Failed to load smart spaces (non-fatal): {e}");
    }

    let confirmation_mgr = ConfirmationManager::new();

    // Safety Interlock — context-aware anomaly detection built on top of the
    // existing operation ledger. Adds no new infrastructure; reads the
    // ledger to baseline user behavior and only surfaces a confirmation
    // dialog when an operation is far outside the user's normal pattern.
    let safety_interlock = SafetyInterlock::new(ledger.clone());

    // Operation Narrator — one-click plain-language story for any
    // correlation-id trace in the ledger. Pure composition over the
    // existing ledger query primitive; zero new infrastructure.
    let narrator = OperationNarrator::new(ledger.clone());

    Ok(AppState {
        repo,
        transfer_mgr,
        connection_mgr,
        transfer_history,
        connector_registry,
        sync_mgr,
        peer_mgr,
        server_transfer_mgr,
        ai_assistant,
        terminal_mgr,
        vault_mgr,
        master_pw_mgr,
        mount_mgr,
        s3_connector,
        gdrive_connector,
        onedrive_connector,
        editor_state,
        automation_mgr,
        spaces_mgr,
        confirmation_mgr,
        ledger,
        safety_interlock,
        narrator,
    })
}

/// Persist transfer queue and workspace state on shutdown.
async fn persist_on_shutdown(repo: &Repository, transfer_mgr: &TransferManager) {
    // Save transfer queue
    let jobs = transfer_mgr.snapshot_jobs().await;
    if let Err(e) = StorageOperations::save_transfer_queue(repo, &jobs).await {
        tracing::error!("Failed to persist transfer queue on shutdown: {e}");
    } else {
        tracing::info!("Transfer queue persisted ({} job(s))", jobs.len());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Unified File Operations Platform");

    // Initialize app state synchronously using a one-shot tokio runtime.
    // This runs before Tauri starts its own runtime.
    let app_state = {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        match rt.block_on(initialize_app_state()) {
            Ok(state) => state,
            Err(e) => {
                tracing::error!("Failed to initialize app state: {e}");
                tracing::warn!("Starting with in-memory database as fallback");
                let rt2 = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                let repo = rt2
                    .block_on(Repository::open_in_memory())
                    .expect("Failed to create in-memory database");
                let credential_store = std::sync::Arc::new(CredentialStore::new());
                let connection_mgr = ConnectionManager::new(repo.clone(), credential_store);
                let transfer_history = TransferHistory::new(repo.pool().clone());
                let ledger = OperationLedger::new(repo.pool().clone());
                let safety_interlock = SafetyInterlock::new(ledger.clone());
                let narrator = OperationNarrator::new(ledger.clone());
                let mut automation_mgr = AutomationManager::new();
                automation_mgr.set_ledger(ledger.clone());
                AppState {
                    repo,
                    transfer_mgr: TransferManager::new(),
                    connection_mgr,
                    transfer_history,
                    connector_registry: ConnectorRegistry::new(),
                    sync_mgr: SyncManager::new(),
                    peer_mgr: PeerManager::new(),
                    server_transfer_mgr: ServerTransferManager::new(),
                    ai_assistant: AiAssistant::new(),
                    terminal_mgr: TerminalManager::new(),
                    vault_mgr: VaultManager::new(),
                    master_pw_mgr: MasterPasswordManager::new(),
                    mount_mgr: MountManager::new(),
                    s3_connector: std::sync::Arc::new(S3Connector::new()),
                    gdrive_connector: std::sync::Arc::new(GoogleDriveConnector::new()),
                    onedrive_connector: std::sync::Arc::new(OneDriveConnector::new()),
                    editor_state: std::sync::Arc::new(commands::editor_commands::EditorState::new()),
                    automation_mgr,
                    spaces_mgr: SpacesManager::new(),
                    confirmation_mgr: ConfirmationManager::new(),
                    ledger,
                    safety_interlock,
                    narrator,
                }
            }
        }
    };

    // Clone for shutdown hook
    let shutdown_repo = app_state.repo.clone();
    let shutdown_transfer = app_state.transfer_mgr.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state.repo)
        .manage(app_state.transfer_mgr)
        .manage(app_state.connection_mgr)
        .manage(app_state.transfer_history)
        .manage(app_state.connector_registry)
        .manage(app_state.sync_mgr)
        .manage(app_state.peer_mgr)
        .manage(app_state.server_transfer_mgr)
        .manage(app_state.ai_assistant)
        .manage(app_state.terminal_mgr)
        .manage(app_state.vault_mgr)
        .manage(app_state.master_pw_mgr)
        .manage(app_state.mount_mgr)
        .manage(app_state.s3_connector)
        .manage(app_state.gdrive_connector)
        .manage(app_state.onedrive_connector)
        .manage(app_state.editor_state)
        .manage(app_state.automation_mgr)
        .manage(app_state.spaces_mgr)
        .manage(app_state.confirmation_mgr)
        .manage(app_state.ledger)
        .manage(app_state.safety_interlock)
        .manage(app_state.narrator)
        .invoke_handler(tauri::generate_handler![
            // System
            system_commands::greet,
            system_commands::get_app_version,
            system_commands::get_platform_info,
            // Filesystem
            fs_commands::list_directory,
            fs_commands::pick_folder,
            // State management
            state_commands::save_workspace_state,
            state_commands::load_workspace_state,
            state_commands::get_config,
            state_commands::set_config,
            state_commands::reset_database,
            // Confirmation tokens for destructive operations
            confirmation_commands::request_destructive_confirmation,
            // Transfer management (T-015)
            transfer_commands::enqueue_transfer,
            transfer_commands::pause_transfer,
            transfer_commands::resume_transfer,
            transfer_commands::cancel_transfer,
            transfer_commands::list_transfers,
            transfer_commands::get_transfer,
            transfer_commands::set_transfer_priority,
            transfer_commands::reorder_transfer,
            transfer_commands::set_global_throttle,
            transfer_commands::set_connection_throttle,
            transfer_commands::get_throttle_config,
            transfer_commands::update_transfer_progress,
            transfer_commands::complete_transfer,
            transfer_commands::fail_transfer,
            // Retry logic (T-016)
            transfer_commands::retry_transfer,
            transfer_commands::retry_all_failed,
            transfer_commands::list_failed_transfers,
            transfer_commands::set_retry_policy,
            // Conflict resolution (T-017)
            transfer_commands::check_conflict,
            transfer_commands::resolve_conflict,
            transfer_commands::set_conflict_policy,
            transfer_commands::set_transfer_conflict_policy,
            // Verification and history (T-018)
            transfer_commands::verify_transfer,
            transfer_commands::compute_checksum,
            transfer_commands::set_verify_checksums,
            transfer_commands::set_checksum_algorithm,
            transfer_commands::search_transfer_history,
            transfer_commands::export_transfer_history,
            transfer_commands::cleanup_transfer_history,
            // Three-layer architecture
            transfer_commands::set_verify_tier,
            transfer_commands::get_verify_tier,
            transfer_commands::get_transfer_config,
            transfer_commands::set_transfer_config,
            // Transfer Completion Hooks
            transfer_commands::get_completion_hooks,
            transfer_commands::set_completion_hooks,
            // Connection management (T-019)
            connection_commands::save_connection,
            connection_commands::list_connections,
            connection_commands::get_connection,
            connection_commands::delete_connection,
            connection_commands::test_connection,
            connection_commands::search_connections,
            connection_commands::create_connection_group,
            connection_commands::list_connection_groups,
            connection_commands::delete_connection_group,
            connection_commands::export_connections,
            connection_commands::import_connections,
            connection_commands::set_global_proxy,
            connection_commands::get_global_proxy,
            connection_commands::resolve_ssh_config,
            connection_commands::list_ssh_config_hosts,
            connection_commands::list_ssh_agents,
            connection_commands::import_from_third_party,
            // Protocol connectors (T-020, T-021, T-022)
            connector_commands::connector_connect,
            connector_commands::connector_disconnect,
            connector_commands::connector_list_remote,
            connector_commands::connector_is_connected,
            connector_commands::connector_list_protocols,
            // SMB/CIFS (T-023)
            drive_commands::smb_discover_shares,
            drive_commands::smb_list_shares,
            // NFS (T-024)
            drive_commands::nfs_list_exports,
            // Drive-to-Drive (T-025)
            drive_commands::detect_drives,
            drive_commands::transfer_preflight,
            drive_commands::eject_drive,
            // Sync engine (T-039..T-042)
            sync_commands::create_sync_pair,
            sync_commands::delete_sync_pair,
            sync_commands::list_sync_pairs,
            sync_commands::update_sync_pair,
            sync_commands::run_sync,
            sync_commands::get_sync_health,
            sync_commands::sync_dry_run,
            sync_commands::export_sync_preview_csv,
            sync_commands::get_sync_conflicts,
            sync_commands::resolve_sync_conflict,
            sync_commands::get_quarantine_entries,
            sync_commands::get_sync_reports,
            sync_commands::rollback_sync,
            sync_commands::export_sync_report_csv,
            sync_commands::export_sync_report_json,
            sync_commands::start_sync_watcher,
            sync_commands::stop_sync_watcher,
            sync_commands::validate_sync_cron,
            // Sync time offset detection
            sync_commands::detect_time_offset,
            // Sync Filter Presets
            sync_commands::get_sync_filter_presets,
            // Peer discovery and transfer (T-031)
            peer_commands::peer_start_discovery,
            peer_commands::peer_stop_discovery,
            peer_commands::peer_is_discovering,
            peer_commands::peer_list_peers,
            peer_commands::peer_list_online,
            peer_commands::peer_get_peer,
            peer_commands::peer_set_trust,
            peer_commands::peer_save_peer,
            peer_commands::peer_remove_saved,
            peer_commands::peer_list_saved,
            peer_commands::peer_set_display_name,
            peer_commands::peer_connect_manual,
            peer_commands::peer_request_transfer,
            peer_commands::peer_list_pending_requests,
            peer_commands::peer_respond_transfer,
            peer_commands::peer_get_transfer_progress,
            peer_commands::peer_list_transfers,
            peer_commands::peer_cancel_transfer,
            // Server-to-server transfer (T-033)
            peer_commands::server_transfer_preview_method,
            peer_commands::server_transfer_create,
            peer_commands::server_transfer_start,
            peer_commands::server_transfer_pause,
            peer_commands::server_transfer_cancel,
            peer_commands::server_transfer_retry,
            peer_commands::server_transfer_get,
            peer_commands::server_transfer_list,
            peer_commands::server_transfer_list_active,
            peer_commands::server_transfer_capability_matrix,
            peer_commands::server_transfer_cleanup,
            // AI features (T-043..T-045)
            ai_commands::ai_explain_error,
            ai_commands::ai_chat,
            ai_commands::ai_get_chat_history,
            ai_commands::ai_clear_chat,
            ai_commands::ai_generate_suggestions,
            ai_commands::ai_get_suggestions,
            ai_commands::ai_dismiss_suggestion,
            ai_commands::ai_accept_suggestion,
            ai_commands::ai_parse_natural_language,
            ai_commands::ai_confirm_action,
            ai_commands::ai_check_confirmation_needed,
            ai_commands::ai_get_audit_log,
            ai_commands::ai_get_feature_toggles,
            ai_commands::ai_set_feature_toggles,
            ai_commands::ai_probe_ollama,
            ai_commands::ai_execute_parsed_job,
            ai_commands::ai_get_ollama_model,
            ai_commands::ai_set_ollama_model,
            // Terminal (T-046)
            terminal_commands::terminal_create_local,
            terminal_commands::terminal_create_remote,
            terminal_commands::terminal_list_sessions,
            terminal_commands::terminal_close_session,
            terminal_commands::terminal_write,
            terminal_commands::terminal_resize,
            terminal_commands::terminal_set_layout,
            terminal_commands::terminal_get_layout,
            terminal_commands::terminal_escape_path,
            terminal_commands::terminal_get_default_shell,
            // Encryption & Vault management (T-047)
            encryption_commands::create_vault,
            encryption_commands::unlock_vault,
            encryption_commands::lock_vault,
            encryption_commands::list_vaults,
            encryption_commands::is_vault_unlocked,
            encryption_commands::get_unlocked_vault_count,
            encryption_commands::vault_encrypt_file,
            encryption_commands::vault_decrypt_file,
            encryption_commands::encrypt_for_upload,
            encryption_commands::decrypt_from_download,
            encryption_commands::change_vault_password,
            encryption_commands::check_transport_security,
            encryption_commands::check_transport_security_custom,
            encryption_commands::get_recommended_security,
            encryption_commands::enforce_https_url,
            encryption_commands::add_encryption_policy,
            encryption_commands::list_encryption_policies,
            encryption_commands::remove_encryption_policy,
            encryption_commands::check_encryption_policy,
            // Master Password (session-based credential encryption)
            master_password_commands::is_master_password_set,
            master_password_commands::set_master_password,
            master_password_commands::verify_master_password,
            master_password_commands::change_master_password,
            master_password_commands::master_lock_app,
            master_password_commands::master_unlock_app,
            master_password_commands::is_app_unlocked,
            master_password_commands::master_encrypt_credential,
            master_password_commands::master_decrypt_credential,
            // File operations
            file_ops_commands::copy_files,
            file_ops_commands::move_files,
            file_ops_commands::rename_file,
            file_ops_commands::duplicate_files,
            file_ops_commands::delete_files,
            file_ops_commands::create_directory,
            file_ops_commands::create_file,
            file_ops_commands::undo_file_operation,
            file_ops_commands::get_file_metadata,
            file_ops_commands::cloud_delete_to_trash,
            file_ops_commands::cloud_delete_permanently,
            // Copy Path / Copy URL (Transmit parity)
            file_ops_commands::copy_path,
            file_ops_commands::copy_remote_path,
            file_ops_commands::copy_url,
            file_ops_commands::copy_relative_path,
            // Batch Rename (T-060)
            batch_rename_commands::batch_rename_preview,
            batch_rename_commands::batch_rename_apply,
            batch_rename_commands::batch_rename_undo,
            // Preview Engine (T-061)
            preview_commands::preview_file,
            preview_commands::preview_get_exif,
            // Archive Tools (T-062)
            archive_commands::archive_browse,
            archive_commands::archive_create,
            archive_commands::archive_extract,
            archive_commands::archive_info,
            // Integrity Tools (T-063)
            integrity_commands::integrity_checksum,
            integrity_commands::integrity_verify,
            integrity_commands::integrity_find_duplicates,
            integrity_commands::integrity_resolve_duplicates,
            integrity_commands::integrity_create_tag,
            integrity_commands::integrity_list_tags,
            integrity_commands::integrity_delete_tag,
            integrity_commands::integrity_tag_file,
            integrity_commands::integrity_untag_file,
            integrity_commands::integrity_get_file_info,
            integrity_commands::integrity_set_label,
            integrity_commands::integrity_remove_label,
            integrity_commands::integrity_create_smart_folder,
            integrity_commands::integrity_list_smart_folders,
            integrity_commands::integrity_delete_smart_folder,
            integrity_commands::integrity_run_smart_folder,
            // Mount Engine (Finder-mounted remote storage)
            mount_commands::check_mount_capability,
            mount_commands::mount_remote,
            mount_commands::unmount_remote,
            mount_commands::list_mounts,
            mount_commands::get_mount_status,
            mount_commands::save_mount_config,
            mount_commands::delete_mount_config,
            // Settings Export/Import & Sync
            settings_commands::export_all_settings,
            settings_commands::import_all_settings,
            settings_commands::get_settings_summary,
            settings_commands::set_settings_sync_dir,
            settings_commands::get_settings_sync_dir,
            settings_commands::sync_settings_to_dir,
            settings_commands::sync_settings_from_dir,
            settings_commands::get_settings_sync_status,
            settings_commands::set_settings_auto_sync,
            // Privacy Settings
            settings_commands::get_privacy_settings,
            settings_commands::set_privacy_settings,
            // Network Diagnostics Wizard
            network_wizard_commands::network_test_dns,
            network_wizard_commands::network_test_port,
            network_wizard_commands::network_test_tls,
            network_wizard_commands::network_test_auth,
            network_wizard_commands::network_run_full_diagnostic,
            // S3 Advanced Object Controls
            s3_commands::s3_get_object_properties,
            s3_commands::s3_set_object_acl,
            s3_commands::s3_set_storage_class,
            s3_commands::s3_get_bucket_settings,
            s3_commands::s3_set_transfer_acceleration,
            // S3 Lifecycle Rules
            s3_commands::s3_get_lifecycle_rules,
            s3_commands::s3_put_lifecycle_rule,
            s3_commands::s3_delete_lifecycle_rule,
            // CloudFront Distribution Management
            s3_commands::s3_list_distributions,
            s3_commands::s3_create_invalidation,
            s3_commands::s3_list_invalidations,
            // S3 MFA Delete
            s3_commands::s3_get_mfa_delete_status,
            s3_commands::s3_delete_with_mfa,
            // OS Notifications
            notification_commands::get_notification_prefs,
            notification_commands::set_notification_prefs,
            notification_commands::send_notification,
            // URL Handler Registration
            url_handler_commands::register_url_handlers,
            url_handler_commands::unregister_url_handlers,
            url_handler_commands::is_url_handler_registered,
            url_handler_commands::handle_incoming_url,
            // AWS Credential Chain, SSO, and STS Federation
            aws_commands::list_aws_profiles,
            aws_commands::resolve_aws_profile,
            aws_commands::start_sso_auth,
            aws_commands::poll_sso_token,
            aws_commands::get_sso_role_credentials,
            aws_commands::assume_role_with_web_identity,
            // File Version Browser
            version_commands::list_file_versions,
            version_commands::restore_file_version,
            // Remote permissions, ownership, and symlinks (WinSCP parity)
            file_ops_commands::set_permissions,
            file_ops_commands::set_owner,
            file_ops_commands::create_symlink,
            file_ops_commands::read_symlink,
            file_ops_commands::resolve_path,
            file_ops_commands::open_file_with_default,
            // S3 Object Tags
            s3_commands::s3_get_object_tags,
            s3_commands::s3_put_object_tags,
            s3_commands::s3_delete_object_tags,
            // S3 Endpoint Presets & GCS
            s3_commands::s3_list_endpoint_presets,
            s3_commands::s3_validate_endpoint,
            // Custom Commands Framework
            custom_commands::list_custom_commands,
            custom_commands::save_custom_command,
            custom_commands::delete_custom_command,
            custom_commands::expand_custom_command,
            custom_commands::execute_custom_command,
            custom_commands::generate_script,
            // External Editor Commands
            editor_commands::list_editor_mappings,
            editor_commands::save_editor_mapping,
            editor_commands::delete_editor_mapping,
            editor_commands::get_double_click_behavior,
            editor_commands::set_double_click_behavior,
            editor_commands::resolve_editor,
            editor_commands::open_in_editor,
            editor_commands::start_file_watch,
            editor_commands::stop_file_watch,
            editor_commands::check_watched_files,
            editor_commands::list_watched_files,
            // Log Controls & Structured Export
            log_commands::get_log_level,
            log_commands::set_log_level,
            log_commands::get_log_directory,
            log_commands::export_logs_json,
            log_commands::export_logs_to_file,
            // Trusted SSH Host CAs
            log_commands::list_trusted_host_cas,
            log_commands::add_trusted_host_ca,
            log_commands::remove_trusted_host_ca,
            // GSSAPI/Kerberos Status
            log_commands::check_gssapi_available,
            // SSH Key Generation
            ssh_key_commands::generate_ssh_key,
            ssh_key_commands::list_ssh_keys,
            // Automation engine (Quickflows)
            automation_commands::create_automation_rule,
            automation_commands::update_automation_rule,
            automation_commands::delete_automation_rule,
            automation_commands::list_automation_rules,
            automation_commands::get_automation_rule,
            automation_commands::enable_automation_rule,
            automation_commands::run_automation_rule,
            automation_commands::test_automation_rule,
            automation_commands::list_automation_logs,
            automation_commands::clear_automation_logs,
            automation_commands::parse_automation_nl,
            // Smart Spaces
            space_commands::create_space,
            space_commands::list_spaces,
            space_commands::get_space,
            space_commands::update_space,
            space_commands::delete_space,
            space_commands::get_space_status,
            space_commands::activate_space,
            space_commands::attach_space_automation,
            space_commands::detach_space_automation,
            // Operation Ledger (unified cross-engine event store)
            ledger_commands::ledger_recent,
            ledger_commands::ledger_query,
            ledger_commands::ledger_count,
            ledger_commands::ledger_prune,
            ledger_commands::ledger_since_last_seen,
            ledger_commands::ledger_recent_paths,
            ledger_commands::ledger_directory_activity,
            ledger_commands::ledger_get_pulse,
            // Universal Time-Travel Undo (cross-session Cmd+Z backed by the ledger)
            undo_commands::undo_last,
            undo_commands::undo_by_correlation,
            undo_commands::list_undoable,
            // File Lineage / Provenance Graph (reads the ledger only; zero writes)
            lineage_commands::get_file_lineage,
            // Safety Interlock (context-aware anomaly detection over the ledger)
            safety_commands::safety_assess_intent,
            safety_commands::safety_confirm_intent,
            // Operation Narrator (plain-language cross-engine story per correlation trace)
            narrator_commands::narrator_narrate_operation,
        ])
        .on_window_event(move |_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                tracing::info!("Window closing, persisting state...");
                let repo = shutdown_repo.clone();
                let mgr = shutdown_transfer.clone();
                // Use spawn_blocking since we're in the event handler
                let rt = tokio::runtime::Handle::try_current();
                if let Ok(handle) = rt {
                    handle.block_on(persist_on_shutdown(&repo, &mgr));
                } else {
                    // If no runtime, create one
                    let rt = tokio::runtime::Runtime::new().ok();
                    if let Some(rt) = rt {
                        rt.block_on(persist_on_shutdown(&repo, &mgr));
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_initialize_with_in_memory() {
        // Verify in-memory fallback works
        let repo = Repository::open_in_memory().await.unwrap();
        let mgr = TransferManager::new();

        // Verify they work together
        let state = StorageOperations::load_workspace_state(&repo).await.unwrap();
        assert_eq!(state.version, 1);

        let jobs = mgr.snapshot_jobs().await;
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn test_persist_on_shutdown() {
        let repo = Repository::open_in_memory().await.unwrap();
        let mgr = TransferManager::new();

        // Enqueue a job
        use crate::core::traits::TransferOperations;
        mgr.enqueue("/src", "/dst", 1000).await.unwrap();

        // Persist
        persist_on_shutdown(&repo, &mgr).await;

        // Verify persisted
        let loaded = StorageOperations::load_transfer_queue(&repo).await.unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[tokio::test]
    async fn test_transfer_queue_crash_safe_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("crash_test.db");

        // Session 1: enqueue a transfer and "crash" (just drop without saving)
        {
            let pool = crate::storage::DbPool::open(&db_path).unwrap();
            let repo = Repository::new(pool).await.unwrap();
            let mgr = TransferManager::new();

            use crate::core::traits::TransferOperations;
            let job = mgr.enqueue("/important/file.zip", "/backup/file.zip", 50000).await.unwrap();

            // Simulate partial progress
            {
                let mut jobs = mgr.snapshot_jobs().await;
                jobs[0].transferred_bytes = 25000;
                jobs[0].status = crate::core::types::TransferStatus::Active;
                StorageOperations::save_transfer_queue(&repo, &jobs).await.unwrap();
            }

            // Simulate crash - just let repo drop
            let _ = job;
        }

        // Session 2: reopen and verify recovery
        {
            let pool = crate::storage::DbPool::open(&db_path).unwrap();
            let repo = Repository::new(pool).await.unwrap();

            let loaded = StorageOperations::load_transfer_queue(&repo).await.unwrap();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].source_path, "/important/file.zip");
            assert_eq!(loaded[0].transferred_bytes, 25000);
            // Active should be re-queued on recovery
            assert_eq!(loaded[0].status, crate::core::types::TransferStatus::Queued);
        }
    }

    #[tokio::test]
    async fn test_workspace_state_restore_under_500ms() {
        let repo = Repository::open_in_memory().await.unwrap();

        // Save a complex workspace state
        let state = crate::core::types::WorkspaceState::default();
        StorageOperations::save_workspace_state(&repo, &state).await.unwrap();

        // Time the restore
        let start = std::time::Instant::now();
        let _loaded = StorageOperations::load_workspace_state(&repo).await.unwrap();
        let elapsed = start.elapsed();

        // Must restore within 500ms
        assert!(
            elapsed.as_millis() < 500,
            "Workspace restore took {}ms, expected < 500ms",
            elapsed.as_millis()
        );
    }
}
