//! The application error type lives in the shared `ufop-core` crate (so the
//! CLI and the desktop app report errors the same way); re-exported here so
//! `crate::core::error::AppError` keeps working everywhere.

pub use ufop_core::error::*;
