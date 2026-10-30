//! Tauri adapter for `ezssh_core::api::sessions`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::db::sessions::{EnvPair, Session, SessionInput};
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn session_list(state: State<'_, AppState>) -> Result<Vec<Session>> {
    api::sessions::session_list(&state).await
}

#[tauri::command]
pub async fn session_get(state: State<'_, AppState>, id: i64) -> Result<Session> {
    api::sessions::session_get(&state, id).await
}

#[tauri::command]
pub async fn session_env_get(state: State<'_, AppState>, id: i64) -> Result<Vec<EnvPair>> {
    api::sessions::session_env_get(&state, id).await
}

#[tauri::command]
pub async fn session_create(state: State<'_, AppState>, input: SessionInput) -> Result<Session> {
    api::sessions::session_create(&state, input).await
}

#[tauri::command]
pub async fn session_update(
    state: State<'_, AppState>,
    id: i64,
    input: SessionInput,
) -> Result<Session> {
    api::sessions::session_update(&state, id, input).await
}

#[tauri::command]
pub async fn session_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    api::sessions::session_delete(&state, id).await
}

#[tauri::command]
pub async fn session_duplicate(state: State<'_, AppState>, id: i64) -> Result<Session> {
    api::sessions::session_duplicate(&state, id).await
}

#[tauri::command]
pub async fn session_move(
    state: State<'_, AppState>,
    id: i64,
    folder_id: Option<i64>,
    sort: i64,
) -> Result<()> {
    api::sessions::session_move(&state, id, folder_id, sort).await
}

#[tauri::command]
pub async fn session_reorder(
    state: State<'_, AppState>,
    folder_id: Option<i64>,
    ids: Vec<i64>,
) -> Result<()> {
    api::sessions::session_reorder(&state, folder_id, ids).await
}
