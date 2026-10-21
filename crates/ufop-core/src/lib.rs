//! UFOP core: the error type, sync data types and the sync/copy engine
//! (planner, Robocopy-compatible copier and parser, executor, rollback,
//! conflicts), plus the versioned backup engine (`backup` feature). Shared by the desktop app (`src-tauri`) and the `ufop` CLI,
//! with no GUI dependencies.

#[cfg(feature = "backup")]
pub mod backup;
pub mod error;
pub mod preload;
pub mod sync;
pub mod sync_types;
