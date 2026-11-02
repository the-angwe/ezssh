use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU32};
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::error::{AppError, Result};
use crate::events::Events;

use crate::local::LocalRegistry;
use crate::sftp::upload_stream::UploadStreamRegistry;
use crate::sftp::SftpRegistry;
use crate::ssh::ConnectionRegistry;
use crate::sync::SyncManager;
use crate::vault::VaultState;
use crate::xserver::XServerManager;

pub struct AppState {
    pub db: SqlitePool,
    pub vault: Arc<RwLock<VaultState>>,
    /// Consecutive failed `vault_unlock` attempts. Reset on a successful unlock.
    pub unlock_failures: AtomicU32,
    /// Unix timestamp (seconds) until which new unlock attempts are refused.
    /// Zero means no cooldown is active.
    pub unlock_locked_until_unix: AtomicI64,
    /// SSH connection registry. Wrapped in `Arc` so the per-connection driver
    /// task (`ssh::client::drive_channel`) can hold its own reference and clean
    /// up on EOF/Close/ExitStatus without needing access to `State<AppState>`.
    pub ssh: Arc<ConnectionRegistry>,
    /// SFTP session registry. Wrapped in `Arc` for the same reason as `ssh`:
    /// the SSH driver task must be able to drop SFTP state when the underlying
    /// transport dies, otherwise `SftpHandle`s leak past connection teardown.
    pub sftp: Arc<SftpRegistry>,
    /// In-flight streaming uploads. One entry per concurrent
    /// `sftp_upload_begin` call; chunks are pushed via
    /// `sftp_upload_chunk` until `sftp_upload_finish` or
    /// `sftp_upload_abort`. See `sftp::upload_stream`.
    pub upload_streams: Arc<UploadStreamRegistry>,
    /// Cancel flags of path-based transfers (`sftp_upload*`, `sftp_download*`)
    /// from ticket to end. See `api::sftp::sftp_transfer_cancel`.
    pub transfers: Arc<crate::sftp::transfer::TransferRegistry>,
    /// Local PTY connection registry — WSL distros and local shells. Uses
    /// its own id space (offset ≥ 2⁴⁸) so ids never collide with SSH.
    pub local: Arc<LocalRegistry>,
    /// VcXsrv lifecycle manager. Ref-counted per X11 display; the first
    /// SSH session with forward_x11 enabled starts it, the last one
    /// closing stops it.
    pub xserver: Arc<XServerManager>,
    /// Cloud sync manager. Call `.trigger()` after any user-data mutation
    /// to enqueue a debounced write to the configured destination.
    pub sync: Arc<SyncManager>,
    /// Where async happenings (connection closed, forward status, transfer
    /// progress) are reported. Supplied by the hosting UI.
    pub events: Events,
}

impl AppState {
    /// Open (creating if needed) the database at `db_path`, run migrations
    /// and set the vault to `Locked` or `Uninitialized` to match what is
    /// stored. The usual way to start the core.
    pub async fn open(db_path: &Path, events: Events) -> Result<Self> {
        let pool = crate::db::init_pool(db_path).await?;
        let initialized = crate::vault::is_initialized(&pool).await?;
        let state = Self::new(pool, events);
        *state.vault.write().await = if initialized {
            crate::vault::VaultState::Locked
        } else {
            crate::vault::VaultState::Uninitialized
        };
        tracing::info!(db = %db_path.display(), initialized, "core state opened");
        Ok(state)
    }

    pub fn new(db: SqlitePool, events: Events) -> Self {
        let vault = Arc::new(RwLock::new(VaultState::Locked));
        let sync = Arc::new(SyncManager::spawn(db.clone(), vault.clone()));
        Self {
            db,
            vault,
            unlock_failures: AtomicU32::new(0),
            unlock_locked_until_unix: AtomicI64::new(0),
            ssh: Arc::new(ConnectionRegistry::new()),
            sftp: Arc::new(SftpRegistry::new()),
            upload_streams: Arc::new(UploadStreamRegistry::new()),
            transfers: Arc::new(crate::sftp::transfer::TransferRegistry::new()),
            local: Arc::new(LocalRegistry::new()),
            xserver: Arc::new(XServerManager::new()),
            sync,
            events,
        }
    }

    /// Drop all per-connection state in the correct order: SFTP first (so any
    /// lingering `SftpHandle` is released before the transport goes away),
    /// then the SSH connection itself (which also signals the driver task to
    /// disconnect). Idempotent: safe to call from both the user-initiated
    /// `ssh_disconnect` command and the driver task's own exit branches.
    ///
    /// X11 display ref counts are NOT touched here — the driver task that
    /// owns the connection handles `xserver.release(display)` in its
    /// cleanup branch (so a remote-initiated EOF releases the ref the same
    /// way a user disconnect does, no matter which path runs first).
    pub async fn close_connection(&self, id: u64) {
        self.sftp.remove(id).await;
        self.ssh.close(id).await;
    }

    /// Chunks pushed into a connection's frontend output channel. The SSH
    /// and local id spaces are disjoint (local ids carry bit 48), so the
    /// first registry that knows the id answers. `None` once the
    /// connection is gone.
    pub async fn output_sent(&self, id: u64) -> Option<u64> {
        use std::sync::atomic::Ordering;
        if let Some(c) = self.ssh.get(id).await {
            return Some(c.output_sent.load(Ordering::Relaxed));
        }
        self.local.get(id).await.map(|c| c.output_sent.load(Ordering::Relaxed))
    }
}

/// The database path every ezTerm / ezSsh build uses:
/// `<local data dir>/ZerosAndOnes/ezTerm/ezterm.sqlite` (platform-specific
/// root from `directories`). Kept until the cut-over so the native build
/// opens the same database as the Tauri one.
pub fn default_db_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "ZerosAndOnes", "ezTerm").ok_or_else(|| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no platform data directory (home directory unknown)",
        ))
    })?;
    Ok(dirs.data_local_dir().join("ezterm.sqlite"))
}
