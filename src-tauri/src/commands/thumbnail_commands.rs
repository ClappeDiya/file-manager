//! Grid-view thumbnails (see `thumbnail_engine`).

use crate::core::error::AppError;
use crate::thumbnail_engine::ThumbnailCache;
use std::sync::{Arc, OnceLock};
use tauri::State;
use tokio::sync::Semaphore;

/// Decodes running at once: a folder of photos must not pin every core.
const PARALLEL_DECODES: usize = 4;

fn permits() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(PARALLEL_DECODES))
}

/// Path of the cached thumbnail (load it with `convertFileSrc`), or `null`
/// for files that aren't images.
#[tauri::command]
pub async fn thumbnail_get(path: String, size: u32, cache: State<'_, Arc<ThumbnailCache>>) -> Result<Option<String>, AppError> {
    let _permit = permits().acquire().await.map_err(|e| AppError::internal(e.to_string()))?;
    let cache = cache.inner().clone();
    tokio::task::spawn_blocking(move || cache.get(std::path::Path::new(&path), size))
        .await
        .map_err(|e| AppError::internal(format!("Thumbnail task failed: {e}")))?
        .map(|p| p.map(|p| p.to_string_lossy().to_string()))
}

/// Delete every cached thumbnail; returns how many were removed.
#[tauri::command]
pub async fn thumbnail_cache_clear(cache: State<'_, Arc<ThumbnailCache>>) -> Result<u64, AppError> {
    let cache = cache.inner().clone();
    tokio::task::spawn_blocking(move || cache.clear()).await.map_err(|e| AppError::internal(e.to_string()))?
}

#[cfg(test)]
mod tests {
    /// Regression: `convertFileSrc` previews (images, PDF, audio, video)
    /// and grid thumbnails need the asset protocol enabled and allowed by
    /// the CSP; without it they silently fail to load in the built app.
    #[test]
    fn asset_protocol_is_enabled_and_allowed_by_csp() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let security = &conf["app"]["security"];
        assert_eq!(security["assetProtocol"]["enable"], true);
        let deny = security["assetProtocol"]["scope"]["deny"].as_array().unwrap();
        assert!(deny.iter().any(|d| d == "$HOME/.ssh/**"), "keys stay out of reach");
        let csp = security["csp"].as_str().unwrap();
        for directive in ["img-src", "media-src", "frame-src"] {
            let part = csp.split(';').find(|p| p.trim().starts_with(directive)).unwrap_or_else(|| panic!("{directive} missing"));
            assert!(part.contains("asset:") && part.contains("http://asset.localhost"), "{directive} must allow the asset protocol");
        }
        let cargo = include_str!("../../Cargo.toml");
        assert!(cargo.contains("\"protocol-asset\""), "tauri needs the protocol-asset feature");
    }
}
