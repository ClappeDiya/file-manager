//! Observable local folder transfers: plan first, then queue each real file.
use crate::core::{error::AppError, types::TransferJob};
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::transfer_engine::{
    location::{self as loc, Location, LocationItem, TransferMode},
    TransferManager,
};
use std::path::PathBuf;
use tauri::State;

#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalTransferOutcome {
    pub jobs: Vec<TransferJob>,
    pub dest_paths: Vec<String>,
    pub folders_created: usize,
}

#[tauri::command]
pub async fn enqueue_local_transfer(
    source_paths: Vec<String>,
    dest_dir: String,
    mode: TransferMode,
    manager: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<LocalTransferOutcome, AppError> {
    let result = local_transfer(&manager, source_paths, &dest_dir, mode).await?;
    ledger
        .record(
            RecordEvent::new(LedgerEngine::Transfer, "enqueue")
                .status(LedgerStatus::Ok)
                .summary(format!("Queued {} file(s) → {dest_dir}", result.jobs.len()))
                .target(dest_dir),
        )
        .await;
    Ok(result)
}

pub(crate) async fn local_transfer(
    manager: &TransferManager,
    sources: Vec<String>,
    dest_dir: &str,
    mode: TransferMode,
) -> Result<LocalTransferOutcome, AppError> {
    if sources.is_empty() {
        return Err(AppError::validation("Select at least one file or folder."));
    }
    // Canonical paths catch aliases (including macOS /var and /private/var),
    // parent traversals and destinations linked back inside the source.
    let dest = tokio::fs::canonicalize(dest_dir).await.map_err(|e| {
        AppError::file_op(
            format!("Cannot open destination {dest_dir}: {e}"),
            "Choose an existing, writable destination folder.",
        )
    })?;
    if !dest.is_dir() {
        return Err(AppError::validation("The destination must be a folder."));
    }
    if manager.try_journal().await.is_none() {
        return Err(AppError::validation(
            "The transfer service is starting. Try again in a moment.",
        ));
    }
    let mut items = Vec::new();
    for path in sources {
        let meta = tokio::fs::symlink_metadata(&path).await.map_err(|e| {
            AppError::file_op(
                format!("Cannot open source {path}: {e}"),
                "Check the source still exists and is readable.",
            )
        })?;
        if meta.file_type().is_symlink() {
            return Err(AppError::validation(
                "Select the target of a symbolic link explicitly.",
            ));
        }
        if !meta.is_file() && !meta.is_dir() {
            return Err(AppError::validation(
                "Only regular files and folders can be transferred.",
            ));
        }
        let path = tokio::fs::canonicalize(&path).await.map_err(|e| {
            AppError::file_op(
                format!("Cannot resolve source {path}: {e}"),
                "Choose an existing source.",
            )
        })?;
        if meta.is_dir() && dest.starts_with(&path) {
            return Err(AppError::validation(
                "The destination cannot be inside the source folder.",
            ));
        }
        let name = path
            .file_name()
            .ok_or_else(|| AppError::validation("Choose a folder inside the filesystem root."))?
            .to_string_lossy()
            .to_string();
        let path = path.to_string_lossy().to_string();
        if items.iter().any(|i: &LocationItem| i.path == path) {
            continue;
        }
        items.push(LocationItem {
            path,
            name,
            is_dir: meta.is_dir(),
            size: meta.len(),
            extension: None,
        });
    }
    let dest = Location::Local(dest.to_string_lossy().to_string());
    let resolver = loc::LocalOnlyResolver;
    let mut taken = loc::taken_names(loc::list(&dest, &resolver).await?);
    let mut plan = loc::TransferPlan::default();
    let mut dest_paths = Vec::new();
    for item in &items {
        if mode == TransferMode::Move {
            let target = dest.join_checked(&item.name)?.to_location_string();
            if tokio::fs::symlink_metadata(&target).await.is_ok() {
                return Err(AppError::file_op(
                    format!("{target} already exists; the move did not start."),
                    "Rename the source or choose another destination.",
                ));
            }
        }
        if items.iter().any(|other| {
            other.path != item.path
                && other.is_dir
                && std::path::Path::new(&item.path).starts_with(&other.path)
        }) {
            return Err(AppError::validation(
                "Select a folder or its contents, rather than both.",
            ));
        }
        let part =
            loc::plan_transfer_taking(std::slice::from_ref(item), &dest, &resolver, &mut taken)
                .await?;
        let target = if item.is_dir {
            part.dirs.first()
        } else {
            part.files.first().map(|f| &f.dest)
        };
        dest_paths.push(
            target
                .cloned()
                .ok_or_else(|| AppError::internal("No destination was planned"))?,
        );
        plan.files.extend(part.files);
        plan.dirs.extend(part.dirs);
        plan.source_dirs.extend(part.source_dirs);
        if plan.files.len() > loc::MAX_PLANNED_FILES {
            return Err(AppError::validation("Transfer fewer files at a time."));
        }
    }
    let targets: Vec<PathBuf> = plan
        .files
        .iter()
        .map(|f| PathBuf::from(&f.dest))
        .chain(plan.dirs.iter().map(PathBuf::from))
        .collect();
    super::policy_commands::enforce(ufop_core::policy::active().check_all(&targets))?;
    loc::create_dirs(&mut plan, &resolver).await?;
    let mut jobs = Vec::new();
    for file in &plan.files {
        jobs.push(
            manager
                .enqueue_location_job(&file.source, &file.dest, file.size)
                .await?,
        );
    }
    if mode == TransferMode::Move {
        if jobs.is_empty() {
            for dir in &plan.source_dirs {
                loc::remove_empty_dir(&Location::parse(dir)?, &resolver).await?;
            }
        } else {
            manager
                .register_move(
                    jobs.iter()
                        .zip(&plan.files)
                        .map(|(j, f)| (j.id, f.source.clone()))
                        .collect(),
                    plan.source_dirs.clone(),
                )
                .await;
        }
    }
    for job in &jobs {
        manager.spawn_runner(job.id).await;
    }
    Ok(LocalTransferOutcome {
        jobs,
        dest_paths,
        folders_created: plan.dirs.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{traits::TransferOperations, types::TransferStatus};
    async fn manager() -> TransferManager {
        let manager = TransferManager::new();
        let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();
        manager.init_journal(pool).await;
        manager
    }
    async fn settled(manager: &TransferManager, out: &LocalTransferOutcome) {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let jobs = manager.list_jobs().await.unwrap();
                if out.jobs.iter().all(|j| {
                    jobs.iter().any(|x| {
                        x.id == j.id
                            && matches!(
                                x.status,
                                TransferStatus::Completed
                                    | TransferStatus::Failed
                                    | TransferStatus::Cancelled
                            )
                    })
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        for id in &out.jobs {
            let job = manager.get_job(id.id).await.unwrap().unwrap();
            assert_eq!(
                job.status,
                TransferStatus::Completed,
                "{:?}",
                job.error_message
            );
            assert_eq!(job.source_checksum, job.dest_checksum);
            assert!(job.source_checksum.is_some());
        }
    }
    #[tokio::test]
    async fn queued_folder_copy_reports_real_files_keeps_both_and_empty_dirs() {
        let manager = manager().await;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");
        std::fs::create_dir_all(source.join("nested/empty")).unwrap();
        std::fs::create_dir_all(dest.join("source")).unwrap();
        std::fs::write(source.join("a.txt"), b"alpha").unwrap();
        std::fs::write(source.join("nested/b.bin"), vec![73; 262144]).unwrap();
        std::fs::write(dest.join("source/untouched"), b"keep me").unwrap();
        let out = local_transfer(
            &manager,
            vec![source.to_string_lossy().into()],
            dest.to_str().unwrap(),
            TransferMode::Copy,
        )
        .await
        .unwrap();
        assert_eq!(out.jobs.len(), 2);
        assert_eq!(out.dest_paths.len(), 1);
        settled(&manager, &out).await;
        let actual = PathBuf::from(&out.dest_paths[0]);
        assert_ne!(actual, dest.join("source"));
        assert_eq!(std::fs::read(actual.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(
            std::fs::read(actual.join("nested/b.bin")).unwrap(),
            vec![73; 262144]
        );
        assert!(actual.join("nested/empty").is_dir());
        assert!(source.join("a.txt").exists());
        assert_eq!(
            std::fs::read(dest.join("source/untouched")).unwrap(),
            b"keep me"
        );
    }
    #[tokio::test]
    async fn queued_move_removes_original_only_after_verified_copy() {
        let manager = manager().await;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(source.join("nested/a"), b"move content").unwrap();
        let out = local_transfer(
            &manager,
            vec![source.to_string_lossy().into()],
            dest.to_str().unwrap(),
            TransferMode::Move,
        )
        .await
        .unwrap();
        settled(&manager, &out).await;
        // Cleanup follows completion; allow the executor to finish that step.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while source.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(PathBuf::from(&out.dest_paths[0]).join("nested/a")).unwrap(),
            b"move content"
        );
    }
    #[tokio::test]
    async fn invalid_and_self_nested_sources_change_nothing() {
        let manager = manager().await;
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("source");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        assert!(local_transfer(
            &manager,
            vec![src.to_string_lossy().into()],
            src.join("nested").to_str().unwrap(),
            TransferMode::Copy
        )
        .await
        .is_err());
        assert!(local_transfer(
            &manager,
            vec![temp.path().join("missing").to_string_lossy().into()],
            temp.path().to_str().unwrap(),
            TransferMode::Copy
        )
        .await
        .is_err());
        assert!(manager.list_jobs().await.unwrap().is_empty());
        assert!(std::fs::read_dir(src.join("nested"))
            .unwrap()
            .next()
            .is_none());
    }
    #[tokio::test]
    async fn move_conflict_preserves_both_existing_files() {
        let manager = manager().await;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("a.txt");
        let dest = temp.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(&source, b"original source").unwrap();
        std::fs::write(dest.join("a.txt"), b"existing destination").unwrap();
        assert!(local_transfer(
            &manager,
            vec![source.to_string_lossy().into()],
            dest.to_str().unwrap(),
            TransferMode::Move
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(source).unwrap(), b"original source");
        assert_eq!(
            std::fs::read(dest.join("a.txt")).unwrap(),
            b"existing destination"
        );
        assert!(manager.list_jobs().await.unwrap().is_empty());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn folder_with_symlink_loop_is_refused_before_writes() {
        let manager = manager().await;
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("source");
        let dest = temp.path().join("dest");
        std::fs::create_dir(&src).unwrap();
        std::fs::create_dir(&dest).unwrap();
        std::os::unix::fs::symlink(&src, src.join("loop")).unwrap();
        assert!(local_transfer(
            &manager,
            vec![src.to_string_lossy().into()],
            dest.to_str().unwrap(),
            TransferMode::Copy
        )
        .await
        .is_err());
        assert!(std::fs::read_dir(dest).unwrap().next().is_none());
    }
}
