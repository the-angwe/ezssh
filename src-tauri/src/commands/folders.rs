//! Tauri adapter for `ezssh_core::api::folders`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::db::folders::Folder;
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn folder_list(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    api::folders::folder_list(&state).await
}

#[tauri::command]
pub async fn folder_create(
    state: State<'_, AppState>,
    parent_id: Option<i64>,
    name: String,
) -> Result<Folder> {
    api::folders::folder_create(&state, parent_id, name).await
}

#[tauri::command]
pub async fn folder_rename(state: State<'_, AppState>, id: i64, name: String) -> Result<()> {
    api::folders::folder_rename(&state, id, name).await
}

#[tauri::command]
pub async fn folder_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    api::folders::folder_delete(&state, id).await
}

#[tauri::command]
pub async fn folder_move(
    state: State<'_, AppState>,
    id: i64,
    parent_id: Option<i64>,
    sort: i64,
) -> Result<()> {
    api::folders::folder_move(&state, id, parent_id, sort).await
}

#[tauri::command]
pub async fn folder_reorder(
    state: State<'_, AppState>,
    parent_id: Option<i64>,
    ids: Vec<i64>,
) -> Result<()> {
    api::folders::folder_reorder(&state, parent_id, ids).await
}
