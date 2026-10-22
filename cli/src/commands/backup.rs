//! `ufop backup …` — versioned, deduplicated, encrypted backups using the
//! same engine as the desktop app, without the app running.
//!
//! Destinations are folders (local disks, removable media, mounted shares).
//! The local database (`--dbpath`) is a cache; `ufop backup recreate`
//! rebuilds it from the destination when lost.

use crate::output;
use crate::OutputFormat;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use ufop_core::backup::crypto::KdfParams;
use ufop_core::backup::db::BackupDb;
use ufop_core::backup::engine::Repo;
use ufop_core::backup::restore::ConflictPolicy;
use ufop_core::backup::{
    backend, maintenance, run_backup, run_restore, BackupOptions, FilterAction, FilterKind, FilterRule, LocalBackend, OpContext,
    RestoreOptions, RetentionPolicy, SourceFilter,
};

#[derive(Args, Debug, Clone)]
pub struct RepoArgs {
    /// Destination folder (local disk, removable drive or mounted share)
    #[arg(long)]
    pub dest: String,
    /// Local database file (default: <dest-hash>.sqlite in the user data dir)
    #[arg(long)]
    pub dbpath: Option<PathBuf>,
    /// Passphrase (prefer --passphrase-env)
    #[arg(long, conflicts_with = "passphrase_env")]
    pub passphrase: Option<String>,
    /// Environment variable holding the passphrase
    #[arg(long, default_value = "UFOP_PASSPHRASE")]
    pub passphrase_env: String,
    /// File whose first line is the passphrase
    #[arg(long, conflicts_with = "passphrase")]
    pub passphrase_file: Option<PathBuf>,
    /// Secret provider for `$name` passphrases: env://PREFIX, file:///path or pass://folder (repeatable)
    #[arg(long = "secret-provider", env = "UFOP_SECRET_PROVIDER", value_delimiter = ',')]
    pub secret_providers: Vec<String>,
    /// Store volumes unencrypted
    #[arg(long)]
    pub no_encryption: bool,
    /// Remote file-name prefix
    #[arg(long, default_value = "ufop")]
    pub prefix: String,
}

/// Report a configuration problem and exit (exit code 2, like other usage errors).
fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

impl RepoArgs {
    fn passphrase(&self) -> Option<String> {
        if self.no_encryption {
            return None;
        }
        let raw = match &self.passphrase_file {
            Some(f) => match std::fs::read_to_string(f) {
                Ok(t) => t.lines().next().map(|l| l.trim_end_matches('\r').to_string()),
                Err(e) => fail(&format!("cannot read --passphrase-file {}: {e}", f.display())),
            },
            None => self.passphrase.clone().or_else(|| std::env::var(&self.passphrase_env).ok()),
        };
        raw.filter(|p| !p.is_empty()).map(|p| crate::secrets::resolve(&p, &self.secret_providers).unwrap_or_else(|e| fail(&e)))
    }

    fn db_path(&self) -> PathBuf {
        self.dbpath.clone().unwrap_or_else(|| {
            let h = ufop_core::backup::hash_file_name_key(&format!("{}|{}", self.dest, self.prefix));
            dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("ufop").join("cli-backups").join(format!("{h}.sqlite"))
        })
    }

    fn options(&self) -> BackupOptions {
        BackupOptions { prefix: self.prefix.clone(), ..Default::default() }
    }
}

/// Engine options beyond the basics (all optional).
#[derive(Args, Debug, Clone, Default)]
pub struct EngineArgs {
    /// Volume container: ufop | zip (fixed after the first backup)
    #[arg(long, default_value = "ufop")]
    pub volume_format: String,
    /// ZIP compression: deflate | stored | bzip2 | zstd
    #[arg(long, default_value = "deflate")]
    pub zip_method: String,
    /// Upload PAR2 recovery files (needs the par2 tool)
    #[arg(long)]
    pub par2: bool,
    #[arg(long, default_value_t = 10)]
    pub par2_redundancy: u8,
    /// Only rescan these paths (repeatable); everything else is taken from the previous version
    #[arg(long = "changed-path")]
    pub changed_paths: Vec<String>,
    /// Paths known to be deleted since the previous version (repeatable)
    #[arg(long = "deleted-path")]
    pub deleted_paths: Vec<String>,
    /// NTFS change journal: off | auto | on | required
    #[arg(long, default_value = "off")]
    pub usn_policy: String,
    /// File stored with each version (repeatable)
    #[arg(long = "control-file")]
    pub control_files: Vec<String>,
    /// Hashing threads (0 = auto)
    #[arg(long, default_value_t = 0)]
    pub hash_threads: usize,
    #[arg(long, default_value_t = 2)]
    pub upload_concurrency: usize,
    /// 1–10; lower values pause between blocks to leave CPU for other work
    #[arg(long, default_value_t = 10)]
    pub cpu_intensity: u8,
    #[arg(long)]
    pub tempdir: Option<String>,
    /// Read buffer in bytes (0 = default)
    #[arg(long, default_value_t = 0)]
    pub read_buffer_size: usize,
    /// Low-priority disk and CPU use
    #[arg(long)]
    pub background_io: bool,
    /// Quota in bytes for capacity warnings (default: destination free space)
    #[arg(long)]
    pub quota_size: Option<u64>,
    #[arg(long, default_value_t = 10)]
    pub quota_warning_percent: u8,
    #[arg(long)]
    pub no_quota_warning: bool,
    /// Skip files with an advisory lock held by another program
    #[arg(long)]
    pub respect_advisory_locks: bool,
    #[arg(long)]
    pub no_xattrs: bool,
    #[arg(long)]
    pub no_acls: bool,
    /// Back up NTFS alternate data streams
    #[arg(long)]
    pub alternate_streams: bool,
    /// Back up items marked with exclusion attributes anyway
    #[arg(long)]
    pub ignore_exclusion_xattrs: bool,
    /// The destination ignores name case
    #[arg(long)]
    pub case_insensitive_remote: bool,
    #[arg(long)]
    pub compact_interval_days: Option<u32>,
    #[arg(long)]
    pub vacuum_interval_days: Option<u32>,
    /// Snapshot sources first: off | auto | on | required (VSS / LVM / APFS)
    #[arg(long, default_value = "off")]
    pub snapshot_policy: String,
    /// LVM snapshot size (`10%ORIGIN`, `2G`)
    #[arg(long, default_value = "10%ORIGIN")]
    pub lvm_snapshot_size: String,
    /// VSS provider GUID
    #[arg(long)]
    pub vss_provider: Option<String>,
    /// VSS writer class GUID to exclude (repeatable)
    #[arg(long = "vss-exclude-writer")]
    pub vss_exclude_writers: Vec<String>,
    /// Volume encryption for a new backup: aes | gpg
    #[arg(long, default_value = "aes")]
    pub encryption_module: String,
    /// GPG: encrypt to this key (repeatable; none = symmetric with the passphrase)
    #[arg(long = "gpg-recipient")]
    pub gpg_recipients: Vec<String>,
    /// GPG keyring folder (default: $GNUPGHOME)
    #[arg(long, default_value = "")]
    pub gpg_homedir: String,
    /// gpg executable (default: $UFOP_GPG or gpg)
    #[arg(long, default_value = "")]
    pub gpg_executable: String,
    /// Argon2id cost for a new AES backup: interactive | default | sensitive
    #[arg(long)]
    pub kdf_preset: Option<String>,
    /// Argon2id memory in KiB (overrides the preset)
    #[arg(long)]
    pub kdf_memory_kib: Option<u32>,
    #[arg(long)]
    pub kdf_iterations: Option<u32>,
    #[arg(long)]
    pub kdf_parallelism: Option<u32>,
    /// Photos sources (`--source photos:///…/Lib.photoslibrary`): include hidden assets
    #[arg(long)]
    pub photos_include_hidden: bool,
    /// Photos sources: include assets in Recently Deleted
    #[arg(long)]
    pub photos_include_trashed: bool,
    /// Photos sources: only assets in this album (repeatable; title, case-insensitive)
    #[arg(long = "photos-album")]
    pub photos_albums: Vec<String>,
}

fn parse_enum<T: serde::de::DeserializeOwned>(flag: &str, v: &str) -> Result<T, ufop_core::error::AppError> {
    serde_json::from_value(serde_json::Value::String(v.to_ascii_lowercase()))
        .map_err(|_| ufop_core::error::AppError::validation(format!("Invalid value '{v}' for --{flag}")))
}

impl EngineArgs {
    fn kdf(&self) -> Result<Option<KdfParams>, ufop_core::error::AppError> {
        let base = match self.kdf_preset.as_deref() {
            Some(p) => Some(KdfParams::preset(p).ok_or_else(|| ufop_core::error::AppError::validation(format!("Unknown --kdf-preset '{p}' (interactive, default, sensitive)")))?),
            None => None,
        };
        if base.is_none() && self.kdf_memory_kib.is_none() && self.kdf_iterations.is_none() && self.kdf_parallelism.is_none() {
            return Ok(None);
        }
        let b = base.unwrap_or_default();
        let k = KdfParams { memory_kib: self.kdf_memory_kib.unwrap_or(b.memory_kib), iterations: self.kdf_iterations.unwrap_or(b.iterations), parallelism: self.kdf_parallelism.unwrap_or(b.parallelism) };
        k.validate()?;
        Ok(Some(k))
    }

    fn snapshot_options(&self) -> ufop_core::backup::snapshot::SnapshotOptions {
        ufop_core::backup::snapshot::SnapshotOptions {
            lvm_size: self.lvm_snapshot_size.clone(),
            vss_provider: self.vss_provider.clone(),
            vss_exclude_writers: self.vss_exclude_writers.clone(),
            mount_root: None,
        }
    }

    fn apply(&self, o: BackupOptions) -> Result<BackupOptions, ufop_core::error::AppError> {
        Ok(BackupOptions {
            volume_format: parse_enum("volume-format", &self.volume_format)?,
            zip_method: parse_enum("zip-method", &self.zip_method)?,
            par2: ufop_core::backup::Par2Options { enabled: self.par2, redundancy_percent: self.par2_redundancy.clamp(1, 100), ..Default::default() },
            changed_paths: (!self.changed_paths.is_empty()).then(|| self.changed_paths.clone()),
            deleted_paths: self.deleted_paths.clone(),
            usn_policy: parse_enum("usn-policy", &self.usn_policy)?,
            control_files: self.control_files.clone(),
            hash_threads: self.hash_threads,
            upload_concurrency: self.upload_concurrency.max(1),
            cpu_intensity: self.cpu_intensity.clamp(1, 10),
            temp_dir: self.tempdir.clone(),
            read_buffer_size: self.read_buffer_size,
            background_io: self.background_io,
            quota_size: self.quota_size,
            quota_warning_percent: self.quota_warning_percent,
            quota_disable: self.no_quota_warning,
            respect_advisory_locks: self.respect_advisory_locks,
            backup_xattrs: !self.no_xattrs,
            backup_acls: !self.no_acls,
            backup_alternate_streams: self.alternate_streams,
            ignore_exclusion_xattrs: self.ignore_exclusion_xattrs,
            remote_case_insensitive: self.case_insensitive_remote,
            auto_compact_interval_days: self.compact_interval_days,
            auto_vacuum_interval_days: self.vacuum_interval_days,
            snapshot_policy: parse_enum("snapshot-policy", &self.snapshot_policy)?,
            snapshot: self.snapshot_options(),
            encryption_module: parse_enum("encryption-module", &self.encryption_module)?,
            gpg: ufop_core::backup::GpgOptions { recipients: self.gpg_recipients.clone(), executable: self.gpg_executable.clone(), homedir: self.gpg_homedir.clone() },
            kdf: self.kdf()?,
            photos: ufop_core::backup::photos::PhotosOptions {
                include_hidden: self.photos_include_hidden,
                include_trashed: self.photos_include_trashed,
                albums: self.photos_albums.clone(),
            },
            ..o
        })
    }
}

/// Passphrase sources for `crypt` commands.
#[derive(Args, Debug, Clone)]
pub struct PassArgs {
    /// Environment variable holding the passphrase
    #[arg(long, default_value = "UFOP_PASSPHRASE")]
    pub passphrase_env: String,
    /// File whose first line is the passphrase
    #[arg(long)]
    pub passphrase_file: Option<PathBuf>,
    /// Secret provider for `$name` passphrases (env://, file://, pass://)
    #[arg(long = "secret-provider", env = "UFOP_SECRET_PROVIDER", value_delimiter = ',')]
    pub secret_providers: Vec<String>,
}

impl PassArgs {
    fn get(&self) -> Option<String> {
        RepoArgs {
            dest: String::new(),
            dbpath: None,
            passphrase: None,
            passphrase_env: self.passphrase_env.clone(),
            passphrase_file: self.passphrase_file.clone(),
            secret_providers: self.secret_providers.clone(),
            no_encryption: false,
            prefix: String::new(),
        }
        .passphrase()
    }
}

/// Standalone encryption (`.ufopenc`) and manual volume decryption.
#[derive(Subcommand, Debug)]
pub enum CryptCommands {
    /// Encrypt a file (AES-256-GCM, Argon2id key, authenticated 4 MiB chunks)
    Encrypt {
        input: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        pass: PassArgs,
        /// interactive | default | sensitive
        #[arg(long, default_value = "default")]
        kdf_preset: String,
    },
    /// Decrypt a .ufopenc file
    Decrypt {
        input: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        pass: PassArgs,
    },
    /// Decrypt one backup volume with its repository header (<prefix>-repo.json)
    DecryptVolume {
        volume: PathBuf,
        /// The repository header file from the destination
        #[arg(long)]
        header: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        pass: PassArgs,
    },
}

/// Recovery tool steps (work without the job database).
#[derive(Subcommand, Debug)]
pub enum RecoveryCommands {
    /// Download and decrypt every volume into a local folder (resumable)
    Download {
        #[command(flatten)]
        repo: RepoArgs,
        /// Local folder for the decrypted copy (keep it private)
        folder: PathBuf,
        #[arg(long, default_value_t = 4)]
        threads: usize,
        /// Download everything again instead of continuing
        #[arg(long)]
        no_resume: bool,
        /// Download limit in KB/s (0 = unlimited)
        #[arg(long, default_value_t = 0)]
        max_kbps: u64,
    },
    /// Index the downloaded block volumes
    Index { folder: PathBuf },
    /// List versions in the downloaded copy
    List { folder: PathBuf },
    /// Rebuild files from the downloaded copy (gaps zero-filled and reported)
    Restore {
        folder: PathBuf,
        target: PathBuf,
        #[arg(long, default_value_t = 0)]
        version: usize,
        /// Paths/folders/wildcards (default: everything)
        paths: Vec<String>,
    },
    /// Write the copy into a new repository with another format or passphrase
    Convert {
        folder: PathBuf,
        /// Destination folder of the new repository
        #[arg(long)]
        out: String,
        #[arg(long, default_value = "ufop")]
        prefix: String,
        #[arg(long, default_value = "ufop")]
        volume_format: String,
        #[arg(long, default_value = "deflate")]
        zip_method: String,
        #[arg(long, default_value_t = 6)]
        compression_level: u32,
        /// Environment variable with the new passphrase (unset = unencrypted)
        #[arg(long, default_value = "UFOP_NEW_PASSPHRASE")]
        new_passphrase_env: String,
    },
}

/// Local job-database tools.
#[derive(Subcommand, Debug)]
pub enum DbCommands {
    /// Schema version, counts, size and integrity of a job database
    Info { dbpath: PathBuf },
    /// Upgrade a job database to the current schema (copy kept as .v<N>.bak)
    Migrate {
        dbpath: PathBuf,
        #[arg(long)]
        no_backup: bool,
    },
    /// Reclaim free space in a job database
    Vacuum { dbpath: PathBuf },
    /// Integrity and reference checks (exit 2 when problems are found)
    Check { dbpath: PathBuf },
    /// Copy for a bug report: paths hashed, job-config snapshots removed
    Sanitize { dbpath: PathBuf, out: PathBuf },
}

#[derive(Subcommand, Debug)]
pub enum BackupCommands {
    /// Back up sources into the destination (creates it on first run)
    Run {
        #[command(flatten)]
        repo: RepoArgs,
        /// Source folder or file, or a macOS Photos library as
        /// `photos:///path/Lib.photoslibrary` (repeatable)
        #[arg(long = "source", required = true)]
        sources: Vec<String>,
        /// Include filter: path, wildcard, [regex] or {group} (ordered, repeatable)
        #[arg(long)]
        include: Vec<String>,
        /// Exclude filter: path, wildcard, [regex] or {group} (ordered, repeatable)
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        exclude_hidden: bool,
        #[arg(long)]
        skip_files_larger_than: Option<u64>,
        #[arg(long, default_value_t = 100 * 1024)]
        block_size: usize,
        #[arg(long, default_value_t = 50 * 1024 * 1024)]
        volume_size: usize,
        #[arg(long, default_value_t = 6)]
        compression_level: u32,
        #[arg(long)]
        keep_versions: Option<u32>,
        #[arg(long)]
        keep_days: Option<u32>,
        /// `smart` or custom rules like `1W:1D,4W:1W,12M:1M`
        #[arg(long)]
        retention_policy: Option<String>,
        /// Volumes to sample-verify after the backup
        #[arg(long, default_value_t = 1)]
        verify_samples: usize,
        #[arg(long)]
        upload_unchanged: bool,
        #[arg(long)]
        no_auto_compact: bool,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// List backup versions
    List {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Search backed-up paths (substring or wildcard)
    Find {
        #[command(flatten)]
        repo: RepoArgs,
        pattern: String,
        /// Only this version (0 = newest); default: all versions
        #[arg(long)]
        version: Option<usize>,
    },
    /// Compare two versions
    Compare {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(default_value_t = 1)]
        base: usize,
        #[arg(default_value_t = 0)]
        compare: usize,
    },
    /// Restore files
    Restore {
        #[command(flatten)]
        repo: RepoArgs,
        /// Paths/folders/wildcards to restore (default: everything)
        paths: Vec<String>,
        #[arg(long, default_value_t = 0)]
        version: usize,
        /// Restore below this folder instead of the original location
        #[arg(long)]
        target: Option<PathBuf>,
        /// overwrite | rename | skip
        #[arg(long, default_value = "overwrite")]
        conflict: String,
        #[arg(long)]
        restore_permissions: bool,
        #[arg(long)]
        preserve_full_paths: bool,
        #[arg(long)]
        skip_verify: bool,
        /// Restore these versions side by side below --target (repeatable)
        #[arg(long = "versions")]
        versions: Vec<usize>,
        /// With --versions: skip files identical to a newer restored version
        #[arg(long)]
        unique: bool,
        /// Take matching blocks from files on this computer before downloading
        #[arg(long)]
        use_local_blocks: bool,
        #[arg(long, default_value_t = 0)]
        download_threads: usize,
        #[arg(long, default_value_t = 0)]
        file_threads: usize,
        /// Sequential single-threaded restore
        #[arg(long)]
        legacy: bool,
        #[arg(long)]
        tempdir: Option<PathBuf>,
        #[arg(long)]
        skip_free_space_check: bool,
        /// Do not re-apply extended attributes / Windows attributes
        #[arg(long)]
        no_metadata: bool,
    },
    /// Rebuild missing block volumes from the source files on this computer
    RebuildMissing {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Extract a version's control files into a folder
    ControlFiles {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(long)]
        target: PathBuf,
        #[arg(long, default_value_t = 0)]
        version: usize,
    },
    /// Mirror a backup to another folder (a complete, restorable copy)
    Mirror {
        /// Source destination folder
        #[arg(long)]
        dest: String,
        /// Target folder for the copy
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "ufop")]
        prefix: String,
        /// Keep files the source no longer has
        #[arg(long)]
        keep_extraneous: bool,
        /// Read back and verify every copied file
        #[arg(long)]
        verify: bool,
        #[arg(long, default_value_t = 2)]
        threads: usize,
    },
    /// Salvage data without the job database (download, index, list, restore, convert)
    Recovery {
        #[command(subcommand)]
        cmd: RecoveryCommands,
    },
    /// Job-database tools (info, migrate, vacuum)
    Db {
        #[command(subcommand)]
        cmd: DbCommands,
    },
    /// Encrypt or decrypt files with the backup cipher; decrypt single volumes
    Crypt {
        #[command(subcommand)]
        cmd: CryptCommands,
    },
    /// Create and remove a test snapshot of the sources, listing every step
    SnapshotTest {
        #[arg(long = "source", required = true)]
        sources: Vec<String>,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Benchmark restore download concurrency on sample volumes
    AutoTune {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(long, default_value_t = 8)]
        samples: usize,
    },
    /// Download and check volumes
    Verify {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(long)]
        full: bool,
        #[arg(long)]
        samples: Option<usize>,
    },
    /// Remove unused data and repack sparse volumes
    Compact {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Re-upload missing index/file-list volumes from the local database
    Repair {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Rebuild the local database from the destination
    Recreate {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// List files that cannot be fully restored
    ListBrokenFiles {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Remove unrecoverable files from history
    PurgeBrokenFiles {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Remove paths from history
    Purge {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Delete versions by index
    Delete {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(required = true)]
        versions: Vec<usize>,
        #[arg(long)]
        allow_full_removal: bool,
    },
    /// Randomized upload/download/hash test of a destination folder
    TestDestination {
        #[arg(long)]
        dest: String,
        #[arg(long, default_value_t = 5)]
        files: usize,
    },
}

pub fn parse_filter(spec: &str, action: FilterAction) -> FilterRule {
    let (kind, pattern) = if let Some(r) = spec.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        (FilterKind::Regex, r.to_string())
    } else if let Some(g) = spec.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        (FilterKind::Group, g.to_string())
    } else if spec.contains('*') || spec.contains('?') {
        (FilterKind::Wildcard, spec.to_string())
    } else {
        (FilterKind::Exact, spec.to_string())
    };
    FilterRule { action, kind, pattern }
}

pub fn parse_retention(keep_versions: Option<u32>, keep_days: Option<u32>, policy: Option<String>) -> RetentionPolicy {
    match (keep_versions, keep_days, policy) {
        (Some(c), _, _) => RetentionPolicy::KeepCount { count: c },
        (_, Some(d), _) => RetentionPolicy::KeepDays { days: d },
        (_, _, Some(p)) if p.eq_ignore_ascii_case("smart") => RetentionPolicy::Smart,
        (_, _, Some(p)) => RetentionPolicy::Custom { rules: p },
        _ => RetentionPolicy::KeepAll,
    }
}

fn progress_ctx(format: &OutputFormat) -> OpContext {
    let ctx = OpContext::new();
    let human = matches!(format, OutputFormat::Human);
    if human {
        cancel_on_ctrl_c(ctx.clone());
    }
    ctx.with_progress(move |p| {
        if let Some(path) = &p.current_path {
            tracing::trace!(phase = ?p.phase, done = p.files_processed, total = p.files_total, "{path}");
            if human {
                eprint!("\r\x1b[2K{:?} {}/{} {}", p.phase, p.files_processed, p.files_total, truncate(path, 70));
            }
        }
    })
}

/// Send an operation's warnings and errors to the console/file log.
fn log_outcome(op: &str, warnings: &[String], errors: &[String]) {
    for w in warnings {
        tracing::warn!(target: "ufop::backup", "{op}: {w}");
    }
    for e in errors {
        tracing::error!(target: "ufop::backup", "{op}: {e}");
    }
    tracing::info!(target: "ufop::backup", "{op} finished: {} warning(s), {} error(s)", warnings.len(), errors.len());
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("…{}", s.chars().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect::<String>()) }
}

/// Cancel cleanly on Ctrl-C: the engine writes a partial version.
fn cancel_on_ctrl_c(ctx: OpContext) {
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        h.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("\nCancelling…");
                ctx.cancel();
            }
        });
    }
}

fn emit<T: serde::Serialize>(format: &OutputFormat, title: &str, v: &T) -> Result<(), Box<dyn std::error::Error>> {
    match format {
        OutputFormat::Human => {
            eprint!("\r\x1b[2K");
            println!("{title}");
            println!("{}", serde_json::to_string_pretty(v)?);
        }
        _ => output::print_data(format, v),
    }
    Ok(())
}

fn with_repo<T>(
    repo: &RepoArgs,
    f: impl FnOnce(&Repo, &BackupDb) -> Result<T, ufop_core::error::AppError>,
) -> Result<T, ufop_core::error::AppError> {
    let be = LocalBackend::new(&repo.dest);
    let db = BackupDb::open(&repo.db_path())?;
    let pass = repo.passphrase();
    let r = Repo::open(&be, &db, pass.as_deref(), &repo.options(), false, KdfParams::default())?;
    f(&r, &db)
}

pub async fn execute(cmd: BackupCommands, format: &OutputFormat, dry_run: bool) -> Result<u8, Box<dyn std::error::Error>> {
    let fmt = format.clone();
    let res = tokio::task::spawn_blocking(move || run(cmd, &fmt, dry_run)).await?;
    match res {
        Ok(code) => Ok(code),
        Err(e) => {
            output::print_error(format, &e.user_message());
            Ok(2)
        }
    }
}

fn run(cmd: BackupCommands, format: &OutputFormat, dry_run: bool) -> Result<u8, ufop_core::error::AppError> {
    let ok = |v: Result<(), Box<dyn std::error::Error>>| v.map_err(|e| ufop_core::error::AppError::internal(e.to_string()));
    match cmd {
        BackupCommands::Run {
            repo, sources, include, exclude, exclude_hidden, skip_files_larger_than, block_size, volume_size, compression_level,
            keep_versions, keep_days, retention_policy, verify_samples, upload_unchanged, no_auto_compact, engine,
        } => {
            let pass = repo.passphrase();
            if pass.is_none() && !repo.no_encryption {
                return Err(ufop_core::error::AppError::validation(format!(
                    "No passphrase: set {} (or pass --no-encryption)",
                    repo.passphrase_env
                )));
            }
            let mut rules: Vec<FilterRule> = include.iter().map(|s| parse_filter(s, FilterAction::Include)).collect();
            rules.extend(exclude.iter().map(|s| parse_filter(s, FilterAction::Exclude)));
            let mut filter = SourceFilter::default();
            filter.rules = rules;
            filter.exclude_hidden = exclude_hidden;
            filter.max_file_size = skip_files_larger_than;
            let filter = filter.compile()?;
            let opts = engine.apply(BackupOptions {
                block_size,
                volume_size,
                compression_level,
                verify_samples,
                upload_unchanged_backups: upload_unchanged,
                auto_compact: !no_auto_compact,
                dry_run,
                ..repo.options()
            })?;
            let retention = parse_retention(keep_versions, keep_days, retention_policy);
            let be = LocalBackend::new(&repo.dest);
            let db = BackupDb::open(&repo.db_path())?;
            let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
            let r = run_backup(&be, &db, &srcs, &filter, &opts, &retention, pass.as_deref(), KdfParams::default(), &progress_ctx(format))?;
            log_outcome("backup", &r.warnings, &r.errors);
            ok(emit(format, "Backup finished", &r))?;
            Ok(if !r.errors.is_empty() || r.partial { 2 } else if !r.warnings.is_empty() { 1 } else { 0 })
        }
        BackupCommands::List { repo } => with_repo(&repo, |_, db| {
            let v = db.filesets()?;
            ok(emit(format, "Versions (0 = newest)", &v))?;
            Ok(0)
        }),
        BackupCommands::Find { repo, pattern, version } => with_repo(&repo, |_, db| {
            ok(emit(format, "Matches", &maintenance::find(db, &pattern, version, 10_000)?))?;
            Ok(0)
        }),
        BackupCommands::Compare { repo, base, compare } => with_repo(&repo, |_, db| {
            ok(emit(format, "Differences", &maintenance::compare_versions(db, base, compare)?))?;
            Ok(0)
        }),
        BackupCommands::Restore {
            repo, paths, version, target, conflict, restore_permissions, preserve_full_paths, skip_verify, versions, unique, use_local_blocks,
            download_threads, file_threads, legacy, tempdir, skip_free_space_check, no_metadata,
        } => {
            let conflict = match conflict.as_str() {
                "rename" => ConflictPolicy::Rename,
                "skip" => ConflictPolicy::Skip,
                _ => ConflictPolicy::Overwrite,
            };
            let opts = RestoreOptions {
                version, versions, unique_versions: unique, paths, target, conflict, restore_permissions, preserve_full_paths, skip_verify, dry_run,
                use_local_blocks, download_threads, file_threads, legacy, temp_dir: tempdir, skip_free_space_check, restore_metadata: !no_metadata,
                ..Default::default()
            };
            // No local database? Rebuild a temporary one from the destination.
            let needs_recreate = !repo.db_path().exists();
            let be = LocalBackend::new(&repo.dest);
            let pass = repo.passphrase();
            let db = if needs_recreate { BackupDb::open_in_memory()? } else { BackupDb::open(&repo.db_path())? };
            if needs_recreate {
                maintenance::recreate_database(&be, &db, pass.as_deref(), &repo.prefix, &OpContext::new())?;
            }
            let r = Repo::open(&be, &db, pass.as_deref(), &repo.options(), false, KdfParams::default())?;
            let rep = run_restore(&r, &opts, &progress_ctx(format))?;
            log_outcome("restore", &rep.warnings, &rep.errors);
            ok(emit(format, "Restore finished", &rep))?;
            Ok(if rep.errors.is_empty() { 0 } else { 1 })
        }
        BackupCommands::Verify { repo, full, samples } => with_repo(&repo, |r, _| {
            let v = maintenance::verify(r, if full { None } else { Some(samples.unwrap_or(3)) }, full)?;
            ok(emit(format, "Verification", &v))?;
            Ok(if v.errors.is_empty() { 0 } else { 2 })
        }),
        BackupCommands::Compact { repo } => with_repo(&repo, |r, _| {
            ok(emit(format, "Compaction", &maintenance::compact(r, &repo.options(), &OpContext::new())?))?;
            Ok(0)
        }),
        BackupCommands::Repair { repo } => with_repo(&repo, |r, _| {
            let rep = maintenance::repair(r)?;
            ok(emit(format, "Repair", &rep))?;
            Ok(if rep.unrecoverable.is_empty() { 0 } else { 1 })
        }),
        BackupCommands::Recreate { repo } => {
            let be = LocalBackend::new(&repo.dest);
            let path = repo.db_path();
            let db = BackupDb::open(&path)?;
            let rep = maintenance::recreate_database(&be, &db, repo.passphrase().as_deref(), &repo.prefix, &OpContext::new())?;
            ok(emit(format, &format!("Database recreated at {}", path.display()), &rep))?;
            Ok(0)
        }
        BackupCommands::ListBrokenFiles { repo } => with_repo(&repo, |r, _| {
            let b = maintenance::list_broken_files(r)?;
            ok(emit(format, "Broken files", &b))?;
            Ok(if b.is_empty() { 0 } else { 1 })
        }),
        BackupCommands::PurgeBrokenFiles { repo } => with_repo(&repo, |r, _| {
            ok(emit(format, "Purge broken files", &maintenance::purge_broken_files(r, dry_run)?))?;
            Ok(0)
        }),
        BackupCommands::Purge { repo, paths } => with_repo(&repo, |r, _| {
            ok(emit(format, "Purge", &maintenance::purge(r, &paths, None, dry_run)?))?;
            Ok(0)
        }),
        BackupCommands::Delete { repo, versions, allow_full_removal } => with_repo(&repo, |r, _| {
            let n = maintenance::delete_versions(r, &versions, allow_full_removal)?;
            ok(emit(format, "Deleted versions", &serde_json::json!({ "versions_deleted": n })))?;
            Ok(0)
        }),
        BackupCommands::RebuildMissing { repo } => with_repo(&repo, |r, _| {
            let rep = maintenance::rebuild_missing_block_volumes(r, &repo.options(), &progress_ctx(format))?;
            ok(emit(format, "Rebuild missing volumes", &rep))?;
            Ok(if rep.incomplete_volumes.is_empty() { 0 } else { 1 })
        }),
        BackupCommands::ControlFiles { repo, target, version } => with_repo(&repo, |r, _| {
            ok(emit(format, "Control files", &maintenance::extract_control_files(r, version, &target)?))?;
            Ok(0)
        }),
        BackupCommands::Mirror { dest, to, prefix, keep_extraneous, verify, threads } => {
            let o = ufop_core::backup::mirror::MirrorOptions { delete_extraneous: !keep_extraneous, verify, dry_run, threads };
            let r = ufop_core::backup::mirror::mirror(&LocalBackend::new(&dest), &LocalBackend::new(&to), &prefix, &o, &progress_ctx(format))?;
            ok(emit(format, "Mirror", &r))?;
            Ok(if r.errors.is_empty() { 0 } else { 2 })
        }
        BackupCommands::Recovery { cmd } => {
            use ufop_core::backup::recovery;
            let ctx = progress_ctx(format);
            match cmd {
                RecoveryCommands::Download { repo, folder, threads, no_resume, max_kbps } => {
                    let o = recovery::DownloadOptions { threads, resume: !no_resume, max_bytes_per_sec: max_kbps * 1024 };
                    let r = recovery::download(&LocalBackend::new(&repo.dest), repo.passphrase().as_deref(), &repo.prefix, &folder, &o, &ctx)?;
                    ok(emit(format, "Recovery download", &r))?;
                    Ok(if r.failed.is_empty() { 0 } else { 1 })
                }
                RecoveryCommands::Index { folder } => {
                    let r = recovery::build_index(&folder, &ctx)?;
                    ok(emit(format, "Recovery index", &r))?;
                    Ok(if r.damaged.is_empty() { 0 } else { 1 })
                }
                RecoveryCommands::List { folder } => {
                    ok(emit(format, "Recovered versions (0 = newest)", &recovery::list_versions(&folder)?))?;
                    Ok(0)
                }
                RecoveryCommands::Restore { folder, target, version, paths } => {
                    let r = recovery::restore(&folder, version, &paths, &target, &ctx)?;
                    ok(emit(format, "Recovery restore", &r))?;
                    Ok(if r.files_partial.is_empty() && r.errors.is_empty() { 0 } else { 1 })
                }
                RecoveryCommands::Convert { folder, out, prefix, volume_format, zip_method, compression_level, new_passphrase_env } => {
                    let o = recovery::ConvertOptions {
                        prefix,
                        volume_format: parse_enum("volume-format", &volume_format)?,
                        zip_method: parse_enum("zip-method", &zip_method)?,
                        compression_level,
                        passphrase: std::env::var(&new_passphrase_env).ok().filter(|p| !p.is_empty()),
                        kdf: KdfParams::default(),
                    };
                    ok(emit(format, "Converted repository", &recovery::convert(&folder, &LocalBackend::new(&out), &o, &ctx)?))?;
                    Ok(0)
                }
            }
        }
        BackupCommands::Crypt { cmd } => {
            use ufop_core::backup::filecrypt;
            let need = |p: Option<String>| p.ok_or_else(|| ufop_core::error::AppError::validation("Set the passphrase (--passphrase-env, --passphrase-file)"));
            let (title, r) = match cmd {
                CryptCommands::Encrypt { input, output, pass, kdf_preset } => {
                    let kdf = KdfParams::preset(&kdf_preset).ok_or_else(|| ufop_core::error::AppError::validation(format!("Unknown --kdf-preset '{kdf_preset}'")))?;
                    ("Encrypted", filecrypt::encrypt_file(&input, &output, &need(pass.get())?, kdf)?)
                }
                CryptCommands::Decrypt { input, output, pass } => ("Decrypted", filecrypt::decrypt_file(&input, &output, &need(pass.get())?)?),
                CryptCommands::DecryptVolume { volume, header, output, pass } => ("Volume decrypted", filecrypt::decrypt_volume(&volume, &header, &output, pass.get().as_deref())?),
            };
            ok(emit(format, title, &r))?;
            Ok(0)
        }
        BackupCommands::Db { cmd } => match cmd {
            DbCommands::Info { dbpath } => {
                let i = BackupDb::inspect(&dbpath)?;
                ok(emit(format, "Database", &i))?;
                Ok(if i.integrity_ok { 0 } else { 2 })
            }
            DbCommands::Migrate { dbpath, no_backup } => {
                ok(emit(format, "Database migration", &BackupDb::migrate(&dbpath, !no_backup)?))?;
                Ok(0)
            }
            DbCommands::Vacuum { dbpath } => {
                let before = BackupDb::inspect(&dbpath)?.size_bytes;
                BackupDb::open(&dbpath)?.vacuum()?;
                let after = BackupDb::inspect(&dbpath)?.size_bytes;
                ok(emit(format, "Database vacuumed", &serde_json::json!({ "bytes_before": before, "bytes_after": after })))?;
                Ok(0)
            }
            DbCommands::Check { dbpath } => {
                let problems = BackupDb::open(&dbpath)?.integrity_problems()?;
                ok(emit(format, "Database check", &serde_json::json!({ "ok": problems.is_empty(), "problems": problems })))?;
                Ok(if problems.is_empty() { 0 } else { 2 })
            }
            DbCommands::Sanitize { dbpath, out } => {
                let r = BackupDb::open(&dbpath)?.export_sanitized(&out)?;
                ok(emit(format, "Sanitized copy written", &serde_json::json!({ "out": out, "paths_obfuscated": r.paths_obfuscated, "settings_removed": r.settings_removed })))?;
                Ok(0)
            }
        },
        BackupCommands::SnapshotTest { sources, engine } => {
            let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
            let d = ufop_core::backup::snapshot::diagnose(&srcs, &engine.snapshot_options());
            ok(emit(format, "Snapshot test", &d))?;
            Ok(if d.ok { 0 } else { 2 })
        }
        BackupCommands::AutoTune { repo, samples } => with_repo(&repo, |r, _| {
            ok(emit(format, "Restore AutoTune", &ufop_core::backup::restore::autotune(r, samples.clamp(1, 50), &OpContext::new())?))?;
            Ok(0)
        }),
        BackupCommands::TestDestination { dest, files } => {
            let log = backend::test_backend_compatibility(&LocalBackend::new(&dest), files, 1024, 4 << 20)?;
            ok(emit(format, "Destination test", &log))?;
            Ok(if log.iter().any(|l| l.starts_with("WARN")) { 1 } else { 0 })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_specs() {
        assert_eq!(parse_filter("[\\.tmp$]", FilterAction::Exclude).kind, FilterKind::Regex);
        assert_eq!(parse_filter("{cache_files}", FilterAction::Exclude).kind, FilterKind::Group);
        assert_eq!(parse_filter("*.log", FilterAction::Exclude).kind, FilterKind::Wildcard);
        assert_eq!(parse_filter("/a/b", FilterAction::Include).kind, FilterKind::Exact);
    }

    #[test]
    fn retention_specs() {
        assert_eq!(parse_retention(Some(3), None, None), RetentionPolicy::KeepCount { count: 3 });
        assert_eq!(parse_retention(None, None, Some("smart".into())), RetentionPolicy::Smart);
        assert_eq!(parse_retention(None, None, None), RetentionPolicy::KeepAll);
    }

    #[test]
    fn end_to_end_run_restore_without_db() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), b"cli").unwrap();
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCommands,
        }
        let parse = |args: &[&str]| <T as clap::Parser>::try_parse_from(std::iter::once("t").chain(args.iter().copied())).unwrap().c;
        let dest = t.path().join("dest").to_string_lossy().into_owned();
        let db = t.path().join("db.sqlite").to_string_lossy().into_owned();
        let src_s = src.to_string_lossy().into_owned();
        let run_cmd = parse(&["run", "--dest", &dest, "--dbpath", &db, "--passphrase", "pw", "--source", &src_s, "--exclude", "*.tmp", "--block-size", "4096", "--verify-samples", "1", "--no-quota-warning"]);
        let code = run(run_cmd, &OutputFormat::Json, false).unwrap();
        assert_eq!(code, 0);
        let missing = t.path().join("missing.sqlite").to_string_lossy().into_owned();
        let out = t.path().join("out");
        let out_s = out.to_string_lossy().into_owned();
        let restore_cmd = parse(&["restore", "--dest", &dest, "--dbpath", &missing, "--passphrase", "pw", "--target", &out_s, "--conflict", "overwrite"]);
        let code = run(restore_cmd, &OutputFormat::Json, false).unwrap();
        assert_eq!(code, 0);
        assert_eq!(std::fs::read(out.join("src/f.txt")).unwrap(), b"cli");
    }

    #[test]
    fn photos_source_with_flags() {
        let t = tempfile::tempdir().unwrap();
        let lib = t.path().join("Lib.photoslibrary");
        std::fs::create_dir_all(lib.join("database")).unwrap();
        std::fs::create_dir_all(lib.join("originals/0")).unwrap();
        std::fs::write(lib.join("originals/0/a.heic"), b"shown").unwrap();
        std::fs::write(lib.join("originals/0/b.heic"), b"hidden").unwrap();
        let c = rusqlite::Connection::open(lib.join("database/Photos.sqlite")).unwrap();
        c.execute_batch(
            "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT, ZUUID TEXT, ZDATECREATED REAL, ZHIDDEN INTEGER, ZTRASHEDSTATE INTEGER);
             INSERT INTO ZASSET VALUES (1, '0', 'a.heic', 'UA', 700000000, 0, 0), (2, '0', 'b.heic', 'UB', 700000000, 1, 0);",
        )
        .unwrap();
        drop(c);
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCommands,
        }
        let parse = |args: &[&str]| <T as clap::Parser>::try_parse_from(std::iter::once("t").chain(args.iter().copied())).unwrap().c;
        let dest = t.path().join("dest").to_string_lossy().into_owned();
        let db = t.path().join("db.sqlite").to_string_lossy().into_owned();
        let src = format!("photos://{}", lib.to_string_lossy().replace(' ', "%20"));
        let args = ["run", "--dest", &dest, "--dbpath", &db, "--no-encryption", "--source", &src, "--photos-include-hidden", "--photos-album", "Trip", "--no-quota-warning"];
        match parse(&args) {
            BackupCommands::Run { engine, .. } => {
                let o = engine.apply(BackupOptions::default()).unwrap();
                assert!(o.photos.include_hidden && !o.photos.include_trashed);
                assert_eq!(o.photos.albums, ["Trip"]);
            }
            _ => panic!("expected run"),
        }
        // Without an album filter both originals and albums.json are stored.
        let code = run(parse(&args[..args.len() - 3].iter().copied().chain(["--no-quota-warning"]).collect::<Vec<_>>()), &OutputFormat::Json, false).unwrap();
        // Exit 1: this minimal schema lacks album / attribute tables, reported as warnings.
        assert_eq!(code, 1);
        let out = t.path().join("out");
        let out_s = out.to_string_lossy().into_owned();
        let code = run(parse(&["restore", "--dest", &dest, "--dbpath", &db, "--no-encryption", "--target", &out_s]), &OutputFormat::Json, false).unwrap();
        assert_eq!(code, 0);
        let ex = out.join("Lib (Photos export)");
        assert_eq!(std::fs::read(ex.join("Photos/2023/03/a.heic")).unwrap(), b"shown");
        assert_eq!(std::fs::read(ex.join("Photos/2023/03/b.heic")).unwrap(), b"hidden");
        assert!(ex.join("albums.json").is_file());
    }
}
