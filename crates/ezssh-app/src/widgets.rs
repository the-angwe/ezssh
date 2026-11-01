//! Small building blocks shared by the views, each the egui form of one
//! class or component of the webview: `.btn-primary` / `.btn-ghost` /
//! `.icon-btn`, the dialog shell, `context-menu.tsx`, `resize-handle.tsx`,
//! `status-dot.tsx`, `empty-state.tsx`.

use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Order, Pos2, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use crate::icons::Icon;
use crate::style::{self, alpha, font, font_semibold, size, tokens};
use crate::tabs::TabStatus;

// ---------------------------------------------------------------- accessibility

/// Describe a hand-painted control to AccessKit (screen readers, and the
/// headless tests that find widgets by label).
pub fn a11y(resp: &Response, role: egui::Role, label: &str) {
    let enabled = resp.enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(role, enabled, label));
}

/// [`a11y`] for something with a selected / pressed state (tabs, rows,
/// toggles).
pub fn a11y_selected(resp: &Response, role: egui::Role, selected: bool, label: &str) {
    let enabled = resp.enabled();
    resp.widget_info(|| egui::WidgetInfo::selected(role, enabled, selected, label));
}

// ---------------------------------------------------------------- buttons

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Danger,
}

/// `.btn-primary` (accent or danger fill, white text). `text_size` is 11
/// for in-card buttons (`text-xs`), 12 for dialog footers (`text-sm`).
pub fn primary(ui: &mut Ui, text: &str, tone: Tone, enabled: bool, text_size: f32) -> Response {
    primary_with_icon(ui, None, text, tone, enabled, text_size)
}

pub fn primary_with_icon(ui: &mut Ui, icon: Option<Icon>, text: &str, tone: Tone, enabled: bool, text_size: f32) -> Response {
    primary_sized(ui, icon, text, tone, enabled, text_size, None)
}

/// [`primary`] at a fixed width (the unlock card's `w-full` button).
pub fn primary_full(ui: &mut Ui, text: &str, enabled: bool, width: f32) -> Response {
    primary_sized(ui, None, text, Tone::Accent, enabled, size::SM, Some(width))
}

fn primary_sized(ui: &mut Ui, icon: Option<Icon>, text: &str, tone: Tone, enabled: bool, text_size: f32, width: Option<f32>) -> Response {
    let t = tokens(ui.ctx());
    let fill = match tone {
        Tone::Accent => t.accent,
        Tone::Danger => t.danger,
    };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_semibold(text_size), Color32::WHITE);
    let icon_w = if icon.is_some() { 12.0 + 6.0 } else { 0.0 };
    let natural = galley.size().x + icon_w + 24.0;
    let size = vec2(width.unwrap_or(natural), (galley.size().y + 12.0).max(26.0));
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let resp = resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed });
    if ui.is_rect_visible(rect) {
        let mut c = fill;
        if enabled && resp.is_pointer_button_down_on() {
            c = style::over(c, Color32::BLACK, 0.1);
        } else if enabled && resp.hovered() {
            c = style::over(c, Color32::WHITE, 0.1);
        }
        if !enabled {
            c = alpha(c, 0.5);
        }
        ui.painter().rect_filled(rect, 2.0, c);
        let mut x = rect.center().x - natural / 2.0 + 12.0;
        if let Some(icon) = icon {
            icon.paint(ui, Rect::from_center_size(Pos2::new(x + 6.0, rect.center().y), Vec2::splat(12.0)), 12.0, Color32::WHITE);
            x += icon_w;
        }
        let text_color = if enabled { Color32::WHITE } else { alpha(Color32::WHITE, 0.7) };
        ui.painter().galley_with_override_text_color(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, text_color);
        focus_ring(ui, &resp, rect);
    }
    // Outside the visibility check (a modal's sizing pass paints nothing),
    // and with the real enabled state: a disabled button senses hover only,
    // which AccessKit would otherwise report as enabled.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    resp
}

/// The bordered secondary button of dialog footers ("Cancel").
pub fn ghost(ui: &mut Ui, text: &str, text_size: f32) -> Response {
    let t = tokens(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font(text_size), t.fg);
    let size = vec2(galley.size().x + 24.0, (galley.size().y + 12.0).max(26.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, t.surface2);
        }
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.fg);
        focus_ring(ui, &resp, rect);
    }
    a11y(&resp, egui::Role::Button, text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// `.icon-btn`: a square ghost button with one icon. `pressed` paints the
/// `aria-pressed` state (accent icon on accent/15).
pub fn icon_button(ui: &mut Ui, icon: Icon, box_size: f32, icon_size: f32, pressed: bool, tooltip: &str) -> Response {
    icon_button_tinted(ui, icon, box_size, icon_size, pressed, tooltip, None)
}

pub fn icon_button_tinted(
    ui: &mut Ui,
    icon: Icon,
    box_size: f32,
    icon_size: f32,
    pressed: bool,
    tooltip: &str,
    hover_color: Option<Color32>,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    paint_icon_button(ui, &resp, rect, icon, icon_size, pressed, hover_color);
    if pressed {
        a11y_selected(&resp, egui::Role::Button, true, tooltip);
    } else {
        a11y(&resp, egui::Role::Button, tooltip);
    }
    let resp = resp.on_hover_text(tooltip);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn paint_icon_button(ui: &Ui, resp: &Response, rect: Rect, icon: Icon, icon_size: f32, pressed: bool, hover_color: Option<Color32>) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let t = tokens(ui.ctx());
    let hovered = resp.hovered();
    let color = if pressed {
        t.accent
    } else if hovered {
        hover_color.unwrap_or(t.fg)
    } else {
        t.muted
    };
    if pressed {
        ui.painter().rect_filled(rect, 2.0, alpha(t.accent, 0.15));
    } else if hovered {
        ui.painter().rect_filled(rect, 2.0, t.surface2);
    }
    icon.paint(ui, rect, icon_size, color);
    focus_ring(ui, resp, rect);
}

/// The 2 px accent outline of `.focus-ring:focus-visible`. egui buttons
/// take focus only from Tab or `request_focus`, never from a click, so
/// "has focus" is the keyboard case.
pub fn focus_ring(ui: &Ui, resp: &Response, rect: Rect) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 3.0, Stroke::new(2.0, tokens(ui.ctx()).accent), StrokeKind::Outside);
    }
}

// ---------------------------------------------------------------- status

/// `status-dot.tsx`: a 6 px dot by status; errors show the alert icon.
pub fn status_dot(ui: &Ui, center: Pos2, status: TabStatus, icon_size: f32) {
    let t = tokens(ui.ctx());
    let color = match status {
        TabStatus::Error => {
            Icon::AlertCircle.paint(ui, Rect::from_center_size(center, Vec2::splat(icon_size)), icon_size, t.danger);
            return;
        }
        TabStatus::Connected => t.success,
        TabStatus::Connecting => alpha(t.warning, pulse(ui.ctx())),
        TabStatus::Closed => alpha(t.muted, 0.6),
    };
    ui.painter().circle_filled(center, 3.0, color);
}

/// Tailwind's `animate-pulse` opacity (1 → 0.5 → 1 over 2 s). Keeps the
/// frame clock running while anything pulses.
pub fn pulse(ctx: &egui::Context) -> f32 {
    ctx.request_repaint_after(Duration::from_millis(50));
    let t = ctx.input(|i| i.time) as f32;
    0.75 + 0.25 * (t * std::f32::consts::PI).cos()
}

// ---------------------------------------------------------------- empty state

/// `empty-state.tsx`. Returns the action button's response, if any.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, body: &str, action: Option<(&str, bool)>, compact: bool) -> Option<Response> {
    let t = tokens(ui.ctx());
    let mut out = None;
    ui.vertical_centered(|ui| {
        let icon_size = if compact { 24.0 } else { 32.0 };
        let content_h = icon_size + 8.0 + 18.0 + if body.is_empty() { 0.0 } else { 34.0 } + if action.is_some() { 40.0 } else { 0.0 };
        let pad = if compact { 24.0 } else { ((ui.available_height() - content_h) / 2.0).max(40.0) };
        ui.add_space(pad);
        icon.show(ui, icon_size, t.muted);
        ui.add_space(8.0);
        ui.label(RichText::new(title).font(font_semibold(size::SM)).color(t.fg));
        if !body.is_empty() {
            ui.add_space(4.0);
            ui.allocate_ui(vec2(260.0, 40.0), |ui| {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.label(RichText::new(body).font(font(size::XS)).color(t.muted));
                });
            });
        }
        if let Some((label, enabled)) = action {
            ui.add_space(12.0);
            out = Some(primary(ui, label, Tone::Accent, enabled, size::XS));
        }
    });
    out
}

// ---------------------------------------------------------------- dialogs

/// Outcome of one frame of a dialog.
pub struct DialogFrame {
    /// Backdrop click or Escape.
    pub dismissed: bool,
}

/// The dialog shell of the webview: a `width` px card (surface, border,
/// 3 px radius, dialog shadow) over a dimmed backdrop. `body` is padded
/// 16 px; `footer` (if any) sits in a bordered strip with right-aligned
/// buttons, laid out right to left (add the primary button first).
pub fn dialog(ctx: &egui::Context, id: Id, width: f32, body: impl FnOnce(&mut Ui), footer: Option<&mut dyn FnMut(&mut Ui)>) -> DialogFrame {
    let t = tokens(ctx);
    let frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(3))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(115) });
    let width = width.min(ctx.content_rect().width() - 32.0);
    let resp = egui::Modal::new(id).backdrop_color(Color32::from_black_alpha(128)).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(width - 32.0);
            body(ui)
        });
        if let Some(footer) = footer {
            let r = ui.available_rect_before_wrap();
            let strip = Rect::from_min_size(r.min, vec2(width, 52.0));
            ui.painter().rect_filled(strip, CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, style::over(t.surface, t.surface2, 0.3));
            ui.painter().hline(strip.x_range(), strip.top(), Stroke::new(1.0, t.border));
            ui.scope_builder(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(16.0, 12.0))), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    footer(ui);
                });
            });
            ui.allocate_rect(strip, Sense::hover());
        }
    });
    DialogFrame { dismissed: resp.should_close() }
}

/// Dialog header row: an icon (22 px, coloured) beside a title and body.
pub fn dialog_heading(ui: &mut Ui, icon: Icon, icon_color: Color32, icon_size: f32, title: &str, body: &str) {
    let t = tokens(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        icon.show(ui, icon_size, icon_color);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).font(font_semibold(size::SM)).color(t.fg));
            if !body.is_empty() {
                ui.add_space(2.0);
                ui.label(RichText::new(body).font(font(size::XS)).color(t.muted));
            }
        });
    });
}

/// The webview's `.input`: surface-2 field with a border and accent focus.
pub fn text_input(ui: &mut Ui, value: &mut String, hint: &str, width: f32, password: bool) -> Response {
    let t = tokens(ui.ctx());
    let edit = egui::TextEdit::singleline(value)
        .hint_text(RichText::new(hint).color(t.muted))
        .desired_width(width)
        .font(font(size::BASE))
        .margin(Margin::symmetric(8, 6))
        .password(password)
        .background_color(t.surface2);
    let resp = ui.add(edit);
    if password {
        crate::form::forget_history(ui, &resp);
    }
    let stroke = if resp.has_focus() { Stroke::new(1.0, t.accent) } else { Stroke::new(1.0, t.border) };
    ui.painter().rect_stroke(resp.rect, 2.0, stroke, StrokeKind::Inside);
    resp
}

/// Red alert strip (`role="alert"` in the unlock / prompt dialogs).
pub fn error_strip(ui: &mut Ui, text: &str) {
    let t = tokens(ui.ctx());
    Frame::new()
        .fill(alpha(t.danger, 0.1))
        .stroke(Stroke::new(1.0, alpha(t.danger, 0.5)))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                Icon::AlertCircle.show(ui, 13.0, t.danger);
                ui.label(RichText::new(text).font(font(size::XS)).color(t.danger));
            });
        });
}

// ---------------------------------------------------------------- menu

/// One row of a [`Menu`].
pub enum MenuEntry<A> {
    Item { label: String, action: A, enabled: bool, danger: bool, hint: Option<&'static str> },
    Separator,
}

impl<A> MenuEntry<A> {
    pub fn item(label: impl Into<String>, action: A) -> Self {
        Self::Item { label: label.into(), action, enabled: true, danger: false, hint: None }
    }

    pub fn danger(label: impl Into<String>, action: A) -> Self {
        Self::Item { label: label.into(), action, enabled: true, danger: true, hint: None }
    }

    pub fn enabled(self, on: bool) -> Self {
        match self {
            Self::Item { label, action, danger, hint, .. } => Self::Item { label, action, enabled: on, danger, hint },
            s => s,
        }
    }
}

/// `context-menu.tsx`: a popup at the pointer that closes on a choice,
/// Escape or a press outside it.
pub struct Menu<A> {
    pub pos: Pos2,
    pub entries: Vec<MenuEntry<A>>,
    just_opened: bool,
}

pub enum MenuResult<A> {
    Open,
    Closed,
    Chosen(A),
}

impl<A: Clone> Menu<A> {
    pub fn new(pos: Pos2, entries: Vec<MenuEntry<A>>) -> Self {
        Self { pos, entries, just_opened: true }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: Id) -> MenuResult<A> {
        let t = tokens(ctx);
        let mut chosen = None;
        let frame = Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::symmetric(0, 4))
            .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) });
        // An Area offers the whole screen as available width; size the menu
        // from its widest label instead (min 200 px, as `min-w-[200px]`).
        let width = ctx.fonts_mut(|f| {
            self.entries
                .iter()
                .filter_map(|e| match e {
                    MenuEntry::Item { label, .. } => Some(f.layout_no_wrap(label.clone(), font(size::XS), t.fg).size().x + 24.0),
                    MenuEntry::Separator => None,
                })
                .fold(200.0_f32, f32::max)
        });
        let area = egui::Area::new(id).order(Order::Foreground).fixed_pos(self.pos).constrain(true).show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                for entry in &self.entries {
                    match entry {
                        MenuEntry::Separator => {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                            ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, alpha(t.border, 0.7)));
                        }
                        MenuEntry::Item { label, action, enabled, danger, hint } => {
                            let galley = ui.painter().layout_no_wrap(label.clone(), font(size::XS), t.fg);
                            let (r, resp) = ui.add_enabled_ui(*enabled, |ui| ui.allocate_exact_size(vec2(width, 24.0), Sense::click())).inner;
                            let mut color = if *danger { t.danger } else { t.fg };
                            if !enabled {
                                color = alpha(color, 0.4);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(r, 0.0, t.surface2);
                            }
                            ui.painter().galley_with_override_text_color(
                                Pos2::new(r.left() + 12.0, r.center().y - galley.size().y / 2.0),
                                galley,
                                color,
                            );
                            a11y(&resp, egui::Role::MenuItem, label);
                            let resp = match hint {
                                Some(h) => resp.on_disabled_hover_text(*h),
                                None => resp,
                            };
                            if *enabled && resp.clicked() {
                                chosen = Some(action.clone());
                            }
                        }
                    }
                }
            });
        });
        if let Some(a) = chosen {
            return MenuResult::Chosen(a);
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            return MenuResult::Closed;
        }
        if !std::mem::take(&mut self.just_opened) {
            let outside = ctx.input(|i| {
                i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p))
            });
            if outside {
                return MenuResult::Closed;
            }
        }
        MenuResult::Open
    }
}

// ---------------------------------------------------------------- resize strip

const KEY_STEP: f32 = 16.0;
const KEY_STEP_LARGE: f32 = 64.0;

/// What a [`resize_strip`] did this frame.
#[derive(Default)]
pub struct ResizeOut {
    /// Width changed (persist only on `ended`).
    pub changed: bool,
    /// A gesture finished (drag released, key step, double-click reset).
    pub ended: bool,
}

/// Width bounds of a resizable pane; `max` is the effective bound for
/// this frame (it shrinks to keep room for the terminal).
pub struct Bounds {
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

/// `resize-handle.tsx`: a 4 px strip on the right edge of a pane. Drag
/// resizes, double-click restores the default, and when focused (click
/// it, or Tab) ←/→ step 16 px (Shift: 64), Home restores the default and
/// End goes to the maximum.
pub fn resize_strip(ui: &mut Ui, id: Id, pane: Rect, width: &mut f32, bounds: Bounds, label: &str) -> ResizeOut {
    let Bounds { min, max, default } = bounds;
    let t = tokens(ui.ctx());
    let strip = Rect::from_min_max(Pos2::new(pane.right() - 4.0, pane.top()), pane.right_bottom());
    let resp = ui.interact(strip, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
    a11y(&resp, egui::Role::Splitter, label);
    let resp = resp.on_hover_text(format!("{label} · Drag to resize · Double-click to reset"));
    let mut out = ResizeOut::default();
    let max = max.max(min);
    let start_id = id.with("start");
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(start_id, *width));
        resp.request_focus();
    }
    if resp.dragged()
        && let Some(start) = ui.ctx().data(|d| d.get_temp::<f32>(start_id))
    {
        let dx = resp.total_drag_delta().map_or(0.0, |d| d.x);
        let w = (start + dx).clamp(min, max).round();
        if w != *width {
            *width = w;
            out.changed = true;
        }
    }
    if resp.drag_stopped() {
        out.ended = true;
    }
    if resp.double_clicked() {
        *width = default.clamp(min, max);
        out.changed = true;
        out.ended = true;
    }
    if resp.clicked() {
        resp.request_focus();
    }
    if resp.has_focus() {
        let (step, home, end) = ui.ctx().input(|i| {
            let step = if i.modifiers.shift { KEY_STEP_LARGE } else { KEY_STEP };
            let mut d = 0.0;
            if i.key_pressed(Key::ArrowLeft) {
                d -= step;
            }
            if i.key_pressed(Key::ArrowRight) {
                d += step;
            }
            (d, i.key_pressed(Key::Home), i.key_pressed(Key::End))
        });
        let next = if home {
            Some(default)
        } else if end {
            Some(max)
        } else if step != 0.0 {
            Some(*width + step)
        } else {
            None
        };
        if let Some(n) = next {
            *width = n.clamp(min, max);
            out.changed = true;
            out.ended = true;
        }
    }
    let active = resp.dragged();
    if active || resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(strip, 0.0, alpha(t.accent, if active { 0.6 } else { 0.4 }));
        ui.painter().rect_filled(
            Rect::from_min_max(Pos2::new(strip.right() - 2.0, strip.top()), strip.right_bottom()),
            0.0,
            if active { t.accent } else { alpha(t.accent, 0.6) },
        );
    }
    out
}

// ---------------------------------------------------------------- misc

/// Truncate `text` with an ellipsis to fit `max_width` at `font_id`.
pub fn elide(ui: &Ui, text: &str, font_id: egui::FontId, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font_id, tokens(ui.ctx()).fg);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.painter().layout_job(job)
}

/// Paint `text` left-aligned and vertically centred in `rect`, elided.
pub fn paint_text(ui: &Ui, rect: Rect, text: &str, font_id: egui::FontId, color: Color32) -> Rect {
    let g = elide(ui, text, font_id, rect.width());
    let pos = Align2::LEFT_CENTER.align_size_within_rect(g.size(), rect).min;
    let r = Rect::from_min_size(pos, g.size());
    ui.painter().galley_with_override_text_color(pos, g, color);
    r
}
