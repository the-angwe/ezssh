//! The unlock screen: create the vault on first run, else unlock it.

use egui::{Align2, Color32, CornerRadius, CursorIcon, Frame, Key, Margin, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, Vec2, vec2};
use ezssh_core::api;
use zeroize::Zeroize;

use crate::app::{App, Screen};
use crate::core::Core;
use crate::icons::Icon;
use crate::style::{alpha, font, font_semibold, size, tokens};
use crate::views::vault_dialogs::{self, RecoveryUnlock, ResetVault};
use crate::widgets;

pub struct UnlockState {
    pub first_run: bool,
    pw: String,
    pw2: String,
    show: bool,
    error: Option<String>,
    busy: bool,
    recovery_available: bool,
    focused: bool,
    /// "Use recovery code" dialog.
    pub(crate) recovery: Option<RecoveryUnlock>,
    /// "Forgot password?" (reset vault) dialog.
    pub(crate) reset: Option<ResetVault>,
}

impl Drop for UnlockState {
    fn drop(&mut self) {
        self.pw.zeroize();
        self.pw2.zeroize();
    }
}

impl UnlockState {
    pub fn new(first_run: bool) -> Self {
        Self { first_run, pw: String::new(), pw2: String::new(), show: false, error: None, busy: false, recovery_available: false, focused: false, recovery: None, reset: None }
    }

    /// Give the password field the keyboard again (a dialog closed).
    pub(crate) fn refocus(&mut self) {
        self.focused = false;
    }

    #[cfg(test)]
    pub fn recovery_available(&self) -> bool {
        self.recovery_available
    }

    #[cfg(test)]
    pub fn reset_dialog(&self) -> Option<&ResetVault> {
        self.reset.as_ref()
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Ask whether the locked vault has a recovery code, so the link only
    /// shows when it would work.
    pub fn probe_recovery(&mut self, core: &Core) {
        if self.first_run {
            return;
        }
        core.call(
            "vault_recovery_status",
            |st| async move { api::vault::vault_recovery_status(&st).await },
            |app, r| {
                if let Screen::Unlock(u) = &mut app.screen {
                    u.recovery_available = r.as_ref().is_ok_and(|s| s.provisioned);
                }
                if let Err(e) = r {
                    tracing::debug!("unlock: recovery status: {e}");
                }
            },
        );
    }
}

/// Password strength from length and character classes (0..=4).
pub fn score_password(pw: &str) -> usize {
    if pw.is_empty() {
        return 0;
    }
    let len = pw.chars().count();
    let mut score = [8, 12, 16].iter().filter(|n| len >= **n).count();
    let classes = [
        pw.chars().any(|c| c.is_ascii_lowercase()),
        pw.chars().any(|c| c.is_ascii_uppercase()),
        pw.chars().any(|c| c.is_ascii_digit()),
        pw.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if classes >= 3 {
        score += 1;
    }
    score.min(4)
}

pub(crate) const STRENGTH_COPY: [&str; 5] = ["Too short", "Weak", "Fair", "Strong", "Very strong"];

pub fn show(app: &mut App, ui: &mut Ui) {
    let t = tokens(ui.ctx());
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, t.bg);
    let Screen::Unlock(state) = &mut app.screen else {
        return;
    };
    let mut submit = false;
    egui::Area::new(egui::Id::unique("unlock-card")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ui.ctx(), |ui| {
        Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::same(24))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(115) })
            .show(ui, |ui| {
                let w = 332.0;
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 3.0, alpha(t.accent, 0.15));
                    (if state.first_run { Icon::ShieldCheck } else { Icon::Lock }).paint(ui, r, 20.0, t.accent);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(if state.first_run { "Set master password" } else { "Unlock ezSsh" })
                                .font(font_semibold(size::BASE))
                                .color(t.fg),
                        );
                        ui.label(
                            RichText::new(if state.first_run {
                                "Encrypts your vault. Cannot be recovered."
                            } else {
                                "Enter your master password."
                            })
                            .font(font(size::XS))
                            .color(t.muted),
                        );
                    });
                });
                ui.add_space(8.0);

                // Password + show/hide toggle.
                let field = ui.allocate_ui(vec2(w, 32.0), |ui| {
                    let r = widgets::text_input(ui, &mut state.pw, "Master password", w, !state.show);
                    let eye = egui::Rect::from_center_size(egui::pos2(r.rect.right() - 16.0, r.rect.center().y), vec2(28.0, 28.0));
                    let eye_resp = ui.interact(eye, ui.scope_id().with("eye"), egui::Sense::click());
                    widgets::paint_icon_button(ui, &eye_resp, eye, if state.show { Icon::EyeOff } else { Icon::Eye }, 13.0, false, None);
                    widgets::a11y(&eye_resp, egui::Role::Button, if state.show { "Hide password" } else { "Show password" });
                    if eye_resp.on_hover_text(if state.show { "Hide" } else { "Show" }).clicked() {
                        state.show = !state.show;
                    }
                    r
                });
                let pw_resp = field.inner;
                if !state.focused {
                    state.focused = true;
                    pw_resp.request_focus();
                }
                let mut enter = pw_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));

                if state.first_run {
                    let r = widgets::text_input(ui, &mut state.pw2, "Confirm password", w, !state.show);
                    enter |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if !state.pw.is_empty() {
                        strength_meter(ui, &state.pw, w);
                    }
                }

                if let Some(e) = &state.error {
                    widgets::error_strip(ui, e);
                }

                ui.add_space(4.0);
                let label = if state.first_run { "Create vault" } else { "Unlock" };
                if widgets::primary_full(ui, label, !state.busy, w).clicked() || enter {
                    submit = true;
                }

                if !state.first_run {
                    ui.add_space(4.0);
                    // Not while an unlock is running: its result would land
                    // under a dialog that changes the vault.
                    let idle = !state.busy;
                    ui.horizontal(|ui| {
                        if state.recovery_available && link(ui, Icon::KeyRound, "Use recovery code", t.fg).clicked() && idle {
                            tracing::debug!("unlock: recovery dialog opened");
                            state.recovery = Some(RecoveryUnlock::default());
                        }
                        let forgot_w = link_width(ui, "Forgot password?");
                        ui.add_space((ui.available_width() - forgot_w).max(0.0));
                        if link(ui, Icon::Trash, "Forgot password?", t.danger).clicked() && idle {
                            tracing::debug!("unlock: reset dialog opened");
                            state.reset = Some(ResetVault::default());
                        }
                    });
                }
            });
    });
    if submit && !state.busy {
        submit_unlock(app);
    }
    vault_dialogs::unlock_dialogs(app, ui.ctx());
}

/// The four-segment strength bar and its caption,
/// shared with the change-password dialog.
pub(crate) fn strength_meter(ui: &mut Ui, pw: &str, w: f32) {
    let t = tokens(ui.ctx());
    let score = score_password(pw);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let (bar, resp) = ui.allocate_exact_size(vec2(w, 4.0), Sense::hover());
        widgets::a11y(&resp, egui::Role::Meter, &format!("Password strength: {}", STRENGTH_COPY[score]));
        let gap = 4.0;
        let seg = (w - 3.0 * gap) / 4.0;
        let fill = match score {
            0 | 1 => t.danger,
            2 => t.warning,
            _ => t.success,
        };
        for i in 0..4 {
            let r = Rect::from_min_size(Pos2::new(bar.left() + i as f32 * (seg + gap), bar.top()), vec2(seg, 4.0));
            ui.painter().rect_filled(r, 1.0, if i < score { fill } else { t.surface2 });
        }
        ui.label(RichText::new(STRENGTH_COPY[score]).font(font(size::XS)).color(t.muted));
    });
}

const LINK_ICON: f32 = 11.0;

fn link_width(ui: &Ui, text: &str) -> f32 {
    let g = ui.painter().layout_no_wrap(text.into(), font(size::XS), Color32::WHITE);
    4.0 + LINK_ICON + 4.0 + g.size().x + 4.0
}

/// The card's small text links (`px-1`, icon then text, `hover` colour).
fn link(ui: &mut Ui, icon: Icon, text: &str, hover: Color32) -> Response {
    let t = tokens(ui.ctx());
    let g = ui.painter().layout_no_wrap(text.into(), font(size::XS), t.muted);
    let (rect, resp) = ui.allocate_exact_size(vec2(link_width(ui, text), g.size().y.max(LINK_ICON) + 4.0), Sense::click());
    let color = if resp.hovered() { hover } else { t.muted };
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 4.0 + LINK_ICON / 2.0, rect.center().y), Vec2::splat(LINK_ICON));
    icon.paint(ui, icon_rect, LINK_ICON, color);
    ui.painter().galley_with_override_text_color(Pos2::new(icon_rect.right() + 4.0, rect.center().y - g.size().y / 2.0), g, color);
    widgets::focus_ring(ui, &resp, rect);
    widgets::a11y(&resp, egui::Role::Button, text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn submit_unlock(app: &mut App) {
    let Screen::Unlock(state) = &mut app.screen else {
        return;
    };
    state.error = None;
    if state.first_run && state.pw != state.pw2 {
        state.error = Some("Passwords do not match".into());
        return;
    }
    if state.first_run && state.pw.chars().count() < 8 {
        state.error = Some("Minimum 8 characters".into());
        return;
    }
    state.busy = true;
    let first_run = state.first_run;
    let pw = state.pw.clone();
    app.core.call(
        if first_run { "vault_init" } else { "vault_unlock" },
        move |st| async move {
            if first_run { api::vault::vault_init(&st, pw).await } else { api::vault::vault_unlock(&st, pw).await }
        },
        |app, r| {
            let Screen::Unlock(state) = &mut app.screen else {
                return;
            };
            state.busy = false;
            match r {
                Ok(()) => app.on_unlocked(),
                Err(e) => {
                    tracing::info!("unlock: refused: {e}");
                    state.error = Some(match e {
                        ezssh_core::AppError::BadPassword => "Incorrect master password".to_string(),
                        other => other.to_string(),
                    });
                    state.focused = false;
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::score_password;

    #[test]
    fn strength_scores() {
        assert_eq!(score_password(""), 0);
        assert_eq!(score_password("abc"), 0);
        assert_eq!(score_password("abcdefgh"), 1);
        assert_eq!(score_password("abcdefghijkl"), 2);
        assert_eq!(score_password("Abcdefgh1"), 2);
        assert_eq!(score_password("Abcdefghijkl1!"), 3);
        assert_eq!(score_password("Abcdefghijklmnop1!"), 4);
        assert_eq!(score_password("aaaaaaaaaaaaaaaaaaaaaaaaa"), 3);
    }
}
