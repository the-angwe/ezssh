//! Form controls shared by the dialogs: sections and labelled fields, inputs
//! (text, secret with an eye toggle, number with a stepper, textarea,
//! select), checkboxes, option cards and key caps.

use egui::{Color32, CornerRadius, CursorIcon, Frame, Id, Key, Margin, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::icons::Icon;
use crate::style::{CONTROL_H, CONTROL_H_SM, alpha, font, font_semibold, radius, size, tokens};
use crate::widgets::{self, a11y_selected};

/// Uppercase muted caption (11 px, semibold, tracking by case).
pub fn caption(ui: &mut Ui, text: &str) -> Response {
    let t = tokens(ui.ctx());
    ui.label(RichText::new(text.to_uppercase()).font(font_semibold(size::XS)).color(t.muted))
}

/// Muted helper text under a field, wrapping.
pub fn hint(ui: &mut Ui, text: &str) -> Response {
    let t = tokens(ui.ctx());
    ui.add(egui::Label::new(RichText::new(text).font(font(size::SM)).color(t.muted)).wrap())
}

/// A form section: a 14 px semibold title, the body with 12 px between
/// rows, and a hairline between consecutive sections (no card).
pub fn section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    section_with(ui, title, None, add)
}

/// [`section`] with a 12 px muted description under the title.
pub fn section_with<R>(ui: &mut Ui, title: &str, description: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = tokens(ui.ctx());
    // A hairline before every section but the first of this Ui, tracked
    // per layout pass (repeat passes of one frame share a pass number
    // only in the sense that each pass starts the sequence over).
    let seq = ui.scope_id().with("form-section-pass");
    let pass = ui.ctx().cumulative_pass_nr();
    let first = ui.ctx().data(|d| d.get_temp::<u64>(seq)) != Some(pass);
    ui.ctx().data_mut(|d| d.insert_temp(seq, pass));
    ui.vertical(|ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        if !first {
            ui.add_space(20.0);
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, t.border));
            ui.add_space(20.0);
        }
        ui.label(RichText::new(title).font(font_semibold(size::MD)).color(t.fg));
        if let Some(d) = description {
            ui.add_space(2.0);
            hint(ui, d);
        }
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.y = 12.0;
        add(ui)
    })
    .inner
}

/// `Field`: a 12 px label above the control, an optional hint below.
pub fn field<R>(ui: &mut Ui, label: &str, hint_text: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = tokens(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(label).font(font(size::SM)).color(t.fg2));
        let r = add(ui);
        if let Some(h) = hint_text {
            hint(ui, h);
        }
        r
    })
    .inner
}

/// Options of [`input`].
#[derive(Clone, Copy, Default)]
pub struct InputOpts<'a> {
    pub hint: &'a str,
    /// Leading muted icon.
    pub icon: Option<Icon>,
    /// Terminal font (`font-mono`).
    pub mono: bool,
    pub password: bool,
    /// `None` = the available width.
    pub width: Option<f32>,
    pub enabled: bool,
    /// Danger border and ring (a validation problem).
    pub error: bool,
    /// 28 px (toolbars, panes) instead of 32.
    pub small: bool,
}

impl<'a> InputOpts<'a> {
    pub fn new(hint: &'a str) -> Self {
        Self { hint, enabled: true, ..Self::default() }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
}

/// One height for every single-line field, so mono and proportional
/// inputs line up in a row (and with the 32 px buttons).
const INPUT_H: f32 = CONTROL_H;

fn input_font(mono: bool) -> egui::FontId {
    if mono { egui::FontId::monospace(size::BASE) } else { font(size::BASE) }
}

/// A text input: `surface2` field, strong border, accent ring when focused,
/// optional leading icon and mono font.
pub fn input(ui: &mut Ui, value: &mut String, o: InputOpts<'_>) -> Response {
    let t = tokens(ui.ctx());
    let width = o.width.unwrap_or_else(|| ui.available_width());
    let left = if o.icon.is_some() { 32 } else { 10 };
    let (h, pad_y) = if o.small { (CONTROL_H_SM, 5) } else { (INPUT_H, 7) };
    let edit = egui::TextEdit::singleline(value)
        .hint_text(RichText::new(o.hint).color(alpha(t.muted, 0.8)))
        .desired_width(width)
        .font(input_font(o.mono))
        .margin(Margin { left, right: 10, top: pad_y, bottom: pad_y })
        .password(o.password)
        .min_size(vec2(0.0, h))
        .background_color(t.surface2);
    let resp = ui.add_enabled(o.enabled, edit);
    if o.password {
        forget_history(ui, &resp);
    }
    paint_input_frame(ui, &resp, o.icon, o.error);
    resp
}

fn paint_input_frame(ui: &Ui, resp: &Response, icon: Option<Icon>, error: bool) {
    let t = tokens(ui.ctx());
    widgets::input_frame(ui, resp.rect, resp.has_focus(), error);
    if let Some(icon) = icon {
        let r = Rect::from_center_size(Pos2::new(resp.rect.left() + 17.0, resp.rect.center().y), vec2(16.0, 16.0));
        icon.paint(ui, r, 15.0, t.muted);
    }
}

/// Give a control the AccessKit label of its visible `field` caption
/// (the caption is a separate node, so screen readers and tests would not
/// otherwise connect them).
pub fn labelled(ui: &Ui, resp: &Response, label: &str) {
    ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(label));
}

/// egui keeps an undo history of every text field (plaintext snapshots in
/// its memory, password fields included). Secret fields call this after
/// `ui.add` so no snapshot outlives the frame it was taken in. The dropped
/// strings are freed, not zeroized: egui's `TextBuffer` is a plain `String`.
pub fn forget_history(ui: &Ui, resp: &Response) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        state.clear_undoer();
        state.store(ui.ctx(), resp.id);
    }
}

/// A password field with an eye toggle.
/// `shown` is the caller's reveal state.
pub fn secret(ui: &mut Ui, value: &mut String, hint_text: &str, shown: &mut bool, width: Option<f32>) -> Response {
    let t = tokens(ui.ctx());
    let width = width.unwrap_or_else(|| ui.available_width());
    let edit = egui::TextEdit::singleline(value)
        .hint_text(RichText::new(hint_text).color(alpha(t.muted, 0.8)))
        .desired_width(width)
        .font(font(size::BASE))
        .margin(Margin { left: 10, right: 40, top: 7, bottom: 7 })
        .password(!*shown)
        .min_size(vec2(0.0, INPUT_H))
        .background_color(t.surface2);
    let resp = ui.add(edit);
    forget_history(ui, &resp);
    paint_input_frame(ui, &resp, None, false);
    let eye = Rect::from_center_size(Pos2::new(resp.rect.right() - 17.0, resp.rect.center().y), vec2(26.0, 26.0));
    let label = if *shown { "Hide secret" } else { "Show secret" };
    let b = ui.interact(eye, resp.id.with("eye"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    widgets::paint_icon_button(ui, &b, eye, if *shown { Icon::EyeOff } else { Icon::Eye }, 14.0, false, None);
    widgets::a11y(&b, egui::Role::Button, label);
    if b.on_hover_text(if *shown { "Hide" } else { "Show" }).clicked() {
        *shown = !*shown;
    }
    resp
}

/// A multi-line input for secrets (private keys): no undo history.
pub fn textarea(ui: &mut Ui, value: &mut String, hint_text: &str, rows: usize, mono: bool) -> Response {
    let t = tokens(ui.ctx());
    let edit = egui::TextEdit::multiline(value)
        .hint_text(RichText::new(hint_text).color(alpha(t.muted, 0.8)))
        .desired_width(ui.available_width())
        .desired_rows(rows)
        .font(if mono { egui::FontId::monospace(size::SM) } else { font(size::BASE) })
        .margin(Margin::symmetric(10, 8))
        .background_color(t.surface2);
    let resp = ui.add(edit);
    forget_history(ui, &resp);
    paint_input_frame(ui, &resp, None, false);
    resp
}

/// What a [`number`] field did this frame.
pub struct NumberOut {
    pub resp: Response,
    /// The stepper was clicked: +1 / −1 (Shift: ±10).
    pub step: i64,
}

/// A number field: a right-aligned text input with a stacked ▲ ▼ stepper.
/// The caller owns the draft string (committed on Enter / blur) and
/// applies `step` to its value.
pub fn number(ui: &mut Ui, draft: &mut String, label: &str, width: f32) -> NumberOut {
    let t = tokens(ui.ctx());
    let edit = egui::TextEdit::singleline(draft)
        .desired_width(width)
        .font(font(size::BASE))
        .margin(Margin { left: 10, right: 28, top: 7, bottom: 7 })
        .min_size(vec2(0.0, INPUT_H))
        .background_color(t.surface2);
    let resp = ui.add(edit);
    labelled(ui, &resp, label);
    paint_input_frame(ui, &resp, None, false);
    let col = Rect::from_min_max(Pos2::new(resp.rect.right() - 26.0, resp.rect.top() + 3.0), Pos2::new(resp.rect.right() - 4.0, resp.rect.bottom() - 3.0));
    let (up, down) = col.split_top_bottom_at_fraction(0.5);
    let shift = ui.input(|i| i.modifiers.shift);
    let mut step = 0;
    for (r, icon, dir, name) in [(up, Icon::ChevronUp, 1, "Increase"), (down, Icon::ChevronDown, -1, "Decrease")] {
        let b = ui.interact(r, resp.id.with(name), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if b.hovered() {
            ui.painter().rect_filled(r, radius::SM - 1.0, t.surface3);
        }
        icon.paint(ui, r, 12.0, if b.hovered() { t.fg } else { t.muted });
        widgets::a11y(&b, egui::Role::Button, &format!("{name} {label}"));
        if b.clicked() {
            step += dir * if shift { 10 } else { 1 };
        }
    }
    NumberOut { resp, step }
}

/// A `<select>` styled as an input. Returns true when the choice changed.
pub fn select<T: PartialEq + Clone>(ui: &mut Ui, id: Id, value: &mut T, options: &[(T, String)], width: Option<f32>) -> bool {
    let t = tokens(ui.ctx());
    let width = width.unwrap_or_else(|| ui.available_width());
    let current = options.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_default();
    let mut changed = false;
    let before = value.clone();
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.widgets.inactive.weak_bg_fill = t.surface2;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border_strong);
        v.widgets.hovered.weak_bg_fill = t.surface3;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.border_strong);
        v.widgets.open.weak_bg_fill = t.surface2;
        v.widgets.open.bg_stroke = Stroke::new(1.0, t.accent);
        ui.spacing_mut().button_padding = vec2(10.0, 8.0);
        egui::ComboBox::from_id_salt(id)
            .width(width)
            .selected_text(RichText::new(current).font(font(size::BASE)).color(t.fg))
            .show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (v, label) in options {
                    ui.selectable_value(value, v.clone(), RichText::new(label).font(font(size::BASE)));
                }
            });
    });
    if *value != before {
        changed = true;
    }
    changed
}

/// A checkbox with a label and an optional muted hint line.
pub fn checkbox(ui: &mut Ui, value: &mut bool, label: &str, hint_text: Option<&str>) -> Response {
    let t = tokens(ui.ctx());
    let resp = ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.corner_radius = CornerRadius::same(radius::SM as u8);
        ui.visuals_mut().widgets.hovered.corner_radius = CornerRadius::same(radius::SM as u8);
        ui.visuals_mut().widgets.active.corner_radius = CornerRadius::same(radius::SM as u8);
        ui.checkbox(value, RichText::new(label).font(font(size::BASE)).color(t.fg))
    })
    .inner;
    if let Some(h) = hint_text {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
            hint(ui, h);
        });
    }
    resp
}

/// One radio card: `(value, icon, title, hint)`.
pub type OptionCard<'a, T> = (T, Icon, &'a str, &'a str);

/// A row of option cards (the session dialog's type and auth pickers):
/// equal widths with 8 px gaps, every card as tall as the tallest, so a
/// hint that wraps does not make one card stick out. Returns the value of
/// the card clicked.
pub fn option_cards<T: PartialEq + Clone>(ui: &mut Ui, selected: &T, cards: &[OptionCard<'_, T>]) -> Option<T> {
    if cards.is_empty() {
        return None;
    }
    let n = cards.len() as f32;
    let width = (ui.available_width() - 8.0 * (n - 1.0)) / n;
    let height = cards.iter().map(|(_, _, title, hint)| option_card_height(ui, title, hint, width)).fold(0.0, f32::max);
    let mut clicked = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (value, icon, title, hint) in cards {
            if option_card(ui, value == selected, *icon, title, hint, width, height).clicked() {
                clicked = Some(value.clone());
            }
        }
    });
    clicked
}

fn option_card_galleys(ui: &Ui, title: &str, hint_text: &str, width: f32) -> (std::sync::Arc<egui::Galley>, std::sync::Arc<egui::Galley>) {
    let t = tokens(ui.ctx());
    let inner_w = width - 24.0;
    let title_g = ui.painter().layout(title.to_string(), font_semibold(size::BASE), t.fg, inner_w);
    let hint_g = ui.painter().layout(hint_text.to_string(), font(size::SM), t.muted, inner_w);
    (title_g, hint_g)
}

fn option_card_height(ui: &Ui, title: &str, hint_text: &str, width: f32) -> f32 {
    let (title_g, hint_g) = option_card_galleys(ui, title, hint_text, width);
    12.0 + 18.0 + 8.0 + title_g.size().y + 2.0 + hint_g.size().y + 12.0
}

/// One card of [`option_cards`]: icon, title, hint; `accent_soft` fill and
/// accent border when selected; at least `min_h` tall.
fn option_card(ui: &mut Ui, selected: bool, icon: Icon, title: &str, hint_text: &str, width: f32, min_h: f32) -> Response {
    let t = tokens(ui.ctx());
    let (title_g, hint_g) = option_card_galleys(ui, title, hint_text, width);
    let h = (12.0 + 18.0 + 8.0 + title_g.size().y + 2.0 + hint_g.size().y + 12.0).max(min_h);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let (border, fill) = if selected {
            (t.accent, t.accent_soft)
        } else if hovered {
            (t.border_strong, t.surface2)
        } else {
            (t.border_strong, Color32::TRANSPARENT)
        };
        ui.painter().rect_filled(rect, radius::LG, fill);
        ui.painter().rect_stroke(rect, radius::LG, Stroke::new(1.0, border), StrokeKind::Inside);
        let x = rect.left() + 12.0;
        let mut y = rect.top() + 12.0;
        let icon_color = if selected { t.accent } else if hovered { t.fg } else { t.muted };
        icon.paint(ui, Rect::from_min_size(Pos2::new(x, y), vec2(18.0, 18.0)), 18.0, icon_color);
        y += 18.0 + 8.0;
        let title_color = if selected || hovered { t.fg } else { t.fg2 };
        let th = title_g.size().y;
        ui.painter().galley_with_override_text_color(Pos2::new(x, y), title_g, title_color);
        y += th + 2.0;
        ui.painter().galley(Pos2::new(x, y), hint_g, t.muted);
        widgets::focus_ring(ui, &resp, rect);
    }
    a11y_selected(&resp, egui::Role::RadioButton, selected, title);
    resp
}

/// `<kbd>`: a small key cap.
pub fn kbd(ui: &mut Ui, text: &str) -> Response {
    let t = tokens(ui.ctx());
    Frame::new()
        .fill(t.surface2)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(radius::SM as u8))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(text).font(font(size::XS)).color(t.muted)))
        .response
}

/// Enter pressed in this single-line field (a form submit).
pub fn entered(ui: &Ui, resp: &Response) -> bool {
    resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
}

/// Ctrl+Enter pressed this frame (the dialogs' "save" chord).
pub fn ctrl_enter(ui: &Ui) -> bool {
    ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter))
}

/// Accent spinner of `size` px; keeps repainting.
pub fn spinner(ui: &mut Ui, size_px: f32) -> Response {
    let t = tokens(ui.ctx());
    ui.add(egui::Spinner::new().size(size_px).color(t.accent))
}

/// A success / warning / info strip (the error strip's siblings).
pub fn note(ui: &mut Ui, icon: Icon, color: Color32, text: &str) {
    Frame::new()
        .fill(alpha(color, 0.1))
        .stroke(Stroke::new(1.0, alpha(color, 0.35)))
        .corner_radius(CornerRadius::same(radius::MD as u8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icon.show(ui, 14.0, color);
                ui.add(egui::Label::new(RichText::new(text).font(font(size::SM)).color(color)).wrap());
            });
        });
}
