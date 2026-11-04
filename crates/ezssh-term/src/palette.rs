//! Terminal colour palettes.
//!
//! 17 built-in palettes (#176). `ezterm-dark` is the palette the app
//! shipped with before palettes were selectable. The palette does not
//! follow the window theme; it is its own setting. Ids are the stored
//! `terminal.palette` values (the ezTerm app wrote the same ones), so they
//! keep their `ezterm-` prefix; only the labels say ezSsh.

use alacritty_terminal::vte::ansi::NamedColor;
pub use alacritty_terminal::vte::ansi::Rgb;

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub id: &'static str,
    /// Shown in the Settings select.
    pub label: &'static str,
    /// Light-background palette.
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
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

pub const DEFAULT_PALETTE: &str = "ezterm-dark";

pub const EZTERM_DARK: Palette = Palette {
    id: "ezterm-dark",
    label: "ezSsh Dark (default)",
    light: false,
    background: hex(0x121214),
    foreground: hex(0xe5e7eb),
    cursor: hex(0xe5e7eb),
    cursor_accent: hex(0x121214),
    selection_background: hex(0x2d3748),
    ansi: [
        hex(0x2d3748),
        hex(0xf87171),
        hex(0x34d399),
        hex(0xfbbf24),
        hex(0x60a5fa),
        hex(0xa78bfa),
        hex(0x22d3ee),
        hex(0xe5e7eb),
        hex(0x475569),
        hex(0xfca5a5),
        hex(0x6ee7b7),
        hex(0xfcd34d),
        hex(0x93c5fd),
        hex(0xc4b5fd),
        hex(0x67e8f9),
        hex(0xf3f4f6),
    ],
};

pub const EZTERM_LIGHT: Palette = Palette {
    id: "ezterm-light",
    label: "ezSsh Light",
    light: true,
    background: hex(0xf8f8f8),
    foreground: hex(0x1a1a1a),
    cursor: hex(0x1a1a1a),
    cursor_accent: hex(0xf8f8f8),
    selection_background: hex(0xb0cef7),
    ansi: [
        hex(0x0c0c0c),
        hex(0xa4251f),
        hex(0x237a3f),
        hex(0x8a7010),
        hex(0x1e6ecd),
        hex(0x8a389a),
        hex(0x1f7f97),
        hex(0x555555),
        hex(0x676767),
        hex(0xc23127),
        hex(0x31a354),
        hex(0xb08a1a),
        hex(0x3b78ff),
        hex(0xb148c6),
        hex(0x3aa4c6),
        hex(0x1a1a1a),
    ],
};

pub const CAMPBELL: Palette = Palette {
    id: "campbell",
    label: "Campbell (Windows Terminal)",
    light: false,
    background: hex(0x0c0c0c),
    foreground: hex(0xcccccc),
    cursor: hex(0xcccccc),
    cursor_accent: hex(0x0c0c0c),
    selection_background: hex(0x3a4b6b),
    ansi: [
        hex(0x0c0c0c),
        hex(0xc50f1f),
        hex(0x13a10e),
        hex(0xc19c00),
        hex(0x0037da),
        hex(0x881798),
        hex(0x3a96dd),
        hex(0xcccccc),
        hex(0x767676),
        hex(0xe74856),
        hex(0x16c60c),
        hex(0xf9f1a5),
        hex(0x3b78ff),
        hex(0xb4009e),
        hex(0x61d6d6),
        hex(0xf2f2f2),
    ],
};

pub const SOLARIZED_DARK: Palette = Palette {
    id: "solarized-dark",
    label: "Solarized Dark",
    light: false,
    background: hex(0x002b36),
    foreground: hex(0x839496),
    cursor: hex(0x839496),
    cursor_accent: hex(0x002b36),
    selection_background: hex(0x073642),
    ansi: [
        hex(0x073642),
        hex(0xdc322f),
        hex(0x859900),
        hex(0xb58900),
        hex(0x268bd2),
        hex(0xd33682),
        hex(0x2aa198),
        hex(0xeee8d5),
        hex(0x002b36),
        hex(0xcb4b16),
        hex(0x586e75),
        hex(0x657b83),
        hex(0x839496),
        hex(0x6c71c4),
        hex(0x93a1a1),
        hex(0xfdf6e3),
    ],
};

pub const SOLARIZED_LIGHT: Palette = Palette {
    id: "solarized-light",
    label: "Solarized Light",
    light: true,
    background: hex(0xfdf6e3),
    foreground: hex(0x657b83),
    cursor: hex(0x657b83),
    cursor_accent: hex(0xfdf6e3),
    selection_background: hex(0xeee8d5),
    ansi: [
        hex(0x073642),
        hex(0xdc322f),
        hex(0x859900),
        hex(0xb58900),
        hex(0x268bd2),
        hex(0xd33682),
        hex(0x2aa198),
        hex(0xeee8d5),
        hex(0x002b36),
        hex(0xcb4b16),
        hex(0x586e75),
        hex(0x657b83),
        hex(0x839496),
        hex(0x6c71c4),
        hex(0x93a1a1),
        hex(0xfdf6e3),
    ],
};

pub const DRACULA: Palette = Palette {
    id: "dracula",
    label: "Dracula",
    light: false,
    background: hex(0x282a36),
    foreground: hex(0xf8f8f2),
    cursor: hex(0xf8f8f2),
    cursor_accent: hex(0x282a36),
    selection_background: hex(0x44475a),
    ansi: [
        hex(0x21222c),
        hex(0xff5555),
        hex(0x50fa7b),
        hex(0xf1fa8c),
        hex(0xbd93f9),
        hex(0xff79c6),
        hex(0x8be9fd),
        hex(0xf8f8f2),
        hex(0x6272a4),
        hex(0xff6e6e),
        hex(0x69ff94),
        hex(0xffffa5),
        hex(0xd6acff),
        hex(0xff92df),
        hex(0xa4ffff),
        hex(0xffffff),
    ],
};

pub const GRUVBOX_DARK: Palette = Palette {
    id: "gruvbox-dark",
    label: "Gruvbox Dark",
    light: false,
    background: hex(0x282828),
    foreground: hex(0xebdbb2),
    cursor: hex(0xebdbb2),
    cursor_accent: hex(0x282828),
    selection_background: hex(0x504945),
    ansi: [
        hex(0x282828),
        hex(0xcc241d),
        hex(0x98971a),
        hex(0xd79921),
        hex(0x458588),
        hex(0xb16286),
        hex(0x689d6a),
        hex(0xa89984),
        hex(0x928374),
        hex(0xfb4934),
        hex(0xb8bb26),
        hex(0xfabd2f),
        hex(0x83a598),
        hex(0xd3869b),
        hex(0x8ec07c),
        hex(0xebdbb2),
    ],
};

pub const NORD: Palette = Palette {
    id: "nord",
    label: "Nord",
    light: false,
    background: hex(0x2e3440),
    foreground: hex(0xd8dee9),
    cursor: hex(0xd8dee9),
    cursor_accent: hex(0x2e3440),
    selection_background: hex(0x434c5e),
    ansi: [
        hex(0x3b4252),
        hex(0xbf616a),
        hex(0xa3be8c),
        hex(0xebcb8b),
        hex(0x81a1c1),
        hex(0xb48ead),
        hex(0x88c0d0),
        hex(0xe5e9f0),
        hex(0x4c566a),
        hex(0xbf616a),
        hex(0xa3be8c),
        hex(0xebcb8b),
        hex(0x81a1c1),
        hex(0xb48ead),
        hex(0x8fbcbb),
        hex(0xeceff4),
    ],
};

pub const ONE_DARK: Palette = Palette {
    id: "one-dark",
    label: "One Dark",
    light: false,
    background: hex(0x282c34),
    foreground: hex(0xabb2bf),
    cursor: hex(0x528bff),
    cursor_accent: hex(0x282c34),
    selection_background: hex(0x3e4451),
    ansi: [
        hex(0x282c34),
        hex(0xe06c75),
        hex(0x98c379),
        hex(0xe5c07b),
        hex(0x61afef),
        hex(0xc678dd),
        hex(0x56b6c2),
        hex(0xabb2bf),
        hex(0x5c6370),
        hex(0xe06c75),
        hex(0x98c379),
        hex(0xe5c07b),
        hex(0x61afef),
        hex(0xc678dd),
        hex(0x56b6c2),
        hex(0xffffff),
    ],
};

pub const MONOKAI: Palette = Palette {
    id: "monokai",
    label: "Monokai",
    light: false,
    background: hex(0x272822),
    foreground: hex(0xf8f8f2),
    cursor: hex(0xf8f8f0),
    cursor_accent: hex(0x272822),
    selection_background: hex(0x49483e),
    ansi: [
        hex(0x272822),
        hex(0xf92672),
        hex(0xa6e22e),
        hex(0xf4bf75),
        hex(0x66d9ef),
        hex(0xae81ff),
        hex(0xa1efe4),
        hex(0xf8f8f2),
        hex(0x75715e),
        hex(0xf92672),
        hex(0xa6e22e),
        hex(0xf4bf75),
        hex(0x66d9ef),
        hex(0xae81ff),
        hex(0xa1efe4),
        hex(0xf9f8f5),
    ],
};

pub const TANGO: Palette = Palette {
    id: "tango",
    label: "Tango",
    light: false,
    background: hex(0x2e3436),
    foreground: hex(0xd3d7cf),
    cursor: hex(0xd3d7cf),
    cursor_accent: hex(0x2e3436),
    selection_background: hex(0x555753),
    ansi: [
        hex(0x2e3436),
        hex(0xcc0000),
        hex(0x4e9a06),
        hex(0xc4a000),
        hex(0x3465a4),
        hex(0x75507b),
        hex(0x06989a),
        hex(0xd3d7cf),
        hex(0x555753),
        hex(0xef2929),
        hex(0x8ae234),
        hex(0xfce94f),
        hex(0x729fcf),
        hex(0xad7fa8),
        hex(0x34e2e2),
        hex(0xeeeeec),
    ],
};

pub const GNOME_DARK: Palette = Palette {
    id: "gnome-dark",
    label: "GNOME Dark",
    light: false,
    background: hex(0x1c1c1f),
    foreground: hex(0xffffff),
    cursor: hex(0xffffff),
    cursor_accent: hex(0x1c1c1f),
    selection_background: hex(0x3d3d42),
    ansi: [
        hex(0x241f31),
        hex(0xc01c28),
        hex(0x2ec27e),
        hex(0xf5c211),
        hex(0x1e78e4),
        hex(0x9841bb),
        hex(0x0ab9dc),
        hex(0xc0bfbc),
        hex(0x5e5c64),
        hex(0xed333b),
        hex(0x57e389),
        hex(0xf8e45c),
        hex(0x51a1ff),
        hex(0xc061cb),
        hex(0x4fd2fd),
        hex(0xf6f5f4),
    ],
};

pub const GNOME_LIGHT: Palette = Palette {
    id: "gnome-light",
    label: "GNOME Light",
    light: true,
    background: hex(0xffffff),
    foreground: hex(0x1d1d20),
    cursor: hex(0x1d1d20),
    cursor_accent: hex(0xffffff),
    selection_background: hex(0xbfd4f0),
    ansi: [
        hex(0x1d1d20),
        hex(0xc01c28),
        hex(0x26a269),
        hex(0xa2734c),
        hex(0x12488b),
        hex(0xa347ba),
        hex(0x2aa1b3),
        hex(0xcfcfcf),
        hex(0x5d5d5d),
        hex(0xf66151),
        hex(0x33d17a),
        hex(0xe9ad0c),
        hex(0x2a7bde),
        hex(0xc061cb),
        hex(0x33c7de),
        hex(0xffffff),
    ],
};

pub const GITHUB_DARK: Palette = Palette {
    id: "github-dark",
    label: "GitHub Dark",
    light: false,
    background: hex(0x101216),
    foreground: hex(0x8b949e),
    cursor: hex(0xc9d1d9),
    cursor_accent: hex(0x101216),
    selection_background: hex(0x263c5a),
    ansi: [
        hex(0x000000),
        hex(0xf78166),
        hex(0x56d364),
        hex(0xe3b341),
        hex(0x6ca4f8),
        hex(0xdb61a2),
        hex(0x2b7489),
        hex(0xffffff),
        hex(0x4d4d4d),
        hex(0xf78166),
        hex(0x56d364),
        hex(0xe3b341),
        hex(0x6ca4f8),
        hex(0xdb61a2),
        hex(0x2b7489),
        hex(0xffffff),
    ],
};

pub const GITHUB_LIGHT: Palette = Palette {
    id: "github-light",
    label: "GitHub Light",
    light: true,
    background: hex(0xf4f4f4),
    foreground: hex(0x3e3e3e),
    cursor: hex(0x3e3e3e),
    cursor_accent: hex(0xf4f4f4),
    selection_background: hex(0xc8e1ff),
    ansi: [
        hex(0x3e3e3e),
        hex(0x970b16),
        hex(0x07962a),
        hex(0xf8eec7),
        hex(0x003e8a),
        hex(0xe94691),
        hex(0x89d1ec),
        hex(0xffffff),
        hex(0x666666),
        hex(0xde0000),
        hex(0x87d5a2),
        hex(0xf1d007),
        hex(0x2e6cba),
        hex(0xffa29f),
        hex(0x1cfafe),
        hex(0xffffff),
    ],
};

pub const GOOGLE_DARK: Palette = Palette {
    id: "google-dark",
    label: "Google Dark",
    light: false,
    background: hex(0x202124),
    foreground: hex(0xe8eaed),
    cursor: hex(0xe8eaed),
    cursor_accent: hex(0x202124),
    selection_background: hex(0x3c4043),
    ansi: [
        hex(0x202124),
        hex(0xea4335),
        hex(0x34a853),
        hex(0xfbbc04),
        hex(0x4285f4),
        hex(0xa142f4),
        hex(0x24c1e0),
        hex(0xe8eaed),
        hex(0x5f6368),
        hex(0xea4335),
        hex(0x34a853),
        hex(0xfbbc05),
        hex(0x4285f4),
        hex(0xa142f4),
        hex(0x24c1e0),
        hex(0xffffff),
    ],
};

pub const GOOGLE_LIGHT: Palette = Palette {
    id: "google-light",
    label: "Google Light",
    light: true,
    background: hex(0xffffff),
    foreground: hex(0x5f6368),
    cursor: hex(0x5f6368),
    cursor_accent: hex(0xffffff),
    selection_background: hex(0xd2e3fc),
    ansi: [
        hex(0x202124),
        hex(0xea4335),
        hex(0x34a853),
        hex(0xfbbc04),
        hex(0x4285f4),
        hex(0xa142f4),
        hex(0x24c1e0),
        hex(0xe8eaed),
        hex(0x5f6368),
        hex(0xea4335),
        hex(0x34a853),
        hex(0xfbbc05),
        hex(0x4285f4),
        hex(0xa142f4),
        hex(0x24c1e0),
        hex(0xffffff),
    ],
};

pub const PALETTES: &[Palette] = &[
    EZTERM_DARK,
    EZTERM_LIGHT,
    CAMPBELL,
    SOLARIZED_DARK,
    SOLARIZED_LIGHT,
    DRACULA,
    GRUVBOX_DARK,
    NORD,
    ONE_DARK,
    MONOKAI,
    TANGO,
    GNOME_DARK,
    GNOME_LIGHT,
    GITHUB_DARK,
    GITHUB_LIGHT,
    GOOGLE_DARK,
    GOOGLE_LIGHT,
];

impl Default for Palette {
    fn default() -> Self {
        EZTERM_DARK
    }
}

/// Palette for a stored `terminal.palette` value; unknown ids fall back to
/// the default.
pub fn by_id(id: &str) -> &'static Palette {
    PALETTES.iter().find(|p| p.id == id).unwrap_or_else(|| {
        tracing::debug!(id, "term.palette: unknown id, using the default");
        &PALETTES[0]
    })
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
                Rgb {
                    r: level(i / 36),
                    g: level((i / 6) % 6),
                    b: level(i % 6),
                }
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
    Rgb {
        r: (c.r as u16 * 2 / 3) as u8,
        g: (c.g as u16 * 2 / 3) as u8,
        b: (c.b as u16 * 2 / 3) as u8,
    }
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
        assert_eq!(
            p.indexed(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(p.indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(
            p.indexed(255),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
    }

    #[test]
    fn ids_and_labels_are_unique() {
        let mut ids: Vec<_> = PALETTES.iter().map(|p| p.id).collect();
        let mut labels: Vec<_> = PALETTES.iter().map(|p| p.label).collect();
        ids.sort();
        ids.dedup();
        labels.sort();
        labels.dedup();
        assert_eq!(ids.len(), PALETTES.len());
        assert_eq!(labels.len(), PALETTES.len());
        assert_eq!(PALETTES.len(), 17);
        assert_eq!(PALETTES[0].id, DEFAULT_PALETTE);
    }

    #[test]
    fn default_is_the_pre_176_palette() {
        let p = by_id(DEFAULT_PALETTE);
        assert_eq!(p.background, hex(0x121214));
        assert_eq!(p.foreground, hex(0xe5e7eb));
        assert_eq!(p.ansi[1], hex(0xf87171));
        assert_eq!(p.ansi[15], hex(0xf3f4f6));
    }

    #[test]
    fn ptyxis_spot_checks() {
        assert_eq!(by_id("gnome-dark").ansi[2], hex(0x2ec27e));
        assert_eq!(by_id("gnome-light").ansi[11], hex(0xe9ad0c));
        assert_eq!(by_id("github-dark").cursor, hex(0xc9d1d9));
        assert_eq!(by_id("github-light").ansi[4], hex(0x003e8a));
        assert_eq!(by_id("google-dark").ansi[11], hex(0xfbbc05));
        assert_eq!(by_id("google-light").foreground, hex(0x5f6368));
        for base in ["gnome", "github", "google"] {
            assert!(!by_id(&format!("{base}-dark")).light);
            assert!(by_id(&format!("{base}-light")).light);
        }
    }

    #[test]
    fn unknown_ids_fall_back_to_the_default() {
        assert_eq!(by_id("dracula").id, "dracula");
        assert_eq!(by_id("no-such-palette").id, DEFAULT_PALETTE);
        assert_eq!(by_id("").id, DEFAULT_PALETTE);
        assert_eq!(by_id("constructor").id, DEFAULT_PALETTE);
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
