//! Tauri adapter for `ezssh_core::api::settings`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>, key: String) -> Result<Option<String>> {
    api::settings::settings_get(&state, key).await
}

#[tauri::command]
pub async fn settings_list(state: State<'_, AppState>) -> Result<Vec<(String, String)>> {
    api::settings::settings_list(&state).await
}

#[tauri::command]
pub async fn settings_set(state: State<'_, AppState>, key: String, value: String) -> Result<()> {
    api::settings::settings_set(&state, key, value).await
}
