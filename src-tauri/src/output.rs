//! Terminal output into a Tauri IPC channel.

use ezssh_core::events::{Output, OutputSink};
use tauri::ipc::{Channel, InvokeResponseBody};

/// Forwards a connection's output into the IPC channel the frontend created
/// before calling `ssh_connect` / `local_connect`. Payloads go as
/// `InvokeResponseBody::Raw` and land in JS as an `ArrayBuffer`. Chunks of
/// 1 KiB and up travel as raw bytes; Tauri still inlines smaller ones as a
/// JSON array, so the win is for bulk output, not keystroke echo.
struct IpcOutput(Channel<InvokeResponseBody>);

impl OutputSink for IpcOutput {
    fn send(&self, bytes: &[u8]) -> bool {
        match self.0.send(InvokeResponseBody::Raw(bytes.to_vec())) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!(len = bytes.len(), "ipc output send failed: {e}");
                false
            }
        }
    }
}

pub fn ipc_output(channel: Channel<InvokeResponseBody>) -> Output {
    Box::new(IpcOutput(channel))
}
