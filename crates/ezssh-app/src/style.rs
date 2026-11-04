//! Design tokens: the colours, sizes and type scale of the app, turned into
//! an egui `Style`.
//!
//! Views read colours from [`Tokens`] (one per theme, fetched with
//! [`tokens`]) rather than from egui's visuals, so rows, badges and tabs
//! look the same everywhere. The direction is "quiet layers": tonal
//! surfaces instead of hairlines, one radius scale, pill selections, the
//! accent reserved for state (`docs/design/ui-facelift.md`).

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
        if s == Some("light") {
            Self::Light
        } else {
            Self::Dark
        }
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

/// The colour tokens of one theme.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    /// Window ground and terminal surround (the recessed layer).
    pub bg: Color32,
    /// Sidebar, panes, bars, dialogs, menus.
    pub surface: Color32,
    /// Inputs, hover fills.
    pub surface2: Color32,
    /// Pressed fills, switch and progress tracks.
    pub surface3: Color32,
    /// Hairlines: `fg` at 8 %, so one token works on every surface.
    pub border: Color32,
    /// Input borders, secondary buttons, the dialog edge: `fg` at 16 %.
    pub border_strong: Color32,
    pub fg: Color32,
    /// Secondary text, row labels at rest.
    pub fg2: Color32,
    /// Captions and icons at rest (≥ 4.5 : 1 on `surface`).
    pub muted: Color32,
    pub accent: Color32,
    /// Selection pills and the focus ring: `accent` at 12–14 %.
    pub accent_soft: Color32,
    /// Text on the primary button.
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub selection: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const(r, g, b, a)
}

pub const DARK: Tokens = Tokens {
    bg: rgb(24, 26, 30),
    surface: rgb(31, 33, 38),
    surface2: rgb(42, 45, 52),
    surface3: rgb(52, 56, 66),
    border: rgba(230, 232, 237, 20),
    border_strong: rgba(230, 232, 237, 41),
    fg: rgb(230, 232, 237),
    fg2: rgb(180, 185, 196),
    muted: rgb(139, 145, 158),
    accent: rgb(108, 166, 245),
    accent_soft: rgba(108, 166, 245, 36),
    on_accent: rgb(15, 18, 23),
    success: rgb(79, 179, 126),
    warning: rgb(224, 176, 74),
    danger: rgb(239, 107, 107),
    selection: rgb(62, 96, 152),
};

pub const LIGHT: Tokens = Tokens {
    bg: rgb(243, 244, 247),
    surface: rgb(255, 255, 255),
    surface2: rgb(238, 240, 244),
    surface3: rgb(227, 230, 236),
    border: rgba(26, 29, 35, 23),
    border_strong: rgba(26, 29, 35, 46),
    fg: rgb(26, 29, 35),
    fg2: rgb(75, 82, 97),
    muted: rgb(105, 112, 128),
    accent: rgb(30, 110, 208),
    accent_soft: rgba(30, 110, 208, 31),
    on_accent: rgb(255, 255, 255),
    success: rgb(31, 138, 80),
    warning: rgb(168, 110, 18),
    danger: rgb(194, 58, 66),
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
    tokens_for(if ctx.global_style().visuals.dark_mode {
        Theme::Dark
    } else {
        Theme::Light
    })
}

/// `c` at `alpha` (0–1) opacity. `c` is taken as opaque (its own alpha is
/// replaced, not multiplied).
pub fn alpha(c: Color32, alpha: f32) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `c` painted at `a` opacity over `base`, as an opaque colour, for a
/// translucent token over a known background.
pub fn over(base: Color32, c: Color32, a: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * a).round() as u8;
    let [br, bg, bb, _] = base.to_srgba_unmultiplied();
    let [cr, cg, cb, _] = c.to_srgba_unmultiplied();
    Color32::from_rgb(mix(br, cr), mix(bg, cg), mix(bb, cb))
}

/// The text scale: xs 11 (captions, badges), sm 12 (secondary), base 13
/// (body, rows, menus), md 14 (section titles), lg 16 (dialog titles),
/// xl 20 (the unlock card).
pub mod size {
    pub const XS: f32 = 11.0;
    pub const SM: f32 = 12.0;
    pub const BASE: f32 = 13.0;
    pub const MD: f32 = 14.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 20.0;
}

/// The corner radius scale (px): checkbox, controls / rows / menu items,
/// cards / menus / toasts, dialogs, and pills.
pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 12.0;
    pub const PILL: f32 = 999.0;
}

/// Fixed chrome heights (px).
pub const ROW_H: f32 = 28.0;
/// Side-rail rows of the dialogs.
pub const NAV_ROW_H: f32 = 32.0;
/// Inputs and buttons in dialogs.
pub const CONTROL_H: f32 = 32.0;
/// Inputs and buttons in panes and toolbars.
pub const CONTROL_H_SM: f32 = 28.0;
/// Square icon buttons.
pub const ICON_BTN: f32 = 28.0;
pub const TAB_BAR_H: f32 = 36.0;
pub const PANE_HEAD_H: f32 = 36.0;
pub const STATUS_BAR_H: f32 = 26.0;
/// Inset of selection pills from the edge of the row they sit in.
pub const ROW_INSET: f32 = 6.0;

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

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// The dialog shadow (`0 16 48 @ 50 %`).
pub fn dialog_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 16],
        blur: 48,
        spread: 0,
        color: Color32::from_black_alpha(128),
    }
}

/// The menu / toast / cascade-frame shadow (`0 8 24 @ 40 %`).
pub fn popup_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(100),
    }
}

/// Install the theme's visuals and the type scale into `ctx`.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let t = tokens_for(theme);
    let mut visuals = if theme == Theme::Dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let r = CornerRadius::same(radius::MD as u8);
    visuals.override_text_color = None;
    visuals.panel_fill = t.surface;
    visuals.window_fill = t.surface;
    visuals.window_stroke = Stroke::new(1.0, t.border_strong);
    visuals.window_corner_radius = CornerRadius::same(radius::XL as u8);
    visuals.menu_corner_radius = CornerRadius::same(radius::LG as u8);
    visuals.extreme_bg_color = t.surface2;
    visuals.text_edit_bg_color = Some(t.surface2);
    visuals.faint_bg_color = t.surface2;
    visuals.code_bg_color = t.surface2;
    visuals.hyperlink_color = t.accent;
    visuals.selection.bg_fill = alpha(t.accent, 0.35);
    visuals.selection.stroke = Stroke::new(1.0, t.fg);
    visuals.warn_fg_color = t.warning;
    visuals.error_fg_color = t.danger;
    visuals.window_shadow = dialog_shadow();
    visuals.popup_shadow = popup_shadow();

    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = t.surface;
    w.noninteractive.weak_bg_fill = t.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.fg);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = t.surface2;
    w.inactive.weak_bg_fill = t.surface2;
    w.inactive.bg_stroke = Stroke::new(1.0, t.border_strong);
    w.inactive.fg_stroke = Stroke::new(1.0, t.fg);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = t.surface3;
    w.hovered.weak_bg_fill = t.surface3;
    w.hovered.bg_stroke = Stroke::new(1.0, t.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.0, t.fg);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = over(t.surface2, t.accent, 0.25);
    w.active.weak_bg_fill = over(t.surface2, t.accent, 0.25);
    w.active.bg_stroke = Stroke::new(1.0, t.accent);
    w.active.fg_stroke = Stroke::new(1.0, t.fg);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open.bg_fill = t.surface2;
    w.open.weak_bg_fill = t.surface2;
    w.open.bg_stroke = Stroke::new(1.0, t.accent);
    w.open.fg_stroke = Stroke::new(1.0, t.fg);
    w.open.corner_radius = r;

    ctx.set_visuals_of(
        if theme == Theme::Dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        },
        visuals,
    );
    ctx.set_theme(if theme == Theme::Dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, font(size::XS)),
            (TextStyle::Body, font(size::BASE)),
            (TextStyle::Button, font(size::BASE)),
            (TextStyle::Heading, font_semibold(size::LG)),
            (
                TextStyle::Monospace,
                FontId::new(size::SM, FontFamily::Monospace),
            ),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 6.0);
        s.button_padding = Vec2::new(12.0, 6.0);
        s.interact_size = Vec2::new(ICON_BTN, ICON_BTN);
        s.menu_margin = egui::Margin::same(6);
        s.window_margin = egui::Margin::same(0);
        s.combo_width = 160.0;
        s.text_edit_width = 220.0;
        s.icon_width = 16.0;
        s.icon_width_inner = 10.0;
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

    /// WCAG relative luminance of an opaque colour.
    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (luminance(a) + 0.05, luminance(b) + 0.05);
        if x > y { x / y } else { y / x }
    }

    /// The acceptance rule of the facelift: every text / background pair
    /// at or above 4.5 : 1 in both themes. `muted` is a caption colour for
    /// `bg` and `surface`; on the raised fills (`surface2`, `surface3`)
    /// text is `fg` or `fg2`.
    #[test]
    fn text_contrast_floor() {
        for t in [&DARK, &LIGHT] {
            for bg in [t.bg, t.surface, t.surface2, t.surface3] {
                for fg in [t.fg, t.fg2] {
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "{fg:?} on {bg:?}: {:.2}",
                        contrast(fg, bg)
                    );
                }
            }
            for bg in [t.bg, t.surface] {
                assert!(
                    contrast(t.muted, bg) >= 4.5,
                    "muted on {bg:?}: {:.2}",
                    contrast(t.muted, bg)
                );
            }
            assert!(
                contrast(t.on_accent, t.accent) >= 4.5,
                "primary button {:?}",
                contrast(t.on_accent, t.accent)
            );
            assert!(
                contrast(t.accent, t.surface) >= 4.0,
                "accent text {:?}",
                contrast(t.accent, t.surface)
            );
            assert!(
                contrast(t.danger, t.surface) >= 4.0,
                "danger text {:?}",
                contrast(t.danger, t.surface)
            );
        }
    }
}
