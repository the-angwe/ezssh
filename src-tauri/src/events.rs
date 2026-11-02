//! Maps core events onto the Tauri event names and payloads the frontend
//! has always listened to:
//!
//! | core event | Tauri event | payload |
//! |---|---|---|
//! | `Closed { Exit(n) }` | `ssh:close:{id}` | `n` |
//! | `Closed { Signal(s) }` | `ssh:close:{id}` | `"killed by SIG{s}"` |
//! | `Closed { Ended }` | `ssh:close:{id}` | `null` |
//! | `WriteFailed` | `ssh:error:{id}` | message string |
//! | `ForwardStatus` | `forwards:status:{id}` | `RuntimeForwardSummary` |
//! | `Transfer` | `sftp:transfer:{transfer_id}` | `TransferProgress` |
//!
//! Local PTY connections use the `ssh:close:` prefix too (shared id space).

use std::sync::OnceLock;

use ezssh_core::events::{CloseReason, Event, EventSink};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// The core's event sink for the Tauri shell. `AppState` is built before
/// the Tauri app exists, so the handle is attached in `setup`. Nothing can
/// emit before then (no connection or transfer exists yet); if something
/// does, it is logged and dropped.
#[derive(Default)]
pub struct TauriEvents {
    app: OnceLock<AppHandle>,
}

impl TauriEvents {
    pub fn attach(&self, app: AppHandle) {
        if self.app.set(app).is_err() {
            tracing::warn!("TauriEvents::attach called twice; keeping the first handle");
        }
    }
}

impl EventSink for TauriEvents {
    fn emit(&self, event: Event) {
        let Some(app) = self.app.get() else {
            tracing::warn!(?event, "event before the Tauri app was attached; dropped");
            return;
        };
        let (name, payload) = match wire(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(?event, "event payload did not serialise: {e}");
                return;
            }
        };
        if let Err(e) = app.emit(&name, payload) {
            tracing::warn!(?event, "tauri emit failed: {e}");
        }
    }
}

/// Event name and JSON payload for `event`, exactly as the frontend has
/// always received them (see the table at the top of this file).
fn wire(event: &Event) -> serde_json::Result<(String, Value)> {
    Ok(match event {
        Event::Closed {
            connection_id,
            reason,
        } => {
            let payload = match reason {
                CloseReason::Exit(code) => Value::from(*code),
                CloseReason::Signal(sig) => Value::from(format!("killed by SIG{sig}")),
                CloseReason::Ended => Value::Null,
            };
            (format!("ssh:close:{connection_id}"), payload)
        }
        Event::WriteFailed {
            connection_id,
            message,
        } => (
            format!("ssh:error:{connection_id}"),
            Value::from(message.as_str()),
        ),
        Event::ForwardStatus {
            connection_id,
            summary,
        } => (
            format!("forwards:status:{connection_id}"),
            serde_json::to_value(summary)?,
        ),
        Event::Transfer(progress) => (
            format!("sftp:transfer:{}", progress.transfer_id),
            serde_json::to_value(progress)?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezssh_core::sftp::transfer::TransferProgress;
    use ezssh_core::ssh::forwards::{
        ForwardKind, ForwardSpec, ForwardStatus, RuntimeForwardSummary,
    };
    use serde_json::json;

    fn closed(reason: CloseReason) -> (String, Value) {
        wire(&Event::Closed {
            connection_id: 42,
            reason,
        })
        .unwrap()
    }

    #[test]
    fn close_payloads_match_the_old_emits() {
        assert_eq!(
            closed(CloseReason::Exit(130)),
            ("ssh:close:42".into(), json!(130))
        );
        assert_eq!(
            closed(CloseReason::Signal("KILL".into())),
            ("ssh:close:42".into(), json!("killed by SIGKILL"))
        );
        assert_eq!(
            closed(CloseReason::Ended),
            ("ssh:close:42".into(), Value::Null)
        );
    }

    #[test]
    fn write_failure_is_ssh_error() {
        let w = wire(&Event::WriteFailed {
            connection_id: 9,
            message: "channel closed".into(),
        })
        .unwrap();
        assert_eq!(w, ("ssh:error:9".into(), json!("channel closed")));
    }

    #[test]
    fn forward_status_keeps_the_pane_shape() {
        // The shape the old connect-time auto-start failure built by hand.
        let w = wire(&Event::ForwardStatus {
            connection_id: 3,
            summary: RuntimeForwardSummary {
                runtime_id: 0,
                persistent_id: None,
                spec: ForwardSpec {
                    name: "auto-start".into(),
                    kind: ForwardKind::Local,
                    bind_addr: String::new(),
                    bind_port: 0,
                    dest_addr: String::new(),
                    dest_port: 0,
                },
                status: ForwardStatus::Error {
                    message: "auto-start db query: boom".into(),
                },
            },
        })
        .unwrap();
        assert_eq!(w.0, "forwards:status:3");
        assert_eq!(
            w.1,
            json!({
                "runtime_id": 0,
                "persistent_id": null,
                "spec": {
                    "name": "auto-start", "kind": "local",
                    "bind_addr": "", "bind_port": 0, "dest_addr": "", "dest_port": 0,
                },
                "status": { "status": "error", "message": "auto-start db query: boom" },
            })
        );
    }

    #[test]
    fn transfer_progress_goes_to_its_transfer_id() {
        let w = wire(&Event::Transfer(TransferProgress {
            transfer_id: 17,
            bytes_sent: 1024,
            total_bytes: 4096,
            done: false,
            error: None,
            cancelled: false,
        }))
        .unwrap();
        assert_eq!(w.0, "sftp:transfer:17");
        // `cancelled` is left out when false: the webview's payload is unchanged.
        assert_eq!(
            w.1,
            json!({"transfer_id": 17, "bytes_sent": 1024, "total_bytes": 4096, "done": false, "error": null})
        );
    }
}
