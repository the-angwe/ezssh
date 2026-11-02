//! `xserver-missing-dialog.tsx`: X11 forwarding is on but no X server is
//! installed; install the bundled VcXsrv (Windows only), or connect
//! without X11. Cancel closes the tab, as in the webview.

use egui::text::LayoutJob;
use egui::{FontId, Id, RichText, TextFormat, Ui};
use ezssh_core::api;

use crate::app::App;
use crate::form;
use crate::icons::Icon;
use crate::style::{font, font_semibold, size, tokens};
use crate::tabs::TabId;
use crate::views::vault_dialogs::para;
use crate::widgets::{self, Tone};

/// The core can download and install VcXsrv only on Windows
/// (`xserver::install_vcxsrv` refuses elsewhere).
const CAN_INSTALL: bool = cfg!(windows);

enum Phase {
    Idle,
    Installing,
    Failed(String),
}

pub struct XServerDialog {
    pub tab: TabId,
    /// The connect error (logged; the dialog has its own copy).
    pub message: String,
    phase: Phase,
    /// Other tabs that hit the same missing server while this one was up;
    /// served one after another, all at once by a successful install.
    waiting: Vec<TabId>,
}

impl XServerDialog {
    pub fn new(tab: TabId, message: String) -> Self {
        tracing::debug!(tab, %message, "xserver: missing-server dialog");
        Self { tab, message, phase: Phase::Idle, waiting: Vec::new() }
    }

    /// Another tab failed the same way while the dialog is open.
    pub fn add(&mut self, tab: TabId) {
        if tab != self.tab && !self.waiting.contains(&tab) {
            tracing::debug!(tab, current = self.tab, "xserver: tab queued behind the open dialog");
            self.waiting.push(tab);
        }
    }
}

/// Done with the current tab: show the next waiting one that is still
/// open, or close the dialog.
fn advance(app: &mut App) {
    let Some(dlg) = app.dialogs.xserver.as_mut() else {
        return;
    };
    while !dlg.waiting.is_empty() {
        let next = dlg.waiting.remove(0);
        if app.tabs.get(next).is_some() {
            dlg.tab = next;
            dlg.phase = Phase::Idle;
            return;
        }
    }
    app.dialogs.xserver = None;
}

enum Action {
    Install,
    WithoutX11,
    Cancel,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(dlg) = app.dialogs.xserver.as_ref() else {
        return;
    };
    let t = tokens(ctx);
    let tab = dlg.tab;
    let installing = matches!(dlg.phase, Phase::Installing);
    let failed = matches!(dlg.phase, Phase::Failed(_));
    let mut action = None;
    let out = widgets::dialog(
        ctx,
        Id::unique("xserver-missing"),
        460.0,
        |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                Icon::AlertTriangle.show(ui, 22.0, t.warning);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(RichText::new("VcXsrv is required for X11 forwarding").font(font_semibold(size::SM)).color(t.fg));
                    explanation(ui);
                    match &dlg.phase {
                        Phase::Failed(msg) => {
                            ui.add_space(4.0);
                            para(ui, &format!("Install failed: {msg}"), t.danger);
                        }
                        Phase::Installing => {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                form::spinner(ui, 12.0);
                                ui.label(RichText::new("Downloading and installing VcXsrv\u{2026}").font(font(size::XS)).color(t.muted));
                            });
                        }
                        Phase::Idle => {}
                    }
                });
            });
        },
        Some(&mut |ui: &mut Ui| {
            if CAN_INSTALL {
                let icon = if installing { Icon::Loader2 } else { Icon::Download };
                let label = if failed { "Retry install" } else { "Install VcXsrv" };
                if widgets::primary_with_icon(ui, Some(icon), label, Tone::Accent, !installing, size::SM).clicked() {
                    action = Some(Action::Install);
                }
            }
            if widgets::Ghost::new("Continue without X11").icon(Icon::MonitorOff).enabled(!installing).show(ui).clicked() {
                action = Some(Action::WithoutX11);
            }
            if widgets::Ghost::new("Cancel").enabled(!installing).show(ui).clicked() {
                action = Some(Action::Cancel);
            }
        }),
    );
    if out.dismissed() && !installing && action.is_none() {
        action = Some(Action::Cancel);
    }
    match action {
        Some(Action::Install) => install(app, tab),
        Some(Action::WithoutX11) => {
            tracing::info!(tab, "xserver: reconnecting without X11");
            advance(app);
            app.connect_tab(tab, false, true);
        }
        Some(Action::Cancel) => {
            tracing::info!(tab, "xserver: cancelled, closing the tab");
            advance(app);
            app.close_tab(tab);
        }
        None => {}
    }
}

/// The body copy. On Windows the TSX text; elsewhere there is nothing to
/// install, so it says so.
fn explanation(ui: &mut Ui) {
    let t = tokens(ui.ctx());
    if !CAN_INSTALL {
        para(
            ui,
            "This session has X11 forwarding enabled, but ezSsh forwards X11 through VcXsrv, which only runs on Windows. Connect without X11 for this session, or turn X11 forwarding off in the session settings.",
            t.muted,
        );
        return;
    }
    let plain = TextFormat { font_id: font(size::XS), color: t.muted, ..TextFormat::default() };
    let mut job = LayoutJob::default();
    job.append("This session has X11 forwarding enabled, but no VcXsrv install was found next to ezSsh, in your user data folder, or under ", 0.0, plain.clone());
    job.append("C:\\Program Files\\VcXsrv\\", 0.0, TextFormat { font_id: FontId::monospace(size::XS), color: t.muted, ..TextFormat::default() });
    job.append(". You can install it now (roughly 3\u{a0}MB, no admin required) or connect without X11 for this session.", 0.0, plain);
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

fn install(app: &mut App, tab: TabId) {
    let Some(dlg) = app.dialogs.xserver.as_mut() else {
        return;
    };
    dlg.phase = Phase::Installing;
    tracing::info!(tab, "xserver: installing VcXsrv");
    app.core.call(
        "xserver_install",
        |st| async move { api::xserver::xserver_install(&st).await },
        move |app, r| match r {
            Ok(path) => {
                // The server is there now: every tab that was waiting for it
                // reconnects, whichever one the dialog shows.
                let mut tabs = vec![tab];
                if let Some(dlg) = app.dialogs.xserver.take() {
                    tabs.push(dlg.tab);
                    tabs.extend(dlg.waiting);
                }
                tabs.sort_unstable();
                tabs.dedup();
                tracing::info!(%path, ?tabs, "xserver: VcXsrv installed, reconnecting");
                if !matches!(app.screen, crate::app::Screen::Main) {
                    return;
                }
                for t in tabs {
                    if app.tabs.get(t).is_some() {
                        app.connect_tab(t, false, false);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(tab, "xserver: install failed: {e}");
                match app.dialogs.xserver.as_mut() {
                    Some(dlg) => dlg.phase = Phase::Failed(e.to_string()),
                    None => app.toasts.danger("VcXsrv install failed", e.to_string()),
                }
            }
        },
    );
}
