//! The window: start-up, the vault gate (`ui/app/page.tsx`), the frame
//! loop and the app-wide behaviour of `main-shell.tsx` (settings hydrate,
//! autodetect, idle lock, quit confirmation, global shortcuts).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{FontFamily, Key, Modifiers};
use ezssh_core::xserver::XServerStatus;
use ezssh_core::{Event, api};
use ezssh_term::view::ImeLoopFilter;
use ezssh_term::{BellMode, CursorKind, FontRegistry, RightClick, TermOptions, TermStyle};

use crate::conn::TermSlot;
use crate::core::Core;
use crate::layout::{self, Layout};
use crate::settings::{Settings, Value};
use crate::style::{self, Theme};
use crate::tabs::{TabId, Tabs};
use crate::toasts::Toasts;
use crate::views;

/// Where the app keeps its files.
#[derive(Clone, Debug)]
pub struct Options {
    pub db_path: PathBuf,
}

pub enum Screen {
    // Boxed: the unlock state (with its dialogs) is much larger than `Main`.
    Unlock(Box<views::unlock::UnlockState>),
    Main,
}

/// Modal state owned by the shell (the sidebar keeps its own dialogs).
#[derive(Default)]
pub struct Dialogs {
    pub settings: Option<views::settings::SettingsDialog>,
    pub quit_confirm: bool,
    pub tile_grid: Option<views::dialogs::TileGridDraft>,
    /// Multi-line paste waiting for the user's yes: (tab, text, lines).
    pub paste: Option<(TabId, String, usize)>,
    pub session: Option<views::session_dialog::SessionDialog>,
    pub credentials: Option<views::credentials::CredentialsDialog>,
    pub change_password: Option<views::vault_dialogs::ChangePassword>,
    pub recovery_code: Option<views::vault_dialogs::RecoveryCode>,
    pub xserver: Option<views::xserver::XServerDialog>,
    pub appearance: Option<views::appearance::AppearanceDialog>,
    pub backup: Option<views::backup::BackupDialog>,
    pub restore: Option<views::restore::RestoreDialog>,
    pub sync: Option<views::sync::SyncDialog>,
    pub import: Option<views::import::ImportDialog>,
}

impl Dialogs {
    pub fn any_open(&self) -> bool {
        self.settings.is_some()
            || self.quit_confirm
            || self.tile_grid.is_some()
            || self.paste.is_some()
            || self.session.is_some()
            || self.credentials.is_some()
            || self.change_password.is_some()
            || self.recovery_code.is_some()
            || self.xserver.is_some()
            || self.appearance.is_some()
            || self.backup.is_some()
            || self.restore.is_some()
            || self.sync.is_some()
            || self.import.is_some()
    }
}

pub struct App {
    pub(crate) core: Core,
    events: mpsc::Receiver<Event>,
    pub(crate) screen: Screen,
    pub(crate) theme: Theme,
    pub(crate) settings: Settings,
    pub(crate) layout: Layout,
    layout_path: PathBuf,
    pub(crate) tabs: Tabs,
    pub(crate) terms: HashMap<TabId, TermSlot>,
    pub(crate) sidebar: views::sidebar::Sidebar,
    pub(crate) strip: views::tabstrip::StripState,
    pub(crate) mdi: views::mdi::MdiState,
    pub(crate) dialogs: Dialogs,
    /// Shared credentials (keys, passphrases) for pickers and the manager.
    pub(crate) creds: crate::creds::CredCache,
    /// The port-forward panes.
    pub(crate) forwards: views::forwards::ForwardsState,
    /// The file panes and their transfers.
    pub(crate) sftp: views::sftp::SftpState,
    pub(crate) toasts: Toasts,
    pub(crate) term_style: TermStyle,
    pub(crate) fonts: FontRegistry,
    /// UI monospace (`font-mono`): the terminal's default font.
    pub(crate) mono: FontFamily,
    pub(crate) xstatus: Option<XServerStatus>,
    xpoll_at: Instant,
    pub(crate) last_input: Instant,
    /// Quit confirmed: let the next close request through.
    allow_close: bool,
    ime_filter: ImeLoopFilter,
    window_title: String,
    pub(crate) ctx: egui::Context,
}

const XSERVER_POLL: Duration = Duration::from_secs(5);
/// How often the idle-lock clock is checked (the webview's 15 s ticker).
const IDLE_TICK: Duration = Duration::from_secs(15);

pub const UPDATER_HINT: &str = "Not in the native build yet: arrives with the updater (rework phase 7). Use the ezTerm app meanwhile.";

impl App {
    /// Open the database and show the unlock screen. Fails when the
    /// database cannot be opened.
    pub fn new(ctx: &egui::Context, options: Options) -> anyhow::Result<Self> {
        egui_extras::install_image_loaders(ctx);
        crate::ui_font::install(ctx);
        ezssh_term::fonts::prewarm_monospace_families();
        let mut fonts = FontRegistry::new();
        let mono = fonts.faces(ctx, "").regular;
        let layout_path = layout::path_for(&options.db_path);
        let layout = layout::load(&layout_path);

        let (core, events) = Core::open(ctx, &options.db_path)?;
        let status = core
            .block_on(|st| async move { api::vault::vault_status(&st).await })
            .map_err(|e| anyhow::anyhow!("reading the vault state: {e}"))?;
        let theme = match core.block_on(|st| async move { api::settings::settings_get(&st, "theme".into()).await }) {
            Ok(v) => Theme::from_setting(v.as_deref()),
            Err(e) => {
                tracing::warn!("app: reading the theme failed, using dark: {e}");
                Theme::Dark
            }
        };
        tracing::info!(vault = status, db = %options.db_path.display(), "app: start");
        let mut unlock = views::unlock::UnlockState::new(status == "uninitialized");
        unlock.probe_recovery(&core);
        let screen = Screen::Unlock(Box::new(unlock));
        style::apply(ctx, theme);
        let tabs = Tabs::new(layout.view_mode, layout.tile_grid);
        let mut app = Self {
            core,
            events,
            screen,
            theme,
            settings: Settings::default(),
            layout,
            layout_path,
            tabs,
            terms: HashMap::new(),
            sidebar: views::sidebar::Sidebar::default(),
            strip: views::tabstrip::StripState::default(),
            mdi: views::mdi::MdiState::default(),
            dialogs: Dialogs::default(),
            creds: crate::creds::CredCache::default(),
            forwards: views::forwards::ForwardsState::default(),
            sftp: views::sftp::SftpState::default(),
            toasts: Toasts::default(),
            term_style: TermStyle::default(),
            fonts,
            mono,
            xstatus: None,
            xpoll_at: Instant::now(),
            last_input: Instant::now(),
            allow_close: false,
            ime_filter: ImeLoopFilter::default(),
            window_title: String::new(),
            ctx: ctx.clone(),
        };
        app.rebuild_term_style();
        Ok(app)
    }

    // ------------------------------------------------------------ vault

    /// The vault was unlocked (or created): show the workspace and do what
    /// `main-shell.tsx` does on mount.
    pub(crate) fn on_unlocked(&mut self) {
        tracing::info!("app: unlocked");
        self.screen = Screen::Main;
        self.last_input = Instant::now();
        self.core.call(
            "settings_list",
            |st| async move { api::settings::settings_list(&st).await },
            |app, rows| match rows {
                Ok(rows) => {
                    app.settings = Settings::from_rows(&rows);
                    app.apply_settings();
                }
                Err(e) => app.toasts.danger("Could not load settings", e.to_string()),
            },
        );
        self.sidebar.reload(&self.core);
        self.creds.reload(&self.core);
        // WSL first, then local shells; each is idempotent, and both
        // report what they did, as in the webview.
        self.core.call(
            "wsl_autodetect_seed",
            |st| async move { api::local::wsl_autodetect_seed(&st).await },
            |app, r| {
                match r {
                    Ok(0) => app.toasts.info("WSL autodetect", "No distros detected (or all already present).".to_string()),
                    Ok(n) => app.toasts.success("WSL detected", format!("Added {n} session{} to the WSL folder.", plural(n))),
                    Err(e) => app.toasts.danger("WSL autodetect failed", e.to_string()),
                }
                app.sidebar.reload(&app.core);
            },
        );
        self.core.call(
            "local_shells_autodetect_seed",
            |st| async move { api::local::local_shells_autodetect_seed(&st).await },
            |app, r| {
                match r {
                    Ok(0) => app.toasts.info("Local shells autodetect", "No new shells detected (or all already present).".to_string()),
                    Ok(n) => app.toasts.success("Local shells detected", format!("Added {n} session{} to the Local Shells folder.", plural(n))),
                    Err(e) => app.toasts.danger("Local shells autodetect failed", e.to_string()),
                }
                app.sidebar.reload(&app.core);
            },
        );
        self.xpoll_at = Instant::now();
    }

    /// Lock the vault. Open tabs and their connections stay up behind the
    /// unlock screen (the Settings hint promises that; the webview
    /// unmounted and reconnected them instead).
    pub(crate) fn lock(&mut self) {
        if !matches!(self.screen, Screen::Main) {
            return;
        }
        tracing::info!("app: lock");
        self.dialogs = Dialogs::default();
        self.sidebar.close_dialogs();
        self.forwards.close_dialogs();
        self.sftp.close_dialogs();
        // Auth-fix overlays hold typed secrets; the failed tabs keep their
        // error card (Reconnect brings the overlay back).
        for slot in self.terms.values_mut() {
            slot.auth_fix = None;
        }
        let mut u = views::unlock::UnlockState::new(false);
        u.probe_recovery(&self.core);
        self.screen = Screen::Unlock(Box::new(u));
        self.core.call("vault_lock", |st| async move { api::vault::vault_lock(&st).await }, |app, r| {
            if let Err(e) = r {
                app.toasts.danger("Lock failed", e.to_string());
            }
        });
    }

    // ------------------------------------------------------------ settings

    /// Store a setting and write it through; a failed write rolls back.
    pub(crate) fn set_setting(&mut self, key: &'static str, value: Value) {
        let Some((prev, clean)) = self.settings.set(key, value) else {
            return;
        };
        tracing::debug!(key, value = ?clean, "app: setting changed");
        self.apply_settings();
        let raw = clean.serialize();
        self.core.call(
            "settings_set",
            move |st| async move { api::settings::settings_set(&st, key.to_string(), raw).await },
            move |app, r| {
                if let Err(e) = r {
                    app.toasts.danger("Setting not saved", e.to_string());
                    if app.settings.rollback(key, &clean, prev) {
                        app.apply_settings();
                    }
                }
            },
        );
    }

    fn rebuild_term_style(&mut self) {
        let s = &self.settings;
        self.term_style = TermStyle {
            palette: ezssh_term::palette::by_id(s.str("terminal.palette")).clone(),
            copy_on_select: s.bool("terminal.copyOnSelect"),
            right_click: RightClick::from_setting(s.str("terminal.rightClick")),
            warn_multiline_paste: s.bool("terminal.warnMultilinePaste"),
            bell: BellMode::from_setting(s.str("terminal.bell")),
            ..TermStyle::default()
        };
    }

    /// Push the settings into every open terminal (palette and OSC 52 apply
    /// live; cursor blink applies to tabs opened afterwards, as before).
    pub(crate) fn apply_settings(&mut self) {
        self.rebuild_term_style();
        let osc52 = self.settings.bool("terminal.osc52Clipboard");
        for slot in self.terms.values() {
            slot.term.set_palette(self.term_style.palette.clone());
            slot.term.set_osc52_write(osc52);
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        style::apply(&self.ctx, theme);
        let value = theme.as_setting().to_string();
        self.core.call(
            "settings_set(theme)",
            move |st| async move { api::settings::settings_set(&st, "theme".into(), value).await },
            |app, r| {
                if let Err(e) = r {
                    app.toasts.danger("Theme not saved", e.to_string());
                }
            },
        );
    }

    pub(crate) fn save_layout(&mut self) {
        self.layout.view_mode = self.tabs.view_mode;
        self.layout.tile_grid = self.tabs.tile_grid;
        if let Err(e) = layout::save(&self.layout_path, &self.layout) {
            self.toasts.warning("Layout not saved", e.to_string());
        }
    }

    // ------------------------------------------------------------ tabs

    /// Open a tab for a saved session (double-click, Connect).
    pub(crate) fn open_session(&mut self, session: ezssh_core::db::sessions::Session) {
        let options = TermOptions {
            scrollback: session.scrollback_lines.max(0) as usize,
            osc52_write: self.settings.bool("terminal.osc52Clipboard"),
            cursor: CursorKind::from_setting(&session.cursor_style),
            cursor_blink: self.settings.bool("terminal.cursorBlink"),
        };
        let mut slot = TermSlot::new(&self.ctx, options, self.term_style.palette.clone());
        slot.term.set_font_size(session.font_size as f32);
        slot.term.set_faces(self.fonts.faces(&self.ctx, &session.font_family));
        tracing::info!(session = session.id, name = %session.name, kind = %session.session_kind, "app: open tab");
        let id = self.tabs.open(session);
        self.terms.insert(id, slot);
        self.strip.reveal_active = true;
    }

    /// A session row was saved: open tabs of that session take its font
    /// (size, family), cursor style, scrollback length and tab colour, each
    /// only when it changed from what the tab was opened (or last saved)
    /// with. An unchanged value keeps a tab-only change (Appearance dialog's
    /// "Apply to tab", Ctrl+Wheel); the cursor keeps the tab's blink setting.
    pub(crate) fn apply_session_terminal(&mut self, saved: &ezssh_core::db::sessions::Session) {
        let ids: Vec<TabId> = self.tabs.tabs.iter().filter(|t| t.session.id == saved.id).map(|t| t.id).collect();
        if ids.is_empty() {
            tracing::debug!(session = saved.id, "app: session saved, no open tab");
            return;
        }
        for id in ids {
            let Some(t) = self.tabs.get_mut(id) else {
                continue;
            };
            let font = t.session.font_size != saved.font_size || t.session.font_family != saved.font_family;
            let cursor = t.session.cursor_style != saved.cursor_style;
            let scrollback = t.session.scrollback_lines != saved.scrollback_lines;
            let color = t.session.color != saved.color;
            if color {
                tracing::info!(tab = id, session = saved.id, color = ?saved.color, "app: session colour applied to open tab");
                t.color = saved.color.clone();
                t.session.color = saved.color.clone();
            }
            if font {
                t.font_family = saved.font_family.clone();
            }
            t.session.font_size = saved.font_size;
            t.session.font_family = saved.font_family.clone();
            t.session.cursor_style = saved.cursor_style.clone();
            t.session.scrollback_lines = saved.scrollback_lines;
            if !(font || cursor || scrollback) {
                tracing::debug!(tab = id, session = saved.id, "app: session saved, terminal options unchanged");
                continue;
            }
            let faces = font.then(|| self.fonts.faces(&self.ctx, &saved.font_family));
            let Some(slot) = self.terms.get_mut(&id) else {
                tracing::warn!(tab = id, session = saved.id, "app: open tab has no terminal, session options not applied");
                continue;
            };
            if let Some(faces) = faces {
                slot.term.set_font_size(saved.font_size as f32);
                slot.term.set_faces(faces);
            }
            if cursor {
                let blink = slot.term.options().cursor_blink;
                slot.term.set_cursor(CursorKind::from_setting(&saved.cursor_style), blink);
            }
            if scrollback {
                slot.term.set_scrollback(saved.scrollback_lines.max(0) as usize);
            }
            tracing::info!(
                tab = id,
                session = saved.id,
                font,
                size = saved.font_size,
                family = %saved.font_family,
                cursor,
                cursor_style = %saved.cursor_style,
                scrollback,
                scrollback_lines = saved.scrollback_lines,
                "app: session options applied to open tab"
            );
        }
    }

    /// Close without asking (the confirmation already happened).
    pub(crate) fn close_tab(&mut self, id: TabId) {
        tracing::info!(tab = id, "app: close tab");
        self.teardown(id, true);
        self.tabs.close(id);
        self.terms.remove(&id);
        if let Some(a) = self.tabs.active
            && let Some(slot) = self.terms.get_mut(&a)
        {
            slot.focus = true;
        }
    }

    /// Close, asking first for a connected tab if the setting says so.
    pub(crate) fn request_close_tab(&mut self, id: TabId) {
        let confirm = self.settings.bool("sessions.confirmCloseLive");
        if self.tabs.get(id).is_some_and(|t| t.status == crate::tabs::TabStatus::Connected) && confirm {
            self.tabs.pending_close = Some(id);
        } else {
            self.close_tab(id);
        }
    }

    pub(crate) fn activate(&mut self, id: TabId) {
        if self.tabs.active != Some(id) {
            self.tabs.active = Some(id);
            self.strip.reveal_active = true;
        }
        if let Some(slot) = self.terms.get_mut(&id) {
            slot.focus = true;
        }
    }

    pub(crate) fn any_modal_open(&self) -> bool {
        self.dialogs.any_open()
            || self.tabs.pending_close.is_some()
            || self.sidebar.dialog_open()
            || self.terms.values().any(|s| s.host_key.is_some())
            || self.forwards.dialog_open()
            || self.sftp.dialog_open()
    }

    // ------------------------------------------------------------ frame

    fn drain(&mut self) {
        for job in self.core.take_jobs() {
            job(self);
        }
        while let Ok(event) = self.events.try_recv() {
            self.on_connection_event(event);
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(next) = self.toasts.expire(now) {
            ctx.request_repaint_after(next);
        }
        if !matches!(self.screen, Screen::Main) {
            return;
        }
        if let Some(next) = views::sftp::tick(self, now) {
            ctx.request_repaint_after(next);
        }
        // Connect tabs whose terminal has been laid out.
        let due: Vec<TabId> = self.terms.iter().filter(|(_, s)| s.connect_due(now)).map(|(id, _)| *id).collect();
        for id in due {
            self.connect_tab(id, false, false);
        }
        if self.terms.values().any(|s| s.connect_at.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        // Idle auto-lock.
        let idle_min = self.settings.int("security.idleLockMinutes");
        if idle_min > 0 {
            let limit = Duration::from_secs(idle_min as u64 * 60);
            let idle = now.saturating_duration_since(self.last_input);
            if idle >= limit {
                tracing::info!(minutes = idle_min, "app: idle lock");
                self.lock();
                return;
            }
            ctx.request_repaint_after((limit - idle).min(IDLE_TICK));
        }
        // X server pill.
        if now >= self.xpoll_at {
            self.xpoll_at = now + XSERVER_POLL;
            self.core.call("xserver_status", |st| async move { api::xserver::xserver_status(&st).await }, |app, r| match r {
                Ok(s) => app.xstatus = Some(s),
                Err(e) => tracing::debug!("app: xserver_status: {e}"),
            });
        }
        ctx.request_repaint_after(self.xpoll_at.saturating_duration_since(now));
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.allow_close {
            tracing::info!("app: quitting");
            return;
        }
        let live = matches!(self.screen, Screen::Main) && self.tabs.any_connected();
        if live && self.settings.bool("sessions.confirmExit") {
            tracing::info!("app: close requested with live tabs, asking");
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialogs.quit_confirm = true;
        }
    }

    pub(crate) fn quit(&mut self) {
        self.allow_close = true;
        self.dialogs.quit_confirm = false;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Ctrl+B / Ctrl+Shift+W, only while no terminal has the keyboard
    /// (a terminal gets ^B and ^W itself: tmux, readline).
    pub(crate) fn global_shortcuts(&mut self, ctx: &egui::Context) {
        if self.any_modal_open() {
            return;
        }
        let (toggle, cycle) = ctx.input_mut(|i| {
            let cycle = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::W);
            let toggle = i.consume_key(Modifiers::COMMAND, Key::B);
            (toggle, cycle)
        });
        if toggle {
            self.tabs.sidebar_collapsed = !self.tabs.sidebar_collapsed;
        }
        if cycle {
            self.tabs.view_mode = self.tabs.view_mode.next();
            self.save_layout();
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match (&self.screen, self.tabs.active_tab()) {
            (Screen::Main, Some(tab)) => {
                let label = tab.label(self.settings.str("appearance.tabLabel") == "userhost");
                format!("{label} — ezSsh")
            }
            _ => "ezSsh".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

impl eframe::App for App {
    /// Runs every frame, also while the window is hidden (eframe skips `ui`
    /// then), so timers, results and core events are handled here.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.tick(ctx);
        self.handle_close_request(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.ime_filter.filter(raw_input);
        let active = raw_input.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key { .. }
                    | egui::Event::Text(_)
                    | egui::Event::PointerButton { .. }
                    | egui::Event::PointerMoved(_)
                    | egui::Event::MouseWheel { .. }
            )
        });
        if active {
            self.last_input = Instant::now();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match &self.screen {
            Screen::Unlock(_) => views::unlock::show(self, ui),
            Screen::Main => views::shell::show(self, ui),
        }
        self.toasts.show(&ctx);
        self.update_title(&ctx);
    }
}
