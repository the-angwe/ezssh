//! The credentials manager: the vault's shared credentials (SSH keys and
//! key passphrases) with rename, rotate secret, add, and delete with a
//! warning that lists the sessions using it. Sessions reference these by
//! id, so every change applies to all of them at once.
//!
//! Session passwords are saved with their session (the core lists only
//! shared rows), so there is no password group. An unowned password left
//! by the ezTerm app's password picker (in a database it wrote) shows in a
//! small delete-only group.

use egui::{Align, CornerRadius, Frame, Id, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use ezssh_core::api;
use ezssh_core::db::credentials::CredentialDetail;
use zeroize::Zeroizing;

use super::dialogs::{self, Confirm};
use crate::app::App;
use crate::form::{self, InputOpts};
use crate::icons::Icon;
use crate::style::{alpha, font, font_semibold, radius, size, tokens};
use crate::widgets::{self, Button, DialogFooter, DialogPart, IconBtn};

struct Group {
    kind: &'static str,
    title: &'static str,
    icon: Icon,
    secret_label: &'static str,
    /// Add / rename / rotate offered (the unattached passwords: delete only).
    editable: bool,
}

const GROUPS: [Group; 2] = [
    Group { kind: "private_key", title: "SSH keys", icon: Icon::KeySquare, secret_label: "Private key (PEM or OpenSSH)", editable: true },
    Group { kind: "key_passphrase", title: "Key passphrases", icon: Icon::KeyRound, secret_label: "Passphrase", editable: true },
];

const UNATTACHED: Group =
    Group { kind: "password", title: "Unattached passwords (from the ezTerm app)", icon: Icon::Lock, secret_label: "Password", editable: false };

enum RowEdit {
    Rename { id: i64, label: String },
    Replace { id: i64, secret: Zeroizing<String>, shown: bool },
}

#[derive(Default)]
pub struct CredentialsDialog {
    items: Vec<CredentialDetail>,
    loaded: bool,
    requested: bool,
    edit: Option<RowEdit>,
    add_kind: Option<&'static str>,
    add_label: String,
    add_secret: Zeroizing<String>,
    add_shown: bool,
    confirm_delete: Option<CredentialDetail>,
    /// A mutation is in flight; further ones are dropped.
    busy: bool,
    /// Focus the input that just opened (rename / add label).
    focus: bool,
}

#[cfg(test)]
impl CredentialsDialog {
    /// (kind, label) of every listed credential, once loaded.
    pub(crate) fn listed(&self) -> Option<Vec<(String, String)>> {
        self.loaded.then(|| self.items.iter().map(|c| (c.kind.clone(), c.label.clone())).collect())
    }

    pub(crate) fn editing(&self) -> bool {
        self.edit.is_some() || self.add_kind.is_some()
    }

    pub(crate) fn confirming(&self) -> bool {
        self.confirm_delete.is_some()
    }
}

/// A mutation asked for this frame.
enum Op {
    Rename { id: i64, label: String },
    Replace { id: i64, label: String, secret: String },
    Add { kind: &'static str, label: String, secret: String },
    Delete { id: i64, label: String },
}

impl Op {
    fn name(&self) -> &'static str {
        match self {
            Op::Rename { .. } => "credential_update(label)",
            Op::Replace { .. } => "credential_update(secret)",
            Op::Add { .. } => "credential_create",
            Op::Delete { .. } => "credential_delete",
        }
    }
}

fn reload(app: &mut App) {
    tracing::debug!("credentials: loading");
    app.core.call(
        "credential_list_detailed",
        |st| async move { api::credentials::credential_list_detailed(&st).await },
        |app, r| match r {
            Ok(items) => {
                tracing::debug!(n = items.len(), "credentials: loaded");
                if let Some(d) = app.dialogs.credentials.as_mut() {
                    d.items = items;
                    d.loaded = true;
                }
            }
            Err(e) => {
                tracing::warn!("credentials: credential_list_detailed failed: {e}");
                app.toasts.danger("Failed to load credentials", e.to_string());
            }
        },
    );
}

/// Run a mutation, then reload this list and the shared [`crate::creds`]
/// cache. A failure keeps the inline form (and what was typed) open.
fn run(app: &mut App, op: Op) {
    let Some(d) = app.dialogs.credentials.as_mut() else {
        return;
    };
    let what = op.name();
    if d.busy {
        tracing::debug!(what, "credentials: busy, dropped");
        return;
    }
    d.busy = true;
    // Never log labels' secrets: only ids and kinds.
    let (toast_title, toast_body, done): (&'static str, Option<String>, fn(&mut CredentialsDialog)) = match &op {
        Op::Rename { id, .. } => {
            tracing::debug!(id, "credentials: rename");
            ("Credential renamed", None, |d| d.edit = None)
        }
        Op::Replace { id, label, .. } => {
            tracing::debug!(id, "credentials: replace secret");
            ("Secret updated", Some(label.clone()), |d| d.edit = None)
        }
        Op::Add { kind, label, .. } => {
            tracing::debug!(kind, "credentials: add");
            ("Credential added", Some(label.clone()), |d| {
                d.add_kind = None;
                d.add_label.clear();
                d.add_secret = Zeroizing::default();
                d.add_shown = false;
            })
        }
        Op::Delete { id, label } => {
            tracing::debug!(id, "credentials: delete");
            ("Credential deleted", Some(label.clone()), |d| d.confirm_delete = None)
        }
    };
    app.core.call(
        what,
        move |st| async move {
            match op {
                Op::Rename { id, label } => api::credentials::credential_update(&st, id, Some(label), None).await,
                // label None: rotating never writes the label, so a stale
                // list cannot revert a rename done elsewhere meanwhile.
                Op::Replace { id, secret, .. } => api::credentials::credential_update(&st, id, None, Some(secret)).await,
                Op::Add { kind, label, secret } => api::credentials::credential_create(&st, kind.into(), label, secret).await.map(|_| ()),
                Op::Delete { id, .. } => api::credentials::credential_delete(&st, id).await,
            }
        },
        move |app, r| {
            if let Some(d) = app.dialogs.credentials.as_mut() {
                d.busy = false;
            }
            match r {
                Ok(()) => {
                    app.toasts.success(toast_title, toast_body);
                    if let Some(d) = app.dialogs.credentials.as_mut() {
                        done(d);
                    }
                    reload(app);
                    app.creds.reload(&app.core);
                }
                Err(e) => {
                    tracing::warn!(what, "credentials: action failed: {e}");
                    app.toasts.danger("Action failed", e.to_string());
                }
            }
        },
    );
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.dialogs.credentials.take() else {
        return;
    };
    let first = !d.requested;
    d.requested = true;
    let t = tokens(ctx);
    let mut ops: Vec<Op> = Vec::new();
    let mut close = false;
    let out = widgets::Dialog::new(Id::unique("credentials-dialog"), 620.0)
        .header(Icon::KeySquare, t.accent, "Credentials", Some("Stored encrypted in the vault · changes apply to every session using them"))
        .plain_subtitle()
        .footer(DialogFooter::Strip)
        .row_gap(20.0)
        .show(ctx, |ui, part| match part {
            DialogPart::Body => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    Icon::Lock.show(ui, 14.0, t.muted);
                    form::hint(ui, "Session passwords are saved with each session: edit the session to change one.");
                });
                for g in &GROUPS {
                    group(ui, &mut d, g, &mut ops);
                }
                if d.items.iter().any(|c| c.kind == UNATTACHED.kind) {
                    group(ui, &mut d, &UNATTACHED, &mut ops);
                }
            }
            DialogPart::Footer => {
                if widgets::Button::secondary("Close").show(ui).clicked() {
                    close = true;
                }
            }
        });
    // Backdrop and ✕ close; Escape unwinds one layer: inline edit, then
    // dialog (the confirm is its own top modal and takes Escape itself).
    if out.backdrop || out.close {
        close = true;
    } else if out.escape {
        if d.edit.is_some() || d.add_kind.is_some() {
            d.edit = None;
            d.add_kind = None;
        } else {
            close = true;
        }
    }

    if let Some(c) = &d.confirm_delete {
        let n = c.used_by.len();
        let body = if n > 0 {
            let names: Vec<String> = c.used_by.iter().map(|s| format!("· {}", s.name)).collect();
            format!(
                "Used by {n} session{}:\n{}\n\nThese sessions will fail to connect until you assign them a new credential.",
                plural(n),
                names.join("\n")
            )
        } else {
            "No sessions use this credential.".to_string()
        };
        let title = format!("Delete \"{}\"?", c.label);
        match dialogs::confirm(ctx, Id::unique("credentials-delete"), Confirm { title: &title, body: &body, confirm_text: "Delete", danger: true }) {
            // The confirm stays until the delete succeeds, so a failure
            // does not look like a success.
            Some(true) => ops.push(Op::Delete { id: c.id, label: c.label.clone() }),
            Some(false) => d.confirm_delete = None,
            None => {}
        }
    }

    if close {
        tracing::debug!("credentials: closed");
    } else {
        app.dialogs.credentials = Some(d);
    }
    if first {
        reload(app);
    }
    for op in ops {
        run(app, op);
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// One group: header with Add, then a bordered list of rows and the add form.
fn group(ui: &mut Ui, d: &mut CredentialsDialog, g: &Group, ops: &mut Vec<Op>) {
    let t = tokens(ui.ctx());
    let rows: Vec<CredentialDetail> = d.items.iter().filter(|c| c.kind == g.kind).cloned().collect();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.set_min_height(28.0);
            g.icon.show(ui, 16.0, t.muted);
            ui.label(RichText::new(g.title).font(font_semibold(size::MD)).color(t.fg));
            if g.editable {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if Button::secondary("Add").small().icon(Icon::Plus).label(&format!("Add {}", g.title)).show(ui).clicked() {
                        d.edit = None;
                        d.add_kind = Some(g.kind);
                        d.add_label.clear();
                        d.add_secret = Zeroizing::default();
                        d.add_shown = false;
                        d.focus = true;
                    }
                });
            }
        });
        Frame::new().stroke(Stroke::new(1.0, t.border_strong)).corner_radius(CornerRadius::same(radius::LG as u8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let adding = d.add_kind == Some(g.kind);
            if rows.is_empty() && !adding {
                Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = if d.loaded { format!("No {} saved.", g.title.to_lowercase()) } else { "Loading…".into() };
                    ui.label(RichText::new(text).font(font(size::SM)).color(t.muted));
                });
            }
            for (i, c) in rows.iter().enumerate() {
                if i > 0 {
                    divider(ui);
                }
                row(ui, d, g, c, ops);
            }
            if adding {
                if !rows.is_empty() {
                    divider(ui);
                }
                add_form(ui, d, g, ops);
            }
        });
    });
}

fn divider(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, tokens(ui.ctx()).border));
}

fn row(ui: &mut Ui, d: &mut CredentialsDialog, g: &Group, c: &CredentialDetail, ops: &mut Vec<Op>) {
    let t = tokens(ui.ctx());
    Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        let busy = d.busy;
        if matches!(&d.edit, Some(RowEdit::Rename { id, .. }) if *id == c.id) {
            let focus = std::mem::take(&mut d.focus);
            let (mut save, mut cancel, mut renamed) = (false, false, None);
            if let Some(RowEdit::Rename { label, .. }) = &mut d.edit {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let w = ui.available_width() - 120.0;
                    let resp = form::input(ui, label, InputOpts::new("").width(w)).accessible_name(format!("Rename {}", c.label));
                    if focus {
                        resp.request_focus();
                    }
                    save |= crate::form::entered(ui, &resp);
                    save |= widgets::Button::primary("Save").small().enabled(!busy).show(ui).clicked();
                    cancel = widgets::Button::secondary("Cancel").small().show(ui).clicked();
                });
                if save && !label.trim().is_empty() {
                    renamed = Some(label.trim().to_string());
                }
            }
            if cancel {
                d.edit = None;
            } else if let Some(label) = renamed {
                ops.push(Op::Rename { id: c.id, label });
            }
            return;
        }

        // Label, date and usage; the buttons show on hover or focus.
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        let hovered = ui.rect_contains_pointer(rect);
        let btns: Vec<(Icon, String, String)> = if g.editable {
            vec![
                (Icon::Pencil, format!("Rename {}", c.label), "Rename".to_string()),
                (Icon::RefreshCw, format!("Replace secret for {}", c.label), format!("Replace {}", g.secret_label.to_lowercase())),
                (Icon::Trash, format!("Delete {}", c.label), "Delete".to_string()),
            ]
        } else {
            vec![(Icon::Trash, format!("Delete {}", c.label), "Delete".to_string())]
        };
        let btns_left = rect.right() - 28.0 * btns.len() as f32;
        let text_w = btns_left - rect.left() - 8.0;
        widgets::paint_text(ui, Rect::from_min_size(rect.min, vec2(text_w, 18.0)), &c.label, font_semibold(size::BASE), t.fg);
        let added = format!("Added {}", c.created_at.get(..10).unwrap_or(&c.created_at));
        let sub = Rect::from_min_size(Pos2::new(rect.left(), rect.top() + 18.0), vec2(text_w, 18.0));
        let added_r = widgets::paint_text(ui, sub, &added, font(size::SM), t.muted);
        let n = c.used_by.len();
        let usage = if n > 0 { format!("· Used by {n} session{}", plural(n)) } else { "· Not used by any session".to_string() };
        let usage_color = if n > 0 { t.muted } else { alpha(t.muted, 0.7) };
        let usage_r = widgets::paint_text(ui, sub.with_min_x(added_r.right() + 8.0), &usage, font(size::SM), usage_color);
        if n > 0 {
            let names: Vec<&str> = c.used_by.iter().map(|s| s.name.as_str()).collect();
            ui.interact(usage_r, Id::unique(("credential-usage", c.id)), Sense::hover()).on_hover_text(names.join("\n"));
        }
        for (i, (icon, label, tip)) in btns.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(btns_left + 28.0 * i as f32, rect.center().y - 14.0), vec2(28.0, 28.0));
            let mut b = IconBtn::new(*icon, label.clone(), tip).icon_size(16.0).visible(hovered);
            if *icon == Icon::Trash {
                b = b.hover(t.danger, alpha(t.danger, 0.1));
            }
            if b.show(ui, r, Id::unique(("credential-btn", c.id, i))).clicked() {
                match icon {
                    Icon::Pencil => {
                        d.add_kind = None;
                        d.edit = Some(RowEdit::Rename { id: c.id, label: c.label.clone() });
                        d.focus = true;
                    }
                    Icon::RefreshCw => {
                        d.add_kind = None;
                        d.edit = Some(RowEdit::Replace { id: c.id, secret: Zeroizing::default(), shown: false });
                        d.focus = true;
                    }
                    _ => d.confirm_delete = Some(c.clone()),
                }
            }
        }

        if matches!(&d.edit, Some(RowEdit::Replace { id, .. }) if *id == c.id) {
            let focus = std::mem::take(&mut d.focus);
            let (mut save, mut cancel, mut secret_out) = (false, false, None);
            if let Some(RowEdit::Replace { secret, shown, .. }) = &mut d.edit {
                let hint = format!("New {}", g.secret_label.to_lowercase());
                let (enter, field) = secret_input(ui, g.kind, &hint, secret, shown, &format!("New secret for {}", c.label));
                if focus {
                    field.request_focus();
                }
                save |= enter;
                widgets::button_row(ui, |ui| {
                    save |= widgets::Button::primary("Replace secret").small().enabled(!busy && !secret.is_empty()).show(ui).clicked();
                    cancel = widgets::Button::secondary("Cancel").small().show(ui).clicked();
                });
                if save && !secret.is_empty() {
                    secret_out = Some((**secret).clone());
                }
            }
            if cancel {
                d.edit = None;
            } else if let Some(secret) = secret_out {
                ops.push(Op::Replace { id: c.id, label: c.label.clone(), secret });
            }
        }
    });
}

/// `SecretInput`: a textarea for private keys (Enter is a newline), else a
/// password field with an eye toggle where Enter saves. Returns (Enter
/// pressed, the field).
fn secret_input(ui: &mut Ui, kind: &str, hint: &str, value: &mut String, shown: &mut bool, label: &str) -> (bool, egui::Response) {
    if kind == "private_key" {
        let resp = form::textarea(ui, value, "-----BEGIN OPENSSH PRIVATE KEY-----", 5, true).accessible_name(label);
        (false, resp)
    } else {
        let resp = form::secret(ui, value, hint, shown, None).accessible_name(label);
        (crate::form::entered(ui, &resp), resp)
    }
}

fn add_form(ui: &mut Ui, d: &mut CredentialsDialog, g: &Group, ops: &mut Vec<Op>) {
    let t = tokens(ui.ctx());
    let busy = d.busy;
    Frame::new().fill(t.surface2).corner_radius(CornerRadius { nw: 0, ne: 0, sw: radius::LG as u8, se: radius::LG as u8 }).inner_margin(Margin::symmetric(12, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        let label_resp =
            form::input(ui, &mut d.add_label, InputOpts::new("Label (e.g. 'Prod server key')")).accessible_name(format!("New {} label", g.title));
        if std::mem::take(&mut d.focus) {
            label_resp.request_focus();
        }
        let (secret_enter, _) =
            secret_input(ui, g.kind, g.secret_label, &mut d.add_secret, &mut d.add_shown, &format!("New {} secret", g.title));
        let ready = !d.add_label.trim().is_empty() && !d.add_secret.is_empty();
        let mut save = crate::form::entered(ui, &label_resp) || secret_enter;
        let mut cancel = false;
        widgets::button_row(ui, |ui| {
            save |= widgets::Button::primary("Save credential").small().enabled(!busy && ready).show(ui).clicked();
            cancel = widgets::Button::secondary("Cancel").small().show(ui).clicked();
        });
        if cancel {
            d.add_kind = None;
        } else if save && ready {
            ops.push(Op::Add { kind: g.kind, label: d.add_label.trim().to_string(), secret: (*d.add_secret).clone() });
        }
    });
}
