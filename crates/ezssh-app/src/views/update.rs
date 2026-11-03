//! The update dialog: check GitHub for a newer
//! release, show the version and notes, download and verify the signed
//! installer, install it and start the new version.
//!
//! The launch check runs once per start, after the settings load, when
//! `updates.checkOnLaunch` is on and `updates.intervalDays` have passed
//! since the last successful check (kept in `layout.json`). A found update
//! opens the dialog; failures of the launch check are only logged. The status-bar button opens
//! the dialog any time and carries a dot while an update is known.
//!
//! Copies that cannot replace themselves (portable archives, `cargo run`)
//! are told about the update and get "Open releases page" instead of
//! "Install & restart".

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{Id, RichText, Stroke, Ui, vec2};
use ezssh_core::api;
use ezssh_core::updater::install::{self, AfterInstall};
use ezssh_core::updater::{Config, Update};

use crate::app::App;
use crate::icons::Icon;
use crate::settings::Value;
use crate::style::{alpha, font, font_semibold, radius, size, tokens};
use crate::views::vault_dialogs::para;
use crate::widgets::{self, Dialog};

/// "Installed" stays up this long before the restart.
const RESTART_DELAY: Duration = Duration::from_millis(800);

enum Phase {
    Checking,
    UpToDate,
    Available(Update),
    Downloading { update: Update, downloaded: u64, total: Option<u64> },
    Installed(Update),
    Error(String),
}

pub struct UpdateDialog {
    phase: Phase,
}

#[derive(Default)]
pub struct UpdaterState {
    /// `None`: this app was started without an updater (tests).
    pub cfg: Option<Config>,
    pub dialog: Option<UpdateDialog>,
    /// What the last successful check found (status-bar dot).
    pub available: Option<Update>,
    launch_checked: bool,
    /// Bumped per check; a result of an older check (the channel was
    /// toggled meanwhile) is dropped.
    seq: u64,
    restart: Option<(Instant, AfterInstall)>,
}

impl UpdaterState {
    pub fn new(cfg: Option<Config>) -> Self {
        Self { cfg, ..Self::default() }
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    fn busy(&self) -> bool {
        matches!(self.dialog.as_ref().map(|d| &d.phase), Some(Phase::Checking | Phase::Downloading { .. } | Phase::Installed(_)))
    }

    /// Lock: an idle dialog goes; a download keeps running behind the
    /// unlock screen and restarts the app when done.
    pub fn on_lock(&mut self) {
        if !self.busy() {
            self.dialog = None;
        }
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Whether the launch check is due: never checked, a clock that went
/// backwards, or `interval_days` passed.
pub fn launch_check_due(last: Option<i64>, now: i64, interval_days: i64) -> bool {
    match last {
        None => true,
        Some(t) if t > now => true,
        Some(t) => now - t >= interval_days.max(1) * 86_400,
    }
}

/// The status-bar button.
pub fn open(app: &mut App) {
    if app.updater.dialog.is_some() {
        return;
    }
    match app.updater.available.clone() {
        Some(u) => app.updater.dialog = Some(UpdateDialog { phase: Phase::Available(u) }),
        None => {
            app.updater.dialog = Some(UpdateDialog { phase: Phase::Checking });
            let pre = app.settings.bool("updates.preRelease");
            check(app, pre, false);
        }
    }
}

/// After the settings loaded on unlock: the cadence-gated launch check.
pub fn launch_check(app: &mut App) {
    if app.updater.launch_checked || app.updater.cfg.is_none() {
        return;
    }
    app.updater.launch_checked = true;
    if !app.settings.bool("updates.checkOnLaunch") {
        tracing::debug!("update: launch check off");
        return;
    }
    let days = app.settings.int("updates.intervalDays");
    if !launch_check_due(app.layout.last_update_check, now_unix(), days) {
        tracing::debug!(last = ?app.layout.last_update_check, days, "update: launch check not due");
        return;
    }
    let pre = app.settings.bool("updates.preRelease");
    check(app, pre, true);
}

/// Run a check. `launch`: a found update opens the dialog, a failure is
/// only logged.
fn check(app: &mut App, pre_release: bool, launch: bool) {
    let Some(cfg) = app.updater.cfg.clone() else { return };
    app.updater.seq += 1;
    let seq = app.updater.seq;
    tracing::info!(pre_release, launch, "update: checking");
    app.core.call(
        "updater_check",
        move |_st| async move { api::updater::updater_check(&cfg, pre_release).await },
        move |app, r| {
            if seq != app.updater.seq {
                tracing::debug!(seq, "update: stale check result dropped");
                return;
            }
            if r.is_ok() {
                app.layout.last_update_check = Some(now_unix());
                app.save_layout();
            }
            let phase = match r {
                Ok(Some(u)) => {
                    tracing::info!(version = %u.version, installable = u.installable(), "update: available");
                    app.updater.available = Some(u.clone());
                    Phase::Available(u)
                }
                Ok(None) => {
                    tracing::info!("update: up to date");
                    app.updater.available = None;
                    Phase::UpToDate
                }
                Err(e) => {
                    tracing::warn!(launch, "update: check failed: {e}");
                    Phase::Error(message(&e))
                }
            };
            match app.updater.dialog.as_mut() {
                Some(d) if matches!(d.phase, Phase::Checking) => d.phase = phase,
                Some(_) => {}
                None if launch && matches!(phase, Phase::Available(_)) => app.updater.dialog = Some(UpdateDialog { phase }),
                None => {}
            }
        },
    );
}

fn install_update(app: &mut App, update: Update) {
    let Some(cfg) = app.updater.cfg.clone() else { return };
    let Some(d) = app.updater.dialog.as_mut() else { return };
    d.phase = Phase::Downloading { update: update.clone(), downloaded: 0, total: None };
    tracing::info!(version = %update.version, kind = ?cfg.install.kind, "update: installing");
    let u = update.clone();
    app.core.call(
        "updater_install",
        move |st| async move {
            let file = api::updater::updater_download(&st, &cfg, &u).await?;
            let inst = cfg.install.clone();
            tokio::task::spawn_blocking(move || install::install(&inst, &file))
                .await
                .map_err(|e| ezssh_core::AppError::Updater(format!("install task: {e}")))?
        },
        move |app, r| {
            let phase = match r {
                Ok(after) => {
                    tracing::info!(version = %update.version, ?after, "update: installed, restarting soon");
                    app.updater.restart = Some((Instant::now() + RESTART_DELAY, after));
                    app.ctx.request_repaint_after(RESTART_DELAY);
                    Phase::Installed(update)
                }
                Err(e) => {
                    tracing::warn!("update: install failed: {e}");
                    Phase::Error(message(&e))
                }
            };
            match app.updater.dialog.as_mut() {
                Some(d) => d.phase = phase,
                None => app.updater.dialog = Some(UpdateDialog { phase }),
            }
        },
    );
}

/// An error for the dialog: the updater's own text, capitalised, without
/// the `updater:` prefix of `AppError`'s Display.
fn message(e: &ezssh_core::AppError) -> String {
    let text = match e {
        ezssh_core::AppError::Updater(m) => m.clone(),
        other => other.to_string(),
    };
    let mut c = text.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => text,
    }
}

/// `Event::UpdateProgress`.
pub fn on_progress(app: &mut App, done: u64, of: Option<u64>) {
    if let Some(UpdateDialog { phase: Phase::Downloading { downloaded, total, .. } }) = app.updater.dialog.as_mut() {
        *downloaded = done;
        *total = of;
    }
}

/// From `App::logic`: start the new version once "Installed" was shown.
pub fn tick(app: &mut App, now: Instant) -> Option<Duration> {
    let (at, _) = app.updater.restart.as_ref()?;
    if now < *at {
        return Some(*at - now);
    }
    let (_, after) = app.updater.restart.take()?;
    match install::relaunch(&after) {
        Ok(()) => {
            tracing::info!("update: exiting for the new version");
            app.quit();
        }
        Err(e) => {
            tracing::error!("update: {e}");
            if let Some(d) = app.updater.dialog.as_mut() {
                d.phase = Phase::Error(format!("The update is installed, but {e}. Start ezSsh again by hand."));
            }
        }
    }
    None
}

enum Action {
    Close,
    Recheck,
    Install(Update),
    OpenPage(String),
    Channel(bool),
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(dlg) = app.updater.dialog.as_ref() else { return };
    let t = tokens(ctx);
    let busy = app.updater.busy();
    let kind = app.updater.cfg.as_ref().map(|c| c.install.kind);
    let mut pre = app.settings.bool("updates.preRelease");
    let mut action = None;
    // The body's checkbox (the footer closure holds `action`).
    let mut channel = None;
    let out = Dialog::new(Id::unique("update-dialog"), 520.0).header(Icon::Sparkles, crate::style::tokens(ctx).accent, "ezSsh updates", None).busy(busy).row_gap(12.0).show_split(
        ctx,
        |ui| {
            match &dlg.phase {
                Phase::Checking => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        crate::form::spinner(ui, 14.0);
                        ui.label(RichText::new("Checking GitHub for a newer release\u{2026}").font(font(size::SM)).color(t.muted));
                    });
                }
                Phase::UpToDate => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        Icon::CheckCircle.show(ui, 16.0, t.success);
                        ui.label(RichText::new(format!("You\u{2019}re on the latest version (v{}).", env!("CARGO_PKG_VERSION"))).font(font(size::SM)).color(t.success));
                    });
                }
                Phase::Available(u) => available(ui, u, kind),
                Phase::Downloading { update, downloaded, total } => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        Icon::Download.show(ui, 14.0, t.accent);
                        ui.label(RichText::new(&update.version).font(egui::FontId::new(size::SM, app.mono.clone())).color(t.fg));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let text = match total {
                                Some(tt) => format!("{} / {}", format_bytes(*downloaded), format_bytes(*tt)),
                                None => format_bytes(*downloaded),
                            };
                            ui.label(RichText::new(text).font(font(size::SM)).color(t.muted));
                        });
                    });
                    progress_bar(ui, *downloaded, *total);
                    let done = total.is_some_and(|tt| tt > 0 && *downloaded >= tt);
                    let note = if done { "Signature verified. Installing\u{2026}" } else { "Downloading and verifying the signature\u{2026}" };
                    ui.label(RichText::new(note).font(font(size::SM)).color(t.muted));
                }
                Phase::Installed(u) => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        Icon::CheckCircle.show(ui, 16.0, t.success);
                        ui.label(RichText::new(format!("Installed v{}. Restarting ezSsh\u{2026}", u.version)).font(font(size::SM)).color(t.success));
                    });
                }
                Phase::Error(msg) => widgets::error_strip(ui, msg),
            }
            if !matches!(dlg.phase, Phase::Downloading { .. } | Phase::Installed(_)) {
                let (sep, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter().hline(sep.x_range(), sep.center().y, Stroke::new(1.0, t.border));
                ui.add_enabled_ui(!busy, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if crate::form::checkbox(
                        ui,
                        &mut pre,
                        "Include pre-releases (beta)",
                        Some("Track the newest release, including pre-release builds, to test versions before they ship publicly. The offered build must have a higher version than this one."),
                    )
                    .changed()
                    {
                        channel = Some(pre);
                    }
                });
            }
        },
        Some(&mut |ui: &mut Ui| match &dlg.phase {
            Phase::Available(u) => {
                if u.installable() {
                    if widgets::Button::primary("Install & restart").icon(Icon::Download).show(ui).clicked() {
                        action = Some(Action::Install(u.clone()));
                    }
                } else if widgets::Button::primary("Open releases page").icon(Icon::Globe2).show(ui).clicked() {
                    action = Some(Action::OpenPage(u.release_page.clone()));
                }
                if widgets::Button::secondary("Later").show(ui).clicked() {
                    action = Some(Action::Close);
                }
            }
            phase => {
                // Right-to-left: the action goes first so it sits rightmost.
                if matches!(phase, Phase::UpToDate | Phase::Error(_)) && widgets::Button::secondary("Check again").icon(Icon::RefreshCw).show(ui).clicked() {
                    action = Some(Action::Recheck);
                }
                if widgets::Button::secondary("Close").enabled(!busy).show(ui).clicked() {
                    action = Some(Action::Close);
                }
            }
        }),
    );
    if let Some(on) = channel {
        action = Some(Action::Channel(on));
    }
    if out.dismissed() && !busy && action.is_none() {
        action = Some(Action::Close);
    }
    match action {
        Some(Action::Close) => app.updater.dialog = None,
        Some(Action::Recheck) => {
            if let Some(d) = app.updater.dialog.as_mut() {
                d.phase = Phase::Checking;
            }
            let pre = app.settings.bool("updates.preRelease");
            check(app, pre, false);
        }
        Some(Action::Install(u)) => install_update(app, u),
        Some(Action::OpenPage(url)) => {
            tracing::info!(%url, "update: opening the releases page");
            if let Err(e) = open::that_detached(&url) {
                app.toasts.danger("Could not open the browser", format!("{url}: {e}"));
            }
        }
        Some(Action::Channel(on)) => {
            app.set_setting("updates.preRelease", Value::Bool(on));
            app.updater.available = None;
            if let Some(d) = app.updater.dialog.as_mut() {
                d.phase = Phase::Checking;
            }
            check(app, on, false);
        }
        None => {}
    }
}

fn available(ui: &mut Ui, u: &Update, kind: Option<ezssh_core::updater::InstallKind>) {
    let t = tokens(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("Current").font(font(size::SM)).color(t.muted));
        ui.label(RichText::new(&u.current_version).font(egui::FontId::monospace(size::BASE)).color(t.fg));
        ui.label(RichText::new("\u{2192}").font(font(size::BASE)).color(t.muted));
        ui.label(RichText::new("New").font(font(size::SM)).color(t.muted));
        ui.label(RichText::new(&u.version).font(egui::FontId::monospace(size::BASE)).strong().color(t.accent));
        if u.pre_release {
            badge(ui, "PRE-RELEASE");
        }
    });
    if let Some(d) = u.date.as_deref() {
        let shown = super::backup::local_time(d, false);
        ui.label(RichText::new(format!("Published {shown}")).font(font(size::SM)).color(t.muted));
    }
    if let Some(notes) = u.notes.as_deref() {
        crate::form::caption(ui, "Release notes");
        egui::Frame::new()
            .fill(t.bg)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(radius::LG as u8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(notes).font(egui::FontId::monospace(size::SM)).color(t.fg)).wrap());
                });
            });
    }
    let text = match kind {
        Some(k) if u.installable() => format!(
            "Installing downloads the signed {}, checks it against the update key built into this copy and restarts ezSsh into the new version. Open SSH / WSL sessions will be closed.",
            k.label()
        ),
        Some(k) if k.self_updates() => {
            "This release has no installer for this kind of install. Download it from the releases page.".to_string()
        }
        Some(k) => format!("This copy is a {} and cannot replace itself. Download the new version from the releases page.", k.label()),
        None => String::new(),
    };
    para(ui, &text, t.muted);
}

fn badge(ui: &mut Ui, text: &str) {
    let t = tokens(ui.ctx());
    egui::Frame::new()
        .fill(alpha(t.warning, 0.14))
        .corner_radius(radius::PILL as u8)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                Icon::FlaskConical.show(ui, 11.0, t.warning);
                ui.label(RichText::new(text).font(font_semibold(size::XS)).color(t.warning));
            });
        });
}

fn progress_bar(ui: &mut Ui, value: u64, total: Option<u64>) {
    let t = tokens(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(r, radius::PILL, t.surface3);
    match total.filter(|t| *t > 0) {
        Some(tt) => {
            let f = (value as f32 / tt as f32).clamp(0.0, 1.0);
            let mut fill = r;
            fill.set_width(r.width() * f);
            ui.painter().rect_filled(fill, radius::PILL, t.accent);
        }
        None => {
            // Unknown size: a third-wide bar that pulses.
            let mut fill = r;
            fill.set_width(r.width() / 3.0);
            ui.painter().rect_filled(fill, radius::PILL, alpha(t.accent, widgets::pulse(ui.ctx())));
        }
    }
}

/// `formatBytes` of the TSX.
pub fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_check_cadence() {
        let day = 86_400;
        assert!(launch_check_due(None, 1_000 * day, 30));
        assert!(!launch_check_due(Some(990 * day), 1_000 * day, 30));
        assert!(launch_check_due(Some(970 * day), 1_000 * day, 30));
        // A clock that went backwards checks.
        assert!(launch_check_due(Some(1_001 * day), 1_000 * day, 30));
        // Interval below one day counts as one.
        assert!(!launch_check_due(Some(1_000 * day - 10), 1_000 * day, 0));
    }

    #[test]
    fn bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024 + 1024 * 1024 / 2), "5.5 MB");
    }
}
