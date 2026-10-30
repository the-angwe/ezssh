//! Tauri adapter for `ezssh_core::api::backup`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::backup::{BackupPreview, BackupSummary, RestoreSummary, SelectionSpec};
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn backup_create(
    state: State<'_, AppState>,
    path: String,
    master_password: String,
    passphrase: String,
) -> Result<BackupSummary> {
    api::backup::backup_create(&state, path, master_password, passphrase).await
}

#[tauri::command]
pub async fn backup_preview(
    state: State<'_, AppState>,
    path: String,
    passphrase: String,
) -> Result<BackupPreview> {
    api::backup::backup_preview(&state, path, passphrase).await
}

#[tauri::command]
pub async fn backup_restore(
    state: State<'_, AppState>,
    path: String,
    passphrase: String,
    selection: SelectionSpec,
) -> Result<RestoreSummary> {
    api::backup::backup_restore(&state, path, passphrase, selection).await
}
