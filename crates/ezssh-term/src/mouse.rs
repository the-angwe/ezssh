//! Mouse-report encoding (X10 / normal, UTF-8 1005 and SGR 1006).

use crate::keys::Mods;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    /// Motion with no button held (any-event tracking, mode 1003).
    None,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Normal,
    Utf8,
    Sgr,
}

/// Encode one mouse event. `col` and `line` are 0-based viewport cells.
/// Returns `None` when the position cannot be encoded in the format
/// (column or line beyond 222 in the normal format).
pub fn encode(
    button: Button,
    kind: Kind,
    col: usize,
    line: usize,
    mods: Mods,
    format: Format,
) -> Option<Vec<u8>> {
    let base: u32 = match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::None => 3,
        Button::WheelUp => 64,
        Button::WheelDown => 65,
    };
    let mut code = base;
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    if kind == Kind::Motion {
        code += 32;
    }
    let (x, y) = (col as u32 + 1, line as u32 + 1);

    match format {
        Format::Sgr => {
            let fin = if kind == Kind::Release { 'm' } else { 'M' };
            Some(format!("\x1b[<{code};{x};{y}{fin}").into_bytes())
        }
        Format::Normal | Format::Utf8 => {
            // The legacy formats cannot say which button was released.
            if kind == Kind::Release {
                code = (code & !3) | 3;
            }
            let mut out = b"\x1b[M".to_vec();
            out.push((32 + code) as u8);
            for v in [x, y] {
                let v = 32 + v;
                if format == Format::Normal {
                    if v > 255 {
                        return None;
                    }
                    out.push(v as u8);
                } else {
                    // Mode 1005: coordinates as UTF-8 code points, up to 2047.
                    let c = char::from_u32(v).filter(|_| v < 2048)?;
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_press_release_motion() {
        let m = Mods::NONE;
        assert_eq!(
            encode(Button::Left, Kind::Press, 0, 0, m, Format::Sgr),
            Some(b"\x1b[<0;1;1M".to_vec())
        );
        assert_eq!(
            encode(Button::Left, Kind::Release, 9, 4, m, Format::Sgr),
            Some(b"\x1b[<0;10;5m".to_vec())
        );
        assert_eq!(
            encode(Button::Left, Kind::Motion, 2, 3, m, Format::Sgr),
            Some(b"\x1b[<32;3;4M".to_vec())
        );
        assert_eq!(
            encode(Button::WheelDown, Kind::Press, 0, 0, m, Format::Sgr),
            Some(b"\x1b[<65;1;1M".to_vec())
        );
        let ctrl = Mods {
            ctrl: true,
            ..Mods::NONE
        };
        assert_eq!(
            encode(Button::Right, Kind::Press, 0, 0, ctrl, Format::Sgr),
            Some(b"\x1b[<18;1;1M".to_vec())
        );
    }

    #[test]
    fn normal_format_and_limits() {
        let m = Mods::NONE;
        assert_eq!(
            encode(Button::Left, Kind::Press, 0, 0, m, Format::Normal),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        assert_eq!(
            encode(Button::Left, Kind::Release, 0, 0, m, Format::Normal),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
        assert_eq!(
            encode(Button::Left, Kind::Press, 300, 0, m, Format::Normal),
            None
        );
        assert!(encode(Button::Left, Kind::Press, 300, 0, m, Format::Utf8).is_some());
    }
}
