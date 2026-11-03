use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::session::SftpHandle;
use crate::error::{AppError, Result};
use crate::events::{Event, Events};

/// Chunk size for streaming transfers. 32 KiB is a compromise: large enough to
/// amortise syscall / SFTP packet overhead, small enough to let the UI see
/// progress events at a reasonable cadence on slow links.
pub(crate) const CHUNK: usize = 32 * 1024;

/// Cap on mid-transfer `TransferProgress` emits (10/sec). At 32 KiB chunks a
/// fast link issues >1000 events/sec without this, which floods the UI
/// event queue and starves the UI (audit I-3). The FINAL `done: true` event is
/// always emitted regardless of this throttle.
const PROGRESS_THROTTLE: Duration = Duration::from_millis(100);

/// Progress of one transfer, delivered as `Event::Transfer`. A transfer emits throttled
/// mid-transfer events plus a final event with `done = true`: plain for
/// success, with `error = Some(..)` if it failed, with `cancelled = true`
/// (and no error) if it was cancelled.
#[derive(Serialize, Clone, Debug, Default)]
pub struct TransferProgress {
    pub transfer_id: u64,
    pub bytes_sent: u64,
    pub total_bytes: u64,
    pub done: bool,
    pub error: Option<String>,
    pub cancelled: bool,
}

// ---------------------------------------------------------------- cancel

/// Cancel flag of one transfer, checked before every chunk.
pub type CancelFlag = Arc<AtomicBool>;

/// Transfers that can still be cancelled: registered when the ticket is
/// handed out (so a queued transfer can be cancelled before it starts),
/// removed when it ends.
#[derive(Default)]
pub struct TransferRegistry {
    inner: std::sync::Mutex<HashMap<u64, CancelFlag>>,
}

impl TransferRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, transfer_id: u64) -> CancelFlag {
        let flag = CancelFlag::default();
        self.inner.lock().expect("transfer registry poisoned").insert(transfer_id, flag.clone());
        flag
    }

    /// Ask `transfer_id` to stop. False when it already ended (or never
    /// existed).
    pub fn cancel(&self, transfer_id: u64) -> bool {
        match self.inner.lock().expect("transfer registry poisoned").get(&transfer_id) {
            Some(f) => {
                f.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn finish(&self, transfer_id: u64) {
        self.inner.lock().expect("transfer registry poisoned").remove(&transfer_id);
    }
}

/// `Err(TransferCancelled)` once the flag is set.
pub(crate) fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) { Err(AppError::TransferCancelled) } else { Ok(()) }
}

// ---------------------------------------------------------------- events

/// A throttled mid-transfer event.
pub(crate) struct Ticker<'a> {
    events: &'a Events,
    transfer_id: u64,
    last: Instant,
}

impl<'a> Ticker<'a> {
    pub(crate) fn new(events: &'a Events, transfer_id: u64) -> Self {
        Self { events, transfer_id, last: Instant::now() }
    }

    pub(crate) fn tick(&mut self, sent: u64, total: u64) {
        if self.last.elapsed() >= PROGRESS_THROTTLE {
            self.events.emit(Event::Transfer(TransferProgress {
                transfer_id: self.transfer_id,
                bytes_sent: sent,
                total_bytes: total,
                ..Default::default()
            }));
            self.last = Instant::now();
        }
    }
}

/// The final event of a transfer that went through.
pub(crate) fn emit_done(events: &Events, transfer_id: u64, sent: u64, total: u64) {
    events.emit(Event::Transfer(TransferProgress { transfer_id, bytes_sent: sent, total_bytes: total, done: true, ..Default::default() }));
}

/// The final event of a transfer that failed or was cancelled, at any
/// point, also before it started (local file missing, target not
/// creatable). Without it a UI waiting for `done` would show the transfer
/// forever.
pub(crate) fn emit_end(events: &Events, transfer_id: u64, total: u64, err: &AppError) {
    let cancelled = matches!(err, AppError::TransferCancelled);
    if cancelled {
        tracing::debug!(transfer_id, "transfer cancelled");
    } else {
        tracing::warn!(transfer_id, "transfer failed: {err}");
    }
    events.emit(Event::Transfer(TransferProgress {
        transfer_id,
        bytes_sent: 0,
        total_bytes: total,
        done: true,
        error: (!cancelled).then(|| err.to_string()),
        cancelled,
    }));
}

/// Open a local file for upload and read its size.
pub(crate) async fn open_local(path: &Path) -> Result<(tokio::fs::File, u64)> {
    let f = tokio::fs::File::open(path).await?;
    let total = f.metadata().await?.len();
    Ok((f, total))
}

/// Remove what a cancelled transfer left behind on the local side.
pub(crate) async fn remove_partial_local(transfer_id: u64, path: &Path) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        tracing::warn!(transfer_id, path = %path.display(), "cancelled transfer: removing the partial file failed: {e}");
    }
}

// ---------------------------------------------------------------- SFTP

/// Stream a local file to a remote path, emitting `TransferProgress` events.
/// A cancel removes the partial remote file.
///
/// Holds the SFTP session lock for the full duration of the transfer — SFTP
/// operations on a single session are not safely interleaved, and the common
/// case is one transfer at a time. Concurrent uploads would require per-file
/// sessions.
pub async fn upload(
    events: &Events,
    handle: &SftpHandle,
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

    let result: Result<()> = handle
        .with_session(async move |s: &russh_sftp::client::SftpSession| -> Result<()> {
            // It may have waited for the session lock behind other transfers.
            check(cancel)?;
            let mut w = s.create(remote_path).await.map_err(|e| AppError::Sftp(format!("open remote: {e}")))?;
            let mut sent: u64 = 0;
            let mut buf = vec![0u8; CHUNK];
            let mut ticker = Ticker::new(events, transfer_id);
            let copied: Result<()> = async {
                loop {
                    check(cancel)?;
                    let n = local.read(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    w.write_all(&buf[..n]).await.map_err(|e| AppError::Sftp(format!("write: {e}")))?;
                    sent += n as u64;
                    ticker.tick(sent, total);
                }
                w.shutdown().await.map_err(|e| AppError::Sftp(format!("close: {e}")))
            }
            .await;
            match copied {
                Ok(()) => {
                    emit_done(events, transfer_id, sent, total);
                    Ok(())
                }
                Err(e) => {
                    if matches!(e, AppError::TransferCancelled) {
                        drop(w);
                        if let Err(re) = s.remove_file(remote_path).await {
                            tracing::warn!(transfer_id, remote = remote_path, "cancelled upload: removing the partial file failed: {re}");
                        }
                    }
                    Err(e)
                }
            }
        })
        .await;

    if let Err(e) = &result {
        emit_end(events, transfer_id, total, e);
    }
    result
}

/// Stream a remote file to a local path, emitting `TransferProgress` events.
/// A cancel removes the partial local file.
pub async fn download(
    events: &Events,
    handle: &SftpHandle,
    transfer_id: u64,
    remote_path: &str,
    local_path: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
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

    // The size is known once `stat` answered; a failure after that point
    // should still report it (uploads do).
    let total_seen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total_out = total_seen.clone();
    let result: Result<()> = handle
        .with_session(async move |s: &russh_sftp::client::SftpSession| -> Result<()> {
            check(cancel)?;
            let meta = s.metadata(remote_path).await.map_err(|e| AppError::Sftp(format!("stat: {e}")))?;
            let total = meta.size.unwrap_or(0);
            total_out.store(total, std::sync::atomic::Ordering::Relaxed);
            let mut r = s.open(remote_path).await.map_err(|e| AppError::Sftp(format!("open: {e}")))?;
            let mut received: u64 = 0;
            let mut buf = vec![0u8; CHUNK];
            let mut ticker = Ticker::new(events, transfer_id);
            loop {
                check(cancel)?;
                let n = r.read(&mut buf).await.map_err(|e| AppError::Sftp(format!("read: {e}")))?;
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
        })
        .await;

    if let Err(e) = &result {
        if matches!(e, AppError::TransferCancelled) {
            remove_partial_local(transfer_id, local_path).await;
        }
        emit_end(events, transfer_id, total_seen.load(std::sync::atomic::Ordering::Relaxed), e);
    }
    result
}
