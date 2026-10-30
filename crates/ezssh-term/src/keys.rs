//! Key-to-bytes encoding: xterm legacy sequences and the kitty keyboard
//! protocol (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>).
//!
//! The plan named `termwiz::KeyCode::encode` for this, but termwiz 0.23
//! only implements the xterm and CSI-u encodings; its `Kitty` variant is
//! accepted and then ignored. Kitty support is what Claude Code, neovim
//! and fish rely on (Shift+Enter, Ctrl+I vs Tab), so the encoder lives here.
//!
//! The encoder is independent of egui so it can be unit-tested; the view
//! maps `egui::Key` into [`Key`] with [`Key::from_egui`].

use alacritty_terminal::term::TermMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A key that types a character. Holds the unshifted character
    /// (`'a'` for the A key, `'1'` for the 1 key).
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 = `F(1)` .. F35.
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, alt: false, ctrl: false, super_: false };

    pub fn from_egui(m: egui::Modifiers) -> Self {
        Mods { shift: m.shift, alt: m.alt, ctrl: m.ctrl, super_: m.mac_cmd }
    }

    /// xterm / kitty modifier parameter: 1 + bit set.
    fn param(self) -> u32 {
        1 + self.shift as u32 + 2 * self.alt as u32 + 4 * self.ctrl as u32 + 8 * self.super_ as u32
    }

    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl || self.super_
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press,
    Repeat,
    Release,
}

/// Terminal modes that change key encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub app_cursor: bool,
    pub newline_mode: bool,
    /// Kitty progressive-enhancement flags (bit 0 disambiguate, 1 event
    /// types, 2 alternate keys, 3 all keys as escapes, 4 associated text).
    pub kitty: u8,
}

pub const KITTY_DISAMBIGUATE: u8 = 1;
pub const KITTY_EVENT_TYPES: u8 = 2;
pub const KITTY_ALTERNATE: u8 = 4;
pub const KITTY_ALL_AS_ESC: u8 = 8;
pub const KITTY_ASSOC_TEXT: u8 = 16;

impl Modes {
    pub fn from_term(mode: TermMode) -> Self {
        let mut kitty = 0;
        for (flag, bit) in [
            (TermMode::DISAMBIGUATE_ESC_CODES, KITTY_DISAMBIGUATE),
            (TermMode::REPORT_EVENT_TYPES, KITTY_EVENT_TYPES),
            (TermMode::REPORT_ALTERNATE_KEYS, KITTY_ALTERNATE),
            (TermMode::REPORT_ALL_KEYS_AS_ESC, KITTY_ALL_AS_ESC),
            (TermMode::REPORT_ASSOCIATED_TEXT, KITTY_ASSOC_TEXT),
        ] {
            if mode.contains(flag) {
                kitty |= bit;
            }
        }
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            newline_mode: mode.contains(TermMode::LINE_FEED_NEW_LINE),
            kitty,
        }
    }
}

impl Key {
    pub fn from_egui(key: egui::Key) -> Option<Key> {
        use egui::Key as E;
        Some(match key {
            E::Enter => Key::Enter,
            E::Tab => Key::Tab,
            E::Backspace => Key::Backspace,
            E::Escape => Key::Escape,
            E::ArrowUp => Key::Up,
            E::ArrowDown => Key::Down,
            E::ArrowLeft => Key::Left,
            E::ArrowRight => Key::Right,
            E::Home => Key::Home,
            E::End => Key::End,
            E::PageUp => Key::PageUp,
            E::PageDown => Key::PageDown,
            E::Insert => Key::Insert,
            E::Delete => Key::Delete,
            E::Space => Key::Char(' '),
            E::F1 => Key::F(1),
            E::F2 => Key::F(2),
            E::F3 => Key::F(3),
            E::F4 => Key::F(4),
            E::F5 => Key::F(5),
            E::F6 => Key::F(6),
            E::F7 => Key::F(7),
            E::F8 => Key::F(8),
            E::F9 => Key::F(9),
            E::F10 => Key::F(10),
            E::F11 => Key::F(11),
            E::F12 => Key::F(12),
            E::F13 => Key::F(13),
            E::F14 => Key::F(14),
            E::F15 => Key::F(15),
            E::F16 => Key::F(16),
            E::F17 => Key::F(17),
            E::F18 => Key::F(18),
            E::F19 => Key::F(19),
            E::F20 => Key::F(20),
            other => {
                // Letters, digits and punctuation: egui's symbol for the
                // key is the unshifted character for the Latin layout.
                let sym = other.symbol_or_name();
                let mut chars = sym.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                Key::Char(c.to_ascii_lowercase())
            }
        })
    }

    /// True for keys that type text when pressed without Ctrl / Alt /
    /// Super. Their bytes come from the text event, not the key event.
    pub fn is_text(self) -> bool {
        matches!(self, Key::Char(_))
    }
}

/// Encode a key event. `text` is the text the key typed (with Shift and
/// the keyboard layout applied), if known. Returns `None` when the event
/// sends nothing (releases outside kitty event-type mode, text keys that
/// the text event will deliver).
pub fn encode(key: Key, mods: Mods, action: Action, text: Option<&str>, modes: Modes) -> Option<Vec<u8>> {
    if modes.kitty != 0 {
        return encode_kitty(key, mods, action, text, modes);
    }
    if action == Action::Release {
        return None;
    }
    encode_legacy(key, mods, text, modes)
}

fn csi(s: &str) -> Vec<u8> {
    let mut v = b"\x1b[".to_vec();
    v.extend_from_slice(s.as_bytes());
    v
}

/// Letter-terminated cursor / F1–F4 keys: (final byte, uses SS3 when
/// unmodified in application mode or for F1–F4).
fn letter_key(key: Key) -> Option<(char, bool)> {
    Some(match key {
        Key::Up => ('A', false),
        Key::Down => ('B', false),
        Key::Right => ('C', false),
        Key::Left => ('D', false),
        Key::Home => ('H', false),
        Key::End => ('F', false),
        Key::F(1) => ('P', true),
        Key::F(2) => ('Q', true),
        Key::F(4) => ('S', true),
        _ => return None,
    })
}

/// Tilde-terminated keys: `CSI n ~`.
fn tilde_key(key: Key, kitty: bool) -> Option<u32> {
    Some(match key {
        Key::Insert => 2,
        Key::Delete => 3,
        Key::PageUp => 5,
        Key::PageDown => 6,
        // F3 is `CSI 13 ~` in kitty (SS3 R collides with a cursor report).
        Key::F(3) if kitty => 13,
        Key::F(5) => 15,
        Key::F(6) => 17,
        Key::F(7) => 18,
        Key::F(8) => 19,
        Key::F(9) => 20,
        Key::F(10) => 21,
        Key::F(11) => 23,
        Key::F(12) => 24,
        _ => return None,
    })
}

/// Control byte for Ctrl + key in the legacy encoding.
fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' | '/' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

fn encode_legacy(key: Key, mods: Mods, text: Option<&str>, modes: Modes) -> Option<Vec<u8>> {
    let m = mods.param();
    let esc_prefix = |mut v: Vec<u8>| {
        if mods.alt {
            v.insert(0, 0x1b);
        }
        v
    };
    match key {
        Key::Char(c) => {
            if mods.ctrl {
                let b = ctrl_byte(c)?;
                return Some(esc_prefix(vec![b]));
            }
            if mods.alt {
                let s = match text {
                    Some(t) if !t.is_empty() => t.to_string(),
                    _ if mods.shift => c.to_uppercase().collect(),
                    _ => c.to_string(),
                };
                return Some(esc_prefix(s.into_bytes()));
            }
            // Plain or shifted text: delivered by the text event.
            None
        }
        Key::Enter => Some(esc_prefix(if modes.newline_mode { b"\r\n".to_vec() } else { b"\r".to_vec() })),
        Key::Tab if mods.shift => Some(csi("Z")),
        Key::Tab => Some(esc_prefix(b"\t".to_vec())),
        Key::Backspace if mods.ctrl => Some(esc_prefix(vec![0x08])),
        Key::Backspace => Some(esc_prefix(vec![0x7f])),
        Key::Escape => Some(esc_prefix(vec![0x1b])),
        _ => {
            if let Some((fin, ss3)) = letter_key(key) {
                return Some(if mods.any() {
                    csi(&format!("1;{m}{fin}"))
                } else if ss3 || modes.app_cursor {
                    format!("\x1bO{fin}").into_bytes()
                } else {
                    csi(&fin.to_string())
                });
            }
            if key == Key::F(3) {
                return Some(if mods.any() { csi(&format!("1;{m}R")) } else { b"\x1bOR".to_vec() });
            }
            if let Some(n) = tilde_key(key, false) {
                return Some(if mods.any() { csi(&format!("{n};{m}~")) } else { csi(&format!("{n}~")) });
            }
            if let Key::F(n @ 13..=20) = key {
                // xterm: F13–F20 are Shift+F1–F8 style codes; send the
                // kitty-compatible private-use form, which xterm ignores.
                return Some(csi(&format!("{}u", 57376 + (n as u32 - 13))));
            }
            None
        }
    }
}

fn encode_kitty(key: Key, mods: Mods, action: Action, text: Option<&str>, modes: Modes) -> Option<Vec<u8>> {
    let flags = modes.kitty;
    let all_as_esc = flags & KITTY_ALL_AS_ESC != 0;
    let event_types = flags & KITTY_EVENT_TYPES != 0;
    if action == Action::Release && !event_types {
        return None;
    }

    let text_mods = mods.ctrl || mods.alt || mods.super_;
    if !all_as_esc {
        // Text keys without Ctrl / Alt / Super still type text.
        if key.is_text() && !text_mods {
            return None;
        }
        // Enter, Tab and Backspace stay legacy when unmodified; their
        // releases are not reported (kitty spec, "Event types").
        if matches!(key, Key::Enter | Key::Tab | Key::Backspace) && !mods.any() {
            if action == Action::Release {
                return None;
            }
            return encode_legacy(key, mods, text, modes);
        }
    }

    let event = match action {
        Action::Press => 1,
        Action::Repeat => 2,
        Action::Release => 3,
    };
    let m = mods.param();
    let mods_field = |force: bool| -> String {
        if event_types && event != 1 {
            format!("{m}:{event}")
        } else if m != 1 || force {
            m.to_string()
        } else {
            String::new()
        }
    };

    if let Some((fin, _)) = letter_key(key) {
        let mf = mods_field(false);
        if mf.is_empty() {
            // Unmodified press: legacy form, honouring application cursor mode.
            return encode_legacy(key, mods, text, modes);
        }
        return Some(csi(&format!("1;{mf}{fin}")));
    }
    if let Some(n) = tilde_key(key, true) {
        let mf = mods_field(false);
        return Some(if mf.is_empty() { csi(&format!("{n}~")) } else { csi(&format!("{n};{mf}~")) });
    }

    let code: u32 = match key {
        Key::Char(c) => c as u32,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        Key::Escape => 27,
        Key::F(n @ 13..=35) => 57376 + (n as u32 - 13),
        _ => return None,
    };

    let mut first = code.to_string();
    if flags & KITTY_ALTERNATE != 0
        && mods.shift
        && let Key::Char(c) = key
    {
        let shifted: String = match text {
            Some(t) if t.chars().count() == 1 => t.to_string(),
            _ => c.to_uppercase().collect(),
        };
        if let Some(s) = shifted.chars().next()
            && s != c
        {
            first.push_str(&format!(":{}", s as u32));
        }
    }

    let assoc = if flags & KITTY_ASSOC_TEXT != 0 && all_as_esc && action != Action::Release {
        text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
            .map(|t| t.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(":"))
    } else {
        None
    };

    let mf = mods_field(assoc.is_some());
    let mut s = first;
    if !mf.is_empty() {
        s.push(';');
        s.push_str(&mf);
    }
    if let Some(a) = assoc {
        s.push(';');
        s.push_str(&a);
    }
    s.push('u');
    Some(csi(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(key: Key, mods: Mods) -> Option<Vec<u8>> {
        encode(key, mods, Action::Press, None, Modes::default())
    }

    fn kitty(key: Key, mods: Mods, flags: u8) -> Option<Vec<u8>> {
        encode(key, mods, Action::Press, None, Modes { kitty: flags, ..Default::default() })
    }

    const CTRL: Mods = Mods { shift: false, alt: false, ctrl: true, super_: false };
    const ALT: Mods = Mods { shift: false, alt: true, ctrl: false, super_: false };
    const SHIFT: Mods = Mods { shift: true, alt: false, ctrl: false, super_: false };

    #[test]
    fn legacy_control_bytes() {
        assert_eq!(legacy(Key::Char('c'), CTRL), Some(vec![3]));
        assert_eq!(legacy(Key::Char('a'), CTRL), Some(vec![1]));
        assert_eq!(legacy(Key::Char(' '), CTRL), Some(vec![0]));
        assert_eq!(legacy(Key::Char('['), CTRL), Some(vec![0x1b]));
        assert_eq!(legacy(Key::Char('x'), Mods { alt: true, ..CTRL }), Some(vec![0x1b, 0x18]));
    }

    #[test]
    fn legacy_plain_text_is_left_to_text_event() {
        assert_eq!(legacy(Key::Char('a'), Mods::NONE), None);
        assert_eq!(legacy(Key::Char('a'), SHIFT), None);
    }

    #[test]
    fn legacy_alt_prefixes_escape() {
        assert_eq!(legacy(Key::Char('b'), ALT), Some(b"\x1bb".to_vec()));
        assert_eq!(legacy(Key::Backspace, ALT), Some(b"\x1b\x7f".to_vec()));
    }

    #[test]
    fn legacy_cursor_keys_and_app_mode() {
        assert_eq!(legacy(Key::Up, Mods::NONE), Some(b"\x1b[A".to_vec()));
        let app = Modes { app_cursor: true, ..Default::default() };
        assert_eq!(encode(Key::Up, Mods::NONE, Action::Press, None, app), Some(b"\x1bOA".to_vec()));
        assert_eq!(legacy(Key::Right, CTRL), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(legacy(Key::Home, SHIFT), Some(b"\x1b[1;2H".to_vec()));
    }

    #[test]
    fn legacy_function_and_tilde_keys() {
        assert_eq!(legacy(Key::F(1), Mods::NONE), Some(b"\x1bOP".to_vec()));
        assert_eq!(legacy(Key::F(3), Mods::NONE), Some(b"\x1bOR".to_vec()));
        assert_eq!(legacy(Key::F(5), Mods::NONE), Some(b"\x1b[15~".to_vec()));
        assert_eq!(legacy(Key::F(12), CTRL), Some(b"\x1b[24;5~".to_vec()));
        assert_eq!(legacy(Key::Delete, Mods::NONE), Some(b"\x1b[3~".to_vec()));
        assert_eq!(legacy(Key::PageUp, SHIFT), Some(b"\x1b[5;2~".to_vec()));
    }

    #[test]
    fn legacy_enter_tab_backspace() {
        assert_eq!(legacy(Key::Enter, Mods::NONE), Some(b"\r".to_vec()));
        assert_eq!(legacy(Key::Tab, Mods::NONE), Some(b"\t".to_vec()));
        assert_eq!(legacy(Key::Tab, SHIFT), Some(b"\x1b[Z".to_vec()));
        assert_eq!(legacy(Key::Backspace, Mods::NONE), Some(vec![0x7f]));
        assert_eq!(legacy(Key::Backspace, CTRL), Some(vec![0x08]));
        let nl = Modes { newline_mode: true, ..Default::default() };
        assert_eq!(encode(Key::Enter, Mods::NONE, Action::Press, None, nl), Some(b"\r\n".to_vec()));
    }

    #[test]
    fn legacy_ignores_release() {
        assert_eq!(encode(Key::Up, Mods::NONE, Action::Release, None, Modes::default()), None);
    }

    #[test]
    fn kitty_disambiguate() {
        let d = KITTY_DISAMBIGUATE;
        // Shift+Enter is the reason Claude Code asks for the protocol.
        assert_eq!(kitty(Key::Enter, SHIFT, d), Some(b"\x1b[13;2u".to_vec()));
        assert_eq!(kitty(Key::Enter, Mods::NONE, d), Some(b"\r".to_vec()));
        assert_eq!(kitty(Key::Escape, Mods::NONE, d), Some(b"\x1b[27u".to_vec()));
        assert_eq!(kitty(Key::Char('c'), CTRL, d), Some(b"\x1b[99;5u".to_vec()));
        assert_eq!(kitty(Key::Char('i'), CTRL, d), Some(b"\x1b[105;5u".to_vec()));
        assert_eq!(kitty(Key::Char('a'), Mods::NONE, d), None);
        assert_eq!(kitty(Key::Char('a'), SHIFT, d), None);
        assert_eq!(kitty(Key::Tab, SHIFT, d), Some(b"\x1b[9;2u".to_vec()));
        assert_eq!(kitty(Key::Up, Mods::NONE, d), Some(b"\x1b[A".to_vec()));
        assert_eq!(kitty(Key::Up, CTRL, d), Some(b"\x1b[1;5A".to_vec()));
        assert_eq!(kitty(Key::F(3), Mods::NONE, d), Some(b"\x1b[13~".to_vec()));
        assert_eq!(kitty(Key::Delete, ALT, d), Some(b"\x1b[3;3~".to_vec()));
    }

    #[test]
    fn kitty_event_types() {
        let f = KITTY_DISAMBIGUATE | KITTY_EVENT_TYPES;
        let m = Modes { kitty: f, ..Default::default() };
        assert_eq!(encode(Key::Char('c'), CTRL, Action::Release, None, m), Some(b"\x1b[99;5:3u".to_vec()));
        assert_eq!(encode(Key::Escape, Mods::NONE, Action::Repeat, None, m), Some(b"\x1b[27;1:2u".to_vec()));
        assert_eq!(encode(Key::Up, Mods::NONE, Action::Release, None, m), Some(b"\x1b[1;1:3A".to_vec()));
        // Plain Enter release is not reported without "all keys as escapes".
        assert_eq!(encode(Key::Enter, Mods::NONE, Action::Release, None, m), None);
    }

    #[test]
    fn kitty_all_keys_with_alternates_and_text() {
        let f = KITTY_DISAMBIGUATE | KITTY_ALL_AS_ESC | KITTY_ALTERNATE | KITTY_ASSOC_TEXT;
        let m = Modes { kitty: f, ..Default::default() };
        assert_eq!(encode(Key::Char('a'), Mods::NONE, Action::Press, Some("a"), m), Some(b"\x1b[97;1;97u".to_vec()));
        assert_eq!(encode(Key::Char('a'), SHIFT, Action::Press, Some("A"), m), Some(b"\x1b[97:65;2;65u".to_vec()));
        assert_eq!(encode(Key::Enter, Mods::NONE, Action::Press, None, m), Some(b"\x1b[13u".to_vec()));
        let plain = Modes { kitty: KITTY_DISAMBIGUATE | KITTY_ALL_AS_ESC, ..Default::default() };
        assert_eq!(encode(Key::Char('a'), Mods::NONE, Action::Press, Some("a"), plain), Some(b"\x1b[97u".to_vec()));
    }

    /// What xterm.js (`common/input/Keyboard.ts`) sends for these keys, so
    /// muscle memory and remote key bindings carry over from the webview.
    #[test]
    fn xterm_js_legacy_table() {
        const CS: Mods = Mods { shift: true, alt: false, ctrl: true, super_: false };
        const AS: Mods = Mods { shift: true, alt: true, ctrl: false, super_: false };
        let app = Modes { app_cursor: true, ..Default::default() };
        let cases: &[(Key, Mods, Modes, &[u8])] = &[
            (Key::Down, Mods::NONE, Modes::default(), b"\x1b[B"),
            (Key::Left, ALT, Modes::default(), b"\x1b[1;3D"),
            (Key::Right, CS, Modes::default(), b"\x1b[1;6C"),
            (Key::Left, Mods::NONE, app, b"\x1bOD"),
            (Key::Left, CTRL, app, b"\x1b[1;5D"),
            (Key::Home, Mods::NONE, Modes::default(), b"\x1b[H"),
            (Key::End, Mods::NONE, Modes::default(), b"\x1b[F"),
            (Key::Home, Mods::NONE, app, b"\x1bOH"),
            (Key::End, CTRL, Modes::default(), b"\x1b[1;5F"),
            (Key::Insert, Mods::NONE, Modes::default(), b"\x1b[2~"),
            (Key::PageDown, Mods::NONE, Modes::default(), b"\x1b[6~"),
            (Key::Delete, CS, Modes::default(), b"\x1b[3;6~"),
            (Key::F(2), Mods::NONE, Modes::default(), b"\x1bOQ"),
            (Key::F(4), Mods::NONE, Modes::default(), b"\x1bOS"),
            (Key::F(1), SHIFT, Modes::default(), b"\x1b[1;2P"),
            (Key::F(4), CTRL, Modes::default(), b"\x1b[1;5S"),
            (Key::F(6), Mods::NONE, Modes::default(), b"\x1b[17~"),
            (Key::F(7), Mods::NONE, Modes::default(), b"\x1b[18~"),
            (Key::F(8), Mods::NONE, Modes::default(), b"\x1b[19~"),
            (Key::F(9), Mods::NONE, Modes::default(), b"\x1b[20~"),
            (Key::F(10), Mods::NONE, Modes::default(), b"\x1b[21~"),
            (Key::F(11), Mods::NONE, Modes::default(), b"\x1b[23~"),
            (Key::F(12), Mods::NONE, Modes::default(), b"\x1b[24~"),
            (Key::F(5), ALT, Modes::default(), b"\x1b[15;3~"),
            (Key::Escape, Mods::NONE, Modes::default(), b"\x1b"),
            (Key::Escape, ALT, Modes::default(), b"\x1b\x1b"),
            (Key::Enter, ALT, Modes::default(), b"\x1b\r"),
            (Key::Tab, ALT, Modes::default(), b"\x1b\t"),
            (Key::Char('z'), CTRL, Modes::default(), &[0x1a]),
            (Key::Char('2'), CTRL, Modes::default(), &[0x00]),
            (Key::Char('3'), CTRL, Modes::default(), &[0x1b]),
            (Key::Char('4'), CTRL, Modes::default(), &[0x1c]),
            (Key::Char('5'), CTRL, Modes::default(), &[0x1d]),
            (Key::Char('6'), CTRL, Modes::default(), &[0x1e]),
            (Key::Char('7'), CTRL, Modes::default(), &[0x1f]),
            (Key::Char('8'), CTRL, Modes::default(), &[0x7f]),
            (Key::Char('\\'), CTRL, Modes::default(), &[0x1c]),
            (Key::Char(']'), CTRL, Modes::default(), &[0x1d]),
            (Key::Char('/'), CTRL, Modes::default(), &[0x1f]),
            (Key::Char('-'), CTRL, Modes::default(), &[0x1f]),
            (Key::Char('a'), AS, Modes::default(), b"\x1bA"),
            (Key::Char('.'), ALT, Modes::default(), b"\x1b."),
        ];
        let mut bad = Vec::new();
        for &(key, mods, modes, want) in cases {
            let text = match key {
                Key::Char(c) if mods.shift => Some(c.to_ascii_uppercase().to_string()),
                Key::Char(c) => Some(c.to_string()),
                _ => None,
            };
            let got = encode(key, mods, Action::Press, text.as_deref(), modes);
            if got.as_deref() != Some(want) {
                bad.push(format!("{key:?} {mods:?} app={}: want {:?} got {:?}", modes.app_cursor, want.escape_ascii().to_string(), got.map(|g| g.escape_ascii().to_string())));
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn egui_key_mapping() {
        assert_eq!(Key::from_egui(egui::Key::A), Some(Key::Char('a')));
        assert_eq!(Key::from_egui(egui::Key::Num1), Some(Key::Char('1')));
        assert_eq!(Key::from_egui(egui::Key::Space), Some(Key::Char(' ')));
        assert_eq!(Key::from_egui(egui::Key::OpenBracket), Some(Key::Char('[')));
        assert_eq!(Key::from_egui(egui::Key::F12), Some(Key::F(12)));
        assert_eq!(Key::from_egui(egui::Key::ArrowUp), Some(Key::Up));
    }
}
