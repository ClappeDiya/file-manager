//! Fleet agent: the client side of the admin console's agent protocol
//! (`/api/agent/*`). Shared by the desktop app, `ufop-server` (both drive
//! the backup manager through an [`AgentHost`]) and the `ufop` CLI.
//!
//! Enrollment is either a registration link token (`ufop_reg_…`) or an
//! interactive claim code an administrator enters in the console. Once
//! registered, the agent sends a heartbeat at the interval the console
//! sets, runs the commands it hands back (`rpc`, `run_backup`,
//! `apply_policies`, `rotate_token`), turns fleet policies into managed
//! jobs and forwards run reports, queueing them while offline.

#[doc(hidden)]
pub mod limits;
pub mod managed;
pub mod mock;
pub mod policy;
pub mod protocol;
pub mod runner;

pub use limits::{AgentLimits, SymlinkPolicy};
pub use managed::*;
pub use protocol::*;
pub use runner::*;

#[cfg(test)]
mod tests;
