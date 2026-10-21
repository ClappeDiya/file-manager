//! `ufop-server` — headless backup server: REST API, browser UI, scheduler.
//!
//! ```text
//! ufop-server --set-password            # first run (reads UFOP_SERVER_PASSWORD or prompts)
//! ufop-server --interface any --allowed-hostnames backup.lan --https auto
//! ```
//! Defaults come from the machine-wide preload file (see `ufop_core::preload`).

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use unified_file_ops_lib::backup_engine::{self, policy::ModulePolicy, BackupManager};
use unified_file_ops_lib::connectors::ConnectionManager;
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

#[tokio::main]
async fn main() {
    let preload = ufop_core::preload::Preload::load();
    if let Some(p) = &preload {
        p.apply_env("ufop-server");
    }
    let argv: Vec<String> = std::env::args().collect();
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
    mgr.set_settings_store(store);
    mgr.set_module_policy(ModulePolicy::from_lists(a.allowed_backends.as_deref(), a.allowed_encryption.as_deref(), a.allowed_compression.as_deref(), a.disable_modules.as_deref()));
    if let Err(e) = mgr.load_from_db().await {
        fail(e);
    }
    if !a.no_scheduler {
        mgr.start_scheduler();
    }

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
    };
    let st = ServerState { mgr, conns: Some(conns), auth, cfg: Arc::new(cfg), started: Instant::now(), oauth };
    if let Err(e) = server::serve(st).await {
        fail(e);
    }
}
