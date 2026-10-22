//! File naming: `{token}` templates for names and paths, and the batch
//! rename pipeline built on them. One implementation for batch rename (app
//! and CLI), automation destinations and renames, and versioned names.

pub mod rename;
pub mod tokens;

pub use rename::{compat_warnings, plan as plan_rename, RenameError, RenamePlanEntry, RenameSpec};
pub use tokens::{render, render_path, TemplateError, TokenContext, UnknownTokens};
