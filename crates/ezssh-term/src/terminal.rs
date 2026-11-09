//! The terminal model shared between a byte source (SSH channel reader,
//! local PTY reader) and the egui view.
//!
//! Bytes do not go through the UI event queue: the reader calls
//! [`Feeder::feed`], which runs the VT parser straight into the locked
//! `Term` and asks egui for a repaint. The UI thread locks the `Term`
//! only to paint, resize, select or scroll. This is the arrangement Zed
//! uses.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::vte::ansi::{ClearMode, CursorShape, CursorStyle, Handler, Processor, Rgb};

use crate::hook::Hooked;
use crate::palette::Palette;

/// Where keystrokes, query answers and size changes go. Implementations
/// must not block: queue the bytes and return.
pub trait Pty: Send + Sync + 'static {
    fn write(&self, bytes: Cow<'static, [u8]>);
    fn resize(&self, size: WindowSize);
}

/// Cursor shape a session starts with (programs can change it with
/// DECSCUSR). Same three choices as the session dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorKind {
    #[default]
    Block,
    Underline,
    Bar,
}

impl CursorKind {
    /// Parse the session row's `cursor_style` (`block` / `underline` / `bar`).
    pub fn from_setting(s: &str) -> Self {
        match s {
            "underline" => Self::Underline,
            "bar" => Self::Bar,
            "block" => Self::Block,
            other => {
                tracing::warn!(value = other, "term: unknown cursor style, using block");
                Self::Block
            }
        }
    }

    fn shape(self) -> CursorShape {
        match self {
            Self::Block => CursorShape::Block,
            Self::Underline => CursorShape::Underline,
            Self::Bar => CursorShape::Beam,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TermOptions {
    pub scrollback: usize,
    /// Allow programs to set the clipboard with OSC 52. Reads are always
    /// refused. Changeable later with [`Terminal::set_osc52_write`].
    pub osc52_write: bool,
    pub cursor: CursorKind,
    pub cursor_blink: bool,
}

impl Default for TermOptions {
    fn default() -> Self {
        Self {
            scrollback: 10_000,
            osc52_write: true,
            cursor: CursorKind::Block,
            cursor_blink: true,
        }
    }
}

impl TermOptions {
    fn config(&self) -> Config {
        Config {
            scrolling_history: self.scrollback,
            kitty_keyboard: true,
            // OSC 52 is handled by `hook.rs`; alacritty's own path stays off.
            osc52: Osc52::Disabled,
            default_cursor_style: CursorStyle {
                shape: self.cursor.shape(),
                blinking: self.cursor_blink,
            },
            ..Config::default()
        }
    }
}

/// Events the view handles on its next frame.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    Title(String),
    ResetTitle,
    Bell,
    ClipboardStore(String),
}

struct ListenerInner {
    ctx: egui::Context,
    pty: Arc<dyn Pty>,
    palette: Mutex<Palette>,
    size: Mutex<WindowSize>,
    events: Mutex<Vec<UiEvent>>,
    osc52_write: AtomicBool,
}

/// alacritty `EventListener`. Called with the `Term` lock held, so it
/// never locks the `Term` itself.
#[derive(Clone)]
pub struct Listener(Arc<ListenerInner>);

impl Listener {
    pub(crate) fn push(&self, ev: UiEvent) {
        match self.0.events.lock() {
            Ok(mut q) => q.push(ev),
            Err(e) => tracing::error!("term.listener: event queue poisoned: {e}"),
        }
        self.0.ctx.request_repaint();
    }

    pub(crate) fn osc52_write(&self) -> bool {
        self.0.osc52_write.load(Ordering::Relaxed)
    }

    pub(crate) fn write_pty(&self, bytes: Cow<'static, [u8]>) {
        self.0.pty.write(bytes);
    }

    /// Palette colour for an alacritty colour index (no OSC override).
    pub(crate) fn palette_color(&self, index: usize) -> Rgb {
        match self.0.palette.lock() {
            Ok(p) => p.by_index(index),
            Err(e) => {
                tracing::error!("term.listener: palette poisoned: {e}");
                Palette::default().by_index(index)
            }
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let inner = &self.0;
        match event {
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {
                inner.ctx.request_repaint()
            }
            Event::PtyWrite(text) => {
                tracing::trace!(len = text.len(), "term.listener: pty write (query answer)");
                inner.pty.write(Cow::Owned(text.into_bytes()));
            }
            Event::ColorRequest(index, format) => {
                // `hook.rs` answers colour queries itself; this only runs if
                // alacritty asks from another path.
                inner
                    .pty
                    .write(Cow::Owned(format(self.palette_color(index)).into_bytes()));
            }
            Event::TextAreaSizeRequest(format) => {
                let size = match inner.size.lock() {
                    Ok(s) => *s,
                    Err(e) => {
                        tracing::error!("term.listener: size poisoned: {e}");
                        return;
                    }
                };
                inner.pty.write(Cow::Owned(format(size).into_bytes()));
            }
            Event::ClipboardStore(ty, text) => {
                // OSC 52 goes through `hook.rs`; alacritty's path is disabled.
                tracing::warn!(
                    ?ty,
                    len = text.len(),
                    "term.osc52: unexpected store from alacritty, dropped"
                );
            }
            Event::ClipboardLoad(ty, _) => {
                tracing::debug!(?ty, "term.osc52: clipboard read refused");
            }
            Event::Title(t) => self.push(UiEvent::Title(t)),
            Event::ResetTitle => self.push(UiEvent::ResetTitle),
            Event::Bell => self.push(UiEvent::Bell),
            Event::Exit | Event::ChildExit(_) => {
                tracing::debug!("term.listener: exit event from parser");
            }
        }
    }
}

/// Grid dimensions for `Term::new` / `Term::resize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

struct Shared {
    term: FairMutex<Term<Listener>>,
    listener: Listener,
    parser: Mutex<Processor>,
    ctx: egui::Context,
    /// Bumped per `feed`: the view keys its caches (link under the
    /// pointer, find matches) on it so they are rebuilt only when the
    /// grid may have changed.
    feed_gen: AtomicU64,
}

/// Handed to the byte source. Cheap to clone.
#[derive(Clone)]
pub struct Feeder(Arc<Shared>);

impl Feeder {
    /// Run `bytes` through the parser into the terminal and request a repaint.
    pub fn feed(&self, bytes: &[u8]) {
        tracing::trace!(len = bytes.len(), head = %bytes[..bytes.len().min(160)].escape_ascii(), "term.feed");
        {
            let mut parser = match self.0.parser.lock() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(
                        "term.feed: parser poisoned, dropping {} bytes: {e}",
                        bytes.len()
                    );
                    return;
                }
            };
            let mut term = self.0.term.lock();
            let mut hooked = Hooked {
                term: &mut term,
                listener: &self.0.listener,
            };
            // A synchronized update (DECSET 2026) that outlived its timeout
            // is flushed before new bytes, so output never stalls behind it.
            if parser
                .sync_timeout()
                .sync_timeout()
                .is_some_and(|d| Instant::now() >= d)
            {
                tracing::debug!("term.feed: synchronized update timed out, flushing");
                parser.stop_sync(&mut hooked);
            }
            parser.advance(&mut hooked, bytes);
        }
        self.0.feed_gen.fetch_add(1, Ordering::Release);
        self.0.ctx.request_repaint();
    }
}

pub struct Terminal {
    shared: Arc<Shared>,
    listener: Listener,
    pty: Arc<dyn Pty>,
    options: TermOptions,
    title: Option<String>,
    pub(crate) view: crate::view::ViewState,
}

impl Terminal {
    /// Create a terminal writing to `pty`. Returns the feeder the byte
    /// source pushes output into.
    pub fn new(
        ctx: &egui::Context,
        pty: Arc<dyn Pty>,
        options: TermOptions,
        palette: Palette,
    ) -> (Terminal, Feeder) {
        let size = GridSize {
            columns: 80,
            lines: 24,
        };
        let listener = Listener(Arc::new(ListenerInner {
            ctx: ctx.clone(),
            pty: pty.clone(),
            palette: Mutex::new(palette),
            size: Mutex::new(WindowSize {
                num_lines: 24,
                num_cols: 80,
                cell_width: 8,
                cell_height: 16,
            }),
            events: Mutex::new(Vec::new()),
            osc52_write: AtomicBool::new(options.osc52_write),
        }));
        let term = Term::new(options.config(), &size, listener.clone());
        let shared = Arc::new(Shared {
            term: FairMutex::new(term),
            listener: listener.clone(),
            parser: Mutex::new(Processor::new()),
            ctx: ctx.clone(),
            feed_gen: AtomicU64::new(0),
        });
        let feeder = Feeder(shared.clone());
        let view = crate::view::ViewState::default();
        (
            Terminal {
                shared,
                listener,
                pty,
                options,
                title: None,
                view,
            },
            feeder,
        )
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.pty.write(bytes.into());
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Changes whenever bytes were fed since the last call: cheap "may the
    /// grid have changed" test for the view's caches.
    pub fn feed_generation(&self) -> u64 {
        self.shared.feed_gen.load(Ordering::Acquire)
    }

    pub fn view_state(&self) -> &crate::view::ViewState {
        &self.view
    }

    pub fn grid_size(&self) -> (usize, usize) {
        (self.view.cols, self.view.rows)
    }

    /// Palette used to answer colour queries. The widget paints with
    /// `TermStyle::palette`; call this whenever that changes.
    pub fn set_palette(&self, palette: Palette) {
        match self.listener.0.palette.lock() {
            Ok(mut p) => *p = palette,
            Err(e) => tracing::error!("term: palette poisoned: {e}"),
        }
    }

    /// Live switch for OSC 52 clipboard writes (`terminal.osc52Clipboard`).
    pub fn set_osc52_write(&mut self, allow: bool) {
        self.options.osc52_write = allow;
        self.listener.0.osc52_write.store(allow, Ordering::Relaxed);
    }

    /// Default cursor shape and blinking. A program's own DECSCUSR choice
    /// stays in effect until it resets the cursor.
    pub fn set_cursor(&mut self, cursor: CursorKind, blink: bool) {
        if self.options.cursor == cursor && self.options.cursor_blink == blink {
            return;
        }
        self.options.cursor = cursor;
        self.options.cursor_blink = blink;
        let config = self.options.config();
        self.lock().set_options(config);
    }

    /// Scrollback length in lines. Shrinking drops the oldest lines.
    pub fn set_scrollback(&mut self, lines: usize) {
        if self.options.scrollback == lines {
            return;
        }
        tracing::debug!(
            from = self.options.scrollback,
            to = lines,
            "term: scrollback"
        );
        self.options.scrollback = lines;
        let config = self.options.config();
        self.lock().set_options(config);
    }

    /// The options in effect (scrollback, cursor, OSC 52).
    pub fn options(&self) -> &TermOptions {
        &self.options
    }

    /// Paste `text` as typed input: newlines become CR and the text is
    /// wrapped in bracketed-paste markers when the program asked for them.
    /// Jumps to the bottom of the scrollback. No multi-line confirmation
    /// here; the view asks for that before calling this.
    pub fn paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let bracketed = self.lock().mode().contains(TermMode::BRACKETED_PASTE);
        tracing::debug!(len = text.len(), bracketed, "term.paste");
        let bytes = paste_bytes(text, bracketed);
        self.scroll_to_bottom();
        self.write(bytes);
    }

    /// Text of the current selection, if any.
    pub fn selection_text(&self) -> Option<String> {
        self.lock().selection_to_string().filter(|t| !t.is_empty())
    }

    pub fn has_selection(&self) -> bool {
        self.lock()
            .selection
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    /// Select the whole buffer, scrollback included.
    pub fn select_all(&mut self) {
        let mut t = self.lock();
        let start = Point::new(t.grid().topmost_line(), Column(0));
        let end = Point::new(t.grid().bottommost_line(), t.grid().last_column());
        let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
        sel.update(end, Side::Right);
        t.selection = Some(sel);
    }

    /// xterm.js `clear()`: drop the scrollback and everything above the
    /// cursor line, which becomes the first line. The alternate screen has
    /// no scrollback, so only the selection is reset there.
    pub fn clear_scrollback(&mut self) {
        let mut t = self.lock();
        t.selection = None;
        if !t.mode().contains(TermMode::ALT_SCREEN) {
            let cursor = t.grid().cursor.point;
            if cursor.line.0 > 0 {
                t.scroll_up(cursor.line.0 as usize);
                t.goto(0, cursor.column.0);
            }
            t.clear_screen(ClearMode::Saved);
        }
        t.scroll_display(Scroll::Bottom);
        tracing::debug!("term: scrollback cleared");
    }

    pub fn scroll_to_bottom(&mut self) {
        let mut t = self.lock();
        if t.grid().display_offset() != 0 {
            t.scroll_display(Scroll::Bottom);
        }
    }

    pub(crate) fn lock(&self) -> impl std::ops::DerefMut<Target = Term<Listener>> + '_ {
        self.shared.term.lock()
    }

    pub(crate) fn take_events(&mut self) -> Vec<UiEvent> {
        let events = match self.listener.0.events.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(e) => {
                tracing::error!("term: event queue poisoned: {e}");
                return Vec::new();
            }
        };
        for ev in &events {
            match ev {
                // An empty title (screen, some prompts) means "no title".
                UiEvent::Title(t) => self.title = Some(t.clone()).filter(|t| !t.trim().is_empty()),
                UiEvent::ResetTitle => self.title = None,
                _ => {}
            }
        }
        events
    }

    /// Resize the grid and tell the PTY. `cell_px` is the cell size in
    /// physical pixels (for TIOCGWINSZ / CSI 14 t answers).
    pub(crate) fn resize(&mut self, cols: usize, rows: usize, cell_px: (u16, u16)) {
        let size = WindowSize {
            num_lines: rows as u16,
            num_cols: cols as u16,
            cell_width: cell_px.0,
            cell_height: cell_px.1,
        };
        tracing::debug!(
            cols,
            rows,
            cell_w = cell_px.0,
            cell_h = cell_px.1,
            "term.resize"
        );
        self.lock().resize(GridSize {
            columns: cols,
            lines: rows,
        });
        match self.listener.0.size.lock() {
            Ok(mut s) => *s = size,
            Err(e) => tracing::error!("term: size poisoned: {e}"),
        }
        self.pty.resize(size);
    }

    /// Flush a synchronized update whose timeout expired while no bytes
    /// arrived. Returns the time left on a pending one.
    pub(crate) fn poll_sync(&self) -> Option<Duration> {
        let Ok(mut parser) = self.shared.parser.try_lock() else {
            // The reader is mid-feed; it checks the deadline itself.
            return None;
        };
        let deadline = parser.sync_timeout().sync_timeout()?;
        let now = Instant::now();
        if now >= deadline {
            tracing::debug!("term: synchronized update timed out on the UI side, flushing");
            let mut term = self.shared.term.lock();
            parser.stop_sync(&mut Hooked {
                term: &mut term,
                listener: &self.shared.listener,
            });
            None
        } else {
            Some(deadline - now)
        }
    }
}

/// Paste text as input bytes: normalise newlines to CR and wrap in
/// bracketed-paste markers when the application asked for them.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalised = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // Strip the end marker so pasted text cannot break out of the bracket.
        let safe = normalised.replace("\x1b[201~", "");
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(safe.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalised.into_bytes()
    }
}

/// Lines in a paste, for the paste warning: `None` when the text has no
/// line break at all. A single line ending in a line break counts (as 1),
/// since the shell runs it the moment it lands. Trailing blank lines are
/// not counted.
pub fn multiline_paste_lines(text: &str) -> Option<usize> {
    if !text.contains(['\r', '\n']) {
        return None;
    }
    let trimmed = text.trim_end();
    Some(trimmed.replace("\r\n", "\n").split(['\r', '\n']).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Sink(Mutex<Vec<u8>>);

    impl Pty for Sink {
        fn write(&self, bytes: Cow<'static, [u8]>) {
            self.0.lock().unwrap().extend_from_slice(&bytes);
        }
        fn resize(&self, _: WindowSize) {}
    }

    fn setup() -> (Terminal, Feeder, Arc<Sink>) {
        let ctx = egui::Context::default();
        let sink = Arc::new(Sink::default());
        let (t, f) = Terminal::new(
            &ctx,
            sink.clone(),
            TermOptions::default(),
            Palette::default(),
        );
        (t, f, sink)
    }

    #[test]
    fn device_attributes_are_answered_through_the_pty() {
        let (_t, f, sink) = setup();
        f.feed(b"\x1b[c");
        let out = sink.0.lock().unwrap().clone();
        assert!(out.starts_with(b"\x1b[?6"), "unexpected DA answer {out:?}");
    }

    #[test]
    fn osc52_write_reaches_the_view_and_strips_nul() {
        let (mut t, f, _) = setup();
        // "hi\0there" base64
        f.feed(b"\x1b]52;c;aGkAdGhlcmU=\x07");
        assert_eq!(
            t.take_events(),
            vec![UiEvent::ClipboardStore("hithere".into())]
        );
    }

    #[test]
    fn osc52_read_is_refused() {
        let (mut t, f, sink) = setup();
        f.feed(b"\x1b]52;c;?\x07");
        assert!(t.take_events().is_empty());
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn osc52_disabled_by_option_and_live_toggle() {
        let ctx = egui::Context::default();
        let sink = Arc::new(Sink::default());
        let opts = TermOptions {
            osc52_write: false,
            ..Default::default()
        };
        let (mut t, f) = Terminal::new(&ctx, sink, opts, Palette::default());
        f.feed(b"\x1b]52;c;aGk=\x07");
        assert!(t.take_events().is_empty());
        t.set_osc52_write(true);
        f.feed(b"\x1b]52;c;aGk=\x07");
        assert_eq!(t.take_events(), vec![UiEvent::ClipboardStore("hi".into())]);
    }

    #[test]
    fn osc52_unpadded_wrapped_and_selectors() {
        let (mut t, f, _) = setup();
        // Unpadded (Rust STANDARD_NO_PAD / Go RawStdEncoding senders).
        f.feed(b"\x1b]52;c;aGVsbG8\x07");
        // Line-wrapped payload, ST terminator, primary selection.
        f.feed(b"\x1b]52;p;aGVs\nbG8=\x1b\\");
        // Empty selector means the clipboard.
        f.feed(b"\x1b]52;;aGk=\x07");
        // Cut buffer: ignored.
        f.feed(b"\x1b]52;0;aGk=\x07");
        assert_eq!(
            t.take_events(),
            vec![
                UiEvent::ClipboardStore("hello".into()),
                UiEvent::ClipboardStore("hello".into()),
                UiEvent::ClipboardStore("hi".into()),
            ]
        );
    }

    #[test]
    fn colour_queries_report_program_changes() {
        let (_t, f, sink) = setup();
        // Set the background, then ask for it and for palette entry 1.
        f.feed(b"\x1b]11;rgb:01/02/03\x07\x1b]11;?\x07\x1b]4;1;?\x1b\\");
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            out,
            "\x1b]11;rgb:0101/0202/0303\x07\x1b]4;1;rgb:f8f8/7171/7171\x1b\\"
        );
    }

    #[test]
    fn colour_answer_keeps_order_with_device_attributes() {
        // Programs send a colour query followed by DA1 and take the DA
        // answer as "no more replies coming"; the colour must come first.
        let (_t, f, sink) = setup();
        f.feed(b"\x1b]10;?\x07\x1b[c");
        let out = sink.0.lock().unwrap().clone();
        let colour = out
            .windows(4)
            .position(|w| w == b"]10;")
            .expect("colour answer");
        let da = out.windows(3).position(|w| w == b"[?6").expect("DA answer");
        assert!(colour < da, "{:?}", String::from_utf8_lossy(&out));
    }

    #[test]
    fn scrollback_resizes_live() {
        let (mut t, f, _) = setup();
        t.resize(20, 4, (8, 16));
        for i in 0..30 {
            f.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(t.lock().grid().history_size(), 27);
        t.set_scrollback(5);
        assert_eq!(t.options().scrollback, 5);
        assert_eq!(t.lock().grid().history_size(), 5, "oldest lines dropped");
        t.set_scrollback(100);
        for i in 0..30 {
            f.feed(format!("more {i}\r\n").as_bytes());
        }
        assert_eq!(
            t.lock().grid().history_size(),
            35,
            "grows again up to the new limit"
        );
    }

    #[test]
    fn select_all_and_clear_scrollback() {
        let (mut t, f, _) = setup();
        t.resize(20, 4, (8, 16));
        f.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n$ ");
        t.select_all();
        let all = t.selection_text().unwrap();
        assert!(all.starts_with("one") && all.contains("five"), "{all:?}");

        t.clear_scrollback();
        assert!(!t.has_selection());
        let tt = t.lock();
        assert_eq!(tt.grid().history_size(), 0);
        // The prompt line is now the first line.
        assert_eq!(tt.grid().cursor.point.line.0, 0);
        assert_eq!(tt.grid().cursor.point.column.0, 2);
        let first = tt.bounds_to_string(
            Point::new(alacritty_terminal::index::Line(0), Column(0)),
            Point::new(alacritty_terminal::index::Line(0), Column(19)),
        );
        assert_eq!(first.trim_end(), "$");
    }

    #[test]
    fn paste_through_the_terminal() {
        let (mut t, f, sink) = setup();
        t.paste("a\r\nb\n");
        f.feed(b"\x1b[?2004h");
        t.paste("c\nd");
        assert_eq!(
            sink.0.lock().unwrap().as_slice(),
            b"a\rb\r\x1b[200~c\rd\x1b[201~"
        );
    }

    #[test]
    fn paste_normalises_newlines_and_guards_the_bracket() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc".to_vec());
        assert_eq!(
            paste_bytes("x\x1b[201~y", true),
            b"\x1b[200~xy\x1b[201~".to_vec()
        );
    }

    #[test]
    fn multiline_paste_detection() {
        assert_eq!(multiline_paste_lines("ls -la"), None);
        assert_eq!(multiline_paste_lines("ls -la  "), None);
        // A trailing newline runs the line, so it counts.
        assert_eq!(multiline_paste_lines("ls -la\n"), Some(1));
        assert_eq!(multiline_paste_lines("ls -la\r\n\n"), Some(1));
        assert_eq!(multiline_paste_lines("a\nb"), Some(2));
        assert_eq!(multiline_paste_lines("a\r\nb\rc\n\n"), Some(3));
    }

    #[test]
    fn cursor_option_sets_the_default_style() {
        let ctx = egui::Context::default();
        let opts = TermOptions {
            cursor: CursorKind::Bar,
            cursor_blink: false,
            ..Default::default()
        };
        let (mut t, _f) = Terminal::new(&ctx, Arc::new(Sink::default()), opts, Palette::default());
        assert_eq!(t.lock().cursor_style().shape, CursorShape::Beam);
        assert!(!t.lock().cursor_style().blinking);
        t.set_cursor(CursorKind::Underline, true);
        assert_eq!(t.lock().cursor_style().shape, CursorShape::Underline);
        assert!(t.lock().cursor_style().blinking);
        assert_eq!(CursorKind::from_setting("bar"), CursorKind::Bar);
    }

    #[test]
    fn title_and_bell() {
        let (mut t, f, _) = setup();
        f.feed(b"\x1b]0;hello\x07\x07");
        let ev = t.take_events();
        assert_eq!(ev, vec![UiEvent::Title("hello".into()), UiEvent::Bell]);
        assert_eq!(t.title(), Some("hello"));
    }

    #[test]
    fn colour_query_answered_from_palette() {
        let (_t, f, sink) = setup();
        f.feed(b"\x1b]11;?\x07");
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        // ezterm-dark background #121214.
        assert!(
            out.contains("rgb:1212/1212/1414"),
            "unexpected answer {out:?}"
        );
    }

    #[test]
    fn kitty_keyboard_flags_reach_term_mode() {
        let (t, f, _) = setup();
        f.feed(b"\x1b[>1u");
        let mode = *t.lock().mode();
        assert_eq!(
            crate::keys::Modes::from_term(mode).kitty,
            crate::keys::KITTY_DISAMBIGUATE
        );
    }
}
