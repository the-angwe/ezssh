//! Terminal colour palettes.
//!
//! Values are copied from `ui/lib/palettes.ts` so the native build shows
//! the same colours as the webview build. The spike carries the two ezTerm
//! palettes; phase 3 ports the remaining entries with their tests.

use alacritty_terminal::vte::ansi::{NamedColor, Rgb};

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub id: &'static str,
    pub label: &'static str,
    pub light: bool,
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    /// Text colour under a block cursor.
    pub cursor_accent: Rgb,
    pub selection_background: Rgb,
    /// 0–7 normal, 8–15 bright.
    pub ansi: [Rgb; 16],
}

const fn hex(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

pub const EZSSH_DARK: Palette = Palette {
    id: "ezterm-dark",
    label: "ezSsh Dark (default)",
    light: false,
    background: hex(0x121214),
    foreground: hex(0xe5e7eb),
    cursor: hex(0xe5e7eb),
    cursor_accent: hex(0x121214),
    selection_background: hex(0x2d3748),
    ansi: [
        hex(0x2d3748), hex(0xf87171), hex(0x34d399), hex(0xfbbf24),
        hex(0x60a5fa), hex(0xa78bfa), hex(0x22d3ee), hex(0xe5e7eb),
        hex(0x475569), hex(0xfca5a5), hex(0x6ee7b7), hex(0xfcd34d),
        hex(0x93c5fd), hex(0xc4b5fd), hex(0x67e8f9), hex(0xf3f4f6),
    ],
};

impl Default for Palette {
    fn default() -> Self {
        EZSSH_DARK
    }
}

impl Palette {
    /// Colour for one of the 256 indexed colours: 0–15 from the palette,
    /// 16–231 the xterm 6×6×6 cube, 232–255 the grey ramp.
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let i = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb { r: level(i / 36), g: level((i / 6) % 6), b: level(i % 6) }
            }
            232..=255 => {
                let v = 8 + (index - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    /// Colour for a named colour. Dim variants are the normal colour at
    /// two thirds brightness, as Alacritty does when none is configured.
    pub fn named(&self, color: NamedColor) -> Rgb {
        let n = color as usize;
        match color {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            _ if n < 16 => self.ansi[n],
            // DimBlack .. DimWhite are 259..=266.
            _ if (259..=266).contains(&n) => dim(self.ansi[n - 259]),
            _ => self.foreground,
        }
    }

    /// Colour for an alacritty colour index (0–268), used to answer
    /// OSC 4 / 10 / 11 / 12 queries.
    pub fn by_index(&self, index: usize) -> Rgb {
        match index {
            0..=255 => self.indexed(index as u8),
            256 => self.foreground,
            257 => self.background,
            258 => self.cursor,
            259..=266 => dim(self.ansi[index - 259]),
            267 => self.foreground,
            _ => dim(self.foreground),
        }
    }
}

pub fn dim(c: Rgb) -> Rgb {
    Rgb { r: (c.r as u16 * 2 / 3) as u8, g: (c.g as u16 * 2 / 3) as u8, b: (c.b as u16 * 2 / 3) as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_ramp_match_xterm() {
        let p = Palette::default();
        assert_eq!(p.indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(p.indexed(21), Rgb { r: 0, g: 0, b: 255 });
        assert_eq!(p.indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(p.indexed(231), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(p.indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(p.indexed(255), Rgb { r: 238, g: 238, b: 238 });
    }

    #[test]
    fn named_maps_to_ansi_and_dim() {
        let p = Palette::default();
        assert_eq!(p.named(NamedColor::Red), p.ansi[1]);
        assert_eq!(p.named(NamedColor::BrightWhite), p.ansi[15]);
        assert_eq!(p.named(NamedColor::DimRed), dim(p.ansi[1]));
        assert_eq!(p.named(NamedColor::Background), p.background);
    }
}
