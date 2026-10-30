//! ezSsh core library.
//!
//! Everything the app does that is not drawing: SSH and SFTP over russh,
//! local and WSL shells, port forwards, the credential vault, the SQLite
//! store, cloud sync, backups, MobaXterm import and the VcXsrv manager.
//! UI shells (the Tauri adapter in `src-tauri`, the native egui app) hold
//! an [`AppState`], call [`api`] functions and receive output through the
//! sinks in [`events`].

pub mod api;
pub mod backup;
pub mod db;
pub mod error;
pub mod events;
pub mod import;
pub mod local;
pub mod log_redacted;
pub mod sftp;
pub mod ssh;
pub mod state;
pub mod sync;
pub mod vault;
pub mod wslfs;
pub mod xserver;

pub use error::{AppError, Result};
pub use events::{CloseReason, Event, EventSink, Events, Output, OutputSink};
pub use state::AppState;
