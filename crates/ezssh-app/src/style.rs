//! Design tokens: the colours, sizes and type scale of the app, turned into
//! an egui `Style`.
//!
//! Views read colours from [`Tokens`] (one per theme, fetched with
//! [`tokens`]) rather than from egui's visuals, so rows, badges and tabs
//! look the same everywhere.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2, Visuals};

/// `theme` setting: window chrome only (terminal colours are the palette).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// Parse the stored `theme` value; anything but `light` is dark.
    pub fn from_setting(s: Option<&str>) -> Self {
        if s == Some("light") { Self::Light } else { Self::Dark }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

/// The CSS custom properties of one theme.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub bg: Color32,
    pub surface: Color32,
    pub surface2: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub fg: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub selection: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const DARK: Tokens = Tokens {
    bg: rgb(24, 26, 30),
    surface: rgb(32, 34, 39),
    surface2: rgb(42, 45, 52),
    border: rgb(58, 62, 70),
    border_strong: rgb(92, 98, 112),
    fg: rgb(226, 228, 233),
    muted: rgb(150, 156, 168),
    accent: rgb(88, 148, 225),
    success: rgb(74, 170, 120),
    warning: rgb(220, 170, 70),
    danger: rgb(230, 95, 95),
    selection: rgb(62, 96, 152),
};

pub const LIGHT: Tokens = Tokens {
    bg: rgb(243, 244, 247),
    surface: rgb(252, 252, 253),
    surface2: rgb(234, 236, 241),
    border: rgb(205, 209, 217),
    border_strong: rgb(188, 194, 205),
    fg: rgb(24, 27, 33),
    muted: rgb(100, 109, 123),
    accent: rgb(30, 110, 205),
    success: rgb(30, 130, 80),
    warning: rgb(180, 125, 30),
    danger: rgb(185, 40, 50),
    selection: rgb(176, 206, 247),
};

pub fn tokens_for(theme: Theme) -> &'static Tokens {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// Tokens of the theme the context is showing.
pub fn tokens(ctx: &egui::Context) -> &'static Tokens {
    tokens_for(if ctx.global_style().visuals.dark_mode { Theme::Dark } else { Theme::Light })
}

/// `rgb(var(--x) / a)`: the colour at `alpha` (0–1) opacity.
pub fn alpha(c: Color32, alpha: f32) -> Color32 {
    let [r, g, b, _] = c.to_array();
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `c` painted at `a` opacity over `base`, as an opaque colour, for a
/// translucent token over a known background.
pub fn over(base: Color32, c: Color32, a: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * a).round() as u8;
    let [br, bg, bb, _] = base.to_array();
    let [cr, cg, cb, _] = c.to_array();
    Color32::from_rgb(mix(br, cr), mix(bg, cg), mix(bb, cb))
}

/// The text scale: xs 11, sm 12, base 13,
/// md 14, lg 15 px.
pub mod size {
    pub const XS: f32 = 11.0;
    pub const SM: f32 = 12.0;
    pub const BASE: f32 = 13.0;
    pub const MD: f32 = 14.0;
    pub const LG: f32 = 15.0;
}

/// Fixed chrome heights (px).
pub const ROW_H: f32 = 28.0;
pub const TAB_BAR_H: f32 = 32.0;
pub const STATUS_BAR_H: f32 = 24.0;

/// Family for semibold UI text (`font-medium` / `font-semibold`); falls
/// back to the regular UI face when the system has no such weight.
pub fn semibold() -> FontFamily {
    crate::ui_font::semibold_family()
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

/// Install the theme's visuals and the type scale into `ctx`.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let t = tokens_for(theme);
    let mut visuals = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
    let r2 = CornerRadius::same(2);
    visuals.override_text_color = None;
    visuals.panel_fill = t.surface;
    visuals.window_fill = t.surface;
    visuals.window_stroke = Stroke::new(1.0, t.border);
    visuals.window_corner_radius = CornerRadius::same(3);
    visuals.menu_corner_radius = CornerRadius::same(3);
    visuals.extreme_bg_color = t.surface2;
    visuals.text_edit_bg_color = Some(t.surface2);
    visuals.faint_bg_color = t.surface2;
    visuals.code_bg_color = t.surface2;
    visuals.hyperlink_color = t.accent;
    visuals.selection.bg_fill = alpha(t.accent, 0.35);
    visuals.selection.stroke = Stroke::new(1.0, t.fg);
    visuals.warn_fg_color = t.warning;
    visuals.error_fg_color = t.danger;
    visuals.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(115) };
    visuals.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) };

    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = t.surface;
    w.noninteractive.weak_bg_fill = t.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.fg);
    w.noninteractive.corner_radius = r2;
    w.inactive.bg_fill = t.surface2;
    w.inactive.weak_bg_fill = t.surface2;
    w.inactive.bg_stroke = Stroke::new(1.0, t.border);
    w.inactive.fg_stroke = Stroke::new(1.0, t.fg);
    w.inactive.corner_radius = r2;
    w.hovered.bg_fill = over(t.surface2, t.fg, 0.06);
    w.hovered.weak_bg_fill = over(t.surface2, t.fg, 0.06);
    w.hovered.bg_stroke = Stroke::new(1.0, t.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.0, t.fg);
    w.hovered.corner_radius = r2;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = over(t.surface2, t.accent, 0.25);
    w.active.weak_bg_fill = over(t.surface2, t.accent, 0.25);
    w.active.bg_stroke = Stroke::new(1.0, t.accent);
    w.active.fg_stroke = Stroke::new(1.0, t.fg);
    w.active.corner_radius = r2;
    w.active.expansion = 0.0;
    w.open.bg_fill = t.surface2;
    w.open.weak_bg_fill = t.surface2;
    w.open.bg_stroke = Stroke::new(1.0, t.accent);
    w.open.fg_stroke = Stroke::new(1.0, t.fg);
    w.open.corner_radius = r2;

    ctx.set_visuals_of(if theme == Theme::Dark { egui::Theme::Dark } else { egui::Theme::Light }, visuals);
    ctx.set_theme(if theme == Theme::Dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, font(size::XS)),
            (TextStyle::Body, font(size::BASE)),
            (TextStyle::Button, font(size::SM)),
            (TextStyle::Heading, font_semibold(size::LG)),
            (TextStyle::Monospace, FontId::new(size::SM, FontFamily::Monospace)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(6.0, 4.0);
        s.button_padding = Vec2::new(8.0, 4.0);
        s.interact_size = Vec2::new(24.0, 24.0);
        s.menu_margin = egui::Margin::symmetric(0, 4);
        s.window_margin = egui::Margin::same(0);
        s.combo_width = 160.0;
        s.text_edit_width = 220.0;
        s.scroll.bar_width = 6.0;
        s.scroll.floating = true;
        style.interaction.selectable_labels = false;
        // `prefers-reduced-motion` has no egui equivalent; animations stay short.
        style.animation_time = 0.12;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_setting_round_trip() {
        assert_eq!(Theme::from_setting(Some("light")), Theme::Light);
        assert_eq!(Theme::from_setting(Some("dark")), Theme::Dark);
        assert_eq!(Theme::from_setting(Some("solarized")), Theme::Dark);
        assert_eq!(Theme::from_setting(None), Theme::Dark);
        assert_eq!(Theme::Light.toggled().as_setting(), "dark");
    }

    #[test]
    fn over_blends_towards_the_colour() {
        let c = over(Color32::BLACK, Color32::WHITE, 0.5);
        assert_eq!(c, Color32::from_rgb(128, 128, 128));
        assert_eq!(over(DARK.bg, DARK.accent, 0.0), DARK.bg);
        assert_eq!(over(DARK.bg, DARK.accent, 1.0), DARK.accent);
    }
}
