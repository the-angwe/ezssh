//! `font-picker-popover.tsx`: the terminal context menu's font picker for
//! one tab. Every change applies to the terminal live; "Save" (or "Done"
//! with "Save as session default" ticked) writes size and family to the
//! session row.

use egui::{Align, Color32, CornerRadius, Frame, Id, Key, Layout, Margin, Modifiers, Order, Pos2, RichText, Stroke, Ui};
use ezssh_core::api;
use ezssh_core::db::sessions::SessionInput;

use crate::app::App;
use crate::font_presets::{self, MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::form::{self, InputOpts};
use crate::icons::Icon;
use crate::style::{alpha, font, font_semibold, size, tokens};
use crate::tabs::TabId;
use crate::widgets::{self, Tone};

const WIDTH: f32 = 260.0;
/// The select's value for a family that is not a preset.
const CUSTOM: &str = "\u{0}custom";

pub struct FontPicker {
    pub tab: TabId,
    /// Where the request came from (pointer position), if known.
    pub anchor: Option<Pos2>,
    /// Live family and size; `None` until the first frame reads the tab.
    live: Option<(String, i64)>,
    /// The size box while it is being typed into.
    size_draft: Option<String>,
    persist: bool,
    /// Past the first frame (the click that opened it is not "outside").
    shown: bool,
}

impl FontPicker {
    pub fn new(tab: TabId, anchor: Option<Pos2>) -> Self {
        Self { tab, anchor, live: None, size_draft: None, persist: false, shown: false }
    }
}

/// The family last applied to a tab (the webview's `liveFontFamily`, which
/// outlives the popover); the size is read back from the terminal.
fn live_family_id(tab: TabId) -> Id {
    Id::unique(("font-picker-live-family", tab))
}

/// The tab's session font was saved elsewhere (session dialog) and applied
/// to the terminal: drop the picker's remembered family and, if the picker
/// is open on that tab, make it read the tab again next frame.
pub(crate) fn session_font_changed(app: &mut App, tab: TabId) {
    app.ctx.data_mut(|d| d.remove::<String>(live_family_id(tab)));
    if let Some(p) = app.dialogs.font_picker.as_mut().filter(|p| p.tab == tab) {
        tracing::debug!(tab, "font picker: session font saved elsewhere, re-reading the tab");
        p.live = None;
        p.size_draft = None;
    }
}

enum Action {
    Close,
    Save,
    Done,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut p) = app.dialogs.font_picker.take() else {
        return;
    };
    let (Some(tab), Some(slot)) = (app.tabs.get(p.tab), app.terms.get(&p.tab)) else {
        tracing::debug!(tab = p.tab, "font picker: tab gone, closing");
        return;
    };
    let (mut family, mut size_pt) = p.live.clone().unwrap_or_else(|| {
        let family = ctx.data(|d| d.get_temp::<String>(live_family_id(p.tab))).unwrap_or_else(|| tab.session.font_family.clone());
        (family, (slot.term.font_size().round() as i64).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE))
    });
    let before = (family.clone(), size_pt);
    let t = tokens(ctx);
    let popup_was_open = egui::Popup::is_any_open(ctx);
    let area_id = Id::unique("font-picker");

    // Near the pointer, kept on screen (last frame's height, if any).
    let screen = ctx.content_rect();
    let h = ctx.memory(|m| m.area_rect(area_id)).map_or(250.0, |r| r.height());
    let at = p.anchor.unwrap_or(Pos2::new(screen.right() - WIDTH - 8.0, screen.top() + 40.0));
    let pos = Pos2::new(
        at.x.min(screen.right() - WIDTH - 8.0).max(screen.left() + 8.0),
        at.y.min(screen.bottom() - h - 8.0).max(screen.top() + 8.0),
    );

    let mut action = None;
    let area = egui::Area::new(area_id).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(12, 8))
            .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(90) })
            .show(ui, |ui| {
                ui.set_width(WIDTH - 24.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Font").font(font_semibold(size::XS)).color(t.fg));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(ui, Icon::X, 20.0, 12.0, false, "Close font picker").clicked() {
                            action = Some(Action::Close);
                        }
                    });
                });
                ui.add_space(4.0);

                caption(ui, "Family");
                // Non-presets show as "(custom)" and stay untouched; they
                // come from the session dialog.
                let mut options: Vec<(String, String)> = font_presets::choices().to_vec();
                let mut choice = family.clone();
                if !font_presets::is_preset(&family) {
                    options.push((CUSTOM.to_string(), format!("{family} (custom)")));
                    choice = CUSTOM.to_string();
                }
                if form::select(ui, Id::unique("font-picker-family"), &mut choice, &options, Some(WIDTH - 24.0)) && choice != CUSTOM {
                    family = choice;
                }
                ui.add_space(8.0);

                caption(ui, "Size");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let dec = ui.add_enabled_ui(size_pt > MIN_FONT_SIZE, |ui| widgets::icon_button(ui, Icon::Minus, 28.0, 12.0, false, "Decrease font size"));
                    if dec.inner.clicked() {
                        size_pt = (size_pt - 1).max(MIN_FONT_SIZE);
                    }
                    size_box(ui, &mut p.size_draft, &mut size_pt);
                    let inc = ui.add_enabled_ui(size_pt < MAX_FONT_SIZE, |ui| widgets::icon_button(ui, Icon::Plus, 28.0, 12.0, false, "Increase font size"));
                    if inc.inner.clicked() {
                        size_pt = (size_pt + 1).min(MAX_FONT_SIZE);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new("pt").font(font(size::XS - 1.0)).color(t.muted));
                    });
                });
                ui.add_space(8.0);

                ui.checkbox(&mut p.persist, RichText::new("Save as session default").font(font(size::XS)).color(t.fg));
                ui.add_space(4.0);
                let r = ui.available_rect_before_wrap();
                ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, alpha(t.border, 0.7)));
                ui.add_space(4.0);
                widgets::button_row(ui, |ui| {
                    if widgets::primary(ui, "Done", Tone::Accent, true, size::XS).clicked() {
                        action = Some(Action::Done);
                    }
                    // The TSX closes after saving too, despite its title.
                    if widgets::Ghost::new("Save").icon(Icon::Save).compact().show(ui).on_hover_text("Save now without closing").clicked() {
                        action = Some(Action::Save);
                    }
                });
            });
    });

    if (family.clone(), size_pt) != before {
        apply(app, ctx, p.tab, &family, size_pt);
    }
    p.live = Some((family.clone(), size_pt));

    // Outside click or Escape closes (not while the family list is open:
    // its own click or Escape closes the list first).
    let rect = area.response.rect;
    let outside = p.shown && !popup_was_open && ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|o| !rect.contains(o)));
    if action.is_none() && (outside || (!popup_was_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)))) {
        action = Some(Action::Close);
    }
    p.shown = true;

    match action {
        Some(Action::Close) => tracing::debug!(tab = p.tab, "font picker: closed"),
        Some(Action::Save) => save(app, p.tab, size_pt, family),
        Some(Action::Done) => {
            if p.persist {
                save(app, p.tab, size_pt, family);
            }
            tracing::debug!(tab = p.tab, persist = p.persist, "font picker: done");
        }
        None => app.dialogs.font_picker = Some(p),
    }
}

/// `text-[11px] text-muted` field caption.
fn caption(ui: &mut Ui, text: &str) {
    let t = tokens(ui.ctx());
    ui.label(RichText::new(text).font(font(size::XS)).color(t.muted));
}

/// The size box: commits on Enter or blur, clamped, so typing "1" on the
/// way to "13" does not jump to the minimum.
fn size_box(ui: &mut Ui, draft: &mut Option<String>, value: &mut i64) {
    let mut text = draft.clone().unwrap_or_else(|| value.to_string());
    let resp = form::input(ui, &mut text, InputOpts::new("").width(64.0)).accessible_name("Font size (pt)");
    if resp.has_focus() {
        *draft = Some(text);
        return;
    }
    if resp.lost_focus() {
        match text.trim().parse::<i64>() {
            Ok(n) => *value = n.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE),
            Err(_) => tracing::debug!(input = %text, "font picker: not a number, kept {value}"),
        }
    }
    *draft = None;
}

/// Apply to the terminal now (the grid and the PTY follow next frame).
fn apply(app: &mut App, ctx: &egui::Context, tab: TabId, family: &str, size_pt: i64) {
    tracing::debug!(tab, family, size = size_pt, "font picker: applied");
    let faces = app.fonts.faces(ctx, family);
    if let Some(slot) = app.terms.get_mut(&tab) {
        slot.term.set_font_size(size_pt as f32);
        slot.term.set_faces(faces);
    }
    ctx.data_mut(|d| d.insert_temp(live_family_id(tab), family.to_string()));
}

/// `saveFontToSession`: write size and family to the session row. Reads
/// the current row first (the TSX resent the tab's copy, which could undo
/// edits made since the tab opened) and fails instead of dropping the env
/// list when it cannot be read.
fn save(app: &mut App, tab: TabId, size_pt: i64, family: String) {
    let Some(t) = app.tabs.get(tab) else {
        tracing::warn!(tab, "font picker: save for a closed tab");
        return;
    };
    let id = t.session.id;
    tracing::debug!(tab, session = id, size = size_pt, family = %family, "font picker: saving as the session default");
    app.core.call(
        "session_update(font)",
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
                credential_id: s.credential_id,
                key_passphrase_credential_id: s.key_passphrase_credential_id,
                color: s.color,
                initial_command: s.initial_command,
                scrollback_lines: s.scrollback_lines,
                font_size: size_pt,
                font_family: family,
                cursor_style: s.cursor_style,
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
        move |app, r| match r {
            Ok(updated) => {
                tracing::info!(tab, session = id, "font picker: saved as the session default");
                if let Some(t) = app.tabs.get_mut(tab) {
                    t.session = updated;
                }
                app.sidebar.reload(&app.core);
            }
            Err(e) => {
                tracing::warn!(tab, session = id, "font picker: save failed: {e}");
                app.toasts.danger("Font not saved", e.to_string());
            }
        },
    );
}
