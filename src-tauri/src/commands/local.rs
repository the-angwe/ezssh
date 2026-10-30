//! Tauri adapter for `ezssh_core::api::local`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::local::LocalConnectResult;
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn local_connect(
    state: State<'_, AppState>,
    session_id: i64,
    cols: u16,
    rows: u16,
    output: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<LocalConnectResult> {
    api::local::local_connect(
        &state,
        session_id,
        cols,
        rows,
        crate::output::ipc_output(output),
    )
    .await
}

#[tauri::command]
pub async fn local_write(
    state: State<'_, AppState>,
    connection_id: u64,
    bytes: Vec<u8>,
) -> Result<()> {
    api::local::local_write(&state, connection_id, bytes).await
}

#[tauri::command]
pub async fn local_resize(
    state: State<'_, AppState>,
    connection_id: u64,
    cols: u16,
    rows: u16,
) -> Result<()> {
    api::local::local_resize(&state, connection_id, cols, rows).await
}

#[tauri::command]
pub async fn local_disconnect(state: State<'_, AppState>, connection_id: u64) -> Result<()> {
    api::local::local_disconnect(&state, connection_id).await
}

#[tauri::command]
pub async fn wsl_list_distros(state: State<'_, AppState>) -> Result<Vec<String>> {
    api::local::wsl_list_distros(&state).await
}

#[tauri::command]
pub async fn wsl_autodetect_seed(state: State<'_, AppState>) -> Result<usize> {
    api::local::wsl_autodetect_seed(&state).await
}

#[tauri::command]
pub async fn local_shells_autodetect_seed(state: State<'_, AppState>) -> Result<usize> {
    api::local::local_shells_autodetect_seed(&state).await
}

#[tauri::command]
pub async fn list_local_shells(
    state: State<'_, AppState>,
) -> Result<Vec<ezssh_core::local::shells::DetectedShell>> {
    api::local::list_local_shells(&state).await
}

#[tauri::command]
pub fn platform() -> &'static str {
    api::local::platform()
}
