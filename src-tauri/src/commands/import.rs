//! Tauri adapter for `ezssh_core::api::import`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::import::{MobaImportPreview, MobaImportResult};
use ezssh_core::error::Result;
use ezssh_core::import::mobaxterm::ParsedMobaSession;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn mobaxterm_preview(
    state: State<'_, AppState>,
    path: String,
) -> Result<MobaImportPreview> {
    api::import::mobaxterm_preview(&state, path).await
}

#[tauri::command]
pub async fn mobaxterm_commit(
    state: State<'_, AppState>,
    sessions: Vec<ParsedMobaSession>,
    duplicate_strategy: String,
) -> Result<MobaImportResult> {
    api::import::mobaxterm_commit(&state, sessions, duplicate_strategy).await
}
