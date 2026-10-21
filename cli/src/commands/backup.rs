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
    /// Store volumes unencrypted
    #[arg(long)]
    pub no_encryption: bool,
    /// Remote file-name prefix
    #[arg(long, default_value = "ufop")]
    pub prefix: String,
}

impl RepoArgs {
    fn passphrase(&self) -> Option<String> {
        if self.no_encryption {
            return None;
        }
        self.passphrase.clone().or_else(|| std::env::var(&self.passphrase_env).ok()).filter(|p| !p.is_empty())
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

#[derive(Subcommand, Debug)]
pub enum BackupCommands {
    /// Back up sources into the destination (creates it on first run)
    Run {
        #[command(flatten)]
        repo: RepoArgs,
        /// Source folder or file (repeatable)
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
    if matches!(format, OutputFormat::Human) {
        cancel_on_ctrl_c(ctx.clone());
        ctx.with_progress(|p| {
            if let Some(path) = &p.current_path {
                eprint!("\r\x1b[2K{:?} {}/{} {}", p.phase, p.files_processed, p.files_total, truncate(path, 70));
            }
        })
    } else {
        ctx
    }
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
            keep_versions, keep_days, retention_policy, verify_samples, upload_unchanged, no_auto_compact,
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
            let opts = BackupOptions {
                block_size,
                volume_size,
                compression_level,
                verify_samples,
                upload_unchanged_backups: upload_unchanged,
                auto_compact: !no_auto_compact,
                dry_run,
                ..repo.options()
            };
            let retention = parse_retention(keep_versions, keep_days, retention_policy);
            let be = LocalBackend::new(&repo.dest);
            let db = BackupDb::open(&repo.db_path())?;
            let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
            let r = run_backup(&be, &db, &srcs, &filter, &opts, &retention, pass.as_deref(), KdfParams::default(), &progress_ctx(format))?;
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
        BackupCommands::Restore { repo, paths, version, target, conflict, restore_permissions, preserve_full_paths, skip_verify } => {
            let conflict = match conflict.as_str() {
                "rename" => ConflictPolicy::Rename,
                "skip" => ConflictPolicy::Skip,
                _ => ConflictPolicy::Overwrite,
            };
            let opts = RestoreOptions { version, paths, target, conflict, restore_permissions, preserve_full_paths, skip_verify, dry_run, ..Default::default() };
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
        let repo = RepoArgs {
            dest: t.path().join("dest").to_string_lossy().into(),
            dbpath: Some(t.path().join("db.sqlite")),
            passphrase: Some("pw".into()),
            passphrase_env: "UNUSED".into(),
            no_encryption: false,
            prefix: "ufop".into(),
        };
        let code = run(
            BackupCommands::Run {
                repo: repo.clone(), sources: vec![src.to_string_lossy().into()], include: vec![], exclude: vec!["*.tmp".into()],
                exclude_hidden: false, skip_files_larger_than: None, block_size: 4096, volume_size: 1 << 20, compression_level: 6,
                keep_versions: None, keep_days: None, retention_policy: None, verify_samples: 1, upload_unchanged: false, no_auto_compact: false,
            },
            &OutputFormat::Json,
            false,
        )
        .unwrap();
        assert_eq!(code, 0);
        let fresh = RepoArgs { dbpath: Some(t.path().join("missing.sqlite")), ..repo };
        let out = t.path().join("out");
        let code = run(
            BackupCommands::Restore { repo: fresh, paths: vec![], version: 0, target: Some(out.clone()), conflict: "overwrite".into(),
                restore_permissions: false, preserve_full_paths: false, skip_verify: false },
            &OutputFormat::Json,
            false,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert_eq!(std::fs::read(out.join("src/f.txt")).unwrap(), b"cli");
    }
}
