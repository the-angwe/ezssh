//! The auth-fix overlay: shown over a tab whose SSH login failed. Fix the
//! username, the method and the password or key without opening the full
//! session dialog, then save and reconnect. Painted over the tab's terminal
//! only, not window-wide.
//!
//! As in the session dialog, a password is typed (it belongs to the
//! session); a blank field keeps the stored one.

use egui::{Frame, Id, Key, Margin, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use ezssh_core::db::sessions::{Session, SessionInput};
use ezssh_core::{Secret, api};
use zeroize::Zeroizing;

use super::cred_picker::CredPicker;
use crate::app::App;
use crate::form::labelled;
use crate::form::{self, InputOpts};
use crate::icons::Icon;
use crate::style::{alpha, tokens};
use crate::tabs::TabId;
use crate::widgets::{self};

pub struct AuthFix {
    pub session: Session,
    pub message: String,
    username: String,
    auth_type: String,
    credential_id: Option<i64>,
    passphrase_id: Option<i64>,
    /// The typed password; wiped on drop.
    password: Zeroizing<String>,
    show_password: bool,
    busy: bool,
    err: Option<String>,
    key_picker: CredPicker,
    pass_picker: CredPicker,
    focus_username: bool,
}

impl AuthFix {
    /// `tab` salts the pickers' ids: two tabs of one session can both fail.
    pub fn new(tab: TabId, session: Session, message: String) -> Self {
        tracing::debug!(session = session.id, "auth_fix: shown");
        let password_auth = session.auth_type == "password";
        Self {
            username: session.username.clone(),
            auth_type: session.auth_type.clone(),
            credential_id: if password_auth {
                None
            } else {
                session.credential_id
            },
            passphrase_id: session.key_passphrase_credential_id,
            password: Zeroizing::new(String::new()),
            show_password: false,
            busy: false,
            err: None,
            key_picker: CredPicker::new("private_key", ("auth-fix", tab)),
            pass_picker: CredPicker::new("key_passphrase", ("auth-fix", tab)),
            focus_username: true,
            session,
            message,
        }
    }

    fn password_stored(&self) -> bool {
        self.session.auth_type == "password" && self.session.credential_id.is_some()
    }

    /// A method change drops the previous method's attachments.
    fn switch_method(&mut self, next: &str) {
        tracing::debug!(from = %self.auth_type, to = next, "auth_fix: method");
        self.auth_type = next.to_string();
        self.credential_id = None;
        self.passphrase_id = None;
    }

    fn validate(&self) -> Option<&'static str> {
        if self.username.trim().is_empty() {
            return Some("Username is required");
        }
        match self.auth_type.as_str() {
            "password" if self.password.is_empty() && !self.password_stored() => {
                Some("Password is required")
            }
            "key" if self.credential_id.is_none() => Some("Pick or add a credential"),
            _ => None,
        }
    }

    /// The overlay's fields applied to the tab's copy of the row (the
    /// save call applies them to the row as stored instead).
    #[cfg(test)]
    fn to_input(&self) -> SessionInput {
        AuthFields {
            username: self.username.trim().to_string(),
            auth_type: self.auth_type.clone(),
            credential_id: self.credential_id,
            passphrase_id: self.passphrase_id,
            password: (self.auth_type == "password" && !self.password.is_empty())
                .then(|| Secret::new(String::clone(&self.password))),
        }
        .into_input(&self.session)
    }

    fn header_line(&self) -> String {
        let user = if self.username.is_empty() {
            "user"
        } else {
            self.username.as_str()
        };
        let port = if self.session.port != 22 {
            format!(":{}", self.session.port)
        } else {
            String::new()
        };
        format!("{user}@{}{port}", self.session.host)
    }
}

/// The overlay's answer, applied to the stored row by the save call.
struct AuthFields {
    username: String,
    auth_type: String,
    credential_id: Option<i64>,
    passphrase_id: Option<i64>,
    password: Option<Secret>,
}

impl AuthFields {
    /// `base` (the row as stored right now, fetched by the save call) with
    /// this overlay's auth fields. `env` is filled in by the save call.
    fn into_input(self, base: &Session) -> SessionInput {
        let s = base;
        let key = self.auth_type == "key";
        SessionInput {
            folder_id: s.folder_id,
            name: s.name.clone(),
            host: s.host.clone(),
            port: s.port,
            username: self.username,
            auth_type: self.auth_type,
            credential_id: if key { self.credential_id } else { None },
            key_passphrase_credential_id: if key { self.passphrase_id } else { None },
            color: s.color.clone(),
            initial_command: s.initial_command.clone(),
            scrollback_lines: s.scrollback_lines,
            font_size: s.font_size,
            font_family: s.font_family.clone(),
            cursor_style: s.cursor_style.clone(),
            compression: s.compression,
            keepalive_secs: s.keepalive_secs,
            connect_timeout_secs: s.connect_timeout_secs,
            env: Vec::new(),
            session_kind: s.session_kind.clone(),
            forward_x11: s.forward_x11,
            starting_dir: s.starting_dir.clone(),
            password: self.password,
        }
    }
}

/// Save: the env vars first (they live in their own table and
/// `session_update` replaces them), then the session; on success update
/// the tab's copy and reconnect.
fn save(app: &mut App, fix: &mut AuthFix, tab: TabId) {
    if let Some(problem) = fix.validate() {
        tracing::debug!(tab, problem, "auth_fix: validation failed");
        fix.err = Some(problem.into());
        return;
    }
    fix.err = None;
    fix.busy = true;
    let id = fix.session.id;
    // The tab's copy may predate a session-dialog save (host, name,
    // folder…): overlay the auth fields onto the row as stored now.
    let overlay = AuthFields {
        username: fix.username.trim().to_string(),
        auth_type: fix.auth_type.clone(),
        credential_id: fix.credential_id,
        passphrase_id: fix.passphrase_id,
        password: (fix.auth_type == "password" && !fix.password.is_empty())
            .then(|| Secret::new(String::clone(&fix.password))),
    };
    tracing::debug!(tab, session = id, auth = %overlay.auth_type, new_password = overlay.password.is_some(), "auth_fix: saving");
    app.core.call(
        "auth_fix save",
        move |st| async move {
            let base = api::sessions::session_get(&st, id).await?;
            let mut input = overlay.into_input(&base);
            input.env = api::sessions::session_env_get(&st, id).await?;
            api::sessions::session_update(&st, id, input).await
        },
        move |app, r| match r {
            Ok(updated) => {
                tracing::info!(tab, session = id, "auth_fix: saved, reconnecting");
                // Other views (sidebar Edit) must not keep the old row.
                app.sidebar.reload(&app.core);
                let Some(t) = app.tabs.get_mut(tab) else {
                    tracing::debug!(tab, "auth_fix: tab closed meanwhile");
                    return;
                };
                t.session = updated;
                if let Some(slot) = app.terms.get_mut(&tab) {
                    slot.auth_fix = None;
                }
                // Locked meanwhile: the vault is closed, the tab's Reconnect
                // will do after unlock.
                if matches!(app.screen, crate::app::Screen::Main) {
                    app.connect_tab(tab, false, false);
                }
            }
            Err(e) => {
                tracing::warn!(tab, session = id, "auth_fix: save failed: {e}");
                match app.terms.get_mut(&tab).and_then(|s| s.auth_fix.as_mut()) {
                    Some(fix) => {
                        fix.busy = false;
                        fix.err = Some(e.to_string());
                    }
                    None => app.toasts.danger("Session not saved", e.to_string()),
                }
            }
        },
    );
}

const METHODS: [(&str, Icon, &str, &str); 3] = [
    ("password", Icon::Key, "Password", "Saved with this session"),
    ("key", Icon::KeyRound, "Private Key", "PEM / OpenSSH key"),
    ("agent", Icon::Cpu, "SSH Agent", "Pageant / OpenSSH Agent"),
];

const CARD_W: f32 = 500.0;
const HEADER_H: f32 = widgets::DIALOG_HEAD_H;
const FOOTER_H: f32 = widgets::DIALOG_FOOTER_H;
const PAD: f32 = 24.0;

/// Paint the overlay of `tab` over its terminal `rect` (the tab's
/// `TermSlot::auth_fix` is Some).
pub fn show(app: &mut App, ui: &mut Ui, tab: TabId, rect: Rect) {
    let Some(mut fix) = app.terms.get_mut(&tab).and_then(|s| s.auth_fix.take()) else {
        return;
    };
    let t = tokens(ui.ctx());
    let mut cancel = false;
    let mut submit = false;

    // Backdrop: dims the terminal and keeps clicks off it.
    ui.painter().rect_filled(rect, 0.0, alpha(t.bg, 0.8));
    ui.interact(rect, Id::unique(("auth-fix-backdrop", tab)), Sense::click());

    let card_w = CARD_W.min(rect.width() - 32.0).max(200.0);
    let max_h = (rect.height() * 0.92).max(160.0);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).id_salt(("auth-fix", tab)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    // Measure the body once per frame from last frame's height.
    let body_h_id = Id::unique(("auth-fix-body-h", tab));
    let body_h: f32 = ui.ctx().data(|d| d.get_temp(body_h_id)).unwrap_or(260.0);
    let card_h = (HEADER_H + body_h + FOOTER_H).min(max_h);
    let card = Rect::from_center_size(rect.center(), vec2(card_w, card_h));
    let ui = &mut child;
    let card_id = Id::unique(("auth-fix-card", tab));
    ui.painter().add(
        widgets::dialog_frame(t)
            .shadow
            .as_shape(card, crate::style::radius::XL),
    );
    ui.painter()
        .rect_filled(card, crate::style::radius::XL, t.surface);
    ui.painter().rect_stroke(
        card,
        crate::style::radius::XL,
        egui::Stroke::new(1.0, t.border_strong),
        egui::StrokeKind::Inside,
    );

    // Header.
    let header = Rect::from_min_size(card.min, vec2(card_w, HEADER_H));
    let line = fix.header_line();
    if widgets::dialog_header(
        ui,
        card_id,
        header,
        Some(Icon::AlertTriangle),
        t.warning,
        "Fix authentication",
        Some(&line),
        true,
        fix.busy,
    ) {
        cancel = true;
    }

    // Body.
    let footer = Rect::from_min_max(pos2(card.left(), card.bottom() - FOOTER_H), card.max);
    let body = Rect::from_min_max(
        pos2(card.left(), header.bottom()),
        pos2(card.right(), footer.top()),
    );
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("auth-fix-scroll", tab))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let inner = Frame::new()
                    .inner_margin(Margin::same(PAD as i8))
                    .show(ui, |ui| {
                        ui.set_width(card_w - 2.0 * PAD);
                        ui.spacing_mut().item_spacing.y = 16.0;
                        form::note(ui, Icon::AlertTriangle, t.danger, &fix.message);
                        body_fields(app, ui, &mut fix);
                        if let Some(e) = &fix.err {
                            widgets::error_strip(ui, e);
                        }
                    });
                ui.ctx()
                    .data_mut(|d| d.insert_temp(body_h_id, inner.response.rect.height()));
            });
    });

    // Footer.
    widgets::dialog_footer(ui, footer, PAD, |ui| {
        if widgets::Button::primary("Save & reconnect")
            .icon(Icon::PlugZap)
            .enabled(!fix.busy)
            .show(ui)
            .clicked()
        {
            submit = true;
        }
        if widgets::Button::secondary("Close tab").show(ui).clicked() && !fix.busy {
            cancel = true;
        }
    });

    // Escape: only for the active tab's overlay, and not while a dialog
    // (the credentials manager) is on top.
    if !fix.busy
        && app.tabs.active == Some(tab)
        && !app.any_modal_open()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
    {
        cancel = true;
    }

    if cancel {
        // Cancelling the fix closes the tab. Dropping
        // `fix` wipes the typed password.
        tracing::debug!(tab, "auth_fix: cancelled, closing the tab");
        app.close_tab(tab);
        return;
    }
    if submit {
        save(app, &mut fix, tab);
    }
    if let Some(slot) = app.terms.get_mut(&tab) {
        slot.auth_fix = Some(fix);
    }
}

fn body_fields(app: &mut App, ui: &mut Ui, fix: &mut AuthFix) {
    form::field(ui, "Username", None, |ui| {
        let r = form::input(ui, &mut fix.username, InputOpts::new(""));
        labelled(ui, &r, "Username");
        if std::mem::take(&mut fix.focus_username) {
            r.request_focus();
        }
    });
    form::field(ui, "Auth method", None, |ui| {
        let current = fix.auth_type.clone();
        if let Some(method) = form::option_cards(ui, &current.as_str(), &METHODS) {
            fix.switch_method(method);
        }
    });
    match fix.auth_type.as_str() {
        "password" => {
            let stored = fix.password_stored();
            form::field(ui, "Password", None, |ui| {
                let hint = if stored {
                    "Saved — leave blank to keep"
                } else {
                    "Password"
                };
                let r = form::secret(ui, &mut fix.password, hint, &mut fix.show_password, None);
                labelled(ui, &r, "Password");
            });
        }
        "key" => {
            form::field(ui, "Private key", None, |ui| {
                fix.key_picker.show(app, ui, &mut fix.credential_id)
            });
            form::field(ui, "Key passphrase (optional)", None, |ui| {
                fix.pass_picker.show(app, ui, &mut fix.passphrase_id)
            });
        }
        _ => super::session_dialog::agent_hint(ui),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(auth: &str, credential: Option<i64>) -> Session {
        Session {
            id: 7,
            folder_id: None,
            name: "box".into(),
            host: "10.0.0.1".into(),
            port: 2222,
            username: "root".into(),
            auth_type: auth.into(),
            credential_id: credential,
            key_passphrase_credential_id: None,
            color: None,
            sort: 0,
            initial_command: Some("htop".into()),
            scrollback_lines: 5000,
            font_size: 13,
            font_family: String::new(),
            cursor_style: "block".into(),
            compression: 0,
            keepalive_secs: 60,
            connect_timeout_secs: 15,
            session_kind: "ssh".into(),
            forward_x11: 0,
            starting_dir: None,
        }
    }

    #[test]
    fn validation_and_password_rule() {
        let mut f = AuthFix::new(
            1,
            session("password", Some(3)),
            "Authentication failed".into(),
        );
        assert_eq!(f.header_line(), "root@10.0.0.1:2222");
        assert_eq!(f.validate(), None, "a stored password may be kept");
        assert!(f.to_input().password.is_none());
        f.password.push_str("hunter2");
        let i = f.to_input();
        assert_eq!(
            (i.password.as_ref().map(|p| p.expose()), i.credential_id),
            (Some("hunter2"), None)
        );
        assert_eq!(
            i.initial_command.as_deref(),
            Some("htop"),
            "untouched fields kept"
        );
        f.switch_method("key");
        assert_eq!(f.validate(), Some("Pick or add a credential"));
        assert!(
            f.to_input().password.is_none(),
            "no password for a key session"
        );
        f.username = " ".into();
        assert_eq!(f.validate(), Some("Username is required"));

        let f = AuthFix::new(1, session("password", None), String::new());
        assert_eq!(f.validate(), Some("Password is required"));
    }
}
