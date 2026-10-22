//! UFOP CLI - Unified File Operations Platform command-line interface.
//!
//! Exit codes:
//! - 0: Success
//! - 1: Partial failure
//! - 2: Full failure
//! - 3: Authentication error
//! - 4: Policy violation

mod commands;
mod config;
mod output;
mod secrets;

use clap::{Parser, Subcommand, ValueEnum};
use std::process::ExitCode;

/// Unified File Operations Platform CLI
#[derive(Parser, Debug)]
#[command(name = "ufop", version, about, long_about = None)]
pub struct Cli {
    /// Output format
    #[arg(long, global = true, default_value = "human")]
    format: OutputFormat,

    /// Output as JSON (shorthand for --format json)
    #[arg(long, global = true, conflicts_with = "yaml")]
    json: bool,

    /// Output as YAML (shorthand for --format yaml)
    #[arg(long, global = true, conflicts_with = "json")]
    yaml: bool,

    /// Dry run - show what would happen without executing
    #[arg(long, global = true)]
    dry_run: bool,

    /// Rate limit in bytes/sec (e.g., 1M, 500K)
    #[arg(long, global = true)]
    limit: Option<String>,

    /// API server URL
    #[arg(long, global = true, env = "UFOP_API_URL")]
    api_url: Option<String>,

    /// Verbosity level (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Console log level (error, warn, info, debug, trace); overrides -v
    #[arg(long, global = true, env = "UFOP_LOG_LEVEL")]
    log_level: Option<String>,

    /// Also write logs to this file (appended)
    #[arg(long, global = true, env = "UFOP_LOG_FILE")]
    log_file: Option<std::path::PathBuf>,

    /// Log level for --log-file (defaults to info)
    #[arg(long, global = true, default_value = "info")]
    log_file_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
    Yaml,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with the UFOP server
    Login {
        /// API key or token
        #[arg(long)]
        token: Option<String>,
        /// Server URL
        #[arg(long)]
        server: Option<String>,
    },

    /// Manage connections (SFTP, FTP, WebDAV, S3, etc.)
    #[command(subcommand)]
    Connection(ConnectionCommands),

    /// Transfer files between locations
    Transfer {
        /// Source path or URI (e.g., sftp://host/path, s3://bucket/key, /local/path)
        src: String,
        /// Destination path or URI
        dest: String,
        /// Overwrite existing files
        #[arg(long)]
        overwrite: bool,
        /// Resume interrupted transfers
        #[arg(long)]
        resume: bool,
        /// Verify checksums after transfer
        #[arg(long)]
        verify: bool,
        /// Number of parallel streams
        #[arg(long, default_value = "4")]
        parallel: u32,
    },

    /// Robocopy-compatible copy: `ufop copy <src> <dst> [files…] [/switches…]`
    ///
    /// Takes Robocopy's switches (/MIR /E /XO /XD /MT:8 /R:3 /W:5 /Z /L
    /// /LOG+:file /MON:n …) and returns Robocopy's exit codes: 0 nothing to
    /// do, 1 copied, 2 extras, 4 mismatches, 8 failures, 16 fatal.
    #[command(
        after_help = "Examples:\n  ufop copy C:\\Data D:\\Backup /MIR /R:3 /W:5 /MT:8\n  ufop copy ~/Photos /mnt/nas/photos *.jpg /E /XO /LOG+:photos.log\n  ufop copy src dst /L          (list only, change nothing)"
    )]
    Copy {
        /// Source, destination, optional file patterns, then Robocopy switches
        #[arg(required = true, num_args = 2.., value_name = "ARGS")]
        args: Vec<String>,
    },

    /// Manage sync pairs
    #[command(subcommand)]
    Sync(SyncCommands),

    /// Control a running ufop-server (login, run, pause, logs, health…)
    #[command(subcommand)]
    Server(commands::server::ServerCommands),

    /// Versioned, deduplicated, encrypted backups (run, restore, verify, repair…)
    #[command(subcommand)]
    Backup(commands::backup::BackupCommands),

    /// Management console agent: register / claim, status, run the agent loop
    #[command(subcommand)]
    Agent(commands::agent::AgentCommands),

    /// Check compatibility of paths across platforms
    Compat {
        /// Paths to check
        paths: Vec<String>,
        /// Target platforms (windows, macos, linux)
        #[arg(long, value_delimiter = ',')]
        targets: Option<Vec<String>>,
    },

    /// Compute checksums for files
    Checksum {
        /// Files to checksum
        paths: Vec<String>,
        /// Algorithm (md5, sha1, sha256)
        #[arg(long, default_value = "sha256")]
        algorithm: String,
        /// Verify against provided hash
        #[arg(long)]
        verify: Option<String>,
    },

    /// Find duplicate files
    Duplicates {
        /// Directory to scan
        path: String,
        /// Minimum file size to consider
        #[arg(long, default_value = "1")]
        min_size: u64,
        /// Action: report, keep-newest, keep-largest, delete
        #[arg(long, default_value = "report")]
        action: String,
    },

    /// Batch rename files
    Rename {
        /// Files or directory to rename
        path: String,
        /// Pattern with tokens: {name}, {ext}, {num}, {date}, {parent}, {counter}
        #[arg(long)]
        pattern: Option<String>,
        /// Find string (literal or regex)
        #[arg(long)]
        find: Option<String>,
        /// Replace string
        #[arg(long)]
        replace: Option<String>,
        /// Use regex for find/replace
        #[arg(long)]
        regex: bool,
        /// Case transform: upper, lower, title, sentence
        #[arg(long)]
        case: Option<String>,
        /// Counter start value
        #[arg(long, default_value = "1")]
        start: u32,
        /// Counter step
        #[arg(long, default_value = "1")]
        step: u32,
        /// Zero-padding width for counter
        #[arg(long, default_value = "0")]
        pad: usize,
    },

    /// Archive operations (create, extract, list)
    #[command(subcommand)]
    Archive(ArchiveCommands),

    /// Show server/platform status
    Status,
}

#[derive(Subcommand, Debug)]
pub enum ConnectionCommands {
    /// List saved connections
    List {
        /// Filter by protocol
        #[arg(long)]
        protocol: Option<String>,
    },
    /// Add a new connection
    Add {
        /// Connection name
        name: String,
        /// Connection URI (sftp://user@host, ftp://host, etc.)
        uri: String,
        /// Save password in keychain
        #[arg(long)]
        save_password: bool,
    },
    /// Test a connection
    Test {
        /// Connection name or URI
        target: String,
    },
    /// Remove a connection
    Remove {
        /// Connection name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SyncCommands {
    /// List sync pairs
    List,
    /// Create a new sync pair
    Create {
        /// Sync pair name
        name: String,
        /// Source path or URI
        source: String,
        /// Destination path or URI
        dest: String,
        /// Sync direction: bidirectional, source-to-dest, dest-to-source
        #[arg(long, default_value = "source-to-dest")]
        direction: String,
        /// Cron schedule for auto-sync
        #[arg(long)]
        schedule: Option<String>,
    },
    /// Run a sync pair
    Run {
        /// Sync pair name or ID
        name: String,
    },
    /// Delete a sync pair
    Delete {
        /// Sync pair name or ID
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ArchiveCommands {
    /// Create an archive
    Create {
        /// Output archive path
        output: String,
        /// Files/directories to include
        paths: Vec<String>,
        /// Archive format: zip, tar, tar.gz, 7z
        #[arg(long, default_value = "zip")]
        format: String,
        /// Password for encryption (ZIP, 7Z)
        #[arg(long)]
        password: Option<String>,
        /// Compression level (0-9)
        #[arg(long, default_value = "6")]
        level: u32,
    },
    /// Extract an archive
    Extract {
        /// Archive path
        archive: String,
        /// Destination directory
        #[arg(long)]
        dest: Option<String>,
        /// Password for encrypted archives
        #[arg(long)]
        password: Option<String>,
    },
    /// List archive contents
    List {
        /// Archive path
        archive: String,
        /// Password for encrypted archives
        #[arg(long)]
        password: Option<String>,
    },
}

impl Cli {
    /// Resolve the effective output format.
    pub fn effective_format(&self) -> OutputFormat {
        if self.json {
            OutputFormat::Json
        } else if self.yaml {
            OutputFormat::Yaml
        } else {
            self.format.clone()
        }
    }

    /// Parse rate limit string to bytes per second.
    pub fn rate_limit_bps(&self) -> Option<u64> {
        self.limit.as_ref().map(|s| parse_rate_limit(s))
    }
}

fn parse_rate_limit(s: &str) -> u64 {
    let s = s.trim().to_uppercase();
    if let Some(num) = s.strip_suffix('G') {
        num.parse::<u64>().unwrap_or(0) * 1_073_741_824
    } else if let Some(num) = s.strip_suffix('M') {
        num.parse::<u64>().unwrap_or(0) * 1_048_576
    } else if let Some(num) = s.strip_suffix('K') {
        num.parse::<u64>().unwrap_or(0) * 1024
    } else {
        s.parse::<u64>().unwrap_or(0)
    }
}

fn parse_level(s: &str) -> Result<tracing_subscriber::filter::LevelFilter, String> {
    use tracing_subscriber::filter::LevelFilter;
    Ok(match s.to_ascii_lowercase().as_str() {
        "off" => LevelFilter::OFF,
        "error" => LevelFilter::ERROR,
        "warn" | "warning" => LevelFilter::WARN,
        "info" => LevelFilter::INFO,
        "debug" => LevelFilter::DEBUG,
        "trace" | "profiling" => LevelFilter::TRACE,
        _ => return Err(format!("unknown log level '{s}' (use error, warn, info, debug or trace)")),
    })
}

/// Command line for the log, with the values of secret-looking options hidden.
fn redact_args(args: impl Iterator<Item = String>) -> Vec<String> {
    let secret = |a: &str| ["pass", "token", "secret", "key", "password"].iter().any(|k| a.starts_with("--") && a.to_ascii_lowercase().contains(k)) && !a.ends_with("-env");
    let mut out = Vec::new();
    let mut hide_next = false;
    for a in args {
        if hide_next {
            out.push("***".into());
            hide_next = false;
        } else if let Some((flag, _)) = a.split_once('=').filter(|(f, _)| secret(f)) {
            out.push(format!("{flag}=***"));
        } else {
            hide_next = secret(&a);
            out.push(a);
        }
    }
    out
}

fn init_logging(verbose: u8, level: Option<&str>, file: Option<&std::path::Path>, file_level: &str) -> Result<(), String> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
    let console = match level {
        Some(l) => parse_level(l)?.to_string(),
        None => match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
        .to_string(),
    };
    let console_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(console));
    let file_layer = match file {
        Some(path) => {
            let lvl = parse_level(file_level)?;
            if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(d).map_err(|e| format!("log file {}: {e}", path.display()))?;
            }
            let f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("log file {}: {e}", path.display()))?;
            Some(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(std::sync::Mutex::new(f)).with_filter(lvl))
        }
        None => None,
    };
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_filter(console_filter))
        .with(file_layer)
        .init();
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    // Machine-wide defaults (arguments and environment) from the preload file.
    let preload = ufop_core::preload::Preload::load();
    if let Some(p) = &preload {
        p.apply_env("ufop");
    }
    let argv: Vec<String> = std::env::args().collect();
    let cli = Cli::parse_from(match &preload {
        Some(p) => p.merge_args("ufop", argv),
        None => argv,
    });

    // Console logging from --log-level / -v (RUST_LOG wins); optional log file.
    if let Err(e) = init_logging(cli.verbose, cli.log_level.as_deref(), cli.log_file.as_deref(), &cli.log_file_level) {
        eprintln!("error: {e}");
        return ExitCode::from(2);
    }

    tracing::info!(target: "ufop", "ufop {} started", redact_args(std::env::args().skip(1)).join(" "));
    let format = cli.effective_format();
    let dry_run = cli.dry_run;
    let rate_limit = cli.rate_limit_bps();

    let result = match cli.command {
        Commands::Login { token, server } => {
            commands::login::execute(token, server, &format).await
        }
        Commands::Connection(cmd) => {
            commands::connection::execute(cmd, &format, dry_run).await
        }
        Commands::Transfer {
            src, dest, overwrite, resume, verify, parallel,
        } => {
            commands::transfer::execute(
                src, dest, overwrite, resume, verify, parallel,
                &format, dry_run, rate_limit,
            ).await
        }
        Commands::Copy { args } => {
            commands::copy::execute(args, &format, dry_run, rate_limit).await
        }
        Commands::Sync(cmd) => {
            commands::sync::execute(cmd, &format, dry_run).await
        }
        Commands::Server(cmd) => {
            commands::server::execute(cmd, &format).await
        }
        Commands::Backup(cmd) => {
            commands::backup::execute(cmd, &format, dry_run).await
        }
        Commands::Agent(cmd) => {
            commands::agent::execute(cmd, &format).await
        }
        Commands::Compat { paths, targets } => {
            commands::compat::execute(paths, targets, &format).await
        }
        Commands::Checksum { paths, algorithm, verify } => {
            commands::checksum::execute(paths, algorithm, verify, &format).await
        }
        Commands::Duplicates { path, min_size, action } => {
            commands::duplicates::execute(path, min_size, action, &format, dry_run).await
        }
        Commands::Rename {
            path, pattern, find, replace, regex, case,
            start, step, pad,
        } => {
            commands::rename::execute(
                path, pattern, find, replace, regex, case,
                start, step, pad, &format, dry_run,
            ).await
        }
        Commands::Archive(cmd) => {
            commands::archive::execute(cmd, &format, dry_run).await
        }
        Commands::Status => {
            commands::status::execute(&format).await
        }
    };

    match result {
        Ok(code) => {
            tracing::info!(target: "ufop", "finished with exit code {code}");
            ExitCode::from(code)
        }
        Err(e) => {
            tracing::error!(target: "ufop", "{e}");
            output::print_error(&format, &e.to_string());
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod logging_tests {
    use super::*;

    #[test]
    fn redacts_secret_options_and_parses_levels() {
        let a = ["backup", "run", "--passphrase", "pw", "--token=abc", "--passphrase-env", "VAR", "--dest", "/d"].map(String::from);
        assert_eq!(redact_args(a.into_iter()).join(" "), "backup run --passphrase *** --token=*** --passphrase-env VAR --dest /d");
        assert!(parse_level("WARNING").is_ok() && parse_level("nope").is_err());
    }
}
