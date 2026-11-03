//! Chunked upload/download for WSL connections. Mirrors
//! `sftp/transfer.rs` byte-for-byte on the event side (same
//! `Event::Transfer` + `TransferProgress` payload) so the file
//! pane's progress strip doesn't need to know whether the
//! transfer is real SFTP or a Plan 9 copy.
//!
//! Why not `tokio::fs::copy`? It's faster but emits no progress and
//! has no per-chunk cancel point. For the SFTP-pane UX (visible
//! progress bar, error attribution to the failing chunk), we replicate
//! the SSH path's read-loop + emit pattern.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, Result};
use crate::events::Events;
use crate::sftp::transfer::{CHUNK, Ticker, check, emit_done, emit_end, open_local, remove_partial_local};

use super::handle::WslFsHandle;


pub async fn upload(
    events: &Events,
    handle: &WslFsHandle,
    transfer_id: u64,
    local_path: &Path,
    remote_path: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let opened = match check(cancel) {
        Ok(()) => open_local(local_path).await,
        Err(e) => Err(e),
    };
    let (mut local, total) = match opened {
        Ok(x) => x,
        Err(e) => {
            emit_end(events, transfer_id, 0, &e);
            return Err(e);
        }
    };
    let unc = handle.linux_to_unc(remote_path);

    let result: Result<()> = async {
        let mut remote = tokio::fs::File::create(&unc).await.map_err(|e| AppError::Sftp(format!("open remote: {e}")))?;
        let mut sent: u64 = 0;
        let mut buf = vec![0u8; CHUNK];
        let mut ticker = Ticker::new(events, transfer_id);
        loop {
            check(cancel)?;
            let n = local.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            remote.write_all(&buf[..n]).await.map_err(|e| AppError::Sftp(format!("write: {e}")))?;
            sent += n as u64;
            ticker.tick(sent, total);
        }
        remote.flush().await?;
        emit_done(events, transfer_id, sent, total);
        Ok(())
    }
    .await;

    if let Err(e) = &result {
        if matches!(e, AppError::TransferCancelled) {
            remove_partial_local(transfer_id, &unc).await;
        }
        emit_end(events, transfer_id, total, e);
    }
    result
}

pub async fn download(
    events: &Events,
    handle: &WslFsHandle,
    transfer_id: u64,
    remote_path: &str,
    local_path: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    let unc = handle.linux_to_unc(remote_path);
    let created = match check(cancel) {
        Ok(()) => tokio::fs::File::create(local_path).await.map_err(AppError::from),
        Err(e) => Err(e),
    };
    let mut local = match created {
        Ok(f) => f,
        Err(e) => {
            emit_end(events, transfer_id, 0, &e);
            return Err(e);
        }
    };

    let mut total_seen = 0u64;
    let total_out = &mut total_seen;
    let result: Result<()> = async {
        let meta = tokio::fs::metadata(&unc).await.map_err(|e| AppError::Sftp(format!("stat: {e}")))?;
        let total = meta.len();
        *total_out = total;
        let mut remote = tokio::fs::File::open(&unc).await.map_err(|e| AppError::Sftp(format!("open: {e}")))?;
        let mut received: u64 = 0;
        let mut buf = vec![0u8; CHUNK];
        let mut ticker = Ticker::new(events, transfer_id);
        loop {
            check(cancel)?;
            let n = remote.read(&mut buf).await.map_err(|e| AppError::Sftp(format!("read: {e}")))?;
            if n == 0 {
                break;
            }
            local.write_all(&buf[..n]).await?;
            received += n as u64;
            ticker.tick(received, total);
        }
        local.flush().await?;
        emit_done(events, transfer_id, received, total);
        Ok(())
    }
    .await;

    if let Err(e) = &result {
        if matches!(e, AppError::TransferCancelled) {
            drop(local);
            remove_partial_local(transfer_id, local_path).await;
        }
        emit_end(events, transfer_id, total_seen, e);
    }
    result
}
