//! Image thumbnails for the grid view, cached on disk.
//!
//! A thumbnail is keyed by the source path, its size and mtime, and the
//! requested pixel size, so an edited image gets a new one. The cache is
//! bounded: when it grows past its byte budget the least recently used
//! files (by access stamp, refreshed on every hit) are removed.

pub mod office;

use crate::core::error::AppError;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Extensions the decoder handles (lowercase).
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"];
/// Larger sources are not decoded (memory and time).
pub const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
pub const MIN_PX: u32 = 32;
pub const MAX_PX: u32 = 512;
pub const DEFAULT_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

pub fn is_thumbnailable(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

pub struct ThumbnailCache {
    dir: PathBuf,
    budget: u64,
    /// Serialises eviction; generation itself runs in parallel.
    evicting: Mutex<()>,
}

impl ThumbnailCache {
    pub fn new(dir: impl Into<PathBuf>, budget: u64) -> Self {
        Self { dir: dir.into(), budget, evicting: Mutex::new(()) }
    }

    /// `<cache dir>/thumbnails`, falling back to the temp dir.
    pub fn open_default() -> Self {
        let dir = directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
            .map(|d| d.cache_dir().join("thumbnails"))
            .unwrap_or_else(|| ufop_core::session::user_temp_subdir("thumbnails"));
        Self::new(dir, DEFAULT_BUDGET_BYTES)
    }

    fn key(path: &Path, meta: &std::fs::Metadata, px: u32) -> String {
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        let mut h = Sha256::new();
        h.update(path.to_string_lossy().as_bytes());
        h.update(format!("|{}|{mtime}|{px}", meta.len()).as_bytes());
        hex::encode(&h.finalize()[..16])
    }

    /// The cached thumbnail for `path` at `px` (longest side), made if
    /// needed. `Ok(None)` for files that aren't decodable images.
    pub fn get(&self, path: &Path, px: u32) -> Result<Option<PathBuf>, AppError> {
        if !is_thumbnailable(path) {
            return Ok(None);
        }
        let meta = std::fs::metadata(path).map_err(|e| AppError::file_op(format!("Cannot read {}: {e}", path.display()), "Check the file still exists"))?;
        if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES || meta.len() == 0 {
            return Ok(None);
        }
        let px = px.clamp(MIN_PX, MAX_PX);
        let out = self.dir.join(format!("{}.png", Self::key(path, &meta, px)));
        if out.exists() {
            touch(&out);
            return Ok(Some(out));
        }
        let img = match decode(path) {
            Some(i) => i,
            None => return Ok(None),
        };
        let thumb = img.thumbnail(px, px);
        let mut bytes = Vec::new();
        thumb
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .map_err(|e| AppError::internal(format!("Thumbnail encode failed: {e}")))?;
        ufop_core::fsutil::write_private_atomic(&out, &bytes).map_err(|e| AppError::internal(format!("Thumbnail not cached: {e}")))?;
        self.evict_if_needed();
        Ok(Some(out))
    }

    /// Total bytes in the cache.
    pub fn size(&self) -> u64 {
        entries(&self.dir).iter().map(|(_, len, _)| len).sum()
    }

    /// Drop least recently used thumbnails until under budget (to 80%).
    pub fn evict_if_needed(&self) {
        let Ok(_g) = self.evicting.try_lock() else { return };
        let mut all = entries(&self.dir);
        let mut total: u64 = all.iter().map(|(_, len, _)| len).sum();
        if total <= self.budget {
            return;
        }
        all.sort_by_key(|(_, _, used)| *used);
        let target = self.budget / 10 * 8;
        for (p, len, _) in all {
            if total <= target {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    pub fn clear(&self) -> Result<u64, AppError> {
        let mut n = 0;
        for (p, _, _) in entries(&self.dir) {
            if std::fs::remove_file(p).is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }
}

/// Decode with size limits; `None` if the file isn't a readable image.
fn decode(path: &Path) -> Option<image::DynamicImage> {
    crate::convert_engine::decode(path).ok()
}

/// Mark a cached file as just used (its mtime is the LRU stamp).
fn touch(p: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

fn entries(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(path: &Path, w: u32, h: u32) {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
        img.save(path).unwrap();
    }

    #[test]
    fn makes_caches_and_rekeys_on_change() {
        let src = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(cache_dir.path(), DEFAULT_BUDGET_BYTES);
        let p = src.path().join("photo.PNG");
        png(&p, 800, 400);
        let t = cache.get(&p, 128).unwrap().unwrap();
        let thumb = image::open(&t).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (128, 64), "aspect kept, longest side = px");
        assert_eq!(cache.get(&p, 128).unwrap().unwrap(), t, "cached");
        assert_ne!(cache.get(&p, 256).unwrap().unwrap(), t, "size is part of the key");
        std::thread::sleep(std::time::Duration::from_millis(20));
        png(&p, 300, 600);
        let t2 = cache.get(&p, 128).unwrap().unwrap();
        assert_ne!(t2, t, "edited image gets a new thumbnail");
        assert_eq!(image::open(&t2).unwrap().height(), 128);
        assert_eq!(cache.get(&p, 10_000).unwrap().map(|t| image::open(t).unwrap().height()), Some(MAX_PX));
    }

    #[test]
    fn skips_non_images_and_broken_files() {
        let src = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(tempfile::tempdir().unwrap().path(), DEFAULT_BUDGET_BYTES);
        std::fs::write(src.path().join("a.txt"), "x").unwrap();
        std::fs::write(src.path().join("fake.jpg"), "not really").unwrap();
        assert!(cache.get(&src.path().join("a.txt"), 64).unwrap().is_none());
        assert!(cache.get(&src.path().join("fake.jpg"), 64).unwrap().is_none());
        assert!(cache.get(&src.path().join("missing.png"), 64).is_err());
    }

    #[test]
    fn evicts_least_recently_used_over_budget() {
        let src = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let big = ThumbnailCache::new(cache_dir.path(), DEFAULT_BUDGET_BYTES);
        let mut thumbs = Vec::new();
        for i in 0..4 {
            let p = src.path().join(format!("{i}.png"));
            png(&p, 200, 200);
            thumbs.push((p.clone(), big.get(&p, 128).unwrap().unwrap()));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Use the oldest again: it becomes the most recent.
        big.get(&thumbs[0].0, 128).unwrap();
        let one = std::fs::metadata(&thumbs[1].1).unwrap().len();
        // Budget for ~2 thumbnails: eviction keeps the recently used ones.
        let small = ThumbnailCache::new(cache_dir.path(), one * 2 + one / 2);
        small.evict_if_needed();
        assert!(thumbs[0].1.exists(), "recently used survives");
        assert!(!thumbs[1].1.exists(), "least recently used goes first");
        assert!(small.size() <= one * 2 + one / 2);
        assert!(small.clear().unwrap() >= 1);
        assert_eq!(small.size(), 0);
    }
}
