//! Filesystem watcher for watch-based sync triggers.
//!
//! Uses the `notify` crate to watch source directories and trigger
//! sync runs when changes are detected. Debounces events to batch
//! changes within a configurable window (default 2s).

use crate::core::error::AppError;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A handle to a running filesystem watcher.
pub struct WatchHandle {
    pub pair_id: Uuid,
    pub source_path: PathBuf,
    _watcher: RecommendedWatcher,
}

/// Callback type for when changes are detected.
pub type OnChangeCallback = Box<dyn Fn(Uuid) + Send + Sync>;

/// Manages filesystem watchers for multiple sync pairs.
pub struct WatcherManager {
    watches: Arc<Mutex<HashMap<Uuid, WatchState>>>,
    debounce_ms: u64,
}

struct WatchState {
    #[allow(dead_code)]
    source_path: PathBuf,
    _watcher: RecommendedWatcher,
    #[allow(dead_code)]
    last_event: Arc<Mutex<Option<Instant>>>,
}

impl WatcherManager {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            watches: Arc::new(Mutex::new(HashMap::new())),
            debounce_ms,
        }
    }

    /// Start watching a directory for a sync pair.
    /// The callback is called (debounced) when filesystem changes are detected.
    pub fn start_watching(
        &self,
        pair_id: Uuid,
        source_path: PathBuf,
        on_change: OnChangeCallback,
    ) -> Result<(), AppError> {
        let debounce_ms = self.debounce_ms;
        let last_event: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let last_event_clone = last_event.clone();
        let pair_id_clone = pair_id;

        // Create a channel for events
        let (tx, rx) = mpsc::channel::<Event>();

        // Create the watcher
        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    let _ = tx.send(event);
                }
            },
            Config::default(),
        )
        .map_err(|e| AppError::Sync {
            message: format!("Cannot create filesystem watcher: {}", e),
            advice: "Check OS filesystem notification support.".to_string(),
        })?;

        // Start watching the directory
        watcher
            .watch(&source_path, RecursiveMode::Recursive)
            .map_err(|e| AppError::Sync {
                message: format!("Cannot watch directory {}: {}", source_path.display(), e),
                advice: "Check that the directory exists and you have read permission.".to_string(),
            })?;

        // Spawn a debounce thread
        let debounce_last = last_event_clone.clone();
        std::thread::spawn(move || {
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(_event) => {
                        let mut last = debounce_last.lock().unwrap();
                        *last = Some(Instant::now());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Check if we should fire the callback
                        let should_fire = {
                            let last = debounce_last.lock().unwrap();
                            if let Some(t) = *last {
                                t.elapsed() >= Duration::from_millis(debounce_ms)
                            } else {
                                false
                            }
                        };

                        if should_fire {
                            // Clear the last event
                            {
                                let mut last = debounce_last.lock().unwrap();
                                *last = None;
                            }
                            on_change(pair_id_clone);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        // Watcher was dropped
                        break;
                    }
                }
            }
        });

        // Store the watcher
        let mut watches = self.watches.lock().map_err(|_| AppError::Sync {
            message: "Watcher lock poisoned.".to_string(),
            advice: "Restart the application.".to_string(),
        })?;

        watches.insert(
            pair_id,
            WatchState {
                source_path,
                _watcher: watcher,
                last_event,
            },
        );

        tracing::info!("Started filesystem watcher for sync pair {}", pair_id);
        Ok(())
    }

    /// Stop watching a directory for a sync pair.
    pub fn stop_watching(&self, pair_id: Uuid) -> Result<(), AppError> {
        let mut watches = self.watches.lock().map_err(|_| AppError::Sync {
            message: "Watcher lock poisoned.".to_string(),
            advice: "Restart the application.".to_string(),
        })?;

        if watches.remove(&pair_id).is_some() {
            tracing::info!("Stopped filesystem watcher for sync pair {}", pair_id);
        }

        Ok(())
    }

    /// Check if a pair is being watched.
    pub fn is_watching(&self, pair_id: Uuid) -> bool {
        self.watches
            .lock()
            .map(|w| w.contains_key(&pair_id))
            .unwrap_or(false)
    }

    /// Get all active watch pair IDs.
    pub fn active_watches(&self) -> Vec<Uuid> {
        self.watches
            .lock()
            .map(|w| w.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Stop all watchers.
    pub fn stop_all(&self) {
        if let Ok(mut watches) = self.watches.lock() {
            let count = watches.len();
            watches.clear();
            tracing::info!("Stopped all {} filesystem watchers", count);
        }
    }
}

impl Default for WatcherManager {
    fn default() -> Self {
        // Default debounce of 2000ms for batch changes
        Self::new(2000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tempfile::TempDir;

    #[test]
    fn test_watcher_manager_creation() {
        let mgr = WatcherManager::new(1000);
        assert!(mgr.active_watches().is_empty());
    }

    #[test]
    fn test_start_and_stop_watching() {
        let mgr = WatcherManager::new(500);
        let dir = TempDir::new().unwrap();
        let pair_id = Uuid::new_v4();

        let callback = Box::new(move |_: Uuid| {});
        mgr.start_watching(pair_id, dir.path().to_path_buf(), callback)
            .unwrap();

        assert!(mgr.is_watching(pair_id));

        mgr.stop_watching(pair_id).unwrap();
        assert!(!mgr.is_watching(pair_id));
    }

    #[test]
    fn test_stop_all() {
        let mgr = WatcherManager::new(500);

        for _ in 0..3 {
            let dir = TempDir::new().unwrap();
            let callback = Box::new(move |_: Uuid| {});
            // Keep TempDir alive by converting to persistent path
            let dir_path = dir.keep();
            mgr.start_watching(Uuid::new_v4(), dir_path, callback)
                .unwrap();
        }

        assert_eq!(mgr.active_watches().len(), 3);
        mgr.stop_all();
        assert!(mgr.active_watches().is_empty());
    }

    #[test]
    fn test_watcher_detects_changes_within_5s() {
        let mgr = WatcherManager::new(500); // 500ms debounce for testing
        let dir = TempDir::new().unwrap();
        let pair_id = Uuid::new_v4();

        let triggered = Arc::new(AtomicU32::new(0));
        let triggered_clone = triggered.clone();

        let callback = Box::new(move |_: Uuid| {
            triggered_clone.fetch_add(1, Ordering::SeqCst);
        });

        mgr.start_watching(pair_id, dir.path().to_path_buf(), callback)
            .unwrap();

        // Create a file to trigger the watcher
        std::fs::write(dir.path().join("trigger.txt"), "hello").unwrap();

        // Wait for debounce + processing (well within 5s)
        std::thread::sleep(Duration::from_millis(2000));

        let count = triggered.load(Ordering::SeqCst);
        assert!(
            count >= 1,
            "Expected watcher to trigger at least once, got {}",
            count
        );

        mgr.stop_watching(pair_id).unwrap();
    }

    #[test]
    fn test_watch_nonexistent_dir_fails() {
        let mgr = WatcherManager::new(500);
        let result = mgr.start_watching(
            Uuid::new_v4(),
            PathBuf::from("/nonexistent/path/to/watch"),
            Box::new(|_| {}),
        );
        assert!(result.is_err());
    }
}
