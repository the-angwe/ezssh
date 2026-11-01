//! `backup-dialog.tsx`: export an encrypted backup in three steps:
//! master-password reauth, backup passphrase + confirm, save-as dialog →
//! `backup_create`. Also holds the dialog shell, field and button helpers
//! the backup / restore / sync / import dialogs share.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{
    Align, Color32, CornerRadius, CursorIcon, Frame, Id, Layout, Margin, Pos2, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2, vec2,
};
use ezssh_core::api;
use ezssh_core::api::backup::BackupSummary;
use zeroize::Zeroizing;

use crate::app::App;
use crate::files::{Filter, Pick};
use crate::form;
use crate::icons::Icon;
use crate::style::{self, alpha, font, font_semibold, size, tokens};
use crate::widgets::{self, Tone};

/// Minimum backup / sync passphrase length (the core enforces it too).
pub(crate) const MIN_PASSPHRASE: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Step {
    #[default]
    Reauth,
    Passphrase,
}

#[derive(Default)]
pub struct BackupDialog {
    pub(crate) step: Step,
    master: Zeroizing<String>,
    passphrase: Zeroizing<String>,
    confirm: Zeroizing<String>,
    show: bool,
    busy: bool,
    /// The native save dialog is open.
    picking: bool,
    pub(crate) err: Option<String>,
    focused: Option<Step>,
    /// Save here instead of asking with the native dialog (headless tests).
    pub(crate) save_to: Option<PathBuf>,
}

enum Act {
    Close,
    Reauth,
    Save,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.backup.take() else {
        return;
    };
    let t = tokens(ctx);
    let mut act = None;
    let head = Head { icon: Icon::PackageOpen, icon_color: t.accent, title: "Export encrypted backup", subtitle: None };
    let busy = dlg.busy || dlg.picking;
    let closed = shell(ctx, Id::unique("backup-dialog"), 460.0, &head, FooterKind::Inline, busy, |ui, part| match (part, dlg.step) {
        (Part::Body, Step::Reauth) => {
            form::note(
                ui,
                Icon::Lock,
                t.warning,
                "Re-enter your master password. Exporting decrypts every credential in memory — this gate blocks walk-up exfiltration from an unlocked session.",
            );
            let r = labeled(ui, "Master password", |ui| form::secret(ui, &mut dlg.master, "", &mut dlg.show, None));
            autofocus(&r, &mut dlg.focused, Step::Reauth);
            if crate::form::entered(ui, &r) {
                act = Some(Act::Reauth);
            }
            error_line(ui, dlg.err.as_deref());
        }
        (Part::Body, Step::Passphrase) => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                rich(
                    ui,
                    &[
                        (Span::Muted, "The backup is wrapped under a "),
                        (Span::Strong, "separate"),
                        (Span::Muted, " passphrase (not your master password) so you can hand the file to someone else without sharing your vault password."),
                    ],
                );
                rich(ui, &[(Span::Strong, "If you forget this passphrase, the backup is unrecoverable.")]);
            });
            let r = labeled(ui, "Backup passphrase (≥ 8 chars)", |ui| form::secret(ui, &mut dlg.passphrase, "", &mut dlg.show, None));
            autofocus(&r, &mut dlg.focused, Step::Passphrase);
            let r = labeled(ui, "Confirm passphrase", |ui| password(ui, &mut dlg.confirm, dlg.show));
            if crate::form::entered(ui, &r) {
                act = Some(Act::Save);
            }
            error_line(ui, dlg.err.as_deref());
        }
        (Part::Footer, step) => {
            let (label, ready) = match step {
                Step::Reauth => ("Continue", !dlg.master.is_empty()),
                Step::Passphrase => ("Save backup…", !dlg.passphrase.is_empty() && !dlg.confirm.is_empty()),
            };
            if widgets::primary(ui, label, Tone::Accent, !busy && ready, size::SM).clicked() {
                act = Some(if step == Step::Reauth { Act::Reauth } else { Act::Save });
            }
            if busy {
                form::spinner(ui, 12.0);
            }
            if cancel_button(ui, !busy).clicked() {
                act = Some(Act::Close);
            }
        }
    });
    if closed {
        act = Some(Act::Close);
    }
    match act {
        Some(Act::Close) => {
            tracing::debug!("backup: closed");
            return; // dropping `dlg` zeroizes the typed secrets
        }
        Some(Act::Reauth) if !busy => reauth(app, &mut dlg),
        Some(Act::Save) if !busy => save(app, &mut dlg),
        _ => {}
    }
    app.dialogs.backup = Some(dlg);
}

fn reauth(app: &mut App, dlg: &mut BackupDialog) {
    if dlg.master.is_empty() {
        dlg.err = Some("Enter your master password".into());
        return;
    }
    dlg.err = None;
    dlg.busy = true;
    let pw = dlg.master.to_string();
    tracing::debug!("backup: verifying the master password");
    app.core.call(
        "vault_verify_password",
        move |st| async move { api::vault::vault_verify_password(&st, pw).await },
        |app, r| {
            let Some(d) = app.dialogs.backup.as_mut() else {
                return;
            };
            d.busy = false;
            match r {
                Ok(true) => {
                    tracing::debug!("backup: reauth ok");
                    d.step = Step::Passphrase;
                }
                Ok(false) => {
                    tracing::debug!("backup: reauth refused");
                    d.err = Some("Incorrect master password".into());
                }
                Err(e) => {
                    tracing::warn!("backup: verifying the master password failed: {e}");
                    d.err = Some(e.to_string());
                }
            }
        },
    );
}

fn save(app: &mut App, dlg: &mut BackupDialog) {
    if let Err(e) = check_passphrase(&dlg.passphrase, &dlg.confirm) {
        dlg.err = Some(e.into());
        return;
    }
    dlg.err = None;
    if let Some(path) = dlg.save_to.take() {
        create(app, dlg, path);
        return;
    }
    dlg.picking = true;
    app.pick_path(
        Pick::Save {
            title: "Save encrypted ezTerm backup",
            file_name: format!("ezterm-backup-{}.json", today_utc()),
            filters: vec![Filter { name: "ezTerm backup", extensions: &["json"] }],
        },
        |app, path| {
            let Some(mut d) = app.dialogs.backup.take() else {
                return;
            };
            d.picking = false;
            match path {
                Some(p) => create(app, &mut d, p),
                None => tracing::debug!("backup: save dialog cancelled"),
            }
            app.dialogs.backup = Some(d);
        },
    );
}

fn create(app: &mut App, dlg: &mut BackupDialog, path: PathBuf) {
    dlg.busy = true;
    let master = dlg.master.to_string();
    let pass = dlg.passphrase.to_string();
    let target = path.to_string_lossy().into_owned();
    tracing::info!(path = %target, "backup: writing");
    app.core.call(
        "backup_create",
        move |st| async move { api::backup::backup_create(&st, target, master, pass).await },
        move |app, r| match r {
            Ok(summary) => {
                tracing::info!(path = %path.display(), ?summary, "backup: saved");
                app.dialogs.backup = None;
                app.toasts.success("Backup saved", summary_text(&summary));
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), "backup: failed: {e}");
                if let Some(d) = app.dialogs.backup.as_mut() {
                    d.busy = false;
                    d.err = Some(e.to_string());
                }
            }
        },
    );
}

/// The toast body of `sessions-sidebar.tsx`. Session passwords are counted
/// with the credentials, so the label says so.
fn summary_text(s: &BackupSummary) -> String {
    let parts = [
        count(s.sessions, "session", "sessions"),
        count(s.credentials, "credential (keys and session passwords)", "credentials (keys and session passwords)"),
        count(s.folders, "folder", "folders"),
        count(s.known_hosts, "known host", "known hosts"),
    ];
    join_or(&parts, "empty backup")
}

/// Validate a new passphrase pair (the dialogs' wording).
pub(crate) fn check_passphrase(pass: &str, confirm: &str) -> Result<(), &'static str> {
    if pass.chars().count() < MIN_PASSPHRASE {
        return Err("Passphrase must be at least 8 characters");
    }
    if pass != confirm {
        return Err("Passphrases do not match");
    }
    Ok(())
}

/// "3 sessions", or `None` for zero.
pub(crate) fn count(n: usize, one: &str, many: &str) -> Option<String> {
    match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    }
}

/// The non-empty parts joined with ", ", or `empty`.
pub(crate) fn join_or(parts: &[Option<String>], empty: &str) -> String {
    let s = parts.iter().flatten().cloned().collect::<Vec<_>>().join(", ");
    if s.is_empty() { empty.to_string() } else { s }
}

/// Today's date in UTC as `YYYY-MM-DD` (the webview's `toISOString`).
fn today_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_else(|e| {
        tracing::warn!("backup: system clock before 1970: {e}");
        0
    });
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m as u32, d as u32)
}

// ------------------------------------------------------------------ shell

/// Header of a [`shell`] dialog.
pub(crate) struct Head<'a> {
    pub icon: Icon,
    pub icon_color: Color32,
    pub title: &'a str,
    /// Muted mono line under the title (a file path), elided.
    pub subtitle: Option<&'a str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FooterKind {
    /// Buttons at the end of the body (`<footer className="pt-2">`).
    Inline,
    /// A bordered strip under the scrolling body.
    Strip,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Body,
    /// Laid out right to left: add the primary button first. Leftover
    /// space on the left can hold more (`ui.with_layout(left_to_right)`).
    Footer,
}

const HEADER_H: f32 = 56.0;
const FOOTER_H: f32 = 52.0;

/// The dialog card of the data dialogs (`w-[460px]` / `w-[620px]`,
/// `max-h-[90vh]`): header with icon, title, subtitle and close button; a
/// scrolling 20 px padded body; a footer. `add` is called once for the
/// body and once for the footer, so both can borrow the dialog state.
/// Returns true when the user asked to close (✕, Escape, backdrop) and
/// the dialog is not `busy`.
pub(crate) fn shell(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    head: &Head<'_>,
    footer: FooterKind,
    busy: bool,
    mut add: impl FnMut(&mut Ui, Part),
) -> bool {
    let t = tokens(ctx);
    let screen = ctx.content_rect();
    let width = width.min(screen.width() - 32.0);
    let frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(3))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(115) });
    let mut close = false;
    let resp = egui::Modal::new(id).backdrop_color(Color32::from_black_alpha(128)).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        // Header.
        let (hrect, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
        ui.painter().hline(hrect.x_range(), hrect.bottom(), Stroke::new(1.0, t.border));
        let inner = hrect.shrink2(vec2(20.0, 0.0));
        let icon_r = Rect::from_center_size(Pos2::new(inner.left() + 8.0, inner.center().y), Vec2::splat(16.0));
        head.icon.paint(ui, icon_r, 16.0, head.icon_color);
        let text_r = Rect::from_min_max(Pos2::new(icon_r.right() + 12.0, inner.top()), Pos2::new(inner.right() - 36.0, inner.bottom()));
        match head.subtitle {
            Some(sub) => {
                let top = Rect::from_min_max(text_r.min, Pos2::new(text_r.right(), text_r.center().y));
                let bottom = Rect::from_min_max(Pos2::new(text_r.left(), text_r.center().y), text_r.max);
                title_label(ui, top, RichText::new(head.title).font(font_semibold(size::BASE)).color(t.fg), Align::BOTTOM);
                title_label(ui, bottom, RichText::new(sub).font(egui::FontId::monospace(size::XS)).color(t.muted), Align::TOP);
            }
            None => title_label(ui, text_r, RichText::new(head.title).font(font_semibold(size::BASE)).color(t.fg), Align::Center),
        }
        let x_r = Rect::from_center_size(Pos2::new(inner.right() - 12.0, inner.center().y), Vec2::splat(24.0));
        let x = ui.interact(x_r, id.with("close"), if busy { Sense::hover() } else { Sense::click() });
        widgets::paint_icon_button(ui, &x, x_r, Icon::X, 14.0, false, None);
        widgets::a11y(&x, egui::Role::Button, "Close");
        if x.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            close = true;
        }
        // Body.
        let foot_h = if footer == FooterKind::Strip { FOOTER_H } else { 0.0 };
        let max_body = (screen.height() * 0.9 - HEADER_H - foot_h).max(120.0);
        // A Modal lays out in last frame's size, and a ScrollArea never
        // grows past the space it is offered, so a body that fits is laid
        // out plainly (the card follows its height); only one taller than
        // `max_body` (judged on last frame's height) scrolls, at that height.
        let content_h_id = id.with("content-h");
        let last_h: f32 = ui.ctx().data(|d| d.get_temp(content_h_id)).unwrap_or(0.0);
        let mut body = |ui: &mut Ui| {
            Frame::new()
                .inner_margin(Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(width - 40.0);
                    ui.spacing_mut().item_spacing.y = 16.0;
                    add(ui, Part::Body);
                    if footer == FooterKind::Inline {
                        ui.add_space(-4.0);
                        ui.allocate_ui_with_layout(vec2(width - 40.0, 28.0), Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            add(ui, Part::Footer);
                        });
                    }
                })
                .response
                .rect
                .height()
        };
        let content_h = if last_h <= max_body {
            body(ui)
        } else {
            egui::ScrollArea::vertical()
                .id_salt(id.with("body"))
                .max_height(max_body)
                .min_scrolled_height(max_body)
                .auto_shrink(false)
                .show(ui, |ui| body(ui))
                .inner
        };
        if content_h != last_h {
            ui.ctx().data_mut(|d| d.insert_temp(content_h_id, content_h));
            ui.ctx().request_repaint();
        }
        // Footer strip.
        if footer == FooterKind::Strip {
            let (strip, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
            ui.painter().rect_filled(strip, CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, style::over(t.surface, t.surface2, 0.3));
            ui.painter().hline(strip.x_range(), strip.top(), Stroke::new(1.0, t.border));
            ui.scope_builder(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(20.0, 12.0))), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    add(ui, Part::Footer);
                });
            });
        }
    });
    (close || resp.should_close()) && !busy
}

/// A truncating label in `rect` (full text for AccessKit and on hover).
/// A child Ui, so the parent's cursor stays below the header.
fn title_label(ui: &mut Ui, rect: Rect, text: RichText, valign: Align) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(valign)));
    child.add(egui::Label::new(text).truncate());
}

// ------------------------------------------------------------------ fields

/// A muted label above a control; the control is labelled by it for
/// AccessKit.
pub(crate) fn labeled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let t = tokens(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let l = ui.label(RichText::new(label).font(font(size::XS)).color(t.muted));
        add(ui).labelled_by(l.id)
    })
    .inner
}

/// A plain password `.input` (the confirm fields, which share the first
/// field's eye toggle).
pub(crate) fn password(ui: &mut Ui, value: &mut String, shown: bool) -> Response {
    let mut o = form::InputOpts::new("");
    o.password = !shown;
    let r = form::input(ui, value, o);
    // Also while revealed.
    form::forget_history(ui, &r);
    r
}

/// `autoFocus`: focus `r` the first frame `step` is shown.
pub(crate) fn autofocus<S: PartialEq + Copy>(r: &Response, done: &mut Option<S>, step: S) {
    if *done != Some(step) {
        r.request_focus();
        *done = Some(step);
    }
}

/// Text styles of [`rich`].
#[derive(Clone, Copy)]
pub(crate) enum Span {
    Muted,
    Strong,
    Mono,
}

/// One wrapping paragraph of mixed `text-xs` spans (`<strong>`,
/// `font-mono`).
pub(crate) fn rich(ui: &mut Ui, parts: &[(Span, &str)]) -> Response {
    let t = tokens(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    for (span, text) in parts {
        let (font_id, color) = match span {
            Span::Muted => (font(size::XS), t.muted),
            Span::Strong => (font_semibold(size::XS), t.fg),
            Span::Mono => (egui::FontId::monospace(size::XS), t.muted),
        };
        job.append(text, 0.0, egui::TextFormat { font_id, color, ..Default::default() });
    }
    ui.add(egui::Label::new(job).wrap())
}

/// The inline `role="alert"` line (`text-danger text-xs` with an icon).
pub(crate) fn error_line(ui: &mut Ui, err: Option<&str>) {
    let Some(err) = err else {
        return;
    };
    let t = tokens(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        Icon::AlertTriangle.show(ui, 12.0, t.danger);
        ui.add(egui::Label::new(RichText::new(err).font(font(size::XS)).color(t.danger)).wrap());
    });
}

/// The green confirmation line of the sync dialog.
pub(crate) fn success_line(ui: &mut Ui, msg: &str) {
    let t = tokens(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        Icon::Check.show(ui, 12.0, t.success);
        ui.add(egui::Label::new(RichText::new(msg).font(font(size::XS)).color(t.success)).wrap());
    });
}

// ------------------------------------------------------------------ buttons

/// "Cancel" ghost button, inert while busy.
pub(crate) fn cancel_button(ui: &mut Ui, enabled: bool) -> Response {
    ui.add_enabled_ui(enabled, |ui| widgets::ghost(ui, "Cancel", size::SM)).inner
}

/// A bordered footer button with an optional icon and colour (`Push now`,
/// `Pull from cloud`, the danger-outlined `Disable`).
pub(crate) fn outline(ui: &mut Ui, icon: Option<Icon>, text: &str, color: Option<Color32>, enabled: bool) -> Response {
    let t = tokens(ui.ctx());
    let fg = color.unwrap_or(t.fg);
    let g = ui.painter().layout_no_wrap(text.to_string(), font(size::SM), fg);
    let icon_w = if icon.is_some() { 18.0 } else { 0.0 };
    let w = g.size().x + icon_w + 24.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, (g.size().y + 12.0).max(26.0)), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let fade = |c: Color32| if enabled { c } else { alpha(c, 0.5) };
        if enabled && resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, if color.is_some() { alpha(fg, 0.1) } else { t.surface2 });
        }
        let border = if color.is_some() { alpha(fg, 0.4) } else { t.border };
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, fade(border)), StrokeKind::Inside);
        let mut x = rect.left() + 12.0;
        if let Some(icon) = icon {
            icon.paint(ui, Rect::from_center_size(Pos2::new(x + 6.0, rect.center().y), Vec2::splat(12.0)), 12.0, fade(fg));
            x += icon_w;
        }
        ui.painter().galley_with_override_text_color(Pos2::new(x, rect.center().y - g.size().y / 2.0), g, fade(fg));
        widgets::focus_ring(ui, &resp, rect);
    }
    widgets::a11y(&resp, egui::Role::Button, text);
    resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed })
}

/// An accent text link (`text-[11px] text-accent hover:underline`), with
/// its own AccessKit label.
pub(crate) fn link(ui: &mut Ui, text: &str, a11y_label: &str) -> Response {
    let t = tokens(ui.ctx());
    let g = ui.painter().layout_no_wrap(text.to_string(), font(size::XS), t.accent);
    let (rect, resp) = ui.allocate_exact_size(g.size(), Sense::click());
    let hovered = resp.hovered();
    ui.painter().galley(rect.min, g, t.accent);
    if hovered {
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.accent));
    }
    widgets::focus_ring(ui, &resp, rect);
    widgets::a11y(&resp, egui::Role::Button, a11y_label);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(today_utc().len(), 10);
    }

    #[test]
    fn passphrase_rules_and_summary() {
        assert_eq!(check_passphrase("short", "short"), Err("Passphrase must be at least 8 characters"));
        assert_eq!(check_passphrase("long enough", "long enougH"), Err("Passphrases do not match"));
        assert_eq!(check_passphrase("long enough", "long enough"), Ok(()));
        let s = BackupSummary { folders: 1, sessions: 2, credentials: 0, known_hosts: 0, settings: 9 };
        assert_eq!(summary_text(&s), "2 sessions, 1 folder");
        assert_eq!(summary_text(&BackupSummary::default()), "empty backup");
    }
}
