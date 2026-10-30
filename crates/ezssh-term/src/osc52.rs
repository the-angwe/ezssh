//! OSC 52, the "manipulate selection data" escape sequence:
//!
//! ```text
//! ESC ] 52 ; <selection> ; <base64 payload> BEL
//! ```
//!
//! A program (Claude Code, neovim, tmux, …) sends it to put text on the
//! terminal's clipboard. It is the only clipboard route that works from the
//! far end of an SSH session. Only the write form is honoured; the read form
//! (`?` payload) is refused, because answering it would let any remote
//! program read the local clipboard.
//!
//! Port of `ui/lib/osc52.ts`, with its tests. alacritty's own handler
//! decodes with a strict base64 engine (padding required), which would drop
//! the unpadded payloads Rust and Go programs send, so the terminal routes
//! OSC 52 here instead (see `hook.rs`).

/// Decoded size cap. Larger than any sane copy; stops a hostile host from
/// making us allocate hundreds of MB on one escape.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Write(String),
    Read,
    Ignore(&'static str),
}

/// Selection characters routed to the system clipboard. `c` is the
/// clipboard proper; `p` (primary) and `s` (select) are X11 selections with
/// no separate meaning on Windows / macOS, so they get the same treatment.
/// An empty selector means the default, `c`. Cut buffers `0`–`7` and `q` are
/// ignored.
fn is_clipboard_selector(selection: &[u8]) -> bool {
    selection.is_empty() || selection.iter().any(|b| matches!(b, b'c' | b'p' | b's'))
}

/// Parse the two parameters after `52;`.
pub fn parse(selection: &[u8], payload: &[u8]) -> Request {
    if !is_clipboard_selector(selection) {
        return Request::Ignore("selection is not a clipboard target");
    }
    if payload == b"?" {
        return Request::Read;
    }
    if payload.is_empty() {
        return Request::Ignore("empty payload");
    }
    // Whitespace is tolerated (some senders wrap long payloads), and so is
    // missing padding (Rust's STANDARD_NO_PAD, Go's RawStdEncoding).
    let stripped: Vec<u8> = payload.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let pad = stripped.iter().rev().take_while(|&&b| b == b'=').count();
    let body = &stripped[..stripped.len() - pad];
    if pad > 2 || stripped.len() % 4 == 1 || !body.iter().all(|&b| sextet(b).is_some()) {
        return Request::Ignore("payload is not base64");
    }
    // 3 decoded bytes per 4 base64 chars; check before decoding.
    if body.len() / 4 * 3 > MAX_BYTES {
        return Request::Ignore("payload exceeds size cap");
    }
    let Some(bytes) = decode(body) else {
        return Request::Ignore("payload is not base64");
    };
    // NUL terminates CF_UNICODETEXT on Windows and is rejected by some Linux
    // clipboard backends; drop it rather than lose everything after it.
    let text = String::from_utf8_lossy(&bytes).replace('\0', "");
    Request::Write(text)
}

fn sextet(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode unpadded standard base64. `None` when the length cannot be valid.
fn decode(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    for chunk in body.chunks(4) {
        let mut acc: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            acc |= (sextet(b)? as u32) << (18 - 6 * i);
        }
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        // Test-only encoder (standard alphabet, padded).
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in s.as_bytes().chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn p(sel: &str, payload: &str) -> Request {
        parse(sel.as_bytes(), payload.as_bytes())
    }

    #[test]
    fn decodes_a_clipboard_write() {
        assert_eq!(p("c", &b64("hello")), Request::Write("hello".into()));
    }

    #[test]
    fn decodes_utf8_payloads() {
        let text = "naïve — 日本語 🚀";
        assert_eq!(p("c", &b64(text)), Request::Write(text.into()));
    }

    #[test]
    fn empty_selector_is_the_clipboard() {
        assert_eq!(p("", &b64("x")), Request::Write("x".into()));
    }

    #[test]
    fn primary_select_and_multi_char_selectors() {
        assert!(matches!(p("p", &b64("x")), Request::Write(_)));
        assert!(matches!(p("s", &b64("x")), Request::Write(_)));
        assert!(matches!(p("pc", &b64("x")), Request::Write(_)));
    }

    #[test]
    fn cut_buffer_only_selectors_are_ignored() {
        assert!(matches!(p("0", &b64("x")), Request::Ignore(_)));
        assert!(matches!(p("q", &b64("x")), Request::Ignore(_)));
    }

    #[test]
    fn read_form_is_reported_not_decoded() {
        assert_eq!(p("c", "?"), Request::Read);
    }

    #[test]
    fn whitespace_wrapped_base64() {
        let wrapped: String = b64("a somewhat longer string")
            .chars()
            .enumerate()
            .flat_map(|(i, c)| if i % 8 == 7 { vec![c, '\n'] } else { vec![c] })
            .collect();
        assert_eq!(p("c", &wrapped), Request::Write("a somewhat longer string".into()));
    }

    #[test]
    fn unpadded_base64() {
        assert_eq!(p("c", "aGVsbG8"), Request::Write("hello".into()));
        assert_eq!(p("c", "aGk"), Request::Write("hi".into()));
    }

    #[test]
    fn strips_nul_bytes() {
        assert_eq!(p("c", &b64("ab\0cd")), Request::Write("abcd".into()));
    }

    #[test]
    fn malformed_input_is_ignored() {
        assert!(matches!(p("c", ""), Request::Ignore(_)));
        assert!(matches!(p("c", "not base64!"), Request::Ignore(_)));
        assert!(matches!(p("c", "abcde"), Request::Ignore(_))); // length 1 mod 4
        assert!(matches!(p("c", "ab==="), Request::Ignore(_)));
        assert!(matches!(p("c", "a=bc"), Request::Ignore(_)));
    }

    #[test]
    fn oversized_payload_refused_before_decoding() {
        let chars = MAX_BYTES.div_ceil(3) * 4 + 4;
        assert!(matches!(p("c", &"A".repeat(chars)), Request::Ignore(_)));
    }
}
