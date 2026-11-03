//! The file pane docked on the left of an SSH or WSL tab (SFTP on the
//! tab's own connection, or the WSL share): breadcrumb, file list,
//! transfers and their progress.
//!
//! - drops and the Upload button use the path-based core transfers
//!   (`sftp_upload_many`), so there is no size cap; one progress strip
//!   shows uploads and downloads alike;
//! - a dropped folder is walked here;
//! - winit 0.30 reports no pointer position during an OS drag and no file
//!   drops at all on Wayland, so a drop lands in the visible pane's
//!   current folder (no per-folder drop targets), and the list's
//!   right-click menu offers "Upload folder…" for desktops without drops;
//! - a failed listing keeps the previous folder instead of showing its
//!   entries under the new path, and a reconnect reopens the browser in
//!   the folder it showed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CursorIcon, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, vec2};
use ezssh_core::api;
use ezssh_core::api::sftp::{SftpEntry, TransferItem, TransferTicket};
use ezssh_core::sftp::transfer::TransferProgress;

use crate::app::App;
use crate::files::Pick;
use crate::icons::Icon;
use crate::layout::{SFTP_COL_NAME, SFTP_COL_SIZE, SFTP_PANE_DEFAULT, SFTP_PANE_MIN, clamp_col};
use crate::style::{ICON_BTN, radius, alpha, font, font_semibold, size, tokens};
use crate::tabs::{TabId, TabStatus};
use crate::views::dialogs::{self, Confirm, PromptAnswer, PromptState};
use crate::widgets::{self, Bounds, Menu, MenuEntry, MenuResult};

/// Mode column: nine monospace characters (`SFTP_MODE_COL_W`).
pub const MODE_COL_W: f32 = 72.0;
const HEADER_H: f32 = crate::style::PANE_HEAD_H;
const COLS_H: f32 = 26.0;
const ROW_H: f32 = 26.0;
const ROW_PAD: f32 = 8.0;
const ROW_GAP: f32 = 8.0;
/// How long a finished / failed transfer stays in the strip.
const DONE_TTL: Duration = Duration::from_millis(1500);
const FAILED_TTL: Duration = Duration::from_secs(4);
/// How long a finished folder job row stays (longer when something failed).
const JOB_TTL: Duration = Duration::from_secs(8);
const JOB_FAILED_TTL: Duration = Duration::from_secs(30);
/// Transfer rows shown at most (the rest is one "N more waiting" line).
const MAX_STRIP_ROWS: usize = 6;
/// Progress of transfers nobody registered (yet) is kept this long.
const EARLY_TTL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------- helpers

/// A file size for the list: B, KB, MB, GB.
pub fn format_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    // Values that would print as "1024.0 KB" move up a unit.
    const ROLL: f64 = 1023.95;
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= ROLL && unit < 3 {
        v /= K;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} B"),
        1 => format!("{v:.1} KB"),
        2 => format!("{v:.1} MB"),
        _ => format!("{v:.2} GB"),
    }
}

/// `rwxrwxrwx` from the lower nine mode bits.
pub fn format_mode(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6, 3, 0] {
        for (i, c) in ['r', 'w', 'x'].into_iter().enumerate() {
            out.push(if (mode >> (shift + 2 - i)) & 1 == 1 { c } else { '-' });
        }
    }
    out
}

/// `joinRemote`: `dir/name`, keeping the root tidy.
pub fn join_remote(dir: &str, name: &str) -> String {
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() { format!("/{name}") } else { format!("{dir}/{name}") }
}

/// Parent folder of a remote path (`/` for top-level entries).
pub fn parent_of(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((p, _)) => p.into(),
    }
}

/// Why `name` cannot be a remote file name, if it cannot.
pub fn invalid_name(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("A name is required")
    } else if name.contains('/') {
        Some("A name cannot contain /")
    } else if name == "." || name == ".." {
        Some("That name is reserved")
    } else {
        None
    }
}

/// A remote file name made safe as one local path component: a remote
/// name like `..\..\x` must not escape the chosen download folder on
/// Windows, and characters Windows refuses become `_`.
pub fn local_name(remote: &str) -> String {
    let bad = |c: char| c == '/' || c == '\\' || c.is_control() || (cfg!(windows) && matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    let mut s: String = remote.chars().map(|c| if bad(c) { '_' } else { c }).collect();
    if cfg!(windows) {
        s = s.trim_end_matches(['.', ' ']).to_string();
        let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
        if reserved {
            s.insert(0, '_');
        }
    }
    if s.is_empty() || s == "." || s == ".." { "_".into() } else { s }
}

/// The file-manager click: plain replaces (unless the row is already part
/// of the selection), Ctrl toggles, Shift extends from the anchor.
pub fn click_select(sel: &mut HashSet<String>, anchor: &mut Option<String>, entries: &[SftpEntry], path: &str, ctrl: bool, shift: bool) {
    if ctrl {
        if !sel.remove(path) {
            sel.insert(path.to_string());
        }
        *anchor = Some(path.to_string());
        return;
    }
    if shift && let Some(a) = anchor.as_deref() {
        let ai = entries.iter().position(|e| e.full_path == a);
        let bi = entries.iter().position(|e| e.full_path == path);
        if let (Some(ai), Some(bi)) = (ai, bi) {
            let (lo, hi) = if ai <= bi { (ai, bi) } else { (bi, ai) };
            *sel = entries[lo..=hi].iter().map(|e| e.full_path.clone()).collect();
        }
        return;
    }
    if !sel.contains(path) {
        sel.clear();
        sel.insert(path.to_string());
    }
    *anchor = Some(path.to_string());
}

// ---------------------------------------------------------------- state

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Opening,
    Ready,
    Failed,
}

/// One tab's browser.
struct Pane {
    /// The connection the browser was opened on; a reconnect reopens it.
    connection_id: u64,
    phase: Phase,
    entries: Vec<SftpEntry>,
    /// Folder `entries` belong to (the tab's `cwd` follows it).
    listed: String,
    /// Latest list request; older answers are dropped.
    serial: u64,
    loading: bool,
    error: Option<String>,
    selected: HashSet<String>,
    anchor: Option<String>,
    /// An upload into `listed` finished: list again once uploads settle.
    refresh_due: bool,
    /// Frame this pane was last drawn.
    seen: u64,
}

impl Pane {
    fn new(connection_id: u64) -> Self {
        Self {
            connection_id,
            phase: Phase::Opening,
            entries: Vec::new(),
            listed: "/".into(),
            serial: 0,
            loading: false,
            error: None,
            selected: HashSet::new(),
            anchor: None,
            refresh_due: false,
            seen: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TState {
    Running,
    Done(Instant),
    Failed(Instant),
    Cancelled(Instant),
}

struct Transfer {
    id: u64,
    tab: TabId,
    way: Way,
    name: String,
    /// Remote folder the file goes into (uploads) or comes from.
    remote_dir: String,
    sent: u64,
    total: u64,
    started: bool,
    state: TState,
    job: Option<u64>,
    /// Cancel asked for, the final event not in yet.
    cancelling: bool,
}

/// A folder going up (dropped / picked) or down (recursive download): its
/// files are ordinary transfers, this counts them.
struct DirJob {
    id: u64,
    tab: TabId,
    way: Way,
    root: String,
    total: usize,
    done: usize,
    failed: usize,
    cancelled: usize,
    /// When the last file ended; the row expires after it.
    finished_at: Option<Instant>,
}

impl DirJob {
    fn finished(&self) -> bool {
        self.done + self.failed + self.cancelled >= self.total
    }
}

/// What a click in the pane asks for.
#[derive(Clone)]
enum Action {
    Download(SftpEntry),
    DownloadMany(Vec<SftpEntry>),
    Rename(SftpEntry),
    Delete(SftpEntry),
    DeleteMany(Vec<SftpEntry>),
    Mkdir,
    UploadFiles,
    UploadFolder,
    Refresh,
}

enum Modal {
    /// `serial` ties an in-flight mkdir / rename to this prompt: a prompt
    /// opened after the first was cancelled must not show its answer.
    Mkdir { tab: TabId, prompt: PromptState, busy: bool, serial: u64 },
    Rename { tab: TabId, entry: SftpEntry, prompt: PromptState, busy: bool, serial: u64 },
    Delete { tab: TabId, entry: SftpEntry },
    DeleteMany { tab: TabId, entries: Vec<SftpEntry> },
}

#[derive(Default)]
pub struct SftpState {
    panes: HashMap<TabId, Pane>,
    transfers: Vec<Transfer>,
    /// Progress that arrived before its ticket was registered.
    early: HashMap<u64, (TransferProgress, Instant)>,
    jobs: Vec<DirJob>,
    next_job: u64,
    menu: Option<(TabId, Menu<Action>)>,
    modal: Option<Modal>,
    /// A native file dialog is open (its serial).
    picking: Option<u64>,
    /// Tab whose pane was drawn this frame and takes OS drops.
    drop_target: Option<TabId>,
}

impl SftpState {
    pub fn close_dialogs(&mut self) {
        self.menu = None;
        self.modal = None;
    }

    pub fn dialog_open(&self) -> bool {
        self.modal.is_some() || self.picking.is_some()
    }
}

#[cfg(test)]
impl SftpState {
    pub(crate) fn listing(&self, tab: TabId) -> Option<(String, Vec<String>)> {
        self.panes.get(&tab).filter(|p| p.phase == Phase::Ready).map(|p| (p.listed.clone(), p.entries.iter().map(|e| e.name.clone()).collect()))
    }

    pub(crate) fn pane_error(&self, tab: TabId) -> Option<String> {
        self.panes.get(&tab).and_then(|p| p.error.clone())
    }

    pub(crate) fn selected(&self, tab: TabId) -> Vec<String> {
        let mut v: Vec<String> = self.panes.get(&tab).map(|p| p.selected.iter().cloned().collect()).unwrap_or_default();
        v.sort();
        v
    }

    /// (running, done, failed) transfers.
    pub(crate) fn transfer_counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for t in &self.transfers {
            match t.state {
                TState::Running => c.0 += 1,
                TState::Done(_) => c.1 += 1,
                TState::Failed(_) => c.2 += 1,
                TState::Cancelled(_) => {}
            }
        }
        c
    }

    pub(crate) fn cancelled_count(&self) -> usize {
        self.transfers.iter().filter(|t| matches!(t.state, TState::Cancelled(_))).count()
    }

    /// (root, total, done, cancelled, finished) of each folder job.
    pub(crate) fn job_states(&self) -> Vec<(String, usize, usize, usize, bool)> {
        self.jobs.iter().map(|j| (j.root.clone(), j.total, j.done, j.cancelled, j.finished())).collect()
    }

    /// Bytes sent of running transfers that reported progress.
    pub(crate) fn progress(&self) -> Vec<u64> {
        self.transfers.iter().filter(|t| t.state == TState::Running && t.started).map(|t| t.sent).collect()
    }

    pub(crate) fn jobs(&self) -> Vec<(String, usize, usize, usize)> {
        self.jobs.iter().map(|j| (j.root.clone(), j.total, j.done, j.failed)).collect()
    }

    pub(crate) fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    pub(crate) fn loading(&self, tab: TabId) -> bool {
        self.panes.get(&tab).is_some_and(|p| p.loading || p.phase == Phase::Opening || p.refresh_due)
    }
}

// ---------------------------------------------------------------- core calls

/// Open the browser of `tab` on `connection_id` unless it already is.
/// A reconnect reopens it in the folder it showed, falling back to home.
fn ensure_open(app: &mut App, tab: TabId, connection_id: u64) {
    let previous = match app.sftp.panes.get(&tab) {
        Some(p) if p.connection_id == connection_id => return,
        Some(p) if p.phase == Phase::Ready => Some(p.listed.clone()),
        _ => None,
    };
    tracing::debug!(tab, connection_id, ?previous, "sftp: open");
    app.sftp.panes.insert(tab, Pane::new(connection_id));
    app.core.call(
        "sftp_open",
        move |st| async move {
            api::sftp::sftp_open(&st, connection_id).await?;
            if let Some(dir) = previous {
                match api::sftp::sftp_list(&st, connection_id, dir.clone()).await {
                    Ok(list) => return Ok((dir, list)),
                    Err(e) => tracing::info!(connection_id, %dir, "sftp: previous folder gone after reconnect, using home: {e}"),
                }
            }
            let home = match api::sftp::sftp_realpath(&st, connection_id, ".".into()).await {
                Ok(h) if !h.is_empty() => h,
                Ok(_) => "/".to_string(),
                Err(e) => {
                    tracing::warn!(connection_id, "sftp: realpath(.) failed, starting at /: {e}");
                    "/".to_string()
                }
            };
            let list = api::sftp::sftp_list(&st, connection_id, home.clone()).await?;
            Ok::<_, ezssh_core::AppError>((home, list))
        },
        move |app, r| {
            let Some(p) = app.sftp.panes.get_mut(&tab).filter(|p| p.connection_id == connection_id) else {
                tracing::debug!(tab, connection_id, "sftp: open result for a replaced pane");
                return;
            };
            match r {
                Ok((dir, entries)) => {
                    tracing::debug!(tab, %dir, n = entries.len(), "sftp: opened");
                    p.phase = Phase::Ready;
                    p.entries = entries;
                    p.listed = dir.clone();
                    p.error = None;
                    if let Some(t) = app.tabs.get_mut(tab) {
                        t.cwd = dir;
                    }
                }
                Err(e) => {
                    tracing::warn!(tab, connection_id, "sftp: open failed: {e}");
                    p.phase = Phase::Failed;
                    p.error = Some(e.to_string());
                }
            }
        },
    );
}

/// List `path`; on success it becomes the pane's folder (and the tab's
/// `cwd`), on failure the pane keeps its folder and shows the error.
fn list(app: &mut App, tab: TabId, path: String) {
    let Some(p) = app.sftp.panes.get_mut(&tab).filter(|p| p.phase == Phase::Ready) else {
        tracing::debug!(tab, %path, "sftp: list ignored, browser not open");
        return;
    };
    p.serial += 1;
    p.loading = true;
    p.refresh_due = false;
    let (serial, connection_id) = (p.serial, p.connection_id);
    tracing::debug!(tab, connection_id, %path, serial, "sftp: list");
    let dir = path.clone();
    app.core.call(
        "sftp_list",
        move |st| async move { api::sftp::sftp_list(&st, connection_id, dir).await },
        move |app, r| {
            let Some(p) = app.sftp.panes.get_mut(&tab).filter(|p| p.connection_id == connection_id && p.serial == serial) else {
                return;
            };
            p.loading = false;
            match r {
                Ok(entries) => {
                    if p.listed != path {
                        p.selected.clear();
                        p.anchor = None;
                    }
                    p.selected.retain(|s| entries.iter().any(|e| &e.full_path == s));
                    p.entries = entries;
                    p.listed = path.clone();
                    p.error = None;
                    if let Some(t) = app.tabs.get_mut(tab) {
                        t.cwd = path;
                    }
                }
                Err(e) => {
                    tracing::warn!(tab, %path, "sftp: list failed: {e}");
                    p.error = Some(e.to_string());
                }
            }
        },
    );
}

fn refresh(app: &mut App, tab: TabId) {
    match app.sftp.panes.get(&tab).map(|p| (p.phase, p.listed.clone(), p.connection_id)) {
        Some((Phase::Ready, dir, _)) => list(app, tab, dir),
        Some((Phase::Failed, _, cid)) => {
            app.sftp.panes.remove(&tab);
            ensure_open(app, tab, cid);
        }
        _ => {}
    }
}

fn pane_conn(app: &App, tab: TabId) -> Option<(u64, String)> {
    app.sftp.panes.get(&tab).filter(|p| p.phase == Phase::Ready).map(|p| (p.connection_id, p.listed.clone()))
}

/// Register freshly started transfers (and apply progress that beat them).
fn register(app: &mut App, tab: TabId, way: Way, tickets: Vec<TransferTicket>, files: Vec<(String, String)>, job: Option<u64>) {
    for (t, (name, remote_dir)) in tickets.into_iter().zip(files) {
        app.sftp.transfers.push(Transfer {
            id: t.transfer_id,
            tab,
            way,
            name,
            remote_dir,
            sent: 0,
            total: 0,
            started: false,
            state: TState::Running,
            job,
            cancelling: false,
        });
        if let Some((p, _)) = app.sftp.early.remove(&t.transfer_id) {
            on_event(app, p);
        }
    }
}

/// What a background upload plan produced.
#[derive(Default)]
struct Planned {
    /// Plain files: ticket + (name, remote dir).
    files: Vec<(TransferTicket, (String, String))>,
    dirs: Vec<PlannedDir>,
    /// Paths that could not be read at all.
    errors: Vec<String>,
}

struct PlannedDir {
    root: String,
    files: Vec<(TransferTicket, (String, String))>,
    /// Files and folders that could not be queued (unreadable, not UTF-8,
    /// directory symlinks).
    failed: Vec<String>,
}

/// One local folder flattened: remote folders to create (parents first)
/// and files to send.
#[derive(Default, Debug)]
struct Walk {
    dirs: Vec<String>,
    files: Vec<(PathBuf, String)>,
    failed: Vec<String>,
}

fn walk(local: &Path, remote: &str, out: &mut Walk) {
    let rd = match std::fs::read_dir(local) {
        Ok(rd) => rd,
        Err(e) => {
            out.failed.push(format!("{}: {e}", local.display()));
            return;
        }
    };
    let mut children: Vec<_> = Vec::new();
    for entry in rd {
        match entry {
            Ok(e) => children.push(e),
            Err(e) => out.failed.push(format!("{}: {e}", local.display())),
        }
    }
    children.sort_by_key(|e| e.file_name());
    for e in children {
        let path = e.path();
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            out.failed.push(format!("{}: name is not valid UTF-8", path.display()));
            continue;
        };
        let rpath = join_remote(remote, &name);
        // Follow symlinks to files; skip symlinked folders (loops).
        let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
        match std::fs::metadata(&path) {
            Ok(m) if m.is_dir() && is_link => out.failed.push(format!("{}: folder symlink skipped", path.display())),
            Ok(m) if m.is_dir() => {
                out.dirs.push(rpath.clone());
                walk(&path, &rpath, out);
            }
            Ok(_) => out.files.push((path, rpath)),
            Err(e) => out.failed.push(format!("{}: {e}", path.display())),
        }
    }
}

/// Upload local files and folders into `target` of `tab`'s browser.
fn upload_paths(app: &mut App, tab: TabId, paths: Vec<PathBuf>, target: String) {
    let Some((cid, _)) = pane_conn(app, tab) else {
        app.toasts.warning("Upload not started", "The file browser is not open.".to_string());
        return;
    };
    tracing::info!(tab, connection_id = cid, n = paths.len(), %target, "sftp: upload");
    app.core.call(
        "sftp upload plan",
        move |st| async move {
            let mut out = Planned::default();
            let mut plain: Vec<TransferItem> = Vec::new();
            let mut plain_names: Vec<(String, String)> = Vec::new();
            for p in paths {
                let meta = match tokio::fs::metadata(&p).await {
                    Ok(m) => m,
                    Err(e) => {
                        out.errors.push(format!("{}: {e}", p.display()));
                        continue;
                    }
                };
                let Some(name) = p.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                    out.errors.push(format!("{}: no usable file name", p.display()));
                    continue;
                };
                if !meta.is_dir() {
                    plain.push(TransferItem { local_path: p.display().to_string(), remote_path: join_remote(&target, &name) });
                    plain_names.push((name, target.clone()));
                    continue;
                }
                let root = join_remote(&target, &name);
                let local = p.clone();
                let remote = root.clone();
                let w = match tokio::task::spawn_blocking(move || {
                    let mut w = Walk::default();
                    walk(&local, &remote, &mut w);
                    w
                })
                .await
                {
                    Ok(w) => w,
                    Err(e) => {
                        out.errors.push(format!("{}: {e}", p.display()));
                        continue;
                    }
                };
                // Existing folders are fine (re-uploading a tree over itself);
                // anything else shows up as failed files below.
                for d in std::iter::once(&root).chain(w.dirs.iter()) {
                    if let Err(e) = api::sftp::sftp_mkdir(&st, cid, d.clone()).await {
                        tracing::debug!(dir = %d, "sftp: mkdir failed (exists?): {e}");
                    }
                }
                let names: Vec<(String, String)> = w.files.iter().map(|(l, r)| (file_label(l), parent_of(r))).collect();
                let items = w.files.into_iter().map(|(l, r)| TransferItem { local_path: l.display().to_string(), remote_path: r }).collect();
                let mut failed = w.failed;
                let files = match api::sftp::sftp_upload_many(&st, cid, items).await {
                    Ok(t) => t.into_iter().zip(names).collect(),
                    Err(e) => {
                        failed.push(e.to_string());
                        Vec::new()
                    }
                };
                out.dirs.push(PlannedDir { root: name, files, failed });
            }
            if !plain.is_empty() {
                match api::sftp::sftp_upload_many(&st, cid, plain).await {
                    Ok(t) => out.files = t.into_iter().zip(plain_names).collect(),
                    Err(e) => out.errors.push(e.to_string()),
                }
            }
            out
        },
        move |app, out: Planned| {
            register_planned(app, tab, Way::Up, out);
            // A folder upload with no files still created folders.
            if let Some(p) = app.sftp.panes.get_mut(&tab) {
                p.refresh_due = true;
            }
        },
    );
}

fn file_label(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Download `(entry, local path)` pairs. A folder entry is walked and
/// recreated under its local path (each name through [`local_name`]); its
/// files become a folder job.
fn download(app: &mut App, tab: TabId, items: Vec<(SftpEntry, PathBuf)>) {
    let Some((cid, _)) = pane_conn(app, tab) else {
        app.toasts.warning("Download not started", "The file browser is not open.".to_string());
        return;
    };
    tracing::info!(tab, connection_id = cid, n = items.len(), "sftp: download");
    app.core.call(
        "sftp download plan",
        move |st| async move {
            let mut out = Planned::default();
            let mut plain: Vec<TransferItem> = Vec::new();
            let mut plain_names: Vec<(String, String)> = Vec::new();
            for (e, local) in items {
                if !e.is_dir {
                    plain_names.push((e.name.clone(), parent_of(&e.full_path)));
                    plain.push(TransferItem { local_path: local.display().to_string(), remote_path: e.full_path });
                    continue;
                }
                let tree = match api::sftp::sftp_walk(&st, cid, e.full_path.clone()).await {
                    Ok(t) => t,
                    Err(err) => {
                        out.errors.push(format!("{}: {err}", e.full_path));
                        continue;
                    }
                };
                let mut failed = tree.errors;
                let local_of = |rel: &str| rel.split('/').fold(local.clone(), |p, c| p.join(local_name(c)));
                let mut dirs_ok = HashSet::new();
                for d in std::iter::once("").chain(tree.dirs.iter().map(String::as_str)) {
                    match tokio::fs::create_dir_all(local_of(d)).await {
                        Ok(()) => {
                            dirs_ok.insert(d.to_string());
                        }
                        Err(err) => failed.push(format!("{}: {err}", local_of(d).display())),
                    }
                }
                let mut items = Vec::new();
                let mut names = Vec::new();
                for (rel, f) in tree.files {
                    let parent_rel = rel.rsplit_once('/').map_or("", |(p, _)| p);
                    if !dirs_ok.contains(parent_rel) {
                        failed.push(format!("{}: local folder not created", f.full_path));
                        continue;
                    }
                    names.push((f.name.clone(), parent_of(&f.full_path)));
                    items.push(TransferItem { local_path: local_of(&rel).display().to_string(), remote_path: f.full_path });
                }
                let files = match api::sftp::sftp_download_many(&st, cid, items).await {
                    Ok(t) => t.into_iter().zip(names).collect(),
                    Err(err) => {
                        failed.push(err.to_string());
                        Vec::new()
                    }
                };
                out.dirs.push(PlannedDir { root: e.name, files, failed });
            }
            if !plain.is_empty() {
                match api::sftp::sftp_download_many(&st, cid, plain).await {
                    Ok(t) => out.files = t.into_iter().zip(plain_names).collect(),
                    Err(err) => out.errors.push(err.to_string()),
                }
            }
            out
        },
        move |app, out: Planned| register_planned(app, tab, Way::Down, out),
    );
}

/// Register what an upload / download plan started; report what it could
/// not.
fn register_planned(app: &mut App, tab: TabId, way: Way, out: Planned) {
    let verb = if way == Way::Up { "Upload" } else { "Download" };
    for e in &out.errors {
        tracing::warn!(tab, ?way, "sftp: transfer not started: {e}");
    }
    if let Some(first) = out.errors.first() {
        let more = if out.errors.len() > 1 { format!(" (and {} more)", out.errors.len() - 1) } else { String::new() };
        app.toasts.danger(format!("{verb} failed"), format!("{first}{more}"));
    }
    let (tickets, files): (Vec<_>, Vec<_>) = out.files.into_iter().unzip();
    register(app, tab, way, tickets, files, None);
    for d in out.dirs {
        for f in &d.failed {
            tracing::warn!(tab, ?way, root = %d.root, "sftp: folder transfer: {f}");
        }
        if let Some(first) = d.failed.first() {
            app.toasts.danger(format!("Folder {} incomplete: {}", verb.to_lowercase(), d.root), format!("{first} ({} item(s) not sent)", d.failed.len()));
        }
        app.sftp.next_job += 1;
        let id = app.sftp.next_job;
        app.sftp.jobs.push(DirJob {
            id,
            tab,
            way,
            root: d.root,
            total: d.files.len() + d.failed.len(),
            done: 0,
            failed: d.failed.len(),
            cancelled: 0,
            finished_at: None,
        });
        let (tickets, files): (Vec<_>, Vec<_>) = d.files.into_iter().unzip();
        register(app, tab, way, tickets, files, Some(id));
    }
}

/// Ask the core to cancel `ids` (running or queued).
fn cancel_transfers(app: &mut App, ids: Vec<u64>) {
    for t in app.sftp.transfers.iter_mut().filter(|t| ids.contains(&t.id)) {
        t.cancelling = true;
    }
    tracing::debug!(n = ids.len(), "sftp: cancel");
    app.core.call(
        "sftp_transfer_cancel",
        move |st| async move {
            for id in ids {
                // False = it ended meanwhile; its final event says how.
                api::sftp::sftp_transfer_cancel(&st, id).await?;
            }
            Ok::<(), ezssh_core::AppError>(())
        },
        |app, r| {
            if let Err(e) = r {
                tracing::warn!("sftp: cancel failed: {e}");
                app.toasts.danger("Cancel failed", e.to_string());
            }
        },
    );
}

/// A core `Event::Transfer`.
pub fn on_event(app: &mut App, p: TransferProgress) {
    let Some(t) = app.sftp.transfers.iter_mut().find(|t| t.id == p.transfer_id) else {
        // Not registered yet (the ticket is still on its way), or its tab
        // closed.
        app.sftp.early.insert(p.transfer_id, (p, Instant::now()));
        return;
    };
    if !matches!(t.state, TState::Running) {
        return;
    }
    t.sent = p.bytes_sent;
    t.total = p.total_bytes.max(t.total);
    t.started = true;
    if !p.done {
        return;
    }
    let (tab, way, job, remote_dir, name) = (t.tab, t.way, t.job, t.remote_dir.clone(), t.name.clone());
    let now = Instant::now();
    t.state = if p.cancelled {
        TState::Cancelled(now)
    } else if p.error.is_some() {
        TState::Failed(now)
    } else {
        TState::Done(now)
    };
    let state = t.state;
    tracing::debug!(id = p.transfer_id, tab, ?way, %name, error = ?p.error, cancelled = p.cancelled, "sftp: transfer finished");
    if let Some(e) = p.error
        && job.is_none()
    {
        // Files of a folder job are counted on the job row instead: a tree
        // dropped into a read-only folder would raise hundreds of toasts.
        let what = if way == Way::Up { "Upload failed" } else { "Download failed" };
        app.toasts.danger(format!("{what}: {name}"), e);
    }
    if let Some(j) = job.and_then(|id| app.sftp.jobs.iter_mut().find(|j| j.id == id)) {
        match state {
            TState::Cancelled(_) => j.cancelled += 1,
            TState::Failed(_) => j.failed += 1,
            _ => j.done += 1,
        }
    }
    if way == Way::Up
        && let Some(pane) = app.sftp.panes.get_mut(&tab)
        && (pane.listed == remote_dir || parent_of(&remote_dir) == pane.listed)
    {
        pane.refresh_due = true;
    }
}

/// Expire finished transfers, drop state of closed tabs, run refreshes
/// that waited for uploads. Returns when to look again.
pub fn tick(app: &mut App, now: Instant) -> Option<Duration> {
    let live: HashSet<TabId> = app.tabs.tabs.iter().map(|t| t.id).collect();
    app.sftp.panes.retain(|tab, _| live.contains(tab));
    app.sftp.early.retain(|_, (_, at)| now.saturating_duration_since(*at) < EARLY_TTL);
    let mut next: Option<Duration> = None;
    // A closed tab's transfers are cancelled, not just forgotten: SSH ones
    // die with the channel, WSL copies would keep going unwatched.
    let orphans: Vec<u64> = app
        .sftp
        .transfers
        .iter()
        .filter(|t| !live.contains(&t.tab) && t.state == TState::Running && !t.cancelling)
        .map(|t| t.id)
        .collect();
    if !orphans.is_empty() {
        tracing::debug!(n = orphans.len(), "sftp: cancelling transfers of closed tabs");
        cancel_transfers(app, orphans);
    }
    app.sftp.jobs.retain_mut(|j| {
        if !live.contains(&j.tab) {
            return false;
        }
        if !j.finished() {
            return true;
        }
        let at = *j.finished_at.get_or_insert(now);
        let ttl = if j.failed > 0 || j.cancelled > 0 { JOB_FAILED_TTL } else { JOB_TTL };
        let left = ttl.saturating_sub(now.saturating_duration_since(at));
        if left.is_zero() {
            return false;
        }
        next = Some(next.map_or(left, |n| n.min(left)));
        true
    });
    app.sftp.transfers.retain(|t| {
        if !live.contains(&t.tab) {
            return false;
        }
        let (at, ttl) = match t.state {
            TState::Running => return true,
            TState::Done(at) => (at, DONE_TTL),
            TState::Failed(at) => (at, FAILED_TTL),
            TState::Cancelled(at) => (at, DONE_TTL),
        };
        let left = ttl.saturating_sub(now.saturating_duration_since(at));
        if left.is_zero() {
            return false;
        }
        next = Some(next.map_or(left, |n| n.min(left)));
        true
    });
    let due: Vec<TabId> = app
        .sftp
        .panes
        .iter()
        .filter(|(tab, p)| p.refresh_due && !p.loading && !app.sftp.transfers.iter().any(|t| t.tab == **tab && t.way == Way::Up && t.state == TState::Running))
        .map(|(tab, _)| *tab)
        .collect();
    for tab in due {
        refresh(app, tab);
    }
    next
}

// ---------------------------------------------------------------- actions

fn apply(app: &mut App, tab: TabId, action: Action) {
    match action {
        Action::Refresh => refresh(app, tab),
        Action::Mkdir => {
            app.sftp.modal = Some(Modal::Mkdir {
                tab,
                prompt: PromptState::new("New directory", "Directory name", "", "", "Create"),
                busy: false,
                serial: crate::files::next_serial(),
            });
        }
        Action::Rename(entry) => {
            let prompt = PromptState::new(format!("Rename \"{}\"", entry.name), "New name", &entry.name, "", "Rename");
            app.sftp.modal = Some(Modal::Rename { tab, entry, prompt, busy: false, serial: crate::files::next_serial() });
        }
        Action::Delete(entry) => app.sftp.modal = Some(Modal::Delete { tab, entry }),
        Action::DeleteMany(entries) => app.sftp.modal = Some(Modal::DeleteMany { tab, entries }),
        // A folder: pick where it goes, it is recreated inside.
        Action::Download(entry) if entry.is_dir => apply(app, tab, Action::DownloadMany(vec![entry])),
        Action::Download(entry) => {
            let serial = crate::files::next_serial();
            app.sftp.picking = Some(serial);
            let what = Pick::Save { title: "Download", file_name: local_name(&entry.name), filters: Vec::new() };
            app.pick_path(what, move |app, path| {
                if app.sftp.picking != Some(serial) {
                    return;
                }
                app.sftp.picking = None;
                if let Some(path) = path {
                    download(app, tab, vec![(entry, path)]);
                }
            });
        }
        Action::DownloadMany(entries) => {
            let serial = crate::files::next_serial();
            app.sftp.picking = Some(serial);
            app.pick_path(Pick::Folder { title: "Download to folder" }, move |app, dir| {
                if app.sftp.picking != Some(serial) {
                    return;
                }
                app.sftp.picking = None;
                if let Some(dir) = dir {
                    let items = entries.into_iter().map(|e| {
                        let local = dir.join(local_name(&e.name));
                        (e, local)
                    });
                    download(app, tab, items.collect());
                }
            });
        }
        Action::UploadFiles => {
            let serial = crate::files::next_serial();
            app.sftp.picking = Some(serial);
            app.pick_files("Upload files", move |app, paths| {
                if app.sftp.picking != Some(serial) {
                    return;
                }
                app.sftp.picking = None;
                if let (false, Some((_, dir))) = (paths.is_empty(), pane_conn(app, tab)) {
                    upload_paths(app, tab, paths, dir);
                }
            });
        }
        Action::UploadFolder => {
            let serial = crate::files::next_serial();
            app.sftp.picking = Some(serial);
            app.pick_path(Pick::Folder { title: "Upload folder" }, move |app, path| {
                if app.sftp.picking != Some(serial) {
                    return;
                }
                app.sftp.picking = None;
                if let (Some(p), Some((_, dir))) = (path, pane_conn(app, tab)) {
                    upload_paths(app, tab, vec![p], dir);
                }
            });
        }
    }
}

/// Upload what the OS dropped on the window into the visible pane. Call
/// once per frame after the panes were drawn.
pub fn handle_drops(app: &mut App, ctx: &egui::Context) {
    let target = app.sftp.drop_target.take();
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    if dropped.is_empty() {
        return;
    }
    let paths: Vec<PathBuf> = dropped.iter().map(|f| f.path().to_path_buf()).filter(|p| p.is_absolute()).collect();
    tracing::info!(n = dropped.len(), with_path = paths.len(), ?target, "sftp: files dropped");
    if paths.is_empty() {
        tracing::warn!("sftp: dropped files carry no local path");
        app.toasts.warning("Nothing to upload", "The drop did not include file paths.".to_string());
        return;
    }
    match target.and_then(|tab| pane_conn(app, tab).map(|(_, dir)| (tab, dir))) {
        Some((tab, dir)) if !app.any_modal_open() => upload_paths(app, tab, paths, dir),
        _ => app.toasts.info("Not uploaded", "Drop files on the open file pane of a connected SSH or WSL tab.".to_string()),
    }
}

// ---------------------------------------------------------------- pane

/// Paint the file pane of `tab` into `rect`; `max_width` bounds the
/// resize handle (the terminal keeps its minimum width).
pub fn pane(app: &mut App, ui: &mut Ui, tab: TabId, rect: Rect, max_width: f32) {
    let Some(t) = app.tabs.get(tab) else {
        return;
    };
    let connection_id = t.connection_id.filter(|_| t.status == TabStatus::Connected);
    let tk = tokens(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, tk.surface);
    ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, tk.border));
    let inner = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 1.0, rect.bottom()));

    let frame = ui.ctx().cumulative_frame_nr();
    if let Some(cid) = connection_id {
        ensure_open(app, tab, cid);
        let p = app.sftp.panes.get_mut(&tab).expect("pane just ensured");
        let mounted = p.seen + 1 < frame;
        p.seen = frame;
        // Shown again after being hidden: the folder may have changed.
        if mounted && p.phase == Phase::Ready && !p.loading {
            list(app, tab, app.sftp.panes[&tab].listed.clone());
        }
    }

    let mut actions: Vec<Action> = Vec::new();
    let mut nav: Option<String> = None;
    let mono = FontId::new(size::SM, app.mono.clone());
    let cwd = app.tabs.get(tab).map(|t| t.cwd.clone()).unwrap_or_else(|| "/".into());
    let phase = connection_id.and_then(|_| app.sftp.panes.get(&tab)).map(|p| p.phase);
    let ready = phase == Some(Phase::Ready);
    let loading = app.sftp.panes.get(&tab).is_some_and(|p| p.loading);

    // Header: breadcrumb + New directory / Upload / Refresh.
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), HEADER_H));
    ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, tk.border));
    let btns_w = 3.0 * ICON_BTN + 2.0 * 2.0 + 8.0;
    let crumbs = Rect::from_min_max(header.min + vec2(6.0, 0.0), Pos2::new(header.right() - btns_w - 4.0, header.bottom() - 1.0));
    ui.scope_builder(UiBuilder::new().max_rect(crumbs).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        if let Some(p) = breadcrumb(ui, tab, &cwd, &mono, ready) {
            nav = Some(p);
        }
    });
    let mut x = header.right() - 8.0 - ICON_BTN;
    for (icon, tip, action, enabled) in [
        (Icon::RefreshCw, "Refresh", Action::Refresh, phase.is_some_and(|p| p != Phase::Opening)),
        (Icon::Upload, "Upload files", Action::UploadFiles, ready),
        (Icon::FolderPlus, "New directory", Action::Mkdir, ready),
    ] {
        let r = Rect::from_min_size(Pos2::new(x, header.center().y - ICON_BTN / 2.0), vec2(ICON_BTN, ICON_BTN));
        let resp = ui.scope_builder(UiBuilder::new().max_rect(r), |ui| {
            ui.add_enabled_ui(enabled, |ui| widgets::icon_btn(ui, icon, false, tip)).inner
        });
        if resp.inner.clicked() {
            actions.push(action);
        }
        x -= ICON_BTN + 2.0;
    }
    if loading {
        let c = Pos2::new(crumbs.right() - 6.0, header.center().y);
        ui.scope_builder(UiBuilder::new().max_rect(Rect::from_center_size(c, vec2(12.0, 12.0))), |ui| crate::form::spinner(ui, 10.0));
    }

    let mut top = header.bottom();
    // Error strip.
    if let Some(err) = app.sftp.panes.get(&tab).and_then(|p| p.error.clone()).filter(|_| connection_id.is_some()) {
        let w = inner.width() - 16.0;
        let g = ui.painter().layout(err.clone(), font(size::SM), tk.danger, w - 40.0);
        let r = Rect::from_min_size(Pos2::new(inner.left() + 8.0, top + 6.0), vec2(w, g.size().y + 12.0));
        ui.painter().rect_filled(r, radius::MD, alpha(tk.danger, 0.1));
        ui.painter().rect_stroke(r, radius::MD, Stroke::new(1.0, alpha(tk.danger, 0.4)), StrokeKind::Inside);
        ui.painter().galley(r.min + vec2(10.0, 6.0), g, tk.danger);
        let x_rect = Rect::from_center_size(Pos2::new(r.right() - 14.0, r.top() + 12.0), vec2(20.0, 20.0));
        let resp = ui.interact(x_rect, Id::unique(("sftp-error-x", tab)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        widgets::a11y(&resp, egui::Role::Button, "Dismiss error");
        widgets::paint_icon_button(ui, &resp, x_rect, Icon::X, 12.0, false, Some(tk.danger));
        widgets::a11y(&ui.interact(r.shrink2(vec2(24.0, 0.0)), Id::unique(("sftp-error", tab)), Sense::hover()), egui::Role::Alert, &err);
        if resp.clicked()
            && let Some(p) = app.sftp.panes.get_mut(&tab)
        {
            p.error = None;
        }
        top = r.bottom() + 6.0;
    }

    // Progress strips at the bottom, at most half the pane.
    let strip_rows = strip_rows(app, tab);
    let strips_h = (section_h(&strip_rows.0) + section_h(&strip_rows.1)).min(inner.height() * 0.5);
    let strips = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - strips_h), inner.max);
    let list_rect = Rect::from_min_max(Pos2::new(inner.left(), top), Pos2::new(inner.right(), strips.top()));

    let mut menu: Option<Menu<Action>> = None;
    match (connection_id, phase) {
        (None, _) => {
            let text = if app.tabs.get(tab).is_some_and(|t| t.status == TabStatus::Connecting) { "Connecting…" } else { "Not connected" };
            let mut child = ui.new_child(UiBuilder::new().max_rect(list_rect.shrink(8.0)).id_salt(("sftp-empty", tab)));
            widgets::empty_state(&mut child, Icon::FolderTree, text, "The file browser opens on the tab's connection.", None, true);
        }
        (Some(_), Some(Phase::Opening)) => {
            ui.scope_builder(UiBuilder::new().max_rect(list_rect).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
                ui.add_space(32.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    crate::form::spinner(ui, 14.0);
                    ui.label(RichText::new("Opening SFTP…").font(font(size::SM)).color(tk.muted));
                });
            });
        }
        (Some(_), Some(Phase::Failed)) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(list_rect.shrink(8.0)).id_salt(("sftp-failed", tab)));
            if widgets::empty_state(&mut child, Icon::AlertTriangle, "File browser unavailable", "", Some(("Retry", true)), true).is_some_and(|r| r.clicked()) {
                actions.push(Action::Refresh);
            }
        }
        (Some(_), _) => {
            let out = file_list(app, ui, tab, list_rect, &mono);
            actions.extend(out.actions);
            if out.nav.is_some() {
                nav = out.nav;
            }
            menu = out.menu;
        }
    }

    if strips_h > 0.0 {
        ui.scope_builder(UiBuilder::new().max_rect(strips), |ui| {
            ui.set_clip_rect(strips);
            strips_ui(app, ui, tab, strips, strip_rows);
        });
    }

    // OS file drag over the window: this pane takes the drop (a modal
    // refuses it, so no overlay then either).
    if ready {
        app.sftp.drop_target = Some(tab);
        if !app.any_modal_open() && ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) {
            drop_overlay(ui, inner, &cwd, &mono);
        }
    }

    // Width handle.
    let out = widgets::resize_strip(
        ui,
        Id::unique(("sftp-pane-resize", tab)),
        rect,
        &mut app.layout.sftp_pane_width,
        Bounds { min: SFTP_PANE_MIN, max: max_width, default: SFTP_PANE_DEFAULT },
        "Resize file pane",
    );
    if out.ended {
        app.save_layout();
    }

    if let Some(m) = menu {
        app.sftp.menu = Some((tab, m));
    }
    if let Some(path) = nav {
        list(app, tab, path);
    }
    for a in actions {
        apply(app, tab, a);
    }
}

/// `SftpBreadcrumb`: root, then one button per segment; the last one is
/// highlighted. Scrolls sideways, kept at its end.
fn breadcrumb(ui: &mut Ui, tab: TabId, path: &str, mono: &FontId, enabled: bool) -> Option<String> {
    let tk = tokens(ui.ctx());
    let mut nav = None;
    egui::ScrollArea::horizontal().id_salt(("sftp-crumbs", tab)).stick_to_right(true).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_enabled_ui(enabled, |ui| {
                if widgets::icon_button(ui, Icon::Slash, 24.0, 14.0, false, "Root").clicked() {
                    nav = Some("/".to_string());
                }
                let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                for (i, seg) in parts.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 20.0), Sense::hover());
                    Icon::ChevronRight.paint(ui, r, 12.0, alpha(tk.muted, 0.7));
                    let last = i + 1 == parts.len();
                    let g = ui.painter().layout_no_wrap(seg.to_string(), mono.clone(), tk.fg);
                    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 10.0, 24.0), Sense::click());
                    let full = format!("/{}", parts[..=i].join("/"));
                    widgets::a11y(&resp, egui::Role::Link, &full);
                    if resp.hovered() {
                        ui.painter().rect_filled(r, radius::SM, tk.surface2);
                    }
                    let color = if last || resp.hovered() { tk.fg } else { tk.muted };
                    ui.painter().galley_with_override_text_color(Pos2::new(r.left() + 5.0, r.center().y - g.size().y / 2.0), g, color);
                    if resp.clicked() {
                        nav = Some(full);
                    }
                }
            });
        });
    });
    nav
}

struct ListOut {
    actions: Vec<Action>,
    nav: Option<String>,
    menu: Option<Menu<Action>>,
}

/// The table: sticky column header with Name / Size handles, rows (only
/// the visible ones are laid out), selection and the context menus.
fn file_list(app: &mut App, ui: &mut Ui, tab: TabId, rect: Rect, mono: &FontId) -> ListOut {
    let tk = tokens(ui.ctx());
    let mut out = ListOut { actions: Vec::new(), nav: None, menu: None };
    let cols = app.layout.sftp_columns;
    let min_w = cols.name + cols.size + MODE_COL_W + 2.0 * ROW_GAP + 2.0 * ROW_PAD;
    let Some(p) = app.sftp.panes.get(&tab) else {
        return out;
    };
    let n = p.entries.len();
    let mods = ui.input(|i| i.modifiers);
    let mut click: Option<(usize, bool, bool)> = None;
    let mut secondary: Option<(usize, Pos2)> = None;
    let mut bg_menu: Option<Pos2> = None;
    let mut clear = false;
    let mut name_w_out = None;
    let mut size_w_out = None;
    let mut ended = false;

    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        egui::ScrollArea::both().id_salt(("sftp-list", tab)).auto_shrink(false).show_viewport(ui, |ui, vp| {
            let total_w = vp.width().max(min_w);
            let name_w = total_w - cols.size - MODE_COL_W - 2.0 * ROW_GAP - 2.0 * ROW_PAD;
            let rows_h = if n == 0 { 120.0 } else { n as f32 * ROW_H };
            let content_h = (COLS_H + rows_h).max(vp.height());
            let (content, bg) = ui.allocate_exact_size(vec2(total_w, content_h), Sense::click());
            let origin = content.min;
            if bg.clicked() {
                clear = true;
            }
            if bg.secondary_clicked() {
                bg_menu = bg.interact_pointer_pos();
            }
            let col_x = |name_w: f32| {
                let name = origin.x + ROW_PAD;
                let size = name + name_w + ROW_GAP;
                let mode = size + cols.size + ROW_GAP;
                (name, size, mode)
            };
            let (nx, sx, mx) = col_x(name_w);
            if n == 0 {
                let r = Rect::from_min_size(origin + vec2(0.0, COLS_H), vec2(vp.width(), 120.0));
                let mut child = ui.new_child(UiBuilder::new().max_rect(r).id_salt(("sftp-empty-dir", tab)));
                widgets::empty_state(&mut child, Icon::Inbox, "Empty directory", "", None, true);
            }
            let first = (((vp.min.y - COLS_H) / ROW_H).floor().max(0.0)) as usize;
            let last = ((((vp.max.y - COLS_H) / ROW_H).ceil().max(0.0)) as usize).min(n);
            for i in first..last {
                let e = &p.entries[i];
                let r = Rect::from_min_size(origin + vec2(0.0, COLS_H + i as f32 * ROW_H), vec2(total_w, ROW_H));
                let resp = ui.interact(r, Id::unique(("sftp-row", tab, &e.full_path)), Sense::click());
                let selected = p.selected.contains(&e.full_path);
                widgets::a11y_selected(&resp, egui::Role::Row, selected, &e.name);
                widgets::row_fill(ui, r, resp.hovered(), selected);
                let (icon, color) = if e.is_dir {
                    (Icon::Folder, tk.fg2)
                } else if e.is_symlink {
                    (Icon::FileSymlink, tk.warning)
                } else {
                    (Icon::File, tk.muted)
                };
                icon.paint(ui, Rect::from_center_size(Pos2::new(nx + 8.0, r.center().y), vec2(15.0, 15.0)), 15.0, color);
                let name_r = Rect::from_min_max(Pos2::new(nx + 22.0, r.top()), Pos2::new(nx + name_w, r.bottom()));
                widgets::paint_text(ui, name_r, &e.name, font(size::BASE), tk.fg);
                if !e.is_dir {
                    let size_r = Rect::from_min_max(Pos2::new(sx, r.top()), Pos2::new(sx + cols.size, r.bottom()));
                    let g = widgets::elide(ui, &format_size(e.size), font(size::SM), cols.size);
                    let pos = Align2::RIGHT_CENTER.align_size_within_rect(g.size(), size_r).min;
                    ui.painter().galley_with_override_text_color(pos, g, tk.muted);
                }
                let mode_r = Rect::from_min_max(Pos2::new(mx, r.top()), Pos2::new(mx + MODE_COL_W, r.bottom()));
                widgets::paint_text(ui, mode_r, &format_mode(e.mode), mono.clone(), tk.muted);
                if resp.double_clicked() {
                    // readdir has lstat semantics: a symlink to a directory
                    // is not `is_dir`. Entering it lists the target or
                    // shows the server's error.
                    if e.is_dir || e.is_symlink {
                        out.nav = Some(e.full_path.clone());
                    }
                } else if resp.clicked() {
                    click = Some((i, mods.command, mods.shift));
                }
                if resp.secondary_clicked() {
                    secondary = resp.interact_pointer_pos().map(|pos| (i, pos));
                }
            }

            // Sticky column header, over the rows (registered last, so it
            // takes the pointer).
            let h = Rect::from_min_size(Pos2::new(origin.x, vp.min.y + origin.y), vec2(total_w, COLS_H));
            ui.interact(h, Id::unique(("sftp-cols", tab)), Sense::click());
            ui.painter().rect_filled(h, 0.0, tk.surface);
            ui.painter().hline(h.x_range(), h.bottom() - 0.5, Stroke::new(1.0, tk.border));
            let label = |ui: &Ui, x: f32, w: f32, text: &str, right: bool| {
                let g = ui.painter().layout_no_wrap(text.to_string(), font_semibold(size::XS), tk.muted);
                let x = if right { x + w - 6.0 - g.size().x } else { x };
                ui.painter().galley(Pos2::new(x, h.center().y - g.size().y / 2.0), g, tk.muted);
            };
            label(ui, nx, name_w, "NAME", false);
            label(ui, sx, cols.size, "SIZE", true);
            label(ui, mx, MODE_COL_W, "MODE", false);
            for (key, x, w, bounds, text) in [
                ("name", nx, name_w, SFTP_COL_NAME, "Resize Name column"),
                ("size", sx, cols.size, SFTP_COL_SIZE, "Resize Size column"),
            ] {
                let cell = Rect::from_min_size(Pos2::new(x, h.top()), vec2(w + ROW_GAP / 2.0 + 2.0, COLS_H));
                let mut width = cell.width();
                let pad = cell.width() - w;
                let r = widgets::resize_strip(
                    ui,
                    Id::unique(("sftp-col", tab, key)),
                    cell,
                    &mut width,
                    Bounds { min: bounds.0 + pad, max: bounds.1 + pad, default: bounds.2 + pad },
                    text,
                );
                if r.changed {
                    let v = clamp_col(bounds, width - pad);
                    if key == "name" { name_w_out = Some(v) } else { size_w_out = Some(v) }
                }
                ended |= r.ended;
            }
        });
    });

    // Apply what the rows asked for.
    let p = app.sftp.panes.get_mut(&tab).expect("pane");
    if let Some((i, ctrl, shift)) = click {
        let path = p.entries[i].full_path.clone();
        click_select(&mut p.selected, &mut p.anchor, &p.entries, &path, ctrl, shift);
    } else if clear {
        p.selected.clear();
        p.anchor = None;
    }
    if let Some((i, pos)) = secondary {
        let e = p.entries[i].clone();
        if !p.selected.contains(&e.full_path) {
            p.selected.clear();
            p.selected.insert(e.full_path.clone());
            p.anchor = Some(e.full_path.clone());
        }
        // The right-clicked row is in the selection now; with more than
        // one selected, Download and Delete act on all of them.
        let picked: Vec<SftpEntry> = p.entries.iter().filter(|x| p.selected.contains(&x.full_path)).cloned().collect();
        let mut entries = Vec::new();
        if picked.len() > 1 {
            let noun = if picked.iter().any(|x| x.is_dir) { "items" } else { "files" };
            let n = picked.len();
            entries.push(MenuEntry::item(format!("Download {n} {noun}…"), Action::DownloadMany(picked.clone())));
            entries.push(MenuEntry::item(format!("Rename \"{}\"…", e.name), Action::Rename(e)));
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::danger(format!("Delete {n} {noun}"), Action::DeleteMany(picked)));
        } else {
            entries.push(MenuEntry::item("Download…", Action::Download(e.clone())));
            entries.push(MenuEntry::item("Rename…", Action::Rename(e.clone())));
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::danger("Delete", Action::Delete(e)));
        }
        out.menu = Some(Menu::new(pos, entries));
    } else if let Some(pos) = bg_menu {
        out.menu = Some(Menu::new(
            pos,
            vec![
                MenuEntry::item("Upload files…", Action::UploadFiles),
                MenuEntry::item("Upload folder…", Action::UploadFolder),
                MenuEntry::item("New directory…", Action::Mkdir),
                MenuEntry::Separator,
                MenuEntry::item("Refresh", Action::Refresh),
            ],
        ));
    }
    if let Some(w) = name_w_out {
        app.layout.sftp_columns.name = w;
    }
    if let Some(w) = size_w_out {
        app.layout.sftp_columns.size = w;
    }
    if ended {
        app.save_layout();
    }
    out
}

/// The × at the end of a strip line.
#[derive(Clone, Debug)]
enum StripBtn {
    /// Remove a finished folder job.
    Dismiss(u64),
    /// Cancel the rest of a folder job.
    CancelJob(u64),
    /// Cancel these transfers.
    Cancel(Vec<u64>),
}

/// One line of a progress strip.
struct StripRow {
    label: String,
    right: String,
    right_color: Color32,
    /// Bar fill 0–1, `None` = no bar.
    bar: Option<f32>,
    /// The × and its accessible name.
    button: Option<(StripBtn, String)>,
}

fn verb(way: Way) -> &'static str {
    if way == Way::Up { "upload" } else { "download" }
}

fn strip_rows(app: &App, tab: TabId) -> (Vec<StripRow>, Vec<StripRow>) {
    let tk = tokens(&app.ctx);
    let jobs = app
        .sftp
        .jobs
        .iter()
        .filter(|j| j.tab == tab)
        .map(|j| {
            let fin = j.finished();
            let what = format!("{} {}/", verb(j.way), j.root);
            let right = if !fin {
                format!("{}/{}", j.done + j.failed + j.cancelled, j.total)
            } else if j.cancelled > 0 {
                format!("cancelled ({}/{})", j.done, j.total)
            } else if j.failed > 0 {
                format!("done ({} failed)", j.failed)
            } else {
                format!("done ({} files)", j.done)
            };
            let button = if fin {
                (StripBtn::Dismiss(j.id), format!("Dismiss {what}"))
            } else {
                (StripBtn::CancelJob(j.id), format!("Cancel {what}"))
            };
            StripRow {
                label: format!("{}{what}", if fin && j.cancelled == 0 { "✓ " } else { "" }),
                right,
                right_color: if j.failed > 0 { tk.danger } else { tk.muted },
                bar: (!fin).then(|| if j.total == 0 { 0.0 } else { (j.done + j.failed + j.cancelled) as f32 / j.total as f32 }),
                button: Some(button),
            }
        })
        .collect();
    // Files of a folder job show only while they send (the job row counts
    // the rest); running ones first, then finished, then queued, capped.
    let mut shown: Vec<&Transfer> =
        app.sftp.transfers.iter().filter(|t| t.tab == tab && (t.job.is_none() || (t.state == TState::Running && t.started))).collect();
    shown.sort_by_key(|t| match t.state {
        TState::Running if t.started => 0,
        TState::Done(_) | TState::Failed(_) | TState::Cancelled(_) => 1,
        TState::Running => 2,
    });
    let waiting: Vec<u64> = shown.iter().skip(MAX_STRIP_ROWS).filter(|t| t.state == TState::Running && !t.cancelling).map(|t| t.id).collect();
    // Hidden finished rows expire on their own; only queued ones are announced.
    let hidden = waiting.len();
    let mut transfers: Vec<StripRow> = shown
        .into_iter()
        .take(MAX_STRIP_ROWS)
        .map(|t| {
            let what = format!("{} {}", verb(t.way), t.name);
            let pct = if t.total > 0 { (t.sent as f64 / t.total as f64 * 100.0).floor() as u32 } else { 0 };
            let (prefix, right, color, bar) = match t.state {
                TState::Running if t.cancelling => ("", "cancelling…".to_string(), tk.muted, Some(pct as f32 / 100.0)),
                TState::Running if !t.started => ("", "…".to_string(), tk.muted, Some(0.0)),
                TState::Running => ("", format!("{pct}%"), tk.muted, Some(pct as f32 / 100.0)),
                TState::Done(_) => ("✓ ", "done".to_string(), tk.muted, None),
                TState::Failed(_) => ("✗ ", "failed".to_string(), tk.danger, None),
                TState::Cancelled(_) => ("", "cancelled".to_string(), tk.muted, None),
            };
            let button = (t.state == TState::Running && !t.cancelling).then(|| (StripBtn::Cancel(vec![t.id]), format!("Cancel {what}")));
            StripRow { label: format!("{prefix}{what}"), right, right_color: color, bar, button }
        })
        .collect();
    if hidden > 0 {
        let button = (!waiting.is_empty()).then(|| {
            let label = format!("Cancel {} waiting", waiting.len());
            (StripBtn::Cancel(waiting), label)
        });
        transfers.push(StripRow { label: format!("{hidden} more waiting"), right: String::new(), right_color: tk.muted, bar: None, button });
    }
    (jobs, transfers)
}

fn row_h(r: &StripRow) -> f32 {
    18.0 + if r.bar.is_some() { 8.0 } else { 0.0 }
}

/// Height of one transfer card (10 px margins around, 10 px padding in).
fn section_h(rows: &[StripRow]) -> f32 {
    if rows.is_empty() {
        return 0.0;
    }
    8.0 + 20.0 + rows.iter().map(row_h).sum::<f32>() + 6.0 * (rows.len() - 1) as f32
}

fn strips_ui(app: &mut App, ui: &mut Ui, tab: TabId, rect: Rect, rows: (Vec<StripRow>, Vec<StripRow>)) {
    let tk = tokens(ui.ctx());
    let (jobs, transfers) = rows;
    let mut y = rect.top();
    let mut pressed: Option<StripBtn> = None;
    for (section, rows) in [jobs, transfers].into_iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        let sec = Rect::from_min_size(Pos2::new(rect.left(), y), vec2(rect.width(), section_h(&rows)));
        let card = Rect::from_min_max(Pos2::new(sec.left() + 8.0, sec.top() + 8.0), Pos2::new(sec.right() - 8.0, sec.bottom()));
        ui.painter().rect_filled(card, radius::LG, tk.surface2);
        let mut ry = card.top() + 10.0;
        for (row, r) in rows.iter().enumerate() {
            let line = Rect::from_min_size(Pos2::new(card.left() + 10.0, ry), vec2(card.width() - 20.0, 18.0));
            let rg = ui.painter().layout_no_wrap(r.right.clone(), font(size::SM), r.right_color);
            let mut right_edge = line.right();
            if let Some((btn, label)) = &r.button {
                let xr = Rect::from_center_size(Pos2::new(line.right() - 9.0, line.center().y), vec2(20.0, 20.0));
                // Two rows can share a label (the same file dropped twice).
                let resp = ui.interact(xr, Id::unique(("sftp-strip-x", tab, section, row)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                widgets::a11y(&resp, egui::Role::Button, label);
                widgets::paint_icon_button(ui, &resp, xr, Icon::X, 12.0, false, None);
                if resp.on_hover_text(label.as_str()).clicked() {
                    pressed = Some(btn.clone());
                }
                right_edge -= 24.0;
            }
            let rpos = Pos2::new(right_edge - rg.size().x, line.center().y - rg.size().y / 2.0);
            ui.painter().galley(rpos, rg, r.right_color);
            let lr = Rect::from_min_max(line.min, Pos2::new(rpos.x - 8.0, line.bottom()));
            let shown = widgets::paint_text(ui, lr, &r.label, font(size::BASE), tk.fg2);
            widgets::a11y(&ui.interact(shown, Id::unique(("sftp-strip", &r.label, ry as i32)), Sense::hover()), egui::Role::ProgressIndicator, &format!("{} {}", r.label, r.right));
            if let Some(f) = r.bar {
                let bar = Rect::from_min_size(Pos2::new(line.left(), line.bottom() + 3.0), vec2(line.width(), 4.0));
                ui.painter().rect_filled(bar, radius::PILL, tk.surface3);
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f.clamp(0.0, 1.0), 4.0)), radius::PILL, tk.accent);
            }
            ry += row_h(r) + 6.0;
        }
        y = sec.bottom();
    }
    match pressed {
        Some(StripBtn::Dismiss(id)) => app.sftp.jobs.retain(|j| j.id != id),
        Some(StripBtn::CancelJob(id)) => {
            let ids = app.sftp.transfers.iter().filter(|t| t.job == Some(id) && t.state == TState::Running && !t.cancelling).map(|t| t.id).collect();
            cancel_transfers(app, ids);
        }
        Some(StripBtn::Cancel(ids)) => cancel_transfers(app, ids),
        None => {}
    }
}

fn drop_overlay(ui: &Ui, rect: Rect, cwd: &str, mono: &FontId) {
    let tk = tokens(ui.ctx());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, alpha(tk.bg, 0.6));
    let r = rect.shrink(8.0);
    p.rect_filled(r, radius::LG, tk.accent_soft);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(2.0, tk.accent), 6.0, 4.0));
    let c = rect.center();
    Icon::CloudUpload.paint(ui, Rect::from_center_size(c - vec2(0.0, 26.0), vec2(32.0, 32.0)), 32.0, tk.accent);
    p.text(c + vec2(0.0, 8.0), Align2::CENTER_CENTER, "Drop to upload", font_semibold(size::MD), tk.fg);
    p.text(c + vec2(0.0, 28.0), Align2::CENTER_CENTER, cwd, mono.clone(), tk.muted);
}

// ---------------------------------------------------------------- dialogs

/// The pane's menu, prompts, delete confirmation and the native-dialog
/// wait. Call once per frame.
pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    if app.sftp.picking.is_some() {
        crate::files::waiting_for_picker(ctx, Id::unique("sftp-picking"));
    }
    if let Some((tab, menu)) = app.sftp.menu.as_mut() {
        let tab = *tab;
        match menu.show(ctx, Id::unique("sftp-menu")) {
            MenuResult::Open => {}
            MenuResult::Closed => app.sftp.menu = None,
            MenuResult::Chosen(a) => {
                app.sftp.menu = None;
                apply(app, tab, a);
            }
        }
    }
    let Some(modal) = app.sftp.modal.as_mut() else {
        return;
    };
    match modal {
        Modal::Mkdir { tab, prompt, busy, serial } => {
            let (tab, serial) = (*tab, *serial);
            match prompt.show(ctx, Id::unique("sftp-mkdir")) {
                Some(PromptAnswer::Cancel) => app.sftp.modal = None,
                Some(PromptAnswer::Submit(_)) if *busy => {}
                Some(PromptAnswer::Submit(name)) => {
                    if let Some(e) = invalid_name(&name) {
                        prompt.set_error(e);
                        return;
                    }
                    *busy = true;
                    let Some((cid, dir)) = pane_conn(app, tab) else {
                        tracing::warn!(tab, "sftp: mkdir submitted but the file browser is gone");
                        app.sftp.modal = None;
                        app.toasts.warning("Not created", "The file browser is not open.".to_string());
                        return;
                    };
                    let path = join_remote(&dir, &name);
                    tracing::debug!(tab, %path, "sftp: mkdir");
                    app.core.call(
                        "sftp_mkdir",
                        move |st| async move { api::sftp::sftp_mkdir(&st, cid, path).await },
                        move |app, r| finish_prompt(app, tab, serial, r, "Directory created", name),
                    );
                }
                None => {}
            }
        }
        Modal::Rename { tab, entry, prompt, busy, serial } => {
            let (tab, serial) = (*tab, *serial);
            match prompt.show(ctx, Id::unique("sftp-rename")) {
                Some(PromptAnswer::Cancel) => app.sftp.modal = None,
                Some(PromptAnswer::Submit(_)) if *busy => {}
                Some(PromptAnswer::Submit(name)) if name == entry.name => app.sftp.modal = None,
                Some(PromptAnswer::Submit(name)) => {
                    if let Some(e) = invalid_name(&name) {
                        prompt.set_error(e);
                        return;
                    }
                    let from = entry.full_path.clone();
                    let to = join_remote(&parent_of(&from), &name);
                    let done = format!("{} → {name}", entry.name);
                    *busy = true;
                    let Some((cid, _)) = pane_conn(app, tab) else {
                        tracing::warn!(tab, "sftp: rename submitted but the file browser is gone");
                        app.sftp.modal = None;
                        app.toasts.warning("Not renamed", "The file browser is not open.".to_string());
                        return;
                    };
                    tracing::debug!(tab, %from, %to, "sftp: rename");
                    app.core.call(
                        "sftp_rename",
                        move |st| async move { api::sftp::sftp_rename(&st, cid, from, to).await },
                        move |app, r| finish_prompt(app, tab, serial, r, "Renamed", done),
                    );
                }
                None => {}
            }
        }
        Modal::DeleteMany { tab, entries } => {
            let tab = *tab;
            let dirs = entries.iter().filter(|e| e.is_dir).count();
            let noun = if dirs > 0 { "items" } else { "files" };
            let title = format!("Delete {} {noun}?", entries.len());
            let body = if dirs > 0 { "Folders must be empty on the remote server. This cannot be undone." } else { "This cannot be undone." };
            let answer = dialogs::confirm(ctx, Id::unique("sftp-delete-many"), Confirm { title: &title, body, confirm_text: "Delete", danger: true });
            let entries = entries.clone();
            match answer {
                Some(false) => app.sftp.modal = None,
                Some(true) => {
                    app.sftp.modal = None;
                    delete_many(app, tab, entries);
                }
                None => {}
            }
        }
        Modal::Delete { tab, entry } => {
            let tab = *tab;
            let title = format!("Delete \"{}\"?", entry.name);
            let body = if entry.is_dir { "The directory must be empty on the remote server." } else { "This cannot be undone." };
            let answer = dialogs::confirm(ctx, Id::unique("sftp-delete"), Confirm { title: &title, body, confirm_text: "Delete", danger: true });
            let entry = entry.clone();
            match answer {
                Some(false) => app.sftp.modal = None,
                Some(true) => {
                    app.sftp.modal = None;
                    let Some((cid, _)) = pane_conn(app, tab) else {
                        tracing::warn!(tab, "sftp: delete confirmed but the file browser is gone");
                        app.toasts.warning("Not deleted", "The file browser is not open.".to_string());
                        return;
                    };
                    tracing::debug!(tab, path = %entry.full_path, dir = entry.is_dir, "sftp: delete");
                    let (path, is_dir, name) = (entry.full_path, entry.is_dir, entry.name);
                    app.core.call(
                        "sftp delete",
                        move |st| async move {
                            if is_dir { api::sftp::sftp_rmdir(&st, cid, path).await } else { api::sftp::sftp_remove(&st, cid, path).await }
                        },
                        move |app, r| match r {
                            Ok(()) => {
                                app.toasts.success("Deleted", name);
                                refresh(app, tab);
                            }
                            Err(e) => {
                                tracing::warn!(tab, %name, "sftp: delete failed: {e}");
                                app.toasts.danger("Delete failed", e.to_string());
                            }
                        },
                    );
                }
                None => {}
            }
        }
    }
}

/// Delete several entries of one folder, one after another; failures do
/// not stop the rest.
fn delete_many(app: &mut App, tab: TabId, entries: Vec<SftpEntry>) {
    let Some((cid, _)) = pane_conn(app, tab) else {
        tracing::warn!(tab, "sftp: delete confirmed but the file browser is gone");
        app.toasts.warning("Not deleted", "The file browser is not open.".to_string());
        return;
    };
    tracing::info!(tab, n = entries.len(), "sftp: delete many");
    let total = entries.len();
    app.core.call(
        "sftp delete many",
        move |st| async move {
            let mut failed: Vec<String> = Vec::new();
            for e in entries {
                let r = if e.is_dir {
                    api::sftp::sftp_rmdir(&st, cid, e.full_path.clone()).await
                } else {
                    api::sftp::sftp_remove(&st, cid, e.full_path.clone()).await
                };
                if let Err(err) = r {
                    tracing::warn!(path = %e.full_path, "sftp: delete failed: {err}");
                    failed.push(format!("{}: {err}", e.name));
                }
            }
            failed
        },
        move |app, failed: Vec<String>| {
            let deleted = total - failed.len();
            match failed.first() {
                None => app.toasts.success("Deleted", format!("{deleted} items")),
                Some(first) => {
                    let more = if failed.len() > 1 { format!(" (and {} more)", failed.len() - 1) } else { String::new() };
                    app.toasts.danger(format!("Deleted {deleted} of {total}"), format!("{first}{more}"));
                }
            }
            refresh(app, tab);
        },
    );
}

/// A mkdir / rename finished: close and refresh, or show the error in the
/// prompt.
fn finish_prompt(app: &mut App, tab: TabId, serial: u64, r: Result<(), ezssh_core::AppError>, title: &str, body: String) {
    let ours = matches!(
        app.sftp.modal.as_ref(),
        Some(Modal::Mkdir { serial: s, .. } | Modal::Rename { serial: s, .. }) if *s == serial
    );
    match r {
        Ok(()) => {
            if ours {
                app.sftp.modal = None;
            }
            app.toasts.success(title, body);
            refresh(app, tab);
        }
        Err(e) => {
            tracing::warn!(tab, "sftp: {title}: {e}");
            match app.sftp.modal.as_mut() {
                Some(Modal::Mkdir { prompt, busy, .. } | Modal::Rename { prompt, busy, .. }) if ours => {
                    *busy = false;
                    prompt.set_error(e.to_string());
                }
                _ => app.toasts.danger("Failed", e.to_string()),
            }
        }
    }
}

/// Download `names` of the listed folder into `dir` (the tests cannot
/// drive the native folder dialog).
#[cfg(test)]
pub(crate) fn test_download(app: &mut App, tab: TabId, names: &[&str], dir: &Path) {
    let entries: Vec<SftpEntry> = app.sftp.panes[&tab].entries.iter().filter(|e| names.contains(&e.name.as_str())).cloned().collect();
    let items = entries.into_iter().map(|e| {
        let l = dir.join(local_name(&e.name));
        (e, l)
    });
    download(app, tab, items.collect());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> SftpEntry {
        SftpEntry { name: name.into(), full_path: format!("/d/{name}"), is_dir: false, is_symlink: false, size: 0, mtime_unix: 0, mode: 0 }
    }

    #[test]
    fn sizes_and_modes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024 / 2), "1.50 GB");
        assert_eq!(format_size(1024 * 1024 - 1), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024 - 1), "1.00 GB");
        assert_eq!(format_mode(0o100755), "rwxr-xr-x");
        assert_eq!(format_mode(0o640), "rw-r-----");
        assert_eq!(format_mode(0o40000), "---------");
    }

    #[test]
    fn remote_paths() {
        assert_eq!(join_remote("/", "a"), "/a");
        assert_eq!(join_remote("", "a"), "/a");
        assert_eq!(join_remote("/x/y/", "a"), "/x/y/a");
        assert_eq!(parent_of("/a"), "/");
        assert_eq!(parent_of("/x/y/a"), "/x/y");
        assert_eq!(parent_of("/"), "/");
        assert_eq!(invalid_name("a/b"), Some("A name cannot contain /"));
        assert_eq!(invalid_name(".."), Some("That name is reserved"));
        assert_eq!(invalid_name("ok.txt"), None);
    }

    #[test]
    fn local_names_stay_one_component() {
        assert_eq!(local_name("..\\..\\evil"), ".._.._evil");
        assert_eq!(local_name("a/b"), "a_b");
        assert_eq!(local_name(".."), "_");
        assert_eq!(local_name("tab\there"), "tab_here");
        assert_eq!(local_name("plain.txt"), "plain.txt");
        for n in ["..\\..\\evil", "a/b", "..", "x"] {
            let name = local_name(n);
            let c: Vec<_> = Path::new(&name).components().collect();
            assert!(matches!(c.as_slice(), [std::path::Component::Normal(_)]), "{n} → {c:?}");
        }
        if cfg!(windows) {
            assert_eq!(local_name("CON.txt"), "_CON.txt");
            assert_eq!(local_name("a:b?. "), "a_b_");
        }
    }

    #[test]
    fn selection_follows_file_manager_rules() {
        let es: Vec<SftpEntry> = ["a", "b", "c", "d"].map(entry).into();
        let (mut sel, mut anchor) = (HashSet::new(), None);
        let sorted = |s: &HashSet<String>| {
            let mut v: Vec<_> = s.iter().cloned().collect();
            v.sort();
            v
        };
        click_select(&mut sel, &mut anchor, &es, "/d/b", false, false);
        assert_eq!(sorted(&sel), ["/d/b"]);
        click_select(&mut sel, &mut anchor, &es, "/d/d", false, true);
        assert_eq!(sorted(&sel), ["/d/b", "/d/c", "/d/d"]);
        // A plain click inside the selection keeps it (drag carries all).
        click_select(&mut sel, &mut anchor, &es, "/d/c", false, false);
        assert_eq!(sel.len(), 3);
        click_select(&mut sel, &mut anchor, &es, "/d/c", true, false);
        assert_eq!(sorted(&sel), ["/d/b", "/d/d"]);
        click_select(&mut sel, &mut anchor, &es, "/d/a", true, false);
        assert_eq!(sorted(&sel), ["/d/a", "/d/b", "/d/d"]);
        click_select(&mut sel, &mut anchor, &es, "/d/c", false, false);
        assert_eq!(sorted(&sel), ["/d/c"]);
        // Shift from the anchor upwards.
        click_select(&mut sel, &mut anchor, &es, "/d/a", false, true);
        assert_eq!(sorted(&sel), ["/d/a", "/d/b", "/d/c"]);
    }

    #[test]
    fn walk_flattens_a_tree_parents_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::write(root.join("sub/b.txt"), "b").unwrap();
        std::fs::write(root.join("sub/deeper/c.txt"), "c").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("sub/loop")).unwrap();
        let mut w = Walk::default();
        walk(&root, "/up/tree", &mut w);
        assert_eq!(w.dirs, ["/up/tree/sub", "/up/tree/sub/deeper"]);
        let remote: Vec<&str> = w.files.iter().map(|(_, r)| r.as_str()).collect();
        assert_eq!(remote, ["/up/tree/a.txt", "/up/tree/sub/b.txt", "/up/tree/sub/deeper/c.txt"]);
        #[cfg(unix)]
        assert!(w.failed.len() == 1 && w.failed[0].contains("folder symlink skipped"), "{:?}", w.failed);
    }
}
