//! Ransomware hardening checklist for a backup job: what in its settings
//! protects the backups if the machine (and its credentials) are taken
//! over, and — only when asked ("Run checks") — a probe of whether the
//! backup credentials can delete objects at the destination.
//!
//! The guide behind each check is `docs/backup-ransomware-hardening.md`.

use super::{BackupDestination, BackupJob, BackupManager, BackendOpts, ReplicaSchedule};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use ufop_core::backup::backend::LockMode;
use ufop_core::backup::RetentionPolicy;

pub const GUIDE: &str = "docs/backup-ransomware-hardening.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    /// Not known until "Run checks" (the destination probe).
    Unknown,
    /// Shown for information; not counted.
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardeningCheck {
    pub id: String,
    pub title: String,
    pub status: CheckStatus,
    pub detail: String,
    pub advice: String,
    /// Link to the guide section.
    pub doc: String,
}

/// The destination probe's last result (kept with the job's statistics).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeResult {
    pub ran_at: String,
    /// The probe object could be written.
    pub can_write: bool,
    /// The backup credentials deleted the probe object (`None` = not tried).
    pub can_delete: Option<bool>,
    /// After deleting, the probe object could no longer be read.
    pub gone_after_delete: Option<bool>,
    /// A probe object that is still there (it could not be removed).
    pub leftover: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardeningReport {
    pub job_id: String,
    pub passed: usize,
    pub total: usize,
    pub checks: Vec<HardeningCheck>,
    pub probe: Option<ProbeResult>,
}

fn check(id: &str, title: &str, status: CheckStatus, detail: impl Into<String>, advice: impl Into<String>, anchor: &str) -> HardeningCheck {
    HardeningCheck { id: id.into(), title: title.into(), status, detail: detail.into(), advice: advice.into(), doc: format!("{GUIDE}#{anchor}") }
}

/// Where a destination lives, to tell a second location from the same one.
fn location(d: &BackupDestination) -> String {
    match d {
        BackupDestination::Local { path, .. } => format!("local:{path}"),
        BackupDestination::Connection { connection_id, .. } => format!("connection:{connection_id}"),
        BackupDestination::Cloud { config, .. } => format!("cloud:{}", config.provider()),
        BackupDestination::Library { destination_id, .. } => format!("library:{destination_id}"),
    }
}

fn provider(d: &BackupDestination) -> &'static str {
    match d {
        BackupDestination::Cloud { config, .. } => config.provider(),
        BackupDestination::Local { .. } => "local",
        BackupDestination::Connection { .. } => "connection",
        BackupDestination::Library { .. } => "library",
    }
}

/// Providers whose stores can lock volumes (`Backend::supports_lock`).
pub const LOCK_PROVIDERS: &[&str] = &["s3", "azure", "gcs", "b2"];

/// Versions a policy keeps at least (roughly), for the retention check.
fn min_kept(p: &RetentionPolicy) -> Option<u32> {
    match p {
        RetentionPolicy::KeepAll => None,
        RetentionPolicy::KeepDays { days } => Some(*days),
        RetentionPolicy::KeepCount { count } => Some(*count),
        RetentionPolicy::Smart | RetentionPolicy::Custom { .. } => Some(7),
        RetentionPolicy::Counts { keep_latest, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly } => Some(keep_latest + keep_hourly + keep_daily + keep_weekly + keep_monthly + keep_yearly),
    }
}

/// The checklist for a job (with its resolved settings), its pinned
/// version count and the last probe.
pub fn evaluate(job: &BackupJob, pinned: usize, probe: Option<&ProbeResult>) -> Vec<HardeningCheck> {
    use CheckStatus::*;
    let mut out = Vec::new();
    let prov = provider(&job.destination);

    out.push(if job.encrypted {
        check("encryption", "Backups are encrypted", Pass, "Volumes are encrypted with the job's passphrase.", "", "encrypt-everything")
    } else {
        check("encryption", "Backups are encrypted", Fail, "Volumes are not encrypted: anyone who reaches the destination can read them, and tampering goes unnoticed.", "Create an encrypted job (encryption cannot be added to an existing repository) and keep the passphrase offline.", "encrypt-everything")
    });

    out.push(match &job.options.object_lock {
        Some(l) if l.days > 0 && LOCK_PROVIDERS.contains(&prov) => {
            let mode = if l.mode == LockMode::Compliance { "compliance" } else { "governance" };
            check("object_lock", "New volumes are locked (immutable)", Pass, format!("Every new volume is locked for {} days ({mode} mode).", l.days), if l.mode == LockMode::Governance { "Governance locks can be lifted by accounts with bypass permission; use compliance mode for the strongest protection." } else { "" }, "object-lock-and-immutability")
        }
        Some(_) => check("object_lock", "New volumes are locked (immutable)", Fail, format!("Object lock is set, but this destination ({prov}) cannot lock objects."), "Use S3 (Object Lock), Azure (immutability), GCS (retention) or B2 (Object Lock), or an append-only SFTP target.", "object-lock-and-immutability"),
        None if LOCK_PROVIDERS.contains(&prov) => check("object_lock", "New volumes are locked (immutable)", Warn, "Volumes are not locked: credentials that can delete can destroy the backups.", "Turn on object lock (Options → Repository safety) on a bucket created with Object Lock / versioning.", "object-lock-and-immutability"),
        None => check("object_lock", "New volumes are locked (immutable)", Warn, format!("This destination ({prov}) has no object lock."), "Use an append-only target (rrsync / chroot with no delete), or replicate to a bucket with Object Lock.", "append-only-sftp"),
    });

    out.push(match &job.soft_delete_prefix {
        Some(p) if !p.is_empty() => check("soft_delete", "Deleted volumes are kept for a while", Pass, format!("Deletes rename volumes with the prefix '{p}' instead of removing them."), "Clean up the soft-deleted volumes on a schedule with the maintenance identity.", "versioning-soft-delete-and-lifecycle"),
        _ => check("soft_delete", "Deleted volumes are kept for a while", Warn, "Deleting a volume removes it at once (unless the provider keeps versions or soft-deleted blobs, which the app cannot see).", "Turn on bucket versioning / blob soft delete with a lifecycle rule, or the job's soft-delete prefix.", "versioning-soft-delete-and-lifecycle"),
    });

    let here = location(&job.destination);
    let replicas: Vec<_> = job.replicas.iter().filter(|r| r.enabled).collect();
    out.push(if let Some(r) = replicas.iter().find(|r| location(&r.destination) != here && provider(&r.destination) != prov) {
        let when = if r.schedule == ReplicaSchedule::AfterBackup { "after every backup" } else { "on its own schedule" };
        check("replica", "A copy is kept with another provider", Pass, format!("Replica '{}' ({}) is updated {when}.", r.name, provider(&r.destination)), if r.delete_extraneous { "The replica mirrors deletions: give it object lock or versioning too, or turn off deleting extraneous files." } else { "" }, "offline-and-second-provider-copies")
    } else if !replicas.is_empty() {
        check("replica", "A copy is kept with another provider", Warn, "The replicas are with the same provider (or location) as the backups.", "Add a replica with a different provider, account or an offline disk.", "offline-and-second-provider-copies")
    } else {
        check("replica", "A copy is kept with another provider", Warn, "There is only one copy of the backups.", "Add a replica (Destination → Replicas) with another provider or an offline disk.", "offline-and-second-provider-copies")
    });

    out.push(if job.encrypt_local_database == Some(true) {
        check("local_database", "The local database is encrypted", Pass, "The job's local database (file names, sizes, hashes) is encrypted.", "", "protect-this-machine")
    } else {
        check("local_database", "The local database is encrypted", Warn, "The local database is stored in plain form: it lists every file name in the backups.", "Encrypt it (Options → Repository safety → Encrypt the local database).", "protect-this-machine")
    });

    let pin_note = if pinned > 0 { format!(" {pinned} pinned version(s) are kept whatever the policy says.") } else { String::new() };
    out.push(match min_kept(&job.retention) {
        None => check("retention", "Enough versions are kept", Pass, format!("Every version is kept.{pin_note}"), "", "retention-and-pins"),
        Some(n) if n >= 7 => check("retention", "Enough versions are kept", Pass, format!("The policy keeps about {n} or more versions.{pin_note}"), "", "retention-and-pins"),
        Some(n) => check("retention", "Enough versions are kept", Warn, format!("The policy keeps only about {n} version(s): encrypted-by-ransomware files can push the clean versions out before anyone notices.{pin_note}"), "Keep at least a few weeks (for example daily 14, weekly 8, monthly 12), and pin known-good versions.", "retention-and-pins"),
    });

    out.push(match probe {
        None => check("credentials", "The backup credentials cannot delete", Unknown, "Not checked yet.", "Run the checks to try deleting a probe object with the backup credentials (the probe is removed afterwards when possible).", "least-privilege-credentials"),
        Some(p) if !p.can_write => check("credentials", "The backup credentials cannot delete", Fail, format!("The probe could not be written: {}", p.detail), "Fix the destination first (Test destination).", "least-privilege-credentials"),
        Some(p) if p.can_delete == Some(false) => check("credentials", "The backup credentials cannot delete", Pass, format!("Deleting the probe object was refused ({}).", p.ran_at), "Use a separate maintenance identity for compaction and retention.", "the-two-credential-model"),
        Some(p) if job.options.object_lock.is_some() && p.gone_after_delete == Some(false) => check("credentials", "The backup credentials cannot delete", Warn, "The credentials may delete, but the deleted probe could still be read (versioned / locked bucket).", "Prefer put-only backup credentials and a separate maintenance identity.", "the-two-credential-model"),
        Some(p) => check("credentials", "The backup credentials cannot delete", Fail, format!("The backup credentials deleted the probe object ({}): ransomware on this machine could delete the backups too.", p.ran_at), "Give backups put/get/list-only credentials and run compaction and retention with a separate maintenance identity (see the guide).", "the-two-credential-model"),
    });

    out.push(match &job.legal_hold {
        Some(h) => check("legal_hold", "Legal hold", Info, format!("Held for matter '{}' since {}: nothing can be removed.", h.matter, h.placed_at), "", "legal-hold"),
        None => check("legal_hold", "Legal hold", Info, "No legal hold.", "Place one (Maintenance → Legal hold) when backups must be preserved for a matter or an incident.", "legal-hold"),
    });
    out
}

pub fn summarize(job_id: &str, checks: Vec<HardeningCheck>, probe: Option<ProbeResult>) -> HardeningReport {
    let counted: Vec<_> = checks.iter().filter(|c| c.status != CheckStatus::Info).collect();
    let passed = counted.iter().filter(|c| c.status == CheckStatus::Pass).count();
    HardeningReport { job_id: job_id.into(), passed, total: counted.len(), checks, probe }
}

impl BackupManager {
    /// The checklist; `probe` = also try the destination with the backup
    /// credentials (explicit user action only; never automatic).
    pub async fn hardening_report(&self, job_id: &str, probe: bool) -> Result<HardeningReport, AppError> {
        let job = self.get_job(job_id).await?;
        let eff = self.resolve_job(&job).await.map(|r| r.job).unwrap_or_else(|_| job.clone());
        let pinned = self.list_versions(job_id).await.map(|v| v.iter().filter(|f| f.pinned).count()).unwrap_or(0);
        let probe_result = if probe {
            let r = self.probe_destination(&eff).await?;
            let snapshot = {
                let mut jobs = self.inner.jobs.write().await;
                jobs.get_mut(job_id).map(|j| {
                    j.stats.last_hardening_probe = Some(r.clone());
                    j.clone()
                })
            };
            if let Some(j) = snapshot {
                self.persist(&j).await?;
            }
            Some(r)
        } else {
            job.stats.last_hardening_probe.clone()
        };
        Ok(summarize(job_id, evaluate(&eff, pinned, probe_result.as_ref()), probe_result))
    }

    /// Write a small probe object, try to delete it with the job's own
    /// credentials (without soft delete), and clean up when possible.
    async fn probe_destination(&self, job: &BackupJob) -> Result<ProbeResult, AppError> {
        if job.read_only {
            return Err(Self::read_only_refusal(job, "write a probe object"));
        }
        let job = self.resolve_job_secrets(job).await?;
        let backend = Box::pin(self.backend_with_timeout(&job.destination, job.throttle, job.retry, None, BackendOpts::of(&job))).await?;
        let name = format!("ufop-hardening-probe-{}.tmp", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let ran_at = chrono::Utc::now().to_rfc3339();
        super::blocking(move || {
            let body = format!("UFOP ransomware-hardening probe written {ran_at}; safe to delete.\n");
            // As a first backup would; a no-op where the folder exists.
            let _ = backend.create_folder();
            if let Err(e) = backend.put(&name, body.as_bytes()) {
                return Ok(ProbeResult { ran_at, can_write: false, detail: e.user_message(), ..Default::default() });
            }
            match backend.delete(&name) {
                Ok(()) => {
                    let gone = backend.get(&name).is_err();
                    let detail = if gone { "The probe object was deleted with the backup credentials." } else { "Deleting returned success, but the probe can still be read (a versioned or locked bucket)." };
                    Ok(ProbeResult { ran_at, can_write: true, can_delete: Some(true), gone_after_delete: Some(gone), leftover: (!gone).then(|| name.clone()), detail: detail.into() })
                }
                Err(e) => Ok(ProbeResult {
                    ran_at,
                    can_write: true,
                    can_delete: Some(false),
                    gone_after_delete: Some(false),
                    leftover: Some(name.clone()),
                    detail: format!("Deleting was refused ({}). The probe object '{name}' is left; remove it with the maintenance identity.", e.user_message()),
                }),
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ufop_core::backup::ObjectLockOptions;

    fn job() -> BackupJob {
        BackupJob { id: "j".into(), name: "J".into(), encrypted: false, encrypt_local_database: None, retention: RetentionPolicy::KeepAll, ..Default::default() }
    }

    #[test]
    fn a_plain_job_fails_or_warns_and_counts_seven() {
        let r = summarize("j", evaluate(&job(), 0, None), None);
        assert_eq!((r.passed, r.total), (1, 7), "{:#?}", r.checks);
        let by = |id: &str| r.checks.iter().find(|c| c.id == id).unwrap().status;
        assert_eq!(by("encryption"), CheckStatus::Fail);
        assert_eq!(by("credentials"), CheckStatus::Unknown);
        assert_eq!(by("retention"), CheckStatus::Pass, "keep-all is the default");
        assert_eq!(by("legal_hold"), CheckStatus::Info);
        assert!(r.checks.iter().all(|c| c.doc.starts_with(GUIDE) && c.doc.contains('#')));
    }

    #[test]
    fn a_hardened_job_passes() {
        let mut j = job();
        j.encrypted = true;
        j.encrypt_local_database = Some(true);
        j.soft_delete_prefix = Some("deleted-".into());
        j.retention = RetentionPolicy::Counts { keep_latest: 0, keep_hourly: 0, keep_daily: 14, keep_weekly: 8, keep_monthly: 12, keep_yearly: 0 };
        let s3: crate::backup_engine::stores::CloudConfig = serde_json::from_value(serde_json::json!({"provider": "s3", "bucket": "b"})).unwrap();
        j.destination = BackupDestination::Cloud { config: s3, credential_id: String::new() };
        j.options.object_lock = Some(ObjectLockOptions { days: 30, mode: LockMode::Compliance });
        j.replicas = vec![crate::backup_engine::Replica { id: "r".into(), name: "Offline disk".into(), destination: BackupDestination::Local { path: "/mnt/usb".into(), alternate_paths: vec![], marker_file: None }, ..Default::default() }];
        let refused = ProbeResult { ran_at: "now".into(), can_write: true, can_delete: Some(false), ..Default::default() };
        let r = summarize("j", evaluate(&j, 2, Some(&refused)), Some(refused));
        assert_eq!((r.passed, r.total), (7, 7), "{:#?}", r.checks);
        assert!(r.checks.iter().find(|c| c.id == "retention").unwrap().detail.contains("2 pinned"));
        // Credentials that delete fail the check, unless the lock kept the data (then a warning).
        let deletes = ProbeResult { can_delete: Some(true), gone_after_delete: Some(true), can_write: true, ..Default::default() };
        assert_eq!(evaluate(&j, 0, Some(&deletes)).iter().find(|c| c.id == "credentials").unwrap().status, CheckStatus::Fail);
        let kept = ProbeResult { gone_after_delete: Some(false), ..deletes };
        assert_eq!(evaluate(&j, 0, Some(&kept)).iter().find(|c| c.id == "credentials").unwrap().status, CheckStatus::Warn);
    }

    #[test]
    fn locks_on_destinations_that_cannot_lock_and_same_provider_replicas() {
        let mut j = job();
        j.destination = BackupDestination::Local { path: "/backup".into(), alternate_paths: vec![], marker_file: None };
        j.options.object_lock = Some(ObjectLockOptions { days: 7, mode: LockMode::Governance });
        j.replicas = vec![crate::backup_engine::Replica { id: "r".into(), name: "Same".into(), destination: BackupDestination::Local { path: "/backup2".into(), alternate_paths: vec![], marker_file: None }, ..Default::default() }];
        j.retention = RetentionPolicy::KeepCount { count: 2 };
        let c = evaluate(&j, 0, None);
        let by = |id: &str| c.iter().find(|x| x.id == id).unwrap().status;
        assert_eq!(by("object_lock"), CheckStatus::Fail);
        assert_eq!(by("replica"), CheckStatus::Warn);
        assert_eq!(by("retention"), CheckStatus::Warn);
    }

    /// GitHub-style anchors of the guide's headings.
    fn anchors() -> Vec<String> {
        include_str!("../../../docs/backup-ransomware-hardening.md")
            .lines()
            .filter(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect::<String>().replace(' ', "-"))
            .collect()
    }

    #[test]
    fn every_check_links_to_a_guide_section() {
        let a = anchors();
        let mut j = job();
        j.legal_hold = Some(crate::backup_engine::legal_hold::LegalHold { matter: "m".into(), reason: "r".into(), placed_by: "u".into(), placed_at: "t".into() });
        let probes = [None, Some(ProbeResult { can_write: true, can_delete: Some(false), ..Default::default() }), Some(ProbeResult::default())];
        for p in &probes {
            for c in evaluate(&j, 0, p.as_ref()).into_iter().chain(evaluate(&job(), 0, p.as_ref())) {
                let anchor = c.doc.split('#').nth(1).unwrap();
                assert!(a.iter().any(|x| x == anchor), "{} links to missing #{anchor}; headings: {a:?}", c.id);
            }
        }
    }
}
