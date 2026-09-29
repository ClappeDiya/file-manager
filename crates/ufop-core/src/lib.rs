//! UFOP core: the error type, sync data types and the sync/copy engine
//! (planner, Robocopy-compatible copier and parser, executor, rollback,
//! conflicts). Shared by the desktop app (`src-tauri`) and the `ufop` CLI,
//! with no GUI dependencies.

pub mod error;
pub mod sync;
pub mod sync_types;
