//! How the core talks back to whichever UI hosts it.
//!
//! Two paths, kept separate on purpose:
//!
//! - [`OutputSink`]: terminal bytes of one connection. Hot path, one sink
//!   per connection, handed to `ssh_connect` / `local_connect`. The Tauri
//!   shell forwards into an IPC channel; the native shell feeds the
//!   terminal model directly.
//! - [`EventSink`]: everything else that happens asynchronously (a
//!   connection closed, a forward changed state, transfer progress). One
//!   sink per [`AppState`](crate::state::AppState).
//!
//! Neither trait knows about event names or JSON. The Tauri adapter maps
//! [`Event`] onto its historic event names and payloads.

use std::sync::{Arc, mpsc};

use crate::sftp::transfer::TransferProgress;
use crate::ssh::forwards::RuntimeForwardSummary;

/// Receives terminal output for one connection.
pub trait OutputSink: Send + 'static {
    /// Deliver one chunk of output. Returns `false` once the consumer is
    /// gone; the caller keeps reading (the connection is torn down through
    /// its own close path) but stops counting delivered chunks.
    fn send(&self, bytes: &[u8]) -> bool;
}

/// Boxed output sink, as taken by the connect calls.
pub type Output = Box<dyn OutputSink>;

/// Why a terminal connection ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// The remote shell exited with this status.
    Exit(u32),
    /// The remote shell was killed by a signal. Holds the bare name
    /// (`"KILL"`, `"TERM"`, or a server-specific name), without `SIG`.
    Signal(String),
    /// EOF, channel close, local PTY exit or a user disconnect; no status.
    Ended,
}

/// Something the UI should hear about.
#[derive(Clone)]
pub enum Event {
    /// A terminal connection (SSH or local PTY) ended. Sent once per
    /// connection.
    Closed {
        connection_id: u64,
        reason: CloseReason,
    },
    /// Writing input to an SSH channel failed. The connection stays up
    /// until its own close arrives.
    WriteFailed { connection_id: u64, message: String },
    /// A port forward on `connection_id` changed state (started, stopped,
    /// conflicted, failed). Also sent for auto-start scan failures with
    /// `runtime_id = 0`.
    ForwardStatus {
        connection_id: u64,
        summary: RuntimeForwardSummary,
    },
    /// Progress of an upload or download (SFTP or WSL). The last event of
    /// a transfer has `done = true`.
    Transfer(TransferProgress),
    /// Bytes of an update installer received so far (`api::updater`).
    UpdateProgress { downloaded: u64, total: Option<u64> },
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Closed {
                connection_id,
                reason,
            } => f
                .debug_struct("Closed")
                .field("connection_id", connection_id)
                .field("reason", reason)
                .finish(),
            Event::WriteFailed {
                connection_id,
                message,
            } => f
                .debug_struct("WriteFailed")
                .field("connection_id", connection_id)
                .field("message", message)
                .finish(),
            Event::ForwardStatus {
                connection_id,
                summary,
            } => f
                .debug_struct("ForwardStatus")
                .field("connection_id", connection_id)
                .field("runtime_id", &summary.runtime_id)
                .field("status", &summary.status)
                .finish(),
            Event::Transfer(p) => f
                .debug_struct("Transfer")
                .field("transfer_id", &p.transfer_id)
                .field("bytes_sent", &p.bytes_sent)
                .field("total_bytes", &p.total_bytes)
                .field("done", &p.done)
                .field("error", &p.error)
                .finish(),
            Event::UpdateProgress { downloaded, total } => f
                .debug_struct("UpdateProgress")
                .field("downloaded", downloaded)
                .field("total", total)
                .finish(),
        }
    }
}

/// Receives [`Event`]s. Called from tokio tasks and plain threads, so it
/// must not block.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: Event);
}

/// Shared event sink, as held by `AppState`.
pub type Events = Arc<dyn EventSink>;

/// Drops every event. For tests and tools that do not care.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, event: Event) {
        tracing::trace!(?event, "event dropped (NullSink)");
    }
}

/// Queues events on a std channel and calls `wake` after each one, so a
/// UI can drain the queue on its next frame (`wake` = request a repaint).
pub struct ChannelSink {
    tx: mpsc::Sender<Event>,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl EventSink for ChannelSink {
    fn emit(&self, event: Event) {
        if let Err(e) = self.tx.send(event) {
            tracing::debug!(event = ?e.0, "event receiver gone; dropped");
            return;
        }
        (self.wake)();
    }
}

/// A [`ChannelSink`] and the receiving end of its queue.
pub fn channel(wake: impl Fn() + Send + Sync + 'static) -> (Events, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    (
        Arc::new(ChannelSink {
            tx,
            wake: Box::new(wake),
        }),
        rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn channel_sink_queues_and_wakes() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let (sink, rx) = channel(move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        sink.emit(Event::Closed {
            connection_id: 7,
            reason: CloseReason::Exit(3),
        });
        sink.emit(Event::WriteFailed {
            connection_id: 7,
            message: "broken pipe".into(),
        });
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        match rx.try_recv().unwrap() {
            Event::Closed {
                connection_id: 7,
                reason: CloseReason::Exit(3),
            } => {}
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            rx.try_recv().unwrap(),
            Event::WriteFailed {
                connection_id: 7,
                ..
            }
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn channel_sink_survives_dropped_receiver() {
        let (sink, rx) = channel(|| {});
        drop(rx);
        // Must not panic; the event is logged and dropped.
        sink.emit(Event::Closed {
            connection_id: 1,
            reason: CloseReason::Ended,
        });
    }
}
