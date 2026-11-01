//! The vault dialogs: `change-password-dialog.tsx` and
//! `recovery-code-dialog.tsx` (sidebar root menu), and the unlock screen's
//! `recovery-unlock-dialog.tsx` and `reset-vault-dialog.tsx`, with the card
//! they share (title strip, padded body, optional footer strip).
//!
//! Typed secrets live in `Zeroizing` strings, so closing a dialog (dropping
//! its state) wipes them. Escape and the backdrop follow each TSX file's
//! gating: never while a call runs or while a one-time result is shown.

use egui::text::LayoutJob;
use egui::{Align, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextFormat, Ui, Vec2, vec2};
use ezssh_core::{AppError, api};
use zeroize::Zeroizing;

use crate::app::{App, Screen};
use crate::form;
use crate::icons::Icon;
use crate::style::{self, alpha, font, font_semibold, size, tokens};
use crate::tabs::TabId;
use crate::views::unlock::{self, UnlockState};
use crate::widgets::{self, Tone};

/// The word the reset dialog asks for.
const CONFIRM_WORD: &str = "DELETE";
/// What `api::vault::vault_reset` wants once the user typed the word (the
/// Tauri build maps it the same way in `lib/tauri.ts`).
const RESET_TOKEN: &str = "DELETE-MY-VAULT";

// ---------------------------------------------------------------- shared card

/// What the user did to the card's surroundings this frame.
pub(crate) struct ShellOut {
    pub backdrop: bool,
    pub escape: bool,
}

/// The dialog card of these TSX files: a title strip (`bg-surface2/30`,
/// optional accent icon), a 16 px padded body with 12 px between rows, and
/// an optional footer strip with right-aligned buttons laid out right to
/// left. Backdrop clicks and Escape are reported, never acted on, so each
/// dialog can gate them.
pub(crate) fn shell(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    icon: Option<Icon>,
    title: &str,
    body: impl FnOnce(&mut Ui),
    footer: Option<&mut dyn FnMut(&mut Ui)>,
) -> ShellOut {
    let t = tokens(ctx);
    let frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(3))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(115) });
    let width = width.min(ctx.content_rect().width() - 32.0);
    let strip_fill = style::over(t.surface, t.surface2, 0.3);
    let resp = egui::Modal::new(id).backdrop_color(Color32::from_black_alpha(128)).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        let (head, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
        ui.painter().rect_filled(head, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, strip_fill);
        ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, t.border));
        let mut head_ui = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(16.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        head_ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(icon) = icon {
            icon.show(&mut head_ui, 14.0, t.accent);
        }
        head_ui.label(RichText::new(title).font(font_semibold(size::SM)).color(t.fg));

        Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(width - 32.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            body(ui);
        });
        if let Some(footer) = footer {
            let r = ui.available_rect_before_wrap();
            let strip = Rect::from_min_size(r.min, vec2(width, 52.0));
            ui.painter().rect_filled(strip, CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, strip_fill);
            ui.painter().hline(strip.x_range(), strip.top(), Stroke::new(1.0, t.border));
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(16.0, 12.0))).layout(Layout::right_to_left(Align::Center)));
            row.spacing_mut().item_spacing.x = 8.0;
            footer(&mut row);
            ui.allocate_rect(strip, Sense::hover());
        }
    });
    let backdrop = resp.backdrop_response.clicked();
    let escape = resp.is_top_modal && !resp.any_popup_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    ShellOut { backdrop, escape }
}

/// The bordered secondary button (`border border-border rounded`), with an
/// optional leading icon and a disabled state. `compact` is the `px-2 py-1
/// text-xs` size of in-card buttons.
pub(crate) fn ghost_button(ui: &mut Ui, icon: Option<Icon>, text: &str, enabled: bool, compact: bool) -> Response {
    let t = tokens(ui.ctx());
    let (text_size, pad, icon_size, min_h) = if compact { (size::XS, vec2(8.0, 4.0), 12.0, 22.0) } else { (size::SM, vec2(12.0, 6.0), 12.0, 26.0) };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font(text_size), t.fg);
    let icon_w = if icon.is_some() { icon_size + if compact { 4.0 } else { 6.0 } } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 2.0 * pad.x, (galley.size().y + 2.0 * pad.y).max(min_h));
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let fade = if enabled { 1.0 } else { 0.5 };
        if enabled && resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, t.surface2);
        }
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, alpha(t.border, fade)), StrokeKind::Inside);
        let mut x = rect.left() + pad.x;
        if let Some(icon) = icon {
            icon.paint(ui, Rect::from_center_size(Pos2::new(x + icon_size / 2.0, rect.center().y), Vec2::splat(icon_size)), icon_size, alpha(t.fg, fade));
            x += icon_w;
        }
        ui.painter().galley_with_override_text_color(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, alpha(t.fg, fade));
        widgets::focus_ring(ui, &resp, rect);
    }
    widgets::a11y(&resp, egui::Role::Button, text);
    resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed })
}

/// `widgets::primary_with_icon` that also tells AccessKit when it is
/// disabled (the shared one reports the hover-only response as enabled).
pub(crate) fn primary_button(ui: &mut Ui, icon: Option<Icon>, text: &str, tone: Tone, enabled: bool) -> Response {
    let resp = widgets::primary_with_icon(ui, icon, text, tone, enabled, size::SM);
    if !enabled {
        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_disabled());
    }
    resp
}

/// A right-aligned row of buttons (add the rightmost first). Fixed height:
/// a centred right-to-left layout would otherwise claim every pixel below.
pub(crate) fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui);
    });
}

/// A wrapping paragraph of `text-xs` (or `text-[11px]`) copy.
pub(crate) fn para(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.add(egui::Label::new(RichText::new(text).font(font(size::XS)).color(color)).wrap())
}

/// A wrapping paragraph with an inline `<code>` path chip in the middle.
pub(crate) fn para_with_code(ui: &mut Ui, before: &str, code: &str, after: &str) -> Response {
    let t = tokens(ui.ctx());
    let mut job = LayoutJob::default();
    let plain = TextFormat { font_id: font(size::XS), color: t.muted, ..TextFormat::default() };
    job.append(before, 0.0, plain.clone());
    job.append(code, 0.0, TextFormat { font_id: FontId::monospace(size::XS - 0.5), color: t.fg, background: alpha(t.surface2, 0.6), ..TextFormat::default() });
    job.append(after, 0.0, plain);
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap())
}

/// A labelled password field (`PasswordField`): muted label above, the
/// field below; `eye` adds the show / hide toggle inside it.
fn password_field(ui: &mut Ui, label: &str, value: &mut String, shown: bool, eye: Option<&mut bool>, enabled: bool) -> Response {
    let t = tokens(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let label_resp = ui.label(RichText::new(label).font(font(size::XS)).color(t.muted));
        let right = if eye.is_some() { 36 } else { 8 };
        let edit = egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .font(font(size::BASE))
            .margin(Margin { left: 8, right, top: 6, bottom: 6 })
            .password(!shown)
            .background_color(t.surface2);
        let resp = ui.add_enabled(enabled, edit).labelled_by(label_resp.id);
        crate::form::forget_history(ui, &resp);
        let stroke = if resp.has_focus() { Stroke::new(1.0, t.accent) } else { Stroke::new(1.0, t.border) };
        ui.painter().rect_stroke(resp.rect, 2.0, stroke, StrokeKind::Inside);
        if let Some(eye) = eye {
            let r = Rect::from_center_size(Pos2::new(resp.rect.right() - 18.0, resp.rect.center().y), vec2(28.0, 28.0));
            let b = ui.interact(r, resp.id.with("eye"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            widgets::paint_icon_button(ui, &b, r, if *eye { Icon::EyeOff } else { Icon::Eye }, 13.0, false, None);
            widgets::a11y(&b, egui::Role::Button, if *eye { "Hide password" } else { "Show password" });
            if b.on_hover_text(if *eye { "Hide" } else { "Show" }).clicked() {
                *eye = !*eye;
            }
        }
        resp
    })
    .inner
}

/// A plain `.input` with an accessible name (the recovery code / DELETE
/// boxes), optionally mono and centred.
fn text_field(ui: &mut Ui, value: &mut String, hint: &str, name: &str, mono_size: Option<f32>, center: bool) -> Response {
    let t = tokens(ui.ctx());
    let font_id = mono_size.map_or_else(|| font(size::BASE), FontId::monospace);
    let edit = egui::TextEdit::singleline(value)
        .hint_text(RichText::new(hint).color(alpha(t.muted, 0.8)))
        .desired_width(ui.available_width())
        .font(font_id)
        .align(if center { egui::Align2::CENTER_TOP } else { egui::Align2::LEFT_TOP })
        .margin(Margin::symmetric(8, 6))
        .background_color(t.surface2);
    let resp = ui.add(edit).accessible_name(name);
    // The recovery code is a secret too.
    crate::form::forget_history(ui, &resp);
    let stroke = if resp.has_focus() { Stroke::new(1.0, t.accent) } else { Stroke::new(1.0, t.border) };
    ui.painter().rect_stroke(resp.rect, 2.0, stroke, StrokeKind::Inside);
    resp
}

/// The success layout of the change-password and reset dialogs: a 22 px
/// check beside the text.
fn success_body(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = tokens(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        Icon::CheckCircle.show(ui, 22.0, t.success);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            add(ui);
        });
    });
}

/// Every tab, closed. A password change or a reset ends all connections in
/// the core, and the webview's page reload dropped its tabs with them.
fn close_all_tabs(app: &mut App) {
    let ids: Vec<TabId> = app.tabs.tabs.iter().map(|t| t.id).collect();
    tracing::debug!(tabs = ids.len(), "vault: closing every tab");
    for id in ids {
        app.close_tab(id);
    }
}

/// The sidebar's dialogs (shown every frame from the shell).
pub fn show(app: &mut App, ctx: &egui::Context) {
    show_change_password(app, ctx);
    show_recovery_code(app, ctx);
}

// ---------------------------------------------------------------- change password

/// `change-password-dialog.tsx`.
#[derive(Default)]
pub struct ChangePassword {
    old: Zeroizing<String>,
    new: Zeroizing<String>,
    confirm: Zeroizing<String>,
    show: bool,
    error: Option<String>,
    busy: bool,
    /// Where the pre-change snapshot went: the success card is showing.
    done: Option<String>,
    focused: bool,
}

impl ChangePassword {
    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.done.is_some()
    }

    fn wipe(&mut self) {
        self.old = Zeroizing::default();
        self.new = Zeroizing::default();
        self.confirm = Zeroizing::default();
    }
}

fn show_change_password(app: &mut App, ctx: &egui::Context) {
    let Some(dlg) = app.dialogs.change_password.as_mut() else {
        return;
    };
    let t = tokens(ctx);
    let id = Id::unique("change-password");

    if let Some(snapshot) = &dlg.done {
        let mut finish = false;
        // Backdrop and Escape are inert here: the only way on is the
        // button, as in the TSX (it reloaded the page).
        shell(
            ctx,
            id,
            460.0,
            None,
            "Master password changed",
            |ui| {
                success_body(ui, |ui| {
                    para(
                        ui,
                        "All credentials and sync passphrases were re-encrypted under the new key. Active SSH/SFTP sessions and cloud sync were closed. The vault is now locked \u{2014} unlock with your new password to continue.",
                        t.muted,
                    );
                    para_with_code(
                        ui,
                        "A snapshot of the previous vault was written to ",
                        snapshot,
                        ". The five most-recent snapshots are kept (and pruned after 30 days). To roll back manually, close ezSsh and replace the active database with this file.",
                    );
                    para(ui, "Any previously generated recovery code is now invalid. We recommend generating a new one right after unlocking.", t.warning);
                });
            },
            Some(&mut |ui: &mut Ui| {
                if primary_button(ui, None, "Unlock", Tone::Accent, true).clicked() {
                    finish = true;
                }
            }),
        );
        if finish {
            finish_change_password(app);
        }
        return;
    }

    let first = !dlg.focused;
    dlg.focused = true;
    let busy = dlg.busy;
    let can_submit = !busy && !dlg.old.is_empty() && !dlg.new.is_empty() && !dlg.confirm.is_empty();
    let mut submit = false;
    let mut close = false;
    let out = shell(
        ctx,
        id,
        460.0,
        None,
        "Change master password",
        |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                Icon::KeyRound.show(ui, 13.0, t.muted);
                para(ui, "Re-encrypts every credential and sync passphrase. A snapshot of the current vault is saved first.", t.muted);
            });
            let shown = dlg.show;
            let r = password_field(ui, "Current password", &mut dlg.old, shown, Some(&mut dlg.show), !busy);
            if first {
                r.request_focus();
            }
            let mut enter = crate::form::entered(ui, &r);
            let r = password_field(ui, "New password", &mut dlg.new, shown, None, !busy);
            enter |= crate::form::entered(ui, &r);
            if !dlg.new.is_empty() {
                let w = ui.available_width();
                unlock::strength_meter(ui, &dlg.new, w);
            }
            let r = password_field(ui, "Confirm new password", &mut dlg.confirm, shown, None, !busy);
            enter |= crate::form::entered(ui, &r);
            if let Some(e) = &dlg.error {
                widgets::error_strip(ui, e);
            }
            ui.add_space(4.0);
            button_row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // Enter submits only when the button would (HTML implicit
                // submission skips a disabled submit button).
                if primary_button(ui, None, if busy { "Changing\u{2026}" } else { "Change password" }, Tone::Accent, can_submit).clicked() || (enter && can_submit) {
                    submit = true;
                }
                if ghost_button(ui, None, "Cancel", !busy, false).clicked() {
                    close = true;
                }
            });
        },
        None,
    );
    if (out.backdrop || out.escape) && !busy {
        close = true;
    }
    if close {
        tracing::debug!("vault: change password dialog closed");
        app.dialogs.change_password = None;
    } else if submit {
        submit_change_password(app);
    }
}

fn submit_change_password(app: &mut App) {
    let Some(dlg) = app.dialogs.change_password.as_mut() else {
        return;
    };
    dlg.error = None;
    // Same checks and order as the TSX (the core then asks for 12+
    // characters and two character classes).
    let refusal = if dlg.new.chars().count() < 8 {
        Some("New password must be at least 8 characters")
    } else if *dlg.new != *dlg.confirm {
        Some("New passwords do not match")
    } else if *dlg.old == *dlg.new {
        Some("New password must differ from old password")
    } else {
        None
    };
    if let Some(msg) = refusal {
        tracing::debug!(reason = msg, "vault: change password refused before the call");
        dlg.error = Some(msg.to_string());
        return;
    }
    dlg.busy = true;
    // The API takes owned strings and zeroizes them itself.
    let old = String::clone(&dlg.old);
    let new = String::clone(&dlg.new);
    tracing::debug!("vault: changing the master password");
    app.core.call(
        "vault_change_password",
        move |st| async move { api::vault::vault_change_password(&st, old, new).await },
        |app, r| {
            // The core locked the vault and closed every connection: the
            // tabs are dead whatever happened to the dialog meanwhile.
            if r.is_ok() {
                close_all_tabs(app);
            }
            let Some(dlg) = app.dialogs.change_password.as_mut() else {
                // Only an idle lock can close it while busy.
                match r {
                    Ok(_) => tracing::warn!("vault: password changed after the dialog was closed"),
                    Err(e) => tracing::warn!("vault: change password failed after the dialog was closed: {e}"),
                }
                return;
            };
            dlg.busy = false;
            match r {
                Ok(res) => {
                    tracing::info!(snapshot = %res.snapshot_path, "vault: master password changed, vault locked");
                    dlg.wipe();
                    dlg.done = Some(res.snapshot_path);
                }
                Err(e) => {
                    tracing::warn!("vault: change password failed: {e}");
                    dlg.error = Some(e.to_string());
                }
            }
        },
    );
}

/// The webview reloaded the page here. The core already locked the vault
/// and closed every connection, so the native equivalent is: close the
/// (dead) tabs and show the unlock screen. Unlocking again reloads
/// settings, sessions and credentials (`App::on_unlocked`), which is what
/// the reload achieved.
fn finish_change_password(app: &mut App) {
    tracing::info!("vault: password changed, back to the unlock screen");
    close_all_tabs(app);
    // Resets every dialog, this one included.
    app.lock();
}

// ---------------------------------------------------------------- recovery code

#[derive(Default)]
enum Phase {
    /// Asking whether a code exists (never generates on open: that would
    /// silently replace a code the user saved).
    #[default]
    Probing,
    Fresh,
    /// A code exists: replacing it needs the "I understand" box.
    Replace,
    Verifying {
        replacing: bool,
    },
    Shown {
        code: Zeroizing<String>,
        snapshot: Option<String>,
    },
    Failed(String),
}

/// `recovery-code-dialog.tsx`: re-authenticate, (re)generate, show once.
#[derive(Default)]
pub struct RecoveryCode {
    phase: Phase,
    probed: bool,
    password: Zeroizing<String>,
    show_pw: bool,
    understand: bool,
    error: Option<String>,
    /// "Copied" shows until this time (`ctx` seconds).
    copied_until: Option<f64>,
    focused: bool,
}

impl RecoveryCode {
    #[cfg(test)]
    pub fn code(&self) -> Option<&str> {
        match &self.phase {
            Phase::Shown { code, .. } => Some(code.as_str()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn probing(&self) -> bool {
        matches!(self.phase, Phase::Probing)
    }
}

enum CodeAction {
    Close,
    Submit,
    Regenerate,
}

fn show_recovery_code(app: &mut App, ctx: &egui::Context) {
    let Some(dlg) = app.dialogs.recovery_code.as_mut() else {
        return;
    };
    if !dlg.probed {
        dlg.probed = true;
        tracing::debug!("vault: recovery code dialog, probing the status");
        app.core.call(
            "vault_recovery_status",
            |st| async move { api::vault::vault_recovery_status(&st).await },
            |app, r| {
                let Some(dlg) = app.dialogs.recovery_code.as_mut().filter(|d| matches!(d.phase, Phase::Probing)) else {
                    tracing::debug!("vault: recovery status arrived after the dialog moved on");
                    return;
                };
                dlg.phase = match r {
                    Ok(s) => {
                        tracing::debug!(provisioned = s.provisioned, "vault: recovery status");
                        if s.provisioned { Phase::Replace } else { Phase::Fresh }
                    }
                    Err(e) => {
                        tracing::warn!("vault: recovery status failed: {e}");
                        Phase::Failed(e.to_string())
                    }
                };
            },
        );
    }

    let t = tokens(ctx);
    let now = ctx.input(|i| i.time);
    let mut action = None;
    let out = shell(
        ctx,
        Id::unique("recovery-code"),
        520.0,
        Some(Icon::KeyRound),
        "Recovery code",
        |ui| match &dlg.phase {
            Phase::Probing => {
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 80.0), Layout::left_to_right(Align::Center).with_main_align(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    form::spinner(ui, 18.0);
                    ui.label(RichText::new("Checking\u{2026}").font(font(size::SM)).color(t.muted));
                });
            }
            Phase::Failed(msg) => {
                para(ui, msg, t.danger);
            }
            Phase::Fresh | Phase::Replace | Phase::Verifying { .. } => {
                let verifying = matches!(dlg.phase, Phase::Verifying { .. });
                let replacing = matches!(dlg.phase, Phase::Replace | Phase::Verifying { replacing: true });
                if replacing {
                    form::note(
                        ui,
                        Icon::AlertTriangle,
                        t.warning,
                        "A recovery code already exists. Generating a new one will invalidate the old one. A snapshot of the current vault will be saved before the change.",
                    );
                } else {
                    para(ui, "The recovery code lets you unlock the vault if you forget your master password. You will see it exactly once. Save it somewhere safe.", t.muted);
                }
                let shown = dlg.show_pw;
                let r = password_field(ui, "Confirm with master password", &mut dlg.password, shown, Some(&mut dlg.show_pw), !verifying);
                if !dlg.focused {
                    dlg.focused = true;
                    r.request_focus();
                }
                let enter = crate::form::entered(ui, &r);
                if matches!(dlg.phase, Phase::Replace) {
                    ui.checkbox(
                        &mut dlg.understand,
                        RichText::new("I understand the existing recovery code will stop working as soon as the new one is generated.").font(font(size::XS)).color(t.fg),
                    );
                }
                if let Some(e) = &dlg.error {
                    widgets::error_strip(ui, e);
                }
                let can_submit = !verifying && !dlg.password.is_empty() && !(matches!(dlg.phase, Phase::Replace) && !dlg.understand);
                button_row(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if primary_button(ui, None, if verifying { "Generating\u{2026}" } else { "Generate code" }, Tone::Accent, can_submit).clicked() || (enter && can_submit) {
                        action = Some(CodeAction::Submit);
                    }
                    if ghost_button(ui, None, "Cancel", !verifying, false).clicked() {
                        action = Some(CodeAction::Close);
                    }
                });
            }
            Phase::Shown { code, snapshot } => {
                form::note(
                    ui,
                    Icon::AlertTriangle,
                    t.warning,
                    "Save this code somewhere safe (printout, password manager) now \u{2014} we will never show it again. Using it to unlock the vault invalidates it, so plan to set a new master password right after.",
                );
                Frame::new()
                    .fill(alpha(t.surface2, 0.6))
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(2))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add(egui::Label::new(RichText::new(code.as_str()).font(FontId::monospace(size::LG)).color(t.fg)).selectable(true).wrap())
                                .accessible_name("Recovery code");
                        });
                    });
                if let Some(path) = snapshot {
                    para_with_code(ui, "A snapshot of the previous vault was saved to ", path, ".");
                }
                ui.horizontal(|ui| {
                    let copied = dlg.copied_until.is_some_and(|until| now < until);
                    if ghost_button(ui, Some(Icon::Copy), if copied { "Copied" } else { "Copy" }, true, true).clicked() {
                        // Best effort, as in the TSX: the code stays on screen.
                        ui.ctx().copy_text(code.to_string());
                        tracing::debug!("vault: recovery code copied to the clipboard");
                        dlg.copied_until = Some(now + 1.5);
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1500));
                    }
                    let regen_w = 12.0 + 4.0 + 16.0 + ui.painter().layout_no_wrap("Regenerate".into(), font(size::XS), t.fg).size().x;
                    ui.add_space((ui.available_width() - regen_w).max(0.0));
                    if ghost_button(ui, Some(Icon::RefreshCcw), "Regenerate", true, true).on_hover_text("Generate a different code (invalidates this one)").clicked() {
                        action = Some(CodeAction::Regenerate);
                    }
                });
                para(ui, "Hyphens are purely visual \u{2014} you can type the code with or without them when unlocking.", t.muted);
                button_row(ui, |ui| {
                    if primary_button(ui, None, "I've saved it", Tone::Accent, true).clicked() {
                        action = Some(CodeAction::Close);
                    }
                });
            }
        },
        None,
    );
    let Some(dlg) = app.dialogs.recovery_code.as_mut() else {
        return;
    };
    if let Some(until) = dlg.copied_until
        && now >= until
    {
        dlg.copied_until = None;
    }
    // Losing a fresh code to a reflex keypress would be permanent, so
    // neither Escape nor the backdrop closes while generating or showing.
    let closable = !matches!(dlg.phase, Phase::Verifying { .. } | Phase::Shown { .. });
    if (out.backdrop || out.escape) && closable {
        action = Some(CodeAction::Close);
    }
    match action {
        Some(CodeAction::Close) => {
            tracing::debug!("vault: recovery code dialog closed");
            app.dialogs.recovery_code = None;
        }
        Some(CodeAction::Regenerate) => {
            tracing::debug!("vault: regenerate requested");
            dlg.phase = Phase::Replace;
            dlg.understand = false;
            dlg.password = Zeroizing::default();
            dlg.error = None;
            dlg.focused = false;
        }
        Some(CodeAction::Submit) => submit_recovery_code(app),
        None => {}
    }
}

fn submit_recovery_code(app: &mut App) {
    let Some(dlg) = app.dialogs.recovery_code.as_mut() else {
        return;
    };
    dlg.error = None;
    let replacing = matches!(dlg.phase, Phase::Replace);
    if dlg.password.is_empty() {
        dlg.error = Some("Enter your master password".into());
        return;
    }
    if replacing && !dlg.understand {
        dlg.error = Some("Confirm you understand the existing recovery code will stop working".into());
        return;
    }
    dlg.phase = Phase::Verifying { replacing };
    let password = String::clone(&dlg.password);
    tracing::debug!(replacing, "vault: generating a recovery code");
    app.core.call(
        "vault_generate_recovery_code",
        move |st| async move { api::vault::vault_generate_recovery_code(&st, password).await },
        |app, r| {
            let Some(dlg) = app.dialogs.recovery_code.as_mut() else {
                match r {
                    Ok(_) => tracing::warn!("vault: a recovery code was generated after the dialog closed; it was never shown"),
                    Err(e) => tracing::warn!("vault: recovery code generation failed after the dialog closed: {e}"),
                }
                return;
            };
            let replacing = matches!(dlg.phase, Phase::Verifying { replacing: true });
            match r {
                Ok(res) => {
                    tracing::info!(snapshot = ?res.snapshot_path, "vault: recovery code generated");
                    dlg.password = Zeroizing::default();
                    dlg.phase = Phase::Shown { code: Zeroizing::new(res.code), snapshot: res.snapshot_path };
                }
                Err(e) => {
                    tracing::warn!("vault: recovery code generation failed: {e}");
                    dlg.error = Some(e.to_string());
                    dlg.phase = if replacing { Phase::Replace } else { Phase::Fresh };
                }
            }
        },
    );
}

// ---------------------------------------------------------------- unlock screen

/// `recovery-unlock-dialog.tsx`.
#[derive(Default)]
pub struct RecoveryUnlock {
    code: Zeroizing<String>,
    error: Option<String>,
    busy: bool,
    focused: bool,
}

/// `reset-vault-dialog.tsx`: typed confirmation, then the snapshot path.
#[derive(Default)]
pub struct ResetVault {
    typed: String,
    error: Option<String>,
    busy: bool,
    done: Option<String>,
    focused: bool,
}

impl ResetVault {
    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.done.is_some()
    }
}

enum UnlockAction {
    Close,
    Submit,
    Finish,
}

/// The unlock screen's dialogs (called from `unlock::show`).
pub(crate) fn unlock_dialogs(app: &mut App, ctx: &egui::Context) {
    let action = match &mut app.screen {
        Screen::Unlock(state) => state.recovery.as_mut().and_then(|d| recovery_unlock_frame(ctx, d)),
        Screen::Main => return,
    };
    match action {
        Some(UnlockAction::Close) => {
            if let Screen::Unlock(state) = &mut app.screen {
                tracing::debug!("unlock: recovery dialog closed");
                state.recovery = None;
                state.refocus();
            }
        }
        Some(UnlockAction::Submit) => submit_recovery_unlock(app),
        Some(UnlockAction::Finish) | None => {}
    }

    let action = match &mut app.screen {
        Screen::Unlock(state) => state.reset.as_mut().and_then(|d| reset_frame(ctx, d)),
        Screen::Main => return,
    };
    match action {
        Some(UnlockAction::Close) => {
            if let Screen::Unlock(state) = &mut app.screen {
                tracing::debug!("unlock: reset dialog closed");
                state.reset = None;
                state.refocus();
            }
        }
        Some(UnlockAction::Submit) => submit_reset(app),
        Some(UnlockAction::Finish) => {
            // `onStatusChanged`: the vault is uninitialized now, so the
            // first-run screen. Tabs kept behind the lock are dead (the
            // reset closed their connections and detached credentials).
            tracing::info!("unlock: vault reset, showing the first-run screen");
            close_all_tabs(app);
            app.screen = Screen::Unlock(Box::new(UnlockState::new(true)));
        }
        None => {}
    }
}

fn recovery_unlock_frame(ctx: &egui::Context, dlg: &mut RecoveryUnlock) -> Option<UnlockAction> {
    let t = tokens(ctx);
    let busy = dlg.busy;
    let mut action = None;
    let out = shell(
        ctx,
        Id::unique("recovery-unlock"),
        460.0,
        Some(Icon::KeyRound),
        "Use recovery code",
        |ui| {
            para(ui, "Enter the 24-character recovery code you saved. Hyphens, spaces, and case are ignored.", t.muted);
            let r = text_field(ui, &mut dlg.code, "ABCD-EFGH-IJKL-MNOP-QRST-UVWX", "Recovery code", Some(size::BASE), true);
            if !dlg.focused {
                dlg.focused = true;
                r.request_focus();
            }
            let can_submit = !busy && !dlg.code.trim().is_empty();
            let enter = crate::form::entered(ui, &r) && can_submit;
            if let Some(e) = &dlg.error {
                widgets::error_strip(ui, e);
            }
            button_row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary_button(ui, None, if busy { "Unlocking\u{2026}" } else { "Unlock" }, Tone::Accent, can_submit).clicked() || enter {
                    action = Some(UnlockAction::Submit);
                }
                if ghost_button(ui, None, "Cancel", !busy, false).clicked() {
                    action = Some(UnlockAction::Close);
                }
            });
            para(
                ui,
                "The recovery code is single-use: a successful unlock invalidates it. Set a new master password (or generate a fresh recovery code) right after.",
                t.warning,
            );
        },
        None,
    );
    if (out.backdrop || out.escape) && !busy {
        action = Some(UnlockAction::Close);
    }
    action
}

fn submit_recovery_unlock(app: &mut App) {
    let Screen::Unlock(state) = &mut app.screen else {
        return;
    };
    let Some(dlg) = state.recovery.as_mut() else {
        return;
    };
    dlg.error = None;
    // The API zeroizes its copy.
    let code = dlg.code.trim().to_string();
    if code.is_empty() {
        dlg.error = Some("Enter your recovery code".into());
        return;
    }
    dlg.busy = true;
    tracing::debug!("unlock: trying a recovery code");
    app.core.call(
        "vault_unlock_with_recovery",
        move |st| async move { api::vault::vault_unlock_with_recovery(&st, code).await },
        |app, r| match r {
            Ok(()) => {
                tracing::info!("unlock: unlocked with the recovery code");
                app.on_unlocked();
            }
            Err(e) => {
                tracing::warn!("unlock: recovery code refused: {e}");
                if let Screen::Unlock(state) = &mut app.screen
                    && let Some(dlg) = state.recovery.as_mut()
                {
                    dlg.busy = false;
                    dlg.focused = false;
                    dlg.error = Some(match e {
                        AppError::BadPassword => "Incorrect recovery code".to_string(),
                        other => other.to_string(),
                    });
                }
            }
        },
    );
}

fn reset_frame(ctx: &egui::Context, dlg: &mut ResetVault) -> Option<UnlockAction> {
    let t = tokens(ctx);
    let id = Id::unique("reset-vault");
    let mut action = None;
    if let Some(snapshot) = &dlg.done {
        shell(
            ctx,
            id,
            480.0,
            None,
            "Vault reset",
            |ui| {
                success_body(ui, |ui| {
                    para(ui, "All vault-encrypted data has been removed. Your sessions list is still here, but each session has been detached from its credential.", t.muted);
                    para_with_code(
                        ui,
                        "A snapshot of the vault before reset was saved to ",
                        snapshot,
                        ". If you recover your master password, you can replace the active database with this file (close ezSsh first).",
                    );
                });
            },
            Some(&mut |ui: &mut Ui| {
                if primary_button(ui, None, "Set a new master password", Tone::Accent, true).clicked() {
                    action = Some(UnlockAction::Finish);
                }
            }),
        );
        // Escape is inert here (the TSX closed back to a "locked" screen
        // whose vault no longer exists).
        return action;
    }

    let busy = dlg.busy;
    let armed = dlg.typed == CONFIRM_WORD;
    let out = shell(
        ctx,
        id,
        480.0,
        None,
        "Reset vault",
        |ui| {
            form::note(
                ui,
                Icon::AlertTriangle,
                t.danger,
                "This wipes every saved credential, sync passphrase, and the vault key itself. Sessions are kept but lose their saved auth. There is no undo \u{2014} the only recoverable copy is the snapshot we will write to disk before the wipe.",
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let mut job = LayoutJob::default();
                let plain = TextFormat { font_id: font(size::XS), color: t.muted, ..TextFormat::default() };
                job.append("Type ", 0.0, plain.clone());
                job.append(CONFIRM_WORD, 0.0, TextFormat { font_id: FontId::monospace(size::XS), color: t.fg, ..TextFormat::default() });
                job.append(" to confirm", 0.0, plain);
                ui.label(job);
                let r = text_field(ui, &mut dlg.typed, CONFIRM_WORD, "Confirmation", Some(size::BASE), false);
                if !dlg.focused {
                    dlg.focused = true;
                    r.request_focus();
                }
                if crate::form::entered(ui, &r) && !busy {
                    action = Some(UnlockAction::Submit);
                }
            });
            if let Some(e) = &dlg.error {
                widgets::error_strip(ui, e);
            }
            button_row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary_button(ui, Some(Icon::Trash), if busy { "Resetting\u{2026}" } else { "Reset vault" }, Tone::Danger, !busy && armed).clicked() {
                    action = Some(UnlockAction::Submit);
                }
                if ghost_button(ui, None, "Cancel", !busy, false).clicked() {
                    action = Some(UnlockAction::Close);
                }
            });
        },
        None,
    );
    // No backdrop close here (the TSX shell has none); Escape cancels.
    if out.escape && !busy {
        action = Some(UnlockAction::Close);
    }
    action
}

fn submit_reset(app: &mut App) {
    let Screen::Unlock(state) = &mut app.screen else {
        return;
    };
    let Some(dlg) = state.reset.as_mut() else {
        return;
    };
    dlg.error = None;
    if dlg.typed != CONFIRM_WORD {
        dlg.error = Some(format!("Type {CONFIRM_WORD} to confirm"));
        return;
    }
    dlg.busy = true;
    tracing::warn!("unlock: resetting the vault");
    app.core.call(
        "vault_reset",
        |st| async move { api::vault::vault_reset(&st, RESET_TOKEN.to_string()).await },
        |app, r| {
            // The vault and everything the tabs used is gone.
            if r.is_ok() {
                close_all_tabs(app);
            }
            let dlg = match &mut app.screen {
                Screen::Unlock(state) => state.reset.as_mut(),
                Screen::Main => None,
            };
            match (r, dlg) {
                (Ok(res), Some(dlg)) => {
                    tracing::info!(snapshot = %res.snapshot_path, "unlock: vault reset");
                    dlg.busy = false;
                    dlg.done = Some(res.snapshot_path);
                }
                (Ok(res), None) => {
                    tracing::warn!(snapshot = %res.snapshot_path, "unlock: vault reset after the dialog closed");
                    app.screen = Screen::Unlock(Box::new(UnlockState::new(true)));
                }
                (Err(e), dlg) => {
                    tracing::warn!("unlock: vault reset failed: {e}");
                    if let Some(dlg) = dlg {
                        dlg.busy = false;
                        dlg.error = Some(e.to_string());
                    }
                }
            }
        },
    );
}
