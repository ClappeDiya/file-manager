pub mod ai_engine;
pub mod automation_engine;
pub mod backup_engine;
pub mod chat_control;
pub mod commands;
pub mod paths;
pub mod compat_engine;
pub mod convert_engine;
pub mod connectors;
pub mod core;
pub mod fleet_agent;
pub mod fs_engine;
pub mod governance;
pub mod ledger;
pub mod lineage;
pub mod metadata_engine;
pub mod embedded_server;
pub mod feedback;
pub mod log_query;
pub mod logging;
pub mod diagnostics;
pub mod managed_settings;
pub mod net;
pub mod notify_hub;
pub mod search_engine;
pub mod thumbnail_engine;
pub mod mount_engine;
pub mod rclone_engine;
pub mod narrator;
pub mod safety;
pub mod security;
pub mod server;
pub mod storage;
pub mod schedule_library;
pub mod sync_engine;
pub mod spaces_engine;
pub mod transfer_engine;
pub mod tray;

use ai_engine::AiAssistant;
use automation_engine::suggester::LedgerSuggester;
use automation_engine::AutomationManager;
use backup_engine::BackupManager;
use spaces_engine::SpacesManager;
use commands::{ai_commands, archive_commands, audit_commands, automation_commands, backup_commands, aws_commands, chat_control_commands, batch_rename_commands, confirmation_commands, connection_commands, connector_commands, custom_commands, oauth_commands, drive_commands, editor_commands, embedded_server_commands, encryption_commands, file_ops_commands, feedback_commands, fleet_agent_commands, fs_commands, integrity_commands, ledger_commands, lineage_commands, log_commands, master_password_commands, mount_commands, narrator_commands, network_wizard_commands, notification_commands, peer_commands, preview_commands, s3_commands, safety_commands, schedule_commands, settings_commands, space_commands, ssh_key_commands, state_commands, sync_commands, system_commands, terminal_commands, transfer_commands, undo_commands, url_handler_commands, version_commands, filter_commands};
use ledger::OperationLedger;
use narrator::OperationNarrator;
use safety::SafetyInterlock;
use commands::terminal_commands::TerminalManager;
use connectors::{ConnectionManager, ConnectorRegistry, DropboxConnector, GoogleDriveConnector, OneDriveConnector, PeerManager, S3Connector, ServerTransferManager};
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
    dropbox_connector: std::sync::Arc<DropboxConnector>,
    /// Google Drive / Dropbox / OneDrive sign-ins in progress.
    oauth_flows: std::sync::Arc<connectors::oauth_flow::OAuthFlows>,
    editor_state: std::sync::Arc<commands::editor_commands::EditorState>,
    automation_mgr: AutomationManager,
    spaces_mgr: SpacesManager,
    confirmation_mgr: ConfirmationManager,
    ledger: OperationLedger,
    safety_interlock: SafetyInterlock,
    narrator: OperationNarrator,
    quickflow_suggester: LedgerSuggester,
    backup_mgr: BackupManager,
    schedule_library: schedule_library::ScheduleLibrary,
    fleet_agent: fleet_agent::FleetAgent,
    /// The web server inside the app (Settings > Local web server; off until started).
    embedded_server: embedded_server::EmbeddedServer,
    /// Optional loopback Prometheus exporter (Settings → Advanced; off by default).
    desktop_metrics: backup_engine::metrics_exporter::DesktopMetrics,
    background_mode: tray::BackgroundMode,
    audit_trail: security::audit::AuditTrail,
    metadata_store: metadata_engine::MetadataStore,
    search_manager: search_engine::SearchManager,
    thumbnails: std::sync::Arc<thumbnail_engine::ThumbnailCache>,
    office_renderer: std::sync::Arc<thumbnail_engine::office::OfficeRenderer>,
    credential_store: std::sync::Arc<CredentialStore>,
}

/// Settings that must be in force before any engine starts: the rclone
/// binary and rclone.conf chosen in Settings → rclone. Applied right after
/// the database opens, in both init paths, so saved auto-mounts, the backup
/// scheduler and every other rclone caller use them from the first call.
async fn apply_startup_settings(repo: &Repository) {
    commands::rclone_commands::activate_stored(repo).await;
}

/// Initialize the database, transfer manager, and connection manager, restoring persisted state.
async fn initialize_app_state() -> Result<AppState, crate::core::error::AppError> {
    // Open database (creates if needed, runs migrations)
    let repo = Repository::open_default().await?;
    // Before anything below can run rclone (auto-mounts, backups, connectors).
    apply_startup_settings(&repo).await;

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

    // Failed jobs are restored too (still failed); keep only what the saved limits allow.
    transfer_engine::retention::enforce_at_startup(&repo).await;
    transfer_engine::retention::persist_failures(&transfer_mgr, repo.clone());
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
    let connection_mgr = ConnectionManager::new(repo.clone(), credential_store.clone());

    // Initialize transfer history and let the manager record terminal jobs
    let transfer_history = TransferHistory::new(repo.pool().clone());
    transfer_mgr.set_history(transfer_history.clone()).await;

    // Shared OAuth connector instances: in the registry and managed for the
    // typed cloud commands; rotated tokens are saved to the credential store.
    let s3_connector = std::sync::Arc::new(S3Connector::new());
    let gdrive_connector = std::sync::Arc::new(GoogleDriveConnector::new().with_credentials(credential_store.clone()));
    let onedrive_connector = std::sync::Arc::new(OneDriveConnector::new().with_credentials(credential_store.clone()));
    let dropbox_connector = std::sync::Arc::new(DropboxConnector::new().with_credentials(credential_store.clone()));

    // Initialize connector registry (SFTP, FTP, WebDAV, SMB, NFS, Local, cloud)
    let connector_registry = ConnectorRegistry::with_cloud(gdrive_connector.clone(), dropbox_connector.clone(), onedrive_connector.clone());

    // Named schedules and holiday calendars, loaded before anything that is
    // scheduled by them (sync pairs, automation rules, backup jobs).
    let schedule_library = schedule_library::ScheduleLibrary::new(repo.pool().clone());
    if let Err(e) = schedule_library.load().await {
        tracing::warn!("Failed to load the schedule library (non-fatal): {e}");
    }

    // Initialize sync manager (T-039..T-042)
    let sync_mgr = SyncManager::new();
    sync_mgr.attach_repository(repo.clone());
    // Restore saved pairs and re-arm their triggers. Non-fatal, matching the
    // automation-rule load below: a corrupt row must not stop the app booting.
    // The in-memory fallback path deliberately does not do this — its DB is
    // created fresh each boot, so there is never anything to load.
    if let Err(e) = sync_mgr.load_pairs_from_db(&repo).await {
        tracing::warn!("Failed to load sync pairs (non-fatal): {e}");
    }
    if let Err(e) = sync_mgr.load_reports_from_db(&repo, 50).await {
        tracing::warn!("Failed to load sync run history (non-fatal): {e}");
    }
    if let Err(e) = sync_mgr.load_default_options(&repo).await {
        tracing::warn!("Failed to load the global sync defaults (non-fatal): {e}");
    }
    if let Err(e) = commands::transfer_commands::load_bandwidth_schedule(&repo).await {
        tracing::warn!("Failed to load the bandwidth timetable (non-fatal): {e}");
    }

    // Initialize peer manager (T-031) and server transfer manager (T-033)
    let peer_mgr = PeerManager::open(connectors::peer::default_peer_dir());
    let server_transfer_mgr = ServerTransferManager::new();

    // Initialize AI assistant (T-043..T-045)
    let ai_assistant = AiAssistant::with_audit(repo.pool().clone());

    // Initialize terminal manager (T-046)
    let terminal_mgr = TerminalManager::new();

    // Initialize vault manager for encrypted vaults (T-047)
    let vault_mgr = VaultManager::new();
    vault_mgr.set_pool(repo.pool().clone()).await;

    // Initialize master password manager
    let master_pw_mgr = MasterPasswordManager::new();

    // Initialize mount manager (Finder-mounted remote storage)
    let mount_mgr = MountManager::new();
    // Restore mounts the user flagged "Auto-mount". Spawned, never awaited:
    // mounting shells out to mount_smbfs/sshfs, which blocks for a long time on
    // an unreachable host, and app boot must not wait on the network. Clones are
    // cheap (Arc / Repository handle) and must be taken here, before these are
    // moved into managed state below.
    {
        let mounts = mount_mgr.clone();
        let mount_repo = repo.clone();
        let mount_conns = connection_mgr.clone();
        tokio::spawn(async move {
            mounts.auto_mount_saved(&mount_repo, &mount_conns).await;
        });
    }

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
    automation_mgr.set_repo(repo.clone());
    if let Err(e) = automation_mgr.load_rules_from_db(&repo).await {
        tracing::warn!("Failed to load automation rules (non-fatal): {e}");
    }
    // Watchers and cron schedules start in `setup`: tasks spawned on this
    // init runtime would die when it is dropped.

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

    // Quickflow Suggester — bridges the ledger and the automation engine.
    // Watches for repeated source→dest move/copy patterns and surfaces
    // single-click "Save as Quickflow" suggestions inside the existing
    // automation panel. Pure derivation over existing ledger data.
    let quickflow_suggester = LedgerSuggester::new(ledger.clone());

    // Versioned backup jobs: restore saved jobs, then arm the scheduler
    // (which also catches up on runs missed while the app was closed).
    let backup_mgr = BackupManager::new(
        repo.pool().clone(),
        credential_store.clone(),
        Some(connection_mgr.clone()),
        Some(ledger.clone()),
        backup_engine::default_data_dir(),
    );
    // Without an OS keychain, job passphrases persist in the (encrypted)
    // settings store instead of process memory.
    let settings_store =
        match crate::security::settings_store::SettingsStore::open(repo.pool().clone(), &credential_store, Default::default()).await {
            Ok(store) => {
                backup_mgr.set_settings_store(store.clone());
                Some(store)
            }
            Err(e) => {
                tracing::warn!("Settings store unavailable (non-fatal): {e}");
                None
            }
        };
    // Tamper-evident security audit log: anchors are signed with a key in
    // the OS keychain (or the encrypted settings store as a fallback).
    let agent_store = settings_store.clone().unwrap_or_else(|| crate::security::settings_store::SettingsStore::plain(repo.pool().clone()));
    let repo_for_metadata = repo.clone();
    let audit_trail =
        security::audit::AuditTrail::new(repo.pool().clone(), Some(credential_store.clone()), settings_store);
    if let Err(e) = backup_mgr.load_from_db().await {
        tracing::warn!("Failed to load backup jobs (non-fatal): {e}");
    }
    // Management console agent (registration, heartbeats, policies, reports);
    // its loop starts in `setup`, like the scheduler.
    let embedded_server = embedded_server::EmbeddedServer::new(repo.clone(), backup_mgr.clone(), connection_mgr.clone(), agent_store.clone());
    let fleet_agent = fleet_agent::init_or_default(backup_mgr.clone(), agent_store, credential_store.clone(), "desktop").await;
    // The scheduler starts in Tauri's `setup` hook: this function runs on a
    // throwaway runtime whose spawned tasks die when it is dropped.
    let background_mode = tray::load_background_mode(&repo).await;

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
        dropbox_connector,
        oauth_flows: std::sync::Arc::new(connectors::oauth_flow::OAuthFlows::new()),
        editor_state,
        automation_mgr,
        spaces_mgr,
        confirmation_mgr,
        ledger,
        schedule_library,
        safety_interlock,
        narrator,
        quickflow_suggester,
        desktop_metrics: backup_engine::metrics_exporter::DesktopMetrics::new(backup_mgr.clone()),
        backup_mgr,
        fleet_agent,
        embedded_server,
        background_mode,
        audit_trail,
        metadata_store: metadata_engine::MetadataStore::new(repo_for_metadata),
        search_manager: search_engine::SearchManager::open_default(),
        thumbnails: std::sync::Arc::new(thumbnail_engine::ThumbnailCache::open_default()),
        office_renderer: std::sync::Arc::new(thumbnail_engine::office::OfficeRenderer::open_default()),
        credential_store: credential_store.clone(),
    })
}

/// Start the long-running automation and sync triggers on the app runtime:
/// file watchers and cron schedules for automation rules, and the cron loop
/// for scheduled sync pairs. Both used to be armed but never run.
fn start_background_engines(app: tauri::AppHandle) {
    use tauri::Manager;
    let automation = app.state::<AutomationManager>();
    automation.set_services(std::sync::Arc::new(
        automation_engine::tauri_services::TauriActionServices::new(app.clone()),
    ));

    let sync_app = app.clone();
    app.state::<SyncManager>().start_scheduler(std::sync::Arc::new(move |pair_id: uuid::Uuid| {
        let app = sync_app.clone();
        tauri::async_runtime::spawn(async move {
            let manager = app.state::<SyncManager>();
            let ledger = app.state::<OperationLedger>();
            let repo = app.state::<Repository>();
            match commands::sync_commands::run_sync_pair_as(&manager, &ledger, &repo, &pair_id.to_string(), ufop_core::run_trigger::RunTrigger::Scheduled).await {
                Ok(report) => tracing::info!("Scheduled sync {} finished: {:?}", report.pair_name, report.status),
                Err(e) => tracing::warn!("Scheduled sync {pair_id} failed: {e}"),
            }
        });
    }));

    tauri::async_runtime::spawn(async move {
        // Rules and proxy first, so no watcher or scheduled job runs unchecked
        // or connects around the proxy.
        commands::policy_commands::activate_stored(&app.state::<Repository>()).await;
        commands::proxy_commands::activate_stored(&app.state::<Repository>(), &app.state::<std::sync::Arc<CredentialStore>>()).await;
        managed_settings::activate_stored(&app.state::<Repository>(), &app.state::<AiAssistant>(), &app.state::<BackupManager>(), &app.state::<MountManager>()).await;
        let automation = app.state::<AutomationManager>();
        if let Err(e) = automation.start_all_watchers().await {
            tracing::warn!("Failed to start automation watchers (non-fatal): {e}");
        }
        automation.start_scheduler();
        // The webhook listener only runs when some rule uses it.
        let uses_webhooks = automation.list_rules().await.iter().any(|r| r.trigger == automation_engine::RuleTrigger::Webhook);
        if uses_webhooks {
            let addr = commands::automation_commands::webhook_bind_addr(&app.state::<Repository>()).await;
            if let Err(e) = automation.start_webhook_listener(addr).await {
                tracing::warn!("Automation webhook listener unavailable on {addr}: {e}");
            }
        }
    });
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
    // Portable mode: point temp, webview and XDG folders into the portable
    // root before anything else starts (single-threaded here).
    paths::apply_process_env();
    let log_dir = logging::log_dir();
    logging::init(&log_dir);
    logging::Retention::load(&log_dir).apply(&log_dir);

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
                rt2.block_on(apply_startup_settings(&repo));
                let credential_store = std::sync::Arc::new(CredentialStore::new());
                let connection_mgr = ConnectionManager::new(repo.clone(), credential_store.clone());
                let fb_gdrive = std::sync::Arc::new(GoogleDriveConnector::new().with_credentials(credential_store.clone()));
                let fb_onedrive = std::sync::Arc::new(OneDriveConnector::new().with_credentials(credential_store.clone()));
                let fb_dropbox = std::sync::Arc::new(DropboxConnector::new().with_credentials(credential_store.clone()));
                let transfer_history = TransferHistory::new(repo.pool().clone());
                let transfer_mgr = TransferManager::new();
                let vault_mgr = VaultManager::new();
                rt2.block_on(async {
                    transfer_mgr.init_journal(repo.pool().clone()).await;
                    transfer_mgr.set_history(transfer_history.clone()).await;
                    transfer_engine::retention::persist_failures(&transfer_mgr, repo.clone());
                    vault_mgr.set_pool(repo.pool().clone()).await;
                });
                let ledger = OperationLedger::new(repo.pool().clone());
                let safety_interlock = SafetyInterlock::new(ledger.clone());
                let narrator = OperationNarrator::new(ledger.clone());
                let quickflow_suggester = LedgerSuggester::new(ledger.clone());
                let mut automation_mgr = AutomationManager::new();
                automation_mgr.set_ledger(ledger.clone());
                automation_mgr.set_repo(repo.clone());
                // Backup jobs can still be created and run this session; the
                // scheduler is not started because nothing persists.
                let backup_mgr = BackupManager::new(
                    repo.pool().clone(),
                    credential_store.clone(),
                    Some(connection_mgr.clone()),
                    Some(ledger.clone()),
                    backup_engine::default_data_dir(),
                );
                // In-memory DB: no settings store; the key comes from the
                // env var or keychain, else is ephemeral like the DB itself.
                // The console agent still works; its settings live in the in-memory DB.
                let fleet_agent = rt2.block_on(fleet_agent::init_or_default(
                    backup_mgr.clone(),
                    crate::security::settings_store::SettingsStore::plain(repo.pool().clone()),
                    credential_store.clone(),
                    "desktop",
                ));
                let embedded_server = embedded_server::EmbeddedServer::new(
                    repo.clone(),
                    backup_mgr.clone(),
                    connection_mgr.clone(),
                    crate::security::settings_store::SettingsStore::plain(repo.pool().clone()),
                );
                let audit_trail = security::audit::AuditTrail::new(repo.pool().clone(), Some(credential_store.clone()), None);
                let ai_assistant = AiAssistant::with_audit(repo.pool().clone());
                let fallback_schedules = schedule_library::ScheduleLibrary::new(repo.pool().clone());
                let fallback_repo = repo.clone();
                AppState {
                    metadata_store: metadata_engine::MetadataStore::new(repo.clone()),
                    search_manager: search_engine::SearchManager::new(
                        search_engine::SearchIndex::open_in_memory().expect("in-memory SQLite"),
                    ),
                    thumbnails: std::sync::Arc::new(thumbnail_engine::ThumbnailCache::open_default()),
                    office_renderer: std::sync::Arc::new(thumbnail_engine::office::OfficeRenderer::open_default()),
                    credential_store: credential_store.clone(),
                    repo,
                    transfer_mgr,
                    connection_mgr,
                    transfer_history,
                    connector_registry: ConnectorRegistry::with_cloud(fb_gdrive.clone(), fb_dropbox.clone(), fb_onedrive.clone()),
                    sync_mgr: {
                        let m = SyncManager::new();
                        m.attach_repository(fallback_repo.clone());
                        m
                    },
                    peer_mgr: PeerManager::new(),
                    server_transfer_mgr: ServerTransferManager::new(),
                    ai_assistant,
                    terminal_mgr: TerminalManager::new(),
                    vault_mgr,
                    master_pw_mgr: MasterPasswordManager::new(),
                    mount_mgr: MountManager::new(),
                    s3_connector: std::sync::Arc::new(S3Connector::new()),
                    gdrive_connector: fb_gdrive,
                    onedrive_connector: fb_onedrive,
                    dropbox_connector: fb_dropbox,
                    oauth_flows: std::sync::Arc::new(connectors::oauth_flow::OAuthFlows::new()),
                    editor_state: std::sync::Arc::new(commands::editor_commands::EditorState::new()),
                    automation_mgr,
                    spaces_mgr: SpacesManager::new(),
                    confirmation_mgr: ConfirmationManager::new(),
                    ledger,
                    schedule_library: fallback_schedules,
                    safety_interlock,
                    narrator,
                    quickflow_suggester,
                    desktop_metrics: backup_engine::metrics_exporter::DesktopMetrics::new(backup_mgr.clone()),
                    backup_mgr,
                    fleet_agent,
                    embedded_server,
                    background_mode: tray::BackgroundMode::default(),
                    audit_trail,
                }
            }
        }
    };

    // Clone for shutdown hook
    let shutdown_repo = app_state.repo.clone();
    let setup_backup_mgr = app_state.backup_mgr.clone();
    let close_mode = app_state.background_mode.clone();
    let shutdown_transfer = app_state.transfer_mgr.clone();
    let shutdown_backup = app_state.backup_mgr.clone();
    let setup_audit = app_state.audit_trail.clone();
    let setup_agent = app_state.fleet_agent.clone();
    let setup_metrics = app_state.desktop_metrics.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .setup(move |app| {
            use tauri::Manager;
            use tauri_plugin_notification::NotificationExt;
            // The main window is created here rather than from the config
            // (`"create": false` in tauri.conf.json), so portable mode on
            // macOS can give it its own web data store (macOS 14+).
            if app.webview_windows().is_empty() {
                if let Some(cfg) = app.config().app.windows.first().cloned() {
                    // `from_config` does not apply the config's data store
                    // identifier, so it is set on the builder.
                    let mut window = tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?;
                    if let Some(id) = paths::webview_store_identifier(paths::dirs()) {
                        window = window.data_store_identifier(id);
                    }
                    window.build()?;
                }
            }
            let mgr = setup_backup_mgr.clone();
            // Backup results → OS notifications (warnings and errors by default).
            let handle = app.handle().clone();
            mgr.set_notifier(std::sync::Arc::new(move |title: &str, body: &str, _status: &str| {
                if let Err(e) = handle.notification().builder().title(title).body(body).show() {
                    tracing::warn!("Backup notification failed: {e}");
                }
            }));
            // Transfers and sync runs → OS notifications, when Settings →
            // Notifications asks for them; sync results → the pair's profile.
            let desktop = app.handle().clone();
            let sink: std::sync::Arc<dyn Fn(notify_hub::desktop::DesktopEvent) + Send + Sync> = std::sync::Arc::new(move |event| {
                let app = desktop.clone();
                tauri::async_runtime::spawn(async move { notify_hub::desktop::show(&app, event).await });
            });
            notify_hub::desktop::wire_transfers(&app.state::<TransferManager>(), sink.clone());
            notify_hub::desktop::wire_sync(&app.state::<SyncManager>(), sink);
            notify_hub::run::wire_sync_reports(app.handle());
            notify_hub::run::wire_profile_users(app.handle());
            // Chat-bot remote control: nothing starts unless the user turned it on.
            app.manage(chat_control::service::ChatControlService::new(app.handle().clone()));
            let chat = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = chat.state::<chat_control::service::ChatControlService>().apply(false).await {
                    tracing::warn!("Remote control did not start: {}", e.user_message());
                }
            });
            mgr.start_scheduler();
            // The Prometheus exporter, when the user turned it on.
            let metrics = setup_metrics.clone();
            tauri::async_runtime::spawn(async move {
                metrics.start_from_settings().await;
            });
            start_background_engines(app.handle().clone());
            // Console-managed proxy and AI limits.
            let apply_handle = app.handle().clone();
            setup_agent.host().set_managed_settings_handler(std::sync::Arc::new(move |settings| {
                let h = apply_handle.clone();
                Box::pin(async move {
                    let org = h.state::<fleet_agent::FleetAgent>().status().organization;
                    let targets = managed_settings::Targets { repo: &h.state::<Repository>(), creds: &h.state::<std::sync::Arc<CredentialStore>>(), ai: &h.state::<AiAssistant>(), backups: &h.state::<BackupManager>(), mounts: &h.state::<MountManager>(), automation: &h.state::<AutomationManager>() };
                    managed_settings::apply(targets, &org, settings).await
                })
            }));
            fleet_agent::spawn(setup_agent.clone(), "unified-file-ops");
            // Signed audit checkpoints: once at start-up, then hourly.
            tauri::async_runtime::spawn(setup_audit.clone().run_anchor_loop(security::audit::ANCHOR_INTERVAL));
            commands::mount_commands::forward_mount_changes(app.handle().clone(), &app.state::<MountManager>());
            if let Err(e) = tray::setup(app, mgr) {
                tracing::warn!("System tray unavailable (non-fatal): {e}");
            }
            // Launched at login: start hidden in the tray.
            if std::env::args().any(|a| a == "--background") {
                for w in app.webview_windows().values() {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
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
        .manage(app_state.dropbox_connector)
        .manage(app_state.oauth_flows)
        .manage(app_state.editor_state)
        .manage(app_state.automation_mgr)
        .manage(app_state.spaces_mgr)
        .manage(app_state.confirmation_mgr)
        .manage(app_state.ledger)
        .manage(app_state.schedule_library)
        .manage(app_state.safety_interlock)
        .manage(app_state.narrator)
        .manage(app_state.quickflow_suggester)
        .manage(app_state.backup_mgr)
        .manage(app_state.fleet_agent)
        .manage(app_state.embedded_server)
        .manage(app_state.desktop_metrics)
        .manage(app_state.background_mode)
        .manage(app_state.audit_trail)
        .manage(app_state.metadata_store)
        .manage(app_state.search_manager)
        .manage(app_state.thumbnails)
        .manage(app_state.office_renderer)
        .manage(app_state.credential_store)
        .invoke_handler(tauri::generate_handler![
            // Security audit log (tamper-evident, append-only)
            audit_commands::verify_audit_log,
            audit_commands::export_audit_log,
            audit_commands::create_audit_anchor,
            audit_commands::get_audit_log_status,
            // System
            system_commands::greet,
            system_commands::get_app_version,
            system_commands::get_portable_status,
            system_commands::get_platform_info,
            // Filesystem
            fs_commands::list_directory,
            fs_commands::pick_folder,
            fs_commands::pick_files,
            fs_commands::pick_save_file,
            fs_commands::read_text_file_full,
            fs_commands::write_text_file_full,
            // State management
            state_commands::save_workspace_state,
            state_commands::load_workspace_state,
            state_commands::get_config,
            state_commands::list_config,
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
            transfer_commands::get_bandwidth_schedule,
            transfer_commands::set_bandwidth_schedule,
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
            transfer_commands::get_failed_transfer_retention,
            transfer_commands::set_failed_transfer_retention,
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
            connection_commands::set_connection_starred,
            connection_commands::list_connection_status,
            connection_commands::clear_connection_credentials_rejected,
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
            connector_commands::connector_upload,
            connector_commands::connector_download,
            connector_commands::connector_make_directory,
            connector_commands::connector_remove,
            connector_commands::connector_list_protocols,
            // OAuth sign-in (Google Drive, Dropbox, OneDrive)
            oauth_commands::oauth_provider_info,
            oauth_commands::oauth_begin,
            oauth_commands::oauth_await,
            oauth_commands::oauth_complete,
            oauth_commands::oauth_cancel,
            oauth_commands::oauth_import_grant,
            oauth_commands::oauth_sign_out,
            oauth_commands::oauth_status,
            oauth_commands::gdrive_list_shared_drives,
            oauth_commands::onedrive_search_sites,
            oauth_commands::onedrive_list_site_libraries,
            oauth_commands::dropbox_list_namespaces,
            connector_commands::connector_quota,
            connector_commands::connector_folder_size,
            connector_commands::connector_folder_size_cancel,
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
            // Management console agent
            fleet_agent_commands::fleet_agent_status,
            fleet_agent_commands::fleet_agent_register,
            fleet_agent_commands::fleet_agent_start_claim,
            fleet_agent_commands::fleet_agent_connect_start,
            fleet_agent_commands::fleet_agent_connect_finish,
            fleet_agent_commands::fleet_agent_connect_cancel,
            fleet_agent_commands::fleet_agent_get_limits,
            fleet_agent_commands::fleet_agent_set_limits,
            fleet_agent_commands::fleet_agent_poll_claim,
            fleet_agent_commands::fleet_agent_cancel_claim,
            fleet_agent_commands::fleet_agent_unregister,
            fleet_agent_commands::fleet_agent_set_enabled,
            fleet_agent_commands::fleet_agent_sync_now,
            // Versioned backups
            backup_commands::backup_list_jobs,
            backup_commands::backup_get_job,
            backup_commands::backup_save_job,
            backup_commands::backup_delete_job,
            backup_commands::duplicate_backup_job,
            backup_commands::backup_test_destination,
            backup_commands::backup_test_destination_compat,
            backup_commands::backup_run,
            backup_commands::backup_execute,
            backup_commands::backup_cancel,
            backup_commands::backup_change_passphrase,
            chat_control_commands::chat_control_get,
            chat_control_commands::chat_control_save,
            chat_control_commands::chat_control_status,
            schedule_commands::schedules_list,
            schedule_commands::upcoming_runs,
            schedule_commands::schedule_save,
            schedule_commands::schedule_delete,
            schedule_commands::calendars_list,
            schedule_commands::calendar_save,
            schedule_commands::calendar_delete,
            schedule_commands::calendar_import_ics,
            schedule_commands::schedule_preview,
            backup_commands::backup_place_legal_hold,
            backup_commands::backup_release_legal_hold,
            backup_commands::ediscovery_export,
            backup_commands::backup_status,
            backup_commands::backup_pause,
            backup_commands::backup_resume,
            backup_commands::backup_list_versions,
            backup_commands::backup_list_sources,
            backup_commands::backup_preview_retention,
            backup_commands::backup_run_all,
            backup_commands::backup_estimate,
            backup_commands::backup_set_version_metadata,
            backup_commands::backup_list_files,
            backup_commands::backup_search,
            backup_commands::backup_compare,
            backup_commands::backup_run_history,
            backup_commands::backup_replica_history,
            backup_commands::desktop_metrics_status,
            backup_commands::desktop_metrics_configure,
            backup_commands::backup_prune_history,
            backup_commands::backup_export_config,
            backup_commands::backup_import_config,
            backup_commands::backup_export_is_encrypted,
            backup_commands::backup_export_cli,
            backup_commands::backup_browse_remote,
            backup_commands::backup_direct_restore,
            backup_commands::backup_discover_remote_config,
            backup_commands::backup_passphrase_strength,
            backup_commands::backup_generate_passphrase,
            backup_commands::backup_capabilities,
            backup_commands::backup_preview_sources,
            backup_commands::backup_photos_libraries,
            backup_commands::backup_photos_library_info,
            backup_commands::backup_get_global_defaults,
            backup_commands::backup_effective_caps,
            backup_commands::backup_set_global_defaults,
            backup_commands::backup_effective_job,
            backup_commands::backup_list_destinations,
            backup_commands::backup_save_destination,
            backup_commands::backup_delete_destination,
            backup_commands::backup_adopt_repository,
            backup_commands::backup_open_point_in_time,
            backup_commands::backup_lock_status,
            backup_commands::backup_hardening_report,
            backup_commands::backup_break_lock,
            backup_commands::backup_set_database_encryption,
            backup_commands::backup_database_inventory,
            backup_commands::backup_cleanup_databases,
            backup_commands::backup_bug_report,
            backup_commands::backup_test_notification,
            backup_commands::backup_get_secret_settings,
            backup_commands::backup_crypt_file,
            backup_commands::backup_decrypt_volume,
            backup_commands::backup_set_secret_settings,
            backup_commands::backup_test_secret_providers,
            backup_commands::backup_job_placeholders,
            backup_commands::backup_diagnose,
            backup_commands::backup_recovery_download,
            backup_commands::backup_recovery_index,
            backup_commands::backup_recovery_list,
            backup_commands::backup_recovery_restore,
            backup_commands::backup_recovery_convert,
            backup_commands::backup_remote_list,
            backup_commands::backup_remote_get,
            backup_commands::backup_remote_put,
            backup_commands::backup_remote_delete,
            backup_commands::backup_start_operation,
            backup_commands::backup_resolve_version,
            backup_commands::backup_read_file,
            backup_commands::backup_export_file,
            backup_commands::backup_preview_file,
            backup_commands::backup_cache_stats,
            backup_commands::backup_bench,
            backup_commands::backup_clear_cache,
            backup_commands::backup_get_cache_settings,
            backup_commands::backup_set_cache_settings,
            backup_commands::backup_schedule_preview,
            backup_commands::backup_list_templates,
            backup_commands::backup_get_template,
            backup_commands::backup_save_template,
            backup_commands::backup_delete_template,
            backup_commands::backup_list_notification_profiles,
            backup_commands::backup_get_notification_profile,
            backup_commands::backup_save_notification_profile,
            backup_commands::backup_delete_notification_profile,
            backup_commands::backup_test_notification_profile,
            backup_commands::backup_preview_notification,
            backup_commands::backup_webhook_preset,
            backup_commands::backup_apply_template,
            backup_commands::backup_export_templates,
            backup_commands::backup_import_templates,
            backup_commands::backup_effective_settings,
            backup_commands::backup_open_drive,
            backup_commands::backup_close_drive,
            backup_commands::backup_list_drives,
            backup_commands::backup_set_destination_secret,
            backup_commands::backup_has_destination_secret,
            backup_commands::backup_cloud_providers,
            backup_commands::backup_get_desktop_settings,
            backup_commands::backup_set_desktop_settings,
            sync_commands::create_sync_pair,
            sync_commands::delete_sync_pair,
            sync_commands::list_sync_pairs,
            sync_commands::update_sync_pair,
            sync_commands::run_sync,
            sync_commands::get_sync_run_history,
            sync_commands::get_sync_run_items,
            sync_commands::get_sync_live_files,
            sync_commands::prune_sync_run_history,
            sync_commands::get_sync_health,
            sync_commands::sync_dry_run,
            sync_commands::export_sync_preview_csv,
            sync_commands::get_sync_conflicts,
            sync_commands::resolve_sync_conflict,
            sync_commands::get_quarantine_entries,
            sync_commands::get_sync_reports,
            sync_commands::rollback_sync,
            sync_commands::get_sync_baseline_info,
            sync_commands::reset_sync_baseline,
            sync_commands::duplicate_sync_pair,
            sync_commands::export_sync_pair,
            sync_commands::import_sync_pair,
            sync_commands::get_sync_default_options,
            sync_commands::set_sync_default_options,
            sync_commands::export_sync_report_csv,
            sync_commands::export_sync_report_json,
            sync_commands::start_sync_watcher,
            sync_commands::stop_sync_watcher,
            sync_commands::validate_sync_cron,
            // Sync time offset detection
            sync_commands::detect_time_offset,
            // Sync Filter Presets
            sync_commands::get_sync_filter_presets,
            filter_commands::parse_filter_rules,
            filter_commands::get_filter_groups,
            filter_commands::list_user_filter_presets,
            filter_commands::save_user_filter_preset,
            filter_commands::delete_user_filter_preset,
            sync_commands::parse_robocopy_command,
            sync_commands::robocopy_command_preview,
            sync_commands::get_sync_progress,
            sync_commands::cancel_sync,
            // Peer discovery and transfer (T-031)
            peer_commands::peer_start_discovery,
            peer_commands::peer_local_info,
            peer_commands::peer_start_listener,
            peer_commands::peer_stop_listener,
            peer_commands::peer_clear_finished,
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
            ai_commands::ai_help,
            ai_commands::ai_activity_report,
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
            terminal_commands::terminal_help_commands,
            terminal_commands::terminal_create_remote,
            terminal_commands::terminal_list_sessions,
            terminal_commands::terminal_close_session,
            terminal_commands::terminal_write,
            terminal_commands::terminal_resize,
            terminal_commands::terminal_read_scrollback,
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
            encryption_commands::tls_fetch_fingerprint,
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
            file_ops_commands::copy_files_to_many,
            commands::editor_draft_commands::editor_draft_save,
            commands::editor_draft_commands::editor_draft_load,
            commands::editor_draft_commands::editor_draft_discard,
            commands::editor_draft_commands::editor_draft_list,
            commands::policy_commands::filename_policies_list,
            commands::policy_commands::filename_policies_save,
            commands::policy_commands::filename_policy_check,
            commands::search_commands::search_index_roots,
            commands::search_commands::search_index_add,
            commands::search_commands::search_index_refresh,
            commands::search_commands::search_index_remove,
            commands::search_commands::search_index_cancel,
            commands::search_commands::search_files_query,
            commands::thumbnail_commands::thumbnail_get,
            commands::thumbnail_commands::thumbnail_cache_clear,
            commands::convert_commands::convert_capabilities,
            commands::convert_commands::convert_images,
            commands::convert_commands::convert_documents,
            commands::gpg_commands::gpg_status,
            commands::gpg_commands::gpg_list_keys,
            commands::gpg_commands::gpg_import_keys,
            commands::gpg_commands::gpg_export_public,
            commands::gpg_commands::gpg_encrypt_files,
            commands::gpg_commands::gpg_decrypt_files,
            commands::gpg_commands::gpg_recrypt_files,
            commands::gpg_commands::gpg_generate_key,
            commands::gpg_commands::gpg_sign_files,
            commands::gpg_commands::gpg_verify,
            commands::share_commands::sharing_policy_get,
            commands::share_commands::sharing_policy_set,
            commands::share_commands::connector_create_share_link,
            commands::operations_commands::operations_snapshot,
            commands::operations_commands::operations_cancel,
            commands::proxy_commands::proxy_get,
            commands::proxy_commands::proxy_set,
            commands::proxy_commands::proxy_test,
            commands::connector_commands::rclone_list_remotes,
            commands::rclone_commands::rclone_status,
            commands::rclone_commands::rclone_settings_get,
            commands::rclone_commands::rclone_settings_set,
            commands::rclone_commands::rclone_detect,
            commands::rclone_commands::rclone_install_managed,
            commands::rclone_commands::rclone_providers,
            commands::rclone_commands::rclone_remote_get,
            commands::rclone_commands::rclone_remote_create,
            commands::rclone_commands::rclone_remote_update,
            commands::rclone_commands::rclone_config_continue,
            commands::rclone_commands::rclone_config_oauth,
            commands::rclone_commands::rclone_config_cancel,
            commands::rclone_commands::rclone_remote_delete,
            commands::rclone_commands::rclone_remote_test,
            commands::rclone_commands::rclone_crypt_info,
            commands::rclone_commands::rclone_cryptdecode,
            commands::connector_commands::connection_presets,
            file_ops_commands::move_files,
            file_ops_commands::rename_file,
            file_ops_commands::duplicate_files,
            file_ops_commands::count_affected_files,
            file_ops_commands::delete_files,
            file_ops_commands::create_directory,
            file_ops_commands::ensure_directory,
            file_ops_commands::create_file,
            file_ops_commands::undo_file_operation,
            file_ops_commands::redo_file_operation,
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
            batch_rename_commands::naming_preview,
            // Preview Engine (T-061)
            preview_commands::preview_file,
            preview_commands::preview_office_render,
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
            integrity_commands::integrity_set_metadata,
            integrity_commands::integrity_remove_metadata,
            integrity_commands::integrity_search_metadata,
            integrity_commands::integrity_metadata_keys,
            integrity_commands::metadata_list_schemas,
            integrity_commands::metadata_save_schema,
            integrity_commands::metadata_delete_schema,
            integrity_commands::metadata_schema_for,
            integrity_commands::metadata_check_compliance,
            // Mount Engine (Finder-mounted remote storage)
            mount_commands::check_mount_capability,
            mount_commands::mount_remote,
            mount_commands::unmount_remote,
            mount_commands::list_mounts,
            mount_commands::get_mount_status,
            mount_commands::save_mount_config,
            mount_commands::mount_policy,
            mount_commands::mount_pin_offline,
            mount_commands::mount_cache_usage,
            mount_commands::mount_clear_cache,
            mount_commands::delete_mount_config,
            mount_commands::update_mount_config,
            mount_commands::saved_mount_options,
            mount_commands::mount_saved_config,
            mount_commands::unmount_all_mounts,
            mount_commands::free_drive_letters,
            mount_commands::open_mount_point,
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
            file_ops_commands::reveal_in_os,
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
            log_commands::get_log_retention,
            commands::proxy_commands::managed_settings_get,
            log_commands::set_log_retention,
            log_commands::diagnostics_create,
            log_commands::diagnostics_send,
            log_commands::set_log_level,
            log_commands::get_log_directory,
            log_commands::query_logs,
            embedded_server_commands::embedded_server_status,
            embedded_server_commands::embedded_server_save_settings,
            embedded_server_commands::embedded_server_start,
            embedded_server_commands::embedded_server_stop,
            embedded_server_commands::embedded_server_set_password,
            embedded_server_commands::open_server_in_browser,
            embedded_server_commands::server_profiles_list,
            embedded_server_commands::server_profile_save,
            embedded_server_commands::server_profile_remove,
            embedded_server_commands::server_profile_test,
            feedback_commands::feedback_destination,
            feedback_commands::feedback_submit,
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
            ssh_key_commands::list_known_hosts,
            ssh_key_commands::forget_known_host,
            // Automation engine (Quickflows)
            automation_commands::automation_rule_revisions,
            automation_commands::automation_restore_revision,
            automation_commands::automation_convert_to_workflow,
            automation_commands::automation_action_risks,
            automation_commands::automation_rule_confirmations,
            automation_commands::create_automation_rule,
            automation_commands::update_automation_rule,
            automation_commands::delete_automation_rule,
            automation_commands::list_automation_rules,
            automation_commands::get_automation_rule,
            automation_commands::enable_automation_rule,
            automation_commands::run_automation_rule,
            automation_commands::test_automation_rule,
            automation_commands::approve_automation_command,
            automation_commands::clone_automation_rule,
            automation_commands::rotate_automation_webhook,
            automation_commands::list_automation_logs,
            automation_commands::clear_automation_logs,
            automation_commands::parse_automation_nl,
            // Quickflow Suggester (ledger → automation bridge)
            automation_commands::quickflow_suggestions_list,
            automation_commands::quickflow_suggestion_dismiss,
            automation_commands::quickflow_suggestion_accept,
            // Operation Pin (single past ledger event → re-runnable manual Quickflow)
            automation_commands::pin_ledger_event,
            // Operation Retry (failure-side symmetry of Pin — re-attempt a failed fs.copy/fs.move)
            automation_commands::retry_failed_event,
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
            // Smart Locations (frecency-ranked paths from ledger — zero new infrastructure)
            ledger_commands::ledger_frecent_paths,
            // Smart Send-To (extension → frecency-ranked destination dirs)
            ledger_commands::ledger_extension_destinations,
            // Universal Time-Travel Undo (cross-session Cmd+Z backed by the ledger)
            undo_commands::undo_last,
            undo_commands::undo_by_correlation,
            undo_commands::list_undoable,
            // Universal Time-Travel Redo (cross-session Cmd+Shift+Z, symmetric with undo)
            undo_commands::redo_last,
            undo_commands::redo_by_correlation,
            undo_commands::list_redoable,
            // File Lineage / Provenance Graph (reads the ledger only; zero writes)
            lineage_commands::get_file_lineage,
            // Safety Interlock (context-aware anomaly detection over the ledger)
            safety_commands::safety_assess_intent,
            safety_commands::safety_confirm_intent,
            // Operation Narrator (plain-language cross-engine story per correlation trace)
            narrator_commands::narrator_narrate_operation,
        ])
        .on_window_event(move |window, event| {
            // "Keep running in the tray": closing hides the window so
            // scheduled backups continue.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if close_mode.enabled() {
                    api.prevent_close();
                    let _ = window.hide();
                    return;
                }
            }
            if let tauri::WindowEvent::Destroyed = event {
                tracing::info!("Window closing, persisting state...");
                // Running backups stop at a safe point (partial version,
                // repository lock released) before the app exits.
                let report = shutdown_backup.shutdown_blocking(backup_engine::shutdown::DEFAULT_GRACE);
                if report.cancelled > 0 {
                    tracing::info!("Stopped {} backup operation(s) on exit ({} still running)", report.cancelled, report.still_running);
                }
                // Backup drives and preview copies do not outlive the app.
                shutdown_backup.close_all_drives_blocking();
                shutdown_backup.cleanup_all_previews();
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

    /// The stored rclone settings become the ones every rclone call uses.
    #[tokio::test]
    async fn startup_settings_make_the_stored_rclone_current() {
        use crate::rclone_engine::resolver::{self, BinarySource, RcloneSettings};
        let prior = resolver::current_settings();
        let repo = Repository::open_in_memory().await.unwrap();
        // The binary the default lookup already finds (rclone tests running in
        // parallel see the same program); without one, a stand-in that fails
        // like a missing rclone.
        let d = tempfile::tempdir().unwrap();
        let found = resolver::resolve();
        let bin = if found.found && std::path::Path::new(&found.path).is_absolute() {
            found.path.clone()
        } else {
            let p = d.path().join("rclone");
            std::fs::write(&p, "#!/bin/sh\nexit 1\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p.to_string_lossy().into_owned()
        };
        let stored = commands::rclone_commands::persist(&repo, RcloneSettings { binary_path: Some(bin.clone()), config_path: None }).await.unwrap();
        apply_startup_settings(&repo).await;
        let (applied, now) = (resolver::current_settings(), resolver::resolve());
        resolver::apply(prior);
        assert_eq!(applied, stored);
        assert_eq!((now.path.as_str(), now.source), (bin.as_str(), BinarySource::Settings));
    }

    /// Regression: saved auto-mounts were started in `initialize_app_state`
    /// while the rclone settings were only applied later (in
    /// `start_background_engines`), so startup mounts ignored the binary and
    /// rclone.conf chosen in Settings. Both init paths must apply them as
    /// soon as the database is open, before anything can run rclone.
    #[test]
    fn startup_settings_are_applied_before_any_engine_starts() {
        let src = include_str!("lib.rs");
        let body = |start: &str, end: &str| {
            let a = src.find(start).unwrap_or_else(|| panic!("{start} not found"));
            let b = a + src[a..].find(end).unwrap_or_else(|| panic!("{end} not found after {start}"));
            &src[a..b]
        };
        let pos = |hay: &str, needle: &str| hay.find(needle).unwrap_or_else(|| panic!("{needle} not found"));
        let apply = "apply_startup_settings(&repo).await";
        // Normal path: right after the database opens, before auto-mounts and managers.
        let init = body("async fn initialize_app_state()", "\n}\n");
        let opened = pos(init, "Repository::open_default()");
        let applied = pos(init, apply);
        assert!(opened < applied, "settings applied before the database opens");
        for later in ["auto_mount_saved", "TransferManager::new()", "BackupManager::"] {
            assert!(applied < pos(init, later), "{later} runs before the rclone settings are applied");
        }
        // Fallback path: the same, before its managers are built.
        let fallback = body("Starting with in-memory database as fallback", "AppState {");
        let mem = pos(fallback, "Repository::open_in_memory()");
        let applied = pos(fallback, "apply_startup_settings(&repo)");
        assert!(mem < applied && applied < pos(fallback, "TransferManager::new()"), "fallback path does not apply the rclone settings first");
    }

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
