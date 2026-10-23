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
use ufop_core::backup::db::{BackupDb, DbKey};
use ufop_core::backup::lock::{self as repo_lock, LockKind, LockOptions};
use ufop_core::backup::engine::Repo;
use ufop_core::backup::restore::ConflictPolicy;
use ufop_core::backup::versions::MetadataPatch;
use ufop_core::backup::{
    maintenance, run_backup, run_restore, Backend, BackupOptions, FilterAction, FilterKind, FilterRule, LocalBackend, OpContext,
    ReadOnlyBackend, RestoreOptions, RetentionPolicy, SourceFilter,
};

/// Program name recorded in repository locks.
const LOCK_APP: &str = "ufop-cli";
/// Keychain service for local-database keys (shared with the desktop app).
const KEYRING_SERVICE: &str = "unified-file-ops";
use ufop_core::backup::backend;

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
    /// Read-only connection: refuse every write to the destination
    /// (restore, list, verify and recovery downloads still work; no lock
    /// file is written)
    #[arg(long)]
    pub read_only: bool,
    /// Key for an encrypted local database: keyring (OS keychain, created on
    /// first use), env:VAR (64 hex digits) or none (plain database; an
    /// encrypted one is rebuilt from the destination)
    #[arg(long, default_value = "none")]
    pub db_key_source: String,
    /// Repository lock lease in seconds (renewed every third of it)
    #[arg(long, default_value_t = ufop_core::backup::lock::DEFAULT_LOCK_TTL_SECS)]
    pub lock_ttl_secs: u64,
    /// Warn when the destination clock differs by this many seconds (0 = never)
    #[arg(long, default_value_t = ufop_core::backup::lock::DEFAULT_SKEW_WARN_SECS)]
    pub clock_skew_warn_secs: u64,
    /// Refuse to run when the destination clock differs by this many seconds (0 = never)
    #[arg(long, default_value_t = ufop_core::backup::lock::DEFAULT_SKEW_ABORT_SECS)]
    pub clock_skew_abort_secs: u64,
    /// Job id recorded in each version's identity: the source in a shared
    /// repository (default: derived from --dest and --prefix; give each
    /// CLI job that shares a repository its own)
    #[arg(long)]
    pub job_id: Option<String>,
    /// Record this host name instead of this computer's (continue another
    /// computer's history after a migration)
    #[arg(long)]
    pub hostname: Option<String>,
    /// Record this user name instead of the current one
    #[arg(long)]
    pub username: Option<String>,
    /// Shared repositories: work on another source's versions (id from
    /// `ufop backup sources`, or * for all) instead of this job's
    #[arg(long)]
    pub source_id: Option<String>,
    #[command(flatten)]
    pub limits: LimitArgs,
}

/// Request-rate limits and listing parallelism for the destination.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct LimitArgs {
    /// Most listing requests per second (0 = unlimited)
    #[arg(long, default_value_t = 0.0)]
    pub list_rate: f64,
    /// Most download requests per second (0 = unlimited)
    #[arg(long, default_value_t = 0.0)]
    pub get_rate: f64,
    /// Most upload requests per second (0 = unlimited)
    #[arg(long, default_value_t = 0.0)]
    pub put_rate: f64,
    /// Most delete requests per second (0 = unlimited)
    #[arg(long, default_value_t = 0.0)]
    pub delete_rate: f64,
    /// Most requests in flight at once (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    pub max_concurrent_requests: u32,
    /// Shard folders of a sharded layout listed at once
    #[arg(long, default_value_t = 1)]
    pub list_parallelism: u32,
}

impl LimitArgs {
    pub fn limits(&self) -> ufop_core::backup::backend::RequestLimits {
        ufop_core::backup::backend::RequestLimits {
            list_per_sec: self.list_rate,
            get_per_sec: self.get_rate,
            put_per_sec: self.put_rate,
            delete_per_sec: self.delete_rate,
            max_concurrent_requests: self.max_concurrent_requests,
            list_parallelism: self.list_parallelism,
        }
    }
}

/// Report a configuration problem and exit (exit code 2, like other usage errors).
fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

impl RepoArgs {
    pub(super) fn passphrase(&self) -> Option<String> {
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

    pub(super) fn db_path(&self) -> PathBuf {
        self.dbpath.clone().unwrap_or_else(|| {
            let h = ufop_core::backup::hash_file_name_key(&format!("{}|{}", self.dest, self.prefix));
            dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("ufop").join("cli-backups").join(format!("{h}.sqlite"))
        })
    }

    /// Who this CLI job writes versions as.
    fn identity(&self) -> ufop_core::backup::versions::ClientIdentity {
        let job = self.job_id.clone().filter(|j| !j.trim().is_empty());
        ufop_core::backup::versions::ClientIdentity {
            hostname: self.hostname.clone().unwrap_or_default(),
            username: self.username.clone().unwrap_or_default(),
            source_label: job.clone().unwrap_or_else(|| "ufop CLI".into()),
            job_id: job.unwrap_or_else(|| format!("cli-{}", ufop_core::backup::hash_file_name_key(&format!("{}|{}", self.dest, self.prefix)))),
        }
    }

    /// Apply `--source-id` to an opened database.
    pub(super) fn apply_source(&self, db: &BackupDb) {
        if let Some(s) = ufop_core::backup::shared::selector_scope(self.source_id.as_deref()) {
            db.set_scope(s);
        }
    }

    pub(super) fn options(&self) -> BackupOptions {
        BackupOptions {
            prefix: self.prefix.clone(),
            identity: self.identity(),
            lock_ttl_secs: self.lock_ttl_secs,
            clock_skew_warn_secs: self.clock_skew_warn_secs,
            clock_skew_abort_secs: self.clock_skew_abort_secs,
            ..Default::default()
        }
    }

    /// The destination, read-only when `--read-only` is given, with the
    /// request limits and listing parallelism of the flags.
    pub(super) fn backend(&self) -> Box<dyn Backend> {
        let limits = self.limits.limits();
        if let Err(e) = limits.validate() {
            fail(&e.user_message());
        }
        let mut local = LocalBackend::new(&self.dest);
        local.list_parallelism = limits.list_parallelism.max(1) as usize;
        let b: Box<dyn Backend> = if limits.is_limited() { Box::new(ufop_core::backup::backend::RateLimitedBackend::new(local, limits)) } else { Box::new(local) };
        if self.read_only {
            Box::new(ReadOnlyBackend::new(b))
        } else {
            b
        }
    }

    fn lock_options(&self) -> LockOptions {
        LockOptions::from_backup_options(&self.options(), LOCK_APP)
    }

    /// Database key and whether the database should be encrypted.
    fn db_key(&self) -> Result<(Option<DbKey>, bool), ufop_core::error::AppError> {
        db_key_from_source(&self.db_key_source, &self.db_path())
    }

    /// Open the local database as `--db-key-source` says; an encrypted
    /// database whose key is missing is set aside and rebuilt from `be`.
    pub(super) fn open_db(&self, be: &dyn Backend, ctx: &OpContext) -> Result<BackupDb, ufop_core::error::AppError> {
        let (key, encrypt) = self.db_key()?;
        let o = maintenance::JobDbOptions { key: key.as_ref(), encrypt };
        let (db, notices) = maintenance::open_job_db(&self.db_path(), &o, Some(be), self.passphrase().as_deref(), &self.prefix, ctx)?;
        for n in notices {
            tracing::warn!(target: "ufop::backup", "{n}");
            eprintln!("note: {n}");
        }
        Ok(db)
    }

    /// Run `f` under the repository lock `kind` (none: no lock).
    pub(super) fn locked<T>(&self, be: &dyn Backend, kind: Option<LockKind>, op: &str, ctx: &OpContext, f: impl FnOnce() -> Result<T, ufop_core::error::AppError>) -> Result<T, ufop_core::error::AppError> {
        let Some(kind) = kind else { return f() };
        let (v, warnings) = repo_lock::with_lock(be, &self.prefix, kind, op, &self.lock_options(), ctx, f)?;
        for w in warnings {
            tracing::warn!(target: "ufop::backup", "{op}: {w}");
            eprintln!("warning: {w}");
        }
        Ok(v)
    }
}

/// Resolve `--db-key-source`: `keyring`, `env:VAR` or `none`.
pub fn db_key_from_source(source: &str, db_path: &std::path::Path) -> Result<(Option<DbKey>, bool), ufop_core::error::AppError> {
    use ufop_core::error::AppError;
    let src = source.trim();
    if src.eq_ignore_ascii_case("none") || src.is_empty() {
        return Ok((None, false));
    }
    if let Some(var) = src.strip_prefix("env:") {
        let v = std::env::var(var).map_err(|_| AppError::validation(format!("--db-key-source {src}: {var} is not set (64 hex digits)")))?;
        return Ok((Some(DbKey::from_hex(&v)?), true));
    }
    if src.eq_ignore_ascii_case("keyring") {
        let account = format!("backup-dbkey-cli:{}", ufop_core::backup::hash_file_name_key(&db_path.to_string_lossy()));
        let entry = keyring::Entry::new(KEYRING_SERVICE, &account).map_err(|e| keychain_error(&e.to_string()))?;
        return match entry.get_password() {
            Ok(h) => Ok((Some(DbKey::from_hex(&h)?), true)),
            Err(keyring::Error::NoEntry) => {
                let k = DbKey::generate();
                entry.set_password(&k.to_hex()).map_err(|e| keychain_error(&e.to_string()))?;
                Ok((Some(k), true))
            }
            Err(e) => Err(keychain_error(&e.to_string())),
        };
    }
    Err(AppError::validation(format!("Invalid --db-key-source '{src}' (keyring, env:VAR, none)")))
}

fn keychain_error(e: &str) -> ufop_core::error::AppError {
    ufop_core::error::AppError::Configuration {
        message: format!("The OS keychain is not available: {e}"),
        advice: "Use --db-key-source env:VAR with a 64-hex-digit key, or none.".into(),
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
    /// Repository format for a new backup: 1 | 2 (default: 2)
    #[arg(long)]
    pub format_version: Option<u32>,
    /// Chunker for a new format 2 backup: fixed | fastcdc (default: fastcdc)
    #[arg(long)]
    pub chunker: Option<String>,
    /// Block codec for a new format 2 backup: deflate | zstd (default: zstd)
    #[arg(long)]
    pub block_codec: Option<String>,
    /// Cipher for a new encrypted format 2 backup: aes256-gcm | chacha20-poly1305
    #[arg(long, default_value = "aes256-gcm")]
    pub cipher: String,
    /// Destination layout for a new format 2 backup: flat | sharded
    #[arg(long, default_value = "flat")]
    pub layout: String,
    /// Hex digits per shard folder name with --layout sharded (1–4)
    #[arg(long, default_value_t = 2)]
    pub shard_prefix_len: u8,
    /// Upload a checkpoint (partial version) at most every N seconds (0 = off)
    #[arg(long, default_value_t = 900)]
    pub checkpoint_interval_secs: u64,
    /// Also checkpoint after this many uploaded bytes (0 = off)
    #[arg(long, default_value_t = 0)]
    pub checkpoint_interval_bytes: u64,
    /// Linux snapshot method: auto | lvm | zfs | btrfs (auto picks it by filesystem)
    #[arg(long, default_value = "auto")]
    pub snapshot_provider: String,
    /// Do not descend into folders on another filesystem (mount points stay empty)
    #[arg(long)]
    pub one_file_system: bool,
    /// Stop at the first error, keeping a partial version of what is finished
    #[arg(long)]
    pub fail_fast: bool,
    /// Unreadable files: continue (warn) | fail (error)
    #[arg(long, default_value = "continue")]
    pub on_file_error: String,
    /// Unlistable folders: continue (warn) | fail (error)
    #[arg(long, default_value = "continue")]
    pub on_dir_error: String,
    /// Sockets, FIFOs, devices: ignore | warn | fail
    #[arg(long, default_value = "ignore")]
    pub on_unknown_entry: String,
    /// Stop after more than N read problems
    #[arg(long)]
    pub max_errors: Option<u32>,
    /// Re-read and re-hash about N% of unchanged-looking files per run (0–100)
    #[arg(long, default_value_t = 0)]
    pub rehash_percent: u8,
    /// Stop after uploading this much (e.g. 500M, 20G) with a partial version; the next run continues
    #[arg(long)]
    pub max_upload: Option<String>,
    /// Hash files larger than this with several threads (e.g. 256M; 0 = off)
    #[arg(long, default_value = "256M")]
    pub parallel_file_threshold: String,
}

/// Ignore files (`--ignore-file`, `--respect-gitignore`).
#[derive(Args, Debug, Clone, Default)]
pub struct IgnoreArgs {
    /// Ignore-file name read in every folder, gitignore syntax (repeatable;
    /// replaces the default `.ufopignore`; `--ignore-file ""` reads none)
    #[arg(long = "ignore-file")]
    pub ignore_files: Vec<String>,
    /// Also honour .gitignore files and .git/info/exclude
    #[arg(long)]
    pub respect_gitignore: bool,
}

impl IgnoreArgs {
    fn apply(&self, f: &mut SourceFilter) {
        if !self.ignore_files.is_empty() {
            f.ignore_files = self.ignore_files.iter().filter(|n| !n.trim().is_empty()).cloned().collect();
        }
        f.respect_gitignore = self.respect_gitignore;
    }
}

/// `10G`, `500MB`, `1.5GiB`, `4096` → bytes (binary units).
pub fn parse_bytes(s: &str) -> Result<u64, ufop_core::error::AppError> {
    let t = s.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().map_err(|_| ufop_core::error::AppError::validation(format!("Invalid size '{s}' (e.g. 500M, 20G)")))?;
    let mult: u64 = match unit.trim().to_ascii_uppercase().trim_end_matches("IB").trim_end_matches('B') {
        "" => 1,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        "T" => 1 << 40,
        _ => return Err(ufop_core::error::AppError::validation(format!("Invalid size unit in '{s}' (K, M, G, T)"))),
    };
    Ok((n * mult as f64) as u64)
}

/// Count-based GFS retention (`--keep-daily 7 --keep-weekly 4 …`). Any
/// non-zero count selects it over --keep-versions / --keep-days.
#[derive(Args, Debug, Clone, Default)]
pub struct CountArgs {
    /// Keep the N newest versions
    #[arg(long, default_value_t = 0)]
    pub keep_latest: u32,
    /// Keep the newest version of each of the N most recent hours
    #[arg(long, default_value_t = 0)]
    pub keep_hourly: u32,
    /// … days (UTC)
    #[arg(long, default_value_t = 0)]
    pub keep_daily: u32,
    /// … ISO weeks
    #[arg(long, default_value_t = 0)]
    pub keep_weekly: u32,
    /// … months
    #[arg(long, default_value_t = 0)]
    pub keep_monthly: u32,
    /// … years
    #[arg(long, default_value_t = 0)]
    pub keep_yearly: u32,
}

impl CountArgs {
    fn policy(&self) -> Option<RetentionPolicy> {
        let c = [self.keep_latest, self.keep_hourly, self.keep_daily, self.keep_weekly, self.keep_monthly, self.keep_yearly];
        c.iter().any(|n| *n > 0).then_some(RetentionPolicy::Counts {
            keep_latest: self.keep_latest,
            keep_hourly: self.keep_hourly,
            keep_daily: self.keep_daily,
            keep_weekly: self.keep_weekly,
            keep_monthly: self.keep_monthly,
            keep_yearly: self.keep_yearly,
        })
    }
}

/// Retention flags shared by `run` and `retention preview|apply`.
#[derive(Args, Debug, Clone, Default)]
pub struct RetentionArgs {
    #[arg(long)]
    pub keep_versions: Option<u32>,
    #[arg(long)]
    pub keep_days: Option<u32>,
    /// `smart` or custom rules like `1W:1D,4W:1W,12M:1M`
    #[arg(long)]
    pub retention_policy: Option<String>,
    #[command(flatten)]
    pub counts: CountArgs,
}

impl RetentionArgs {
    pub fn policy(&self) -> RetentionPolicy {
        self.counts.policy().unwrap_or_else(|| parse_retention(self.keep_versions, self.keep_days, self.retention_policy.clone()))
    }
}

/// Retention tools.
#[derive(Subcommand, Debug)]
pub enum RetentionCommands {
    /// Show which versions a policy keeps (and why) and which it deletes; changes nothing
    Preview {
        #[command(flatten)]
        repo: RepoArgs,
        #[command(flatten)]
        retention: RetentionArgs,
    },
    /// Apply a policy now (pinned and locked versions are kept)
    Apply {
        #[command(flatten)]
        repo: RepoArgs,
        #[command(flatten)]
        retention: RetentionArgs,
        #[arg(long)]
        allow_full_removal: bool,
    },
}

/// Parse `--layout` / `--shard-prefix-len`.
pub fn parse_layout(layout: &str, prefix_len: u8) -> Result<ufop_core::backup::layout::RepoLayout, ufop_core::error::AppError> {
    use ufop_core::backup::layout::RepoLayout;
    let l = match layout.to_ascii_lowercase().as_str() {
        "flat" => RepoLayout::Flat,
        "sharded" => RepoLayout::Sharded { prefix_len },
        other => return Err(ufop_core::error::AppError::validation(format!("Invalid value '{other}' for --layout (flat, sharded)"))),
    };
    l.validate()?;
    Ok(l)
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
            provider: parse_enum("snapshot-provider", &self.snapshot_provider).unwrap_or_default(),
        }
    }

    fn error_policy(&self) -> Result<ufop_core::backup::errors::ErrorPolicy, ufop_core::error::AppError> {
        Ok(ufop_core::backup::errors::ErrorPolicy {
            on_file_error: parse_enum("on-file-error", &self.on_file_error)?,
            on_dir_error: parse_enum("on-dir-error", &self.on_dir_error)?,
            on_unknown_entry: parse_enum("on-unknown-entry", &self.on_unknown_entry)?,
            fail_fast: self.fail_fast,
            max_errors: self.max_errors,
        })
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
            format_version: self.format_version,
            chunker: self.chunker.as_deref().map(|c| parse_enum("chunker", c)).transpose()?,
            block_codec: self.block_codec.as_deref().map(|c| parse_enum("block-codec", c)).transpose()?,
            cipher: ufop_core::backup::crypto::CipherKind::parse(&self.cipher)
                .ok_or_else(|| ufop_core::error::AppError::validation(format!("Invalid value '{}' for --cipher (aes256-gcm, chacha20-poly1305)", self.cipher)))?,
            layout: parse_layout(&self.layout, self.shard_prefix_len)?,
            checkpoint_interval_secs: self.checkpoint_interval_secs,
            checkpoint_interval_bytes: self.checkpoint_interval_bytes,
            one_file_system: self.one_file_system,
            error_policy: self.error_policy()?,
            rehash_percent: if self.rehash_percent <= 100 {
                self.rehash_percent
            } else {
                return Err(ufop_core::error::AppError::validation("--rehash-percent must be between 0 and 100"));
            },
            max_upload_bytes_per_run: self.max_upload.as_deref().map(parse_bytes).transpose()?.filter(|b| *b > 0),
            parallel_file_threshold: parse_bytes(&self.parallel_file_threshold)?,
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
            read_only: true,
            db_key_source: "none".into(),
            lock_ttl_secs: 0,
            clock_skew_warn_secs: 0,
            clock_skew_abort_secs: 0,
            job_id: None,
            hostname: None,
            username: None,
            source_id: None,
            limits: LimitArgs::default(),
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

/// Repository lock tools.
#[derive(Subcommand, Debug)]
pub enum LockCommands {
    /// Who holds the repository lock(s): host, operation, since, expiry
    Status {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Remove the lock(s), only when the program named in them is no longer running
    Break {
        #[command(flatten)]
        repo: RepoArgs,
        /// Confirm that the holder is no longer running
        #[arg(long)]
        yes: bool,
        /// Only remove the lock with this owner id (from `lock status`)
        #[arg(long)]
        owner_id: Option<String>,
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
    Sanitize {
        dbpath: PathBuf,
        out: PathBuf,
        /// Key of an encrypted database: keyring, env:VAR or none
        #[arg(long, default_value = "none")]
        db_key_source: String,
    },
    /// Encrypt a plain database in place (key from --db-key-source), or
    /// with --decrypt turn an encrypted one back into a plain one
    Encrypt {
        dbpath: PathBuf,
        /// keyring or env:VAR
        #[arg(long, default_value = "keyring")]
        db_key_source: String,
        #[arg(long)]
        decrypt: bool,
    },
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
        #[command(flatten)]
        ignore: IgnoreArgs,
        #[arg(long, default_value_t = 100 * 1024)]
        block_size: usize,
        #[arg(long, default_value_t = 50 * 1024 * 1024)]
        volume_size: usize,
        #[arg(long, default_value_t = 6)]
        compression_level: u32,
        #[command(flatten)]
        retention: RetentionArgs,
        /// Note stored with the new version
        #[arg(long)]
        note: Option<String>,
        /// Tag stored with the new version, `key=value` or `key` (repeatable)
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Create the repository as shared (format 2): other jobs and
        /// computers can write to it and deduplicate against each other
        #[arg(long)]
        shared: bool,
        /// Shared repositories: compaction leaves volumes younger than this alone
        #[arg(long, default_value_t = ufop_core::backup::shared::DEFAULT_GC_DELAY_HOURS)]
        shared_gc_delay_hours: u32,
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
    /// Back up standard input as one file of a new version
    /// (`pg_dump db | ufop backup stdin --name db.sql --dest …`)
    Stdin {
        #[command(flatten)]
        repo: RepoArgs,
        /// File name in the version: `name` → /stdin/name, or an absolute path
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 100 * 1024)]
        block_size: usize,
        #[arg(long, default_value_t = 50 * 1024 * 1024)]
        volume_size: usize,
        #[arg(long, default_value_t = 6)]
        compression_level: u32,
        #[command(flatten)]
        retention: RetentionArgs,
        #[arg(long)]
        note: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Estimate the next backup: files, new data and duration (reads no file contents)
    Estimate {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(long = "source", required = true)]
        sources: Vec<String>,
        #[arg(long)]
        include: Vec<String>,
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        exclude_hidden: bool,
        #[arg(long)]
        skip_files_larger_than: Option<u64>,
        #[command(flatten)]
        ignore: IgnoreArgs,
        /// Assumed upload speed for the duration (e.g. 20M per second; 0 = none)
        #[arg(long, default_value = "20M")]
        assumed_speed: String,
        /// List this many of the largest new or changed files
        #[arg(long, default_value_t = 10)]
        largest: usize,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Back up every job of the signed-in ufop-server now (see `ufop server login`)
    RunAll {
        /// Only these jobs (id or name, repeatable)
        #[arg(long = "job")]
        jobs: Vec<String>,
        /// Only jobs with a schedule
        #[arg(long)]
        scheduled_only: bool,
        /// Wait until every started backup finished; exit with the worst result
        #[arg(long)]
        wait: bool,
    },
    /// List backup versions (with note, tags, pin, host and storage added)
    List {
        #[command(flatten)]
        repo: RepoArgs,
        /// Only versions carrying this tag, `key=value` or `key` (repeatable)
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// Pin a version ("Keep forever"): retention never removes it and
    /// `delete` needs --force
    Pin {
        #[command(flatten)]
        repo: RepoArgs,
        version: usize,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove a version's pin
    Unpin {
        #[command(flatten)]
        repo: RepoArgs,
        version: usize,
    },
    /// Change a version's note or tags
    Annotate {
        #[command(flatten)]
        repo: RepoArgs,
        version: usize,
        /// New note (empty string removes it)
        #[arg(long)]
        note: Option<String>,
        /// Replace the tags with these, `key=value` or `key` (repeatable)
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Remove every tag
        #[arg(long)]
        clear_tags: bool,
    },
    /// Retention preview and apply
    Retention {
        #[command(subcommand)]
        cmd: RetentionCommands,
    },
    /// Copy versions to another repository (only the blocks it lacks;
    /// re-cut when its chunker differs)
    CopyVersions {
        #[command(flatten)]
        repo: RepoArgs,
        /// Version numbers to copy (0 = newest)
        #[arg(required = true)]
        versions: Vec<usize>,
        /// Destination folder of the target repository (created when empty)
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "ufop")]
        to_prefix: String,
        /// Local database of the target (default: derived from --to)
        #[arg(long)]
        to_dbpath: Option<PathBuf>,
        /// Environment variable with the target's passphrase
        #[arg(long, default_value = "UFOP_TO_PASSPHRASE")]
        to_passphrase_env: String,
        #[arg(long)]
        to_no_encryption: bool,
        /// Format of a new target repository: 1 | 2
        #[arg(long)]
        to_format_version: Option<u32>,
        /// Chunker of a new format 2 target: fixed | fastcdc
        #[arg(long)]
        to_chunker: Option<String>,
    },
    /// Sources (jobs / computers) with versions in a shared repository
    Sources {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Make a format 2 repository shared by several jobs or computers
    Share {
        #[command(flatten)]
        repo: RepoArgs,
        /// Assign only versions without an identity to this job (default: all)
        #[arg(long)]
        keep_identities: bool,
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
        /// Version: a number (0 = newest), latest, oldest, or a version's timestamp
        #[arg(long, default_value = "0")]
        version: String,
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
        #[command(flatten)]
        extra: super::backup_access::RestoreExtraArgs,
    },
    /// Write one backed-up file to standard output
    Cat {
        #[command(flatten)]
        repo: RepoArgs,
        /// Backed-up path of the file
        path: String,
        #[command(flatten)]
        version: super::backup_access::VersionArgs,
        /// Start at this byte
        #[arg(long)]
        offset: Option<u64>,
        /// Write at most this many bytes
        #[arg(long)]
        length: Option<u64>,
        #[command(flatten)]
        cache: super::backup_access::CacheArgs,
    },
    /// Serve a version (or every version) as a read-only WebDAV drive until Ctrl-C
    Mount {
        #[command(flatten)]
        repo: RepoArgs,
        #[command(flatten)]
        version: super::backup_access::VersionArgs,
        /// Every version as a folder named by its date and time
        #[arg(long)]
        all: bool,
        /// Address to listen on (default: this computer only)
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Port (0 = any free port)
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Stop after this many seconds instead of waiting for Ctrl-C
        #[arg(long, hide = true)]
        seconds: Option<u64>,
        #[command(flatten)]
        cache: super::backup_access::CacheArgs,
    },
    /// The local volume cache used by cat, mount and --use-cache restores
    Cache {
        #[command(subcommand)]
        cmd: super::backup_access::CacheCommands,
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
    /// Job-database tools (info, migrate, vacuum, encrypt)
    Db {
        #[command(subcommand)]
        cmd: DbCommands,
    },
    /// Repository lock status and guarded force-unlock
    Lock {
        #[command(subcommand)]
        cmd: LockCommands,
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
    /// Check the backup: structure, sampled or all volumes, and files
    Verify {
        #[command(flatten)]
        repo: RepoArgs,
        /// Download every volume in scope and check every block's hash
        #[arg(long)]
        full: bool,
        /// Download this many volumes (default 3)
        #[arg(long)]
        samples: Option<usize>,
        /// Only these versions: number (0 = newest), latest, oldest or a timestamp (repeatable)
        #[arg(long = "version")]
        versions: Vec<String>,
        /// Only these paths: a folder or a wildcard (repeatable)
        #[arg(long = "path")]
        paths: Vec<String>,
        /// Download this percentage of the volumes in scope
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
        percent: Option<u8>,
        /// Check the index and the destination listing only (no downloads)
        #[arg(long, conflicts_with_all = ["full", "samples", "percent"])]
        structure: bool,
        /// Rebuild this percentage of the files and check their hashes
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
        files_percent: Option<u8>,
        /// Downloads at the same time (default 4)
        #[arg(long, default_value_t = 0)]
        parallel: usize,
        /// Stop after this many problems (0 = check everything)
        #[arg(long, default_value_t = 0)]
        max_errors: usize,
        /// Read volumes from the local volume cache when there (default: always download)
        #[arg(long)]
        use_cache: bool,
        #[command(flatten)]
        cache: super::backup_access::CacheArgs,
    },
    /// Policy templates of the signed-in ufop-server
    Template {
        #[command(subcommand)]
        cmd: super::backup_policy::TemplateCommands,
    },
    /// Each setting a server job runs with and where it comes from
    Effective {
        /// Job id or name
        job: String,
    },
    /// Open a server job's versioned S3 destination as it was at a time, as a
    /// new read-only job (recovery after ransomware or accidental deletes)
    OpenAt {
        /// Job id or name (signed-in ufop-server)
        job: String,
        /// RFC 3339 time, e.g. 2026-10-01T08:00:00Z
        #[arg(long = "point-in-time")]
        point_in_time: String,
        /// Name of the new read-only job
        #[arg(long)]
        name: Option<String>,
    },
    /// Validate a schedule and list its next runs (signed-in ufop-server)
    SchedulePreview {
        /// Cron expression, `min hour day month weekday` (repeatable)
        #[arg(long)]
        cron: Vec<String>,
        /// Interval schedules: start time HH:MM
        #[arg(long)]
        start_time: Option<String>,
        /// Interval schedules: minutes between runs
        #[arg(long)]
        interval_minutes: Option<u32>,
        #[arg(long, default_value_t = 5)]
        count: usize,
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
        /// Also delete pinned versions
        #[arg(long)]
        force: bool,
    },
    /// Upgrade the repository to format 2 (rewrites only the header), or
    /// with --rollback restore the saved format 1 header (only until the
    /// first format 2 write)
    Upgrade {
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(long)]
        rollback: bool,
    },
    /// Change the passphrase of a format 2 repository (rewrites only the header)
    ChangePassword {
        #[command(flatten)]
        repo: RepoArgs,
        /// Environment variable holding the new passphrase
        #[arg(long, default_value = "UFOP_NEW_PASSPHRASE")]
        new_passphrase_env: String,
        /// File whose first line is the new passphrase
        #[arg(long)]
        new_passphrase_file: Option<PathBuf>,
        /// Argon2id cost for the new key: interactive | default | sensitive (default: keep)
        #[arg(long)]
        kdf_preset: Option<String>,
    },
    /// Move volumes between the flat and sharded layouts (format 2; --dry-run previews)
    Reshard {
        #[command(flatten)]
        repo: RepoArgs,
        /// flat | sharded
        #[arg(long)]
        layout: String,
        #[arg(long, default_value_t = 2)]
        shard_prefix_len: u8,
    },
    /// Show the repository format (version, chunker, codec, cipher, layout)
    FormatInfo {
        #[command(flatten)]
        repo: RepoArgs,
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

pub(super) fn progress_ctx(format: &OutputFormat) -> OpContext {
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
pub(super) fn log_outcome(op: &str, warnings: &[String], errors: &[String]) {
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

pub(super) fn emit<T: serde::Serialize>(format: &OutputFormat, title: &str, v: &T) -> Result<(), Box<dyn std::error::Error>> {
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

/// Open the repository under a lock of `kind` and run `f`.
pub(super) fn with_repo<T>(
    repo: &RepoArgs,
    kind: LockKind,
    op: &str,
    f: impl FnOnce(&Repo, &BackupDb) -> Result<T, ufop_core::error::AppError>,
) -> Result<T, ufop_core::error::AppError> {
    with_repo_cache(repo, kind, op, None, f)
}

/// [`with_repo`] reading volumes through a local volume cache.
pub(super) fn with_repo_cache<T>(
    repo: &RepoArgs,
    kind: LockKind,
    op: &str,
    cache: Option<std::sync::Arc<ufop_core::backup::cache::VolumeCache>>,
    f: impl FnOnce(&Repo, &BackupDb) -> Result<T, ufop_core::error::AppError>,
) -> Result<T, ufop_core::error::AppError> {
    let be = repo.backend();
    let ctx = OpContext::new();
    repo.locked(be.as_ref(), Some(kind), op, &ctx, || {
        let db = repo.open_db(be.as_ref(), &ctx)?;
        let pass = repo.passphrase();
        let r = Repo::open(be.as_ref(), &db, pass.as_deref(), &repo.options(), false, KdfParams::default())?.with_cache(cache);
        repo.apply_source(&db);
        f(&r, &db)
    })
}

/// Change a version's metadata under the exclusive lock and show it.
fn set_meta(repo: &RepoArgs, version: usize, patch: MetadataPatch, format: &OutputFormat) -> Result<u8, ufop_core::error::AppError> {
    with_repo(repo, LockKind::Exclusive, "set_version_metadata", |r, _| {
        let f = ufop_core::backup::versions::set_version_metadata(r, version, &patch)?;
        if matches!(format, OutputFormat::Human) {
            print!("{}", version_table(std::slice::from_ref(&f)));
        } else {
            output::print_data(format, &f);
        }
        Ok(0)
    })
}

fn short_bytes(b: u64) -> String {
    let mut v = b as f64;
    for u in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 {
            return if u == "B" { format!("{b} B") } else { format!("{v:.1} {u}") };
        }
        v /= 1024.0;
    }
    format!("{v:.1} PB")
}

/// Human version list: index, time, files, size, storage added, flags,
/// host, note and tags ("—" where an older version has no data).
pub fn version_table(v: &[ufop_core::backup::Fileset]) -> String {
    let mut out = format!("{:>3}  {:<25}  {:>7}  {:>10}  {:>10}  {:<5}  {:<16}  {}\n", "#", "Time", "Files", "Size", "Added", "Flags", "Host", "Note / tags");
    for (i, f) in v.iter().enumerate() {
        let added = f.stats.map(|s| short_bytes(s.bytes_added)).unwrap_or_else(|| "—".into());
        let flags = format!("{}{}", if f.pinned { "P" } else { "" }, if f.partial { "~" } else { "" });
        let host = f.identity.as_ref().map(|i| i.hostname.clone()).unwrap_or_else(|| "—".into());
        let tags: Vec<String> = f.tags.iter().map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") }).collect();
        let mut extra = f.note.clone().unwrap_or_default();
        if !tags.is_empty() {
            if !extra.is_empty() {
                extra.push_str("  ");
            }
            extra.push_str(&format!("[{}]", tags.join(", ")));
        }
        out.push_str(&format!("{i:>3}  {:<25}  {:>7}  {:>10}  {:>10}  {:<5}  {:<16}  {extra}\n", f.timestamp, f.file_count, short_bytes(f.total_size), added, flags, host));
    }
    out
}

/// Human-readable estimate.
pub fn estimate_text(e: &ufop_core::backup::estimate::BackupEstimate) -> String {
    let mut out = format!(
        "{} files ({}) in {} folders: {} new, {} changed, {} unchanged, {} deleted\n",
        e.files,
        short_bytes(e.bytes_examined),
        e.folders,
        e.new_files,
        e.changed_files,
        e.unchanged_files,
        e.deleted_files
    );
    if e.upper_bound {
        out.push_str(&format!("New data: at most {} (at least {})\n", short_bytes(e.new_bytes_at_most), short_bytes(e.new_bytes_at_least)));
    } else {
        out.push_str(&format!("New data: {}\n", short_bytes(e.new_bytes_at_most)));
    }
    if let (Some(d), Some(b)) = (e.estimated_duration_secs, e.bytes_per_sec) {
        out.push_str(&format!("Estimated time: {} at {}/s ({})\n", human_duration(d), short_bytes(b), e.speed_source));
    }
    for f in &e.largest {
        out.push_str(&format!("  {:>10}  {:<7}  {}\n", short_bytes(f.size), f.status, f.path));
    }
    for w in &e.warnings {
        out.push_str(&format!("warning: {w}\n"));
    }
    out
}

/// Human-readable verification report.
pub fn verify_text(v: &ufop_core::backup::verify::VerifyReport) -> String {
    let mut out = format!(
        "{} version(s), {} file(s) in scope, {} block(s) referenced, {} volume(s)\nDownloaded and checked {} volume(s); rebuilt and checked {} file(s)\n",
        v.versions_checked, v.files_in_scope, v.blocks_referenced, v.volumes_in_scope, v.checked, v.files_checked
    );
    if v.problems.is_empty() {
        out.push_str("No problems found\n");
    }
    for p in &v.problems {
        out.push_str(&format!("[{}] {}\n", p.kind, p.message));
        if !p.versions.is_empty() {
            out.push_str(&format!("    versions: {}\n", p.versions.join(", ")));
        }
        for path in &p.paths {
            out.push_str(&format!("    {path}\n"));
        }
    }
    if let Some(r) = &v.stop_reason {
        out.push_str(&format!("{r}\n"));
    }
    out
}

fn human_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

pub async fn execute(cmd: BackupCommands, format: &OutputFormat, dry_run: bool) -> Result<u8, Box<dyn std::error::Error>> {
    if let BackupCommands::RunAll { jobs, scheduled_only, wait } = cmd {
        return super::server::execute(super::server::ServerCommands::RunAll { jobs, scheduled_only, dry_run, wait }, format).await;
    }
    let server_backed = match &cmd {
        BackupCommands::Template { .. } | BackupCommands::Effective { .. } | BackupCommands::SchedulePreview { .. } | BackupCommands::OpenAt { .. } => true,
        _ => false,
    };
    if server_backed {
        let r = match cmd {
            BackupCommands::Template { cmd } => super::backup_policy::template(cmd, format).await,
            BackupCommands::Effective { job } => super::backup_policy::effective(&job, format).await,
            BackupCommands::OpenAt { job, point_in_time, name } => super::backup_policy::open_at(&job, &point_in_time, name, format).await,
            BackupCommands::SchedulePreview { cron, start_time, interval_minutes, count } => {
                super::backup_policy::schedule_preview(&cron, start_time.as_deref(), interval_minutes, count, format).await
            }
            _ => unreachable!("server-backed commands only"),
        };
        return match r {
            Ok(code) => Ok(code),
            Err(e) => {
                output::print_error(format, &e);
                Ok(2)
            }
        };
    }
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
            repo, sources, include, exclude, exclude_hidden, skip_files_larger_than, ignore, block_size, volume_size, compression_level,
            retention, note, tags, shared, shared_gc_delay_hours, verify_samples, upload_unchanged, no_auto_compact, engine,
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
            ignore.apply(&mut filter);
            let filter = filter.compile()?;
            let opts = engine.apply(BackupOptions {
                block_size,
                volume_size,
                compression_level,
                verify_samples,
                upload_unchanged_backups: upload_unchanged,
                auto_compact: !no_auto_compact,
                dry_run,
                default_note: note,
                default_tags: ufop_core::backup::versions::parse_tags(&tags)?,
                shared_repository: shared,
                shared_gc_delay_hours,
                ..repo.options()
            })?;
            let retention = retention.policy();
            let be = repo.backend();
            let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
            let ctx = progress_ctx(format);
            let kind = if dry_run { LockKind::Shared } else { LockKind::Exclusive };
            let lo = LockOptions::from_backup_options(&opts, LOCK_APP);
            let (mut r, lock_warnings) = repo_lock::with_lock(be.as_ref(), &repo.prefix, kind, "backup", &lo, &ctx, || {
                let db = repo.open_db(be.as_ref(), &ctx)?;
                run_backup(be.as_ref(), &db, &srcs, &filter, &opts, &retention, pass.as_deref(), KdfParams::default(), &ctx)
            })?;
            r.warnings.extend(lock_warnings);
            log_outcome("backup", &r.warnings, &r.errors);
            ok(emit(format, "Backup finished", &r))?;
            Ok(if !r.errors.is_empty() || r.partial { 2 } else if !r.warnings.is_empty() { 1 } else { 0 })
        }
        BackupCommands::Stdin { repo, name, block_size, volume_size, compression_level, retention, note, tags, engine } => {
            let pass = repo.passphrase();
            if pass.is_none() && !repo.no_encryption {
                return Err(ufop_core::error::AppError::validation(format!("No passphrase: set {} (or pass --no-encryption)", repo.passphrase_env)));
            }
            let opts = engine.apply(BackupOptions {
                block_size,
                volume_size,
                compression_level,
                dry_run,
                default_note: note,
                default_tags: ufop_core::backup::versions::parse_tags(&tags)?,
                ..repo.options()
            })?;
            let retention = retention.policy();
            let be = repo.backend();
            let ctx = progress_ctx(format);
            let kind = if dry_run { LockKind::Shared } else { LockKind::Exclusive };
            let lo = LockOptions::from_backup_options(&opts, LOCK_APP);
            let filter = SourceFilter::default().compile()?;
            let (mut r, lock_warnings) = repo_lock::with_lock(be.as_ref(), &repo.prefix, kind, "backup", &lo, &ctx, || {
                let db = repo.open_db(be.as_ref(), &ctx)?;
                let stream = ufop_core::backup::StreamSource::new(name.clone(), std::io::stdin());
                ufop_core::backup::run_backup_with_streams(be.as_ref(), &db, &[], &filter, &opts, &retention, pass.as_deref(), None, KdfParams::default(), &ctx, vec![stream])
            })?;
            r.warnings.extend(lock_warnings);
            log_outcome("backup", &r.warnings, &r.errors);
            ok(emit(format, &format!("Stored standard input as {}", ufop_core::backup::reader::stream_path(&name)), &r))?;
            Ok(if !r.errors.is_empty() || r.partial { 2 } else if !r.warnings.is_empty() { 1 } else { 0 })
        }
        BackupCommands::Estimate { repo, sources, include, exclude, exclude_hidden, skip_files_larger_than, ignore, assumed_speed, largest, engine } => {
            let mut filter = SourceFilter::default();
            filter.rules = include.iter().map(|s| parse_filter(s, FilterAction::Include)).chain(exclude.iter().map(|s| parse_filter(s, FilterAction::Exclude))).collect();
            filter.exclude_hidden = exclude_hidden;
            filter.max_file_size = skip_files_larger_than;
            ignore.apply(&mut filter);
            let filter = filter.compile()?;
            let opts = engine.apply(repo.options())?;
            let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
            let eo = ufop_core::backup::estimate::EstimateOptions { history_bytes_per_sec: None, assumed_bytes_per_sec: parse_bytes(&assumed_speed)?, largest };
            // A destination without a backup yet: everything is new.
            let be = repo.backend();
            let est = match be.list() {
                Ok(l) if l.iter().any(|f| f.name == ufop_core::backup::volume::repo_header_name(&repo.prefix)) => {
                    with_repo(&repo, LockKind::Shared, "estimate", |_, db| ufop_core::backup::estimate::estimate(db, &srcs, &filter, &opts, &eo))?
                }
                _ => ufop_core::backup::estimate::estimate(&ufop_core::backup::db::BackupDb::open_in_memory()?, &srcs, &filter, &opts, &eo)?,
            };
            if matches!(format, OutputFormat::Human) {
                print!("{}", estimate_text(&est));
            } else {
                ok(emit(format, "Estimate", &est))?;
            }
            Ok(0)
        }
        BackupCommands::RunAll { .. } | BackupCommands::Template { .. } | BackupCommands::Effective { .. } | BackupCommands::SchedulePreview { .. } | BackupCommands::OpenAt { .. } => Err(ufop_core::error::AppError::internal("server-backed commands are handled before the blocking runner")),
        BackupCommands::List { repo, tags } => with_repo(&repo, LockKind::Shared, "list", |_, db| {
            let filter = ufop_core::backup::versions::parse_tags(&tags)?;
            let v = ufop_core::backup::versions::filter_by_tags(db.filesets()?, &filter);
            if matches!(format, OutputFormat::Human) {
                print!("{}", version_table(&v));
            } else {
                ok(emit(format, "Versions (0 = newest)", &v))?;
            }
            Ok(0)
        }),
        BackupCommands::Pin { repo, version, reason } => {
            let patch = MetadataPatch { pinned: Some(true), pin_reason: reason, ..Default::default() };
            set_meta(&repo, version, patch, format)
        }
        BackupCommands::Unpin { repo, version } => set_meta(&repo, version, MetadataPatch { pinned: Some(false), ..Default::default() }, format),
        BackupCommands::Annotate { repo, version, note, tags, clear_tags } => {
            let tags = if clear_tags { Some(Default::default()) } else if tags.is_empty() { None } else { Some(ufop_core::backup::versions::parse_tags(&tags)?) };
            if note.is_none() && tags.is_none() {
                return Err(ufop_core::error::AppError::validation("Give --note, --tag or --clear-tags"));
            }
            set_meta(&repo, version, MetadataPatch { note, tags, ..Default::default() }, format)
        }
        BackupCommands::Retention { cmd } => match cmd {
            RetentionCommands::Preview { repo, retention } => with_repo(&repo, LockKind::Shared, "retention_preview", |_, db| {
                let p = maintenance::preview_retention(db, &retention.policy(), false)?;
                if matches!(format, OutputFormat::Human) {
                    for (i, v) in p.versions.iter().enumerate() {
                        println!("{i:>4}  {:<25}  {}", v.timestamp, v.summary);
                    }
                    println!("{} kept, {} deleted", p.keep, p.delete);
                } else {
                    ok(emit(format, "Retention preview", &p))?;
                }
                Ok(0)
            }),
            RetentionCommands::Apply { repo, retention, allow_full_removal } => with_repo(&repo, if dry_run { LockKind::Shared } else { LockKind::Exclusive }, "apply_retention", |r, db| {
                if dry_run {
                    ok(emit(format, "Retention (dry run)", &maintenance::preview_retention(db, &retention.policy(), false)?))?;
                    return Ok(0);
                }
                let n = maintenance::apply_retention(r, &retention.policy(), allow_full_removal)?;
                ok(emit(format, "Retention applied", &serde_json::json!({ "versions_deleted": n })))?;
                Ok(0)
            }),
        },
        BackupCommands::CopyVersions { repo, versions, to, to_prefix, to_dbpath, to_passphrase_env, to_no_encryption, to_format_version, to_chunker } => {
            let target = RepoArgs {
                dest: to,
                dbpath: to_dbpath,
                passphrase: None,
                passphrase_env: to_passphrase_env,
                passphrase_file: None,
                secret_providers: repo.secret_providers.clone(),
                no_encryption: to_no_encryption,
                prefix: to_prefix,
                read_only: false,
                db_key_source: repo.db_key_source.clone(),
                lock_ttl_secs: repo.lock_ttl_secs,
                clock_skew_warn_secs: repo.clock_skew_warn_secs,
                clock_skew_abort_secs: repo.clock_skew_abort_secs,
                job_id: None,
                hostname: None,
                username: None,
                source_id: None,
                limits: repo.limits.clone(),
            };
            let dst_opts = BackupOptions {
                format_version: to_format_version,
                chunker: to_chunker.as_deref().map(|c| parse_enum("to-chunker", c)).transpose()?,
                dry_run,
                ..target.options()
            };
            let (sb, tb) = (repo.backend(), target.backend());
            let ctx = progress_ctx(format);
            let opts = ufop_core::backup::copy::CopyOptions { versions, source: repo.source_id.clone(), dry_run };
            let rep = repo.locked(sb.as_ref(), Some(LockKind::Shared), "copy_versions", &ctx, || {
                let sdb = repo.open_db(sb.as_ref(), &ctx)?;
                let src = Repo::open(sb.as_ref(), &sdb, repo.passphrase().as_deref(), &repo.options(), false, KdfParams::default())?;
                target.locked(tb.as_ref(), Some(if dry_run { LockKind::Shared } else { LockKind::Exclusive }), "copy_versions", &ctx, || {
                    let ddb = target.open_db(tb.as_ref(), &ctx)?;
                    let dst = Repo::open(tb.as_ref(), &ddb, target.passphrase().as_deref(), &dst_opts, true, KdfParams::default())?;
                    ufop_core::backup::copy::copy_versions(&src, &dst, &opts, &dst_opts, &ctx)
                })
            })?;
            log_outcome("copy_versions", &rep.warnings, &[]);
            ok(emit(format, "Copy versions", &rep))?;
            Ok(if rep.warnings.is_empty() { 0 } else { 1 })
        }
        BackupCommands::Sources { repo } => with_repo(&repo, LockKind::Shared, "sources", |r, db| {
            let own = repo.identity().resolve().source_id();
            let list = ufop_core::backup::shared::list_sources(db, Some(&own))?;
            if matches!(format, OutputFormat::Human) {
                println!("Shared repository: {}", if ufop_core::backup::shared::is_shared(&r.header) { "yes" } else { "no" });
                for s in &list {
                    println!("{} {:<16}  {:>4} version(s)  newest {}  {}", if s.own { "*" } else { " " }, s.id, s.versions, s.newest.as_deref().unwrap_or("-"), s.label);
                }
            } else {
                ok(emit(format, "Sources", &list))?;
            }
            Ok(0)
        }),
        BackupCommands::Share { repo, keep_identities } => with_repo(&repo, LockKind::Exclusive, "enable_sharing", |r, _| {
            ok(emit(format, "Sharing", &ufop_core::backup::shared::enable_sharing(r, &repo.identity(), !keep_identities)?))?;
            Ok(0)
        }),
        BackupCommands::Find { repo, pattern, version } => with_repo(&repo, LockKind::Shared, "find", |_, db| {
            ok(emit(format, "Matches", &maintenance::find(db, &pattern, version, 10_000)?))?;
            Ok(0)
        }),
        BackupCommands::Compare { repo, base, compare } => with_repo(&repo, LockKind::Shared, "compare", |_, db| {
            ok(emit(format, "Differences", &maintenance::compare_versions(db, base, compare)?))?;
            Ok(0)
        }),
        BackupCommands::Restore {
            repo, paths, version, target, conflict, restore_permissions, preserve_full_paths, skip_verify, versions, unique, use_local_blocks,
            download_threads, file_threads, legacy, tempdir, skip_free_space_check, no_metadata, extra,
        } => {
            let conflict = match conflict.as_str() {
                "rename" => ConflictPolicy::Rename,
                "skip" => ConflictPolicy::Skip,
                _ => ConflictPolicy::Overwrite,
            };
            let mut opts = RestoreOptions {
                versions, unique_versions: unique, paths, target, conflict, restore_permissions, preserve_full_paths, skip_verify, dry_run,
                use_local_blocks, download_threads, file_threads, legacy, temp_dir: tempdir, skip_free_space_check, restore_metadata: !no_metadata,
                source: repo.source_id.clone(),
                ..Default::default()
            };
            extra.apply(&version, &mut opts)?;
            let cache = extra.cache()?;
            // No local database? Rebuild a temporary one from the destination.
            let needs_recreate = !repo.db_path().exists();
            let be = repo.backend();
            let pass = repo.passphrase();
            let ctx = progress_ctx(format);
            let rep = repo.locked(be.as_ref(), Some(LockKind::Shared), "restore", &ctx, || {
                let db = if needs_recreate { BackupDb::open_in_memory()? } else { repo.open_db(be.as_ref(), &ctx)? };
                if needs_recreate {
                    maintenance::recreate_database(be.as_ref(), &db, pass.as_deref(), &repo.prefix, &OpContext::new())?;
                }
                let r = Repo::open(be.as_ref(), &db, pass.as_deref(), &repo.options(), false, KdfParams::default())?.with_cache(cache);
                if opts.delete_extraneous && !opts.dry_run {
                    // List what the mirror deletes and ask first.
                    let plan = run_restore(&r, &RestoreOptions { dry_run: true, ..opts.clone() }, &OpContext::new())?;
                    if !super::backup_access::confirm_mirror(&plan.planned_deletions, extra.yes)? {
                        return Err(ufop_core::error::AppError::state("Mirror restore cancelled", "Nothing was restored or deleted."));
                    }
                }
                run_restore(&r, &opts, &ctx)
            })?;
            log_outcome("restore", &rep.warnings, &rep.errors);
            ok(emit(format, "Restore finished", &rep))?;
            Ok(if rep.errors.is_empty() { 0 } else { 1 })
        }
        BackupCommands::Cat { repo, path, version, offset, length, cache } => {
            let range = (offset.is_some() || length.is_some()).then(|| (offset.unwrap_or(0), length.unwrap_or(u64::MAX)));
            super::backup_access::cat(&repo, &path, &version, &cache, range, &mut std::io::stdout().lock())
        }
        BackupCommands::Mount { repo, version, all, bind, port, seconds, cache } => super::backup_access::mount(&repo, &version, all, &bind, port, &cache, seconds, format),
        BackupCommands::Cache { cmd } => super::backup_access::cache_command(cmd, format),
        BackupCommands::Verify { repo, full, samples, versions, paths, percent, structure, files_percent, parallel, max_errors, use_cache, cache } => {
            use ufop_core::backup::verify::{VerifyMode, VerifyOptions, VerifyScope};
            let mode = if structure {
                VerifyMode::Structure
            } else if full {
                VerifyMode::Full
            } else if let Some(p) = percent {
                VerifyMode::SamplePercent { percent: p }
            } else {
                VerifyMode::SampleVolumes { count: samples.unwrap_or(3) }
            };
            let opts = VerifyOptions { scope: VerifyScope { versions, paths }, mode, files_percent, parallelism: parallel, max_errors, use_cache };
            let vc = if use_cache { cache.open()? } else { None };
            let ctx = progress_ctx(format);
            with_repo_cache(&repo, LockKind::Shared, "verify", vc, |r, _| {
                let v = ufop_core::backup::verify::verify(r, &opts, &ctx)?;
                if matches!(format, OutputFormat::Human) {
                    eprint!("\r\x1b[2K");
                    print!("{}", verify_text(&v));
                } else {
                    output::print_data(format, &v);
                }
                Ok(if v.problems.is_empty() { 0 } else { 2 })
            })
        }
        BackupCommands::Compact { repo } => with_repo(&repo, LockKind::Exclusive, "compact", |r, _| {
            ok(emit(format, "Compaction", &maintenance::compact(r, &repo.options(), &OpContext::new())?))?;
            Ok(0)
        }),
        BackupCommands::Repair { repo } => with_repo(&repo, LockKind::Exclusive, "repair", |r, _| {
            let rep = maintenance::repair(r)?;
            ok(emit(format, "Repair", &rep))?;
            Ok(if rep.unrecoverable.is_empty() { 0 } else { 1 })
        }),
        BackupCommands::Recreate { repo } => {
            let be = repo.backend();
            let path = repo.db_path();
            let ctx = OpContext::new();
            let rep = repo.locked(be.as_ref(), Some(LockKind::Shared), "recreate_database", &ctx, || {
                let db = repo.open_db(be.as_ref(), &ctx)?;
                maintenance::recreate_database(be.as_ref(), &db, repo.passphrase().as_deref(), &repo.prefix, &ctx)
            })?;
            ok(emit(format, &format!("Database recreated at {}", path.display()), &rep))?;
            Ok(0)
        }
        BackupCommands::ListBrokenFiles { repo } => with_repo(&repo, LockKind::Shared, "list_broken_files", |r, _| {
            let b = maintenance::list_broken_files(r)?;
            ok(emit(format, "Broken files", &b))?;
            Ok(if b.is_empty() { 0 } else { 1 })
        }),
        BackupCommands::PurgeBrokenFiles { repo } => with_repo(&repo, if dry_run { LockKind::Shared } else { LockKind::Exclusive }, "purge_broken_files", |r, _| {
            ok(emit(format, "Purge broken files", &maintenance::purge_broken_files(r, dry_run)?))?;
            Ok(0)
        }),
        BackupCommands::Purge { repo, paths } => with_repo(&repo, if dry_run { LockKind::Shared } else { LockKind::Exclusive }, "purge", |r, _| {
            ok(emit(format, "Purge", &maintenance::purge(r, &paths, None, dry_run)?))?;
            Ok(0)
        }),
        BackupCommands::Delete { repo, versions, allow_full_removal, force } => with_repo(&repo, LockKind::Exclusive, "delete_versions", |r, _| {
            let n = maintenance::delete_versions_ex(r, &versions, allow_full_removal, force)?;
            ok(emit(format, "Deleted versions", &serde_json::json!({ "versions_deleted": n })))?;
            Ok(0)
        }),
        BackupCommands::RebuildMissing { repo } => with_repo(&repo, LockKind::Exclusive, "rebuild_missing", |r, _| {
            let rep = maintenance::rebuild_missing_block_volumes(r, &repo.options(), &progress_ctx(format))?;
            ok(emit(format, "Rebuild missing volumes", &rep))?;
            Ok(if rep.incomplete_volumes.is_empty() { 0 } else { 1 })
        }),
        BackupCommands::ControlFiles { repo, target, version } => with_repo(&repo, LockKind::Shared, "extract_control_files", |r, _| {
            ok(emit(format, "Control files", &maintenance::extract_control_files(r, version, &target)?))?;
            Ok(0)
        }),
        BackupCommands::Mirror { dest, to, prefix, keep_extraneous, verify, threads } => {
            let o = ufop_core::backup::mirror::MirrorOptions { delete_extraneous: !keep_extraneous, verify, dry_run, threads };
            let lo = LockOptions { app: LOCK_APP.into(), ..Default::default() };
            let r = ufop_core::backup::mirror::mirror_locked(&LocalBackend::new(&dest), &LocalBackend::new(&to), &prefix, &o, &lo, &progress_ctx(format))?;
            log_outcome("mirror", &r.warnings, &r.errors);
            ok(emit(format, "Mirror", &r))?;
            Ok(if r.errors.is_empty() { 0 } else { 2 })
        }
        BackupCommands::Recovery { cmd } => {
            use ufop_core::backup::recovery;
            let ctx = progress_ctx(format);
            match cmd {
                RecoveryCommands::Download { repo, folder, threads, no_resume, max_kbps } => {
                    let o = recovery::DownloadOptions { threads, resume: !no_resume, max_bytes_per_sec: max_kbps * 1024 };
                    let be = repo.backend();
                    let r = repo.locked(be.as_ref(), Some(LockKind::Shared), "recovery_download", &ctx, || {
                        recovery::download(be.as_ref(), repo.passphrase().as_deref(), &repo.prefix, &folder, &o, &ctx)
                    })?;
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
                    let target = LocalBackend::new(&out);
                    target.create_folder()?;
                    let lo = LockOptions { app: LOCK_APP.into(), ..Default::default() };
                    let (r, w) = repo_lock::with_lock(&target, &o.prefix.clone(), LockKind::Exclusive, "convert", &lo, &ctx, || recovery::convert(&folder, &target, &o, &ctx))?;
                    log_outcome("convert", &w, &[]);
                    ok(emit(format, "Converted repository", &r))?;
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
            DbCommands::Sanitize { dbpath, out, db_key_source } => {
                let (key, _) = db_key_from_source(&db_key_source, &dbpath)?;
                let r = BackupDb::open_with_key(&dbpath, key.as_ref())?.export_sanitized(&out)?;
                ok(emit(format, "Sanitized copy written", &serde_json::json!({ "out": out, "paths_obfuscated": r.paths_obfuscated, "settings_removed": r.settings_removed })))?;
                Ok(0)
            }
            DbCommands::Encrypt { dbpath, db_key_source, decrypt } => {
                let (key, _) = db_key_from_source(&db_key_source, &dbpath)?;
                let key = key.ok_or_else(|| ufop_core::error::AppError::validation("Choose a key with --db-key-source keyring or env:VAR"))?;
                let mut db = BackupDb::open_with_key(&dbpath, Some(&key))?;
                let r = db.rekey(if decrypt { None } else { Some(&key) })?;
                ok(emit(format, if decrypt { "Database decrypted" } else { "Database encrypted" }, &r))?;
                Ok(0)
            }
        },
        BackupCommands::Lock { cmd } => match cmd {
            LockCommands::Status { repo } => {
                let st = repo_lock::status(repo.backend().as_ref(), &repo.prefix)?;
                if matches!(format, OutputFormat::Human) {
                    eprintln!("{}", st.summary().unwrap_or_else(|| "The repository is not in use".into()));
                }
                ok(emit(format, "Repository locks", &st))?;
                Ok(if st.exclusive || st.readers > 0 { 1 } else { 0 })
            }
            LockCommands::Break { repo, yes, owner_id } => {
                let r = repo_lock::break_lock(repo.backend().as_ref(), &repo.prefix, yes, owner_id.as_deref())?;
                ok(emit(format, "Repository lock broken", &r))?;
                Ok(0)
            }
        },
        BackupCommands::SnapshotTest { sources, engine } => {
            let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
            let d = ufop_core::backup::snapshot::diagnose(&srcs, &engine.snapshot_options());
            ok(emit(format, "Snapshot test", &d))?;
            Ok(if d.ok { 0 } else { 2 })
        }
        BackupCommands::AutoTune { repo, samples } => with_repo(&repo, LockKind::Shared, "autotune", |r, _| {
            ok(emit(format, "Restore AutoTune", &ufop_core::backup::restore::autotune(r, samples.clamp(1, 50), &OpContext::new())?))?;
            Ok(0)
        }),
        BackupCommands::Upgrade { repo, rollback } => {
            use ufop_core::backup::format;
            let be = repo.backend();
            let r = repo.locked(be.as_ref(), Some(LockKind::Exclusive), "upgrade_format", &OpContext::new(), || {
                if rollback {
                    format::rollback_upgrade(be.as_ref(), &repo.prefix)
                } else {
                    format::upgrade_repository(be.as_ref(), &repo.prefix, repo.passphrase().as_deref(), ufop_core::backup::REPO_FORMAT_VERSION)
                }
            })?;
            ok(emit(format, if rollback { "Rollback" } else { "Upgrade" }, &r))?;
            Ok(0)
        }
        BackupCommands::ChangePassword { repo, new_passphrase_env, new_passphrase_file, kdf_preset } => {
            let current = repo.passphrase().ok_or_else(|| ufop_core::error::AppError::validation(format!("Set the current passphrase ({})", repo.passphrase_env)))?;
            let new = match &new_passphrase_file {
                Some(f) => std::fs::read_to_string(f)?.lines().next().map(|l| l.trim_end_matches('\r').to_string()),
                None => std::env::var(&new_passphrase_env).ok(),
            }
            .filter(|p| !p.is_empty())
            .ok_or_else(|| ufop_core::error::AppError::validation(format!("Set the new passphrase ({new_passphrase_env} or --new-passphrase-file)")))?;
            let kdf = match kdf_preset.as_deref() {
                Some(p) => Some(KdfParams::preset(p).ok_or_else(|| ufop_core::error::AppError::validation(format!("Unknown --kdf-preset '{p}' (interactive, default, sensitive)")))?),
                None => None,
            };
            let be = repo.backend();
            let r = repo.locked(be.as_ref(), Some(LockKind::Exclusive), "change_password", &OpContext::new(), || {
                ufop_core::backup::format::change_password(be.as_ref(), &repo.prefix, &current, &new, kdf)
            })?;
            ok(emit(format, "Passphrase changed", &r))?;
            Ok(0)
        }
        BackupCommands::Reshard { repo, layout, shard_prefix_len } => {
            let l = parse_layout(&layout, shard_prefix_len)?;
            let be = repo.backend();
            let ctx = progress_ctx(format);
            let kind = if dry_run { LockKind::Shared } else { LockKind::Exclusive };
            let r = repo.locked(be.as_ref(), Some(kind), "reshard", &ctx, || ufop_core::backup::layout::reshard(be.as_ref(), &repo.prefix, l, dry_run, &ctx))?;
            ok(emit(format, "Reshard", &r))?;
            Ok(if r.errors.is_empty() { 0 } else { 2 })
        }
        BackupCommands::FormatInfo { repo } => {
            ok(emit(format, "Repository format", &ufop_core::backup::format::format_info(repo.backend().as_ref(), &repo.prefix)?))?;
            Ok(0)
        }
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
    fn sizes_and_run_flags() {
        assert_eq!(parse_bytes("4096").unwrap(), 4096);
        assert_eq!(parse_bytes("500M").unwrap(), 500 << 20);
        assert_eq!(parse_bytes("20GB").unwrap(), 20 << 30);
        assert_eq!(parse_bytes("1.5GiB").unwrap(), 3 << 29);
        assert!(parse_bytes("ten").is_err() && parse_bytes("5X").is_err());
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCommands,
        }
        let c = <T as clap::Parser>::try_parse_from([
            "t", "run", "--dest", "/d", "--source", "/s", "--ignore-file", ".a", "--ignore-file", ".b", "--respect-gitignore", "--one-file-system", "--fail-fast",
            "--on-file-error", "fail", "--on-dir-error", "fail", "--on-unknown-entry", "warn", "--max-errors", "3", "--rehash-percent", "10", "--max-upload", "1G",
            "--parallel-file-threshold", "0", "--snapshot-provider", "zfs",
        ])
        .unwrap()
        .c;
        let BackupCommands::Run { engine, ignore, .. } = c else { panic!("expected run") };
        let o = engine.apply(BackupOptions::default()).unwrap();
        use ufop_core::backup::errors::{ErrorAction, UnknownEntryAction};
        assert!(o.one_file_system && o.error_policy.fail_fast);
        assert_eq!((o.error_policy.on_file_error, o.error_policy.on_dir_error, o.error_policy.on_unknown_entry), (ErrorAction::Fail, ErrorAction::Fail, UnknownEntryAction::Warn));
        assert_eq!((o.error_policy.max_errors, o.rehash_percent, o.max_upload_bytes_per_run, o.parallel_file_threshold), (Some(3), 10, Some(1 << 30), 0));
        assert_eq!(o.snapshot.provider, ufop_core::backup::snapshot::SnapshotProvider::Zfs);
        let mut f = SourceFilter::default();
        ignore.apply(&mut f);
        assert_eq!((f.ignore_files, f.respect_gitignore), (vec![".a".to_string(), ".b".to_string()], true));
        // Defaults keep today's behaviour.
        let c = <T as clap::Parser>::try_parse_from(["t", "run", "--dest", "/d", "--source", "/s"]).unwrap().c;
        let BackupCommands::Run { engine, ignore, .. } = c else { panic!() };
        let o = engine.apply(BackupOptions::default()).unwrap();
        assert_eq!(o.error_policy, Default::default());
        assert_eq!(o.parallel_file_threshold, ufop_core::backup::DEFAULT_PARALLEL_FILE_THRESHOLD);
        let mut f = SourceFilter::default();
        ignore.apply(&mut f);
        assert_eq!(f.ignore_files, vec![".ufopignore".to_string()]);
        assert!(<T as clap::Parser>::try_parse_from(["t", "run-all", "--job", "Docs", "--scheduled-only", "--wait"]).is_ok());
        assert!(<T as clap::Parser>::try_parse_from(["t", "stdin", "--dest", "/d", "--name", "x"]).is_ok());
    }

    #[test]
    fn estimate_text_says_at_most() {
        let e = ufop_core::backup::estimate::BackupEstimate {
            files: 3,
            new_files: 1,
            changed_files: 1,
            new_bytes_at_most: 5 << 20,
            new_bytes_at_least: 1 << 20,
            upper_bound: true,
            bytes_per_sec: Some(1 << 20),
            speed_source: "history".into(),
            estimated_duration_secs: Some(65),
            ..Default::default()
        };
        let t = estimate_text(&e);
        assert!(t.contains("at most 5.0 MB") || t.contains("at most 5"), "{t}");
        assert!(t.contains("1m 5s") && t.contains("history"), "{t}");
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
    fn restore_by_time_archive_mirror_cat_mount_and_cache() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), b"first").unwrap();
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCommands,
        }
        let parse = |args: &[&str]| <T as clap::Parser>::try_parse_from(std::iter::once("t").chain(args.iter().copied())).unwrap().c;
        let dest = t.path().join("dest").to_string_lossy().into_owned();
        let db = t.path().join("db.sqlite").to_string_lossy().into_owned();
        let cache = t.path().join("cache").to_string_lossy().into_owned();
        let src_s = src.to_string_lossy().into_owned();
        let backup = || run(parse(&["run", "--dest", &dest, "--dbpath", &db, "--passphrase", "pw", "--source", &src_s, "--block-size", "4096", "--no-quota-warning"]), &OutputFormat::Json, false).unwrap();
        assert_eq!(backup(), 0);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let between = chrono::Utc::now().to_rfc3339();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(src.join("f.txt"), b"second").unwrap();
        assert_eq!(backup(), 0);
        let repo = ["--dest", dest.as_str(), "--dbpath", db.as_str(), "--passphrase", "pw"];
        let with = |cmd: &[&str], more: &[&str]| -> Vec<String> { cmd.iter().chain(repo.iter()).chain(more.iter()).map(|s| s.to_string()).collect() };
        let p = |v: Vec<String>| parse(&v.iter().map(String::as_str).collect::<Vec<_>>());
        // --time picks the older version; --version oldest too.
        let out = t.path().join("out");
        let out_s = out.to_string_lossy().into_owned();
        assert_eq!(run(p(with(&["restore"], &["--target", &out_s, "--time", &between])), &OutputFormat::Json, false).unwrap(), 0);
        assert_eq!(std::fs::read(out.join("src/f.txt")).unwrap(), b"first");
        let out2 = t.path().join("out2").to_string_lossy().into_owned();
        assert_eq!(run(p(with(&["restore"], &["--target", &out2, "--version", "oldest"])), &OutputFormat::Json, false).unwrap(), 0);
        assert_eq!(std::fs::read(t.path().join("out2/src/f.txt")).unwrap(), b"first");
        // An archive.
        let zip = t.path().join("v.zip").to_string_lossy().into_owned();
        assert_eq!(run(p(with(&["restore"], &["--archive", &zip, "--version", "latest"])), &OutputFormat::Json, false).unwrap(), 0);
        let mut z = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
        let mut body = String::new();
        std::io::Read::read_to_string(&mut z.by_name("src/f.txt").unwrap(), &mut body).unwrap();
        assert_eq!(body, "second");
        // Mirror: refused without --yes when not interactive, deletes with it.
        std::fs::write(out.join("src/extra.tmp"), b"x").unwrap();
        assert_eq!(run(p(with(&["restore"], &["--target", &out_s, "--mirror"])), &OutputFormat::Json, false).unwrap_err().user_message().contains("--yes"), true);
        assert!(out.join("src/extra.tmp").exists());
        assert_eq!(run(p(with(&["restore"], &["--target", &out_s, "--mirror", "--yes"])), &OutputFormat::Json, false).unwrap(), 0);
        assert!(!out.join("src/extra.tmp").exists());
        assert_eq!(std::fs::read(out.join("src/f.txt")).unwrap(), b"second");
        // cat, with a range and through the cache.
        let path = src.join("f.txt").to_string_lossy().into_owned();
        let BackupCommands::Cat { repo: r, path: fp, version, offset: _, length: _, cache: c } = p(with(&["cat"], &[&path, "--time", &between, "--cache-dir", &cache])) else { panic!() };
        let mut buf = Vec::new();
        assert_eq!(super::super::backup_access::cat(&r, &fp, &version, &c, None, &mut buf).unwrap(), 0);
        assert_eq!(buf, b"first");
        let mut buf = Vec::new();
        super::super::backup_access::cat(&r, &fp, &super::super::backup_access::VersionArgs { version: "latest".into(), ..Default::default() }, &c, Some((1, 3)), &mut buf).unwrap();
        assert_eq!(buf, b"eco");
        assert_eq!(run(p(vec!["cache".into(), "stats".into(), "--cache-dir".into(), cache.clone()]), &OutputFormat::Json, false).unwrap(), 0);
        assert!(std::fs::read_dir(&cache).unwrap().count() > 0, "cat filled the cache");
        assert_eq!(run(p(vec!["cache".into(), "clear".into(), "--cache-dir".into(), cache.clone()]), &OutputFormat::Json, false).unwrap(), 0);
        // mount: the drive serves the file.
        let BackupCommands::Mount { repo: r, version, all, cache: c, .. } = p(with(&["mount"], &["--version", "latest", "--cache-size", "0"])) else { panic!() };
        let (srv, label) = super::super::backup_access::start_drive(&r, &version, all, "127.0.0.1:0".parse().unwrap(), &c).unwrap();
        assert!(label.starts_with("version 0"));
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(srv.addr()).unwrap();
        write!(s, "GET /{}/src/f.txt HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", srv.token()).unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200") && raw.ends_with("second"), "{raw}");
        srv.stop();
        assert_eq!(run(p(with(&["mount"], &["--all", "--seconds", "0", "--cache-size", "0"])), &OutputFormat::Json, false).unwrap(), 0);
    }

    #[test]
    fn format_flags_upgrade_reshard_and_change_password() {
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
        let base = ["--dest", dest.as_str(), "--dbpath", db.as_str(), "--passphrase", "pw"];
        let with = |cmd: &str, extra: &[&str]| -> Vec<String> { std::iter::once(cmd).chain(base.iter().copied()).chain(extra.iter().copied()).map(String::from).collect() };
        let p = |v: Vec<String>| parse(&v.iter().map(String::as_str).collect::<Vec<_>>());
        match p(with("run", &["--source", &src_s, "--cipher", "chacha20-poly1305", "--chunker", "fixed", "--block-codec", "deflate", "--layout", "sharded", "--shard-prefix-len", "1"])) {
            BackupCommands::Run { engine, .. } => {
                let o = engine.apply(BackupOptions::default()).unwrap();
                assert_eq!(o.cipher, ufop_core::backup::crypto::CipherKind::ChaCha20Poly1305);
                assert_eq!(o.chunker, Some(ufop_core::backup::ChunkerKind::Fixed));
                assert_eq!(o.layout, ufop_core::backup::layout::RepoLayout::Sharded { prefix_len: 1 });
            }
            _ => panic!("expected run"),
        }
        // A format 1 backup, upgraded, resharded, password changed, restored.
        assert_eq!(run(p(with("run", &["--source", &src_s, "--format-version", "1", "--no-quota-warning"])), &OutputFormat::Json, false).unwrap(), 0);
        let info = |d: &str| ufop_core::backup::format::format_info(&LocalBackend::new(d), "ufop").unwrap();
        assert_eq!(info(&dest).format_version, 1);
        assert_eq!(run(p(with("upgrade", &[])), &OutputFormat::Json, false).unwrap(), 0);
        assert!(info(&dest).rollback_available);
        assert_eq!(run(p(with("upgrade", &["--rollback"])), &OutputFormat::Json, false).unwrap(), 0);
        assert_eq!(info(&dest).format_version, 1);
        assert_eq!(run(p(with("upgrade", &[])), &OutputFormat::Json, false).unwrap(), 0);
        assert_eq!(run(p(with("reshard", &["--layout", "sharded"])), &OutputFormat::Json, true).unwrap(), 0);
        assert_eq!(info(&dest).layout, ufop_core::backup::layout::RepoLayout::Flat, "dry run");
        assert_eq!(run(p(with("reshard", &["--layout", "sharded"])), &OutputFormat::Json, false).unwrap(), 0);
        assert!(info(&dest).layout.is_sharded());
        let newpw = t.path().join("new.txt");
        std::fs::write(&newpw, "pw2\n").unwrap();
        let newpw_s = newpw.to_string_lossy().into_owned();
        assert_eq!(run(p(with("change-password", &["--new-passphrase-file", &newpw_s])), &OutputFormat::Json, false).unwrap(), 0);
        assert!(run(p(with("format-info", &[])), &OutputFormat::Json, false).is_ok());
        let out = t.path().join("out").to_string_lossy().into_owned();
        let missing = t.path().join("missing.sqlite").to_string_lossy().into_owned();
        let code = run(parse(&["restore", "--dest", &dest, "--dbpath", &missing, "--passphrase", "pw2", "--target", &out]), &OutputFormat::Json, false).unwrap();
        assert_eq!(code, 0);
        assert_eq!(std::fs::read(t.path().join("out/src/f.txt")).unwrap(), b"cli");
    }

    #[test]
    fn versions_pins_retention_copy_and_sharing_commands() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCommands,
        }
        let parse = |args: &[String]| <T as clap::Parser>::try_parse_from(std::iter::once("t".to_string()).chain(args.iter().cloned())).unwrap().c;
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let (dest, db, src_s) = (s(&t.path().join("dest")), s(&t.path().join("db.sqlite")), s(&src));
        let base = |cmd: &str, extra: &[&str]| -> Vec<String> {
            [cmd, "--dest", &dest, "--dbpath", &db, "--passphrase", "pw"].iter().chain(extra.iter()).map(|x| x.to_string()).collect()
        };
        let go = |args: Vec<String>, dry: bool| run(parse(&args), &OutputFormat::Json, dry).unwrap();
        for i in 0..3u8 {
            std::fs::write(src.join("f.txt"), [i; 10]).unwrap();
            assert_eq!(go(base("run", &["--source", &src_s, "--note", "nightly", "--tag", "env=prod", "--shared", "--job-id", "laptop-docs", "--no-quota-warning"]), false), 0);
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let open = || BackupDb::open(std::path::Path::new(&db)).unwrap();
        let v = open().all_filesets().unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].note.as_deref(), v[0].tags.get("env").map(String::as_str)), (Some("nightly"), Some("prod")));
        assert_eq!(v[0].identity.as_ref().unwrap().job_id, "laptop-docs");
        assert!(ufop_core::backup::shared::repository_is_shared(&LocalBackend::new(&dest), "ufop").unwrap());
        let job = ["--job-id", "laptop-docs"];
        assert_eq!(go(base("list", &[&job[..], &["--tag", "env=prod"]].concat()), false), 0);
        assert!(version_table(&v).contains("P") || !v[0].pinned);
        // Pin the oldest, preview and apply GFS retention, delete needs --force.
        assert_eq!(go(base("pin", &[&job[..], &["2", "--reason", "audit"]].concat()), false), 0);
        assert!(open().all_filesets().unwrap()[2].pinned);
        assert_eq!(go(base("annotate", &[&job[..], &["0", "--note", "latest", "--tag", "keep"]].concat()), false), 0);
        assert_eq!(open().all_filesets().unwrap()[0].note.as_deref(), Some("latest"));
        assert_eq!(go(base("retention", &[]).into_iter().take(1).chain(base("preview", &[&job[..], &["--keep-latest", "1"]].concat())).collect(), false), 0);
        assert_eq!(open().all_filesets().unwrap().len(), 3, "preview changes nothing");
        assert_eq!(go(base("retention", &[]).into_iter().take(1).chain(base("apply", &[&job[..], &["--keep-latest", "1"]].concat())).collect(), false), 0);
        assert_eq!(open().all_filesets().unwrap().len(), 2, "newest and pinned kept");
        assert!(run(parse(&base("delete", &[&job[..], &["1"]].concat())), &OutputFormat::Json, false).is_err(), "pinned: refused");
        assert_eq!(go(base("unpin", &[&job[..], &["1"]].concat()), false), 0);
        assert_eq!(go(base("sources", &job), false), 0);
        // Copy the newest version into a new format 1 repository (re-cut).
        let to = s(&t.path().join("copy"));
        let todb = s(&t.path().join("copy.sqlite"));
        std::env::set_var("UFOP_TO_PASSPHRASE_TEST", "pw2");
        let copy = base("copy-versions", &[&job[..], &["0", "--to", &to, "--to-dbpath", &todb, "--to-passphrase-env", "UFOP_TO_PASSPHRASE_TEST", "--to-format-version", "1"]].concat());
        assert_eq!(go(copy.clone(), true), 0);
        assert!(BackupDb::open(std::path::Path::new(&todb)).unwrap().all_filesets().unwrap().is_empty(), "dry run");
        assert_eq!(go(copy, false), 0);
        let copied = BackupDb::open(std::path::Path::new(&todb)).unwrap().all_filesets().unwrap();
        assert_eq!(copied[0].note.as_deref(), Some("latest"));
        assert_eq!(ufop_core::backup::format::format_info(&LocalBackend::new(&to), "ufop").unwrap().format_version, 1);
        // A second CLI job on the shared repository sees only its own versions.
        let other = ["--job-id", "nas-photos"];
        assert_eq!(go(base("run", &[&other[..], &["--source", &src_s, "--no-quota-warning"]].concat()), false), 0);
        let db2 = BackupDb::open(std::path::Path::new(&db)).unwrap();
        ufop_core::backup::shared::scope_db(&db2, &ufop_core::backup::versions::ClientIdentity { job_id: "nas-photos".into(), ..Default::default() }).unwrap();
        assert_eq!(db2.filesets().unwrap().len(), 1);
        assert_eq!(db2.all_filesets().unwrap().len(), 3);
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

    #[test]
    fn lock_read_only_and_encrypted_database_flags() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("zzCliSecret.txt"), b"cli").unwrap();
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCommands,
        }
        let parse = |args: &[&str]| <T as clap::Parser>::try_parse_from(std::iter::once("t").chain(args.iter().copied())).unwrap().c;
        let dest = t.path().join("dest").to_string_lossy().into_owned();
        let db = t.path().join("db.sqlite");
        let db_s = db.to_string_lossy().into_owned();
        let src_s = src.to_string_lossy().into_owned();
        std::env::set_var("UFOP_TEST_DB_KEY", ufop_core::backup::db::DbKey::generate().to_hex());
        let base = ["--dest", dest.as_str(), "--dbpath", db_s.as_str(), "--passphrase", "pw"];
        let with = |cmd: &[&str], extra: &[&str]| -> Vec<String> { cmd.iter().copied().chain(base.iter().copied()).chain(extra.iter().copied()).map(String::from).collect() };
        let p = |v: Vec<String>| parse(&v.iter().map(String::as_str).collect::<Vec<_>>());
        // Encrypted local database from an environment key, with checkpoint flags.
        let code = run(p(with(&["run"], &["--source", &src_s, "--no-quota-warning", "--db-key-source", "env:UFOP_TEST_DB_KEY", "--checkpoint-interval-bytes", "1"])), &OutputFormat::Json, false).unwrap();
        assert_eq!(code, 0);
        assert!(BackupDb::inspect(&db).unwrap().encrypted);
        assert!(!String::from_utf8_lossy(&std::fs::read(&db).unwrap()).contains("zzCliSecret"));
        assert_eq!(run(p(with(&["list"], &["--db-key-source", "env:UFOP_TEST_DB_KEY"])), &OutputFormat::Json, false).unwrap(), 0);
        // Without the key the database is rebuilt (plain) from the destination.
        assert_eq!(run(p(with(&["list"], &[])), &OutputFormat::Json, false).unwrap(), 0);
        assert!(!BackupDb::inspect(&db).unwrap().encrypted);
        assert!(run(p(with(&["list"], &["--db-key-source", "bogus"])), &OutputFormat::Json, false).is_err());

        // Lock status, contention and break.
        assert_eq!(run(p(with(&["lock", "status"], &[])), &OutputFormat::Json, false).unwrap(), 0, "free");
        let other = LocalBackend::new(&dest);
        let held = repo_lock::RepoLock::acquire(&other, "ufop", LockKind::Exclusive, "compact", &LockOptions::default()).unwrap();
        assert_eq!(run(p(with(&["lock", "status"], &[])), &OutputFormat::Json, false).unwrap(), 1, "in use");
        let e = run(p(with(&["run"], &["--source", &src_s, "--no-quota-warning"])), &OutputFormat::Json, false).unwrap_err();
        assert!(e.user_message().contains("Repository in use by"), "{e}");
        assert!(run(p(with(&["lock", "break"], &[])), &OutputFormat::Json, false).is_err(), "needs --yes");
        let owner = held.info().owner_id;
        assert_eq!(run(p(with(&["lock", "break"], &["--yes", "--owner-id", &owner])), &OutputFormat::Json, false).unwrap(), 0);
        drop(held);

        // Read-only: restore works and writes nothing; backups are refused.
        let before: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name()).collect();
        let e = run(p(with(&["run"], &["--source", &src_s, "--no-quota-warning", "--read-only"])), &OutputFormat::Json, false).unwrap_err();
        assert!(e.user_message().contains("read-only"), "{e}");
        let out = t.path().join("out").to_string_lossy().into_owned();
        assert_eq!(run(p(with(&["restore"], &["--read-only", "--target", &out])), &OutputFormat::Json, false).unwrap(), 0);
        assert_eq!(std::fs::read(t.path().join("out/src/zzCliSecret.txt")).unwrap(), b"cli");
        let after: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(before.len(), after.len());
        assert!(run(p(with(&["compact"], &["--read-only"])), &OutputFormat::Json, false).is_err());

        // db encrypt / decrypt on demand.
        assert_eq!(run(parse(&["db", "encrypt", &db_s, "--db-key-source", "env:UFOP_TEST_DB_KEY"]), &OutputFormat::Json, false).unwrap(), 0);
        assert!(BackupDb::inspect(&db).unwrap().encrypted);
        assert_eq!(run(parse(&["db", "encrypt", &db_s, "--db-key-source", "env:UFOP_TEST_DB_KEY", "--decrypt"]), &OutputFormat::Json, false).unwrap(), 0);
        assert!(!BackupDb::inspect(&db).unwrap().encrypted);
    }
}
