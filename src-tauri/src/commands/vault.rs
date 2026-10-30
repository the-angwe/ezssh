//! Tauri adapter for `ezssh_core::api::vault`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::vault::{
    ChangePasswordResult, GenerateRecoveryResult, RecoveryStatus, ResetResult,
};
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn vault_status(state: State<'_, AppState>) -> Result<&'static str> {
    api::vault::vault_status(&state).await
}

#[tauri::command]
pub async fn vault_init(state: State<'_, AppState>, password: String) -> Result<()> {
    api::vault::vault_init(&state, password).await
}

#[tauri::command]
pub async fn vault_unlock(state: State<'_, AppState>, password: String) -> Result<()> {
    api::vault::vault_unlock(&state, password).await
}

#[tauri::command]
pub async fn vault_lock(state: State<'_, AppState>) -> Result<()> {
    api::vault::vault_lock(&state).await
}

#[tauri::command]
pub async fn vault_verify_password(state: State<'_, AppState>, password: String) -> Result<bool> {
    api::vault::vault_verify_password(&state, password).await
}

#[tauri::command]
pub async fn vault_change_password(
    state: State<'_, AppState>,
    old_password: String,
    new_password: String,
) -> Result<ChangePasswordResult> {
    api::vault::vault_change_password(&state, old_password, new_password).await
}

#[tauri::command]
pub async fn vault_recovery_status(state: State<'_, AppState>) -> Result<RecoveryStatus> {
    api::vault::vault_recovery_status(&state).await
}

#[tauri::command]
pub async fn vault_generate_recovery_code(
    state: State<'_, AppState>,
    password: String,
) -> Result<GenerateRecoveryResult> {
    api::vault::vault_generate_recovery_code(&state, password).await
}

#[tauri::command]
pub async fn vault_unlock_with_recovery(state: State<'_, AppState>, code: String) -> Result<()> {
    api::vault::vault_unlock_with_recovery(&state, code).await
}

#[tauri::command]
pub async fn vault_reset(state: State<'_, AppState>, confirmation: String) -> Result<ResetResult> {
    api::vault::vault_reset(&state, confirmation).await
}
