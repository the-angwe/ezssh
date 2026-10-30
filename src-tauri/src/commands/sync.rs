//! Tauri adapter for `ezssh_core::api::sync`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::sync::S3ConfigInput;
use ezssh_core::error::Result;
use ezssh_core::state::AppState;
use ezssh_core::sync::SyncStatus;

#[tauri::command]
pub async fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus> {
    api::sync::sync_status(&state).await
}

#[tauri::command]
pub async fn sync_configure_local(
    state: State<'_, AppState>,
    path: String,
    passphrase: String,
) -> Result<()> {
    api::sync::sync_configure_local(&state, path, passphrase).await
}

#[tauri::command]
pub async fn sync_disable(state: State<'_, AppState>) -> Result<()> {
    api::sync::sync_disable(&state).await
}

#[tauri::command]
pub async fn sync_configure_s3(state: State<'_, AppState>, cfg: S3ConfigInput) -> Result<()> {
    api::sync::sync_configure_s3(&state, cfg).await
}

#[tauri::command]
pub async fn sync_pull_to_temp(state: State<'_, AppState>) -> Result<String> {
    api::sync::sync_pull_to_temp(&state).await
}

#[tauri::command]
pub async fn sync_push_now(state: State<'_, AppState>) -> Result<()> {
    api::sync::sync_push_now(&state).await
}
