//! Ctrl+C / Ctrl+X / Ctrl+V for egui's text fields on Linux and Windows.
//!
//! eframe's own translation of those keys into `Event::Copy` / `Cut` /
//! `Paste` is off (`clipboard_shortcuts: false` in `main.rs`): a focused
//! terminal needs them as keys (^C interrupts, ^V is quoted-insert), and
//! eframe would swallow the key before the terminal saw it. While no
//! terminal has the keyboard they are translated here instead, in
//! `App::raw_input_hook`, so the frame's input reaches egui the way eframe
//! would have delivered it. macOS keeps eframe's translation: there the
//! shortcuts are ⌘C / ⌘X / ⌘V, which no shell wants.

use egui::{Event, Key};

#[derive(Default)]
pub(crate) struct ClipboardKeys {
    clipboard: Option<arboard::Clipboard>,
}

impl ClipboardKeys {
    pub(crate) fn translate(&mut self, events: &mut Vec<Event>) {
        translate(events, || self.read());
    }

    fn read(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.clipboard = Some(c),
                Err(e) => {
                    tracing::error!("clipboard: unavailable, Ctrl+V does nothing: {e}");
                    return None;
                }
            }
        }
        match self.clipboard.as_mut()?.get_text() {
            Ok(t) => Some(t.replace("\r\n", "\n")),
            Err(e) => {
                tracing::warn!("clipboard: read failed, Ctrl+V does nothing: {e}");
                None
            }
        }
    }
}

enum Clip {
    Copy,
    Cut,
    Paste,
}

/// eframe's rules (`egui-winit`): Ctrl+C / X / V, and on Windows
/// Ctrl+Insert / Shift+Delete / Shift+Insert. The press becomes the
/// clipboard event (a paste with nothing to paste is dropped); the release
/// stays a key event, as eframe leaves it.
fn clip_of(key: Key, m: egui::Modifiers) -> Option<Clip> {
    match key {
        Key::C if m.command_only() => Some(Clip::Copy),
        Key::X if m.command_only() => Some(Clip::Cut),
        Key::V if m.command_only() => Some(Clip::Paste),
        Key::Insert if cfg!(windows) && m.ctrl => Some(Clip::Copy),
        Key::Delete if cfg!(windows) && m.shift => Some(Clip::Cut),
        Key::Insert if cfg!(windows) && m.shift => Some(Clip::Paste),
        _ => None,
    }
}

fn translate(events: &mut Vec<Event>, mut read: impl FnMut() -> Option<String>) {
    let mut i = 0;
    while i < events.len() {
        let clip = match &events[i] {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => clip_of(*key, *modifiers),
            _ => None,
        };
        let Some(clip) = clip else {
            i += 1;
            continue;
        };
        let replacement = match clip {
            Clip::Copy => Some(Event::Copy),
            Clip::Cut => Some(Event::Cut),
            Clip::Paste => read().filter(|t| !t.is_empty()).map(Event::Paste),
        };
        tracing::trace!(key = ?events[i], replaced = replacement.is_some(), "clipboard: key");
        match replacement {
            Some(ev) => {
                events[i] = ev;
                i += 1;
            }
            None => {
                events.remove(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    fn key(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn ctrl_c_x_v_become_clipboard_events() {
        let mut events = vec![
            key(Key::C, Modifiers::COMMAND, true),
            key(Key::C, Modifiers::COMMAND, false),
            key(Key::X, Modifiers::COMMAND, true),
            key(Key::V, Modifiers::COMMAND, true),
            key(Key::V, Modifiers::COMMAND | Modifiers::SHIFT, true),
            key(Key::B, Modifiers::COMMAND, true),
            Event::Text("v".into()),
        ];
        translate(&mut events, || Some("clip".into()));
        assert!(matches!(events[0], Event::Copy));
        assert!(matches!(
            events[1],
            Event::Key {
                key: Key::C,
                pressed: false,
                ..
            }
        ));
        assert!(matches!(events[2], Event::Cut));
        assert!(matches!(&events[3], Event::Paste(t) if t == "clip"));
        assert!(
            matches!(events[4], Event::Key { key: Key::V, .. }),
            "Ctrl+Shift+V is not eframe's paste"
        );
        assert!(matches!(events[5], Event::Key { key: Key::B, .. }));
        assert_eq!(events.len(), 7);
    }

    #[test]
    fn paste_with_an_empty_clipboard_is_dropped() {
        let mut events = vec![
            key(Key::V, Modifiers::COMMAND, true),
            key(Key::V, Modifiers::COMMAND, false),
        ];
        translate(&mut events, || None);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Key { pressed: false, .. }));
    }
}
