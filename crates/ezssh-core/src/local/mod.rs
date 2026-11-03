//! Local PTY backend — WSL distros and local shells (cmd / pwsh / powershell).
//!
//! Uses `portable-pty`'s ConPTY-backed implementation on Windows. Data flow
//! mirrors the SSH path: the reader thread pushes PTY bytes into the UI's
//! output sink (created before `local_connect` runs, so ConPTY's prompt —
//! written microseconds after spawn — can never outrun a consumer) and
//! emits `Event::Closed` on EOF. `local_write` / `local_resize` /
//! `local_disconnect` commands route through the per-connection mpsc to a
//! writer thread that owns the `MasterPty`.
//!
//! The writer thread owns the child: on `LocalInput::Close` (from the UI's
//! disconnect, or from the reader thread once it saw EOF) it kills a
//! still-running child, waits on it, and only then drops the writer and
//! the master. Killing first matters on Unix, where portable-pty's writer
//! sends "\n" + VEOF on drop.

pub mod registry;
pub mod shells;
mod spawn;

pub use registry::{Connection, LocalInput, LocalRegistry, WslMeta};
pub use spawn::spawn_from_session;
