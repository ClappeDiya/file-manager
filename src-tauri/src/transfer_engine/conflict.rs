//! Conflict resolution engine (T-017).
//!
//! Policies:
//! - Overwrite always
//! - Skip always
//! - Overwrite if newer
//! - Overwrite if larger
//! - Rename (append number)
//! - Ask each time (pauses job, frontend shows comparison dialog)
//!
//! Policy settable per-transfer, per-connection, or global default.
//! Conflict resolution is logged in transfer summary.

use crate::core::error::AppError;
use crate::core::types::{ConflictPolicy, ConflictResolution};
use chrono::{DateTime, Utc};
use std::path::Path;
use uuid::Uuid;

/// Information about an existing file at the destination.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub exists: bool,
}

/// Result of conflict detection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConflictCheck {
    pub has_conflict: bool,
    pub source: FileInfo,
    pub dest: FileInfo,
}

/// Result of conflict resolution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    /// Proceed with transfer (overwrite destination).
    Proceed,
    /// Skip this transfer.
    Skip,
    /// Use an alternative destination path.
    Rename { new_path: String },
    /// Ask the user (pause the job and wait for input).
    AskUser,
}

/// Check whether a conflict exists at the destination.
pub async fn check_conflict(source_path: &str, dest_path: &str) -> Result<ConflictCheck, AppError> {
    let source_info = get_file_info(source_path).await?;
    let dest_info = get_file_info(dest_path).await?;

    Ok(ConflictCheck {
        has_conflict: dest_info.exists,
        source: source_info,
        dest: dest_info,
    })
}

/// Get file information (size, modified time, existence).
pub async fn get_file_info(path: &str) -> Result<FileInfo, AppError> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(FileInfo {
            path: path.to_string(),
            size: 0,
            modified: None,
            exists: false,
        });
    }

    let metadata = tokio::fs::metadata(p).await.map_err(|e| {
        AppError::Transfer {
            message: format!("Cannot read file metadata: {e}"),
            advice: "Check file permissions.".to_string(),
        }
    })?;

    let modified = metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from);

    Ok(FileInfo {
        path: path.to_string(),
        size: metadata.len(),
        modified,
        exists: true,
    })
}

/// Resolve a conflict based on the configured policy.
pub async fn resolve_conflict(
    job_id: Uuid,
    source_path: &str,
    dest_path: &str,
    policy: ConflictPolicy,
) -> Result<(ConflictAction, ConflictResolution), AppError> {
    let check = check_conflict(source_path, dest_path).await?;

    if !check.has_conflict {
        let resolution = ConflictResolution {
            job_id,
            source_path: source_path.to_string(),
            dest_path: dest_path.to_string(),
            policy_applied: policy,
            resolved_dest_path: dest_path.to_string(),
            source_size: check.source.size,
            dest_size: 0,
            source_modified: check.source.modified,
            dest_modified: None,
            timestamp: Utc::now(),
        };
        return Ok((ConflictAction::Proceed, resolution));
    }

    let action = match policy {
        ConflictPolicy::OverwriteAlways => ConflictAction::Proceed,

        ConflictPolicy::SkipAlways => ConflictAction::Skip,

        ConflictPolicy::OverwriteIfNewer => {
            match (check.source.modified, check.dest.modified) {
                (Some(src_mod), Some(dst_mod)) if src_mod > dst_mod => {
                    ConflictAction::Proceed
                }
                _ => ConflictAction::Skip,
            }
        }

        ConflictPolicy::OverwriteIfLarger => {
            if check.source.size > check.dest.size {
                ConflictAction::Proceed
            } else {
                ConflictAction::Skip
            }
        }

        ConflictPolicy::Rename => {
            let new_path = generate_unique_path(dest_path).await?;
            ConflictAction::Rename { new_path }
        }

        ConflictPolicy::AskEachTime => ConflictAction::AskUser,
    };

    let resolved_dest = match &action {
        ConflictAction::Rename { new_path } => new_path.clone(),
        _ => dest_path.to_string(),
    };

    let resolution = ConflictResolution {
        job_id,
        source_path: source_path.to_string(),
        dest_path: dest_path.to_string(),
        policy_applied: policy,
        resolved_dest_path: resolved_dest,
        source_size: check.source.size,
        dest_size: check.dest.size,
        source_modified: check.source.modified,
        dest_modified: check.dest.modified,
        timestamp: Utc::now(),
    };

    Ok((action, resolution))
}

/// Generate a unique path by appending a number to the filename.
/// Example: "/dst/file.txt" -> "/dst/file (1).txt"
pub async fn generate_unique_path(path: &str) -> Result<String, AppError> {
    let p = Path::new(path);
    let parent = p.parent().unwrap_or(Path::new(""));
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = p.extension().map(|e| e.to_string_lossy().to_string());

    for i in 1..=9999 {
        let new_name = match &ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let new_path = parent.join(&new_name);
        if !new_path.exists() {
            return Ok(new_path.to_string_lossy().to_string());
        }
    }

    Err(AppError::Transfer {
        message: "Cannot generate unique filename after 9999 attempts.".to_string(),
        advice: "Clean up the destination directory.".to_string(),
    })
}

/// Determine the effective conflict policy for a transfer.
/// Priority: per-job > per-connection > global default.
pub fn effective_policy(
    job_policy: Option<ConflictPolicy>,
    connection_policy: Option<ConflictPolicy>,
    global_policy: ConflictPolicy,
) -> ConflictPolicy {
    job_policy.unwrap_or_else(|| connection_policy.unwrap_or(global_policy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_conflict_no_file() {
        let check = check_conflict(
            "/nonexistent/source.txt",
            "/nonexistent/dest.txt",
        )
        .await
        .unwrap();
        assert!(!check.has_conflict);
        assert!(!check.source.exists);
        assert!(!check.dest.exists);
    }

    #[tokio::test]
    async fn test_check_conflict_existing_dest() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        tokio::fs::write(&src, b"source content").await.unwrap();
        tokio::fs::write(&dst, b"dest content").await.unwrap();

        let check = check_conflict(
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
        )
        .await
        .unwrap();
        assert!(check.has_conflict);
        assert!(check.source.exists);
        assert!(check.dest.exists);
    }

    #[tokio::test]
    async fn test_resolve_overwrite_always() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        tokio::fs::write(&src, b"source").await.unwrap();
        tokio::fs::write(&dst, b"dest").await.unwrap();

        let (action, resolution) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ConflictPolicy::OverwriteAlways,
        )
        .await
        .unwrap();

        assert!(matches!(action, ConflictAction::Proceed));
        assert_eq!(resolution.policy_applied, ConflictPolicy::OverwriteAlways);
    }

    #[tokio::test]
    async fn test_resolve_skip_always() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        tokio::fs::write(&src, b"source").await.unwrap();
        tokio::fs::write(&dst, b"dest").await.unwrap();

        let (action, _) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ConflictPolicy::SkipAlways,
        )
        .await
        .unwrap();

        assert!(matches!(action, ConflictAction::Skip));
    }

    #[tokio::test]
    async fn test_resolve_rename() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        tokio::fs::write(&src, b"source").await.unwrap();
        tokio::fs::write(&dst, b"dest").await.unwrap();

        let (action, resolution) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ConflictPolicy::Rename,
        )
        .await
        .unwrap();

        match action {
            ConflictAction::Rename { ref new_path } => {
                assert!(new_path.contains("dest (1).txt"));
                assert_eq!(resolution.resolved_dest_path, *new_path);
            }
            _ => panic!("Expected Rename action"),
        }
    }

    #[tokio::test]
    async fn test_resolve_overwrite_if_larger() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        tokio::fs::write(&src, b"larger source content here").await.unwrap();
        tokio::fs::write(&dst, b"small").await.unwrap();

        let (action, _) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ConflictPolicy::OverwriteIfLarger,
        )
        .await
        .unwrap();

        assert!(matches!(action, ConflictAction::Proceed));

        // Now with smaller source
        tokio::fs::write(&src, b"sm").await.unwrap();
        tokio::fs::write(&dst, b"larger dest").await.unwrap();

        let (action, _) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ConflictPolicy::OverwriteIfLarger,
        )
        .await
        .unwrap();

        assert!(matches!(action, ConflictAction::Skip));
    }

    #[tokio::test]
    async fn test_resolve_ask_each_time() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        tokio::fs::write(&src, b"source").await.unwrap();
        tokio::fs::write(&dst, b"dest").await.unwrap();

        let (action, _) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            ConflictPolicy::AskEachTime,
        )
        .await
        .unwrap();

        assert!(matches!(action, ConflictAction::AskUser));
    }

    #[tokio::test]
    async fn test_generate_unique_path() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("file.txt");
        tokio::fs::write(&existing, b"data").await.unwrap();

        let unique = generate_unique_path(existing.to_str().unwrap()).await.unwrap();
        assert!(unique.contains("file (1).txt"));
    }

    #[tokio::test]
    async fn test_generate_unique_path_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("file.txt");
        let copy1 = dir.path().join("file (1).txt");
        tokio::fs::write(&base, b"data").await.unwrap();
        tokio::fs::write(&copy1, b"data").await.unwrap();

        let unique = generate_unique_path(base.to_str().unwrap()).await.unwrap();
        assert!(unique.contains("file (2).txt"));
    }

    #[test]
    fn test_effective_policy() {
        // Job policy takes precedence
        assert_eq!(
            effective_policy(
                Some(ConflictPolicy::OverwriteAlways),
                Some(ConflictPolicy::SkipAlways),
                ConflictPolicy::AskEachTime,
            ),
            ConflictPolicy::OverwriteAlways
        );

        // Connection policy next
        assert_eq!(
            effective_policy(
                None,
                Some(ConflictPolicy::SkipAlways),
                ConflictPolicy::AskEachTime,
            ),
            ConflictPolicy::SkipAlways
        );

        // Global default
        assert_eq!(
            effective_policy(None, None, ConflictPolicy::Rename),
            ConflictPolicy::Rename
        );
    }

    #[tokio::test]
    async fn test_no_conflict_proceeds() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.txt");
        tokio::fs::write(&src, b"data").await.unwrap();

        let (action, _) = resolve_conflict(
            Uuid::new_v4(),
            src.to_str().unwrap(),
            "/nonexistent/dest.txt",
            ConflictPolicy::AskEachTime,
        )
        .await
        .unwrap();

        assert!(matches!(action, ConflictAction::Proceed));
    }
}
