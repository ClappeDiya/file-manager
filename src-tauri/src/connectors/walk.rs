//! Walk a folder tree with a bounded number of listings in flight.
//!
//! The one tree walker behind the recursive folder size
//! ([`super::folder_size`]) and the folder comparison
//! ([`crate::compare_engine`]): `list(dir)` lists one folder (a connector's
//! `list_remote`, or a local `read_dir`), `visit(dir, listing)` takes what
//! it returned and names the folders to list next. A cancel token stops it.

use crate::core::error::AppError;
use std::future::Future;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Walk from `root`, `concurrency` listings at a time. `visit` sees every
/// listing as it lands and returns the folders below it to list; an error
/// from it stops the walk. `Ok(false)` when `cancel` stopped it first.
pub async fn walk_tree<P, T, L, F, V>(list: L, root: P, concurrency: usize, cancel: &CancellationToken, mut visit: V) -> Result<bool, AppError>
where
    P: Clone + Send + 'static,
    T: Send + 'static,
    L: Fn(P) -> F,
    F: Future<Output = T> + Send + 'static,
    V: FnMut(P, T) -> Result<Vec<P>, AppError>,
{
    let mut pending: Vec<P> = vec![root];
    let mut running: JoinSet<(P, T)> = JoinSet::new();
    let limit = concurrency.max(1);
    loop {
        while running.len() < limit {
            let Some(dir) = pending.pop() else { break };
            let listing = list(dir.clone());
            running.spawn(async move { (dir, listing.await) });
        }
        if running.is_empty() {
            return Ok(true);
        }
        let next = tokio::select! {
            r = running.join_next() => r,
            _ = cancel.cancelled() => {
                running.abort_all();
                return Ok(false);
            }
        };
        let Some(joined) = next else { return Ok(true) };
        let (dir, listing) = joined.map_err(|e| AppError::internal(format!("Listing task failed: {e}")))?;
        pending.extend(visit(dir, listing)?);
    }
}
