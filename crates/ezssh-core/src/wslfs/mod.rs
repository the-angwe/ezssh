//! WSL filesystem adapter — exposes the same surface as the SSH SFTP
//! path so the `api::sftp::sftp_*` functions dispatch transparently for
//! both connection kinds. The file pane always works in Linux-style
//! paths; Windows UNC (`\\wsl.localhost\<distro>\…`)
//! translation is confined to `handle.rs`.
//!
//! Layout mirrors `sftp/` deliberately:
//!   - `handle`  : per-connection state (distro / user / cached $HOME)
//!   - `list`    : directory listings via `wsl.exe ls -la`
//!   - `ops`     : mkdir / rmdir / remove / rename / realpath
//!   - `transfer`: chunked upload / download with progress events

pub mod handle;
pub mod list;
pub mod ops;
pub mod transfer;

pub use handle::WslFsHandle;
