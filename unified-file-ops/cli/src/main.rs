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

    /// Manage sync pairs
    #[command(subcommand)]
    Sync(SyncCommands),

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

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Initialize tracing based on verbosity
    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .init();

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
        Commands::Sync(cmd) => {
            commands::sync::execute(cmd, &format, dry_run).await
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
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            output::print_error(&format, &e.to_string());
            ExitCode::from(2)
        }
    }
}
