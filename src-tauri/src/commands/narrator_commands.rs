//! IPC commands for the Operation Narrator.
//!
//! Thin adapter layer over [`OperationNarrator`]. All real logic lives
//! in `narrator::summarize()` — these handlers just forward a
//! correlation id and return the resulting narrative.

use crate::core::error::AppError;
use crate::narrator::{OperationNarrative, OperationNarrator};
use tauri::State;

/// Build a plain-language narrative for one correlation-id trace.
///
/// Never returns a "not found" error: an unknown correlation id yields
/// a valid-but-empty narrative that explains the absence, so the UI
/// never has to handle a special missing-trace branch.
#[tauri::command]
pub async fn narrator_narrate_operation(
    correlation_id: String,
    narrator: State<'_, OperationNarrator>,
) -> Result<OperationNarrative, AppError> {
    narrator.narrate(&correlation_id).await
}
