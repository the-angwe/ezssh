//! The session editor: New / Edit session, a 760 px dialog with a side
//! rail (General, Terminal, Advanced, Forwards for SSH). Esc closes,
//! Ctrl+Enter saves.
//!
//! Passwords belong to their session: the Password method shows a
//! password field instead of a credential picker. A new password session
//! needs one; when editing, a blank field keeps the stored password. Keys
//! and key passphrases stay shared and use [`CredPicker`].

use std::sync::atomic::{AtomicU64, Ordering};

use egui::text::LayoutJob;
use egui::{Align, Color32, CursorIcon, FontId, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind, TextFormat, Ui, UiBuilder, Vec2, pos2, vec2};
use ezssh_core::api;
use ezssh_core::db::sessions::{EnvPair, Session, SessionInput};
use ezssh_core::local::shells::DetectedShell;
use ezssh_core::Secret;
use zeroize::Zeroizing;

use super::cred_picker::CredPicker;
use crate::form::labelled;
use crate::app::App;
use crate::font_presets;
use crate::form::{self, InputOpts};
use crate::icons::Icon;
use crate::settings::Settings;
use crate::style::{alpha, font, radius, size, tokens};
use crate::widgets::{self, Segment};

pub enum Mode {
    New { folder: Option<i64> },
    Edit(Box<Session>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    General,
    Terminal,
    Advanced,
    Forwards,
}

/// The form values (`SessionInput` without the password).
#[derive(Clone, Debug)]
struct Draft {
    folder_id: Option<i64>,
    name: String,
    host: String,
    port: i64,
    username: String,
    auth_type: String,
    credential_id: Option<i64>,
    key_passphrase_credential_id: Option<i64>,
    color: Option<String>,
    initial_command: String,
    scrollback_lines: i64,
    font_size: i64,
    font_family: String,
    cursor_style: String,
    compression: bool,
    keepalive_secs: i64,
    connect_timeout_secs: i64,
    env: Vec<EnvPair>,
    session_kind: String,
    forward_x11: bool,
    starting_dir: String,
}

impl Draft {
    /// A new session: display and connection defaults from Settings
    /// (`makeDefaults`).
    fn new_session(folder_id: Option<i64>, s: &Settings) -> Self {
        Self {
            folder_id,
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: "agent".into(),
            credential_id: None,
            key_passphrase_credential_id: None,
            color: None,
            initial_command: String::new(),
            scrollback_lines: s.int("terminal.scrollback"),
            font_size: s.int("terminal.fontSize"),
            font_family: s.str("terminal.fontFamily").to_string(),
            cursor_style: s.str("terminal.cursorStyle").to_string(),
            compression: false,
            keepalive_secs: s.int("sessions.keepaliveSecs"),
            connect_timeout_secs: s.int("sessions.connectTimeoutSecs"),
            env: Vec::new(),
            session_kind: "ssh".into(),
            forward_x11: false,
            starting_dir: String::new(),
        }
    }

    fn from_session(s: &Session) -> Self {
        Self {
            folder_id: s.folder_id,
            name: s.name.clone(),
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            auth_type: s.auth_type.clone(),
            // A password session's credential is its own row, managed by
            // the core; only key sessions pick one.
            credential_id: if s.auth_type == "password" { None } else { s.credential_id },
            key_passphrase_credential_id: s.key_passphrase_credential_id,
            color: s.color.clone(),
            initial_command: s.initial_command.clone().unwrap_or_default(),
            scrollback_lines: s.scrollback_lines,
            font_size: s.font_size,
            font_family: s.font_family.clone(),
            cursor_style: s.cursor_style.clone(),
            compression: s.compression == 1,
            keepalive_secs: s.keepalive_secs,
            connect_timeout_secs: s.connect_timeout_secs,
            env: Vec::new(), // loaded on open
            session_kind: s.session_kind.clone(),
            forward_x11: s.forward_x11 == 1,
            starting_dir: s.starting_dir.clone().unwrap_or_default(),
        }
    }

    fn is_ssh(&self) -> bool {
        self.session_kind == "ssh"
    }

    /// Header subtitle: `user@host:port` for SSH, the
    /// distro / program for WSL and local shells.
    fn summary(&self) -> String {
        let or = |s: &str, d: &str| if s.is_empty() { d.to_string() } else { s.to_string() };
        match self.session_kind.as_str() {
            "wsl" if self.username.is_empty() => or(&self.host, "(default distro)"),
            "wsl" => format!("{}@{}", self.username, or(&self.host, "(default distro)")),
            "local" => or(&self.host, "(shell)"),
            _ => format!("{}@{}:{}", or(&self.username, "user"), or(&self.host, "host"), if self.port == 0 { 22 } else { self.port }),
        }
    }

    /// The save payload, cleaned: blank initial command
    /// and starting dir become NULL, blank env rows are dropped.
    fn to_input(&self, password: Option<Secret>) -> SessionInput {
        let ssh = self.is_ssh();
        let pw_auth = ssh && self.auth_type == "password";
        SessionInput {
            folder_id: self.folder_id,
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            auth_type: if ssh { self.auth_type.clone() } else { "agent".into() },
            // The core manages a password session's own row.
            credential_id: if ssh && self.auth_type == "key" { self.credential_id } else { None },
            key_passphrase_credential_id: if ssh && self.auth_type == "key" { self.key_passphrase_credential_id } else { None },
            color: self.color.clone(),
            initial_command: (!self.initial_command.trim().is_empty()).then(|| self.initial_command.clone()),
            scrollback_lines: self.scrollback_lines,
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            cursor_style: self.cursor_style.clone(),
            compression: i64::from(self.compression),
            keepalive_secs: self.keepalive_secs,
            connect_timeout_secs: self.connect_timeout_secs,
            env: self.env.iter().filter(|p| !p.key.trim().is_empty()).cloned().collect(),
            session_kind: self.session_kind.clone(),
            // The checkbox is SSH-only; the core rejects it elsewhere.
            forward_x11: i64::from(ssh && self.forward_x11),
            starting_dir: Some(self.starting_dir.trim().to_string()).filter(|s| !s.is_empty()),
            password: if pw_auth { password.filter(|p| !p.is_empty()) } else { None },
        }
    }
}

/// The first problem with the draft, if any. A password is required
/// unless one is stored already.
fn validate(d: &Draft, password_typed: bool, password_stored: bool) -> Option<&'static str> {
    if d.name.trim().is_empty() {
        return Some("Name is required");
    }
    if d.is_ssh() {
        if d.host.trim().is_empty() {
            return Some("Host is required");
        }
        if d.username.trim().is_empty() {
            return Some("Username is required");
        }
        if !(1..=65535).contains(&d.port) {
            return Some("Port must be between 1 and 65535");
        }
        if d.auth_type == "password" && !password_typed && !password_stored {
            return Some("Password is required");
        }
        if d.auth_type == "key" && d.credential_id.is_none() {
            return Some("Credential is required for this auth type");
        }
    }
    if !(0..=100_000).contains(&d.scrollback_lines) {
        return Some("Scrollback must be between 0 and 100000");
    }
    if !(font_presets::MIN_FONT_SIZE..=font_presets::MAX_FONT_SIZE).contains(&d.font_size) {
        return Some("Font size must be between 8 and 48");
    }
    if !(0..=7200).contains(&d.keepalive_secs) {
        return Some("Keepalive must be between 0 and 7200 seconds");
    }
    if !(1..=600).contains(&d.connect_timeout_secs) {
        return Some("Connect timeout must be between 1 and 600 seconds");
    }
    for p in &d.env {
        if p.key.trim().is_empty() {
            return Some("Env var name cannot be empty");
        }
        if p.key.contains('=') || p.key.contains('\0') {
            return Some("Env var name cannot contain \"=\" or NUL");
        }
    }
    None
}

/// Tells async results which dialog instance they belong to.
static SERIAL: AtomicU64 = AtomicU64::new(1);

pub struct SessionDialog {
    pub mode: Mode,
    serial: u64,
    /// Settings defaults, shells and env are fetched on the first frame.
    initialized: bool,
    /// The env vars of the edited row arrived (`session_update` replaces
    /// the whole env table; saving before that would delete them).
    env_loaded: bool,
    draft: Draft,
    /// The typed password (password method only); wiped on drop.
    password: Zeroizing<String>,
    show_password: bool,
    pane: Pane,
    err: Option<String>,
    busy: bool,
    /// Detected local shells; None while loading.
    shells: Option<Vec<DetectedShell>>,
    /// The font family picker shows the free-text field.
    font_custom: bool,
    key_picker: CredPicker,
    pass_picker: CredPicker,
    focus_name: bool,
}

impl SessionDialog {
    fn with_mode(mode: Mode) -> Self {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let draft = match &mode {
            Mode::New { folder } => Draft::new_session(*folder, &Settings::default()),
            Mode::Edit(s) => Draft::from_session(s),
        };
        let env_loaded = !matches!(mode, Mode::Edit(_));
        Self {
            mode,
            serial,
            initialized: false,
            env_loaded,
            draft,
            password: Zeroizing::new(String::new()),
            show_password: false,
            pane: Pane::General,
            err: None,
            busy: false,
            shells: None,
            font_custom: false,
            key_picker: CredPicker::new("private_key", ("session", serial)),
            pass_picker: CredPicker::new("key_passphrase", ("session", serial)),
            focus_name: true,
        }
    }

    pub fn new_session(folder: Option<i64>) -> Self {
        Self::with_mode(Mode::New { folder })
    }

    pub fn edit(session: Session) -> Self {
        Self::with_mode(Mode::Edit(Box::new(session)))
    }

    fn edit_id(&self) -> Option<i64> {
        match &self.mode {
            Mode::Edit(s) => Some(s.id),
            Mode::New { .. } => None,
        }
    }

    /// The session being edited has a password saved with it.
    fn password_stored(&self) -> bool {
        matches!(&self.mode, Mode::Edit(s) if s.auth_type == "password" && s.credential_id.is_some())
    }

    /// First frame: Settings defaults, then the shell list and (edit) the
    /// env vars, which live in their own table.
    fn init(&mut self, app: &mut App) {
        self.initialized = true;
        if let Mode::New { folder } = &self.mode {
            self.draft = Draft::new_session(*folder, &app.settings);
        }
        self.font_custom = !font_presets::is_preset(&self.draft.font_family) && !self.draft.font_family.is_empty();
        tracing::debug!(serial = self.serial, edit = ?self.edit_id(), "session_dialog: open");
        let serial = self.serial;
        app.core.call(
            "list_local_shells",
            |st| async move { api::local::list_local_shells(&st).await },
            move |app, r| {
                let shells = match r {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("session_dialog: list_local_shells failed: {e}");
                        app.toasts.danger("Could not list local shells", e.to_string());
                        Vec::new()
                    }
                };
                if let Some(d) = this(app, serial) {
                    tracing::debug!(n = shells.len(), "session_dialog: shells detected");
                    d.shells = Some(shells);
                }
            },
        );
        if let Some(id) = self.edit_id() {
            app.core.call(
                "session_env_get",
                move |st| async move { api::sessions::session_env_get(&st, id).await },
                move |app, r| {
                    let Some(d) = this(app, serial) else {
                        return;
                    };
                    match r {
                        Ok(env) => {
                            tracing::debug!(session = id, n = env.len(), "session_dialog: env loaded");
                            d.draft.env = env;
                            d.env_loaded = true;
                        }
                        Err(e) => {
                            tracing::warn!(session = id, "session_dialog: session_env_get failed: {e}");
                            d.err = Some(format!("Could not load environment variables: {e}"));
                        }
                    }
                },
            );
        }
    }

    fn switch_kind(&mut self, next: &str) {
        let d = &mut self.draft;
        if d.session_kind == next {
            return;
        }
        tracing::debug!(from = %d.session_kind, to = next, "session_dialog: kind");
        // Reset kind-specific fields so stale values don't leak across kinds.
        d.host = if next == "local" { self.shells.as_ref().and_then(|s| s.first()).map(|s| s.program.clone()).unwrap_or_default() } else { String::new() };
        d.port = if next == "ssh" { if d.port == 0 { 22 } else { d.port } } else { 0 };
        d.username.clear();
        if next != "ssh" {
            d.auth_type = "agent".into();
            d.credential_id = None;
            d.key_passphrase_credential_id = None;
            if self.pane == Pane::Forwards {
                self.pane = Pane::General;
            }
        }
        d.session_kind = next.to_string();
    }

    fn switch_auth(&mut self, next: &str) {
        tracing::debug!(from = %self.draft.auth_type, to = next, "session_dialog: auth method");
        self.draft.auth_type = next.to_string();
        self.draft.credential_id = None;
        self.draft.key_passphrase_credential_id = None;
    }

    fn save(&mut self, app: &mut App) {
        if self.busy {
            return;
        }
        let d = &self.draft;
        if let Some(problem) = validate(d, !self.password.is_empty(), self.password_stored()) {
            tracing::debug!(problem, "session_dialog: validation failed");
            self.err = Some(problem.into());
            return;
        }
        if !self.env_loaded {
            tracing::debug!("session_dialog: save refused, env vars not loaded");
            self.err = Some("Environment variables are still loading; try again in a moment.".into());
            return;
        }
        self.err = None;
        self.busy = true;
        let password = (!self.password.is_empty()).then(|| Secret::new(String::clone(&self.password)));
        let input = d.to_input(password);
        let edit_id = self.edit_id();
        let serial = self.serial;
        tracing::debug!(
            edit = ?edit_id,
            kind = %input.session_kind,
            auth = %input.auth_type,
            new_password = input.password.is_some(),
            "session_dialog: saving"
        );
        app.core.call(
            "session_save",
            move |st| async move {
                match edit_id {
                    Some(id) => api::sessions::session_update(&st, id, input).await,
                    None => api::sessions::session_create(&st, input).await,
                }
            },
            move |app, r| match r {
                Ok(s) => {
                    tracing::info!(session = s.id, edit = edit_id.is_some(), "session_dialog: saved");
                    if edit_id.is_some() {
                        app.apply_session_terminal(&s);
                    }
                    if this(app, serial).is_some() {
                        app.dialogs.session = None;
                    }
                    app.sidebar.reload(&app.core);
                    app.toasts.success(if edit_id.is_some() { "Session saved" } else { "Session created" }, None);
                }
                Err(e) => {
                    tracing::warn!(edit = ?edit_id, "session_dialog: save failed: {e}");
                    match this(app, serial) {
                        Some(d) => {
                            d.busy = false;
                            d.err = Some(e.to_string());
                        }
                        None => app.toasts.danger("Session not saved", e.to_string()),
                    }
                }
            },
        );
    }
}

/// Read-outs for the headless tests.
#[cfg(test)]
impl SessionDialog {
    pub(crate) fn shells_loaded(&self) -> bool {
        self.shells.is_some()
    }

    pub(crate) fn key_credential(&self) -> Option<i64> {
        self.draft.credential_id
    }
}

/// The open dialog, if it is still instance `serial`.
fn this(app: &mut App, serial: u64) -> Option<&mut SessionDialog> {
    app.dialogs.session.as_mut().filter(|d| d.serial == serial)
}

const W: f32 = 760.0;
const H: f32 = 640.0;
const HEADER_H: f32 = widgets::DIALOG_HEAD_H;
const FOOTER_H: f32 = widgets::DIALOG_FOOTER_H;
const ERR_H: f32 = 36.0;
const NAV_W: f32 = 176.0;
const PAD: f32 = 24.0;

/// Show `app.dialogs.session` (if any) for one frame.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.session.take() else {
        return;
    };
    if !dlg.initialized {
        dlg.init(app);
    }
    let t = tokens(ctx);
    let w = W.min(ctx.content_rect().width() - 32.0);
    let h = H.min(ctx.content_rect().height() * 0.9);
    // Ctrl+Enter belongs to a dialog stacked on top (credentials manager,
    // forward dialog).
    let manager_open = app.dialogs.credentials.is_some() || app.forwards.dialog_open();
    let mut close = false;
    let mut save = false;
    let id = Id::unique("session-dialog");
    let resp = egui::Modal::new(id).backdrop_color(Color32::from_black_alpha(128)).frame(widgets::dialog_frame(t)).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let header = Rect::from_min_size(rect.min, vec2(w, HEADER_H));
        let footer = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOTER_H), rect.max);
        let err_rect = Rect::from_min_max(pos2(rect.left(), footer.top() - if dlg.err.is_some() { ERR_H } else { 0.0 }), pos2(rect.right(), footer.top()));
        let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), pos2(rect.right(), err_rect.top()));
        let nav = Rect::from_min_max(body.min, pos2(body.left() + NAV_W, body.bottom()));
        let content = Rect::from_min_max(pos2(nav.right(), body.top()), body.max);

        // Header: the kind's icon on a tile in the session colour, the
        // name and the live summary.
        let accent = dlg.draft.color.as_deref().and_then(parse_hex).unwrap_or(t.accent);
        let kind_icon = match dlg.draft.session_kind.as_str() {
            "wsl" => Icon::SquareTerminal,
            "local" => Icon::MonitorDot,
            _ => Icon::Server,
        };
        let title = if dlg.draft.name.trim().is_empty() {
            if dlg.edit_id().is_some() { "Edit session".to_string() } else { "New session".to_string() }
        } else {
            dlg.draft.name.clone()
        };
        let summary = dlg.draft.summary();
        if widgets::dialog_header(ui, id, header, Some(kind_icon), accent, &title, Some(&summary), true, dlg.busy) {
            close = true;
        }
        // Side rail.
        ui.painter().vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, t.border));
        at(ui, nav.shrink2(vec2(8.0, 12.0)), |ui| {
            let mut panes = vec![
                (Pane::General, "General", Icon::Sliders),
                (Pane::Terminal, "Terminal", Icon::Terminal),
                (Pane::Advanced, "Advanced", Icon::Cog),
            ];
            if dlg.draft.is_ssh() {
                panes.push((Pane::Forwards, "Forwards", Icon::Network));
            }
            widgets::nav_rail(ui, NAV_W - 16.0, &panes, &mut dlg.pane);
        });
        // Content.
        at(ui, content, |ui| {
            egui::ScrollArea::vertical().id_salt(("session-dialog-pane", dlg.serial, dlg.pane as u8)).auto_shrink(false).show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: PAD as i8, right: PAD as i8, top: 20, bottom: PAD as i8 }).show(ui, |ui| {
                    ui.set_width(content.width() - 2.0 * PAD);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match dlg.pane {
                        Pane::General => general(&mut dlg, app, ui),
                        Pane::Terminal => terminal(&mut dlg, ui),
                        Pane::Advanced => advanced(&mut dlg, ui),
                        Pane::Forwards => crate::views::forwards::session_tab(app, ui, dlg.edit_id()),
                    }
                });
            });
        });
        if let Some(e) = &dlg.err {
            ui.painter().rect_filled(err_rect, 0.0, alpha(t.danger, 0.1));
            ui.painter().hline(err_rect.x_range(), err_rect.top() + 0.5, Stroke::new(1.0, alpha(t.danger, 0.4)));
            at(ui, err_rect.shrink2(vec2(PAD, 0.0)), |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    Icon::AlertCircle.show(ui, 14.0, t.danger);
                    ui.label(RichText::new(e).font(font(size::SM)).color(t.danger));
                });
            });
        }
        // Footer: keyboard hints on the left, the buttons on the right.
        widgets::dialog_footer(ui, footer, PAD, |ui| {
            let label = if dlg.edit_id().is_some() { "Save changes" } else { "Create session" };
            if widgets::Button::primary(label).enabled(!dlg.busy).show(ui).clicked() {
                save = true;
            }
            if widgets::Button::secondary("Cancel").show(ui).clicked() {
                close = true;
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let muted = |ui: &mut Ui, s: &str| ui.label(RichText::new(s).font(font(size::XS)).color(t.muted));
                form::kbd(ui, "Esc");
                muted(ui, "close");
                ui.add_space(8.0);
                form::kbd(ui, "Ctrl");
                muted(ui, "+");
                form::kbd(ui, "Enter");
                muted(ui, "save");
            });
        });
        if !manager_open && form::ctrl_enter(ui) {
            save = true;
        }
    });
    if resp.should_close() {
        close = true;
    }
    if close {
        // Dropping the dialog wipes the typed password.
        tracing::debug!(serial = dlg.serial, "session_dialog: closed");
        return;
    }
    if save {
        dlg.save(app);
    }
    app.dialogs.session = Some(dlg);
}

/// Run `add` in a child ui confined to `rect`.
fn at<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(UiBuilder::new().max_rect(rect), add).inner
}

// ---------------------------------------------------------------- panes

const KINDS: [(&str, Icon, &str); 3] = [("ssh", Icon::Network, "SSH"), ("wsl", Icon::SquareTerminal, "WSL"), ("local", Icon::MonitorDot, "Local Shell")];

const AUTHS: [(&str, Icon, &str, &str); 3] = [
    ("password", Icon::Key, "Password", "Saved with this session"),
    ("key", Icon::KeyRound, "Private Key", "PEM / OpenSSH key"),
    ("agent", Icon::Cpu, "SSH Agent", "Pageant / OpenSSH Agent"),
];

/// The agent method's explanation (shared with the auth-fix overlay).
pub(crate) fn agent_hint(ui: &mut Ui) {
    rich_hint(
        ui,
        &[
            ("Uses your OS SSH agent (Pageant / OpenSSH Agent service). On Windows the agent service is disabled by default — if you see ", false),
            ("Agent failure", true),
            (", switch to Password or Private Key.", false),
        ],
    );
}

fn general(dlg: &mut SessionDialog, app: &mut App, ui: &mut Ui) {
    let windows = api::local::platform() == "windows";
    form::section(ui, "Type", |ui| {
        let cards = KINDS.map(|(kind, icon, title)| {
            let hint = match kind {
                "ssh" => "Remote Linux / Unix server",
                "wsl" => "Local Linux subsystem distro",
                _ if windows => "cmd / PowerShell / pwsh",
                _ => "bash / zsh / fish / …",
            };
            (kind, icon, title, hint)
        });
        let current = dlg.draft.session_kind.clone();
        if let Some(kind) = form::option_cards(ui, &current.as_str(), &cards) {
            dlg.switch_kind(kind);
        }
    });

    let kind = dlg.draft.session_kind.clone();
    form::section(ui, "Connection", |ui| {
        form::field(ui, "Name", None, |ui| {
            let hint = match kind.as_str() {
                "wsl" => "My Ubuntu WSL",
                "local" => "Local PowerShell",
                _ => "My production server",
            };
            let r = form::input(ui, &mut dlg.draft.name, InputOpts::new(hint));
            labelled(ui, &r, "Name");
            if std::mem::take(&mut dlg.focus_name) {
                r.request_focus();
            }
        });
        form::field(ui, "Folder", None, |ui| {
            let mut options: Vec<(Option<i64>, String)> = vec![(None, "(root)".into())];
            options.extend(app.sidebar.folders.iter().map(|f| (Some(f.id), f.name.clone())));
            form::select(ui, Id::unique(("session-dialog", "folder")), &mut dlg.draft.folder_id, &options, None);
        });
        match kind.as_str() {
            "ssh" => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let host_w = ui.available_width() - 110.0 - 12.0;
                    ui.vertical(|ui| {
                        ui.set_width(host_w);
                        form::field(ui, "Host", None, |ui| {
                            let r = form::input(ui, &mut dlg.draft.host, InputOpts::new("example.com or 10.0.0.1").icon(Icon::Server));
                            labelled(ui, &r, "Host");
                        });
                    });
                    ui.vertical(|ui| {
                        ui.set_width(110.0);
                        form::field(ui, "Port", None, |ui| number_field(ui, "Port", &mut dlg.draft.port, 110.0));
                    });
                });
                form::field(ui, "Username", None, |ui| {
                    let r = form::input(ui, &mut dlg.draft.username, InputOpts::new("root").icon(Icon::User));
                    labelled(ui, &r, "Username");
                });
            }
            "wsl" => {
                form::field(ui, "Distro", Some("Leave blank to launch the default distro. Type exactly as shown by `wsl -l --quiet`."), |ui| {
                    let r = form::input(ui, &mut dlg.draft.host, InputOpts::new("Ubuntu-24.04").icon(Icon::SquareTerminal).mono());
                    labelled(ui, &r, "Distro");
                });
                form::field(ui, "WSL user (optional)", None, |ui| {
                    let r = form::input(ui, &mut dlg.draft.username, InputOpts::new("(distro default)").icon(Icon::User).mono());
                    labelled(ui, &r, "WSL user (optional)");
                });
            }
            _ => local_fields(dlg, ui, windows),
        }
    });

    if dlg.draft.is_ssh() {
        form::section(ui, "Authentication", |ui| {
            form::field(ui, "Method", None, |ui| {
                let current = dlg.draft.auth_type.clone();
                if let Some(auth) = form::option_cards(ui, &current.as_str(), &AUTHS) {
                    dlg.switch_auth(auth);
                }
            });
            match dlg.draft.auth_type.as_str() {
                "password" => {
                    let stored = dlg.password_stored();
                    form::field(ui, "Password", None, |ui| {
                        let hint = if stored { "Saved — leave blank to keep" } else { "Password" };
                        let r = form::secret(ui, &mut dlg.password, hint, &mut dlg.show_password, None);
                        labelled(ui, &r, "Password");
                    });
                }
                "key" => {
                    form::field(ui, "Private key", None, |ui| dlg.key_picker.show(app, ui, &mut dlg.draft.credential_id));
                    form::field(ui, "Key passphrase (optional)", None, |ui| {
                        dlg.pass_picker.show(app, ui, &mut dlg.draft.key_passphrase_credential_id);
                    });
                }
                _ => agent_hint(ui),
            }
        });
    }

    form::section(ui, "Appearance", |ui| {
        form::field(ui, "Tab color", None, |ui| swatches(ui, &mut dlg.draft.color));
    });
}

const CUSTOM: &str = "__custom";

fn local_fields(dlg: &mut SessionDialog, ui: &mut Ui, windows: bool) {
    let hint = if windows {
        "Short name or absolute path. Short names resolve via %PATH%."
    } else {
        "Pick a detected shell or enter a custom binary path."
    };
    let shells = dlg.shells.as_deref().unwrap_or_default();
    let detected = shells.iter().any(|s| s.program == dlg.draft.host);
    form::field(ui, "Shell", Some(hint), |ui| {
        let mut options: Vec<(String, String)> = shells.iter().map(|s| (s.program.clone(), s.display_name.clone())).collect();
        options.push((CUSTOM.into(), "Custom path…".into()));
        let mut choice = if detected { dlg.draft.host.clone() } else { CUSTOM.to_string() };
        if form::select(ui, Id::unique(("session-dialog", "shell")), &mut choice, &options, None) {
            tracing::debug!(shell = %choice, "session_dialog: shell picked");
            if choice != CUSTOM {
                dlg.draft.host = choice;
            } else if detected {
                // Start the custom path empty; keeping the detected program
                // would snap the dropdown straight back to it.
                dlg.draft.host.clear();
            }
        }
        if !shells.iter().any(|s| s.program == dlg.draft.host) {
            let ph = if windows { "C:\\Program Files\\Git\\bin\\bash.exe" } else { "/usr/local/bin/fish" };
            let r = form::input(ui, &mut dlg.draft.host, InputOpts::new(ph).mono());
            labelled(ui, &r, "Custom shell path");
        }
    });
    form::field(ui, "Starting directory (optional)", None, |ui| {
        let ph = if windows { "C:\\Users\\you\\projects" } else { "/home/you/projects" };
        let r = form::input(ui, &mut dlg.draft.username, InputOpts::new(ph).icon(Icon::FolderOpen).mono());
        labelled(ui, &r, "Starting directory (optional)");
    });
}

fn terminal(dlg: &mut SessionDialog, ui: &mut Ui) {
    let d = &mut dlg.draft;
    form::section(ui, "On connect", |ui| {
        let hint = match d.session_kind.as_str() {
            "wsl" => "Linux path. Empty uses your Linux home (~).",
            "ssh" => "cd to this after connect. Empty uses the remote $HOME.",
            _ => "Ignored for local shells — the Host field is used as cwd.",
        };
        form::field(ui, "Starting directory (optional)", Some(hint), |ui| {
            let ph = if d.session_kind == "wsl" { "~/projects" } else { "/var/log" };
            let r = form::input(ui, &mut d.starting_dir, InputOpts::new(ph).icon(Icon::FolderOpen).mono());
            labelled(ui, &r, "Starting directory (optional)");
        });
        form::field(ui, "Initial command (optional)", Some("Runs once after the shell opens, as if you typed it."), |ui| {
            let r = form::input(ui, &mut d.initial_command, InputOpts::new("tmux attach -t main || tmux new -s main").mono());
            labelled(ui, &r, "Initial command (optional)");
        });
    });

    form::section(ui, "Display", |ui| {
        two_columns(
            ui,
            |ui, w| form::field(ui, "Scrollback lines", Some("0 disables scrollback."), |ui| number_field(ui, "Scrollback lines", &mut d.scrollback_lines, w)),
            |ui, w| form::field(ui, "Font size", None, |ui| number_field(ui, "Font size", &mut d.font_size, w)),
        );
        form::field(
            ui,
            "Font family",
            Some("(default) uses the bundled monospace font. The list shows the monospace families installed on this computer."),
            |ui| font_family_picker(ui, &mut d.font_family, &mut dlg.font_custom),
        );
        form::field(ui, "Cursor style", None, |ui| cursor_style(ui, &mut d.cursor_style));
    });

    form::section(ui, "Environment variables", |ui| {
        rich_hint(
            ui,
            &[
                ("Sent via SSH ", false),
                ("env", true),
                (" requests on connect. Most servers reject unlisted names via ", false),
                ("AcceptEnv", true),
                ("; ", false),
                ("LANG", true),
                (" and ", false),
                ("LC_*", true),
                (" usually work.", false),
            ],
        );
        env_editor(ui, &mut d.env);
    });
}

fn advanced(dlg: &mut SessionDialog, ui: &mut Ui) {
    let d = &mut dlg.draft;
    form::section(ui, "Connection", |ui| {
        two_columns(
            ui,
            |ui, w| form::field(ui, "Connect timeout (seconds)", None, |ui| number_field(ui, "Connect timeout (seconds)", &mut d.connect_timeout_secs, w)),
            |ui, w| form::field(ui, "Keepalive (seconds)", Some("0 disables SSH keepalives."), |ui| number_field(ui, "Keepalive (seconds)", &mut d.keepalive_secs, w)),
        );
    });
    form::section(ui, "Transport", |ui| {
        form::checkbox(
            ui,
            &mut d.compression,
            "Enable SSH compression (zlib)",
            Some("Helps on slow links. Negligible or negative effect on LANs; leave off unless you know the link is constrained."),
        );
    });
    if d.is_ssh() {
        form::section(ui, "X11 forwarding", |ui| {
            form::checkbox(ui, &mut d.forward_x11, "Forward X11 (display remote GUI apps locally)", None);
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
                ui.vertical(|ui| {
                    rich_hint(
                        ui,
                        &[
                            ("On connect, ezSsh starts a local VcXsrv display on ", false),
                            (":0", true),
                            (" and asks the SSH server for X11 forwarding. Requires VcXsrv installed at ", false),
                            ("%ProgramFiles%\\VcXsrv\\", true),
                            (".", false),
                        ],
                    );
                    let t = tokens(ui.ctx());
                    ui.hyperlink_to(RichText::new("Get VcXsrv").font(font(size::SM)).color(t.accent), "https://sourceforge.net/projects/vcxsrv/");
                });
            });
        });
    }
}

// ---------------------------------------------------------------- controls

/// Two equal columns 12 px apart (`grid grid-cols-2 gap-3`); each gets its
/// width.
pub(crate) fn two_columns(ui: &mut Ui, left: impl FnOnce(&mut Ui, f32), right: impl FnOnce(&mut Ui, f32)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let w = (ui.available_width() - 12.0) / 2.0;
        ui.vertical(|ui| {
            ui.set_width(w);
            left(ui, w);
        });
        ui.vertical(|ui| {
            ui.set_width(w);
            right(ui, w);
        });
    });
}

/// A labelled number field with a stepper, committed on Enter / blur /
/// step; unclamped: out-of-range values are reported by [`validate`].
pub(crate) fn number_field(ui: &mut Ui, label: &str, value: &mut i64, width: f32) {
    let draft_id = Id::unique(("session-dialog-number", label));
    let mut draft: String = ui.ctx().data(|d| d.get_temp(draft_id)).unwrap_or_else(|| value.to_string());
    let out = form::number(ui, &mut draft, label, width);
    let resp = out.resp;
    if out.step != 0 {
        *value = draft.trim().parse::<i64>().unwrap_or(*value) + out.step;
        ui.ctx().data_mut(|d| d.remove::<String>(draft_id));
        return;
    }
    // Ctrl+Enter saves the dialog in this same frame while the field keeps
    // its focus (a single-line TextEdit only surrenders it on plain Enter):
    // commit the draft first, or the save writes the previous value.
    let save_chord = form::ctrl_enter(ui);
    if resp.has_focus() && !save_chord {
        ui.ctx().data_mut(|d| d.insert_temp(draft_id, draft));
        return;
    }
    if resp.lost_focus() || save_chord {
        match draft.trim().parse::<i64>() {
            Ok(n) => *value = n,
            Err(_) => tracing::debug!(field = label, input = %draft, "session_dialog: not a number, kept {value}"),
        }
    }
    ui.ctx().data_mut(|d| d.remove::<String>(draft_id));
}

/// Muted wrapped text with `font-mono` spans.
fn rich_hint(ui: &mut Ui, parts: &[(&str, bool)]) {
    let t = tokens(ui.ctx());
    let mut job = LayoutJob::default();
    for (text, mono) in parts {
        let font_id = if *mono { FontId::monospace(size::SM) } else { font(size::SM) };
        job.append(text, 0.0, TextFormat { font_id, color: t.muted, ..Default::default() });
    }
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

const SLATE: Color32 = Color32::from_rgb(0x94, 0xa3, 0xb8);

/// Tab colours; `None` = no accent (shown slate).
const SWATCHES: [(Option<&str>, &str); 6] = [
    (Some("#60a5fa"), "Blue"),
    (Some("#34d399"), "Green"),
    (Some("#fbbf24"), "Amber"),
    (Some("#f87171"), "Red"),
    (Some("#a78bfa"), "Purple"),
    (None, "None"),
];

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub(crate) fn swatches(ui: &mut Ui, color: &mut Option<String>) {
    let t = tokens(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for (value, label) in SWATCHES {
            let selected = color.as_deref() == value;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            let fill = value.and_then(parse_hex).unwrap_or(SLATE);
            ui.painter().rect_filled(rect, radius::MD, fill);
            if selected {
                ui.painter().rect_stroke(rect.expand(3.0), radius::MD + 3.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_stroke(rect.expand(3.0), radius::MD + 3.0, Stroke::new(2.0, t.border_strong), StrokeKind::Inside);
            }
            widgets::focus_ring(ui, &resp, rect);
            widgets::a11y_selected(&resp, egui::Role::Button, selected, label);
            if resp.on_hover_text(label).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                *color = value.map(str::to_string);
            }
        }
    });
}

/// `FontFamilyPicker`: installed monospace fonts plus "Custom…", which shows a free-text
/// field. A value that is not a preset shows as Custom.
pub(crate) fn font_family_picker(ui: &mut Ui, value: &mut String, custom: &mut bool) {
    const CUSTOM_FONT: &str = font_presets::CUSTOM_FONT;
    let options = font_presets::choices_with_custom();
    let mut choice = if *custom { CUSTOM_FONT.to_string() } else { value.clone() };
    if form::select(ui, Id::unique(("session-dialog", "font-family")), &mut choice, options, None) {
        tracing::debug!(font = %choice, "session_dialog: font family picked");
        if choice == CUSTOM_FONT {
            *custom = true;
        } else {
            *custom = false;
            *value = choice;
        }
    }
    if *custom {
        let r = form::input(ui, value, InputOpts::new("e.g. \"Iosevka\", \"Fira Code\""));
        labelled(ui, &r, "Custom font family");
    }
}

const CURSORS: [(&str, &str, Icon); 3] = [("block", "Block", Icon::Square), ("bar", "Bar", Icon::Type), ("underline", "Underline", Icon::Minus)];

/// Segmented Block / Bar / Underline buttons.
pub(crate) fn cursor_style(ui: &mut Ui, value: &mut String) {
    let items: Vec<Segment<'_, &str>> = CURSORS.iter().map(|(v, label, icon)| Segment::text(*v, label).with_icon(*icon)).collect();
    let mut chosen = value.as_str();
    if widgets::segmented(ui, Id::unique("session-dialog-cursor"), &mut chosen, &items) {
        *value = chosen.to_string();
    }
}

fn env_editor(ui: &mut Ui, env: &mut Vec<EnvPair>) {
    let t = tokens(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if env.is_empty() {
            ui.label(RichText::new("No variables.").font(font(size::SM)).color(t.muted).italics());
        }
        let mut remove = None;
        for (i, p) in env.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = form::input(ui, &mut p.key, InputOpts::new("NAME").mono().width(180.0));
                labelled(ui, &r, &format!("env name {}", i + 1));
                let value_w = ui.available_width() - 28.0 - 8.0;
                let r = form::input(ui, &mut p.value, InputOpts::new("value").mono().width(value_w));
                labelled(ui, &r, &format!("env value {}", i + 1));
                let what = if p.key.is_empty() { (i + 1).to_string() } else { p.key.clone() };
                if widgets::Button::icon_only(Icon::Trash, &format!("remove env {what}"), widgets::Variant::Ghost).small().show(ui).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            env.remove(i);
        }
        if widgets::Button::ghost("Add variable").icon(Icon::Plus).small().show(ui).clicked() {
            env.push(EnvPair { key: String::new(), value: String::new() });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_draft() -> Draft {
        let mut d = Draft::new_session(None, &Settings::default());
        d.name = "prod".into();
        d.host = "example.com".into();
        d.username = "root".into();
        d
    }

    #[test]
    fn defaults_come_from_settings() {
        let d = Draft::new_session(Some(3), &Settings::default());
        assert_eq!((d.folder_id, d.port, d.auth_type.as_str(), d.session_kind.as_str()), (Some(3), 22, "agent", "ssh"));
        assert_eq!((d.scrollback_lines, d.font_size, d.keepalive_secs, d.connect_timeout_secs), (5000, 13, 60, 15));
        assert_eq!(d.cursor_style, "block");
    }

    #[test]
    fn validation_messages_in_order() {
        let ok = ssh_draft();
        assert_eq!(validate(&ok, false, false), None);
        let mut d = ok.clone();
        d.name = "  ".into();
        assert_eq!(validate(&d, false, false), Some("Name is required"));
        let mut d = ok.clone();
        d.host.clear();
        assert_eq!(validate(&d, false, false), Some("Host is required"));
        let mut d = ok.clone();
        d.port = 70000;
        assert_eq!(validate(&d, false, false), Some("Port must be between 1 and 65535"));
        let mut d = ok.clone();
        d.auth_type = "key".into();
        assert_eq!(validate(&d, false, false), Some("Credential is required for this auth type"));
        let mut d = ok.clone();
        d.font_size = 7;
        assert_eq!(validate(&d, false, false), Some("Font size must be between 8 and 48"));
        let mut d = ok.clone();
        d.env.push(EnvPair { key: "A=B".into(), value: String::new() });
        assert_eq!(validate(&d, false, false), Some("Env var name cannot contain \"=\" or NUL"));
        // WSL ignores host / username / port.
        let mut d = ok;
        d.session_kind = "wsl".into();
        d.host.clear();
        d.port = 0;
        assert_eq!(validate(&d, false, false), None);
    }

    #[test]
    fn password_required_only_when_none_is_stored() {
        let mut d = ssh_draft();
        d.auth_type = "password".into();
        assert_eq!(validate(&d, false, false), Some("Password is required"));
        assert_eq!(validate(&d, true, false), None);
        assert_eq!(validate(&d, false, true), None, "edit: blank keeps the stored one");
    }

    #[test]
    fn input_is_cleaned_and_carries_the_password_only_for_password_ssh() {
        let mut d = ssh_draft();
        d.auth_type = "password".into();
        d.credential_id = Some(9);
        d.initial_command = "   ".into();
        d.starting_dir = " /var/log ".into();
        d.env = vec![EnvPair { key: " ".into(), value: "x".into() }, EnvPair { key: "LANG".into(), value: "C".into() }];
        let i = d.to_input(Some(Secret::new("hunter2".into())));
        assert_eq!(i.password.as_ref().map(|p| p.expose()), Some("hunter2"));
        assert_eq!(i.credential_id, None, "the core manages the password row");
        assert_eq!((i.initial_command, i.starting_dir.as_deref()), (None, Some("/var/log")));
        assert_eq!(i.env.len(), 1);

        d.auth_type = "key".into();
        d.credential_id = Some(4);
        let i = d.to_input(Some(Secret::new("hunter2".into())));
        assert!(i.password.is_none());
        assert_eq!(i.credential_id, Some(4));

        d.session_kind = "local".into();
        d.forward_x11 = true;
        let i = d.to_input(None);
        assert_eq!((i.auth_type.as_str(), i.credential_id, i.forward_x11), ("agent", None, 0));
    }

    #[test]
    fn edit_draft_hides_the_own_password_row() {
        let d = Draft::new_session(None, &Settings::default());
        let s = Session {
            id: 1,
            folder_id: None,
            name: "a".into(),
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_type: "password".into(),
            credential_id: Some(5),
            key_passphrase_credential_id: None,
            color: Some("#60a5fa".into()),
            sort: 0,
            initial_command: None,
            scrollback_lines: d.scrollback_lines,
            font_size: d.font_size,
            font_family: String::new(),
            cursor_style: "bar".into(),
            compression: 1,
            keepalive_secs: 60,
            connect_timeout_secs: 15,
            session_kind: "ssh".into(),
            forward_x11: 0,
            starting_dir: None,
        };
        let dlg = SessionDialog::edit(s);
        assert!(dlg.password_stored());
        assert_eq!(dlg.draft.credential_id, None);
        assert!(dlg.draft.compression);
        assert_eq!(dlg.draft.summary(), "u@h:22");
    }

    #[test]
    fn summaries_by_kind_and_hex_colours() {
        let mut d = Draft::new_session(None, &Settings::default());
        assert_eq!(d.summary(), "user@host:22");
        d.session_kind = "wsl".into();
        d.host = "Ubuntu".into();
        d.username = "me".into();
        assert_eq!(d.summary(), "me@Ubuntu");
        d.session_kind = "local".into();
        d.host = "/bin/zsh".into();
        assert_eq!(d.summary(), "/bin/zsh");
        assert_eq!(parse_hex("#60a5fa"), Some(Color32::from_rgb(0x60, 0xa5, 0xfa)));
        assert_eq!(parse_hex("blue"), None);
    }
}
