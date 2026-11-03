//! The backup dialog: export an encrypted backup in three steps:
//! master-password reauth, backup passphrase + confirm, save-as dialog →
//! `backup_create`. Also holds the field helpers the backup / restore /
//! sync / import dialogs share (the card is `widgets::Dialog::data`).

use std::path::PathBuf;

use egui::{CursorIcon, Id, Response, RichText, Sense, Stroke, Ui};
use ezssh_core::api;
use ezssh_core::api::backup::BackupSummary;
use zeroize::Zeroizing;

use crate::app::App;
use crate::files::{Filter, Pick};
use crate::form;
use crate::icons::Icon;
use crate::style::{font, font_semibold, size, tokens};
use crate::widgets::{self, Button, Dialog, DialogFooter, DialogPart};

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
    let busy = dlg.busy || dlg.picking;
    let out = Dialog::data(Id::unique("backup-dialog"), 460.0)
        .header(Icon::PackageOpen, t.accent, "Export encrypted backup", None)
        .footer(DialogFooter::Inline)
        .busy(busy)
        .show(ctx, |ui, part| match (part, dlg.step) {
        (DialogPart::Body, Step::Reauth) => {
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
        (DialogPart::Body, Step::Passphrase) => {
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
        (DialogPart::Footer, step) => {
            let (label, ready) = match step {
                Step::Reauth => ("Continue", !dlg.master.is_empty()),
                Step::Passphrase => ("Save backup…", !dlg.passphrase.is_empty() && !dlg.confirm.is_empty()),
            };
            if widgets::Button::primary(label).enabled(!busy && ready).show(ui).clicked() {
                act = Some(if step == Step::Reauth { Act::Reauth } else { Act::Save });
            }
            if busy {
                form::spinner(ui, 12.0);
            }
            if Button::secondary("Cancel").enabled(!busy).show(ui).clicked() {
                act = Some(Act::Close);
            }
        }
    });
    if out.dismissed() && !busy {
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
            title: "Save encrypted ezSsh backup",
            file_name: format!("ezssh-backup-{}.json", today_utc()),
            filters: vec![Filter { name: "ezSsh backup", extensions: &["json"] }],
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

/// The result toast's body. Session passwords are counted
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

/// Today's date in UTC as `YYYY-MM-DD`.
fn today_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

// ------------------------------------------------------------------ fields

/// A muted label above a control; the control is labelled by it for
/// AccessKit.
pub(crate) fn labeled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let t = tokens(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let l = ui.label(RichText::new(label).font(font(size::SM)).color(t.fg2));
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
            Span::Muted => (font(size::SM), t.muted),
            Span::Strong => (font_semibold(size::SM), t.fg),
            Span::Mono => (egui::FontId::monospace(size::SM), t.muted),
        };
        job.append(text, 0.0, egui::TextFormat { font_id, color, ..Default::default() });
    }
    ui.add(egui::Label::new(job).wrap())
}

/// A core RFC 3339 timestamp in local time, `YYYY-MM-DD HH:MM:SS` (or
/// without seconds). Anything else is shown as stored.
pub(crate) fn local_time(s: &str, seconds: bool) -> String {
    local_time_in(s, &chrono::Local, seconds)
}

pub(crate) fn local_time_in<Tz: chrono::TimeZone>(s: &str, tz: &Tz, seconds: bool) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let fmt = if seconds { "%Y-%m-%d %H:%M:%S" } else { "%Y-%m-%d %H:%M" };
    match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(t) => t.with_timezone(tz).format(fmt).to_string(),
        Err(e) => {
            tracing::debug!(value = s, error = %e, "time not RFC 3339, shown as stored");
            s.to_string()
        }
    }
}

/// The inline `role="alert"` line (`text-danger text-xs` with an icon).
pub(crate) fn error_line(ui: &mut Ui, err: Option<&str>) {
    if let Some(err) = err {
        crate::widgets::error_strip(ui, err);
    }
}

/// The green confirmation line of the sync dialog.
pub(crate) fn success_line(ui: &mut Ui, msg: &str) {
    let t = tokens(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        Icon::Check.show(ui, 14.0, t.success);
        ui.add(egui::Label::new(RichText::new(msg).font(font(size::SM)).color(t.success)).wrap());
    });
}

// ------------------------------------------------------------------ buttons

/// An accent text link (`text-[11px] text-accent hover:underline`), with
/// its own AccessKit label.
pub(crate) fn link(ui: &mut Ui, text: &str, a11y_label: &str) -> Response {
    let t = tokens(ui.ctx());
    let g = ui.painter().layout_no_wrap(text.to_string(), font(size::SM), t.accent);
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
    fn backup_file_date() {
        let d = today_utc();
        assert_eq!((d.len(), &d[4..5], &d[7..8]), (10, "-", "-"));
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
