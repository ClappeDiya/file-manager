//! `ufop-server` — headless backup server: REST API, browser UI, scheduler.
//!
//! ```text
//! ufop-server --set-password            # first run (reads UFOP_SERVER_PASSWORD or prompts)
//! ufop-server --interface any --allowed-hostnames backup.lan --https auto
//! ufop-server --interface any --allowed-cidrs 10.0.0.0/8,fd00::/8   # only these clients (by TCP peer)
//! ```
//! Defaults come from the machine-wide preload file (see `ufop_core::preload`).

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use unified_file_ops_lib::backup_engine::{self, policy::ModulePolicy, BackupManager};
use unified_file_ops_lib::connectors::ConnectionManager;
use unified_file_ops_lib::fleet_agent;
use unified_file_ops_lib::ledger::OperationLedger;
use unified_file_ops_lib::security::CredentialStore;
use unified_file_ops_lib::server::{self, auth::AuthService, oauth, settings, ServerConfig, ServerState, TlsMode};
use unified_file_ops_lib::storage::{pool::DbPool, Repository};

#[derive(Parser, Debug)]
#[command(name = "ufop-server", version, about = "UFOP headless backup server")]
struct Args {
    /// Port to listen on
    #[arg(long, env = "UFOP_SERVER_PORT", default_value_t = 8200)]
    port: u16,
    /// `loopback`, `any`, or an IP address
    #[arg(long, env = "UFOP_SERVER_INTERFACE", default_value = "loopback")]
    interface: String,
    /// Extra Host names accepted (comma separated; `*` = any)
    #[arg(long, env = "UFOP_SERVER_ALLOWED_HOSTNAMES", value_delimiter = ',')]
    allowed_hostnames: Vec<String>,
    /// Data directory (database, certificates, job indexes)
    #[arg(long, env = "UFOP_SERVER_DATADIR")]
    data_dir: Option<PathBuf>,
    /// Folder with the built web UI (`pnpm build` → dist/)
    #[arg(long, env = "UFOP_SERVER_WEBROOT")]
    webroot: Option<PathBuf>,
    /// Serve only the API, no web UI
    #[arg(long)]
    api_only: bool,
    /// Path prefixes that serve index.html (single-page app routes)
    #[arg(long, value_delimiter = ',', default_value = "/")]
    spa_paths: Vec<String>,
    /// Allowed CORS origins (comma separated)
    #[arg(long, env = "UFOP_SERVER_CORS_ORIGINS", value_delimiter = ',')]
    cors_origins: Vec<String>,
    /// Reverse-proxy pre-authentication tokens (sent in X-UFOP-PreAuth)
    /// Client address ranges allowed to connect, e.g. 10.0.0.0/8,192.168.1.0/24 (default: any).
    #[arg(long, env = "UFOP_SERVER_ALLOWED_CIDRS", value_delimiter = ',')]
    allowed_cidrs: Vec<String>,
    #[arg(long, env = "UFOP_SERVER_PREAUTH_TOKENS", value_delimiter = ',')]
    preauth_tokens: Vec<String>,
    /// `off`, `auto` (local CA + certificate) or `pfx`
    #[arg(long, default_value = "off")]
    https: String,
    /// Hostnames for the generated certificate
    #[arg(long, value_delimiter = ',', default_value = "localhost,127.0.0.1")]
    cert_hostnames: Vec<String>,
    #[arg(long, env = "UFOP_SERVER_PFX")]
    pfx: Option<PathBuf>,
    #[arg(long, env = "UFOP_SERVER_PFX_PASSWORD", default_value = "")]
    pfx_password: String,
    /// Do not run the management console agent (registration via UFOP_CONSOLE_* / preload, heartbeats, policies)
    #[arg(long, env = "UFOP_NO_CONSOLE_AGENT")]
    no_console_agent: bool,
    /// Set (or change) the UI password and exit. Reads UFOP_SERVER_PASSWORD or stdin.
    #[arg(long)]
    set_password: bool,
    /// Allow issuing long-lived automation tokens
    #[arg(long)]
    allow_automation_tokens: bool,
    /// Revoke every outstanding token (rotate the signing key) and exit
    #[arg(long)]
    reset_tokens: bool,
    /// Refuse to start without a settings encryption key
    #[arg(long)]
    require_settings_key: bool,
    /// Store server secrets unencrypted (explicit opt-out)
    #[arg(long)]
    disable_settings_encryption: bool,
    /// Lost-key recovery: back up the database, delete encrypted settings, exit
    #[arg(long)]
    clear_encrypted_settings: bool,
    #[arg(long, env = "UFOP_ALLOWED_BACKENDS")]
    allowed_backends: Option<String>,
    #[arg(long, env = "UFOP_ALLOWED_ENCRYPTION")]
    allowed_encryption: Option<String>,
    #[arg(long, env = "UFOP_ALLOWED_COMPRESSION")]
    allowed_compression: Option<String>,
    #[arg(long, env = "UFOP_DISABLE_MODULES")]
    disable_modules: Option<String>,
    /// OAuth broker configuration (JSON: public_url + providers)
    #[arg(long, env = "UFOP_OAUTH_CONFIG")]
    oauth_config: Option<PathBuf>,
    /// OAuth token store folder (default: <data-dir>/oauth-tokens)
    #[arg(long)]
    oauth_store: Option<PathBuf>,
    /// Do not run scheduled backups (API only)
    #[arg(long)]
    no_scheduler: bool,
    /// Windows: register ufop-server as an automatic-start service with the
    /// remaining arguments, then exit
    #[arg(long)]
    install_service: bool,
    /// Windows: remove the service, then exit
    #[arg(long)]
    uninstall_service: bool,
    /// Windows: run under the Service Control Manager (set by --install-service)
    #[arg(long, hide = true)]
    windows_service: bool,
}

/// Arguments forwarded to the service: everything except the install flag.
fn service_args(argv: &[String]) -> Vec<String> {
    argv.iter().skip(1).filter(|a| *a != "--install-service" && *a != "--uninstall-service").cloned().collect()
}

#[cfg(windows)]
mod winsvc {
    use std::ffi::OsString;
    use std::time::Duration;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState,
        ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher};

    pub const NAME: &str = "ufop-server";

    pub fn install(args: Vec<String>) -> windows_service::Result<()> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;
        let exe = std::env::current_exe().map_err(windows_service::Error::Winapi)?;
        let mut launch: Vec<OsString> = vec!["--windows-service".into()];
        launch.extend(args.into_iter().map(OsString::from));
        let info = ServiceInfo {
            name: NAME.into(),
            display_name: "UFOP backup server".into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe,
            launch_arguments: launch,
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let svc = mgr.create_service(&info, ServiceAccess::CHANGE_CONFIG)?;
        svc.set_description("Runs scheduled UFOP backups and serves the backup web UI")?;
        Ok(())
    }

    pub fn uninstall() -> windows_service::Result<()> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
        let svc = mgr.open_service(NAME, ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS)?;
        let _ = svc.stop();
        svc.delete()
    }

    define_windows_service!(ffi_service_main, service_main);

    pub fn run() -> windows_service::Result<()> {
        service_dispatcher::start(NAME, ffi_service_main)
    }

    fn service_main(_args: Vec<OsString>) {
        let handler = service_control_handler::register(NAME, |ev| match ev {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                std::process::exit(0);
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        });
        let Ok(h) = handler else { return };
        let _ = h.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
        // Process arguments (not the SCM start arguments) carry the configuration.
        let argv: Vec<String> = std::env::args().collect();
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(super::run(argv));
    }
}

fn read_password() -> String {
    if let Ok(p) = std::env::var("UFOP_SERVER_PASSWORD") {
        return p;
    }
    eprint!("New server password: ");
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).ok();
    s.trim_end_matches(['\r', '\n']).to_string()
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("ufop-server: {msg}");
    std::process::exit(2)
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|a| a == "--install-service" || a == "--uninstall-service" || a == "--windows-service") {
        #[cfg(windows)]
        {
            let r = if argv.iter().any(|a| a == "--install-service") {
                winsvc::install(service_args(&argv)).map(|_| println!("Service '{}' installed (automatic start).", winsvc::NAME))
            } else if argv.iter().any(|a| a == "--uninstall-service") {
                winsvc::uninstall().map(|_| println!("Service '{}' removed.", winsvc::NAME))
            } else {
                winsvc::run()
            };
            if let Err(e) = r {
                fail(format!("Windows service error: {e}"));
            }
            return;
        }
        #[cfg(not(windows))]
        {
            let _ = service_args(&argv);
            fail("Windows service options are only available on Windows; use packaging/linux/ufop-server.service or packaging/macos/com.ufop.server.plist");
        }
    }
    tokio::runtime::Runtime::new().unwrap_or_else(|e| fail(e)).block_on(run(argv));
}

async fn run(argv: Vec<String>) {
    let preload = ufop_core::preload::Preload::load();
    if let Some(p) = &preload {
        p.apply_env("ufop-server");
    }
    let argv = match &preload {
        Some(p) => p.merge_args("ufop-server", argv),
        None => argv,
    };
    let a = Args::parse_from(argv);
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")))
        .init();

    let data_dir = a.data_dir.clone().unwrap_or_else(|| backup_engine::default_data_dir().parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")));
    if let Err(e) = server::enforce_data_dir_permissions(&data_dir) {
        fail(e);
    }
    let db_file = preload.as_ref().and_then(|p| p.db_path_for("ufop-server")).map(PathBuf::from).unwrap_or_else(|| data_dir.join("ufop-server.sqlite"));
    let pool = DbPool::open(&db_file).unwrap_or_else(|e| fail(e));
    let repo = Repository::new(pool).await.unwrap_or_else(|e| fail(e));
    let creds = Arc::new(CredentialStore::new());
    let policy = settings::SettingsKeyPolicy { require_key: a.require_settings_key, disabled: a.disable_settings_encryption };
    let store = settings::SettingsStore::open(repo.pool().clone(), &creds, policy).await.unwrap_or_else(|e| fail(e));

    if a.clear_encrypted_settings {
        match store.clear_encrypted(Some(&db_file)).await {
            Ok(keys) => {
                println!("Removed {} encrypted setting(s): {}. Set a new password with --set-password.", keys.len(), keys.join(", "));
                return;
            }
            Err(e) => fail(e),
        }
    }
    let auth = AuthService::new(store.clone());
    if a.set_password {
        auth.set_password(&read_password()).await.unwrap_or_else(|e| fail(e));
        println!("Password set; existing sessions were signed out.");
        return;
    }
    if a.reset_tokens {
        auth.reset_jwt().await.unwrap_or_else(|e| fail(e));
        println!("All tokens revoked.");
        return;
    }
    if a.allow_automation_tokens {
        auth.set_automation_allowed(true).await.unwrap_or_else(|e| fail(e));
    }
    if !auth.has_password().await.unwrap_or(false) {
        fail("no password is set; run `ufop-server --set-password` first");
    }

    let conns = ConnectionManager::new(repo.clone(), creds.clone());
    let ledger = OperationLedger::new(repo.pool().clone());
    let mgr = BackupManager::new(repo.pool().clone(), creds.clone(), Some(conns.clone()), Some(ledger), data_dir.join("backups"));
    mgr.set_settings_store(store.clone());
    mgr.set_module_policy(ModulePolicy::from_lists(a.allowed_backends.as_deref(), a.allowed_encryption.as_deref(), a.allowed_compression.as_deref(), a.disable_modules.as_deref()));
    if let Err(e) = mgr.load_from_db().await {
        fail(e);
    }
    mgr.refresh_caps().await;
    if !a.no_scheduler {
        mgr.start_scheduler();
    }

    // Management console agent: registers from UFOP_CONSOLE_URL /
    // UFOP_CONSOLE_REGISTRATION_TOKEN (preload) and runs while registered.
    let agent = if a.no_console_agent {
        None
    } else {
        let agent = fleet_agent::init_or_default(mgr.clone(), store.clone(), creds.clone(), "server").await;
        // Of the console's app settings, the server applies the transfer caps.
        let caps_mgr = mgr.clone();
        agent.host().set_managed_settings_handler(Arc::new(move |settings| {
            let m = caps_mgr.clone();
            Box::pin(async move {
                m.set_managed_caps(settings.and_then(|s| s.transfers)).await;
                Ok(())
            })
        }));
        fleet_agent::spawn(agent.clone(), "ufop-server");
        Some(agent)
    };

    let oauth = a.oauth_config.as_ref().map(|p| {
        let cfg: oauth::BrokerConfig = serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| fail(e))).unwrap_or_else(|e| fail(e));
        let dir = a.oauth_store.clone().unwrap_or_else(|| data_dir.join("oauth-tokens"));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(e));
        oauth::OAuthBroker::new(cfg, Arc::new(ufop_core::backup::LocalBackend::new(dir)))
    });

    let tls = match a.https.as_str() {
        "off" => TlsMode::None,
        "auto" => TlsMode::Generated { dir: data_dir.join("tls"), hostnames: a.cert_hostnames.clone() },
        "pfx" => TlsMode::Pfx { path: a.pfx.clone().unwrap_or_else(|| fail("--https pfx needs --pfx <file>")), password: a.pfx_password.clone() },
        other => fail(format!("unknown --https mode '{other}'")),
    };
    let cfg = ServerConfig {
        port: a.port,
        interface: a.interface,
        allowed_hostnames: a.allowed_hostnames,
        webroot: a.webroot,
        api_only: a.api_only,
        spa_paths: a.spa_paths,
        cors_origins: a.cors_origins,
        preauth_tokens: a.preauth_tokens,
        tls,
        allowed_clients: ufop_core::netacl::IpAllowlist::parse(&a.allowed_cidrs).unwrap_or_else(|e| fail(format!("--allowed-cidrs: {e}"))),
    };
    let st = ServerState { mgr, conns: Some(conns), auth, cfg: Arc::new(cfg), started: Instant::now(), oauth, agent };
    if let Err(e) = server::serve(st).await {
        fail(e);
    }
}
