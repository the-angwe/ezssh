//! Headless UI tests of the terminal widget: context menu, find bar,
//! connection overlays, paste confirmation and shortcuts, driven through
//! egui_kittest (offscreen, AccessKit queries, synthetic input).
//!
//! `EZSSH_UI_SHOTS=<dir> cargo test -p ezssh-term --test ui` also renders
//! each state to `<dir>/<name>.png` with wgpu, for a visual check.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ezssh_term::{
    Feeder, FontRegistry, Overlay, Pty, TermAction, TermOptions, TermStyle, Terminal, TerminalView,
    WindowSize,
};

#[derive(Default)]
struct Sink(Mutex<Vec<u8>>);

impl Pty for Sink {
    fn write(&self, bytes: Cow<'static, [u8]>) {
        self.0.lock().unwrap().extend_from_slice(&bytes);
    }
    fn resize(&self, _: WindowSize) {}
}

struct State {
    term: Option<(Terminal, Feeder)>,
    sink: Arc<Sink>,
    style: TermStyle,
    overlay: Option<Overlay>,
    focus: bool,
    actions: Vec<TermAction>,
    /// Bytes fed on the first frame.
    initial: Vec<u8>,
    font: Option<String>,
}

impl State {
    fn new(initial: &[u8]) -> Self {
        Self {
            term: None,
            sink: Arc::new(Sink::default()),
            style: TermStyle::default(),
            overlay: None,
            focus: true,
            actions: Vec::new(),
            initial: initial.to_vec(),
            font: std::env::var("EZSSH_UI_SHOTS")
                .ok()
                .map(|_| "JetBrains Mono".to_string()),
        }
    }

    fn term(&mut self) -> &mut Terminal {
        &mut self
            .term
            .as_mut()
            .expect("terminal created on the first frame")
            .0
    }
}

fn harness(state: State) -> Harness<'static, State> {
    let mut h = Harness::builder()
        .with_size(vec2(720.0, 360.0))
        .build_ui_state(
            |ui, s: &mut State| {
                if s.term.is_none() {
                    // Steady cursor: a blinking one keeps requesting frames.
                    let opts = TermOptions {
                        cursor_blink: false,
                        ..TermOptions::default()
                    };
                    let (mut term, feeder) =
                        Terminal::new(ui.ctx(), s.sink.clone(), opts, s.style.palette.clone());
                    if let Some(font) = &s.font {
                        term.set_faces(FontRegistry::new().faces(ui.ctx(), font));
                    }
                    term.set_font_size(13.0);
                    feeder.feed(&s.initial);
                    s.term = Some((term, feeder));
                }
                let (term, _) = s.term.as_mut().unwrap();
                let focus = std::mem::take(&mut s.focus);
                let out = TerminalView::new(term, &s.style)
                    .focus(focus)
                    .overlay(s.overlay.as_ref())
                    .show(ui);
                s.actions.extend(out.actions);
            },
            state,
        );
    h.run_steps(3);
    h
}

fn shot(h: &mut Harness<'_, State>, name: &str) {
    let Ok(dir) = std::env::var("EZSSH_UI_SHOTS") else {
        return;
    };
    // Past egui's fade-in of popups and windows.
    h.run_steps(20);
    match h.render() {
        Ok(img) => {
            let path = std::path::Path::new(&dir).join(format!("{name}.png"));
            img.save(&path)
                .unwrap_or_else(|e| panic!("saving {}: {e}", path.display()));
            eprintln!("wrote {}", path.display());
        }
        Err(e) => panic!("render failed: {e}"),
    }
}

fn click(h: &mut Harness<'_, State>, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button,
        pressed: true,
        modifiers,
    });
    h.event(Event::PointerButton {
        pos,
        button,
        pressed: false,
        modifiers,
    });
    h.run_steps(3);
}

const SAMPLE: &[u8] = b"$ ls\r\nREADME.md  Cargo.toml  src\r\n$ echo see https://example.com/docs\r\nsee https://example.com/docs\r\n$ ";

#[test]
fn context_menu_items_and_actions() {
    let mut h = harness(State::new(SAMPLE));
    click(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    // Items with a shortcut expose "label shortcut" to accessibility.
    for label in [
        "Copy",
        "Paste",
        "Select All",
        "Find…",
        "Appearance…",
        "Clear scrollback",
    ] {
        assert!(
            h.query_by_label_contains(label).is_some(),
            "menu item {label} missing"
        );
    }
    assert!(
        h.get_by_label_contains("Copy")
            .accesskit_node()
            .is_disabled(),
        "Copy needs a selection"
    );
    shot(&mut h, "context-menu");

    h.get_by_label("Select All").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Select All").is_none(),
        "menu closes after a choice"
    );
    assert!(h.state_mut().term().has_selection());

    click(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    assert!(
        !h.get_by_label_contains("Copy")
            .accesskit_node()
            .is_disabled(),
        "Copy enabled with a selection"
    );
    h.get_by_label_contains("Appearance…").click();
    h.run_steps(3);
    assert_eq!(h.state().actions, vec![TermAction::OpenAppearance]);

    // Escape closes it.
    click(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Clear scrollback").is_none());
}

#[test]
fn clear_scrollback_from_the_menu() {
    let mut text = Vec::new();
    for i in 0..100 {
        text.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    text.extend_from_slice(b"$ ");
    let mut h = harness(State::new(&text));
    click(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    h.get_by_label("Clear scrollback").click();
    h.run_steps(3);
    let t = h.state_mut().term();
    t.select_all();
    assert_eq!(t.selection_text().unwrap().trim(), "$");
}

#[test]
fn right_click_paste_mode_skips_the_menu_unless_ctrl() {
    let mut state = State::new(SAMPLE);
    state.style.right_click = ezssh_term::RightClick::Paste;
    let mut h = harness(state);
    click(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    assert!(
        h.query_by_label("Select All").is_none(),
        "paste mode: no menu"
    );
    click(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        Modifiers::CTRL,
    );
    assert!(
        h.query_by_label("Select All").is_some(),
        "Ctrl+right-click still opens the menu"
    );
}

#[test]
fn find_bar_counts_and_closes() {
    let mut text = Vec::new();
    for i in 0..60 {
        text.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    let mut h = harness(State::new(&text));
    // Ctrl+Shift+F from the focused terminal.
    h.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, egui::Key::F);
    h.run_steps(3);
    let input = h.get_by_role(egui::accesskit::Role::TextInput);
    input.type_text("line 4");
    h.run_steps(3);
    // "line 4" and "line 40".."line 49": 11 matches, the most recent selected.
    assert!(h.query_by_label("11/11").is_some(), "counter after typing");
    assert_eq!(
        h.state_mut().term().selection_text().as_deref(),
        Some("line 4")
    );
    shot(&mut h, "find-bar");

    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.query_by_label("10/11").is_some(),
        "Shift+Enter goes to the previous match"
    );
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.query_by_label("11/11").is_some(),
        "Enter goes to the next match"
    );

    h.get_by_label("Aa").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("11/11").is_some(),
        "lower-case query, case-sensitive: same count"
    );
    h.get_by_role(egui::accesskit::Role::TextInput).focus();
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Aa").is_none(), "Esc closes the find bar");
    assert!(!h.state().term.as_ref().unwrap().0.view_state().find_open());
}

#[test]
fn overlays_and_reconnect() {
    let mut state = State::new(SAMPLE);
    state.overlay = Some(Overlay::Failed {
        message: "authentication failed".into(),
    });
    let mut h = harness(state);
    assert!(h.query_by_label("Connection failed").is_some());
    assert!(h.query_by_label("authentication failed").is_some());
    shot(&mut h, "overlay-failed");
    h.get_by_label("Reconnect").click();
    h.run_steps(3);
    assert_eq!(h.state().actions, vec![TermAction::Reconnect]);

    h.state_mut().overlay = Some(Overlay::Closed {
        reason: Some("exit 130".into()),
        summary: "root@db1".into(),
    });
    h.run_steps(3);
    assert!(h.query_by_label("Disconnected (exit 130)").is_some());
    assert!(h.query_by_label("root@db1").is_some());

    h.state_mut().overlay = Some(Overlay::Connecting {
        summary: "root@db1:2222".into(),
    });
    h.run_steps(3);
    assert!(h.query_by_label("Connecting…").is_some());
    assert!(h.query_by_label("Reconnect").is_none());
    shot(&mut h, "overlay-connecting");

    // The overlay blocks the terminal: no menu, no keystrokes.
    h.state_mut().sink.0.lock().unwrap().clear();
    click(
        &mut h,
        pos2(100.0, 100.0),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    assert!(h.query_by_label("Select All").is_none());
    h.event(Event::Text("x".into()));
    h.run_steps(2);
    assert!(h.state().sink.0.lock().unwrap().is_empty());
}

#[test]
fn multiline_paste_asks_first() {
    let mut h = harness(State::new(b"$ "));
    h.event(Event::Paste("echo one\necho two\n".into()));
    h.run_steps(2);
    assert_eq!(
        h.state().actions,
        vec![TermAction::ConfirmPaste {
            text: "echo one\necho two\n".into(),
            lines: 2
        }]
    );
    assert!(
        h.state().sink.0.lock().unwrap().is_empty(),
        "nothing typed before the answer"
    );

    // A single line goes straight through.
    h.event(Event::Paste("ls -la\n".into()));
    h.run_steps(2);
    assert_eq!(h.state().sink.0.lock().unwrap().as_slice(), b"ls -la\r");

    // With the warning off, everything goes through.
    h.state_mut().style.warn_multiline_paste = false;
    h.state_mut().sink.0.lock().unwrap().clear();
    h.event(Event::Paste("a\nb".into()));
    h.run_steps(2);
    assert_eq!(h.state().sink.0.lock().unwrap().as_slice(), b"a\rb");
}

#[test]
fn typing_reaches_the_pty() {
    let mut h = harness(State::new(b"$ "));
    h.event(Event::Text("ls".into()));
    h.key_press(egui::Key::Enter);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::C);
    h.run_steps(2);
    assert_eq!(h.state().sink.0.lock().unwrap().as_slice(), b"ls\r\x03");
}

#[test]
fn ctrl_wheel_zooms_this_terminal() {
    let mut h = harness(State::new(b"$ "));
    let before = h.state_mut().term().font_size();
    h.event(Event::PointerMoved(pos2(300.0, 200.0)));
    h.event_modifiers(
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::CTRL,
        },
        Modifiers::CTRL,
    );
    h.run_steps(2);
    assert_eq!(h.state_mut().term().font_size(), before + 2.0);
}

#[test]
fn link_click_opens_url() {
    let mut h = harness(State::new(SAMPLE));
    // Row 3 ("see https://example.com/docs"), column 10 is inside the URL.
    // Cell size depends on the font; find it from the terminal's grid.
    let (cols, rows) = h.state().term.as_ref().unwrap().0.grid_size();
    let cell = vec2((720.0 - 8.0) / cols as f32, (360.0 - 8.0) / rows as f32);
    let pos = pos2(4.0 + cell.x * 10.5, 4.0 + cell.y * 3.5);
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    assert_eq!(
        h.state()
            .term
            .as_ref()
            .unwrap()
            .0
            .view_state()
            .hovered_link(),
        Some("https://example.com/docs")
    );
    click(&mut h, pos, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(
        h.state().actions,
        vec![TermAction::OpenUrl("https://example.com/docs".into())]
    );
}

/// Paint cost of a full, dense screen, ligature splitting on and off.
/// `cargo test --release -p ezssh-term --test ui paint_cost -- --ignored --nocapture`
#[test]
#[ignore = "measurement, not a check"]
fn paint_cost() {
    let mut text = Vec::new();
    for i in 0..80 {
        let line = format!(
            "\x1b[3{}m{i:04}\x1b[0m fn main() -> Result<(), Error> {{ if a != b && c >= d {{ x => y }} }} // === {}\r\n",
            i % 8,
            "word ".repeat(i % 7)
        );
        text.extend_from_slice(line.as_bytes());
    }
    for ligatures in [false, true] {
        let mut state = State::new(&text);
        state.style.ligatures = ligatures;
        state.focus = false;
        let mut h = Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            .build_ui_state(
                |ui, s: &mut State| {
                    if s.term.is_none() {
                        let opts = TermOptions {
                            cursor_blink: false,
                            ..TermOptions::default()
                        };
                        let (mut term, feeder) =
                            Terminal::new(ui.ctx(), s.sink.clone(), opts, s.style.palette.clone());
                        term.set_font_size(13.0);
                        feeder.feed(&s.initial);
                        s.term = Some((term, feeder));
                    }
                    let (term, _) = s.term.as_mut().unwrap();
                    TerminalView::new(term, &s.style).show(ui);
                    // Fresh galleys every frame, as when output keeps arriving.
                    ui.ctx().request_repaint();
                },
                state,
            );
        h.run_steps(5);
        let mut paints = Vec::new();
        for _ in 0..60 {
            h.step();
            paints.push(h.state().term.as_ref().unwrap().0.view_state().last_paint());
        }
        paints.sort();
        let (cols, rows) = h.state().term.as_ref().unwrap().0.grid_size();
        eprintln!(
            "ligatures={ligatures} {cols}x{rows}: paint p50 {:.2} ms, p95 {:.2} ms",
            paints[30].as_secs_f64() * 1e3,
            paints[57].as_secs_f64() * 1e3
        );
    }
}

/// macOS: ⌘A / ⌘C / ⌘F / ⌘+ / ⌘− / ⌘0 are the terminal's (egui reports
/// `mac_cmd` only there; nothing reaches the shell).
#[test]
fn mac_command_chords() {
    let mut h = harness(State::new(b"hello world\r\n$ "));
    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::A);
    h.run_steps(2);
    assert!(
        h.state_mut()
            .term()
            .selection_text()
            .is_some_and(|t| t.contains("hello world")),
        "⌘A selects everything"
    );
    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::C);
    h.run_steps(2);
    assert!(
        h.state().sink.0.lock().unwrap().is_empty(),
        "⌘A / ⌘C send nothing to the pty"
    );

    let base = h.state_mut().term().font_size();
    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::Equals);
    h.run_steps(2);
    assert_eq!(h.state_mut().term().font_size(), base + 1.0, "⌘= grows");
    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::Minus);
    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::Minus);
    h.run_steps(2);
    assert_eq!(h.state_mut().term().font_size(), base - 1.0, "⌘− shrinks");
    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::Num0);
    h.run_steps(2);
    assert_eq!(h.state_mut().term().font_size(), base, "⌘0 resets");

    h.key_press_modifiers(Modifiers::MAC_CMD, egui::Key::F);
    h.run_steps(3);
    assert!(h.query_by_label("Aa").is_some(), "⌘F opens the find bar");
    assert!(
        h.state().sink.0.lock().unwrap().is_empty(),
        "the chords send nothing to the pty"
    );
}
