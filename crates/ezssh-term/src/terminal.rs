//! The terminal model shared between a byte source (SSH channel reader,
//! local PTY reader) and the egui view.
//!
//! Bytes do not go through the UI event queue: the reader calls
//! [`Feeder::feed`], which runs the VT parser straight into the locked
//! `Term` and asks egui for a repaint. The UI thread locks the `Term`
//! only to paint, resize, select or scroll. This is the arrangement Zed
//! uses and replaces the IPC channel / arm-detach buffering / stall
//! watchdog of the webview build.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term};
use alacritty_terminal::vte::ansi::Processor;

use crate::palette::Palette;

/// Where keystrokes, query answers and size changes go. Implementations
/// must not block: queue the bytes and return.
pub trait Pty: Send + Sync + 'static {
    fn write(&self, bytes: Cow<'static, [u8]>);
    fn resize(&self, size: WindowSize);
}

/// OSC 52 writes larger than this are dropped (same cap as `osc52.ts`).
pub const OSC52_MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TermOptions {
    pub scrollback: usize,
    /// Allow programs to set the clipboard with OSC 52. Reads are always refused.
    pub osc52_write: bool,
}

impl Default for TermOptions {
    fn default() -> Self {
        Self { scrollback: 10_000, osc52_write: true }
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
    osc52_write: bool,
}

/// alacritty `EventListener`. Called with the `Term` lock held, so it
/// never locks the `Term` itself.
#[derive(Clone)]
pub struct Listener(Arc<ListenerInner>);

impl Listener {
    fn push(&self, ev: UiEvent) {
        match self.0.events.lock() {
            Ok(mut q) => q.push(ev),
            Err(e) => tracing::error!("term.listener: event queue poisoned: {e}"),
        }
        self.0.ctx.request_repaint();
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let inner = &self.0;
        match event {
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => inner.ctx.request_repaint(),
            Event::PtyWrite(text) => {
                tracing::trace!(len = text.len(), "term.listener: pty write (query answer)");
                inner.pty.write(Cow::Owned(text.into_bytes()));
            }
            Event::ColorRequest(index, format) => {
                // Answers from the palette. Colours changed by OSC 4/10/11
                // live in the locked Term and are not visible here; phase 3
                // moves the answer to the view if a program needs them.
                let rgb = match inner.palette.lock() {
                    Ok(p) => p.by_index(index),
                    Err(e) => {
                        tracing::error!("term.listener: palette poisoned: {e}");
                        return;
                    }
                };
                inner.pty.write(Cow::Owned(format(rgb).into_bytes()));
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
                if !inner.osc52_write {
                    tracing::debug!(?ty, "term.osc52: write refused by setting");
                    return;
                }
                if text.len() > OSC52_MAX_BYTES {
                    tracing::warn!(?ty, len = text.len(), "term.osc52: write over size cap dropped");
                    return;
                }
                let text = text.replace('\0', "");
                tracing::debug!(?ty, len = text.len(), "term.osc52: clipboard write");
                self.push(UiEvent::ClipboardStore(text));
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

/// Throughput counters, for the spike's gate measurements.
#[derive(Default)]
pub struct Stats {
    pub bytes: AtomicU64,
    pub feed_nanos: AtomicU64,
    pub feeds: AtomicU64,
}

struct Shared {
    term: FairMutex<Term<Listener>>,
    parser: Mutex<Processor>,
    ctx: egui::Context,
    stats: Stats,
    exit: Mutex<Option<String>>,
}

/// Handed to the byte source. Cheap to clone.
#[derive(Clone)]
pub struct Feeder(Arc<Shared>);

impl Feeder {
    /// Run `bytes` through the parser into the terminal and request a repaint.
    pub fn feed(&self, bytes: &[u8]) {
        let started = Instant::now();
        tracing::trace!(len = bytes.len(), head = %bytes[..bytes.len().min(160)].escape_ascii(), "term.feed");
        {
            let mut parser = match self.0.parser.lock() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("term.feed: parser poisoned, dropping {} bytes: {e}", bytes.len());
                    return;
                }
            };
            let mut term = self.0.term.lock();
            // A synchronized update (DECSET 2026) that outlived its timeout
            // is flushed before new bytes, so output never stalls behind it.
            if parser.sync_timeout().sync_timeout().is_some_and(|d| Instant::now() >= d) {
                tracing::debug!("term.feed: synchronized update timed out, flushing");
                parser.stop_sync(&mut *term);
            }
            parser.advance(&mut *term, bytes);
        }
        let s = &self.0.stats;
        s.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        s.feeds.fetch_add(1, Ordering::Relaxed);
        s.feed_nanos.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.0.ctx.request_repaint();
    }

    /// The byte source ended (shell exited, channel closed, error).
    pub fn exited(&self, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::info!(%reason, "term.feed: source ended");
        match self.0.exit.lock() {
            Ok(mut e) => *e = Some(reason),
            Err(e) => tracing::error!("term.feed: exit slot poisoned: {e}"),
        }
        self.0.ctx.request_repaint();
    }
}

pub struct Terminal {
    shared: Arc<Shared>,
    listener: Listener,
    pty: Arc<dyn Pty>,
    title: Option<String>,
    pub(crate) view: crate::view::ViewState,
}

impl Terminal {
    /// Create a terminal writing to `pty`. Returns the feeder the byte
    /// source pushes output into.
    pub fn new(ctx: &egui::Context, pty: Arc<dyn Pty>, options: TermOptions, palette: Palette) -> (Terminal, Feeder) {
        let size = GridSize { columns: 80, lines: 24 };
        let listener = Listener(Arc::new(ListenerInner {
            ctx: ctx.clone(),
            pty: pty.clone(),
            palette: Mutex::new(palette),
            size: Mutex::new(WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 }),
            events: Mutex::new(Vec::new()),
            osc52_write: options.osc52_write,
        }));
        let config = Config {
            scrolling_history: options.scrollback,
            kitty_keyboard: true,
            osc52: if options.osc52_write { Osc52::OnlyCopy } else { Osc52::Disabled },
            ..Config::default()
        };
        let term = Term::new(config, &size, listener.clone());
        let shared = Arc::new(Shared {
            term: FairMutex::new(term),
            parser: Mutex::new(Processor::new()),
            ctx: ctx.clone(),
            stats: Stats::default(),
            exit: Mutex::new(None),
        });
        let feeder = Feeder(shared.clone());
        (Terminal { shared, listener, pty, title: None, view: crate::view::ViewState::default() }, feeder)
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.pty.write(bytes.into());
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn exit_reason(&self) -> Option<String> {
        match self.shared.exit.lock() {
            Ok(e) => e.clone(),
            Err(e) => {
                tracing::error!("term: exit slot poisoned: {e}");
                None
            }
        }
    }

    pub fn stats(&self) -> &Stats {
        &self.shared.stats
    }

    pub fn view_state(&self) -> &crate::view::ViewState {
        &self.view
    }

    pub fn grid_size(&self) -> (usize, usize) {
        (self.view.cols, self.view.rows)
    }

    pub fn set_palette(&self, palette: Palette) {
        match self.listener.0.palette.lock() {
            Ok(mut p) => *p = palette,
            Err(e) => tracing::error!("term: palette poisoned: {e}"),
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
        tracing::debug!(cols, rows, cell_w = cell_px.0, cell_h = cell_px.1, "term.resize");
        self.lock().resize(GridSize { columns: cols, lines: rows });
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
            parser.stop_sync(&mut *term);
            None
        } else {
            Some(deadline - now)
        }
    }
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
        let (t, f) = Terminal::new(&ctx, sink.clone(), TermOptions::default(), Palette::default());
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
        assert_eq!(t.take_events(), vec![UiEvent::ClipboardStore("hithere".into())]);
    }

    #[test]
    fn osc52_read_is_refused() {
        let (mut t, f, sink) = setup();
        f.feed(b"\x1b]52;c;?\x07");
        assert!(t.take_events().is_empty());
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn osc52_disabled_by_option() {
        let ctx = egui::Context::default();
        let sink = Arc::new(Sink::default());
        let opts = TermOptions { osc52_write: false, ..Default::default() };
        let (mut t, f) = Terminal::new(&ctx, sink, opts, Palette::default());
        f.feed(b"\x1b]52;c;aGk=\x07");
        assert!(t.take_events().is_empty());
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
        assert!(out.contains("rgb:1212/1212/1414"), "unexpected answer {out:?}");
    }

    #[test]
    fn kitty_keyboard_flags_reach_term_mode() {
        let (t, f, _) = setup();
        f.feed(b"\x1b[>1u");
        let mode = *t.lock().mode();
        assert_eq!(crate::keys::Modes::from_term(mode).kitty, crate::keys::KITTY_DISAMBIGUATE);
    }
}
