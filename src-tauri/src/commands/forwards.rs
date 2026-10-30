//! Tauri adapter for `ezssh_core::api::forwards`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::forwards::ForwardStartTarget;
use ezssh_core::db;
use ezssh_core::error::Result;
use ezssh_core::ssh::forwards::{ForwardSpec, RuntimeForwardSummary};
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn forward_list(
    state: State<'_, AppState>,
    session_id: i64,
) -> Result<Vec<db::forwards::Forward>> {
    api::forwards::forward_list(&state, session_id).await
}

#[tauri::command]
pub async fn forward_create(
    state: State<'_, AppState>,
    session_id: i64,
    input: db::forwards::ForwardInput,
) -> Result<db::forwards::Forward> {
    api::forwards::forward_create(&state, session_id, input).await
}

#[tauri::command]
pub async fn forward_update(
    state: State<'_, AppState>,
    id: i64,
    input: db::forwards::ForwardInput,
) -> Result<db::forwards::Forward> {
    api::forwards::forward_update(&state, id, input).await
}

#[tauri::command]
pub async fn forward_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    api::forwards::forward_delete(&state, id).await
}

#[tauri::command]
pub async fn forward_runtime_list(
    state: State<'_, AppState>,
    connection_id: u64,
) -> Result<Vec<RuntimeForwardSummary>> {
    api::forwards::forward_runtime_list(&state, connection_id).await
}

#[tauri::command]
pub async fn forward_start(
    state: State<'_, AppState>,
    connection_id: u64,
    target: ForwardStartTarget,
) -> Result<RuntimeForwardSummary> {
    api::forwards::forward_start(&state, connection_id, target).await
}

#[tauri::command]
pub async fn forward_replace(
    state: State<'_, AppState>,
    connection_id: u64,
    runtime_id: u64,
    spec: ForwardSpec,
) -> Result<RuntimeForwardSummary> {
    api::forwards::forward_replace(&state, connection_id, runtime_id, spec).await
}

#[tauri::command]
pub async fn forward_stop(
    state: State<'_, AppState>,
    connection_id: u64,
    runtime_id: u64,
) -> Result<()> {
    api::forwards::forward_stop(&state, connection_id, runtime_id).await
}

#[tauri::command]
pub async fn forward_stop_all(state: State<'_, AppState>, connection_id: u64) -> Result<()> {
    api::forwards::forward_stop_all(&state, connection_id).await
}
