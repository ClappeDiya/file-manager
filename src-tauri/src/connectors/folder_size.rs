//! Recursive size of a remote folder: the connector's server-side figure
//! when it has one (rclone `size`), else a walk of `list_remote` with a
//! bounded number of listings in flight, which a cancel token stops (the
//! partial figures are returned with `complete: false`).

use super::{Connector, FolderSize};
use crate::core::error::AppError;
use std::sync::Arc;
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
    walk_with(move |dir: String| {
        let c = connector.clone();
        async move { c.list_remote(&dir).await }
    }, root, concurrency, cancel)
    .await
}

/// [`walk`] over any lister (`list(dir)` → the entries in `dir`).
pub async fn walk_with<L, F>(list: L, root: &str, concurrency: usize, cancel: CancellationToken) -> Result<FolderSize, AppError>
where
    L: Fn(String) -> F,
    F: std::future::Future<Output = Result<Vec<crate::core::types::FileEntry>, AppError>> + Send + 'static,
{
    let mut total = FolderSize { complete: true, folders: Some(0), ..Default::default() };
    let complete = super::walk::walk_tree(list, root.to_string(), concurrency, &cancel, |_dir, listing| {
        let mut dirs = Vec::new();
        for e in listing? {
            if e.is_dir {
                *total.folders.get_or_insert(0) += 1;
                dirs.push(e.path);
            } else {
                total.files += 1;
                total.bytes += e.size;
            }
        }
        Ok(dirs)
    })
    .await?;
    total.complete = complete;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::FileEntry;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn f(path: &str, size: u64) -> FileEntry {
        FileEntry { name: path.rsplit('/').next().unwrap().into(), path: path.into(), is_dir: false, is_symlink: false, size, modified: None, created: None, is_hidden: false, extension: None, permissions: None }
    }

    fn d(path: &str) -> FileEntry {
        FileEntry { is_dir: true, ..f(path, 0) }
    }

    type Listing = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<FileEntry>, AppError>> + Send>>;

    /// A tree: /a (2 files), /a/b (1 file), /c (empty), plus 1 file at the
    /// root; each listing takes `delay_ms` and in-flight listings are counted.
    fn tree(delay_ms: u64) -> (impl Fn(String) -> Listing, Arc<AtomicUsize>) {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let m2 = max.clone();
        let lister = move |path: String| {
            let (in_flight, max) = (in_flight.clone(), m2.clone());
            Box::pin(async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(match path.as_str() {
                    "/" => vec![d("/a"), d("/c"), f("/top.bin", 5)],
                    "/a" => vec![f("/a/1", 10), f("/a/2", 20), d("/a/b")],
                    "/a/b" => vec![f("/a/b/3", 100)],
                    _ => vec![],
                })
            }) as Listing
        };
        (lister, max)
    }

    #[tokio::test]
    async fn walks_the_whole_tree_with_bounded_concurrency() {
        let (list, max) = tree(20);
        let size = walk_with(list, "/", 2, CancellationToken::new()).await.unwrap();
        assert_eq!(size, FolderSize { bytes: 135, files: 4, folders: Some(3), complete: true });
        assert!(max.load(Ordering::SeqCst) <= 2);
        let (list, _) = tree(0);
        assert_eq!(walk_with(list, "/a", 8, CancellationToken::new()).await.unwrap(), FolderSize { bytes: 130, files: 3, folders: Some(1), complete: true });
    }

    #[tokio::test]
    async fn a_cancelled_walk_returns_partial_figures() {
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            c2.cancel();
        });
        let (list, _) = tree(1000);
        let size = walk_with(list, "/", 1, cancel).await.unwrap();
        assert!(!size.complete);
        assert_eq!(size.files, 0);
    }

    /// The real connector path, over a local folder tree.
    #[tokio::test]
    async fn walks_a_real_connector() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("x/y")).unwrap();
        std::fs::write(t.path().join("x/a.bin"), [0u8; 7]).unwrap();
        std::fs::write(t.path().join("x/y/b.bin"), [0u8; 5]).unwrap();
        let c: Arc<dyn Connector> = Arc::new(crate::connectors::LocalDriveConnector::new());
        let root = t.path().to_string_lossy().to_string();
        c.connect(&crate::connectors::live_tests::profile(crate::core::types::ConnectionProtocol::Local, "", None, "", &root)).await.unwrap();
        let size = folder_size(c, &t.path().join("x").to_string_lossy(), 4, CancellationToken::new()).await.unwrap();
        assert_eq!((size.bytes, size.files, size.folders, size.complete), (12, 2, Some(1), true));
    }
}
