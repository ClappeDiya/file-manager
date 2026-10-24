//! Everything the app does with the `rclone` command-line tool, in one place.
//!
//! - [`resolver`]: which rclone binary and which `rclone.conf` every caller
//!   uses (the connector, the backup store, mounts, the remote manager),
//!   plus `rclone version` parsing and the minimum-version check.
//! - [`install`]: the managed copy of rclone (Advanced): a pinned release
//!   downloaded from downloads.rclone.org, checked against the release's
//!   `SHA256SUMS` and unpacked into the app data folder.
//! - [`config`]: the in-app remote manager (`rclone config providers`,
//!   `create` / `update` / `delete` through rclone's non-interactive JSON
//!   state machine, including OAuth sign-in steps).
//! - [`crypt`]: crypt remotes (underlying remote, `rclone cryptdecode`).

pub mod config;
pub mod crypt;
pub mod install;
pub mod resolver;

pub use resolver::{Rclone, RcloneSettings, RcloneStatus};
