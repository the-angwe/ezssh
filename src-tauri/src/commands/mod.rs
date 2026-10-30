//! `#[tauri::command]` wrappers. Most modules are generated one-to-one from
//! `ezssh_core::api`; `splash`, `terminal` and `updater` exist only in the
//! Tauri shell (splash window, webview stall watchdog, tauri-plugin-updater).

pub mod backup;
pub mod credentials;
pub mod folders;
pub mod forwards;
pub mod import;
pub mod local;
pub mod sessions;
pub mod settings;
pub mod sftp;
pub mod splash;
pub mod ssh;
pub mod sync;
pub mod terminal;
pub mod updater;
pub mod vault;
pub mod xserver;
