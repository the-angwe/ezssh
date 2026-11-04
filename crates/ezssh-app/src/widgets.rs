//! Small building blocks shared by the views: the button family, icon
//! buttons, switches, segmented controls, chips, the nav rail, rows, the
//! dialog card, context menus, the pane resize handle, status dots and
//! empty states. Every view draws its chrome with these so the app has one
//! vocabulary (`docs/design/ui-facelift.md`).

use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Modifiers,
    Order, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use crate::icons::Icon;
use crate::style::{
    self, CONTROL_H, CONTROL_H_SM, ICON_BTN, NAV_ROW_H, ROW_INSET, alpha, font, font_semibold,
    radius, size, tokens,
};
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

/// The four button looks. Primary and Danger are filled; Secondary is the
/// bordered `surface2` button of dialog footers; Ghost has no border and
/// fills only on hover (toolbars, inline actions).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// Button heights: `Md` 32 in dialogs, `Sm` 28 in panes and toolbars,
/// `Lg` 36 for the unlock card.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Lg,
    Md,
    Sm,
}

impl Size {
    fn height(self) -> f32 {
        match self {
            Size::Lg => 36.0,
            Size::Md => CONTROL_H,
            Size::Sm => CONTROL_H_SM,
        }
    }

    fn text(self) -> f32 {
        match self {
            Size::Lg | Size::Md => size::BASE,
            Size::Sm => size::SM,
        }
    }

    fn icon(self) -> f32 {
        match self {
            Size::Lg | Size::Md => 16.0,
            Size::Sm => 14.0,
        }
    }

    fn pad_x(self) -> f32 {
        match self {
            Size::Lg => 16.0,
            Size::Md => 12.0,
            Size::Sm => 10.0,
        }
    }
}

/// The one button builder. `Button::primary("Save").show(ui)`.
pub struct Button<'a> {
    text: Option<&'a str>,
    icon: Option<Icon>,
    variant: Variant,
    size: Size,
    enabled: bool,
    width: Option<f32>,
    /// Screen-reader name when the visible text is ambiguous ("Add").
    label: Option<&'a str>,
    tooltip: Option<&'a str>,
    /// Text, icon and border colour of a Secondary / Ghost button (the
    /// danger-outlined "Disable").
    tint: Option<Color32>,
    /// A trailing ▾ (the button opens a menu).
    chevron: bool,
}

impl<'a> Button<'a> {
    pub fn new(text: &'a str, variant: Variant) -> Self {
        Self {
            text: Some(text),
            icon: None,
            variant,
            size: Size::Md,
            enabled: true,
            width: None,
            label: None,
            tooltip: None,
            tint: None,
            chevron: false,
        }
    }

    pub fn primary(text: &'a str) -> Self {
        Self::new(text, Variant::Primary)
    }

    pub fn secondary(text: &'a str) -> Self {
        Self::new(text, Variant::Secondary)
    }

    pub fn ghost(text: &'a str) -> Self {
        Self::new(text, Variant::Ghost)
    }

    pub fn danger(text: &'a str) -> Self {
        Self::new(text, Variant::Danger)
    }

    /// A square button with only an icon; `label` names it.
    pub fn icon_only(icon: Icon, label: &'a str, variant: Variant) -> Self {
        Self {
            text: None,
            icon: Some(icon),
            label: Some(label),
            ..Self::new("", variant)
        }
    }

    pub fn icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// 28 px (panes, toolbars, in-card forms).
    pub fn small(self) -> Self {
        self.size(Size::Sm)
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// A fixed width (the unlock card's full-width button).
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn tooltip(mut self, tip: impl Into<Option<&'a str>>) -> Self {
        self.tooltip = tip.into();
        self
    }

    pub fn tint(mut self, c: impl Into<Option<Color32>>) -> Self {
        self.tint = c.into();
        self
    }

    pub fn chevron(mut self) -> Self {
        self.chevron = true;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = tokens(ui.ctx());
        let filled = matches!(self.variant, Variant::Primary | Variant::Danger);
        let text_font = if filled {
            font_semibold(self.size.text())
        } else {
            font(self.size.text())
        };
        let galley = self
            .text
            .filter(|s| !s.is_empty())
            .map(|s| ui.painter().layout_no_wrap(s.to_string(), text_font, t.fg));
        let text_w = galley.as_ref().map_or(0.0, |g| g.size().x);
        let icon_px = self.size.icon();
        let icon_w = if self.icon.is_some() { icon_px } else { 0.0 };
        let gap = if self.icon.is_some() && galley.is_some() {
            8.0
        } else {
            0.0
        };
        let chevron_w = if self.chevron { 12.0 + 4.0 } else { 0.0 };
        let content_w = icon_w + gap + text_w + chevron_w;
        let h = self.size.height();
        let natural = if galley.is_some() {
            content_w + 2.0 * self.size.pad_x()
        } else {
            h
        };
        let w = self.width.unwrap_or(natural);
        let enabled = self.enabled;
        let (rect, resp) = ui.allocate_exact_size(
            vec2(w, h),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if ui.is_rect_visible(rect) {
            let hovered = enabled && resp.hovered();
            let pressed = enabled && resp.is_pointer_button_down_on();
            let fade = |c: Color32| if enabled { c } else { alpha(c, 0.45) };
            let (fill, stroke, fg, icon_color) = match self.variant {
                Variant::Primary | Variant::Danger => {
                    let base = if self.variant == Variant::Primary {
                        t.accent
                    } else {
                        t.danger
                    };
                    let fill = if pressed {
                        style::over(base, Color32::BLACK, 0.12)
                    } else if hovered {
                        style::over(base, Color32::WHITE, 0.08)
                    } else {
                        base
                    };
                    (fade(fill), None, fade(t.on_accent), fade(t.on_accent))
                }
                Variant::Secondary => {
                    let tint = self.tint;
                    let fill = match (tint, pressed, hovered) {
                        (Some(c), true, _) => alpha(c, 0.18),
                        (Some(c), _, true) => alpha(c, 0.1),
                        (None, true, _) => style::over(t.surface3, t.fg, 0.04),
                        (None, _, true) => t.surface3,
                        _ => t.surface2,
                    };
                    let stroke = match tint {
                        Some(c) => alpha(c, 0.4),
                        None => t.border_strong,
                    };
                    let fg = tint.unwrap_or(t.fg);
                    (
                        fade(fill),
                        Some(fade(stroke)),
                        fade(fg),
                        fade(tint.unwrap_or(if hovered { t.fg } else { t.fg2 })),
                    )
                }
                Variant::Ghost => {
                    let tint = self.tint;
                    let fill = match (tint, pressed, hovered) {
                        (Some(c), true, _) => alpha(c, 0.18),
                        (Some(c), _, true) => alpha(c, 0.1),
                        (None, true, _) => t.surface3,
                        (None, _, true) => t.surface2,
                        _ => Color32::TRANSPARENT,
                    };
                    let fg = tint.unwrap_or(if hovered { t.fg } else { t.fg2 });
                    let icon = tint.unwrap_or(if hovered { t.fg } else { t.muted });
                    (fade(fill), None, fade(fg), fade(icon))
                }
            };
            if fill != Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, radius::MD, fill);
            }
            if let Some(s) = stroke {
                ui.painter()
                    .rect_stroke(rect, radius::MD, Stroke::new(1.0, s), StrokeKind::Inside);
            }
            let mut x = rect.center().x - content_w / 2.0;
            if let Some(icon) = self.icon {
                icon.paint(
                    ui,
                    Rect::from_center_size(
                        Pos2::new(x + icon_px / 2.0, rect.center().y),
                        Vec2::splat(icon_px),
                    ),
                    icon_px,
                    icon_color,
                );
                x += icon_w + gap;
            }
            if let Some(g) = galley {
                let th = g.size().y;
                ui.painter().galley_with_override_text_color(
                    Pos2::new(x, rect.center().y - th / 2.0),
                    g,
                    fg,
                );
                x += text_w;
            }
            if self.chevron {
                Icon::ChevronDown.paint(
                    ui,
                    Rect::from_center_size(
                        Pos2::new(x + 4.0 + 6.0, rect.center().y),
                        Vec2::splat(12.0),
                    ),
                    12.0,
                    icon_color,
                );
            }
            focus_ring(ui, &resp, rect);
        }
        // Outside the visibility check (a modal's sizing pass paints
        // nothing), with the real enabled state: a disabled button senses
        // hover only, which AccessKit would otherwise report as enabled.
        let label = self.label.or(self.text).unwrap_or_default();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
        let resp = resp.on_hover_cursor(if enabled {
            CursorIcon::PointingHand
        } else {
            CursorIcon::NotAllowed
        });
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
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), CONTROL_H),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add(ui);
        },
    );
}

/// `IconButton`: a square button with one icon, 28 px with a 16 px glyph
/// by default (`box_size` / `icon_size` for dense rows). `pressed` paints
/// the `aria-pressed` state: `accent_soft` fill, accent glyph.
pub fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    box_size: f32,
    icon_size: f32,
    pressed: bool,
    tooltip: &str,
) -> Response {
    icon_button_tinted(ui, icon, box_size, icon_size, pressed, tooltip, None)
}

/// [`icon_button`] at the default size.
pub fn icon_btn(ui: &mut Ui, icon: Icon, pressed: bool, tooltip: &str) -> Response {
    icon_button_tinted(ui, icon, ICON_BTN, 16.0, pressed, tooltip, None)
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

pub fn paint_icon_button(
    ui: &Ui,
    resp: &Response,
    rect: Rect,
    icon: Icon,
    icon_size: f32,
    pressed: bool,
    hover_color: Option<Color32>,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let t = tokens(ui.ctx());
    let hovered = resp.hovered();
    let down = resp.is_pointer_button_down_on();
    let color = if pressed {
        t.accent
    } else if hovered {
        hover_color.unwrap_or(t.fg)
    } else {
        t.muted
    };
    if pressed {
        ui.painter().rect_filled(rect, radius::MD, t.accent_soft);
    } else if down {
        ui.painter().rect_filled(rect, radius::MD, t.surface3);
    } else if hovered {
        ui.painter().rect_filled(
            rect,
            radius::MD,
            hover_color.map_or(t.surface2, |c| alpha(c, 0.12)),
        );
    }
    icon.paint(ui, rect, icon_size, color);
    focus_ring(ui, resp, rect);
}

/// An icon button painted at a given rect, with a screen-reader `label`
/// apart from its `tooltip`.
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
    /// When false it is painted only while hovered or focused.
    visible: bool,
}

impl<'a> IconBtn<'a> {
    pub fn new(icon: Icon, label: String, tooltip: &'a str) -> Self {
        Self {
            icon,
            icon_size: 14.0,
            label,
            tooltip,
            tone: None,
            hover_tone: None,
            hover_bg: None,
            enabled: true,
            visible: true,
        }
    }

    pub fn icon_size(mut self, px: f32) -> Self {
        self.icon_size = px;
        self
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
        let resp = ui.interact(
            rect,
            id,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &self.label));
        let hovered = enabled && resp.hovered();
        if (self.visible || resp.has_focus() || hovered) && ui.is_rect_visible(rect) {
            if hovered {
                ui.painter()
                    .rect_filled(rect, radius::MD, self.hover_bg.unwrap_or(tk.surface2));
            }
            let mut color = if hovered {
                self.hover_tone.or(self.tone).unwrap_or(tk.fg)
            } else {
                self.tone.unwrap_or(tk.muted)
            };
            if !enabled {
                color = alpha(color, 0.4);
            }
            self.icon.paint(ui, rect, self.icon_size, color);
            focus_ring(ui, &resp, rect);
        }
        let resp = resp.on_hover_cursor(if enabled {
            CursorIcon::PointingHand
        } else {
            CursorIcon::NotAllowed
        });
        resp.on_hover_text(self.tooltip)
    }
}

/// The 2 px accent outline of a keyboard-focused control. egui buttons take
/// focus only from Tab or `request_focus`, never from a click, so "has
/// focus" is the keyboard case.
pub fn focus_ring(ui: &Ui, resp: &Response, rect: Rect) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            radius::MD + 2.0,
            Stroke::new(2.0, tokens(ui.ctx()).accent),
            StrokeKind::Outside,
        );
    }
}

// ---------------------------------------------------------------- toggles

/// A 36 × 20 pill switch for boolean settings. Returns the response;
/// `changed()` when toggled.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let t = tokens(ui.ctx());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    a11y_selected(&resp, egui::Role::Switch, *on, label);
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(resp.id, *on);
        let hovered = resp.hovered();
        let track = if *on {
            t.accent
        } else if hovered {
            t.surface3
        } else {
            t.surface2
        };
        ui.painter().rect_filled(rect, radius::PILL, track);
        if !*on {
            ui.painter().rect_stroke(
                rect,
                radius::PILL,
                Stroke::new(1.0, t.border_strong),
                StrokeKind::Inside,
            );
        }
        let cx = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), how_on);
        let knob = if *on { t.on_accent } else { t.muted };
        ui.painter()
            .circle_filled(Pos2::new(cx, rect.center().y), 7.0, knob);
        focus_ring(ui, &resp, rect);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// One item of a [`segmented`] control.
pub struct Segment<'a, T> {
    pub value: T,
    pub icon: Option<Icon>,
    pub text: Option<&'a str>,
    /// Screen-reader name (and tooltip of an icon-only item).
    pub label: &'a str,
}

impl<'a, T> Segment<'a, T> {
    pub fn text(value: T, text: &'a str) -> Self {
        Self {
            value,
            icon: None,
            text: Some(text),
            label: text,
        }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A pill group of exclusive options; the selected one is lifted onto
/// `surface`. 28 px tall. Returns true when the choice changed.
pub fn segmented<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: Id,
    value: &mut T,
    items: &[Segment<'_, T>],
) -> bool {
    let t = tokens(ui.ctx());
    const PAD: f32 = 2.0;
    const ITEM_H: f32 = 24.0;
    let widths: Vec<f32> = items
        .iter()
        .map(|s| {
            let text_w = s.text.map_or(0.0, |x| {
                ui.painter()
                    .layout_no_wrap(x.to_string(), font(size::SM), t.fg)
                    .size()
                    .x
            });
            let icon_w = if s.icon.is_some() { 14.0 } else { 0.0 };
            let gap = if s.icon.is_some() && s.text.is_some() {
                6.0
            } else {
                0.0
            };
            if s.text.is_some() {
                text_w + icon_w + gap + 20.0
            } else {
                ITEM_H + 4.0
            }
        })
        .collect();
    let total: f32 =
        widths.iter().sum::<f32>() + PAD * 2.0 + PAD * (items.len().saturating_sub(1)) as f32;
    let (group, _) = ui.allocate_exact_size(vec2(total, ITEM_H + 2.0 * PAD), Sense::hover());
    ui.painter()
        .rect_filled(group, radius::MD + 1.0, t.surface2);
    let mut changed = false;
    let mut x = group.left() + PAD;
    for (seg, w) in items.iter().zip(widths) {
        let r = Rect::from_min_size(Pos2::new(x, group.top() + PAD), vec2(w, ITEM_H));
        x += w + PAD;
        let on = *value == seg.value;
        let resp = ui
            .interact(r, id.with(seg.label), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let hovered = resp.hovered();
        if on {
            ui.painter().rect_filled(r, radius::MD - 1.0, t.surface);
            ui.painter().rect_stroke(
                r,
                radius::MD - 1.0,
                Stroke::new(1.0, t.border),
                StrokeKind::Inside,
            );
        } else if hovered {
            ui.painter().rect_filled(r, radius::MD - 1.0, t.surface3);
        }
        let color = if on || hovered { t.fg } else { t.muted };
        let icon_w = if seg.icon.is_some() { 14.0 } else { 0.0 };
        let gap = if seg.icon.is_some() && seg.text.is_some() {
            6.0
        } else {
            0.0
        };
        let text_w = seg.text.map_or(0.0, |x| {
            ui.painter()
                .layout_no_wrap(x.to_string(), font(size::SM), t.fg)
                .size()
                .x
        });
        let mut cx = r.center().x - (icon_w + gap + text_w) / 2.0;
        if let Some(icon) = seg.icon {
            icon.paint(
                ui,
                Rect::from_center_size(Pos2::new(cx + 7.0, r.center().y), Vec2::splat(14.0)),
                14.0,
                if on { t.accent } else { color },
            );
            cx += icon_w + gap;
        }
        if let Some(text) = seg.text {
            paint_text(
                ui,
                Rect::from_min_max(Pos2::new(cx, r.top()), r.right_bottom()),
                text,
                font(size::SM),
                color,
            );
        }
        focus_ring(ui, &resp, r);
        a11y_selected(&resp, egui::Role::Button, on, seg.label);
        let resp = if seg.text.is_none() {
            resp.on_hover_text(seg.label)
        } else {
            resp
        };
        if resp.clicked() && !on {
            *value = seg.value.clone();
            changed = true;
        }
    }
    changed
}

/// A pill group of independent toggles (the pane toggles of the tab
/// strip): `(icon, on, label)`. Returns the index clicked.
pub fn toggle_group(ui: &mut Ui, id: Id, items: &[(Icon, bool, &str)]) -> Option<usize> {
    let t = tokens(ui.ctx());
    const PAD: f32 = 2.0;
    const ITEM: f32 = 24.0;
    let n = items.len() as f32;
    let (group, _) = ui.allocate_exact_size(
        vec2(n * (ITEM + 4.0) + PAD * (n + 1.0), ITEM + 2.0 * PAD),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(group, radius::MD + 1.0, t.surface2);
    let mut clicked = None;
    for (i, (icon, on, label)) in items.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(
                group.left() + PAD + i as f32 * (ITEM + 4.0 + PAD),
                group.top() + PAD,
            ),
            vec2(ITEM + 4.0, ITEM),
        );
        let resp = ui
            .interact(r, id.with(i), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let hovered = resp.hovered();
        if *on {
            ui.painter().rect_filled(r, radius::MD - 1.0, t.accent_soft);
        } else if hovered {
            ui.painter().rect_filled(r, radius::MD - 1.0, t.surface3);
        }
        let color = if *on {
            t.accent
        } else if hovered {
            t.fg
        } else {
            t.muted
        };
        icon.paint(ui, r, 14.0, color);
        focus_ring(ui, &resp, r);
        a11y_selected(&resp, egui::Role::Button, *on, label);
        if resp.on_hover_text(*label).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

// ---------------------------------------------------------------- chips, tiles, rows

/// An 18 px pill with 11 px text: counts, the status bar's cwd / X server /
/// update states. `color` tints the fill and text (muted on `surface2`
/// when `None`).
pub fn chip(ui: &mut Ui, text: &str, color: Option<Color32>, mono: bool) -> Response {
    let w = chip_width(ui, text, mono);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_chip(ui, rect, text, color, mono, false);
    }
    resp
}

/// A clickable [`chip`] named `label` for AccessKit.
pub fn chip_button(
    ui: &mut Ui,
    text: &str,
    color: Option<Color32>,
    mono: bool,
    label: &str,
) -> Response {
    let w = chip_width(ui, text, mono);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::click());
    if ui.is_rect_visible(rect) {
        paint_chip(ui, rect, text, color, mono, resp.hovered());
        focus_ring(ui, &resp, rect);
    }
    a11y(&resp, egui::Role::Button, label);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn chip_width(ui: &Ui, text: &str, mono: bool) -> f32 {
    let f = if mono {
        style::mono(size::XS)
    } else {
        font(size::XS)
    };
    ui.painter()
        .layout_no_wrap(text.to_string(), f, Color32::WHITE)
        .size()
        .x
        + 14.0
}

/// Paint a chip into `rect` (18 px tall).
pub fn paint_chip(
    ui: &Ui,
    rect: Rect,
    text: &str,
    color: Option<Color32>,
    mono: bool,
    hovered: bool,
) {
    let t = tokens(ui.ctx());
    let f = if mono {
        style::mono(size::XS)
    } else {
        font(size::XS)
    };
    let fg = color.unwrap_or(t.muted);
    let g = ui.painter().layout_no_wrap(text.to_string(), f, fg);
    let fill = match (color, hovered) {
        (Some(c), true) => alpha(c, 0.24),
        (Some(c), false) => alpha(c, 0.14),
        (None, true) => t.surface3,
        (None, false) => t.surface2,
    };
    ui.painter().rect_filled(rect, radius::PILL, fill);
    ui.painter().galley_with_override_text_color(
        Pos2::new(rect.left() + 7.0, rect.center().y - g.size().y / 2.0),
        g,
        fg,
    );
}

/// A rounded `accent_soft` tile with a centred glyph (dialog headers,
/// the unlock card, confirm dialogs).
pub fn icon_tile(ui: &Ui, rect: Rect, icon: Icon, color: Color32, icon_size: f32) {
    ui.painter()
        .rect_filled(rect, radius::LG, alpha(color, 0.14));
    icon.paint(ui, rect, icon_size, color);
}

/// Paint the hover / selected pill of a list row: inset [`ROW_INSET`]
/// horizontally, 1 px vertically, radius 6. Sidebar rows, file rows,
/// forwards rows and nav items all use it.
pub fn row_fill(ui: &Ui, rect: Rect, hovered: bool, selected: bool) {
    let t = tokens(ui.ctx());
    let pill = Rect::from_min_max(
        Pos2::new(rect.left() + ROW_INSET, rect.top() + 1.0),
        Pos2::new(rect.right() - ROW_INSET, rect.bottom() - 1.0),
    );
    if selected {
        ui.painter().rect_filled(pill, radius::MD, t.accent_soft);
    } else if hovered {
        ui.painter().rect_filled(pill, radius::MD, t.surface2);
    }
}

/// A side rail of sections (settings, the session dialog): 32 px rows with
/// a pill selection, 16 px icons, 13 px text. `Role::Tab` for AccessKit.
pub fn nav_rail<T: PartialEq + Copy>(
    ui: &mut Ui,
    width: f32,
    items: &[(T, &str, Icon)],
    current: &mut T,
) {
    let t = tokens(ui.ctx());
    ui.spacing_mut().item_spacing.y = 2.0;
    for (value, label, icon) in items {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, NAV_ROW_H), Sense::click());
        let on = *current == *value;
        let hovered = resp.hovered();
        row_fill(ui, rect, hovered, on);
        let (icon_c, text_c) = if on {
            (t.accent, t.fg)
        } else if hovered {
            (t.fg, t.fg)
        } else {
            (t.muted, t.fg2)
        };
        icon.paint(
            ui,
            Rect::from_center_size(
                Pos2::new(rect.left() + ROW_INSET + 10.0 + 8.0, rect.center().y),
                Vec2::splat(16.0),
            ),
            16.0,
            icon_c,
        );
        paint_text(
            ui,
            Rect::from_min_max(
                Pos2::new(rect.left() + ROW_INSET + 10.0 + 16.0 + 10.0, rect.top()),
                Pos2::new(rect.right() - ROW_INSET, rect.bottom()),
            ),
            label,
            font(size::BASE),
            text_c,
        );
        focus_ring(ui, &resp, rect.shrink2(vec2(ROW_INSET, 1.0)));
        a11y_selected(&resp, egui::Role::Tab, on, label);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            *current = *value;
        }
    }
}

// ---------------------------------------------------------------- status

/// A 6 px dot by status; errors show the alert icon.
pub fn status_dot(ui: &Ui, center: Pos2, status: TabStatus, icon_size: f32) {
    let t = tokens(ui.ctx());
    let color = match status {
        TabStatus::Error => {
            Icon::AlertCircle.paint(
                ui,
                Rect::from_center_size(center, Vec2::splat(icon_size)),
                icon_size,
                t.danger,
            );
            return;
        }
        TabStatus::Connected => t.success,
        TabStatus::Connecting => alpha(t.warning, pulse(ui.ctx())),
        TabStatus::Closed => alpha(t.muted, 0.6),
    };
    ui.painter().circle_filled(center, 3.0, color);
}

/// A pulsing opacity (1 → 0.5 → 1 over 2 s). Keeps the
/// frame clock running while anything pulses.
pub fn pulse(ctx: &egui::Context) -> f32 {
    ctx.request_repaint_after(Duration::from_millis(50));
    let t = ctx.input(|i| i.time) as f32;
    0.75 + 0.25 * (t * std::f32::consts::PI).cos()
}

// ---------------------------------------------------------------- empty state

/// An empty pane: icon, title, body and an optional action button. Returns
/// the action button's response, if any.
pub fn empty_state(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    body: &str,
    action: Option<(&str, bool)>,
    compact: bool,
) -> Option<Response> {
    let t = tokens(ui.ctx());
    let mut out = None;
    ui.vertical_centered(|ui| {
        let icon_size = if compact { 24.0 } else { 32.0 };
        let content_h = icon_size
            + 10.0
            + 20.0
            + if body.is_empty() { 0.0 } else { 36.0 }
            + if action.is_some() { 44.0 } else { 0.0 };
        let pad = if compact {
            24.0
        } else {
            ((ui.available_height() - content_h) / 2.0).max(40.0)
        };
        ui.add_space(pad);
        icon.show(ui, icon_size, t.muted);
        ui.add_space(10.0);
        ui.label(
            RichText::new(title)
                .font(font_semibold(size::MD))
                .color(t.fg),
        );
        if !body.is_empty() {
            ui.add_space(4.0);
            ui.allocate_ui(vec2(280.0, 40.0), |ui| {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.label(RichText::new(body).font(font(size::SM)).color(t.muted));
                });
            });
        }
        if let Some((label, enabled)) = action {
            ui.add_space(12.0);
            out = Some(Button::primary(label).small().enabled(enabled).show(ui));
        }
    });
    out
}

// ---------------------------------------------------------------- dialogs

/// The dialog's header.
pub enum DialogHead<'a> {
    None,
    /// The one header (56 px): an optional icon tile, a 16 px title, an
    /// optional mono subtitle and a ✕ button.
    Full {
        icon: Option<Icon>,
        icon_color: Color32,
        title: &'a str,
        subtitle: Option<&'a str>,
        mono: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogFooter {
    None,
    /// Right-aligned under the body, inside the padding.
    Inline,
    /// A footer strip under the body, with a hairline above.
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

pub const DIALOG_HEAD_H: f32 = 56.0;
pub const DIALOG_FOOTER_H: f32 = 60.0;
pub const DIALOG_PAD: i8 = 24;

/// The dialog card frame (surface, strong border, 12 px radius, dialog
/// shadow), for dialogs that lay their body out by hand.
pub fn dialog_frame(t: &style::Tokens) -> Frame {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border_strong))
        .corner_radius(CornerRadius::same(radius::XL as u8))
        .shadow(style::dialog_shadow())
}

/// Paint the standard header into `rect` (hairline below, icon tile,
/// title, subtitle, ✕). Returns true when ✕ was clicked. Dialogs that size
/// themselves by hand (settings, the session editor) share it with
/// [`Dialog`].
#[allow(clippy::too_many_arguments)]
pub fn dialog_header(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    icon: Option<Icon>,
    icon_color: Color32,
    title: &str,
    subtitle: Option<&str>,
    mono: bool,
    busy: bool,
) -> bool {
    let t = tokens(ui.ctx());
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, t.border),
    );
    let inner = rect.shrink2(vec2(20.0, 0.0));
    let mut text_l = inner.left();
    if let Some(icon) = icon {
        let tile = Rect::from_center_size(
            Pos2::new(inner.left() + 14.0, inner.center().y),
            Vec2::splat(28.0),
        );
        icon_tile(ui, tile, icon, icon_color, 16.0);
        text_l = tile.right() + 12.0;
    }
    let x_r = Rect::from_center_size(
        Pos2::new(inner.right() - ICON_BTN / 2.0 + 4.0, inner.center().y),
        Vec2::splat(ICON_BTN),
    );
    let text_r = Rect::from_min_max(
        Pos2::new(text_l, inner.top()),
        Pos2::new(x_r.left() - 12.0, inner.bottom()),
    );
    let title_text = RichText::new(title)
        .font(font_semibold(size::LG))
        .color(t.fg);
    match subtitle {
        Some(sub) => {
            let top = Rect::from_min_max(
                text_r.min,
                Pos2::new(text_r.right(), text_r.center().y + 1.0),
            );
            let bottom = Rect::from_min_max(
                Pos2::new(text_r.left(), text_r.center().y + 1.0),
                text_r.max,
            );
            title_label(ui, top, title_text, Align::BOTTOM);
            let sub_font = if mono {
                style::mono(size::SM)
            } else {
                font(size::SM)
            };
            title_label(
                ui,
                bottom,
                RichText::new(sub).font(sub_font).color(t.muted),
                Align::TOP,
            );
        }
        None => title_label(ui, text_r, title_text, Align::Center),
    }
    let x = ui.interact(
        x_r,
        id.with("close"),
        if busy { Sense::hover() } else { Sense::click() },
    );
    paint_icon_button(ui, &x, x_r, Icon::X, 16.0, false, None);
    a11y(&x, egui::Role::Button, "Close dialog");
    x.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// Paint the footer hairline and run `add` right-to-left inside `rect`
/// (horizontal padding `pad`).
pub fn dialog_footer(ui: &mut Ui, rect: Rect, pad: f32, add: impl FnOnce(&mut Ui)) {
    let t = tokens(ui.ctx());
    ui.painter()
        .hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, t.border));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(rect.shrink2(vec2(pad, (rect.height() - CONTROL_H) / 2.0))),
        |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                add(ui);
            });
        },
    );
}

/// The dialog card (fixed width, at most 90 % of the window high): the
/// one dialog chrome, with an optional header, a padded body that scrolls
/// once it no longer fits, and a footer.
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
        Self {
            id,
            width,
            head: DialogHead::None,
            footer: DialogFooter::None,
            padding: DIALOG_PAD,
            row_gap: None,
            busy: false,
        }
    }

    /// The backup / restore / sync / import dialogs: 16 px between rows
    /// (add a [`Dialog::header`]).
    pub fn data(id: Id, width: f32) -> Self {
        Self::new(id, width).row_gap(16.0)
    }

    /// The header; a subtitle is shown in the mono font (paths, hosts).
    pub fn header(
        mut self,
        icon: impl Into<Option<Icon>>,
        icon_color: Color32,
        title: &'a str,
        subtitle: Option<&'a str>,
    ) -> Self {
        self.head = DialogHead::Full {
            icon: icon.into(),
            icon_color,
            title,
            subtitle,
            mono: true,
        };
        self
    }

    /// The subtitle in the UI font (a sentence, a session name).
    pub fn plain_subtitle(mut self) -> Self {
        if let DialogHead::Full { mono, .. } = &mut self.head {
            *mono = false;
        }
        self
    }

    pub fn footer(mut self, kind: DialogFooter) -> Self {
        self.footer = kind;
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
    pub fn show_split(
        mut self,
        ctx: &egui::Context,
        body: impl FnOnce(&mut Ui),
        mut footer: Option<&mut dyn FnMut(&mut Ui)>,
    ) -> DialogOut {
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
        let frame = dialog_frame(t);
        let mut close = false;
        let resp = egui::Modal::new(id)
            .backdrop_color(Color32::from_black_alpha(128))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                let default_gap = ui.spacing().item_spacing.y;
                ui.spacing_mut().item_spacing.y = 0.0;
                let head_h = match self.head {
                    DialogHead::None => 0.0,
                    DialogHead::Full {
                        icon,
                        icon_color,
                        title,
                        subtitle,
                        mono,
                    } => {
                        let (hrect, _) =
                            ui.allocate_exact_size(vec2(width, DIALOG_HEAD_H), Sense::hover());
                        if dialog_header(
                            ui, id, hrect, icon, icon_color, title, subtitle, mono, self.busy,
                        ) {
                            close = true;
                        }
                        DIALOG_HEAD_H
                    }
                };
                // Body. A Modal lays out in last frame's size and a ScrollArea
                // never grows past the space it is offered, so a body that fits
                // is laid out plainly (the card follows its height); only one
                // taller than `max_body` (judged on last frame's height)
                // scrolls, at that height.
                let foot_h = if self.footer == DialogFooter::Strip {
                    DIALOG_FOOTER_H
                } else {
                    0.0
                };
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
                                ui.add_space(4.0);
                                ui.allocate_ui_with_layout(
                                    vec2(width - 2.0 * pad, CONTROL_H),
                                    Layout::right_to_left(Align::Center),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        add(ui, DialogPart::Footer);
                                    },
                                );
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
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(content_h_id, content_h));
                    ui.ctx().request_repaint();
                }
                if self.footer == DialogFooter::Strip {
                    let (strip, _) =
                        ui.allocate_exact_size(vec2(width, DIALOG_FOOTER_H), Sense::hover());
                    dialog_footer(ui, strip, pad, |ui| add(ui, DialogPart::Footer));
                }
            });
        let backdrop = resp.backdrop_response.clicked();
        let close = close || resp.response.should_close();
        let escape = resp.is_top_modal
            && !resp.any_popup_open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        DialogOut {
            backdrop,
            escape,
            close,
        }
    }
}

/// A truncating label in `rect` (full text for AccessKit and on hover).
/// A child Ui, so the parent's cursor stays below the header.
fn title_label(ui: &mut Ui, rect: Rect, text: RichText, valign: Align) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(valign)),
    );
    child.add(egui::Label::new(text).truncate());
}

/// [`Dialog`] without a header: the compact confirm card. Body padded
/// 24 px, `footer` (if any) in a strip with right-aligned buttons, laid
/// out right to left (add the primary button first).
pub fn dialog(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    body: impl FnOnce(&mut Ui),
    footer: Option<&mut dyn FnMut(&mut Ui)>,
) -> DialogOut {
    Dialog::new(id, width).show_split(ctx, body, footer)
}

/// The compact dialog's heading: a 36 px icon tile beside a 14 px title
/// and a 13 px body.
pub fn dialog_heading(ui: &mut Ui, icon: Icon, icon_color: Color32, title: &str, body: &str) {
    let t = tokens(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
        icon_tile(ui, r, icon, icon_color, 18.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(1.0);
            ui.label(
                RichText::new(title)
                    .font(font_semibold(size::MD))
                    .color(t.fg),
            );
            if !body.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(body).font(font(size::BASE)).color(t.fg2))
                        .wrap(),
                );
            }
        });
    });
}

/// Paint the input frame: `surface2` fill painted by the TextEdit itself,
/// a strong border at rest, the accent border plus a 3 px `accent_soft`
/// ring when focused, the danger pair when `error`.
pub fn input_frame(ui: &Ui, rect: Rect, focused: bool, error: bool) {
    let t = tokens(ui.ctx());
    let border = if error {
        t.danger
    } else if focused {
        t.accent
    } else {
        t.border_strong
    };
    if focused || error {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            radius::MD + 3.0,
            Stroke::new(3.0, alpha(border, 0.18)),
            StrokeKind::Inside,
        );
    }
    ui.painter().rect_stroke(
        rect,
        radius::MD,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
}

/// A text input: `surface2` field, 32 px, radius 6, accent focus ring.
pub fn text_input(
    ui: &mut Ui,
    value: &mut String,
    hint: &str,
    width: f32,
    password: bool,
) -> Response {
    let t = tokens(ui.ctx());
    let edit = egui::TextEdit::singleline(value)
        .hint_text(RichText::new(hint).color(t.muted))
        .desired_width(width)
        .font(font(size::BASE))
        .margin(Margin::symmetric(10, 7))
        .min_size(vec2(0.0, CONTROL_H))
        .password(password)
        .background_color(t.surface2);
    let resp = ui.add(edit);
    if password {
        crate::form::forget_history(ui, &resp);
    }
    input_frame(ui, resp.rect, resp.has_focus(), false);
    resp
}

/// Red alert strip (`role="alert"` in the unlock / prompt dialogs).
pub fn error_strip(ui: &mut Ui, text: &str) {
    let t = tokens(ui.ctx());
    Frame::new()
        .fill(alpha(t.danger, 0.1))
        .stroke(Stroke::new(1.0, alpha(t.danger, 0.4)))
        .corner_radius(CornerRadius::same(radius::MD as u8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                Icon::AlertCircle.show(ui, 14.0, t.danger);
                // Wrap long messages (a label in a row does not by default).
                ui.add(
                    egui::Label::new(RichText::new(text).font(font(size::SM)).color(t.danger))
                        .wrap(),
                );
            });
        });
}

// ---------------------------------------------------------------- menu

/// One row of a [`Menu`].
pub enum MenuEntry<A> {
    Item {
        label: String,
        action: A,
        enabled: bool,
        danger: bool,
        /// Tooltip while disabled (why it is).
        hint: Option<&'static str>,
        icon: Option<Icon>,
        /// Right-aligned shortcut text ("Ctrl+K").
        shortcut: Option<&'static str>,
        /// A check mark on the left (the current view mode).
        checked: bool,
    },
    Separator,
}

impl<A> MenuEntry<A> {
    pub fn item(label: impl Into<String>, action: A) -> Self {
        Self::Item {
            label: label.into(),
            action,
            enabled: true,
            danger: false,
            hint: None,
            icon: None,
            shortcut: None,
            checked: false,
        }
    }

    pub fn danger(label: impl Into<String>, action: A) -> Self {
        Self::Item {
            label: label.into(),
            action,
            enabled: true,
            danger: true,
            hint: None,
            icon: None,
            shortcut: None,
            checked: false,
        }
    }

    pub fn enabled(mut self, on: bool) -> Self {
        if let Self::Item { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    pub fn icon(mut self, i: Icon) -> Self {
        if let Self::Item { icon, .. } = &mut self {
            *icon = Some(i);
        }
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        if let Self::Item { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
}

/// A context menu: a popup at `pos` that closes on a choice, Escape or a
/// press outside it. Radius 8, 28 px items with a pill hover, 13 px text,
/// optional leading icons and right-aligned shortcuts.
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

const MENU_ITEM_H: f32 = 28.0;
const MENU_PAD: f32 = 6.0;

impl<A: Clone> Menu<A> {
    pub fn new(pos: Pos2, entries: Vec<MenuEntry<A>>) -> Self {
        Self {
            pos,
            entries,
            just_opened: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: Id) -> MenuResult<A> {
        let t = tokens(ctx);
        let mut chosen = None;
        let frame = Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border_strong))
            .corner_radius(CornerRadius::same(radius::LG as u8))
            .inner_margin(Margin::same(MENU_PAD as i8))
            .shadow(style::popup_shadow());
        let any_icon = self.entries.iter().any(|e| {
            matches!(
                e,
                MenuEntry::Item { icon: Some(_), .. } | MenuEntry::Item { checked: true, .. }
            )
        });
        let lead = if any_icon { 16.0 + 10.0 } else { 0.0 };
        // An Area offers the whole screen as available width; size the menu
        // from its widest row instead (min 200 px).
        let width = ctx.fonts_mut(|f| {
            self.entries
                .iter()
                .filter_map(|e| match e {
                    MenuEntry::Item {
                        label, shortcut, ..
                    } => {
                        let text = f
                            .layout_no_wrap(label.clone(), font(size::BASE), t.fg)
                            .size()
                            .x;
                        let sc = shortcut.map_or(0.0, |s| {
                            16.0 + f
                                .layout_no_wrap(s.to_string(), font(size::XS), t.muted)
                                .size()
                                .x
                        });
                        Some(12.0 + lead + text + sc + 12.0)
                    }
                    MenuEntry::Separator => None,
                })
                .fold(200.0_f32, f32::max)
        });
        let area = egui::Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(self.pos)
            .constrain(true)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for entry in &self.entries {
                        match entry {
                            MenuEntry::Separator => {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                                ui.painter().hline(
                                    (r.left() + 6.0)..=(r.right() - 6.0),
                                    r.center().y,
                                    Stroke::new(1.0, t.border),
                                );
                            }
                            MenuEntry::Item {
                                label,
                                action,
                                enabled,
                                danger,
                                hint,
                                icon,
                                shortcut,
                                checked,
                            } => {
                                let (r, resp) = ui
                                    .add_enabled_ui(*enabled, |ui| {
                                        ui.allocate_exact_size(
                                            vec2(width, MENU_ITEM_H),
                                            Sense::click(),
                                        )
                                    })
                                    .inner;
                                let mut color = if *danger { t.danger } else { t.fg };
                                let mut icon_color = if *danger { t.danger } else { t.muted };
                                if !enabled {
                                    color = alpha(color, 0.4);
                                    icon_color = alpha(icon_color, 0.4);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(
                                        r,
                                        radius::MD - 1.0,
                                        if *danger {
                                            alpha(t.danger, 0.12)
                                        } else {
                                            t.surface2
                                        },
                                    );
                                    icon_color = color;
                                }
                                let mut x = r.left() + 12.0;
                                if *checked {
                                    Icon::Check.paint(
                                        ui,
                                        Rect::from_center_size(
                                            Pos2::new(x + 8.0, r.center().y),
                                            Vec2::splat(16.0),
                                        ),
                                        14.0,
                                        t.accent,
                                    );
                                } else if let Some(icon) = icon {
                                    icon.paint(
                                        ui,
                                        Rect::from_center_size(
                                            Pos2::new(x + 8.0, r.center().y),
                                            Vec2::splat(16.0),
                                        ),
                                        16.0,
                                        icon_color,
                                    );
                                }
                                x += lead;
                                let mut right = r.right() - 12.0;
                                if let Some(s) = shortcut {
                                    let g = ui.painter().layout_no_wrap(
                                        s.to_string(),
                                        font(size::XS),
                                        t.muted,
                                    );
                                    right -= g.size().x;
                                    ui.painter().galley_with_override_text_color(
                                        Pos2::new(right, r.center().y - g.size().y / 2.0),
                                        g,
                                        alpha(t.muted, if *enabled { 1.0 } else { 0.4 }),
                                    );
                                    right -= 12.0;
                                }
                                paint_text(
                                    ui,
                                    Rect::from_min_max(
                                        Pos2::new(x, r.top()),
                                        Pos2::new(right, r.bottom()),
                                    ),
                                    label,
                                    font(size::BASE),
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
                i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !area.response.rect.contains(p))
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

/// A pane resize handle: a 4 px strip on the right edge of a pane. Drag
/// resizes, double-click restores the default, and when focused (click
/// it, or Tab) ←/→ step 16 px (Shift: 64), Home restores the default and
/// End goes to the maximum.
pub fn resize_strip(
    ui: &mut Ui,
    id: Id,
    pane: Rect,
    width: &mut f32,
    bounds: Bounds,
    label: &str,
) -> ResizeOut {
    let Bounds { min, max, default } = bounds;
    let t = tokens(ui.ctx());
    let strip = Rect::from_min_max(
        Pos2::new(pane.right() - 4.0, pane.top()),
        pane.right_bottom(),
    );
    let resp = ui
        .interact(strip, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal);
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
            let step = if i.modifiers.shift {
                KEY_STEP_LARGE
            } else {
                KEY_STEP
            };
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
        ui.painter()
            .rect_filled(strip, 0.0, alpha(t.accent, if active { 0.6 } else { 0.4 }));
        ui.painter().rect_filled(
            Rect::from_min_max(
                Pos2::new(strip.right() - 2.0, strip.top()),
                strip.right_bottom(),
            ),
            0.0,
            if active {
                t.accent
            } else {
                alpha(t.accent, 0.6)
            },
        );
    }
    out
}

// ---------------------------------------------------------------- misc

/// Truncate `text` with an ellipsis to fit `max_width` at `font_id`.
pub fn elide(
    ui: &Ui,
    text: &str,
    font_id: egui::FontId,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.to_string(), font_id, tokens(ui.ctx()).fg);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.painter().layout_job(job)
}

/// Paint `text` left-aligned and vertically centred in `rect`, elided.
pub fn paint_text(ui: &Ui, rect: Rect, text: &str, font_id: egui::FontId, color: Color32) -> Rect {
    let g = elide(ui, text, font_id, rect.width());
    let pos = Align2::LEFT_CENTER
        .align_size_within_rect(g.size(), rect)
        .min;
    let r = Rect::from_min_size(pos, g.size());
    ui.painter().galley_with_override_text_color(pos, g, color);
    r
}
