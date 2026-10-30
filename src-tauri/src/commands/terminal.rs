//! Commands that apply to any terminal connection, SSH or local PTY.

use ezssh_core::error::{AppError, Result};
use ezssh_core::state::AppState;
use tauri::State;

/// Chunks the backend has pushed into a connection's output channel. The
/// frontend's stall watchdog (lib/ssh.ts) compares this with what it has
/// received. `not_found` once the connection is gone, which tells the
/// watchdog to stop.
#[tauri::command]
pub async fn terminal_output_sent(state: State<'_, AppState>, connection_id: u64) -> Result<u64> {
    state.output_sent(connection_id).await.ok_or(AppError::NotFound)
}
