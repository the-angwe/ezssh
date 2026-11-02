//! Terminal connections: the glue between a tab's [`Terminal`] and the
//! core's SSH / local PTY connections, and the connect state machine of
//! `ui/components/terminal.tsx`.
//!
//! - Output: the core's reader task feeds the terminal directly through
//!   [`FeedSink`]. Every connect attempt has a generation number; a sink
//!   whose generation is no longer current drops its bytes (the webview's
//!   `detach()`), so a late chunk from a torn-down connection never lands
//!   in the reconnected one.
//! - Input: [`TabPty`] is the terminal's `Pty`. Keystrokes, query answers
//!   and resizes go into a per-connection queue drained by one writer task
//!   in order. The queue exists before the connect call, so a query answer
//!   produced while the handshake is still running (bash asks for the
//!   cursor position during prompt setup) waits for the connection id
//!   instead of being lost. The terminal outlives connections: a reconnect
//!   attaches a new queue and keeps the scrollback, as xterm.js did.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ezssh_core::{AppError, AppState, CloseReason, OutputSink, api};
use ezssh_term::{Feeder, Pty, Terminal, WindowSize};
use tokio::sync::{mpsc, oneshot};

use crate::app::App;
use crate::tabs::{TabId, TabStatus};

/// Which command family a session kind uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Ssh,
    /// WSL distros and local shells.
    Local,
}

impl Family {
    pub fn of(session_kind: &str) -> Self {
        if session_kind == "ssh" { Family::Ssh } else { Family::Local }
    }
}

enum PtyMsg {
    Write(Vec<u8>),
    Resize(u16, u16),
}

struct PtyInner {
    tx: Option<mpsc::UnboundedSender<PtyMsg>>,
    size: WindowSize,
}

/// The terminal's way out. See the module docs.
pub struct TabPty {
    inner: Mutex<PtyInner>,
}

impl TabPty {
    fn new() -> Self {
        Self { inner: Mutex::new(PtyInner { tx: None, size: WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 } }) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut PtyInner) -> R) -> Option<R> {
        match self.inner.lock() {
            Ok(mut g) => Some(f(&mut g)),
            Err(e) => {
                tracing::error!("conn.pty: state poisoned: {e}");
                None
            }
        }
    }

    fn attach(&self, tx: mpsc::UnboundedSender<PtyMsg>) {
        self.with(|i| i.tx = Some(tx));
    }

    fn detach(&self) {
        self.with(|i| i.tx = None);
    }

    /// Grid size last reported by the terminal (cols, rows).
    pub fn size(&self) -> (u16, u16) {
        self.with(|i| (i.size.num_cols, i.size.num_lines)).unwrap_or((80, 24))
    }
}

impl Pty for TabPty {
    fn write(&self, bytes: Cow<'static, [u8]>) {
        self.with(|i| match &i.tx {
            Some(tx) => {
                if tx.send(PtyMsg::Write(bytes.into_owned())).is_err() {
                    tracing::debug!("conn.pty: writer gone, input dropped");
                }
            }
            None => tracing::trace!(len = bytes.len(), "conn.pty: no connection, input dropped"),
        });
    }

    fn resize(&self, size: WindowSize) {
        self.with(|i| {
            i.size = size;
            if let Some(tx) = &i.tx
                && tx.send(PtyMsg::Resize(size.num_cols, size.num_lines)).is_err()
            {
                tracing::debug!("conn.pty: writer gone, resize dropped");
            }
        });
    }
}

/// Output of one connect attempt into the tab's terminal.
struct FeedSink {
    feeder: Feeder,
    generation: u64,
    current: Arc<AtomicU64>,
}

impl OutputSink for FeedSink {
    fn send(&self, bytes: &[u8]) -> bool {
        if self.current.load(Ordering::Acquire) != self.generation {
            return false;
        }
        self.feeder.feed(bytes);
        true
    }
}

/// Fingerprint prompt after `host_key_untrusted` / `host_key_mismatch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub mismatch: bool,
    pub fingerprint: String,
    pub expected: Option<String>,
}

/// A terminal and its connection plumbing; one per tab.
pub struct TermSlot {
    pub term: Terminal,
    pub pty: Arc<TabPty>,
    feeder: Feeder,
    generation: Arc<AtomicU64>,
    /// Connect once the terminal has been laid out (so the remote starts
    /// at the right size), or at this deadline if it never is.
    pub connect_at: Option<Instant>,
    pub host_key: Option<HostKeyPrompt>,
    /// Login failed: the auth-fix overlay over this tab.
    pub auth_fix: Option<crate::views::auth_fix::AuthFix>,
    /// Bell flash / font picker etc. live in the widget; this is only the
    /// "take keyboard focus next frame" request.
    pub focus: bool,
}

/// How long a new tab waits for its first layout before connecting at
/// 80×24 anyway (a tab opened into a hidden slot).
const FIRST_LAYOUT_WAIT: Duration = Duration::from_millis(300);

impl TermSlot {
    pub fn new(ctx: &egui::Context, options: ezssh_term::TermOptions, palette: ezssh_term::Palette) -> Self {
        let pty = Arc::new(TabPty::new());
        let (term, feeder) = Terminal::new(ctx, pty.clone(), options, palette);
        Self {
            term,
            pty,
            feeder,
            generation: Arc::new(AtomicU64::new(0)),
            connect_at: Some(Instant::now() + FIRST_LAYOUT_WAIT),
            host_key: None,
            auth_fix: None,
            focus: true,
        }
    }

    /// Drop this tab's side of the current connection: later output is
    /// ignored and input has nowhere to go until the next connect.
    fn detach(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.pty.detach();
    }

    /// Ready to connect: laid out, or waited long enough.
    pub fn connect_due(&self, now: Instant) -> bool {
        match self.connect_at {
            Some(at) => self.term.grid_size().0 > 0 || now >= at,
            None => false,
        }
    }
}

/// Sends queued input / resizes to the connection, in order, once its id
/// is known. Ends when the tab detaches (sender dropped) or the connect
/// failed (id sender dropped).
async fn writer(state: Arc<AppState>, family: Family, mut rx: mpsc::UnboundedReceiver<PtyMsg>, id: oneshot::Receiver<u64>) {
    let Ok(id) = id.await else {
        tracing::debug!("conn.writer: connect did not complete, queued input dropped");
        return;
    };
    tracing::debug!(connection_id = id, ?family, "conn.writer: started");
    while let Some(msg) = rx.recv().await {
        let result = match (msg, family) {
            (PtyMsg::Write(b), Family::Ssh) => api::ssh::ssh_write(&state, id, b).await,
            (PtyMsg::Write(b), Family::Local) => api::local::local_write(&state, id, b).await,
            (PtyMsg::Resize(c, r), Family::Ssh) => api::ssh::ssh_resize(&state, id, c, r).await,
            (PtyMsg::Resize(c, r), Family::Local) => api::local::local_resize(&state, id, c, r).await,
        };
        if let Err(e) = result {
            tracing::warn!(connection_id = id, "conn.writer: {e}");
        }
    }
    tracing::debug!(connection_id = id, "conn.writer: ended");
}

/// What a failed connect turns into.
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    HostKey(HostKeyPrompt),
    /// X11 forwarding is on but VcXsrv is missing: the install dialog.
    XServerMissing(String),
    /// Bad credentials: the auth-fix overlay.
    Auth(String),
    Other(String),
}

/// The webview's `isAuthError`: anything the auth-fix flow can help with.
fn is_auth_error(e: &AppError) -> bool {
    match e {
        AppError::AuthFailed => true,
        AppError::Validation(m) if m.to_lowercase().contains("credential") => true,
        other => {
            let m = other.to_string().to_lowercase();
            ["authentication failed", "key parse", "could not read key", "no authentication methods", "no more authentication"]
                .iter()
                .any(|p| m.contains(p))
        }
    }
}

pub fn classify(e: &AppError, family: Family) -> ConnectFailure {
    match (e, family) {
        (AppError::HostKeyUntrusted { actual }, Family::Ssh) => {
            ConnectFailure::HostKey(HostKeyPrompt { mismatch: false, fingerprint: actual.clone(), expected: None })
        }
        (AppError::HostKeyMismatch { expected, actual }, Family::Ssh) => {
            ConnectFailure::HostKey(HostKeyPrompt { mismatch: true, fingerprint: actual.clone(), expected: Some(expected.clone()) })
        }
        (AppError::XServerMissing, Family::Ssh) => ConnectFailure::XServerMissing(e.to_string()),
        (e, Family::Ssh) if is_auth_error(e) => ConnectFailure::Auth(e.to_string()),
        (e, _) => ConnectFailure::Other(e.to_string()),
    }
}

/// Text of a close event, as the webview showed it: non-zero exit status
/// and signals are worth showing; a clean end reads as a plain disconnect.
pub fn close_text(reason: &CloseReason) -> Option<String> {
    match reason {
        CloseReason::Exit(0) | CloseReason::Ended => None,
        CloseReason::Exit(n) => Some(format!("exit {n}")),
        CloseReason::Signal(s) => Some(format!("killed by SIG{s}")),
    }
}

impl App {
    /// Start (or restart) the connection of `tab`. Replaces any current one.
    pub(crate) fn connect_tab(&mut self, tab: TabId, trust_any: bool, disable_x11: bool) {
        let Some(t) = self.tabs.get(tab) else {
            tracing::warn!(tab, "conn: connect for a closed tab");
            return;
        };
        let session_id = t.session.id;
        let family = Family::of(&t.session.session_kind);
        self.teardown(tab, true);
        let Some(slot) = self.terms.get_mut(&tab) else {
            tracing::error!(tab, "conn: tab has no terminal");
            return;
        };
        slot.connect_at = None;
        slot.host_key = None;
        slot.auth_fix = None;
        let generation = slot.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (tx, rx) = mpsc::unbounded_channel();
        let (id_tx, id_rx) = oneshot::channel();
        slot.pty.attach(tx);
        let (cols, rows) = match slot.term.grid_size() {
            (0, _) | (_, 0) => slot.pty.size(),
            (c, r) => (c as u16, r as u16),
        };
        let sink: ezssh_core::Output =
            Box::new(FeedSink { feeder: slot.feeder.clone(), generation, current: slot.generation.clone() });
        self.tabs.set_status(tab, TabStatus::Connecting, None);
        tracing::info!(tab, session_id, ?family, cols, rows, trust_any, disable_x11, "conn: connect");

        let state = self.core.state.clone();
        self.core.spawn(writer(state, family, rx, id_rx));
        self.core.call(
            "connect",
            move |st| async move {
                match family {
                    Family::Ssh => api::ssh::ssh_connect(&st, session_id, cols, rows, trust_any, Some(disable_x11), sink)
                        .await
                        .map(|r| r.connection_id),
                    Family::Local => api::local::local_connect(&st, session_id, cols, rows, sink).await.map(|r| r.connection_id),
                }
            },
            move |app, result| app.connect_done(tab, generation, family, result, id_tx),
        );
    }

    fn connect_done(&mut self, tab: TabId, generation: u64, family: Family, result: Result<u64, AppError>, id_tx: oneshot::Sender<u64>) {
        let current = self.terms.get(&tab).map(|s| s.generation.load(Ordering::Acquire));
        if current != Some(generation) {
            // Tab closed or reconnected meanwhile: this connection is orphaned.
            if let Ok(id) = result {
                tracing::debug!(tab, connection_id = id, "conn: stale connect result, disconnecting");
                self.disconnect(family, id);
            }
            return;
        }
        match result {
            Ok(id) => {
                tracing::info!(tab, connection_id = id, "conn: connected");
                if id_tx.send(id).is_err() {
                    tracing::warn!(tab, connection_id = id, "conn: writer gone before connect finished");
                }
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.connection_id = Some(id);
                    if self.settings.bool("sessions.autoOpenSftp") && t.has_file_pane() {
                        t.sftp_open = true;
                    }
                }
                self.tabs.set_status(tab, TabStatus::Connected, None);
                if let Some(slot) = self.terms.get_mut(&tab) {
                    slot.focus = true;
                }
            }
            Err(e) => {
                tracing::warn!(tab, "conn: connect failed: {e}");
                drop(id_tx);
                if let Some(slot) = self.terms.get(&tab) {
                    slot.detach();
                }
                match classify(&e, family) {
                    ConnectFailure::HostKey(prompt) => {
                        if let Some(slot) = self.terms.get_mut(&tab) {
                            slot.host_key = Some(prompt);
                        }
                    }
                    ConnectFailure::XServerMissing(msg) => {
                        self.tabs.set_status(
                            tab,
                            TabStatus::Error,
                            Some(format!("{msg}. Install VcXsrv or turn off X11 forwarding for this session.")),
                        );
                        match self.dialogs.xserver.as_mut() {
                            Some(open) => open.add(tab),
                            None => self.dialogs.xserver = Some(crate::views::xserver::XServerDialog::new(tab, msg)),
                        }
                    }
                    ConnectFailure::Auth(msg) => {
                        self.tabs.set_status(tab, TabStatus::Error, Some(msg.clone()));
                        let session = self.tabs.get(tab).map(|t| t.session.clone());
                        if let (Some(session), Some(slot)) = (session, self.terms.get_mut(&tab)) {
                            slot.auth_fix = Some(crate::views::auth_fix::AuthFix::new(tab, session, msg));
                        }
                    }
                    ConnectFailure::Other(msg) => self.tabs.set_status(tab, TabStatus::Error, Some(msg)),
                }
            }
        }
    }

    /// Forget the tab's connection; with `disconnect`, also end it in the
    /// core (not needed when the core reported the close itself).
    pub(crate) fn teardown(&mut self, tab: TabId, disconnect: bool) {
        if let Some(slot) = self.terms.get(&tab) {
            slot.detach();
        }
        let Some(t) = self.tabs.get_mut(tab) else {
            return;
        };
        let family = Family::of(&t.session.session_kind);
        if let Some(id) = t.connection_id.take()
            && disconnect
        {
            tracing::debug!(tab, connection_id = id, "conn: disconnect");
            self.disconnect(family, id);
        }
    }

    fn disconnect(&self, family: Family, id: u64) {
        match family {
            Family::Ssh => self.core.fire("ssh_disconnect", move |st| async move { api::ssh::ssh_disconnect(&st, id).await }),
            Family::Local => self.core.fire("local_disconnect", move |st| async move { api::local::local_disconnect(&st, id).await }),
        }
    }

    /// A core event for a terminal connection.
    pub(crate) fn on_connection_event(&mut self, event: ezssh_core::Event) {
        use ezssh_core::Event;
        match event {
            Event::Closed { connection_id, reason } => {
                let Some(tab) = self.tabs.by_connection(connection_id) else {
                    tracing::debug!(connection_id, ?reason, "conn: close for an unknown connection");
                    return;
                };
                tracing::info!(tab, connection_id, ?reason, "conn: closed");
                self.teardown(tab, false);
                let text = close_text(&reason);
                let errored = self.tabs.get(tab).is_some_and(|t| t.status == TabStatus::Error);
                // An error already reported for this connection keeps its message.
                if text.is_none() && errored {
                    return;
                }
                self.tabs.set_status(tab, TabStatus::Closed, text);
            }
            Event::WriteFailed { connection_id, message } => {
                if let Some(tab) = self.tabs.by_connection(connection_id) {
                    self.tabs.set_status(tab, TabStatus::Error, Some(message));
                }
            }
            ev @ Event::ForwardStatus { .. } => crate::views::forwards::on_event(self, ev),
            Event::Transfer(p) => crate::views::sftp::on_event(self, p),
            Event::UpdateProgress { downloaded, total } => crate::views::update::on_progress(self, downloaded, total),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_text_matches_the_webview() {
        assert_eq!(close_text(&CloseReason::Exit(0)), None);
        assert_eq!(close_text(&CloseReason::Ended), None);
        assert_eq!(close_text(&CloseReason::Exit(130)).as_deref(), Some("exit 130"));
        assert_eq!(close_text(&CloseReason::Signal("KILL".into())).as_deref(), Some("killed by SIGKILL"));
    }

    #[test]
    fn classify_routes_errors() {
        let hk = classify(&AppError::HostKeyUntrusted { actual: "fp".into() }, Family::Ssh);
        assert_eq!(hk, ConnectFailure::HostKey(HostKeyPrompt { mismatch: false, fingerprint: "fp".into(), expected: None }));
        let mm = classify(&AppError::HostKeyMismatch { expected: "a".into(), actual: "b".into() }, Family::Ssh);
        assert_eq!(mm, ConnectFailure::HostKey(HostKeyPrompt { mismatch: true, fingerprint: "b".into(), expected: Some("a".into()) }));
        assert!(matches!(classify(&AppError::AuthFailed, Family::Ssh), ConnectFailure::Auth(_)));
        assert!(matches!(classify(&AppError::Validation("missing credential".into()), Family::Ssh), ConnectFailure::Auth(_)));
        assert!(matches!(classify(&AppError::Ssh("No more authentication methods".into()), Family::Ssh), ConnectFailure::Auth(_)));
        assert!(matches!(classify(&AppError::XServerMissing, Family::Ssh), ConnectFailure::XServerMissing(_)));
        assert!(matches!(classify(&AppError::Ssh("connect timeout".into()), Family::Ssh), ConnectFailure::Other(_)));
        // Local kinds never get SSH-specific routing.
        assert!(matches!(classify(&AppError::AuthFailed, Family::Local), ConnectFailure::Other(_)));
    }

    #[test]
    fn pty_queues_until_attached_and_drops_after_detach() {
        let pty = TabPty::new();
        pty.write(Cow::Borrowed(b"lost"));
        let (tx, mut rx) = mpsc::unbounded_channel();
        pty.attach(tx);
        pty.write(Cow::Borrowed(b"a"));
        pty.resize(WindowSize { num_lines: 40, num_cols: 120, cell_width: 9, cell_height: 18 });
        pty.write(Cow::Borrowed(b"b"));
        assert_eq!(pty.size(), (120, 40));
        let got: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|m| match m {
                PtyMsg::Write(b) => String::from_utf8(b).unwrap(),
                PtyMsg::Resize(c, r) => format!("{c}x{r}"),
            })
            .collect();
        assert_eq!(got, ["a", "120x40", "b"], "in order, nothing from before attach");
        pty.detach();
        pty.write(Cow::Borrowed(b"c"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn stale_generation_output_is_dropped() {
        let ctx = egui::Context::default();
        let slot = TermSlot::new(&ctx, ezssh_term::TermOptions::default(), ezssh_term::Palette::default());
        let current = slot.generation.clone();
        let sink = FeedSink { feeder: slot.feeder.clone(), generation: 0, current: current.clone() };
        assert!(sink.send(b"hello"));
        slot.detach();
        assert!(!sink.send(b"late"), "after detach the old sink is stale");
    }
}
