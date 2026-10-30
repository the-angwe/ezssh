//! egui widget: paints the grid and turns egui input into PTY bytes.

use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape, Rgb};
use egui::{
    Align2, Color32, CursorIcon, Event, Mesh, Shape, EventFilter, FontFamily, FontId, ImeEvent, MouseWheelUnit,
    PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::boxdraw;
use crate::keys::{self, Action, Key, Mods, Modes};
use crate::mouse;
use crate::palette::{Palette, dim};
use crate::terminal::{Terminal, UiEvent};

pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 48.0;
const BELL_FLASH: Duration = Duration::from_millis(120);
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// Look of the widget. Owned by the app, shared by every tab.
#[derive(Clone, Debug)]
pub struct TermStyle {
    pub font_size: f32,
    pub family: FontFamily,
    /// Family with the bold face of the same font, if one was found.
    pub bold_family: Option<FontFamily>,
    pub padding: f32,
    pub palette: Palette,
    pub copy_on_select: bool,
    /// Draw box-drawing and block characters as shapes instead of glyphs.
    pub builtin_box_drawing: bool,
}

impl Default for TermStyle {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            family: FontFamily::Monospace,
            bold_family: None,
            padding: 4.0,
            palette: Palette::default(),
            copy_on_select: false,
            builtin_box_drawing: true,
        }
    }
}

/// Per-terminal UI state.
#[derive(Default)]
pub struct ViewState {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    cell: Vec2,
    origin: Pos2,
    selecting: bool,
    last_click: Option<(Instant, Point, u8)>,
    /// Buttons currently reported to the application (mouse mode).
    reported_button: Option<mouse::Button>,
    last_motion_cell: Option<(usize, usize)>,
    scroll_accum: f32,
    zoom_accum: f32,
    bell_at: Option<Instant>,
    ime_preedit: String,
    focused_last: bool,
    clipboard: Option<arboard::Clipboard>,
    /// Paint time of the last frame, for the spike's measurements.
    pub(crate) last_paint: Duration,
}

impl ViewState {
    pub fn last_paint(&self) -> Duration {
        self.last_paint
    }
}

pub struct TerminalView<'a> {
    term: &'a mut Terminal,
    style: &'a mut TermStyle,
    focus_on_show: bool,
}

impl<'a> TerminalView<'a> {
    pub fn new(term: &'a mut Terminal, style: &'a mut TermStyle) -> Self {
        Self { term, style, focus_on_show: false }
    }

    /// Take keyboard focus this frame (new tab, tab switch).
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus_on_show = focus;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let TerminalView { term, style, focus_on_show } = self;
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let id = response.id;
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();

        if focus_on_show || response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                )
            });
        }

        // Cell metrics.
        let font = FontId::new(style.font_size, style.family.clone());
        let (glyph_w, row_h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        // Width stays exact so a run of ASCII glyphs laid out as one galley
        // lines up with the grid; height snaps to device pixels so rows and
        // box-drawing joins are crisp.
        let cell = vec2(glyph_w.max(1.0), ((row_h * ppp).ceil() / ppp).max(1.0));
        let inner = rect.shrink(style.padding);
        let cols = ((inner.width() / cell.x).floor() as usize).max(2);
        let rows = ((inner.height() / cell.y).floor() as usize).max(1);
        let origin = pos2((inner.min.x * ppp).round() / ppp, (inner.min.y * ppp).round() / ppp);
        {
            let v = &mut term.view;
            v.cell = cell;
            v.origin = origin;
        }
        if (cols, rows) != (term.view.cols, term.view.rows) {
            term.view.cols = cols;
            term.view.rows = rows;
            let cell_px = ((cell.x * ppp).round() as u16, (cell.y * ppp).round() as u16);
            term.resize(cols, rows, cell_px);
        }

        // Model-side events (title, bell, OSC 52).
        for ev in term.take_events() {
            match ev {
                UiEvent::Bell => term.view.bell_at = Some(Instant::now()),
                UiEvent::ClipboardStore(text) => ctx.copy_text(text),
                UiEvent::Title(_) | UiEvent::ResetTitle => {}
            }
        }
        if let Some(left) = term.poll_sync() {
            ctx.request_repaint_after(left);
        }

        // Focus reporting (DECSET 1004).
        if focused != term.view.focused_last {
            term.view.focused_last = focused;
            let mode = *term.lock().mode();
            term.lock().is_focused = focused;
            if mode.contains(TermMode::FOCUS_IN_OUT) {
                term.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
            }
        }

        let events = ui.input(|i| i.events.clone());
        let hovered = response.hovered() || response.contains_pointer();
        handle_pointer(term, style, &events, rect, hovered, &ctx);
        if focused {
            handle_keyboard(term, &events, &ctx);
        }

        let paint_started = Instant::now();
        paint(ui, term, style, rect, focused, &font, ppp);
        term.view.last_paint = paint_started.elapsed();

        // IME: tell the platform where the cursor is so candidate windows
        // open next to it.
        if focused {
            let cursor_rect = {
                let t = term.lock();
                let c = t.grid().cursor.point;
                let row = (c.line.0 + t.grid().display_offset() as i32).max(0) as f32;
                Rect::from_min_size(origin + vec2(c.column.0 as f32 * cell.x, row * cell.y), cell)
            };
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Terminal,
                    rect,
                    cursor_rect,
                    should_interrupt_composition: false,
                })
            });
        }

        let mouse_mode = term.lock().mode().intersects(TermMode::MOUSE_MODE);
        if hovered {
            ctx.set_cursor_icon(if mouse_mode && !ui.input(|i| i.modifiers.shift) {
                CursorIcon::Default
            } else {
                CursorIcon::Text
            });
        }
        response
    }
}

/// Breaks an IME feedback loop seen with egui-winit 0.37 on Wayland
/// (GNOME / IBus): egui-winit re-sends the IME cursor area on every frame
/// that had input, the compositor answers each one with an empty pre-edit
/// event, and that event makes the next frame. The result is a constant
/// repaint at the display rate while idle (~9 % of a core here).
///
/// Call [`ImeLoopFilter::filter`] from `eframe::App::raw_input_hook`. It
/// drops empty pre-edits unless a composition is in progress, where an
/// empty pre-edit means "composition cancelled" and is kept.
#[derive(Debug, Default)]
pub struct ImeLoopFilter {
    composing: bool,
    dropped: u64,
}

impl ImeLoopFilter {
    pub fn filter(&mut self, raw: &mut egui::RawInput) {
        raw.events.retain(|e| match e {
            Event::Ime(ImeEvent::Preedit { text, .. }) if text.is_empty() => {
                let keep = std::mem::take(&mut self.composing);
                if !keep {
                    self.dropped += 1;
                    if self.dropped.is_power_of_two() {
                        tracing::debug!(dropped = self.dropped, "term.ime: dropped empty pre-edit events");
                    }
                }
                keep
            }
            Event::Ime(ImeEvent::Preedit { .. }) => {
                self.composing = true;
                true
            }
            Event::Ime(ImeEvent::Commit(_)) => {
                self.composing = false;
                true
            }
            _ => true,
        });
    }
}

// ---------------------------------------------------------------- input

fn cell_at(v: &ViewState, pos: Pos2) -> (usize, usize, Side) {
    let rel = pos - v.origin;
    let fx = rel.x / v.cell.x;
    let col = (fx.floor().max(0.0) as usize).min(v.cols.saturating_sub(1));
    let row = ((rel.y / v.cell.y).floor().max(0.0) as usize).min(v.rows.saturating_sub(1));
    let side = if fx - fx.floor() < 0.5 { Side::Left } else { Side::Right };
    (col, row, side)
}

fn grid_point(display_offset: usize, col: usize, row: usize) -> Point {
    Point::new(Line(row as i32 - display_offset as i32), Column(col))
}

fn mouse_format(mode: TermMode) -> mouse::Format {
    if mode.contains(TermMode::SGR_MOUSE) {
        mouse::Format::Sgr
    } else if mode.contains(TermMode::UTF8_MOUSE) {
        mouse::Format::Utf8
    } else {
        mouse::Format::Normal
    }
}

fn read_clipboard(term: &mut Terminal) -> Option<String> {
    if term.view.clipboard.is_none() {
        match arboard::Clipboard::new() {
            Ok(c) => term.view.clipboard = Some(c),
            Err(e) => {
                tracing::error!("term.paste: clipboard unavailable: {e}");
                return None;
            }
        }
    }
    let cb = term.view.clipboard.as_mut()?;
    match cb.get_text() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!("term.paste: clipboard read failed: {e}");
            None
        }
    }
}

/// Paste text: normalise newlines to CR and wrap in bracketed-paste
/// markers when the application asked for them.
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

fn paste(term: &mut Terminal, text: &str) {
    if text.is_empty() {
        return;
    }
    let bracketed = term.lock().mode().contains(TermMode::BRACKETED_PASTE);
    tracing::debug!(len = text.len(), bracketed, "term.paste");
    let bytes = paste_bytes(text, bracketed);
    scroll_to_bottom(term);
    term.write(bytes);
}

fn copy_selection(term: &mut Terminal, ctx: &egui::Context) -> bool {
    let text = term.lock().selection_to_string();
    match text {
        Some(t) if !t.is_empty() => {
            tracing::debug!(len = t.len(), "term.copy");
            ctx.copy_text(t);
            true
        }
        _ => false,
    }
}

fn scroll_to_bottom(term: &mut Terminal) {
    let mut t = term.lock();
    if t.grid().display_offset() != 0 {
        t.scroll_display(Scroll::Bottom);
    }
}

fn send_key_bytes(term: &mut Terminal, bytes: Vec<u8>) {
    scroll_to_bottom(term);
    term.write(bytes);
}

fn handle_keyboard(term: &mut Terminal, events: &[Event], ctx: &egui::Context) {
    let mut skip_text_at: Option<usize> = None;
    for (i, ev) in events.iter().enumerate() {
        match ev {
            Event::Key { key, pressed, repeat, modifiers, .. } => {
                let mods = Mods::from_egui(*modifiers);
                if *pressed {
                    // Terminal shortcuts (webview build parity).
                    let ctrl_shift = mods.ctrl && mods.shift && !mods.alt;
                    match key {
                        egui::Key::C if ctrl_shift => {
                            copy_selection(term, ctx);
                            continue;
                        }
                        egui::Key::V if ctrl_shift => {
                            if let Some(t) = read_clipboard(term) {
                                paste(term, &t);
                            }
                            continue;
                        }
                        egui::Key::Insert if mods.shift && !mods.ctrl => {
                            if let Some(t) = read_clipboard(term) {
                                paste(term, &t);
                            }
                            continue;
                        }
                        egui::Key::Insert if mods.ctrl && !mods.shift => {
                            copy_selection(term, ctx);
                            continue;
                        }
                        egui::Key::PageUp | egui::Key::PageDown | egui::Key::Home | egui::Key::End
                            if mods.shift && !mods.ctrl && !mods.alt =>
                        {
                            let mut t = term.lock();
                            if !t.mode().contains(TermMode::ALT_SCREEN) {
                                t.scroll_display(match key {
                                    egui::Key::PageUp => Scroll::PageUp,
                                    egui::Key::PageDown => Scroll::PageDown,
                                    egui::Key::Home => Scroll::Top,
                                    _ => Scroll::Bottom,
                                });
                                continue;
                            }
                        }
                        _ => {}
                    }
                }
                let Some(k) = Key::from_egui(*key) else {
                    continue;
                };
                let action = match (pressed, repeat) {
                    (false, _) => Action::Release,
                    (true, true) => Action::Repeat,
                    (true, false) => Action::Press,
                };
                let modes = Modes::from_term(*term.lock().mode());
                // The text this key typed, if egui delivers it right after.
                let next_text = match events.get(i + 1) {
                    Some(Event::Text(t)) if *pressed => Some(t.as_str()),
                    _ => None,
                };
                if let Some(bytes) = keys::encode(k, mods, action, next_text, modes) {
                    tracing::trace!(?k, ?mods, ?action, len = bytes.len(), "term.key");
                    if k.is_text() && next_text.is_some() {
                        skip_text_at = Some(i + 1);
                    }
                    send_key_bytes(term, bytes);
                }
            }
            Event::Text(text) => {
                if skip_text_at == Some(i) {
                    continue;
                }
                send_key_bytes(term, text.as_bytes().to_vec());
            }
            Event::Ime(ime) => match ime {
                ImeEvent::Preedit { text, .. } => term.view.ime_preedit = text.clone(),
                ImeEvent::Commit(text) => {
                    term.view.ime_preedit.clear();
                    if !text.is_empty() {
                        send_key_bytes(term, text.as_bytes().to_vec());
                    }
                }
                _ => {}
            },
            Event::Paste(text) => paste(term, text),
            Event::Copy => {
                copy_selection(term, ctx);
            }
            _ => {}
        }
    }
}

fn handle_pointer(
    term: &mut Terminal,
    style: &mut TermStyle,
    events: &[Event],
    rect: Rect,
    hovered: bool,
    ctx: &egui::Context,
) {
    for ev in events {
        match ev {
            Event::PointerButton { pos, button, pressed, modifiers } => {
                if *pressed && !rect.contains(*pos) {
                    continue;
                }
                if !*pressed && !term.view.selecting && term.view.reported_button.is_none() {
                    continue;
                }
                let mods = Mods::from_egui(*modifiers);
                let (col, row, side) = cell_at(&term.view, *pos);
                let mode = *term.lock().mode();
                let reporting = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;
                let mbutton = match button {
                    PointerButton::Primary => mouse::Button::Left,
                    PointerButton::Middle => mouse::Button::Middle,
                    PointerButton::Secondary => mouse::Button::Right,
                    _ => continue,
                };
                if reporting || (!*pressed && term.view.reported_button == Some(mbutton)) {
                    let kind = if *pressed { mouse::Kind::Press } else { mouse::Kind::Release };
                    term.view.reported_button = if *pressed { Some(mbutton) } else { None };
                    term.view.last_motion_cell = Some((col, row));
                    if let Some(bytes) = mouse::encode(mbutton, kind, col, row, mods, mouse_format(mode)) {
                        term.write(bytes);
                    }
                    continue;
                }
                match (button, pressed) {
                    (PointerButton::Primary, true) => {
                        let now = Instant::now();
                        let mut t = term.lock();
                        let point = grid_point(t.grid().display_offset(), col, row);
                        let extend = mods.shift && t.selection.is_some();
                        let count = match term.view.last_click {
                            Some((at, p, n)) if now - at < MULTI_CLICK && p == point => n % 3 + 1,
                            _ => 1,
                        };
                        if let Some(sel) = t.selection.as_mut().filter(|_| extend) {
                            sel.update(point, side);
                        } else {
                            let ty = match count {
                                2 => SelectionType::Semantic,
                                3 => SelectionType::Lines,
                                _ => SelectionType::Simple,
                            };
                            t.selection = Some(Selection::new(ty, point, side));
                        }
                        drop(t);
                        if !extend {
                            term.view.last_click = Some((now, point, count));
                        }
                        term.view.selecting = true;
                    }
                    (PointerButton::Primary, false) => {
                        term.view.selecting = false;
                        let empty = term.lock().selection.as_ref().is_some_and(|s| s.is_empty());
                        if empty {
                            term.lock().selection = None;
                        } else if style.copy_on_select {
                            copy_selection(term, ctx);
                        }
                    }
                    (PointerButton::Secondary, true) => {
                        // Right-click pastes (the `terminal.rightClick = paste`
                        // behaviour). The context menu comes with phase 3.
                        if let Some(t) = read_clipboard(term) {
                            paste(term, &t);
                        }
                    }
                    _ => {}
                }
            }
            Event::PointerMoved(pos) => {
                let mode = *term.lock().mode();
                if term.view.selecting {
                    let (col, row, side) = cell_at(&term.view, *pos);
                    let mut t = term.lock();
                    // Drag past the top / bottom edge scrolls the history.
                    if pos.y < rect.top() {
                        t.scroll_display(Scroll::Delta(1));
                        ctx.request_repaint();
                    } else if pos.y > rect.bottom() {
                        t.scroll_display(Scroll::Delta(-1));
                        ctx.request_repaint();
                    }
                    let point = grid_point(t.grid().display_offset(), col, row);
                    if let Some(sel) = t.selection.as_mut() {
                        sel.update(point, side);
                    }
                    continue;
                }
                let motion = mode.contains(TermMode::MOUSE_MOTION)
                    || (mode.contains(TermMode::MOUSE_DRAG) && term.view.reported_button.is_some());
                if motion && (rect.contains(*pos) || term.view.reported_button.is_some()) {
                    let (col, row, _) = cell_at(&term.view, *pos);
                    if term.view.last_motion_cell == Some((col, row)) {
                        continue;
                    }
                    term.view.last_motion_cell = Some((col, row));
                    let button = term.view.reported_button.unwrap_or(mouse::Button::None);
                    let mods = Mods::from_egui(ctx.input(|i| i.modifiers));
                    if let Some(bytes) = mouse::encode(button, mouse::Kind::Motion, col, row, mods, mouse_format(mode)) {
                        term.write(bytes);
                    }
                }
            }
            Event::MouseWheel { unit, delta, modifiers, .. } if hovered => {
                let mods = Mods::from_egui(*modifiers);
                let lines = match unit {
                    MouseWheelUnit::Line => delta.y,
                    MouseWheelUnit::Point => delta.y / term.view.cell.y.max(1.0),
                    MouseWheelUnit::Page => delta.y * term.view.rows as f32,
                };
                if mods.ctrl {
                    // Ctrl+wheel zoom, one point per notch.
                    term.view.zoom_accum += lines;
                    let steps = term.view.zoom_accum.trunc();
                    if steps != 0.0 {
                        term.view.zoom_accum -= steps;
                        style.font_size = (style.font_size + steps).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                        tracing::debug!(size = style.font_size, "term.zoom");
                    }
                    continue;
                }
                term.view.scroll_accum += lines;
                let n = term.view.scroll_accum.trunc() as i32;
                if n == 0 {
                    continue;
                }
                term.view.scroll_accum -= n as f32;
                let mode = *term.lock().mode();
                if mode.intersects(TermMode::MOUSE_MODE) && !mods.shift {
                    let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap_or(rect.center());
                    let (col, row, _) = cell_at(&term.view, pos);
                    let button = if n > 0 { mouse::Button::WheelUp } else { mouse::Button::WheelDown };
                    for _ in 0..n.unsigned_abs() {
                        if let Some(b) = mouse::encode(button, mouse::Kind::Press, col, row, mods, mouse_format(mode)) {
                            term.write(b);
                        }
                    }
                } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !mods.shift {
                    let seq: &[u8] = match (n > 0, mode.contains(TermMode::APP_CURSOR)) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    term.write(seq.repeat(n.unsigned_abs() as usize));
                } else {
                    term.lock().scroll_display(Scroll::Delta(n));
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- paint

fn to_c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn resolve(color: Color, colors: &Colors, palette: &Palette) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => colors[i as usize].unwrap_or_else(|| palette.indexed(i)),
        Color::Named(n) => colors[n as usize].unwrap_or_else(|| palette.named(n)),
    }
}

/// Bold text in one of the first eight colours uses the bright variant;
/// dim text uses the dim variant.
fn adjust_fg(color: Color, flags: Flags) -> Color {
    match color {
        Color::Named(n) if flags.contains(Flags::BOLD) && (n as usize) < 8 => {
            Color::Indexed(n as u8 + 8)
        }
        Color::Indexed(i) if flags.contains(Flags::BOLD) && i < 8 => Color::Indexed(i + 8),
        _ => color,
    }
}

struct PCell {
    col: usize,
    c: char,
    extra: Option<Vec<char>>,
    fg: Color32,
    bg: Rgb,
    flags: Flags,
}

fn is_simple(c: char) -> bool {
    c.is_ascii_graphic() || c == ' '
}

fn paint(ui: &Ui, term: &mut Terminal, style: &TermStyle, rect: Rect, focused: bool, font: &FontId, ppp: f32) {
    let painter = ui.painter_at(rect);
    let palette = &style.palette;
    let default_bg = palette.background;
    painter.rect_filled(rect, 0.0, to_c32(default_bg));

    let v = &term.view;
    let (cell, origin, cols, rows) = (v.cell, v.origin, v.cols, v.rows);
    let bold_font = style.bold_family.as_ref().map(|f| FontId::new(font.size, f.clone()));

    // Snapshot the visible cells, then release the lock before drawing.
    let mut lines: Vec<Vec<PCell>> = (0..rows).map(|_| Vec::with_capacity(cols)).collect();
    let (cursor_cell, cursor_shape, cursor_wide);
    {
        let t = term.lock();
        let content = t.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset as i32;
        let selection = content.selection;
        cursor_shape = content.cursor.shape;
        let cp = content.cursor.point;
        let cursor_row = cp.line.0 + offset;
        cursor_cell = (cursor_row >= 0 && (cursor_row as usize) < rows).then_some((cp.column.0, cursor_row as usize));
        cursor_wide = t.grid()[cp].flags.contains(Flags::WIDE_CHAR);
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + offset;
            if row < 0 || row as usize >= rows {
                continue;
            }
            let c = indexed.cell;
            if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                // Its background is painted with the wide character.
                continue;
            }
            let mut fg = resolve(adjust_fg(c.fg, c.flags), colors, palette);
            let mut bg = resolve(c.bg, colors, palette);
            if c.flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            if c.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if c.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if selection.is_some_and(|s| s.contains(indexed.point)) {
                bg = palette.selection_background;
            }
            lines[row as usize].push(PCell {
                col: indexed.point.column.0,
                c: c.c,
                extra: c.zerowidth().map(|z| z.to_vec()),
                fg: to_c32(fg),
                bg,
                flags: c.flags,
            });
        }
    }

    let focused_block = focused && cursor_shape == CursorShape::Block;
    let cell_rect = |col: usize, row: usize, span: usize| {
        Rect::from_min_size(origin + vec2(col as f32 * cell.x, row as f32 * cell.y), vec2(cell.x * span as f32, cell.y))
    };
    let span_of = |p: &PCell| if p.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
    // Backgrounds go into one un-feathered mesh with edges on whole device
    // pixels, so neighbouring runs meet without seams.
    let snap = |v: f32| (v * ppp).round() / ppp;
    let snapped = |r: Rect| Rect::from_min_max(pos2(snap(r.min.x), snap(r.min.y)), pos2(snap(r.max.x), snap(r.max.y)));
    let mut bg_mesh = Mesh::default();

    // Backgrounds, merged into runs.
    for (row, line) in lines.iter().enumerate() {
        let mut run: Option<(usize, usize, Rgb)> = None;
        for p in line {
            let end = p.col + span_of(p);
            match &mut run {
                Some((_, run_end, bg)) if *bg == p.bg && *run_end == p.col => *run_end = end,
                _ => {
                    if let Some((s, e, bg)) = run.take()
                        && bg != default_bg
                    {
                        bg_mesh.add_colored_rect(snapped(cell_rect(s, row, e - s)), to_c32(bg));
                    }
                    run = Some((p.col, end, p.bg));
                }
            }
        }
        if let Some((s, e, bg)) = run
            && bg != default_bg
        {
            bg_mesh.add_colored_rect(snapped(cell_rect(s, row, e - s)), to_c32(bg));
        }
    }

    // Block cursor background goes under the text.
    if let Some((ccol, crow)) = cursor_cell
        && focused_block
    {
        let span = if cursor_wide { 2 } else { 1 };
        bg_mesh.add_colored_rect(snapped(cell_rect(ccol, crow, span)), to_c32(palette.cursor));
    }
    painter.add(Shape::mesh(bg_mesh));

    // Text. Box-drawing rectangles collect in their own mesh.
    let mut fg_mesh = Mesh::default();
    let box_stroke = boxdraw::stroke_px(cell.x, ppp);
    for (row, line) in lines.iter().enumerate() {
        let y = origin.y + row as f32 * cell.y;
        let mut run = String::new();
        let mut run_start = 0usize;
        let mut run_key: Option<(Color32, bool)> = None;
        let flush = |run: &mut String, start: usize, key: Option<(Color32, bool)>| {
            if let Some((fg, bold)) = key
                && !run.trim_end().is_empty()
            {
                let f = if bold { bold_font.as_ref().unwrap_or(font) } else { font };
                painter.text(pos2(origin.x + start as f32 * cell.x, y), Align2::LEFT_TOP, run.trim_end(), f.clone(), fg);
            }
            run.clear();
        };
        for p in line {
            let under_cursor = focused_block && cursor_cell == Some((p.col, row));
            let fg = if under_cursor { to_c32(palette.cursor_accent) } else { p.fg };
            let bold = p.flags.contains(Flags::BOLD);
            let span = span_of(p);

            if style.builtin_box_drawing && boxdraw::handles(p.c) {
                flush(&mut run, run_start, run_key.take());
                boxdraw::draw(&mut fg_mesh, &painter, cell_rect(p.col, row, span), p.c, fg, ppp, box_stroke);
                continue;
            }
            if is_simple(p.c) && p.extra.is_none() && span == 1 {
                let key = Some((fg, bold));
                if run_key != key || run_start + run.chars().count() != p.col {
                    flush(&mut run, run_start, run_key.take());
                    run_start = p.col;
                    run_key = key;
                }
                run.push(p.c);
                continue;
            }
            flush(&mut run, run_start, run_key.take());
            // Non-ASCII, wide or combining: one galley, centred in its cells.
            let mut s = String::new();
            s.push(p.c);
            if let Some(extra) = &p.extra {
                s.extend(extra.iter());
            }
            let f = if bold { bold_font.as_ref().unwrap_or(font) } else { font };
            let galley = painter.layout_no_wrap(s, f.clone(), fg);
            let r = cell_rect(p.col, row, span);
            let x = r.center().x - galley.size().x / 2.0;
            painter.galley(pos2(x.max(r.min.x - cell.x * 0.25), y), galley, fg);
        }
        flush(&mut run, run_start, run_key.take());

        // Underline and strikeout.
        for p in line {
            if !p.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
                continue;
            }
            let r = cell_rect(p.col, row, span_of(p));
            // About one device pixel per 14 pt of font size.
            let t = ((font.size / 14.0) * ppp).round().max(1.0) / ppp;
            if p.flags.intersects(Flags::ALL_UNDERLINES) {
                let uy = ((r.bottom() - cell.y * 0.12) * ppp).round() / ppp;
                painter.line_segment([pos2(r.left(), uy), pos2(r.right(), uy)], Stroke::new(t, p.fg));
                if p.flags.contains(Flags::DOUBLE_UNDERLINE) {
                    let uy2 = uy - 2.0 * t - 1.0 / ppp;
                    painter.line_segment([pos2(r.left(), uy2), pos2(r.right(), uy2)], Stroke::new(t, p.fg));
                }
            }
            if p.flags.contains(Flags::STRIKEOUT) {
                let sy = (r.center().y * ppp).round() / ppp;
                painter.line_segment([pos2(r.left(), sy), pos2(r.right(), sy)], Stroke::new(t, p.fg));
            }
        }
    }

    painter.add(Shape::mesh(fg_mesh));

    // Other cursor shapes.
    if let Some((ccol, crow)) = cursor_cell {
        let span = if cursor_wide { 2 } else { 1 };
        let r = cell_rect(ccol, crow, span);
        let color = to_c32(palette.cursor);
        let px = 1.0 / ppp;
        let w = (2.0 * px).max((cell.x / 8.0 / px).round() * px);
        match cursor_shape {
            CursorShape::Block | CursorShape::HollowBlock if !focused_block => {
                painter.rect_stroke(r.shrink(px / 2.0), 0.0, Stroke::new(px, color), StrokeKind::Inside);
            }
            CursorShape::Beam => {
                painter.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), 0.0, color);
            }
            CursorShape::Underline => {
                painter.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - w), r.max), 0.0, color);
            }
            _ => {}
        }
        // IME pre-edit text at the cursor, underlined.
        let pre = &term.view.ime_preedit;
        if focused && !pre.is_empty() {
            let galley = painter.layout_no_wrap(pre.clone(), font.clone(), to_c32(palette.foreground));
            let pr = Rect::from_min_size(r.min, galley.size());
            painter.rect_filled(pr, 0.0, to_c32(palette.background));
            painter.galley(r.min, galley, to_c32(palette.foreground));
            painter.line_segment([pr.left_bottom(), pr.right_bottom()], Stroke::new(px, to_c32(palette.foreground)));
        }
    }

    // Visual bell: a short inset ring.
    if let Some(at) = term.view.bell_at {
        let age = at.elapsed();
        if age < BELL_FLASH {
            let a = 1.0 - age.as_secs_f32() / BELL_FLASH.as_secs_f32();
            painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(3.0, to_c32(palette.foreground).gamma_multiply(a)), StrokeKind::Inside);
            ui.ctx().request_repaint();
        } else {
            term.view.bell_at = None;
        }
    }

    // Exit overlay.
    if let Some(reason) = term.exit_reason() {
        let band = Rect::from_min_max(pos2(rect.left(), rect.bottom() - cell.y * 1.6), rect.max);
        painter.rect_filled(band, 0.0, Color32::from_black_alpha(200));
        painter.text(band.center(), Align2::CENTER_CENTER, format!("[{reason}]"), font.clone(), to_c32(palette.ansi[3]));
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::NamedColor;

    fn preedit(text: &str) -> Event {
        Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: None })
    }

    #[test]
    fn ime_filter_drops_idle_empty_preedits_only() {
        let mut f = ImeLoopFilter::default();
        let mut raw = egui::RawInput { events: vec![preedit("")], ..Default::default() };
        f.filter(&mut raw);
        assert!(raw.events.is_empty());

        // Composition then cancel: the empty pre-edit is kept.
        let mut raw = egui::RawInput { events: vec![preedit("か"), preedit("")], ..Default::default() };
        f.filter(&mut raw);
        assert_eq!(raw.events.len(), 2);

        let mut raw = egui::RawInput { events: vec![preedit("")], ..Default::default() };
        f.filter(&mut raw);
        assert!(raw.events.is_empty());
    }

    #[test]
    fn paste_normalises_newlines() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc".to_vec());
    }

    #[test]
    fn bracketed_paste_wraps_and_strips_end_marker() {
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~".to_vec());
    }

    #[test]
    fn bold_uses_bright_variant() {
        assert_eq!(adjust_fg(Color::Named(NamedColor::Red), Flags::BOLD), Color::Indexed(9));
        assert_eq!(adjust_fg(Color::Indexed(2), Flags::BOLD), Color::Indexed(10));
        assert_eq!(adjust_fg(Color::Indexed(2), Flags::empty()), Color::Indexed(2));
    }
}
