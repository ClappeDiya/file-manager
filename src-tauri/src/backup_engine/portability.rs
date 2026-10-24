//! Job configuration portability: JSON export/import (optionally with the
//! passphrase, optionally encrypted with a separate export passphrase),
//! the secret-stripped form embedded in remote file lists, and a
//! command-line representation for scripting.

use super::{BackupDestination, BackupJob, JobStats};
use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use ufop_core::backup::crypto::{KdfParams, VolumeCipher};
use ufop_core::backup::{RetentionPolicy, SourceFilter};

const FORMAT: &str = "ufop-backup-config";

#[derive(Debug, Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
    /// Plain export: the payload object. Encrypted: base64 of the sealed payload.
    payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    job: BackupJob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passphrase: Option<String>,
    /// The policy templates the job is based on (created on import when missing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    templates: Vec<super::templates::PolicyTemplate>,
}

/// Job JSON safe to store remotely: no stats, no report callback URLs
/// (which often embed tokens). Passphrases never live in the job itself.
pub fn stripped_config(job: &BackupJob) -> Result<serde_json::Value, AppError> {
    let mut j = job.clone();
    j.stats = JobStats::default();
    j.reports.http_urls.clear();
    j.reports.headers.clear();
    // Webhook URLs and header values often embed tokens too.
    j.notifications.retain(|n| !matches!(n.channel, super::notify::Channel::Webhook { .. }));
    Ok(serde_json::to_value(j)?)
}

/// Job JSON including report URLs and scripts, for encrypted backups whose
/// file lists are encrypted too (`remote_config_include_secrets`).
pub fn full_config(job: &BackupJob) -> Result<serde_json::Value, AppError> {
    let mut j = job.clone();
    j.stats = JobStats::default();
    Ok(serde_json::to_value(j)?)
}

/// Export a job. `passphrase` is included only when given;
/// `export_passphrase` encrypts the whole file.
pub fn export_job(job: &BackupJob, passphrase: Option<String>, export_passphrase: Option<&str>, include_stats: bool) -> Result<String, AppError> {
    export_job_with(job, passphrase, export_passphrase, include_stats, Vec::new())
}

/// [`export_job`] carrying the job's policy templates.
pub fn export_job_with(
    job: &BackupJob,
    passphrase: Option<String>,
    export_passphrase: Option<&str>,
    include_stats: bool,
    templates: Vec<super::templates::PolicyTemplate>,
) -> Result<String, AppError> {
    let mut j = job.clone();
    if !include_stats {
        j.stats = JobStats::default();
    }
    let templates = templates.into_iter().map(|t| super::templates::PolicyTemplate { managed: false, enforced: false, ..t }).collect();
    let payload = Payload { job: j, passphrase, templates };
    let file = match export_passphrase.filter(|p| !p.is_empty()) {
        None => ExportFile { format: FORMAT.into(), version: 1, encrypted: false, salt: None, payload: serde_json::to_value(&payload)? },
        Some(p) => {
            let salt = VolumeCipher::random_salt();
            let c = VolumeCipher::derive(p, &salt, KdfParams::default())?;
            let sealed = c.seal(&serde_json::to_vec(&payload)?)?;
            ExportFile {
                format: FORMAT.into(),
                version: 1,
                encrypted: true,
                salt: Some(hex::encode(salt)),
                payload: base64::engine::general_purpose::STANDARD.encode(sealed).into(),
            }
        }
    };
    Ok(serde_json::to_string_pretty(&file)?)
}

/// Whether an export file needs a passphrase to import.
pub fn is_encrypted_export(json: &str) -> bool {
    serde_json::from_str::<ExportFile>(json).map(|f| f.encrypted).unwrap_or(false)
}

/// Parse an export (or a bare job JSON, e.g. from remote discovery). The
/// returned job has a fresh id; statistics are kept only on request.
pub fn import_job(json: &str, export_passphrase: Option<&str>, import_stats: bool) -> Result<(BackupJob, Option<String>), AppError> {
    import_job_with(json, export_passphrase, import_stats).map(|(j, p, _)| (j, p))
}

/// [`import_job`] also returning the policy templates the export carried.
pub fn import_job_with(json: &str, export_passphrase: Option<&str>, import_stats: bool) -> Result<(BackupJob, Option<String>, Vec<super::templates::PolicyTemplate>), AppError> {
    let payload: Payload = match serde_json::from_str::<ExportFile>(json) {
        Ok(f) if f.format == FORMAT => {
            if f.encrypted {
                let p = export_passphrase.filter(|p| !p.is_empty()).ok_or_else(|| AppError::validation("This export is encrypted; enter its passphrase"))?;
                let salt = hex::decode(f.salt.unwrap_or_default()).map_err(|_| AppError::validation("Export salt is malformed"))?;
                let sealed = base64::engine::general_purpose::STANDARD
                    .decode(f.payload.as_str().unwrap_or_default())
                    .map_err(|_| AppError::validation("Export payload is malformed"))?;
                let c = VolumeCipher::derive(p, &salt, KdfParams::default())?;
                serde_json::from_slice(&c.open(&sealed).map_err(|_| AppError::encryption("Wrong export passphrase", "Check the passphrase used when exporting."))?)?
            } else {
                serde_json::from_value(f.payload)?
            }
        }
        _ => Payload { job: serde_json::from_str(json).map_err(|e| AppError::validation(format!("Not a backup configuration: {e}")))?, passphrase: None, templates: Vec::new() },
    };
    let mut job = payload.job;
    job.id = String::new();
    if !import_stats {
        job.stats = JobStats::default();
    }
    Ok((job, payload.passphrase, payload.templates))
}

fn quote(s: &str) -> String {
    shlex::try_quote(s).map(|c| c.into_owned()).unwrap_or_else(|_| format!("'{s}'"))
}

/// `ufop backup run …` equivalent of a job (local destinations only carry
/// a path; connection destinations name the saved connection id).
pub fn cli_command(job: &BackupJob) -> String {
    let mut parts = vec!["ufop".to_string(), "backup".into(), "run".into()];
    for s in &job.sources {
        parts.push("--source".into());
        parts.push(quote(s));
    }
    match &job.destination {
        BackupDestination::Local { path, .. } => {
            parts.push("--dest".into());
            parts.push(quote(path));
        }
        BackupDestination::Connection { connection_id, remote_path } => {
            parts.push("--dest".into());
            parts.push(quote(&format!("connection://{connection_id}{remote_path}")));
        }
        BackupDestination::Library { destination_id, remote_path } => {
            parts.push("--dest".into());
            parts.push(quote(&format!("library://{destination_id}/{remote_path}")));
        }
        BackupDestination::Cloud { config, .. } => {
            parts.push("--dest".into());
            parts.push(quote(&config.describe()));
        }
    }
    parts.push("--dbpath".into());
    parts.push(quote(&super::default_data_dir().join(format!("{}.sqlite", job.id)).to_string_lossy()));
    if job.encrypted {
        parts.push("--passphrase-env".into());
        parts.push("UFOP_PASSPHRASE".into());
    } else {
        parts.push("--no-encryption".into());
    }
    let o = &job.options;
    parts.push(format!("--prefix={}", o.prefix));
    parts.push(format!("--block-size={}", o.block_size));
    parts.push(format!("--volume-size={}", o.volume_size));
    parts.push(format!("--compression-level={}", o.compression_level));
    // Repository format choices (only those differing from the defaults).
    if let Some(v) = o.format_version {
        parts.push(format!("--format-version={v}"));
    }
    if let Some(c) = o.chunker {
        parts.push(format!("--chunker={}", if c == ufop_core::backup::ChunkerKind::Fixed { "fixed" } else { "fastcdc" }));
    }
    if let Some(c) = o.block_codec {
        parts.push(format!("--block-codec={}", if c == ufop_core::backup::BlockCodec::Zstd { "zstd" } else { "deflate" }));
    }
    if o.cipher != ufop_core::backup::crypto::CipherKind::Aes256Gcm {
        parts.push("--cipher=chacha20-poly1305".into());
    }
    if let ufop_core::backup::layout::RepoLayout::Sharded { prefix_len } = o.layout {
        parts.push(format!("--layout=sharded --shard-prefix-len={prefix_len}"));
    }
    match &job.retention {
        RetentionPolicy::KeepAll => {}
        RetentionPolicy::KeepDays { days } => parts.push(format!("--keep-days={days}")),
        RetentionPolicy::KeepCount { count } => parts.push(format!("--keep-versions={count}")),
        RetentionPolicy::Smart => parts.push("--retention-policy=smart".into()),
        RetentionPolicy::Custom { rules } => parts.push(format!("--retention-policy={}", quote(rules))),
        RetentionPolicy::Counts { keep_latest, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly } => {
            for (flag, n) in [("latest", keep_latest), ("hourly", keep_hourly), ("daily", keep_daily), ("weekly", keep_weekly), ("monthly", keep_monthly), ("yearly", keep_yearly)] {
                if *n > 0 {
                    parts.push(format!("--keep-{flag}={n}"));
                }
            }
        }
    }
    // Sources and runs (only those differing from the defaults).
    let f = &job.filter;
    if f.ignore_files != SourceFilter::default().ignore_files {
        if f.ignore_files.is_empty() {
            parts.push("--ignore-file=''".into());
        }
        for n in &f.ignore_files {
            parts.push(format!("--ignore-file={}", quote(n)));
        }
    }
    if f.respect_gitignore {
        parts.push("--respect-gitignore".into());
    }
    if o.one_file_system {
        parts.push("--one-file-system".into());
    }
    let ep = &o.error_policy;
    use ufop_core::backup::errors::{ErrorAction, UnknownEntryAction};
    if ep.on_file_error == ErrorAction::Fail {
        parts.push("--on-file-error=fail".into());
    }
    if ep.on_dir_error == ErrorAction::Fail {
        parts.push("--on-dir-error=fail".into());
    }
    match ep.on_unknown_entry {
        UnknownEntryAction::Ignore => {}
        UnknownEntryAction::Warn => parts.push("--on-unknown-entry=warn".into()),
        UnknownEntryAction::Fail => parts.push("--on-unknown-entry=fail".into()),
    }
    if ep.fail_fast {
        parts.push("--fail-fast".into());
    }
    if let Some(m) = ep.max_errors {
        parts.push(format!("--max-errors={m}"));
    }
    if o.rehash_percent > 0 {
        parts.push(format!("--rehash-percent={}", o.rehash_percent));
    }
    if let Some(cap) = o.max_upload_bytes_per_run {
        parts.push(format!("--max-upload={cap}"));
    }
    if o.parallel_file_threshold != ufop_core::backup::DEFAULT_PARALLEL_FILE_THRESHOLD {
        parts.push(format!("--parallel-file-threshold={}", o.parallel_file_threshold));
    }
    if o.snapshot.provider != Default::default() {
        parts.push(format!("--snapshot-provider={}", o.snapshot.provider.name()));
    }
    // Versions are recorded as this job's source (shared repositories).
    parts.push(format!("--job-id={}", quote(&job.id)));
    if !o.identity.hostname.is_empty() {
        parts.push(format!("--hostname={}", quote(&o.identity.hostname)));
    }
    if !o.identity.username.is_empty() {
        parts.push(format!("--username={}", quote(&o.identity.username)));
    }
    if o.shared_repository {
        parts.push("--shared".into());
    }
    if let Some(n) = o.default_note.as_deref().filter(|n| !n.is_empty()) {
        parts.push(format!("--note={}", quote(n)));
    }
    for (k, v) in &o.default_tags {
        parts.push(format!("--tag={}", quote(&if v.is_empty() { k.clone() } else { format!("{k}={v}") })));
    }
    // The CLI applies every --include, then every --exclude, then the
    // --filter (rclone) lines, so the job's order survives as long as its
    // rclone rules come after the others (as an imported rules file does).
    for r in &job.filter.rules {
        let (flag, value) = ufop_core::filter_rules::cli_filter_arg(r);
        parts.push(format!("{flag}={}", quote(&value)));
    }
    if let Some(m) = job.filter.max_file_size {
        parts.push(format!("--skip-files-larger-than={m}"));
    }
    if job.filter.exclude_hidden {
        parts.push("--exclude-hidden".into());
    }
    if job.sources.iter().any(|s| ufop_core::backup::photos::is_photos_source(s)) {
        let p = &o.photos;
        if p.include_hidden {
            parts.push("--photos-include-hidden".into());
        }
        if p.include_trashed {
            parts.push("--photos-include-trashed".into());
        }
        for a in &p.albums {
            parts.push(format!("--photos-album={}", quote(a)));
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> BackupJob {
        BackupJob {
            id: "abc".into(),
            name: "Docs".into(),
            sources: vec!["/home/me/Docs".into()],
            destination: BackupDestination::Local { path: "/mnt/backup".into(), alternate_paths: vec![], marker_file: None },
            stats: JobStats { version_count: 4, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn cli_command_carries_format_options() {
        let mut j = job();
        assert!(!cli_command(&j).contains("--cipher"));
        j.options.cipher = ufop_core::backup::crypto::CipherKind::ChaCha20Poly1305;
        j.options.chunker = Some(ufop_core::backup::ChunkerKind::Fixed);
        j.options.layout = ufop_core::backup::layout::RepoLayout::Sharded { prefix_len: 2 };
        j.options.format_version = Some(2);
        j.retention = RetentionPolicy::Counts { keep_latest: 2, keep_hourly: 0, keep_daily: 7, keep_weekly: 4, keep_monthly: 0, keep_yearly: 0 };
        j.options.shared_repository = true;
        j.options.default_tags.insert("env".into(), "prod".into());
        let c = cli_command(&j);
        for f in ["--cipher=chacha20-poly1305", "--chunker=fixed", "--layout=sharded --shard-prefix-len=2", "--format-version=2", "--keep-latest=2", "--keep-daily=7", "--keep-weekly=4", "--shared", "--job-id=abc", "--tag='env=prod'"] {
            assert!(c.contains(f), "{f} in {c}");
        }
    }

    #[test]
    fn cli_command_carries_source_and_run_options() {
        let mut j = job();
        let plain = cli_command(&j);
        assert!(!plain.contains("--ignore-file") && !plain.contains("--fail-fast") && !plain.contains("--parallel-file-threshold"), "{plain}");
        j.filter.ignore_files = vec![".backupignore".into()];
        j.filter.respect_gitignore = true;
        j.options.one_file_system = true;
        j.options.error_policy = ufop_core::backup::errors::ErrorPolicy {
            on_file_error: ufop_core::backup::errors::ErrorAction::Fail,
            on_unknown_entry: ufop_core::backup::errors::UnknownEntryAction::Warn,
            fail_fast: true,
            max_errors: Some(5),
            ..Default::default()
        };
        j.options.rehash_percent = 10;
        j.options.max_upload_bytes_per_run = Some(1 << 30);
        j.options.parallel_file_threshold = 0;
        j.options.snapshot.provider = ufop_core::backup::snapshot::SnapshotProvider::Btrfs;
        let c = cli_command(&j);
        for f in ["--ignore-file=.backupignore", "--respect-gitignore", "--one-file-system", "--on-file-error=fail", "--on-unknown-entry=warn", "--fail-fast", "--max-errors=5", "--rehash-percent=10", "--max-upload=1073741824", "--parallel-file-threshold=0", "--snapshot-provider=btrfs"] {
            assert!(c.contains(f), "{f} in {c}");
        }
    }

    /// Regression: rclone rules were exported as `[regex]` filters, which
    /// the CLI matches on absolute paths, while rclone rules (and the app)
    /// match inside the source folder, so an anchored `/Photos/**` selected
    /// nothing in the generated command.
    #[test]
    fn cli_command_selects_the_same_files_for_rclone_rules() {
        use ufop_core::backup::{FilterAction, FilterKind, FilterRule, SourceFilter};
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("Docs");
        let mut j = job();
        j.sources = vec![src.to_string_lossy().to_string()];
        let rclone = |action, p: &str| FilterRule { action, kind: FilterKind::Rclone, pattern: p.into() };
        j.filter.rules = vec![
            FilterRule { action: FilterAction::Exclude, kind: FilterKind::Regex, pattern: "\\.bak$".into() },
            rclone(FilterAction::Include, "/Photos/**"),
            rclone(FilterAction::Include, "*.md"),
            rclone(FilterAction::Exclude, "*"),
        ];
        let cmd = cli_command(&j);
        // What `ufop backup run` reads back from that command line.
        let args = shlex::split(&cmd).unwrap();
        let values = |flag: &str| -> Vec<String> { args.iter().filter_map(|a| a.strip_prefix(&format!("{flag}="))).map(String::from).collect() };
        let mut cli = SourceFilter::default();
        cli.rules = ufop_core::filter_rules::cli_rules(&values("--include"), &values("--exclude"), &values("--filter")).unwrap();
        let cli = cli.compile().unwrap();
        let app = j.filter.clone().compile().unwrap();
        for (rel, is_dir) in [("Photos", true), ("Photos/a.jpg", false), ("Photos/2024/b.jpg", false), ("Photos/old.bak", false), ("notes.md", false), ("sub/readme.md", false), ("other.txt", false), ("sub", true), ("sub/Photos/c.jpg", false)] {
            let path = src.join(rel);
            assert_eq!(cli.evaluate_under(&path, Some(&src), is_dir), app.evaluate_under(&path, Some(&src), is_dir), "{rel}: CLI and app disagree\n{cmd}");
        }
        assert_eq!(app.evaluate_under(&src.join("Photos/a.jpg"), Some(&src), false), Some(FilterAction::Include));
    }

    #[test]
    fn cli_command_carries_photos_options() {
        let mut j = job();
        assert!(!cli_command(&j).contains("--photos"));
        j.sources = vec!["photos:///Users/me/Pictures/Photos Library.photoslibrary".into()];
        j.options.photos = ufop_core::backup::photos::PhotosOptions { include_hidden: true, include_trashed: false, albums: vec!["Trip 2024".into()] };
        let c = cli_command(&j);
        assert!(c.contains("--source 'photos:///Users/me/Pictures/Photos Library.photoslibrary'"), "{c}");
        assert!(c.contains("--photos-include-hidden") && !c.contains("--photos-include-trashed"));
        assert!(c.contains("--photos-album='Trip 2024'"), "{c}");
    }

    #[test]
    fn plain_and_encrypted_roundtrip() {
        let j = job();
        let plain = export_job(&j, None, None, false).unwrap();
        assert!(!plain.contains("\"version_count\": 4"));
        let (imp, pass) = import_job(&plain, None, false).unwrap();
        assert_eq!(imp.name, "Docs");
        assert!(imp.id.is_empty() && pass.is_none());

        let enc = export_job(&j, Some("secret".into()), Some("exportpw"), true).unwrap();
        assert!(is_encrypted_export(&enc));
        assert!(!enc.contains("secret") && !enc.contains("Docs"));
        assert!(import_job(&enc, None, false).is_err());
        assert!(import_job(&enc, Some("wrong"), false).is_err());
        let (imp, pass) = import_job(&enc, Some("exportpw"), true).unwrap();
        assert_eq!(pass.as_deref(), Some("secret"));
        assert_eq!(imp.stats.version_count, 4);
    }

    #[test]
    fn bare_job_and_stripping() {
        let mut j = job();
        j.reports.http_urls = vec!["https://x/?token=1".into()];
        let v = stripped_config(&j).unwrap();
        assert!(v["reports"]["http_urls"].as_array().unwrap().is_empty());
        let (imp, _) = import_job(&v.to_string(), None, false).unwrap();
        assert_eq!(imp.sources, j.sources);
        assert!(import_job("{\"nope\": 1", None, false).is_err());
    }

    #[test]
    fn cli_export() {
        let mut j = job();
        j.retention = RetentionPolicy::KeepCount { count: 3 };
        let c = cli_command(&j);
        assert!(c.starts_with("ufop backup run --source /home/me/Docs --dest /mnt/backup"));
        assert!(c.contains("--keep-versions=3") && c.contains("--passphrase-env UFOP_PASSPHRASE"));
    }
}
