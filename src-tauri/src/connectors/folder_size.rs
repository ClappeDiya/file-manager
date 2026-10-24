//! Recursive size of a remote folder: the connector's server-side figure
//! when it has one (rclone `size`), else a walk of `list_remote` with a
//! bounded number of listings in flight, which a cancel token stops (the
//! partial figures are returned with `complete: false`).

use super::{Connector, FolderSize};
use crate::core::error::AppError;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Listings in flight at once while walking.
pub const DEFAULT_CONCURRENCY: usize = 8;

/// Size of `root` on `connector` (connected).
pub async fn folder_size(connector: Arc<dyn Connector>, root: &str, concurrency: usize, cancel: CancellationToken) -> Result<FolderSize, AppError> {
    if let Some(fast) = connector.folder_size_fast(root).await? {
        return Ok(fast);
    }
    walk(connector, root, concurrency, cancel).await
}

/// Walk every folder below `root`, `concurrency` listings at a time.
pub async fn walk(connector: Arc<dyn Connector>, root: &str, concurrency: usize, cancel: CancellationToken) -> Result<FolderSize, AppError> {
    let mut total = FolderSize { complete: true, folders: Some(0), ..Default::default() };
    let mut pending: Vec<String> = vec![root.to_string()];
    let mut running: JoinSet<Result<Vec<crate::core::types::FileEntry>, AppError>> = JoinSet::new();
    let limit = concurrency.max(1);
    loop {
        while running.len() < limit {
            let Some(dir) = pending.pop() else { break };
            let c = connector.clone();
            running.spawn(async move { c.list_remote(&dir).await });
        }
        if running.is_empty() {
            break;
        }
        let next = tokio::select! {
            r = running.join_next() => r,
            _ = cancel.cancelled() => {
                running.abort_all();
                total.complete = false;
                return Ok(total);
            }
        };
        let Some(joined) = next else { break };
        let entries = joined.map_err(|e| AppError::internal(format!("Folder size task failed: {e}")))??;
        for e in entries {
            if e.is_dir {
                *total.folders.get_or_insert(0) += 1;
                pending.push(e.path);
            } else {
                total.files += 1;
                total.bytes += e.size;
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{ConnectionProfile, FileEntry};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A tree: /a (2 files), /a/b (1 file), /c (empty), plus 1 file at the
    /// root; each listing takes a moment and in-flight listings are counted.
    struct Tree {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        delay_ms: u64,
    }

    fn f(path: &str, size: u64) -> FileEntry {
        FileEntry { name: path.rsplit('/').next().unwrap().into(), path: path.into(), is_dir: false, is_symlink: false, size, modified: None, created: None, is_hidden: false, extension: None, permissions: None }
    }

    fn d(path: &str) -> FileEntry {
        FileEntry { is_dir: true, ..f(path, 0) }
    }

    impl Connector for Tree {
        fn connect(&self, _: &ConnectionProfile) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn list_remote(&self, path: &str) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>> {
            let path = path.to_string();
            Box::pin(async move {
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_in_flight.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(match path.as_str() {
                    "/" => vec![d("/a"), d("/c"), f("/top.bin", 5)],
                    "/a" => vec![f("/a/1", 10), f("/a/2", 20), d("/a/b")],
                    "/a/b" => vec![f("/a/b/3", 100)],
                    _ => vec![],
                })
            })
        }
    }

    fn tree(delay_ms: u64) -> Arc<Tree> {
        Arc::new(Tree { in_flight: AtomicUsize::new(0), max_in_flight: AtomicUsize::new(0), delay_ms })
    }

    #[tokio::test]
    async fn walks_the_whole_tree_with_bounded_concurrency() {
        let t = tree(20);
        let size = folder_size(t.clone(), "/", 2, CancellationToken::new()).await.unwrap();
        assert_eq!(size, FolderSize { bytes: 135, files: 4, folders: Some(3), complete: true });
        assert!(t.max_in_flight.load(Ordering::SeqCst) <= 2);
        assert_eq!(walk(tree(0), "/a", 8, CancellationToken::new()).await.unwrap(), FolderSize { bytes: 130, files: 3, folders: Some(1), complete: true });
    }

    #[tokio::test]
    async fn a_cancelled_walk_returns_partial_figures() {
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            c2.cancel();
        });
        let size = walk(tree(1000), "/", 1, cancel).await.unwrap();
        assert!(!size.complete);
        assert_eq!(size.files, 0);
    }
}
