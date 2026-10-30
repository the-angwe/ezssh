//! Tauri adapter for `ezssh_core::api::xserver`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::error::Result;
use ezssh_core::state::AppState;
use ezssh_core::xserver::XServerStatus;

#[tauri::command]
pub async fn xserver_status(state: State<'_, AppState>) -> Result<XServerStatus> {
    api::xserver::xserver_status(&state).await
}

#[tauri::command]
pub async fn xserver_install(state: State<'_, AppState>) -> Result<String> {
    api::xserver::xserver_install(&state).await
}
