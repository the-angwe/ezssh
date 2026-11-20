//! The private-key picker: pick a shared private key, add a new one inline
//! (the key and its optional passphrase are one credential), open the
//! manager. Session passwords are not in the shared list (they belong to
//! their session).
//!
//! The list is `app.creds`; a selection whose credential vanished (deleted
//! in the manager, or a session row naming a deleted one) is cleared on
//! the next reload.

use std::sync::{Arc, Mutex};

use egui::{Color32, CornerRadius, Frame, Id, Margin, Ui};
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

/// The only kind the picker lists and creates.
const KIND: &str = "private_key";

pub struct CredPicker {
    /// Salt for the widget ids (one picker per owner).
    salt: Id,
    adding: bool,
    label: String,
    /// The private key being added.
    secret: Zeroizing<String>,
    /// Its passphrase (optional).
    passphrase: Zeroizing<String>,
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
    /// `salt` keeps the ids of several pickers apart.
    pub fn new(salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            salt: Id::unique(("cred-picker", KIND)).with(salt),
            adding: false,
            label: String::new(),
            secret: Zeroizing::new(String::new()),
            passphrase: Zeroizing::new(String::new()),
            show: false,
            err: None,
            busy: false,
            created: Arc::default(),
            seen_generation: u64::MAX,
            pending: None,
            focus_label: false,
        }
    }

    fn reset_form(&mut self) {
        self.adding = false;
        self.label.clear();
        // Zeroizing only wipes on drop; overwrite the typed secrets now.
        self.secret = Zeroizing::new(String::new());
        self.passphrase = Zeroizing::new(String::new());
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
        let listed = app.creds.of_kind(KIND).any(|c| c.id == id);
        if listed {
            if self.pending == Some(id) {
                self.pending = None;
            }
        } else if self.pending != Some(id) {
            tracing::debug!(
                kind = KIND,
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
                tracing::warn!(kind = KIND, "cred_picker: result slot poisoned: {e}");
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
                    kind = KIND,
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
            self.err = Some("Label and private key are required".into());
            tracing::debug!(kind = KIND, "cred_picker: label or key missing");
            return;
        }
        self.err = None;
        self.busy = true;
        // credential_create zeroizes its copies on every path.
        let secret = String::clone(&self.secret);
        let passphrase = (!self.passphrase.is_empty()).then(|| String::clone(&self.passphrase));
        let slot = self.created.clone();
        tracing::debug!(
            kind = KIND,
            has_passphrase = passphrase.is_some(),
            "cred_picker: creating"
        );
        app.core.call(
            "credential_create",
            move |st| async move {
                api::credentials::credential_create(&st, KIND.into(), label, secret, passphrase)
                    .await
            },
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
                    vec![(None, "— choose saved private key —".to_string())];
                options.extend(
                    app.creds
                        .of_kind(KIND)
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
                    tracing::debug!(kind = KIND, credential = ?value, "cred_picker: picked");
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
                    form::caption(ui, "New private key");
                    let l = form::input(
                        ui,
                        &mut self.label,
                        form::InputOpts::new("Label (e.g. 'Prod server key')"),
                    );
                    crate::form::labelled(ui, &l, "New credential label");
                    if std::mem::take(&mut self.focus_label) {
                        l.request_focus();
                    }
                    let s = form::textarea(
                        ui,
                        &mut self.secret,
                        "-----BEGIN OPENSSH PRIVATE KEY-----",
                        6,
                        true,
                    );
                    crate::form::labelled(ui, &s, "New credential secret");
                    let p = form::secret(
                        ui,
                        &mut self.passphrase,
                        "Passphrase (leave blank if the key has none)",
                        &mut self.show,
                        None,
                    );
                    crate::form::labelled(ui, &p, "New credential passphrase");
                    // Enter in the label or the passphrase submits, as in
                    // the credentials manager's form.
                    let enter = form::entered(ui, &l) || form::entered(ui, &p);
                    if let Some(e) = &self.err {
                        ui.label(egui::RichText::new(e).font(font(size::SM)).color(t.danger));
                    }
                    widgets::button_row(ui, |ui| {
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
    fn salts_keep_pickers_apart() {
        assert_ne!(CredPicker::new(1).salt, CredPicker::new(2).salt);
    }

    #[test]
    fn reset_wipes_the_typed_secrets() {
        let mut k = CredPicker::new(2);
        k.adding = true;
        k.label.push_str("prod");
        k.secret.push_str("-----BEGIN");
        k.passphrase.push_str("hunter2");
        k.show = true;
        k.reset_form();
        assert!(
            !k.adding
                && k.label.is_empty()
                && k.secret.is_empty()
                && k.passphrase.is_empty()
                && !k.show
                && k.err.is_none()
        );
    }
}
