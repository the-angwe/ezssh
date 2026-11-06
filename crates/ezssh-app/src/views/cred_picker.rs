//! The credential picker: pick a shared private key or key passphrase,
//! add a new one inline, open the manager. Session passwords are not in
//! the shared list (they belong to their session), so the kinds here are
//! `private_key` and `key_passphrase` only.
//!
//! The list is `app.creds`; a selection whose credential vanished (deleted
//! in the manager, or a session row naming a deleted one) is cleared on
//! the next reload.

use std::sync::{Arc, Mutex};

use egui::{Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Ui};
use ezssh_core::api;
use ezssh_core::db::credentials::CredentialMeta;
use zeroize::Zeroizing;

use crate::app::App;
use crate::form;
use crate::icons::Icon;
use crate::style::{font, radius, size, tokens};
use crate::widgets;

/// Result slot of an in-flight `credential_create`, filled by the core
/// continuation and taken by the picker on its next frame.
type Created = Arc<Mutex<Option<Result<CredentialMeta, String>>>>;

pub struct CredPicker {
    kind: &'static str,
    /// Salt for the widget ids (one picker per slot and owner).
    salt: Id,
    adding: bool,
    label: String,
    secret: Zeroizing<String>,
    show: bool,
    err: Option<String>,
    busy: bool,
    created: Created,
    /// `app.creds.generation` last checked for a dangling selection.
    seen_generation: u64,
    /// Just created and selected: not clamped until a reload lists it.
    pending: Option<i64>,
    focus_label: bool,
}

impl CredPicker {
    /// `kind` is `private_key` or `key_passphrase`; `salt` keeps the ids of
    /// several pickers apart.
    pub fn new(kind: &'static str, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        debug_assert!(
            matches!(kind, "private_key" | "key_passphrase"),
            "picker kind {kind}"
        );
        Self {
            kind,
            salt: Id::unique(("cred-picker", kind)).with(salt),
            adding: false,
            label: String::new(),
            secret: Zeroizing::new(String::new()),
            show: false,
            err: None,
            busy: false,
            created: Arc::default(),
            seen_generation: u64::MAX,
            pending: None,
            focus_label: false,
        }
    }

    fn verb(&self) -> &'static str {
        if self.kind == "private_key" {
            "private key"
        } else {
            "passphrase"
        }
    }

    fn secret_hint(&self) -> &'static str {
        if self.kind == "private_key" {
            "Private key (PEM or OpenSSH)"
        } else {
            "Passphrase"
        }
    }

    fn reset_form(&mut self) {
        self.adding = false;
        self.label.clear();
        // Zeroizing only wipes on drop; overwrite the typed secret now.
        self.secret = Zeroizing::new(String::new());
        self.show = false;
        self.err = None;
    }

    /// Clear `value` when it names a credential the list no longer has.
    fn clamp(&mut self, app: &App, value: &mut Option<i64>) {
        if app.creds.generation == self.seen_generation
            || app.creds.loading
            || app.creds.generation == 0
        {
            return;
        }
        self.seen_generation = app.creds.generation;
        let Some(id) = *value else {
            return;
        };
        let listed = app.creds.of_kind(self.kind).any(|c| c.id == id);
        if listed {
            if self.pending == Some(id) {
                self.pending = None;
            }
        } else if self.pending != Some(id) {
            tracing::debug!(
                kind = self.kind,
                credential = id,
                "cred_picker: selection no longer listed, cleared"
            );
            *value = None;
        }
    }

    /// Take a finished `credential_create`.
    fn take_created(&mut self, value: &mut Option<i64>) {
        let done = match self.created.lock() {
            Ok(mut slot) => slot.take(),
            Err(e) => {
                tracing::warn!(kind = self.kind, "cred_picker: result slot poisoned: {e}");
                None
            }
        };
        let Some(done) = done else {
            return;
        };
        self.busy = false;
        match done {
            Ok(meta) => {
                tracing::debug!(
                    kind = self.kind,
                    credential = meta.id,
                    "cred_picker: created and selected"
                );
                self.reset_form();
                self.pending = Some(meta.id);
                *value = Some(meta.id);
            }
            Err(e) => self.err = Some(e),
        }
    }

    fn create(&mut self, app: &mut App) {
        let label = self.label.trim().to_string();
        if label.is_empty() || self.secret.is_empty() {
            self.err = Some("Label and secret are required".into());
            tracing::debug!(kind = self.kind, "cred_picker: label or secret missing");
            return;
        }
        self.err = None;
        self.busy = true;
        let kind = self.kind.to_string();
        // credential_create zeroizes its copy on every path.
        let secret = String::clone(&self.secret);
        let slot = self.created.clone();
        tracing::debug!(kind = %kind, "cred_picker: creating");
        app.core.call(
            "credential_create",
            move |st| async move { api::credentials::credential_create(&st, kind, label, secret).await },
            move |app, r| {
                let r = match r {
                    Ok(meta) => {
                        // List it now; the reload confirms it from the DB.
                        app.creds.list.push(meta.clone());
                        app.creds.reload(&app.core);
                        Ok(meta)
                    }
                    Err(e) => {
                        tracing::warn!("cred_picker: credential_create failed: {e}");
                        Err(e.to_string())
                    }
                };
                match slot.lock() {
                    Ok(mut s) => *s = Some(r),
                    Err(e) => tracing::warn!("cred_picker: result slot poisoned: {e}"),
                }
            },
        );
    }

    /// The picker for one frame; `value` is the selected credential id.
    pub fn show(&mut self, app: &mut App, ui: &mut Ui, value: &mut Option<i64>) {
        self.take_created(value);
        self.clamp(app, value);
        let t = tokens(ui.ctx());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let mut options: Vec<(Option<i64>, String)> =
                    vec![(None, format!("— choose saved {} —", self.verb()))];
                options.extend(
                    app.creds
                        .of_kind(self.kind)
                        .map(|c| (Some(c.id), c.label.clone())),
                );
                let manage_w = 28.0;
                let add_w = if self.adding {
                    0.0
                } else {
                    add_button_width(ui) + 8.0
                };
                let select_w = (ui.available_width() - manage_w - add_w - 8.0).max(120.0);
                if form::select(
                    ui,
                    self.salt.with("select"),
                    value,
                    &options,
                    Some(select_w),
                ) {
                    tracing::debug!(kind = self.kind, credential = ?value, "cred_picker: picked");
                }
                if !self.adding
                    && widgets::Button::ghost("Add new")
                        .icon(Icon::Plus)
                        .small()
                        .show(ui)
                        .clicked()
                {
                    self.adding = true;
                    self.focus_label = true;
                }
                if widgets::Button::icon_only(
                    Icon::Settings2,
                    "Manage credentials",
                    widgets::Variant::Ghost,
                )
                .small()
                .show(ui)
                .on_hover_text("Manage credentials")
                .clicked()
                {
                    tracing::debug!("cred_picker: open the manager");
                    app.dialogs.credentials = Some(Default::default());
                }
            });
            if !self.adding {
                return;
            }
            Frame::new()
                .fill(t.surface2)
                .corner_radius(CornerRadius::same(radius::LG as u8))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    form::caption(ui, &format!("New {}", self.verb()));
                    let l = form::input(
                        ui,
                        &mut self.label,
                        form::InputOpts::new("Label (e.g. 'Prod server key')"),
                    );
                    crate::form::labelled(ui, &l, "New credential label");
                    if std::mem::take(&mut self.focus_label) {
                        l.request_focus();
                    }
                    let s = if self.kind == "private_key" {
                        form::textarea(
                            ui,
                            &mut self.secret,
                            "-----BEGIN OPENSSH PRIVATE KEY-----",
                            6,
                            true,
                        )
                    } else {
                        let hint = self.secret_hint();
                        form::secret(ui, &mut self.secret, hint, &mut self.show, None)
                    };
                    crate::form::labelled(ui, &s, "New credential secret");
                    // Enter in the label or (single-line) secret submits, as in
                    // the credentials manager's form.
                    let enter = form::entered(ui, &l)
                        || (self.kind != "private_key" && form::entered(ui, &s));
                    if let Some(e) = &self.err {
                        ui.label(egui::RichText::new(e).font(font(size::SM)).color(t.danger));
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        widgets::trailing(ui, |ui| {
                            if widgets::Button::secondary("Cancel")
                                .small()
                                .show(ui)
                                .clicked()
                            {
                                self.reset_form();
                            }
                            if widgets::Button::primary("Save credential")
                                .small()
                                .enabled(!self.busy)
                                .show(ui)
                                .clicked()
                                || (enter && !self.busy)
                            {
                                self.create(app);
                            }
                        });
                    });
                });
        });
    }
}

fn add_button_width(ui: &Ui) -> f32 {
    let g = ui
        .painter()
        .layout_no_wrap("Add new".into(), font(size::SM), Color32::WHITE);
    g.size().x + 14.0 + 8.0 + 20.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_and_hints_follow_the_kind() {
        let k = CredPicker::new("private_key", 1);
        assert_eq!(
            (k.verb(), k.secret_hint()),
            ("private key", "Private key (PEM or OpenSSH)")
        );
        let p = CredPicker::new("key_passphrase", 1);
        assert_eq!((p.verb(), p.secret_hint()), ("passphrase", "Passphrase"));
        assert_ne!(k.salt, p.salt);
    }

    #[test]
    fn reset_wipes_the_typed_secret() {
        let mut k = CredPicker::new("key_passphrase", 2);
        k.adding = true;
        k.label.push_str("prod");
        k.secret.push_str("hunter2");
        k.show = true;
        k.reset_form();
        assert!(
            !k.adding && k.label.is_empty() && k.secret.is_empty() && !k.show && k.err.is_none()
        );
    }
}
