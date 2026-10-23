//! `ufop backup` reading without restoring, and the restore options added
//! with it: restore as of a time, to an archive, mirror, owners, error
//! policy, sparse files; `cat`, `mount` (a read-only WebDAV drive) and the
//! local volume cache.

use super::backup::{emit, with_repo_cache, RepoArgs};
use crate::OutputFormat;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use ufop_core::backup::access::{find_entry, open_file, BlockSource};
use ufop_core::backup::archive::{ArchiveFormat, ArchiveTarget};
use ufop_core::backup::cache::{VolumeCache, DEFAULT_MAX_BYTES};
use ufop_core::backup::davserve::{self, DavContent, DavServer, DavVersion};
use ufop_core::backup::engine::Repo;
use ufop_core::backup::errors::{ErrorAction, RestoreErrorPolicy, UnknownEntryAction};
use ufop_core::backup::lock::LockKind;
use ufop_core::backup::select::VersionSelector;
use ufop_core::backup::{OpContext, RestoreOptions};
use ufop_core::error::AppError;

/// Which version: `--version latest|oldest|N|<id>` and `--time`.
#[derive(Args, Debug, Clone, Default)]
pub struct VersionArgs {
    /// Version: a number (0 = newest), latest, oldest, or a version's timestamp
    #[arg(long, default_value = "0")]
    pub version: String,
    /// As it was at this time: the newest complete version made at or before
    /// it ("2026-10-01 09:00" or 2026-10-01 in local time, or RFC 3339)
    #[arg(long)]
    pub time: Option<String>,
    /// latest / oldest / --time may pick a partial (interrupted) version
    #[arg(long)]
    pub allow_partial: bool,
}

impl VersionArgs {
    pub fn selector(&self) -> Result<VersionSelector, AppError> {
        VersionSelector::from_parts(Some(&self.version), self.time.as_deref(), self.allow_partial)
    }
}

/// The local volume cache (`--cache-dir`, `--cache-size`).
#[derive(Args, Debug, Clone, Default)]
pub struct CacheArgs {
    /// Folder of the local volume cache (default: the user cache folder)
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    /// Size limit of the cache, e.g. 2G (0 = no cache)
    #[arg(long)]
    pub cache_size: Option<String>,
}

impl CacheArgs {
    pub fn dir(&self) -> PathBuf {
        self.cache_dir.clone().unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("ufop").join("backup-volumes"))
    }

    pub fn open(&self) -> Result<Option<Arc<VolumeCache>>, AppError> {
        let max = match self.cache_size.as_deref() {
            Some(s) => super::backup::parse_bytes(s)?,
            None => DEFAULT_MAX_BYTES,
        };
        if max == 0 {
            return Ok(None);
        }
        Ok(Some(Arc::new(VolumeCache::new(self.dir(), max)?)))
    }
}

/// Restore options beyond the classic flags.
#[derive(Args, Debug, Clone, Default)]
pub struct RestoreExtraArgs {
    /// As it was at this time ("2026-10-01 09:00" local, 2026-10-01, RFC 3339)
    #[arg(long)]
    pub time: Option<String>,
    /// latest / oldest / --time may pick a partial (interrupted) version
    #[arg(long)]
    pub allow_partial: bool,
    /// Write a ZIP, TAR or TAR.GZ archive instead of files (type from the name: .zip, .tar, .tar.gz, .tgz)
    #[arg(long)]
    pub archive: Option<PathBuf>,
    /// Archive type: zip | zip-stored | tar | tgz (default: from --archive's name)
    #[arg(long)]
    pub archive_format: Option<String>,
    /// Mirror: also delete what the restored folders in --target hold but the
    /// version does not (lists them and asks first)
    #[arg(long)]
    pub mirror: bool,
    /// With --mirror: delete without asking
    #[arg(long)]
    pub yes: bool,
    /// Restore file owners: auto (when running as root/administrator) | always | never
    #[arg(long, default_value = "auto")]
    pub restore_owner: String,
    /// Files that cannot be written: fail (error) | continue (warning)
    #[arg(long, default_value = "fail")]
    pub on_file_error: String,
    /// Folders that cannot be created: fail | continue
    #[arg(long, default_value = "fail")]
    pub on_dir_error: String,
    /// Data missing or damaged at the destination: fail | continue
    #[arg(long, default_value = "fail")]
    pub on_missing_data: String,
    /// Permissions, owner, times or attributes that cannot be applied: ignore | warn | fail
    #[arg(long, default_value = "warn")]
    pub on_metadata_error: String,
    /// Stop at the first error
    #[arg(long)]
    pub fail_fast: bool,
    /// Stop after more than N problems
    #[arg(long)]
    pub max_errors: Option<u32>,
    /// Leave holes for runs of zeros: auto | off
    #[arg(long, default_value = "auto")]
    pub sparse: String,
    /// Read volumes through the local volume cache (and keep them there)
    #[arg(long)]
    pub use_cache: bool,
    #[command(flatten)]
    pub cache: CacheArgs,
}

fn parse_enum<T: serde::de::DeserializeOwned>(flag: &str, v: &str) -> Result<T, AppError> {
    serde_json::from_value(serde_json::Value::String(v.to_ascii_lowercase().replace('-', "_")))
        .map_err(|_| AppError::validation(format!("Invalid value '{v}' for --{flag}")))
}

impl RestoreExtraArgs {
    /// Apply these flags to `o` (the version comes from `version` and `--time`).
    pub fn apply(&self, version: &str, o: &mut RestoreOptions) -> Result<(), AppError> {
        let sel = VersionSelector::from_parts(Some(version), self.time.as_deref(), self.allow_partial)?;
        if o.versions.is_empty() {
            o.selector = Some(sel);
        }
        if let Some(path) = &self.archive {
            let format = match self.archive_format.as_deref() {
                None => None,
                Some("tgz" | "tar.gz" | "tar-gz") => Some(ArchiveFormat::TarGz),
                Some(f) => Some(parse_enum::<ArchiveFormat>("archive-format", f)?),
            };
            o.archive = Some(ArchiveTarget { path: path.clone(), format });
            o.archive.as_ref().map(|a| a.format()).transpose()?;
        }
        o.delete_extraneous = self.mirror;
        o.restore_owner = parse_enum("restore-owner", &self.restore_owner)?;
        o.error_policy = RestoreErrorPolicy {
            on_write_error: parse_enum::<ErrorAction>("on-file-error", &self.on_file_error)?,
            on_dir_error: parse_enum::<ErrorAction>("on-dir-error", &self.on_dir_error)?,
            on_missing_data: parse_enum::<ErrorAction>("on-missing-data", &self.on_missing_data)?,
            on_metadata_error: parse_enum::<UnknownEntryAction>("on-metadata-error", &self.on_metadata_error)?,
            fail_fast: self.fail_fast,
            max_errors: self.max_errors,
        };
        o.sparse = parse_enum("sparse", &self.sparse)?;
        o.use_cache = self.use_cache;
        Ok(())
    }

    /// The cache for a restore that reads through it.
    pub fn cache(&self) -> Result<Option<Arc<VolumeCache>>, AppError> {
        if self.use_cache {
            self.cache.open()
        } else {
            Ok(None)
        }
    }
}

/// Ask before a mirror restore deletes `planned` (unless `--yes`).
pub fn confirm_mirror(planned: &[String], yes: bool) -> Result<bool, AppError> {
    if planned.is_empty() || yes {
        return Ok(true);
    }
    eprintln!("The mirror restore will delete {} item(s) the version does not have:", planned.len());
    for p in planned.iter().take(20) {
        eprintln!("  {p}");
    }
    if planned.len() > 20 {
        eprintln!("  … and {} more", planned.len() - 20);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(AppError::state("A mirror restore deletes files", "Run it again with --yes to confirm, or use --dry-run to see the list."));
    }
    eprint!("Delete them? [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

#[derive(Subcommand, Debug)]
pub enum CacheCommands {
    /// Entries, size and limit of the local volume cache
    Stats {
        #[command(flatten)]
        cache: CacheArgs,
    },
    /// Delete every cached volume
    Clear {
        #[command(flatten)]
        cache: CacheArgs,
    },
}

pub fn cache_command(cmd: CacheCommands, format: &OutputFormat) -> Result<u8, AppError> {
    let ok = |v: Result<(), Box<dyn std::error::Error>>| v.map_err(|e| AppError::internal(e.to_string()));
    match cmd {
        CacheCommands::Stats { cache } => {
            let c = VolumeCache::new(cache.dir(), cache.open()?.map(|c| c.max_bytes()).unwrap_or(0))?;
            ok(emit(format, "Volume cache", &c.stats()))?;
        }
        CacheCommands::Clear { cache } => {
            let c = VolumeCache::new(cache.dir(), 0)?;
            ok(emit(format, "Volume cache cleared", &serde_json::json!({ "bytes_freed": c.clear()? })))?;
        }
    }
    Ok(0)
}

/// `ufop backup cat`: one file of a version to standard output.
pub fn cat(repo: &RepoArgs, path: &str, version: &VersionArgs, cache: &CacheArgs, range: Option<(u64, u64)>, out: &mut dyn std::io::Write) -> Result<u8, AppError> {
    let sel = version.selector()?;
    let cache = cache.open()?;
    with_repo_cache(repo, LockKind::Shared, "cat", cache, |r, db| {
        let sets = db.filesets()?;
        let fs = &sets[sel.resolve(&sets)?];
        let entries = db.files(fs.id)?;
        let e = find_entry(&entries, path).ok_or_else(|| AppError::not_found(format!("{path} is not in the version of {}", fs.timestamp)))?;
        let src = BlockSource::new(r)?;
        let mut f = open_file(&src, e)?;
        match range {
            Some((off, len)) => out.write_all(&f.read_range(off, len)?)?,
            None => {
                f.copy_to(out)?;
            }
        }
        out.flush()?;
        Ok(0)
    })
}

/// Start serving a version (or every version) of a repository as a drive.
pub fn start_drive(repo: &RepoArgs, version: &VersionArgs, all: bool, bind: std::net::SocketAddr, cache: &CacheArgs) -> Result<(DavServer, String), AppError> {
    let sel = version.selector()?;
    let cache = cache.open()?;
    let be: Arc<dyn ufop_core::backup::Backend> = Arc::from(repo.backend());
    let ctx = OpContext::new();
    let (content, label) = repo.locked(be.as_ref(), Some(LockKind::Shared), "mount", &ctx, || {
        let db = repo.open_db(be.as_ref(), &ctx)?;
        let (src, versions, label) = {
            let r = Repo::open(be.as_ref(), &db, repo.passphrase().as_deref(), &repo.options(), false, Default::default())?.with_cache(cache.clone());
            repo.apply_source(&db);
            let sets = db.filesets()?;
            let (versions, label) = if all {
                (sets.iter().map(|f| DavVersion { fileset_id: f.id, timestamp: f.timestamp.clone() }).collect::<Vec<_>>(), format!("all {} versions", sets.len()))
            } else {
                let i = sel.resolve(&sets)?;
                (vec![DavVersion { fileset_id: sets[i].id, timestamp: sets[i].timestamp.clone() }], format!("version {i} ({})", sets[i].timestamp))
            };
            (BlockSource::shared(&r, be.clone())?, versions, label)
        };
        let db = Mutex::new(db);
        let loader = Box::new(move |id: i64| db.lock().unwrap_or_else(|e| e.into_inner()).files(id));
        Ok((DavContent::new(src, versions, !all, loader)?, label))
    })?;
    Ok((davserve::serve(Arc::new(content), bind, davserve::new_token())?, label))
}

/// `ufop backup mount`: serve until Ctrl-C (or `seconds`).
pub fn mount(repo: &RepoArgs, version: &VersionArgs, all: bool, bind: &str, port: u16, cache: &CacheArgs, seconds: Option<u64>, format: &OutputFormat) -> Result<u8, AppError> {
    let ip: std::net::IpAddr = bind.parse().map_err(|_| AppError::validation(format!("Invalid --bind address '{bind}'")))?;
    if !ip.is_loopback() {
        tracing::warn!("Serving the backup drive on {ip}: anyone who can reach it and has the token can read the files");
    }
    let (server, label) = start_drive(repo, version, all, std::net::SocketAddr::new(ip, port), cache)?;
    let info = serde_json::json!({ "url": server.url(), "username": "ufop", "password": server.token(), "serving": label });
    if matches!(format, OutputFormat::Human) {
        eprintln!("Serving {label} read-only at {}", server.url());
        eprintln!("WebDAV clients that ask for a login: any user name, password {}", server.token());
        eprintln!("Mount it with e.g. `gio mount {}` (Linux), Finder → Connect to Server (macOS) or `net use * {}` (Windows). Press Ctrl-C to stop.", server.url().replacen("http://", "dav://", 1), server.url());
    } else {
        crate::output::print_data(format, &info);
    }
    match seconds {
        Some(s) => std::thread::sleep(std::time::Duration::from_secs(s)),
        None => {
            let h = tokio::runtime::Handle::current();
            h.block_on(async {
                let _ = tokio::signal::ctrl_c().await;
            });
        }
    }
    server.stop();
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_flags_map_to_options() {
        let a = RestoreExtraArgs {
            time: Some("2026-10-01 09:00".into()),
            archive: Some("out.tgz".into()),
            restore_owner: "never".into(),
            on_file_error: "continue".into(),
            on_dir_error: "fail".into(),
            on_missing_data: "continue".into(),
            on_metadata_error: "ignore".into(),
            fail_fast: true,
            max_errors: Some(4),
            sparse: "off".into(),
            ..Default::default()
        };
        let mut o = RestoreOptions::default();
        a.apply("0", &mut o).unwrap();
        assert!(matches!(o.selector, Some(VersionSelector::AsOf { .. })));
        assert_eq!(o.archive.as_ref().unwrap().format().unwrap(), ArchiveFormat::TarGz);
        assert_eq!(o.restore_owner, ufop_core::backup::owner::OwnerPolicy::Never);
        assert_eq!((o.error_policy.on_write_error, o.error_policy.on_missing_data, o.error_policy.on_metadata_error), (ErrorAction::Continue, ErrorAction::Continue, UnknownEntryAction::Ignore));
        assert!(o.error_policy.fail_fast && o.error_policy.max_errors == Some(4));
        assert_eq!(o.sparse, ufop_core::backup::sparse::SparseMode::Off);
        let mut o = RestoreOptions::default();
        RestoreExtraArgs { archive: Some("x.bin".into()), archive_format: Some("zip-stored".into()), ..defaults() }.apply("latest", &mut o).unwrap();
        assert_eq!(o.archive.unwrap().format, Some(ArchiveFormat::ZipStored));
        assert_eq!(o.selector, Some(VersionSelector::Latest { allow_partial: false }));
        assert!(RestoreExtraArgs { archive: Some("x.rar".into()), ..defaults() }.apply("0", &mut RestoreOptions::default()).is_err());
        assert!(RestoreExtraArgs { restore_owner: "sometimes".into(), ..defaults() }.apply("0", &mut RestoreOptions::default()).is_err());
        assert!(confirm_mirror(&[], false).unwrap());
        assert!(confirm_mirror(&["/x".into()], true).unwrap());
    }

    fn defaults() -> RestoreExtraArgs {
        RestoreExtraArgs { restore_owner: "auto".into(), on_file_error: "fail".into(), on_dir_error: "fail".into(), on_missing_data: "fail".into(), on_metadata_error: "warn".into(), sparse: "auto".into(), ..Default::default() }
    }
}
