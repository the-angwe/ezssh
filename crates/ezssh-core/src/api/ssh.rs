use serde::Serialize;

use crate::error::Result;
use crate::ssh::{self, ConnectRequest};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ConnectResult {
    pub connection_id: u64,
    pub fingerprint_sha256: String,
}

pub async fn ssh_connect(
    state: &AppState,
    session_id: i64,
    cols: u16,
    rows: u16,
    trust_any: bool,
    disable_x11: Option<bool>,
    output: crate::events::Output,
) -> Result<ConnectResult> {
    super::require_unlocked(state).await?;
    let deps = ssh::ConnectDeps::from_state(state);
    let out = ssh::connect(
        deps,
        ConnectRequest {
            session_id,
            cols,
            rows,
            trust_any,
            disable_x11: disable_x11.unwrap_or(false),
            output,
        },
    )
    .await?;
    Ok(ConnectResult {
        connection_id: out.connection_id,
        fingerprint_sha256: out.fingerprint_sha256,
    })
}

pub async fn ssh_write(
    state: &AppState,
    connection_id: u64,
    bytes: Vec<u8>,
) -> Result<()> {
    if !state.ssh.write(connection_id, bytes).await {
        tracing::debug!(connection_id, "ssh_write: connection is gone, input dropped");
    }
    Ok(())
}

pub async fn ssh_resize(
    state: &AppState,
    connection_id: u64,
    cols: u16,
    rows: u16,
) -> Result<()> {
    if !state.ssh.resize(connection_id, cols, rows).await {
        tracing::debug!(connection_id, "ssh_resize: connection is gone, resize dropped");
    }
    Ok(())
}

pub async fn ssh_disconnect(state: &AppState, connection_id: u64) -> Result<()> {
    // Route through `close_connection` so SFTP state is dropped alongside the
    // SSH session. Going through `state.ssh.close` directly would leak the
    // per-connection `SftpHandle` (audit I-1).
    state.close_connection(connection_id).await;
    Ok(())
}
