use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::sftp::{FileBrowser, SftpHandle};
use crate::state::AppState;
use crate::wslfs::{self, WslFsHandle};

/// Monotonic id for transfers. The frontend correlates progress events to a
/// ticket through `Event::Transfer` (`sftp:transfer:{id}` in the Tauri shell).
static TRANSFER_ID: AtomicU64 = AtomicU64::new(0);

fn next_transfer_id() -> u64 {
    TRANSFER_ID.fetch_add(1, Ordering::Relaxed) + 1
}

/// Handle returned to the frontend so it can subscribe to progress before
/// the transfer task has started emitting.
#[derive(Serialize)]
pub struct TransferTicket {
    pub transfer_id: u64,
}

/// One entry returned by `sftp_list`. Mirrors the fields the UI needs to
/// render a directory row: name + full path for navigation, type flags,
/// size, mtime (unix seconds), and the POSIX mode bits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SftpEntry {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_unix: i64,
    pub mode: u32,
}

/// Open the file browser for `connection_id`. Routes by connection kind:
/// SSH connections get a real SFTP subsystem channel; WSL connections get a
/// Plan-9-backed adapter that translates Linux paths to `\\wsl.localhost`
/// UNC and shells out for POSIX-aware ops. Plain local shells (cmd, pwsh)
/// have no file browser — calling this for them returns `NotFound`.
pub async fn sftp_open(state: &AppState, connection_id: u64) -> Result<()> {
    super::require_unlocked(state).await?;

    // SSH path first — established connections live in state.ssh.
    if let Some(conn) = state.ssh.get(connection_id).await {
        let channel = conn
            .ssh_handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::Ssh(e.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::Ssh(e.to_string()))?;
        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::Sftp(format!("init: {e}")))?;
        state
            .sftp
            .insert(connection_id, FileBrowser::Sftp(SftpHandle::new(sftp)))
            .await;
        return Ok(());
    }

    // WSL path — look up the local PTY connection and confirm it was
    // spawned with WSL metadata. Plain local shells (cmd/pwsh) land
    // here without `wsl_meta` and are rejected.
    if let Some(local) = state.local.get(connection_id).await {
        let meta = local
            .wsl_meta
            .as_ref()
            .ok_or_else(|| AppError::Validation(
                "file browser is only available for SSH and WSL sessions".into(),
            ))?;
        let handle = WslFsHandle::new(meta.distro.clone(), meta.user.clone());
        state
            .sftp
            .insert(connection_id, FileBrowser::Wsl(handle))
            .await;
        return Ok(());
    }

    Err(AppError::NotFound)
}

/// Fetch a directory listing. Sorted directories-first, name-ascending,
/// with `.`/`..` filtered.
pub async fn sftp_list(
    state: &AppState,
    connection_id: u64,
    path: String,
) -> Result<Vec<SftpEntry>> {
    super::require_unlocked(state).await?;
    let path = crate::sftp::normalise_remote_path(&path)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            let mut out = handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<Vec<SftpEntry>> {
                        let dir = s
                            .read_dir(&path)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))?;
                        let mut acc: Vec<SftpEntry> = Vec::new();
                        for entry in dir {
                            let name = entry.file_name();
                            let attrs = entry.metadata();
                            let full = if path.ends_with('/') {
                                format!("{path}{name}")
                            } else {
                                format!("{path}/{name}")
                            };
                            acc.push(SftpEntry {
                                name,
                                full_path: full,
                                is_dir: attrs.is_dir(),
                                is_symlink: attrs.is_symlink(),
                                size: attrs.size.unwrap_or(0),
                                mtime_unix: attrs.mtime.map(|m| m as i64).unwrap_or(0),
                                mode: attrs.permissions.unwrap_or(0),
                            });
                        }
                        Ok(acc)
                    },
                )
                .await?;
            out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
            Ok(out)
        }
        FileBrowser::Wsl(handle) => wslfs::list::list(handle, &path).await,
    }
}

pub async fn sftp_mkdir(
    state: &AppState,
    connection_id: u64,
    path: String,
) -> Result<()> {
    super::require_unlocked(state).await?;
    let path = crate::sftp::normalise_remote_path(&path)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<()> {
                        s.create_dir(&path)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))
                    },
                )
                .await
        }
        FileBrowser::Wsl(handle) => wslfs::ops::mkdir(handle, &path).await,
    }
}

pub async fn sftp_rmdir(
    state: &AppState,
    connection_id: u64,
    path: String,
) -> Result<()> {
    super::require_unlocked(state).await?;
    let path = crate::sftp::normalise_remote_path(&path)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<()> {
                        s.remove_dir(&path)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))
                    },
                )
                .await
        }
        FileBrowser::Wsl(handle) => wslfs::ops::rmdir(handle, &path).await,
    }
}

pub async fn sftp_remove(
    state: &AppState,
    connection_id: u64,
    path: String,
) -> Result<()> {
    super::require_unlocked(state).await?;
    let path = crate::sftp::normalise_remote_path(&path)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<()> {
                        s.remove_file(&path)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))
                    },
                )
                .await
        }
        FileBrowser::Wsl(handle) => wslfs::ops::remove(handle, &path).await,
    }
}

pub async fn sftp_rename(
    state: &AppState,
    connection_id: u64,
    from: String,
    to: String,
) -> Result<()> {
    super::require_unlocked(state).await?;
    let from = crate::sftp::normalise_remote_path(&from)?;
    let to = crate::sftp::normalise_remote_path(&to)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<()> {
                        s.rename(&from, &to)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))
                    },
                )
                .await
        }
        FileBrowser::Wsl(handle) => wslfs::ops::rename(handle, &from, &to).await,
    }
}

pub async fn sftp_chmod(
    state: &AppState,
    connection_id: u64,
    path: String,
    mode: u32,
) -> Result<()> {
    super::require_unlocked(state).await?;
    let path = crate::sftp::normalise_remote_path(&path)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<()> {
                        // Only the permission bits are mutated — leave
                        // size/uid/gid/atime/mtime untouched on the server.
                        let mut attrs = russh_sftp::protocol::FileAttributes::empty();
                        attrs.permissions = Some(mode);
                        s.set_metadata(&path, attrs)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))
                    },
                )
                .await
        }
        FileBrowser::Wsl(handle) => wslfs::ops::chmod(handle, &path, mode).await,
    }
}

pub async fn sftp_realpath(
    state: &AppState,
    connection_id: u64,
    path: String,
) -> Result<String> {
    super::require_unlocked(state).await?;
    let path = crate::sftp::normalise_remote_path(&path)?;
    let browser = require_browser(state, connection_id).await?;
    match &*browser {
        FileBrowser::Sftp(handle) => {
            handle
                .with_session(
                    async move |s: &mut russh_sftp::client::SftpSession| -> Result<String> {
                        s.canonicalize(&path)
                            .await
                            .map_err(|e| AppError::Sftp(e.to_string()))
                    },
                )
                .await
        }
        FileBrowser::Wsl(handle) => wslfs::ops::realpath(handle, &path).await,
    }
}

/// Transfer tasks run detached; their failure already went out as the
/// final `Event::Transfer`, this keeps it in the log too.
fn log_failed(transfer_id: u64, what: &str, r: Result<()>) {
    if let Err(e) = r {
        tracing::warn!(transfer_id, what, "sftp transfer failed: {e}");
    }
}

async fn require_browser(
    state: &AppState,
    connection_id: u64,
) -> Result<std::sync::Arc<FileBrowser>> {
    state
        .sftp
        .get(connection_id)
        .await
        .ok_or(AppError::NotFound)
}

/// Start a streaming upload. Returns a ticket immediately; progress flows
/// as `Event::Transfer` with this id. The final event has
/// `done = true` (success) or populates `error` (failure).
pub async fn sftp_upload(
    state: &AppState,
    connection_id: u64,
    local_path: String,
    remote_path: String,
) -> Result<TransferTicket> {
    let mut t = transfer_many(state, connection_id, vec![TransferItem { local_path, remote_path }], Direction::Up).await?;
    t.pop().ok_or_else(|| AppError::Validation("no transfer started".into()))
}

/// Hard cap on drag-drop upload size. The Chromium webview holds the whole
/// file in memory before shipping it to Rust, and ballooning past a few
/// hundred MB OOM-risks the webview. Users with larger uploads should hit
/// the Upload button (native file dialog → path-based streaming).
const MAX_DRAG_DROP_BYTES: usize = 256 * 1024 * 1024;

/// Drag-drop upload variant: file contents are passed inline (the webview
/// can't expose a local path on Chromium for security reasons, so the
/// frontend reads the File via FileReader and sends bytes here).
pub async fn sftp_upload_bytes(
    state: &AppState,
    connection_id: u64,
    remote_path: String,
    bytes: Vec<u8>,
) -> Result<TransferTicket> {
    super::require_unlocked(state).await?;
    if bytes.len() > MAX_DRAG_DROP_BYTES {
        return Err(AppError::Validation(format!(
            "drag-drop upload capped at {} MB; use the Upload button for larger files",
            MAX_DRAG_DROP_BYTES / (1024 * 1024)
        )));
    }
    let remote_path = crate::sftp::normalise_remote_path(&remote_path)?;
    let browser = require_browser(state, connection_id).await?;
    let transfer_id = next_transfer_id();
    let events = state.events.clone();

    tokio::spawn(async move {
        match &*browser {
            FileBrowser::Sftp(handle) => {
                log_failed(transfer_id, "upload_bytes", crate::sftp::transfer::upload_bytes(
                    &events, handle, transfer_id, &bytes, &remote_path,
                )
                .await);
            }
            FileBrowser::Wsl(handle) => {
                log_failed(transfer_id, "upload_bytes", crate::wslfs::transfer::upload_bytes(
                    &events, handle, transfer_id, &bytes, &remote_path,
                )
                .await);
            }
        }
    });
    Ok(TransferTicket { transfer_id })
}

/// Start a streaming download. See `sftp_upload` for the progress-event model.
pub async fn sftp_download(
    state: &AppState,
    connection_id: u64,
    remote_path: String,
    local_path: String,
) -> Result<TransferTicket> {
    let mut t = transfer_many(state, connection_id, vec![TransferItem { local_path, remote_path }], Direction::Down).await?;
    t.pop().ok_or_else(|| AppError::Validation("no transfer started".into()))
}

/// One file of a batch transfer.
#[derive(Clone, Debug, Deserialize)]
pub struct TransferItem {
    pub local_path: String,
    pub remote_path: String,
}

#[derive(Clone, Copy, Debug)]
enum Direction {
    Up,
    Down,
}

/// Upload several files one after another in one background task. Each
/// file has its own ticket and `Event::Transfer` stream; events of the
/// first file can arrive before this returns. Only one local file is open
/// at a time, so a dropped tree of thousands of files cannot exhaust file
/// descriptors while the transfers wait for the session lock.
pub async fn sftp_upload_many(
    state: &AppState,
    connection_id: u64,
    items: Vec<TransferItem>,
) -> Result<Vec<TransferTicket>> {
    transfer_many(state, connection_id, items, Direction::Up).await
}

/// Download several files one after another; see [`sftp_upload_many`].
pub async fn sftp_download_many(
    state: &AppState,
    connection_id: u64,
    items: Vec<TransferItem>,
) -> Result<Vec<TransferTicket>> {
    transfer_many(state, connection_id, items, Direction::Down).await
}

async fn transfer_many(
    state: &AppState,
    connection_id: u64,
    items: Vec<TransferItem>,
    dir: Direction,
) -> Result<Vec<TransferTicket>> {
    super::require_unlocked(state).await?;
    let mut paths = Vec::with_capacity(items.len());
    for it in items {
        let remote = crate::sftp::normalise_remote_path(&it.remote_path)?;
        paths.push((PathBuf::from(it.local_path), remote));
    }
    let browser = require_browser(state, connection_id).await?;
    let registry = state.transfers.clone();
    // Ids and cancel flags exist before the task runs, so a queued file can
    // be cancelled before its turn.
    let jobs: Vec<_> = paths
        .into_iter()
        .map(|(local, remote)| {
            let id = next_transfer_id();
            (id, registry.register(id), local, remote)
        })
        .collect();
    let events = state.events.clone();
    let tickets = jobs.iter().map(|(id, ..)| TransferTicket { transfer_id: *id }).collect();
    tracing::debug!(connection_id, n = jobs.len(), ?dir, "sftp: transfers queued");
    tokio::spawn(async move {
        for (id, cancel, local, remote) in jobs {
            let r = match (&*browser, dir) {
                (FileBrowser::Sftp(h), Direction::Up) => crate::sftp::transfer::upload(&events, h, id, &local, &remote, &cancel).await,
                (FileBrowser::Sftp(h), Direction::Down) => crate::sftp::transfer::download(&events, h, id, &remote, &local, &cancel).await,
                (FileBrowser::Wsl(h), Direction::Up) => crate::wslfs::transfer::upload(&events, h, id, &local, &remote, &cancel).await,
                (FileBrowser::Wsl(h), Direction::Down) => crate::wslfs::transfer::download(&events, h, id, &remote, &local, &cancel).await,
            };
            registry.finish(id);
            log_failed(id, if matches!(dir, Direction::Up) { "upload" } else { "download" }, r);
        }
    });
    Ok(tickets)
}

/// Cancel a path-based transfer: a running one stops before its next
/// chunk and removes its partial file, a queued one ends without starting.
/// Its final `Event::Transfer` has `cancelled = true`. Returns false when
/// the transfer already ended.
pub async fn sftp_transfer_cancel(state: &AppState, transfer_id: u64) -> Result<bool> {
    super::require_unlocked(state).await?;
    let found = state.transfers.cancel(transfer_id);
    tracing::debug!(transfer_id, found, "sftp: cancel transfer");
    Ok(found)
}

/// A remote folder flattened for a recursive download.
#[derive(Debug, Default, Serialize)]
pub struct RemoteTree {
    /// Folders below the root, parents first, as paths relative to it
    /// (`a`, `a/b`).
    pub dirs: Vec<String>,
    /// Files (symlinks included, as files) with their relative path.
    pub files: Vec<(String, SftpEntry)>,
    /// Folders below the root that could not be listed.
    pub errors: Vec<String>,
}

/// Upper bound on entries `sftp_walk` collects.
const WALK_MAX_ENTRIES: usize = 200_000;

/// Walk the remote folder `root` depth first. Symlinked folders are not
/// followed (they would loop); they are listed as files. A root that
/// cannot be listed is an error, a sub-folder that cannot is reported in
/// `errors` and skipped.
pub async fn sftp_walk(state: &AppState, connection_id: u64, root: String) -> Result<RemoteTree> {
    let root = crate::sftp::normalise_remote_path(&root)?;
    let mut out = RemoteTree::default();
    // (remote path, relative path); popped in reverse push order.
    let mut stack: Vec<(String, String)> = vec![(root.clone(), String::new())];
    let mut seen = 0usize;
    while let Some((dir, rel)) = stack.pop() {
        let list = match sftp_list(state, connection_id, dir.clone()).await {
            Ok(l) => l,
            Err(e) if rel.is_empty() => return Err(e),
            Err(e) => {
                tracing::warn!(connection_id, %dir, "sftp: walk could not list a folder: {e}");
                out.errors.push(format!("{dir}: {e}"));
                continue;
            }
        };
        seen += list.len();
        if seen > WALK_MAX_ENTRIES {
            return Err(AppError::Validation(format!("{root} has more than {WALK_MAX_ENTRIES} entries")));
        }
        let mut subdirs = Vec::new();
        for e in list {
            let child = if rel.is_empty() { e.name.clone() } else { format!("{rel}/{}", e.name) };
            if e.is_dir && !e.is_symlink {
                out.dirs.push(child.clone());
                subdirs.push((e.full_path.clone(), child));
            } else {
                out.files.push((child, e));
            }
        }
        // Keep listing order: first sub-folder is walked first.
        stack.extend(subdirs.into_iter().rev());
    }
    tracing::debug!(connection_id, %root, dirs = out.dirs.len(), files = out.files.len(), errors = out.errors.len(), "sftp: walked");
    Ok(out)
}

/// Streaming upload — phase A3 of issue #28. Lifts the 256 MB cap by
/// letting the frontend slice the dropped `File` and ship chunks one
/// at a time. Returns an `upload_id` the frontend uses to push
/// chunks; remember to call `sftp_upload_finish` (or
/// `sftp_upload_abort` on cancel) to release the file handle.
pub async fn sftp_upload_begin(
    state: &AppState,
    connection_id: u64,
    remote_path: String,
) -> Result<u64> {
    super::require_unlocked(state).await?;
    let remote_path = crate::sftp::normalise_remote_path(&remote_path)?;
    let browser = require_browser(state, connection_id).await?;
    state.upload_streams.begin(browser, remote_path).await
}

/// Append one chunk. Returns when the chunk has been acknowledged by
/// the writer task — surfaces per-chunk remote write errors so the
/// frontend can stop slicing rather than pushing chunks into a
/// already-failed upload. Bounded channel depth means this awaits if
/// the SFTP link is slower than the frontend.
pub async fn sftp_upload_chunk(
    state: &AppState,
    upload_id: u64,
    bytes: Vec<u8>,
) -> Result<()> {
    super::require_unlocked(state).await?;
    state.upload_streams.chunk(upload_id, bytes).await
}

pub async fn sftp_upload_finish(state: &AppState, upload_id: u64) -> Result<()> {
    super::require_unlocked(state).await?;
    state.upload_streams.finish(upload_id).await
}

pub async fn sftp_upload_abort(state: &AppState, upload_id: u64) -> Result<()> {
    super::require_unlocked(state).await?;
    state.upload_streams.abort(upload_id).await
}

// NOTE on SFTP drag-out: an earlier iteration shipped an OLE
// IDataObject + IStream drag-source so the user could drag a remote
// file from the SFTP pane directly into Windows Explorer. That code
// hit an unworkable WebView2 limitation — `DoDragDrop` from the host
// process always returns E_FAIL once WebView2 is loaded (process-wide
// claim by Chromium's drag controller), and the HTML5
// `dragstart` → OS drag conversion also never fires from a Tauri
// webview. There is no current Microsoft-supplied API surface to
// initiate an OS-level virtual-file drag from a WebView2 host. We
// retain the right-click → Download… save-dialog path in the SFTP
// pane as the supported download UX until WebView2 exposes a drag
// API or we move off WebView2.
