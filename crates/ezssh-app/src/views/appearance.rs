//! Appearance dialog for one tab, opened from the terminal and tab-header
//! context menus (Appearance…). Same fields as the session dialog: tab
//! colour, font family and size, scrollback, cursor style.
//!
//! Colour, font and cursor show on the tab while the dialog is open (the
//! scrollback waits, shrinking it drops lines); Cancel, Escape or ✕ put
//! them back. "Apply to tab" keeps the values for this tab only. "Save to
//! session" writes them to the session row and every open tab of that
//! session follows ([`App::apply_session_terminal`]).

use std::sync::atomic::{AtomicU64, Ordering};

use egui::Id;
use ezssh_core::api;
use ezssh_core::db::sessions::SessionInput;
use ezssh_term::CursorKind;

use super::session_dialog::{
    cursor_style, font_family_picker, number_field, swatches, two_columns,
};
use crate::app::App;
use crate::font_presets::{self, MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::form;
use crate::icons::Icon;
use crate::style::tokens;
use crate::tabs::TabId;
use crate::widgets::{self, DialogPart};

const WIDTH: f32 = 480.0;

/// Tells async results which dialog instance they belong to.
static SERIAL: AtomicU64 = AtomicU64::new(1);

/// The appearance fields of a session row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Look {
    color: Option<String>,
    font_family: String,
    font_size: i64,
    cursor_style: String,
    scrollback_lines: i64,
}

pub struct AppearanceDialog {
    pub tab: TabId,
    serial: u64,
    /// The tab as it was when the dialog opened (Cancel restores it).
    before: Look,
    draft: Look,
    /// What the tab shows now (the preview).
    shown: Look,
    font_custom: bool,
    err: Option<String>,
    busy: bool,
}

fn cursor_setting(c: CursorKind) -> &'static str {
    match c {
        CursorKind::Block => "block",
        CursorKind::Underline => "underline",
        CursorKind::Bar => "bar",
    }
}

/// Open the dialog on `tab` with what the tab shows now.
pub fn open(app: &mut App, tab: TabId) {
    let (Some(t), Some(slot)) = (app.tabs.get(tab), app.terms.get(&tab)) else {
        tracing::warn!(tab, "appearance: tab or terminal gone, not opening");
        return;
    };
    let look = Look {
        color: t.color.clone(),
        font_family: t.font_family.clone(),
        font_size: (slot.term.font_size().round() as i64).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE),
        cursor_style: cursor_setting(slot.term.options().cursor).to_string(),
        scrollback_lines: slot.term.options().scrollback as i64,
    };
    let font_custom = !look.font_family.is_empty() && !font_presets::is_preset(&look.font_family);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(tab, serial, ?look, "appearance: open");
    app.dialogs.appearance = Some(AppearanceDialog {
        tab,
        serial,
        before: look.clone(),
        draft: look.clone(),
        shown: look,
        font_custom,
        err: None,
        busy: false,
    });
}

/// The session dialog's rules for these fields.
fn validate(l: &Look) -> Option<&'static str> {
    if !(0..=100_000).contains(&l.scrollback_lines) {
        return Some("Scrollback must be between 0 and 100000");
    }
    if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&l.font_size) {
        return Some("Font size must be between 8 and 48");
    }
    None
}

/// Show colour, font and cursor on the tab (an out-of-range size is left
/// as it is until fixed). Returns what the tab shows afterwards.
fn preview(app: &mut App, tab: TabId, shown: &Look, want: &Look) -> Look {
    let mut now = shown.clone();
    if want.color != shown.color
        && let Some(t) = app.tabs.get_mut(tab)
    {
        t.color = want.color.clone();
        now.color = want.color.clone();
    }
    let size_ok = (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&want.font_size);
    let family = want.font_family != shown.font_family;
    let size_changed = size_ok && want.font_size != shown.font_size;
    let cursor = want.cursor_style != shown.cursor_style;
    if !(family || size_changed || cursor) {
        return now;
    }
    let faces = family.then(|| app.fonts.faces(&app.ctx, &want.font_family));
    let Some(slot) = app.terms.get_mut(&tab) else {
        tracing::warn!(tab, "appearance: no terminal for the preview");
        return now;
    };
    if let Some(faces) = faces {
        slot.term.set_faces(faces);
        now.font_family = want.font_family.clone();
    }
    if size_changed {
        slot.term.set_font_size(want.font_size as f32);
        now.font_size = want.font_size;
    }
    if cursor {
        let blink = slot.term.options().cursor_blink;
        slot.term
            .set_cursor(CursorKind::from_setting(&want.cursor_style), blink);
        now.cursor_style = want.cursor_style.clone();
    }
    tracing::debug!(tab, ?now, "appearance: preview");
    now
}

/// Make `look` the tab's own: preview plus the scrollback and the tab's
/// remembered family.
fn apply_to_tab(app: &mut App, tab: TabId, shown: &Look, look: &Look) {
    preview(app, tab, shown, look);
    if let Some(t) = app.tabs.get_mut(tab) {
        t.font_family = look.font_family.clone();
        t.color = look.color.clone();
    }
    match app.terms.get_mut(&tab) {
        Some(slot) => slot
            .term
            .set_scrollback(look.scrollback_lines.max(0) as usize),
        None => tracing::warn!(tab, "appearance: no terminal, scrollback not applied"),
    }
    tracing::info!(tab, ?look, "appearance: applied to tab");
}

enum Action {
    Cancel,
    Apply,
    Save,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.appearance.take() else {
        return;
    };
    let Some(tab) = app.tabs.get(dlg.tab) else {
        tracing::debug!(tab = dlg.tab, "appearance: tab gone, closing");
        return;
    };
    let title = tab.session.name.clone();
    let t = tokens(ctx);
    let mut action = None;
    let out = widgets::Dialog::new(Id::unique("appearance-dialog"), WIDTH)
        .row_gap(16.0)
        .header(Icon::Palette, t.accent, "Appearance", Some(&title))
        .plain_subtitle()
        .footer(widgets::DialogFooter::Strip)
        .busy(dlg.busy)
        .show(ctx, |ui, part| match part {
            DialogPart::Body => {
                let d = &mut dlg.draft;
                form::field(ui, "Tab color", None, |ui| swatches(ui, &mut d.color));
                form::field(
                    ui,
                    "Font family",
                    Some("(default) uses the bundled monospace font. The list shows the monospace families installed on this computer."),
                    |ui| font_family_picker(ui, &mut d.font_family, &mut dlg.font_custom),
                );
                two_columns(
                    ui,
                    |ui, w| form::field(ui, "Font size", None, |ui| number_field(ui, "Font size", &mut d.font_size, w)),
                    |ui, w| {
                        form::field(ui, "Scrollback lines", Some("0 disables scrollback."), |ui| {
                            number_field(ui, "Scrollback lines", &mut d.scrollback_lines, w)
                        })
                    },
                );
                form::field(ui, "Cursor style", None, |ui| cursor_style(ui, &mut d.cursor_style));
                if let Some(e) = &dlg.err {
                    widgets::error_strip(ui, e);
                }
                if form::ctrl_enter(ui) {
                    action = Some(Action::Save);
                }
            }
            DialogPart::Footer => {
                if widgets::Button::primary("Save to session").enabled(!dlg.busy).show(ui)
                    .on_hover_text("Save as the session's default; its other open tabs follow")
                    .clicked()
                {
                    action = Some(Action::Save);
                }
                if widgets::Button::secondary("Apply to tab").enabled(!dlg.busy).show(ui).on_hover_text("Use for this tab only").clicked() {
                    action = Some(Action::Apply);
                }
                if widgets::Button::secondary("Cancel").show(ui).clicked() {
                    action = Some(Action::Cancel);
                }
            }
        });
    if action.is_none() && out.dismissed() && !dlg.busy {
        action = Some(Action::Cancel);
    }

    dlg.shown = preview(app, dlg.tab, &dlg.shown, &dlg.draft);

    match action {
        Some(Action::Cancel) => {
            tracing::debug!(tab = dlg.tab, "appearance: cancelled, restoring the tab");
            let before = dlg.before.clone();
            apply_to_tab(app, dlg.tab, &dlg.shown, &before);
            return;
        }
        Some(Action::Apply) => match validate(&dlg.draft) {
            Some(problem) => dlg.err = Some(problem.into()),
            None => {
                apply_to_tab(app, dlg.tab, &dlg.shown, &dlg.draft);
                return;
            }
        },
        Some(Action::Save) if !dlg.busy => match validate(&dlg.draft) {
            Some(problem) => dlg.err = Some(problem.into()),
            None => {
                dlg.err = None;
                dlg.busy = true;
                save(app, dlg.tab, dlg.serial, dlg.draft.clone());
            }
        },
        _ => {}
    }
    app.dialogs.appearance = Some(dlg);
}

/// Write `look` to the tab's session row. Reads the current row first so
/// edits made since the tab opened survive, and fails instead of dropping
/// the env list when it cannot be read.
fn save(app: &mut App, tab: TabId, serial: u64, look: Look) {
    let Some(t) = app.tabs.get(tab) else {
        tracing::warn!(tab, "appearance: save for a closed tab");
        return;
    };
    let id = t.session.id;
    tracing::debug!(
        tab,
        session = id,
        ?look,
        "appearance: saving to the session"
    );
    let sent = look.clone();
    app.core.call(
        "session_update(appearance)",
        move |st| async move {
            let s = api::sessions::session_get(&st, id).await?;
            let env = api::sessions::session_env_get(&st, id).await?;
            let input = SessionInput {
                folder_id: s.folder_id,
                name: s.name,
                host: s.host,
                port: s.port,
                username: s.username,
                auth_type: s.auth_type,
                // A password session's own row: the core keeps it.
                credential_id: s.credential_id,
                key_passphrase_credential_id: s.key_passphrase_credential_id,
                color: sent.color,
                initial_command: s.initial_command,
                scrollback_lines: sent.scrollback_lines,
                font_size: sent.font_size,
                font_family: sent.font_family,
                cursor_style: sent.cursor_style,
                compression: s.compression,
                keepalive_secs: s.keepalive_secs,
                connect_timeout_secs: s.connect_timeout_secs,
                env,
                session_kind: s.session_kind,
                forward_x11: s.forward_x11,
                starting_dir: s.starting_dir,
                password: None,
            };
            api::sessions::session_update(&st, id, input).await
        },
        move |app, r| {
            let mine = app
                .dialogs
                .appearance
                .as_ref()
                .is_some_and(|d| d.serial == serial);
            match r {
                Ok(saved) => {
                    tracing::info!(tab, session = id, "appearance: saved to the session");
                    // This tab first: it may hold a tab-only value equal to
                    // nothing the session row changed.
                    if mine && let Some(d) = app.dialogs.appearance.take() {
                        apply_to_tab(app, tab, &d.shown, &look);
                    }
                    app.apply_session_terminal(&saved);
                    app.sidebar.reload(&app.core);
                    app.toasts.success("Appearance saved", None);
                }
                Err(e) => {
                    tracing::warn!(tab, session = id, "appearance: save failed: {e}");
                    match app
                        .dialogs
                        .appearance
                        .as_mut()
                        .filter(|d| d.serial == serial)
                    {
                        Some(d) => {
                            d.busy = false;
                            d.err = Some(e.to_string());
                        }
                        None => app.toasts.danger("Appearance not saved", e.to_string()),
                    }
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look {
            color: None,
            font_family: String::new(),
            font_size: 14,
            cursor_style: "block".into(),
            scrollback_lines: 1000,
        }
    }

    #[test]
    fn validation_matches_the_session_dialog() {
        assert_eq!(validate(&look()), None);
        assert_eq!(
            validate(&Look {
                font_size: 7,
                ..look()
            }),
            Some("Font size must be between 8 and 48")
        );
        assert_eq!(
            validate(&Look {
                scrollback_lines: -1,
                ..look()
            }),
            Some("Scrollback must be between 0 and 100000")
        );
    }

    #[test]
    fn cursor_round_trip() {
        for s in ["block", "underline", "bar"] {
            assert_eq!(cursor_setting(CursorKind::from_setting(s)), s);
        }
    }
}
