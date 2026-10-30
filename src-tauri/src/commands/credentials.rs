//! Tauri adapter for `ezssh_core::api::credentials`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::db::credentials::{CredentialDetail, CredentialMeta};
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn credential_list(state: State<'_, AppState>) -> Result<Vec<CredentialMeta>> {
    api::credentials::credential_list(&state).await
}

#[tauri::command]
pub async fn credential_create(
    state: State<'_, AppState>,
    kind: String,
    label: String,
    plaintext: String,
) -> Result<CredentialMeta> {
    api::credentials::credential_create(&state, kind, label, plaintext).await
}

#[tauri::command]
pub async fn credential_list_detailed(state: State<'_, AppState>) -> Result<Vec<CredentialDetail>> {
    api::credentials::credential_list_detailed(&state).await
}

#[tauri::command]
pub async fn credential_update(
    state: State<'_, AppState>,
    id: i64,
    label: Option<String>,
    plaintext: Option<String>,
) -> Result<()> {
    api::credentials::credential_update(&state, id, label, plaintext).await
}

#[tauri::command]
pub async fn credential_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    api::credentials::credential_delete(&state, id).await
}
