//! Help > Send feedback: a message and category, optionally with the
//! scrubbed diagnostics bundle, sent to the management console this machine
//! is registered with, or saved to a file when there is none.

use crate::core::error::AppError;
use crate::feedback::{self, Destination, Outcome, Request};
use crate::fleet_agent::FleetAgent;
use tauri::State;

/// Where feedback goes on this machine: the console it is registered with, or a file.
#[tauri::command]
pub async fn feedback_destination(agent: State<'_, FleetAgent>) -> Result<Destination, AppError> {
    Ok(feedback::destination(&agent))
}

/// Send the feedback to the console (or save it to `save_path` when there is
/// none). Diagnostics are included only when the request says so.
#[tauri::command]
pub async fn feedback_submit(request: Request, agent: State<'_, FleetAgent>) -> Result<Outcome, AppError> {
    feedback::submit(&agent, request, &crate::logging::log_dir()).await
}
