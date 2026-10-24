//! UFOP core: the error type, sync data types and the sync/copy engine
//! (planner, Robocopy-compatible copier and parser, executor, rollback,
//! conflicts), plus the versioned backup engine (`backup` feature). Shared by the desktop app (`src-tauri`) and the `ufop` CLI,
//! with no GUI dependencies.

#[cfg(feature = "backup")]
pub mod backup;
#[cfg(feature = "agent")]
pub mod agent;
#[cfg(feature = "doctext")]
pub mod doctext;
pub mod calendar;
#[cfg(feature = "cli-gen")]
pub mod cli_gen;
pub mod confine;
pub mod error;
pub mod fsutil;
pub mod gateway;
pub mod gpg;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod naming;
pub mod netacl;
pub mod policy;
pub mod preload;
pub mod retry;
pub mod run_notify;
pub mod session;
pub mod stability;
pub mod webdav;
pub mod sync;
pub mod sync_types;
