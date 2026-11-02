//! Small building blocks shared by the views, each the egui form of one
//! class or component of the webview: `.btn-primary` / `.btn-ghost` /
//! `.icon-btn`, the dialog shell, `context-menu.tsx`, `resize-handle.tsx`,
//! `status-dot.tsx`, `empty-state.tsx`.

use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Modifiers, Order, Pos2, Rect, Response, RichText,
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
    Ghost::new(text).size(text_size).show(ui)
}

/// `.btn-ghost`: a bordered, transparent button with a hover fill, an
/// optional leading icon and a disabled state. Every bordered secondary
/// button of the app is one of these.
pub struct Ghost<'a> {
    text: Option<&'a str>,
    icon: Option<Icon>,
    text_size: f32,
    pad: Vec2,
    min_h: f32,
    icon_gap: f32,
    enabled: bool,
    /// Text, icon and border colour (the danger-outlined "Disable").
    color: Option<Color32>,
    quiet: bool,
    label: Option<&'a str>,
    tooltip: Option<&'a str>,
}

impl<'a> Ghost<'a> {
    /// Footer size (`text-sm`, `px-3 py-1.5`).
    pub fn new(text: &'a str) -> Self {
        Self {
            text: Some(text),
            icon: None,
            text_size: size::SM,
            pad: vec2(12.0, 6.0),
            min_h: 26.0,
            icon_gap: 6.0,
            enabled: true,
            color: None,
            quiet: false,
            label: None,
            tooltip: None,
        }
    }

    /// A square-ish button with only an icon; `label` names it.
    pub fn icon_only(icon: Icon, label: &'a str) -> Self {
        Self { text: None, icon: Some(icon), label: Some(label), ..Self::new("") }
    }

    pub fn icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn size(mut self, text_size: f32) -> Self {
        self.text_size = text_size;
        self
    }

    /// In-card size (`px-2 py-1 text-xs`).
    pub fn compact(mut self) -> Self {
        self.text_size = size::XS;
        self.pad = vec2(8.0, 4.0);
        self.min_h = 22.0;
        self.icon_gap = 4.0;
        self
    }

    /// Toolbar style of the credential picker: 28 px tall, the icon muted
    /// until hovered, a stronger border on hover.
    pub fn quiet(mut self) -> Self {
        self.text_size = size::XS;
        self.pad = vec2(10.0, 6.0);
        self.min_h = 28.0;
        self.quiet = true;
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn color(mut self, c: impl Into<Option<Color32>>) -> Self {
        self.color = c.into();
        self
    }

    /// Screen-reader name when the visible text is ambiguous ("Add").
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn tooltip(mut self, tip: impl Into<Option<&'a str>>) -> Self {
        self.tooltip = tip.into();
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = tokens(ui.ctx());
        let fg = self.color.unwrap_or(t.fg);
        let galley = self.text.map(|s| ui.painter().layout_no_wrap(s.to_string(), font(self.text_size), fg));
        let text_w = galley.as_ref().map_or(0.0, |g| g.size().x);
        let text_h = galley.as_ref().map_or(0.0, |g| g.size().y);
        let icon_w = if self.icon.is_some() { 12.0 } else { 0.0 };
        let gap = if self.icon.is_some() && galley.is_some() { self.icon_gap } else { 0.0 };
        let content_w = icon_w + gap + text_w;
        // An icon-only button is 30 px wide (`w-[30px]`) whatever the padding.
        let pad_x = if galley.is_some() { self.pad.x } else { 9.0 };
        let size = vec2(content_w + 2.0 * pad_x, (text_h + 2.0 * self.pad.y).max(self.min_h));
        let enabled = self.enabled;
        let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
        if ui.is_rect_visible(rect) {
            let fade = |c: Color32| if enabled { c } else { alpha(c, 0.5) };
            let hovered = enabled && resp.hovered();
            if hovered {
                ui.painter().rect_filled(rect, 2.0, if self.color.is_some() { alpha(fg, 0.1) } else { t.surface2 });
            }
            let border = match self.color {
                Some(_) => alpha(fg, 0.4),
                None if self.quiet && hovered => t.border_strong,
                None => t.border,
            };
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, fade(border)), StrokeKind::Inside);
            let mut x = rect.center().x - content_w / 2.0;
            if let Some(icon) = self.icon {
                let c = if self.quiet && !hovered { t.muted } else { fg };
                icon.paint(ui, Rect::from_center_size(Pos2::new(x + 6.0, rect.center().y), Vec2::splat(12.0)), 12.0, fade(c));
                x += icon_w + gap;
            }
            if let Some(g) = galley {
                ui.painter().galley_with_override_text_color(Pos2::new(x, rect.center().y - text_h / 2.0), g, fade(fg));
            }
            focus_ring(ui, &resp, rect);
        }
        // Outside the visibility check (a modal's sizing pass paints
        // nothing), with the real enabled state.
        let label = self.label.or(self.text).unwrap_or_default();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
        let resp = resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed });
        match self.tooltip {
            Some(tip) => resp.on_hover_text(tip),
            None => resp,
        }
    }
}

/// A right-aligned row of buttons, one button tall (add the rightmost
/// first). Fixed height: a centred right-to-left layout would otherwise
/// claim every pixel below.
pub fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui);
    });
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

/// An icon button (the webview's `.icon-btn` / `IconBtn`) painted at a
/// given rect, with a screen-reader `label` apart from its `tooltip`.
pub struct IconBtn<'a> {
    icon: Icon,
    icon_size: f32,
    label: String,
    tooltip: &'a str,
    /// Icon colour at rest (muted when `None`).
    tone: Option<Color32>,
    hover_tone: Option<Color32>,
    hover_bg: Option<Color32>,
    enabled: bool,
    /// When false it is painted only while hovered or focused (the
    /// webview's `opacity-0 group-hover:opacity-100 focus:opacity-100`).
    visible: bool,
}

impl<'a> IconBtn<'a> {
    pub fn new(icon: Icon, label: String, tooltip: &'a str) -> Self {
        Self { icon, icon_size: 12.0, label, tooltip, tone: None, hover_tone: None, hover_bg: None, enabled: true, visible: true }
    }

    pub fn tone(mut self, c: Color32) -> Self {
        self.tone = Some(c);
        self
    }

    pub fn hover(mut self, tone: Color32, bg: Color32) -> Self {
        self.hover_tone = Some(tone);
        self.hover_bg = Some(bg);
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn visible(mut self, on: bool) -> Self {
        self.visible = on;
        self
    }

    pub fn show(&self, ui: &mut Ui, rect: Rect, id: Id) -> Response {
        let tk = tokens(ui.ctx());
        let enabled = self.enabled;
        let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &self.label));
        let hovered = enabled && resp.hovered();
        if (self.visible || resp.has_focus() || hovered) && ui.is_rect_visible(rect) {
            if hovered {
                ui.painter().rect_filled(rect, 2.0, self.hover_bg.unwrap_or(tk.surface2));
            }
            let mut color = if hovered { self.hover_tone.or(self.tone).unwrap_or(tk.fg) } else { self.tone.unwrap_or(tk.muted) };
            if !enabled {
                color = alpha(color, 0.4);
            }
            self.icon.paint(ui, rect, self.icon_size, color);
            focus_ring(ui, &resp, rect);
        }
        let resp = resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed });
        resp.on_hover_text(self.tooltip)
    }
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

/// The dialog's header.
pub enum DialogHead<'a> {
    None,
    /// The vault / forward dialogs' title strip (`bg-surface2/30`, 42 px):
    /// optional accent icon and a title.
    Strip { icon: Option<Icon>, title: &'a str },
    /// The data dialogs' header (56 px): icon, title, optional mono
    /// subtitle and a ✕ button.
    Full { icon: Icon, icon_color: Color32, title: &'a str, subtitle: Option<&'a str> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogFooter {
    None,
    /// Right-aligned under the body, inside the padding.
    Inline,
    /// A bordered strip under the body.
    Strip,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogPart {
    Body,
    /// Laid out right to left: add the primary button first. Leftover
    /// space on the left can hold more (`ui.with_layout(left_to_right)`).
    Footer,
}

/// What the user did to the dialog's surroundings this frame. Reported,
/// never acted on, so each dialog can gate closing its own way.
#[derive(Clone, Copy, Debug, Default)]
pub struct DialogOut {
    pub backdrop: bool,
    /// Escape, consumed only when this is the top modal and no popup
    /// (combo box list) is open.
    pub escape: bool,
    /// The header's ✕, or `ui.close()` from inside.
    pub close: bool,
}

impl DialogOut {
    pub fn dismissed(&self) -> bool {
        self.backdrop || self.escape || self.close
    }
}

const STRIP_HEAD_H: f32 = 42.0;
const FULL_HEAD_H: f32 = 56.0;
const FOOTER_H: f32 = 52.0;

/// The dialog card of the webview (`w-[…px]`, `max-h-[90vh]`): surface,
/// border, 3 px radius and dialog shadow over a dimmed backdrop, with a
/// header, a padded body that scrolls once it no longer fits, and a
/// footer.
pub struct Dialog<'a> {
    id: Id,
    width: f32,
    head: DialogHead<'a>,
    footer: DialogFooter,
    padding: i8,
    row_gap: Option<f32>,
    busy: bool,
}

impl<'a> Dialog<'a> {
    pub fn new(id: Id, width: f32) -> Self {
        Self { id, width, head: DialogHead::None, footer: DialogFooter::None, padding: 16, row_gap: None, busy: false }
    }

    /// The backup / restore / sync / import dialogs: 20 px padding, 16 px
    /// between rows (add a [`Dialog::header`]).
    pub fn data(id: Id, width: f32) -> Self {
        Self::new(id, width).padding(20).row_gap(16.0)
    }

    pub fn strip(mut self, icon: Option<Icon>, title: &'a str) -> Self {
        self.head = DialogHead::Strip { icon, title };
        self
    }

    pub fn header(mut self, icon: Icon, icon_color: Color32, title: &'a str, subtitle: Option<&'a str>) -> Self {
        self.head = DialogHead::Full { icon, icon_color, title, subtitle };
        self
    }

    pub fn footer(mut self, kind: DialogFooter) -> Self {
        self.footer = kind;
        self
    }

    pub fn padding(mut self, px: i8) -> Self {
        self.padding = px;
        self
    }

    /// Vertical space between body rows (egui's default when unset).
    pub fn row_gap(mut self, px: f32) -> Self {
        self.row_gap = Some(px);
        self
    }

    /// A call is running: the ✕ is inert.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Show with separate body and footer closures (a `Strip` footer when
    /// `footer` is given).
    pub fn show_split(mut self, ctx: &egui::Context, body: impl FnOnce(&mut Ui), mut footer: Option<&mut dyn FnMut(&mut Ui)>) -> DialogOut {
        if footer.is_some() && self.footer == DialogFooter::None {
            self.footer = DialogFooter::Strip;
        }
        let mut body = Some(body);
        self.show(ctx, |ui, part| match part {
            DialogPart::Body => {
                if let Some(b) = body.take() {
                    b(ui);
                }
            }
            DialogPart::Footer => {
                if let Some(f) = footer.as_mut() {
                    f(ui);
                }
            }
        })
    }

    /// Show one frame. `add` is called once for the body and once for the
    /// footer (unless `DialogFooter::None`), so both can borrow the
    /// dialog's state.
    pub fn show(self, ctx: &egui::Context, mut add: impl FnMut(&mut Ui, DialogPart)) -> DialogOut {
        let t = tokens(ctx);
        let screen = ctx.content_rect();
        let width = self.width.min(screen.width() - 32.0);
        let id = self.id;
        let frame = Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(3))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(115) });
        let strip_fill = style::over(t.surface, t.surface2, 0.3);
        let mut close = false;
        let resp = egui::Modal::new(id).backdrop_color(Color32::from_black_alpha(128)).frame(frame).show(ctx, |ui| {
            ui.set_width(width);
            let default_gap = ui.spacing().item_spacing.y;
            ui.spacing_mut().item_spacing.y = 0.0;
            let head_h = match self.head {
                DialogHead::None => 0.0,
                DialogHead::Strip { icon, title } => {
                    let (head, _) = ui.allocate_exact_size(vec2(width, STRIP_HEAD_H), Sense::hover());
                    ui.painter().rect_filled(head, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, strip_fill);
                    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, t.border));
                    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(16.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
                    row.spacing_mut().item_spacing.x = 8.0;
                    if let Some(icon) = icon {
                        icon.show(&mut row, 14.0, t.accent);
                    }
                    row.label(RichText::new(title).font(font_semibold(size::SM)).color(t.fg));
                    STRIP_HEAD_H
                }
                DialogHead::Full { icon, icon_color, title, subtitle } => {
                    let (hrect, _) = ui.allocate_exact_size(vec2(width, FULL_HEAD_H), Sense::hover());
                    ui.painter().hline(hrect.x_range(), hrect.bottom(), Stroke::new(1.0, t.border));
                    let inner = hrect.shrink2(vec2(20.0, 0.0));
                    let icon_r = Rect::from_center_size(Pos2::new(inner.left() + 8.0, inner.center().y), Vec2::splat(16.0));
                    icon.paint(ui, icon_r, 16.0, icon_color);
                    let text_r = Rect::from_min_max(Pos2::new(icon_r.right() + 12.0, inner.top()), Pos2::new(inner.right() - 36.0, inner.bottom()));
                    let title_text = RichText::new(title).font(font_semibold(size::BASE)).color(t.fg);
                    match subtitle {
                        Some(sub) => {
                            let top = Rect::from_min_max(text_r.min, Pos2::new(text_r.right(), text_r.center().y));
                            let bottom = Rect::from_min_max(Pos2::new(text_r.left(), text_r.center().y), text_r.max);
                            title_label(ui, top, title_text, Align::BOTTOM);
                            title_label(ui, bottom, RichText::new(sub).font(egui::FontId::monospace(size::XS)).color(t.muted), Align::TOP);
                        }
                        None => title_label(ui, text_r, title_text, Align::Center),
                    }
                    let x_r = Rect::from_center_size(Pos2::new(inner.right() - 12.0, inner.center().y), Vec2::splat(24.0));
                    let x = ui.interact(x_r, id.with("close"), if self.busy { Sense::hover() } else { Sense::click() });
                    paint_icon_button(ui, &x, x_r, Icon::X, 14.0, false, None);
                    a11y(&x, egui::Role::Button, "Close");
                    if x.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        close = true;
                    }
                    FULL_HEAD_H
                }
            };
            // Body. A Modal lays out in last frame's size and a ScrollArea
            // never grows past the space it is offered, so a body that fits
            // is laid out plainly (the card follows its height); only one
            // taller than `max_body` (judged on last frame's height)
            // scrolls, at that height.
            let foot_h = if self.footer == DialogFooter::Strip { FOOTER_H } else { 0.0 };
            let max_body = (screen.height() * 0.9 - head_h - foot_h).max(120.0);
            let content_h_id = id.with("content-h");
            let last_h: f32 = ui.ctx().data(|d| d.get_temp(content_h_id)).unwrap_or(0.0);
            let pad = f32::from(self.padding);
            let mut body = |ui: &mut Ui| {
                Frame::new()
                    .inner_margin(Margin::same(self.padding))
                    .show(ui, |ui| {
                        ui.set_width(width - 2.0 * pad);
                        ui.spacing_mut().item_spacing.y = self.row_gap.unwrap_or(default_gap);
                        add(ui, DialogPart::Body);
                        if self.footer == DialogFooter::Inline {
                            ui.add_space(-4.0);
                            ui.allocate_ui_with_layout(vec2(width - 2.0 * pad, 28.0), Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                add(ui, DialogPart::Footer);
                            });
                        }
                    })
                    .response
                    .rect
                    .height()
            };
            let content_h = if last_h <= max_body {
                body(ui)
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(id.with("body"))
                    .max_height(max_body)
                    .min_scrolled_height(max_body)
                    .auto_shrink(false)
                    .show(ui, |ui| body(ui))
                    .inner
            };
            if content_h != last_h {
                ui.ctx().data_mut(|d| d.insert_temp(content_h_id, content_h));
                ui.ctx().request_repaint();
            }
            if self.footer == DialogFooter::Strip {
                let (strip, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
                ui.painter().rect_filled(strip, CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, strip_fill);
                ui.painter().hline(strip.x_range(), strip.top(), Stroke::new(1.0, t.border));
                ui.scope_builder(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(pad, 12.0))), |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        add(ui, DialogPart::Footer);
                    });
                });
            }
        });
        let backdrop = resp.backdrop_response.clicked();
        let close = close || resp.response.should_close();
        let escape = resp.is_top_modal && !resp.any_popup_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        DialogOut { backdrop, escape, close }
    }
}

/// A truncating label in `rect` (full text for AccessKit and on hover).
/// A child Ui, so the parent's cursor stays below the header.
fn title_label(ui: &mut Ui, rect: Rect, text: RichText, valign: Align) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(valign)));
    child.add(egui::Label::new(text).truncate());
}

/// [`Dialog`] without a header: body padded 16 px, `footer` (if any) in a
/// bordered strip with right-aligned buttons, laid out right to left (add
/// the primary button first).
pub fn dialog(ctx: &egui::Context, id: Id, width: f32, body: impl FnOnce(&mut Ui), footer: Option<&mut dyn FnMut(&mut Ui)>) -> DialogOut {
    Dialog::new(id, width).show_split(ctx, body, footer)
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
                // Wrap long messages (a label in a row does not by default).
                ui.add(egui::Label::new(RichText::new(text).font(font(size::XS)).color(t.danger)).wrap());
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
