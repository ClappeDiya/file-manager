//! Live-provider contract tests: the backend compatibility test plus a full
//! backup → list → restore → verify → delete version → compact cycle
//! against REAL services. Each provider runs only when its environment
//! variables are set (otherwise it is skipped with the reason printed),
//! works under a unique `ufop-live/<run id>` folder, and always removes
//! everything it wrote.
//!
//! ```text
//! cargo test --lib live_providers -- --nocapture
//! UFOP_LIVE_USE_MOCKS=1 …   # the same suite against the in-process S3, Azure and GCS mocks
//! ```
//!
//! Variables (see docs/backup-destinations.md, "Live provider tests"):
//! S3 `UFOP_LIVE_S3_BUCKET`, `_KEY`, `_SECRET` (+ `_ENDPOINT`, `_REGION`);
//! Azure `UFOP_LIVE_AZURE_ACCOUNT`, `_CONTAINER` and `_KEY`, or `_TENANT`,
//! `_CLIENT_ID`, `_CLIENT_SECRET`; GCS `UFOP_LIVE_GCS_BUCKET`, `_KEY_JSON`;
//! B2 `UFOP_LIVE_B2_KEY_ID`, `_KEY`, `_BUCKET`; Swift `UFOP_LIVE_SWIFT_AUTH_URL`,
//! `_USER`, `_PASSWORD`, `_PROJECT`, `_CONTAINER` (+ `_REGION`); Dropbox
//! `UFOP_LIVE_DROPBOX_TOKEN`; OneDrive `UFOP_LIVE_ONEDRIVE_TOKEN` (+ `_DRIVE`);
//! Google Drive `UFOP_LIVE_GDRIVE_TOKEN`; Filen `UFOP_LIVE_FILEN_EMAIL`,
//! `_PASSWORD`; Drime `UFOP_LIVE_DRIME_TOKEN`. Tokens are the same OAuth
//! token JSON the destination's credentials field takes.

use super::*;
use std::sync::Arc;
use ufop_core::backup::crypto::KdfParams;
use ufop_core::backup::db::BackupDb;
use ufop_core::backup::*;

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// All of `keys`, or the names of the missing ones.
fn need(keys: &[&str]) -> Result<Vec<String>, String> {
    let missing: Vec<&str> = keys.iter().copied().filter(|k| var(k).is_none()).collect();
    if missing.is_empty() {
        Ok(keys.iter().map(|k| var(k).unwrap_or_default()).collect())
    } else {
        Err(format!("set {}", missing.join(", ")))
    }
}

/// A provider to test: its configuration and secret, or why it is skipped.
pub(crate) struct Target {
    pub name: &'static str,
    pub config: Result<(CloudConfig, Option<String>), String>,
}

/// Real providers configured through the environment, each in `folder`.
pub(crate) fn live_targets(folder: &str) -> Vec<Target> {
    let t = |name, config| Target { name, config };
    vec![
        t("s3", need(&["UFOP_LIVE_S3_BUCKET", "UFOP_LIVE_S3_KEY", "UFOP_LIVE_S3_SECRET"]).map(|v| {
            let endpoint = var("UFOP_LIVE_S3_ENDPOINT").unwrap_or_default();
            let cfg = s3::S3Config {
                preset: if endpoint.is_empty() { "aws".into() } else { "custom".into() },
                path_style: var("UFOP_LIVE_S3_PATH_STYLE").is_some(),
                endpoint,
                region: var("UFOP_LIVE_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                bucket: v[0].clone(),
                prefix: folder.into(),
                access_key_id: v[1].clone(),
                ..Default::default()
            };
            (CloudConfig::S3(cfg), Some(v[2].clone()))
        })),
        t("azure", need(&["UFOP_LIVE_AZURE_ACCOUNT", "UFOP_LIVE_AZURE_CONTAINER"]).and_then(|v| {
            let base = azure::AzureConfig { account: v[0].clone(), container: v[1].clone(), prefix: folder.into(), endpoint: var("UFOP_LIVE_AZURE_ENDPOINT").unwrap_or_default(), ..Default::default() };
            if let Some(k) = var("UFOP_LIVE_AZURE_KEY") {
                return Ok((CloudConfig::Azure(base), Some(k)));
            }
            let sp = need(&["UFOP_LIVE_AZURE_TENANT", "UFOP_LIVE_AZURE_CLIENT_ID", "UFOP_LIVE_AZURE_CLIENT_SECRET"]).map_err(|e| format!("set UFOP_LIVE_AZURE_KEY, or {}", e.trim_start_matches("set ")))?;
            let auth = azure::AzureAuth::ServicePrincipalSecret { tenant: sp[0].clone(), client_id: sp[1].clone() };
            Ok((CloudConfig::Azure(azure::AzureConfig { auth, ..base }), Some(sp[2].clone())))
        })),
        t("gcs", need(&["UFOP_LIVE_GCS_BUCKET", "UFOP_LIVE_GCS_KEY_JSON"]).map(|v| (CloudConfig::Gcs(gcs::GcsConfig { bucket: v[0].clone(), prefix: folder.into(), ..Default::default() }), Some(v[1].clone())))),
        t("b2", need(&["UFOP_LIVE_B2_KEY_ID", "UFOP_LIVE_B2_KEY", "UFOP_LIVE_B2_BUCKET"]).map(|v| (CloudConfig::B2(b2::B2Config { key_id: v[0].clone(), bucket: v[2].clone(), prefix: folder.into(), ..Default::default() }), Some(v[1].clone())))),
        t("swift", need(&["UFOP_LIVE_SWIFT_AUTH_URL", "UFOP_LIVE_SWIFT_USER", "UFOP_LIVE_SWIFT_PASSWORD", "UFOP_LIVE_SWIFT_PROJECT", "UFOP_LIVE_SWIFT_CONTAINER"]).map(|v| {
            let cfg = swift::SwiftConfig { auth_url: v[0].clone(), username: v[1].clone(), project: v[3].clone(), container: v[4].clone(), region: var("UFOP_LIVE_SWIFT_REGION").unwrap_or_default(), prefix: folder.into(), ..Default::default() };
            (CloudConfig::Swift(cfg), Some(v[2].clone()))
        })),
        t("dropbox", need(&["UFOP_LIVE_DROPBOX_TOKEN"]).map(|v| (CloudConfig::Dropbox(oauth_drive::DriveConfig { folder: format!("/{folder}"), ..Default::default() }), Some(v[0].clone())))),
        t("onedrive", need(&["UFOP_LIVE_ONEDRIVE_TOKEN"]).map(|v| {
            let cfg = graph::GraphConfig { drive: var("UFOP_LIVE_ONEDRIVE_DRIVE").unwrap_or_else(|| "me".into()), folder: folder.into(), ..Default::default() };
            (CloudConfig::OneDrive(cfg), Some(v[0].clone()))
        })),
        t("google_drive", need(&["UFOP_LIVE_GDRIVE_TOKEN"]).map(|v| (CloudConfig::GoogleDrive(oauth_drive::DriveConfig { folder: folder.into(), ..Default::default() }), Some(v[0].clone())))),
        t("filen", need(&["UFOP_LIVE_FILEN_EMAIL", "UFOP_LIVE_FILEN_PASSWORD"]).map(|v| (CloudConfig::Filen(filen::FilenConfig { email: v[0].clone(), folder: folder.into(), ..Default::default() }), Some(v[1].clone())))),
        t("drime", need(&["UFOP_LIVE_DRIME_TOKEN"]).map(|v| (CloudConfig::Drime(drime::DrimeConfig { folder: folder.into(), ..Default::default() }), Some(v[0].clone())))),
    ]
}

/// The in-process mocks as targets (`UFOP_LIVE_USE_MOCKS=1`, and always in
/// [`contract_cycle_against_mocks`]).
pub(crate) async fn mock_targets(folder: &str) -> Vec<Target> {
    let (s3_base, st) = super::mock_s3::serve().await;
    st.lock().unwrap().buckets.push("bkt".into());
    let s3 = s3::S3Config { preset: "custom".into(), endpoint: s3_base, bucket: "bkt".into(), prefix: folder.into(), access_key_id: "AKIDBASE".into(), path_style: true, ..Default::default() };
    let (az_base, az_ep, _) = super::mock_azure::serve().await;
    let az = azure::AzureConfig {
        account: crate::connectors::azure_rest::DEV_ACCOUNT.into(),
        container: "live".into(),
        prefix: folder.into(),
        endpoint: az_ep,
        auth: azure::AzureAuth::ServicePrincipalSecret { tenant: super::mock_azure::TENANT.into(), client_id: super::mock_azure::CLIENT.into() },
        authority_host: az_base,
        ..Default::default()
    };
    let (gcs_base, _) = super::mock_gcs::serve().await;
    let gcs = gcs::GcsConfig { bucket: "bkt".into(), prefix: folder.into(), endpoint: gcs_base.clone(), ..Default::default() };
    vec![
        Target { name: "s3 (mock)", config: Ok((CloudConfig::S3(s3), Some("basesecret".into()))) },
        Target { name: "azure (mock)", config: Ok((CloudConfig::Azure(az), Some(super::mock_azure::CLIENT_SECRET.into()))) },
        Target { name: "gcs (mock)", config: Ok((CloudConfig::Gcs(gcs), Some(super::mock_gcs::service_account(&gcs_base)))) },
    ]
}

/// Remove everything below the test folder (also after a failure).
fn clean(backend: &dyn Backend) -> Result<usize, AppError> {
    let files = if backend.supports_subfolders() { backend.list_recursive()? } else { backend.list()? };
    for f in &files {
        backend.delete(&f.name)?;
    }
    Ok(files.len())
}

/// The contract: compatibility round trips, then two backups, listing,
/// restore with byte comparison, full verification, deleting the older
/// version and compacting, and a final verification.
fn cycle(backend: &dyn Backend, name: &str) -> Result<Vec<String>, AppError> {
    let mut log = ufop_core::backup::backend::test_backend_compatibility(backend, 3, 1024, 256 * 1024)?;
    let src = tempfile::tempdir()?;
    let out = tempfile::tempdir()?;
    let first: Vec<u8> = (0..400_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    std::fs::write(src.path().join("a.bin"), &first)?;
    std::fs::write(src.path().join("note.txt"), b"first version")?;
    let o = BackupOptions { block_size: 32 * 1024, volume_size: 150_000, verify_samples: 1, quota_disable: true, ..Default::default() };
    let db = BackupDb::open_in_memory()?;
    let run = |db: &BackupDb| run_backup(backend, db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("live-pw"), KdfParams::insecure_fast(), &OpContext::new());
    let r = run(&db)?;
    if !r.errors.is_empty() {
        return Err(AppError::transfer(format!("{name}: first backup: {:?}", r.errors), "See the run errors."));
    }
    std::fs::write(src.path().join("note.txt"), b"second version, longer")?;
    let r = run(&db)?;
    if !r.errors.is_empty() {
        return Err(AppError::transfer(format!("{name}: second backup: {:?}", r.errors), "See the run errors."));
    }
    log.push("OK two backups".into());
    // Listing from the destination alone: rebuild the index into a fresh database.
    let db2 = BackupDb::open_in_memory()?;
    maintenance::recreate_database(backend, &db2, Some("live-pw"), "ufop", &OpContext::new())?;
    let versions = db2.filesets()?;
    if versions.len() != 2 {
        return Err(AppError::transfer(format!("{name}: {} versions listed, expected 2", versions.len()), "The listing lost file lists."));
    }
    log.push("OK listed 2 versions after rebuilding the index".into());
    let repo = engine::Repo::open(backend, &db2, Some("live-pw"), &o, false, KdfParams::insecure_fast())?;
    let rr = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new())?;
    let root = out.path().join(src.path().file_name().unwrap_or_default());
    if !rr.errors.is_empty() || std::fs::read(root.join("a.bin"))? != first || std::fs::read(root.join("note.txt"))? != b"second version, longer" {
        return Err(AppError::transfer(format!("{name}: restored data differs ({:?})", rr.errors), "The destination corrupted or lost data."));
    }
    log.push("OK restore matches byte for byte".into());
    let v = maintenance::verify(&repo, None, true)?;
    if !v.errors.is_empty() {
        return Err(AppError::transfer(format!("{name}: verify: {:?}", v.errors), "Volumes are damaged."));
    }
    log.push(format!("OK verified {} volumes", v.checked));
    let removed = maintenance::delete_versions(&repo, &[1], false)?;
    let c = maintenance::compact(&repo, &o, &OpContext::new())?;
    let v = maintenance::verify(&repo, None, true)?;
    if removed != 1 || !v.errors.is_empty() {
        return Err(AppError::transfer(format!("{name}: delete/compact: removed {removed}, verify {:?}", v.errors), "Compaction broke the repository."));
    }
    log.push(format!("OK deleted the older version, compacted ({} volumes removed, {} rewritten), verified again", c.volumes_removed, c.volumes_rewritten));
    Ok(log)
}

/// Run every target that is configured; returns (name, outcome) lines and
/// whether any failed.
pub(crate) async fn run_targets(targets: Vec<Target>) -> (Vec<String>, bool) {
    let (mut report, mut failed) = (Vec::new(), false);
    for t in targets {
        let (cfg, secret) = match t.config {
            Ok(c) => c,
            Err(why) => {
                report.push(format!("SKIP {}: {why}", t.name));
                continue;
            }
        };
        let store = match open(&cfg, secret).await {
            Ok(s) => s,
            Err(e) => {
                failed = true;
                report.push(format!("FAIL {}: cannot connect: {}", t.name, e.user_message()));
                continue;
            }
        };
        let backend = Arc::new(StoreBackend::new(store, Some(std::time::Duration::from_secs(300))));
        let (b, name) = (backend.clone(), t.name);
        let outcome = tokio::task::spawn_blocking(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cycle(b.as_ref(), name)));
            // Always clean up, whatever happened.
            let cleaned = clean(b.as_ref());
            (r, cleaned)
        })
        .await;
        match outcome {
            Ok((Ok(Ok(lines)), cleaned)) => {
                report.extend(lines.into_iter().map(|l| format!("{}: {l}", t.name)));
                match cleaned {
                    Ok(n) => report.push(format!("PASS {} (cleaned up {n} files)", t.name)),
                    Err(e) => {
                        failed = true;
                        report.push(format!("FAIL {}: cleanup: {}", t.name, e.user_message()));
                    }
                }
            }
            Ok((Ok(Err(e)), _)) => {
                failed = true;
                report.push(format!("FAIL {}: {}", t.name, e.user_message()));
            }
            Ok((Err(_), _)) | Err(_) => {
                failed = true;
                report.push(format!("FAIL {}: the cycle panicked", t.name));
            }
        }
    }
    (report, failed)
}

fn run_folder() -> String {
    format!("ufop-live/{}-{}", chrono::Utc::now().format("%Y%m%dT%H%M%S"), &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// Real providers from the environment (plus the mocks with
/// `UFOP_LIVE_USE_MOCKS=1`); unset ones are skipped with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn live_provider_contracts() {
    let folder = run_folder();
    let mut targets = live_targets(&folder);
    if var("UFOP_LIVE_USE_MOCKS").is_some() {
        targets.extend(mock_targets(&folder).await);
    }
    let (report, failed) = run_targets(targets).await;
    println!("live provider contracts in {folder}:\n  {}", report.join("\n  "));
    assert!(!failed, "{}", report.join("\n"));
}

/// The same cycle always runs against the in-process mocks, so the harness
/// itself stays correct without any account.
#[tokio::test(flavor = "multi_thread")]
async fn contract_cycle_against_mocks() {
    let (report, failed) = run_targets(mock_targets(&run_folder()).await).await;
    assert!(!failed, "{}", report.join("\n"));
    assert_eq!(report.iter().filter(|l| l.starts_with("PASS")).count(), 3, "{report:?}");
    assert!(report.iter().any(|l| l.contains("restore matches byte for byte")));
    // Unset providers are reported as skipped, with the variables to set.
    let skipped = live_targets("x").into_iter().filter_map(|t| t.config.err().map(|e| (t.name, e))).collect::<Vec<_>>();
    if var("UFOP_LIVE_S3_BUCKET").is_none() {
        assert!(skipped.iter().any(|(n, e)| *n == "s3" && e.contains("UFOP_LIVE_S3_BUCKET")), "{skipped:?}");
    }
}
