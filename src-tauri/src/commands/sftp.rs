//! Tauri adapter for `ezssh_core::api::sftp`. Each command forwards to the
//! core function of the same name; argument names are the frontend's
//! `invoke()` keys and must not change.

use tauri::State;

use ezssh_core::api;
use ezssh_core::api::sftp::{SftpEntry, TransferTicket};
use ezssh_core::error::Result;
use ezssh_core::state::AppState;

#[tauri::command]
pub async fn sftp_open(state: State<'_, AppState>, connection_id: u64) -> Result<()> {
    api::sftp::sftp_open(&state, connection_id).await
}

#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    connection_id: u64,
    path: String,
) -> Result<Vec<SftpEntry>> {
    api::sftp::sftp_list(&state, connection_id, path).await
}

#[tauri::command]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    connection_id: u64,
    path: String,
) -> Result<()> {
    api::sftp::sftp_mkdir(&state, connection_id, path).await
}

#[tauri::command]
pub async fn sftp_rmdir(
    state: State<'_, AppState>,
    connection_id: u64,
    path: String,
) -> Result<()> {
    api::sftp::sftp_rmdir(&state, connection_id, path).await
}

#[tauri::command]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    connection_id: u64,
    path: String,
) -> Result<()> {
    api::sftp::sftp_remove(&state, connection_id, path).await
}

#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    connection_id: u64,
    from: String,
    to: String,
) -> Result<()> {
    api::sftp::sftp_rename(&state, connection_id, from, to).await
}

#[tauri::command]
pub async fn sftp_chmod(
    state: State<'_, AppState>,
    connection_id: u64,
    path: String,
    mode: u32,
) -> Result<()> {
    api::sftp::sftp_chmod(&state, connection_id, path, mode).await
}

#[tauri::command]
pub async fn sftp_realpath(
    state: State<'_, AppState>,
    connection_id: u64,
    path: String,
) -> Result<String> {
    api::sftp::sftp_realpath(&state, connection_id, path).await
}

#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    connection_id: u64,
    local_path: String,
    remote_path: String,
) -> Result<TransferTicket> {
    api::sftp::sftp_upload(&state, connection_id, local_path, remote_path).await
}

#[tauri::command]
pub async fn sftp_upload_bytes(
    state: State<'_, AppState>,
    connection_id: u64,
    remote_path: String,
    bytes: Vec<u8>,
) -> Result<TransferTicket> {
    api::sftp::sftp_upload_bytes(&state, connection_id, remote_path, bytes).await
}

#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    connection_id: u64,
    remote_path: String,
    local_path: String,
) -> Result<TransferTicket> {
    api::sftp::sftp_download(&state, connection_id, remote_path, local_path).await
}

#[tauri::command]
pub async fn sftp_upload_begin(
    state: State<'_, AppState>,
    connection_id: u64,
    remote_path: String,
) -> Result<u64> {
    api::sftp::sftp_upload_begin(&state, connection_id, remote_path).await
}

#[tauri::command]
pub async fn sftp_upload_chunk(
    state: State<'_, AppState>,
    upload_id: u64,
    bytes: Vec<u8>,
) -> Result<()> {
    api::sftp::sftp_upload_chunk(&state, upload_id, bytes).await
}

#[tauri::command]
pub async fn sftp_upload_finish(state: State<'_, AppState>, upload_id: u64) -> Result<()> {
    api::sftp::sftp_upload_finish(&state, upload_id).await
}

#[tauri::command]
pub async fn sftp_upload_abort(state: State<'_, AppState>, upload_id: u64) -> Result<()> {
    api::sftp::sftp_upload_abort(&state, upload_id).await
}
