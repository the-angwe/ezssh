//! Tauri adapter for `ezssh_core::api::ssh`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::ssh::ConnectResult;
use ezssh_core::db::known_hosts::KnownHost;
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn ssh_connect(
    state: State<'_, AppState>,
    session_id: i64,
    cols: u16,
    rows: u16,
    trust_any: bool,
    disable_x11: Option<bool>,
    output: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<ConnectResult> {
    api::ssh::ssh_connect(
        &state,
        session_id,
        cols,
        rows,
        trust_any,
        disable_x11,
        crate::output::ipc_output(output),
    )
    .await
}

#[tauri::command]
pub async fn ssh_write(
    state: State<'_, AppState>,
    connection_id: u64,
    bytes: Vec<u8>,
) -> Result<()> {
    api::ssh::ssh_write(&state, connection_id, bytes).await
}

#[tauri::command]
pub async fn ssh_resize(
    state: State<'_, AppState>,
    connection_id: u64,
    cols: u16,
    rows: u16,
) -> Result<()> {
    api::ssh::ssh_resize(&state, connection_id, cols, rows).await
}

#[tauri::command]
pub async fn ssh_disconnect(state: State<'_, AppState>, connection_id: u64) -> Result<()> {
    api::ssh::ssh_disconnect(&state, connection_id).await
}

#[tauri::command]
pub async fn known_host_list(state: State<'_, AppState>) -> Result<Vec<KnownHost>> {
    api::ssh::known_host_list(&state).await
}

#[tauri::command]
pub async fn known_host_remove(state: State<'_, AppState>, host: String, port: i64) -> Result<()> {
    api::ssh::known_host_remove(&state, host, port).await
}
