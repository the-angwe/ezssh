//! ezSsh core library.
//!
//! Everything the app does that is not drawing: SSH and SFTP over russh,
//! local and WSL shells, port forwards, the credential vault, the SQLite
//! store, cloud sync, backups, MobaXterm import, the VcXsrv manager and the
//! in-app updater.
//! The app (`ezssh-app`) holds an [`AppState`], call [`api`] functions and receive output through the
//! sinks in [`events`].

pub mod api;
pub mod backup;
pub mod db;
pub mod error;
pub mod events;
pub mod import;
pub mod local;
pub mod log_redacted;
pub mod secret;
pub mod sftp;
pub mod ssh;
pub mod state;
pub mod sync;
pub mod updater;
pub mod vault;
pub mod wslfs;
pub mod xserver;

pub use error::{AppError, Result};
pub use events::{CloseReason, Event, EventSink, Events, Output, OutputSink};
pub use secret::Secret;
pub use state::AppState;
