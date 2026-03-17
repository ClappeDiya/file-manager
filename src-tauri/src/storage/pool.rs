//! SQLite connection pool using Arc<Mutex<Connection>> with tokio::task::spawn_blocking.
//!
//! SQLite in WAL mode supports concurrent reads with a single writer.
//! We use a single connection wrapped in a Mutex for write operations
//! and run all operations on the blocking thread pool to keep async code
//! non-blocking.

use crate::core::error::AppError;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Database connection pool wrapping a single SQLite connection.
/// All operations are run via `tokio::task::spawn_blocking` to
/// keep the async runtime non-blocking.
#[derive(Clone)]
pub struct DbPool {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl DbPool {
    /// Open (or create) a database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::database(
                    format!("Cannot create database directory: {e}"),
                    "Check write permissions for the application data folder.",
                )
            })?;
        }

        let conn = Connection::open(&path)?;

        // Enable WAL mode for better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        // Busy timeout 5 seconds
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    /// Open an in-memory database (useful for testing).
    pub fn open_in_memory() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        })
    }

    /// Execute a closure with the database connection on the blocking pool.
    /// This is the primary way to perform database operations without
    /// blocking the async runtime.
    pub async fn execute<F, T>(&self, f: F) -> Result<T, AppError>
    where
        F: FnOnce(&Connection) -> Result<T, AppError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| {
                AppError::database(
                    format!("Database lock poisoned: {e}"),
                    "Restart the application.",
                )
            })?;
            f(&conn)
        })
        .await
        .map_err(|e| {
            AppError::database(
                format!("Database task panicked: {e}"),
                "Restart the application.",
            )
        })?
    }

    /// Get the database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the OS-appropriate application data directory for the database.
    pub fn default_db_path() -> Result<PathBuf, AppError> {
        let proj_dirs = directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
            .ok_or_else(|| {
                AppError::database(
                    "Cannot determine application data directory.",
                    "Check your OS user profile configuration.",
                )
            })?;
        let data_dir = proj_dirs.data_dir();
        Ok(data_dir.join("ufop.db"))
    }
}

impl std::fmt::Debug for DbPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbPool")
            .field("path", &self.path)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_open_in_memory() {
        let pool = DbPool::open_in_memory().unwrap();
        assert_eq!(pool.path().to_str().unwrap(), ":memory:");
    }

    #[tokio::test]
    async fn test_execute_query() {
        let pool = DbPool::open_in_memory().unwrap();
        let result = pool
            .execute(|conn| {
                let version: String =
                    conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
                Ok(version)
            })
            .await
            .unwrap();
        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn test_open_file_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = DbPool::open(&db_path).unwrap();
        assert!(db_path.exists());

        // Verify WAL mode
        let mode = pool
            .execute(|conn| {
                let mode: String =
                    conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
                Ok(mode)
            })
            .await
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[tokio::test]
    async fn test_concurrent_reads() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| {
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)", [])?;
            conn.execute("INSERT INTO test (val) VALUES ('hello')", [])?;
            Ok(())
        })
        .await
        .unwrap();

        // Launch multiple concurrent reads
        let mut handles = Vec::new();
        for _ in 0..10 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                pool.execute(|conn| {
                    let val: String =
                        conn.query_row("SELECT val FROM test WHERE id = 1", [], |row| row.get(0))?;
                    Ok(val)
                })
                .await
            }));
        }

        for handle in handles {
            let result = handle.await.unwrap().unwrap();
            assert_eq!(result, "hello");
        }
    }

    #[test]
    fn test_default_db_path() {
        let path = DbPool::default_db_path().unwrap();
        assert!(path.to_string_lossy().contains("ufop"));
    }
}
