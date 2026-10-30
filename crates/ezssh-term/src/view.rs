//! egui widget: paints the grid, turns egui input into PTY bytes, and
//! carries the terminal's own UI (context menu, find bar, link hover,
//! connection overlays).
//!
//! Anything that needs the rest of the app (a confirmation dialog, the
//! font picker, opening a browser, playing a sound, reconnecting) is
//! returned from [`TerminalView::show`] as a [`TermAction`].

use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::search::Match;
use alacritty_terminal::vte::ansi::{Color, CursorShape, Rgb};
use egui::{
    Align2, Button, Color32, CursorIcon, Event, EventFilter, FontId, ImeEvent, LayerId, Mesh, MouseWheelUnit,
    Order, PointerButton, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2,
    pos2, vec2,
};

use crate::boxdraw;
use crate::fonts::Faces;
use crate::keys::{self, Action, Key, Mods, Modes};
use crate::links::{Link, LinkFinder};
use crate::mouse;
use crate::palette::{Palette, dim};
use crate::search::{Find, MAX_COUNTED};
use crate::terminal::{Terminal, UiEvent, multiline_paste_lines};

pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 48.0;
const BELL_FLASH: Duration = Duration::from_millis(120);
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// xterm.js blink rate.
const BLINK: Duration = Duration::from_millis(600);
/// Stop blinking (cursor stays on) after this long without input, so an
/// idle window does not repaint forever.
const BLINK_IDLE_STOP: Duration = Duration::from_secs(300);
/// Ctrl+wheel: one font step per 40 px of wheel travel, one wheel notch
/// being 100 px (what WebView2 reports), as in `terminal.tsx`.
const ZOOM_STEP_PX: f32 = 40.0;
const WHEEL_NOTCH_PX: f32 = 100.0;

/// `terminal.rightClick`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RightClick {
    #[default]
    Menu,
    Paste,
}

impl RightClick {
    pub fn from_setting(s: &str) -> Self {
        if s == "paste" { Self::Paste } else { Self::Menu }
    }
}

/// `terminal.bell`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BellMode {
    #[default]
    None,
    Visual,
    Sound,
}

impl BellMode {
    pub fn from_setting(s: &str) -> Self {
        match s {
            "visual" => Self::Visual,
            "sound" => Self::Sound,
            _ => Self::None,
        }
    }
}

/// App-wide look and behaviour, shared by every tab (the `terminal.*`
/// settings that are not per session).
#[derive(Clone, Debug)]
pub struct TermStyle {
    pub padding: f32,
    pub palette: Palette,
    pub copy_on_select: bool,
    pub right_click: RightClick,
    pub warn_multiline_paste: bool,
    pub bell: BellMode,
    /// Draw box-drawing and block characters as shapes instead of glyphs.
    pub builtin_box_drawing: bool,
    /// Let the font join symbol sequences (`->`, `!=`) into ligatures.
    /// Off: such sequences are drawn one cell at a time.
    pub ligatures: bool,
}

impl Default for TermStyle {
    fn default() -> Self {
        Self {
            padding: 4.0,
            palette: Palette::default(),
            copy_on_select: false,
            right_click: RightClick::Menu,
            warn_multiline_paste: true,
            bell: BellMode::None,
            builtin_box_drawing: true,
            ligatures: false,
        }
    }
}

/// Connection state painted over the terminal (the webview's overlay
/// card). The app owns the state machine; the widget only draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    Connecting { summary: String },
    Failed { message: String },
    Closed { reason: Option<String>, summary: String },
}

/// Something the app has to do for the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermAction {
    /// The overlay's Reconnect button.
    Reconnect,
    /// A paste with line breaks while `warn_multiline_paste` is on. Ask,
    /// then call [`Terminal::paste`].
    ConfirmPaste { text: String, lines: usize },
    /// A link was clicked.
    OpenUrl(String),
    /// BEL with `BellMode::Sound`.
    Bell,
    /// Context menu → Font…
    OpenFontPicker,
}

pub struct TermOutput {
    pub response: Response,
    pub actions: Vec<TermAction>,
}

/// Per-terminal UI state.
pub struct ViewState {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    cell: Vec2,
    origin: Pos2,
    /// Live font size: starts at the session's, changed by Ctrl+wheel.
    pub(crate) font_size: f32,
    pub(crate) faces: Faces,
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
    find: Option<Find>,
    /// Context menu position, and whether it opened this frame.
    menu: Option<(Pos2, bool)>,
    links: Option<LinkFinder>,
    hover: Option<Link>,
    /// Link under the primary button press; opened on release if no
    /// selection was dragged.
    press_link: Option<String>,
    blink_epoch: Instant,
    last_input: Instant,
    /// Take keyboard focus on the next frame (after a popup closes).
    refocus: bool,
    /// Paint time of the last frame, for the spike's measurements.
    pub(crate) last_paint: Duration,
}

impl Default for ViewState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            cols: 0,
            rows: 0,
            cell: Vec2::ZERO,
            origin: Pos2::ZERO,
            font_size: 14.0,
            faces: Faces::default(),
            selecting: false,
            last_click: None,
            reported_button: None,
            last_motion_cell: None,
            scroll_accum: 0.0,
            zoom_accum: 0.0,
            bell_at: None,
            ime_preedit: String::new(),
            focused_last: false,
            clipboard: None,
            find: None,
            menu: None,
            links: None,
            hover: None,
            press_link: None,
            blink_epoch: now,
            last_input: now,
            refocus: false,
            last_paint: Duration::ZERO,
        }
    }
}

impl ViewState {
    pub fn last_paint(&self) -> Duration {
        self.last_paint
    }

    pub fn find_open(&self) -> bool {
        self.find.is_some()
    }

    /// URL of the link under the pointer, for a status-bar preview.
    pub fn hovered_link(&self) -> Option<&str> {
        self.hover.as_ref().map(|l| l.uri.as_str())
    }
}

impl Terminal {
    pub fn font_size(&self) -> f32 {
        self.view.font_size
    }

    /// Clamped to 8–48 pt. The grid is re-measured on the next frame and
    /// the PTY told about the new size.
    pub fn set_font_size(&mut self, size: f32) {
        self.view.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    }

    pub fn faces(&self) -> &Faces {
        &self.view.faces
    }

    pub fn set_faces(&mut self, faces: Faces) {
        self.view.faces = faces;
    }

    /// Open the find bar (Ctrl+Shift+F, context menu).
    pub fn open_find(&mut self) {
        match &mut self.view.find {
            Some(f) => f.focus_input = true,
            None => self.view.find = Some(Find::new()),
        }
    }

    /// Open the find bar with `query` and jump to its most recent match
    /// (an app-level "Find…" command, and test automation).
    pub fn find(&mut self, query: &str) {
        self.open_find();
        let mut find = self.view.find.take().unwrap_or_default();
        find.query = query.to_string();
        find.update(&mut *self.lock());
        self.view.find = Some(find);
    }

    /// Open the context menu at `pos` (screen points), as a right-click
    /// does.
    pub fn open_menu(&mut self, pos: Pos2) {
        self.view.menu = Some((pos, true));
    }
}

pub struct TerminalView<'a> {
    term: &'a mut Terminal,
    style: &'a TermStyle,
    focus_on_show: bool,
    overlay: Option<&'a Overlay>,
}

impl<'a> TerminalView<'a> {
    pub fn new(term: &'a mut Terminal, style: &'a TermStyle) -> Self {
        Self { term, style, focus_on_show: false, overlay: None }
    }

    /// Take keyboard focus this frame (new tab, tab switch).
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus_on_show = focus;
        self
    }

    /// Paint a connection overlay over the terminal and stop input to it.
    pub fn overlay(mut self, overlay: Option<&'a Overlay>) -> Self {
        self.overlay = overlay;
        self
    }

    pub fn show(self, ui: &mut Ui) -> TermOutput {
        let TerminalView { term, style, focus_on_show, overlay } = self;
        let mut actions = Vec::new();
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let id = response.id;
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let blocked = overlay.is_some();

        let popup_open = term.view.find.is_some() || term.view.menu.is_some();
        let wants_focus = focus_on_show || std::mem::take(&mut term.view.refocus);
        if !blocked && (wants_focus || ((response.clicked() || response.drag_started()) && !popup_open)) {
            response.request_focus();
        }
        let focused = response.has_focus() && !blocked;
        if focused {
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                )
            });
        }

        // Cell metrics.
        let font = FontId::new(term.view.font_size, term.view.faces.regular.clone());
        let (glyph_w, row_h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        // Width stays exact so a run of ASCII glyphs laid out as one galley
        // lines up with the grid; height snaps to device pixels so rows and
        // box-drawing joins are crisp.
        let cell = vec2(glyph_w.max(1.0), ((row_h * ppp).ceil() / ppp).max(1.0));
        let inner = rect.shrink(style.padding);
        let cols = ((inner.width() / cell.x).floor() as usize).max(2);
        let rows = ((inner.height() / cell.y).floor() as usize).max(1);
        let origin = pos2((inner.min.x * ppp).round() / ppp, (inner.min.y * ppp).round() / ppp);
        term.view.cell = cell;
        term.view.origin = origin;
        if (cols, rows) != (term.view.cols, term.view.rows) {
            term.view.cols = cols;
            term.view.rows = rows;
            let cell_px = ((cell.x * ppp).round() as u16, (cell.y * ppp).round() as u16);
            term.resize(cols, rows, cell_px);
        }

        // Model-side events (title, bell, OSC 52).
        for ev in term.take_events() {
            match ev {
                UiEvent::Bell => match style.bell {
                    BellMode::None => {}
                    BellMode::Visual => term.view.bell_at = Some(Instant::now()),
                    BellMode::Sound => actions.push(TermAction::Bell),
                },
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
            let mut t = term.lock();
            t.is_focused = focused;
            let report = t.mode().contains(TermMode::FOCUS_IN_OUT);
            drop(t);
            if report {
                term.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
            }
        }

        let events = ui.input(|i| i.events.clone());
        let layer = ui.layer_id();
        let hovered = !blocked && (response.hovered() || response.contains_pointer());
        if !blocked {
            let env = PointerEnv { ctx: &ctx, rect, layer, hovered };
            handle_pointer(term, style, &events, &env, &mut actions);
        }
        if focused {
            handle_keyboard(term, style, &events, &ctx, &mut actions);
        }
        update_hover(term, &ctx, rect, layer, hovered);

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

        if hovered {
            let mouse_mode = term.lock().mode().intersects(TermMode::MOUSE_MODE);
            let shift = ui.input(|i| i.modifiers.shift);
            ctx.set_cursor_icon(if term.view.hover.is_some() {
                CursorIcon::PointingHand
            } else if mouse_mode && !shift {
                CursorIcon::Default
            } else {
                CursorIcon::Text
            });
        }

        show_find(&ctx, term, rect, id);
        show_menu(&ctx, term, style, id, &mut actions);
        if let Some(o) = overlay {
            // Dim the terminal under the card (bg at 70 %, as the webview
            // did), on the widget's own layer so the card stays on top.
            let bg = to_c32(style.palette.background);
            ui.painter_at(rect).rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 178));
            show_overlay(&ctx, rect, id, o, &mut actions);
        }
        TermOutput { response, actions }
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

/// Paste, or ask first when the text would run several lines.
fn request_paste(term: &mut Terminal, style: &TermStyle, text: String, actions: &mut Vec<TermAction>) {
    if text.is_empty() {
        return;
    }
    if style.warn_multiline_paste
        && let Some(lines) = multiline_paste_lines(&text)
    {
        tracing::debug!(lines, "term.paste: multi-line, asking first");
        actions.push(TermAction::ConfirmPaste { text, lines });
        return;
    }
    term.paste(&text);
}

fn paste_clipboard(term: &mut Terminal, style: &TermStyle, actions: &mut Vec<TermAction>) {
    if let Some(t) = read_clipboard(term) {
        request_paste(term, style, t, actions);
    }
}

fn copy_selection(term: &mut Terminal, ctx: &egui::Context) -> bool {
    match term.selection_text() {
        Some(t) => {
            tracing::debug!(len = t.len(), "term.copy");
            ctx.copy_text(t);
            true
        }
        None => false,
    }
}

fn send_key_bytes(term: &mut Terminal, bytes: Vec<u8>) {
    term.scroll_to_bottom();
    let now = Instant::now();
    term.view.blink_epoch = now;
    term.view.last_input = now;
    term.write(bytes);
}

fn handle_keyboard(
    term: &mut Terminal,
    style: &TermStyle,
    events: &[Event],
    ctx: &egui::Context,
    actions: &mut Vec<TermAction>,
) {
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
                            paste_clipboard(term, style, actions);
                            continue;
                        }
                        egui::Key::F if ctrl_shift => {
                            term.open_find();
                            continue;
                        }
                        egui::Key::Insert if mods.shift && !mods.ctrl => {
                            paste_clipboard(term, style, actions);
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
            Event::Paste(text) => request_paste(term, style, text.clone(), actions),
            Event::Copy => {
                copy_selection(term, ctx);
            }
            _ => {}
        }
    }
}

/// Wheel travel in pixels, as a browser would report it.
fn wheel_px(unit: MouseWheelUnit, delta: f32, cell_h: f32, rows: usize) -> f32 {
    match unit {
        MouseWheelUnit::Line => delta * WHEEL_NOTCH_PX,
        MouseWheelUnit::Point => delta,
        MouseWheelUnit::Page => delta * rows as f32 * cell_h,
    }
}

/// Ctrl+wheel zoom: returns the new size, carrying the remainder in `accum`.
/// Wheel up (positive) grows the font.
fn zoom_step(size: f32, accum: &mut f32, px: f32) -> f32 {
    *accum += px;
    let steps = (*accum / ZOOM_STEP_PX).trunc();
    *accum -= steps * ZOOM_STEP_PX;
    (size + steps).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// Where the widget is this frame, for pointer handling.
struct PointerEnv<'a> {
    ctx: &'a egui::Context,
    rect: Rect,
    /// The widget's layer: presses over a popup above it are not ours.
    layer: LayerId,
    hovered: bool,
}

impl PointerEnv<'_> {
    fn on_top(&self, pos: Pos2) -> bool {
        self.ctx.layer_id_at(pos).is_none_or(|l| l == self.layer)
    }
}

fn handle_pointer(
    term: &mut Terminal,
    style: &TermStyle,
    events: &[Event],
    env: &PointerEnv<'_>,
    actions: &mut Vec<TermAction>,
) {
    let (ctx, rect, hovered) = (env.ctx, env.rect, env.hovered);
    for ev in events {
        match ev {
            Event::PointerButton { pos, button, pressed, modifiers } => {
                if *pressed && (!rect.contains(*pos) || !env.on_top(*pos)) {
                    continue;
                }
                if !*pressed && !term.view.selecting && term.view.reported_button.is_none() && term.view.press_link.is_none() {
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
                        term.view.press_link = term.view.hover.as_ref().map(|l| l.uri.clone());
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
                        let link = term.view.press_link.take();
                        let empty = term.lock().selection.as_ref().is_none_or(|s| s.is_empty());
                        if empty {
                            term.lock().selection = None;
                            // A click (no drag) on a link opens it.
                            if let Some(uri) = link.filter(|u| term.view.hover.as_ref().is_some_and(|h| h.uri == *u)) {
                                tracing::debug!(%uri, "term.link: open");
                                actions.push(TermAction::OpenUrl(uri));
                            }
                        } else if style.copy_on_select {
                            copy_selection(term, ctx);
                        }
                    }
                    (PointerButton::Secondary, true) => {
                        // Right-click paste (PuTTY / MobaXterm) is a setting;
                        // Ctrl + right-click always reaches the menu.
                        if style.right_click == RightClick::Paste && !mods.ctrl {
                            paste_clipboard(term, style, actions);
                        } else {
                            term.view.menu = Some((*pos, true));
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
                if ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| !env.on_top(p)) {
                    continue;
                }
                let mods = Mods::from_egui(*modifiers);
                if mods.ctrl {
                    let px = wheel_px(*unit, delta.y, term.view.cell.y, term.view.rows);
                    let size = zoom_step(term.view.font_size, &mut term.view.zoom_accum, px);
                    if size != term.view.font_size {
                        tracing::debug!(from = term.view.font_size, to = size, "term.zoom");
                        term.view.font_size = size;
                    }
                    continue;
                }
                let lines = match unit {
                    MouseWheelUnit::Line => delta.y,
                    MouseWheelUnit::Point => delta.y / term.view.cell.y.max(1.0),
                    MouseWheelUnit::Page => delta.y * term.view.rows as f32,
                };
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

/// Track the link under the pointer. Links are inactive while a program
/// takes the mouse (unless Shift is held, as for selection).
fn update_hover(term: &mut Terminal, ctx: &egui::Context, rect: Rect, layer: LayerId, hovered: bool) {
    let pos = ctx.input(|i| i.pointer.hover_pos());
    let shift = ctx.input(|i| i.modifiers.shift);
    let Some(pos) = pos.filter(|p| hovered && rect.contains(*p) && ctx.layer_id_at(*p).is_none_or(|l| l == layer))
    else {
        term.view.hover = None;
        return;
    };
    // A drag that started off a link is a selection; one that started on a
    // link keeps the link until release decides (click opens, drag selects).
    let selecting_text = term.view.selecting && term.view.press_link.is_none();
    if selecting_text || (term.lock().mode().intersects(TermMode::MOUSE_MODE) && !shift) {
        term.view.hover = None;
        return;
    }
    let mut finder = term.view.links.take().or_else(LinkFinder::new);
    let (col, row, _) = cell_at(&term.view, pos);
    let t = term.lock();
    let point = grid_point(t.grid().display_offset(), col, row);
    let link = finder.as_mut().and_then(|f| f.at(&*t, point));
    drop(t);
    term.view.links = finder;
    term.view.hover = link;
}

// ---------------------------------------------------------------- popups

fn show_menu(ctx: &egui::Context, term: &mut Terminal, style: &TermStyle, id: egui::Id, actions: &mut Vec<TermAction>) {
    let Some((pos, just_opened)) = term.view.menu else {
        return;
    };
    let has_selection = term.has_selection();
    let mut chosen: Option<&'static str> = None;
    let area = egui::Area::new(id.with("context-menu")).order(Order::Foreground).fixed_pos(pos).constrain(true).show(
        ctx,
        |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(190.0);
                let item = |ui: &mut Ui, enabled: bool, label: &'static str, shortcut: &str| {
                    let mut b = Button::new(label).frame(false);
                    if !shortcut.is_empty() {
                        b = b.shortcut_text(shortcut);
                    }
                    ui.add_enabled(enabled, b).clicked().then_some(label)
                };
                chosen = item(ui, has_selection, "Copy", "Ctrl+Shift+C")
                    .or_else(|| item(ui, true, "Paste", "Shift+Insert"))
                    .or_else(|| item(ui, true, "Select All", ""))
                    .or_else(|| item(ui, true, "Find…", "Ctrl+Shift+F"));
                ui.separator();
                chosen = chosen
                    .or_else(|| item(ui, true, "Font…", "Ctrl+Wheel"))
                    .or_else(|| item(ui, true, "Clear scrollback", ""));
            });
        },
    );
    let mut close = chosen.is_some() || ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if !just_opened {
        let outside = ctx.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p))
        });
        close |= outside;
    }
    match chosen {
        Some("Copy") => {
            copy_selection(term, ctx);
        }
        Some("Paste") => paste_clipboard(term, style, actions),
        Some("Select All") => term.select_all(),
        Some("Find…") => term.open_find(),
        Some("Font…") => actions.push(TermAction::OpenFontPicker),
        Some("Clear scrollback") => term.clear_scrollback(),
        Some(other) => tracing::error!(item = other, "term.menu: unhandled item"),
        None => {}
    }
    if close {
        term.view.menu = None;
        if chosen != Some("Find…") {
            term.view.refocus = true;
        }
    } else {
        term.view.menu = Some((pos, false));
    }
}

fn show_find(ctx: &egui::Context, term: &mut Terminal, rect: Rect, id: egui::Id) {
    let Some(mut find) = term.view.find.take() else {
        return;
    };
    let mut close = false;
    egui::Area::new(id.with("find"))
        .order(Order::Foreground)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(rect.right_top() + vec2(-8.0, 8.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(TextEdit::singleline(&mut find.query).hint_text("Find").desired_width(176.0));
                    if std::mem::take(&mut find.focus_input) {
                        edit.request_focus();
                    }
                    let mut changed = edit.changed();
                    let (enter, escape, shift) =
                        ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape), i.modifiers.shift));
                    let mut step: Option<bool> = None;
                    if edit.lost_focus() && enter {
                        step = Some(!shift);
                        edit.request_focus();
                    }
                    if escape && (edit.has_focus() || edit.lost_focus()) {
                        close = true;
                    }
                    if !find.query.is_empty() {
                        let text = match (&find.error, find.counter()) {
                            (Some(_), _) => "bad regex".to_string(),
                            (None, (i, n)) if find.truncated && n == MAX_COUNTED => format!("{i}/{n}+"),
                            (None, (i, n)) => format!("{i}/{n}"),
                        };
                        let label = ui.label(RichText::new(text).small().weak());
                        if let Some(e) = &find.error {
                            label.on_hover_text(e.as_str());
                        }
                    }
                    if ui.selectable_label(find.case_sensitive, "Aa").on_hover_text("Case sensitive").clicked() {
                        find.case_sensitive = !find.case_sensitive;
                        changed = true;
                    }
                    if ui.selectable_label(find.regex, ".*").on_hover_text("Regular expression").clicked() {
                        find.regex = !find.regex;
                        changed = true;
                    }
                    if ui.small_button("Prev").on_hover_text("Previous (Shift+Enter)").clicked() {
                        step = Some(false);
                    }
                    if ui.small_button("Next").on_hover_text("Next (Enter)").clicked() {
                        step = Some(true);
                    }
                    if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                    if changed {
                        find.update(&mut *term.lock());
                    }
                    if let Some(forward) = step {
                        find.step(&mut *term.lock(), forward);
                    }
                });
            });
        });
    if close {
        tracing::debug!("term.find: closed");
        term.view.refocus = true;
    } else {
        term.view.find = Some(find);
    }
}

fn show_overlay(ctx: &egui::Context, rect: Rect, id: egui::Id, overlay: &Overlay, actions: &mut Vec<TermAction>) {
    egui::Area::new(id.with("overlay"))
        .order(Order::Middle)
        .pivot(Align2::CENTER_CENTER)
        .fixed_pos(rect.center())
        .constrain_to(rect)
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.set_width(320.0_f32.min(rect.width() * 0.9));
                ui.vertical_centered(|ui| {
                    let reconnect = match overlay {
                        Overlay::Connecting { summary } => {
                            ui.add(egui::Spinner::new().size(24.0));
                            ui.label(RichText::new("Connecting…").strong());
                            ui.label(RichText::new(summary).monospace().small().weak());
                            false
                        }
                        Overlay::Failed { message } => {
                            ui.label(RichText::new("Connection failed").strong().color(ui.visuals().error_fg_color));
                            ui.label(RichText::new(message).small().weak());
                            ui.add_space(4.0);
                            ui.button("Reconnect").clicked()
                        }
                        Overlay::Closed { reason, summary } => {
                            let title = match reason {
                                Some(r) => format!("Disconnected ({r})"),
                                None => "Disconnected".to_string(),
                            };
                            ui.label(RichText::new(title).strong());
                            ui.label(RichText::new(summary).monospace().small().weak());
                            ui.add_space(4.0);
                            ui.button("Reconnect").clicked()
                        }
                    };
                    if reconnect {
                        tracing::debug!("term.overlay: reconnect");
                        actions.push(TermAction::Reconnect);
                    }
                });
            });
        });
}

// ---------------------------------------------------------------- paint

fn to_c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b) }
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
        Color::Named(n) if flags.contains(Flags::BOLD) && (n as usize) < 8 => Color::Indexed(n as u8 + 8),
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
    /// Under the hovered link.
    link: bool,
}

fn is_simple(c: char) -> bool {
    c.is_ascii_graphic() || c == ' '
}

/// Characters that coding fonts join into ligatures (`->`, `!=`, `===`).
fn ligature_prone(c: char) -> bool {
    c.is_ascii_punctuation()
}

/// Whether the cursor is drawn this frame, and when to repaint for the
/// next blink phase.
fn cursor_phase(blinking: bool, since_epoch: Duration, since_input: Duration) -> (bool, Option<Duration>) {
    if !blinking || since_input >= BLINK_IDLE_STOP {
        return (true, None);
    }
    let period = BLINK.as_millis();
    let ms = since_epoch.as_millis();
    let visible = (ms / period).is_multiple_of(2);
    let next = Duration::from_millis((period - ms % period) as u64);
    (visible, Some(next))
}

fn visible_matches(find: Option<&Find>, top: i32, bottom: i32) -> Vec<Match> {
    let Some(f) = find else {
        return Vec::new();
    };
    f.matches.iter().filter(|m| m.start().line.0 <= bottom && m.end().line.0 >= top).cloned().collect()
}

fn paint(ui: &Ui, term: &mut Terminal, style: &TermStyle, rect: Rect, focused: bool, font: &FontId, ppp: f32) {
    let painter = ui.painter_at(rect);
    let palette = &style.palette;
    let default_bg = palette.background;
    painter.rect_filled(rect, 0.0, to_c32(default_bg));

    let v = &term.view;
    let (cell, origin, cols, rows) = (v.cell, v.origin, v.cols, v.rows);
    let faces = v.faces.clone();
    let hover = v.hover.clone();
    let match_bg = mix(default_bg, palette.ansi[3], 0.35);

    // Snapshot the visible cells, then release the lock before drawing.
    let mut lines: Vec<Vec<PCell>> = (0..rows).map(|_| Vec::with_capacity(cols)).collect();
    let (cursor_cell, cursor_shape, cursor_wide, cursor_blinks);
    {
        let t = term.lock();
        let content = t.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset as i32;
        let selection = content.selection;
        cursor_shape = content.cursor.shape;
        cursor_blinks = t.cursor_style().blinking;
        let cp = content.cursor.point;
        let cursor_row = cp.line.0 + offset;
        cursor_cell = (cursor_row >= 0 && (cursor_row as usize) < rows).then_some((cp.column.0, cursor_row as usize));
        cursor_wide = t.grid()[cp].flags.contains(Flags::WIDE_CHAR);
        let matches = visible_matches(term.view.find.as_ref(), -offset, rows as i32 - 1 - offset);
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
            if !matches.is_empty() && matches.iter().any(|m| m.contains(&indexed.point)) {
                bg = match_bg;
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
                link: hover.as_ref().is_some_and(|h| h.range.contains(&indexed.point)),
            });
        }
    }

    let (cursor_on, next_blink) = cursor_phase(
        focused && cursor_blinks,
        term.view.blink_epoch.elapsed(),
        term.view.last_input.elapsed(),
    );
    if let Some(next) = next_blink {
        ui.ctx().request_repaint_after(next);
    }
    let focused_block = focused && cursor_on && cursor_shape == CursorShape::Block;
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
    let font_for = |bold: bool, italic: bool| FontId::new(font.size, faces.pick(bold, italic).clone());
    for (row, line) in lines.iter().enumerate() {
        let y = origin.y + row as f32 * cell.y;
        let mut run = String::new();
        let mut run_start = 0usize;
        // (colour, bold, italic)
        let mut run_key: Option<(Color32, bool, bool)> = None;
        let flush = |run: &mut String, start: usize, key: Option<(Color32, bool, bool)>| {
            if let Some((fg, bold, italic)) = key
                && !run.trim_end().is_empty()
            {
                painter.text(
                    pos2(origin.x + start as f32 * cell.x, y),
                    Align2::LEFT_TOP,
                    run.trim_end(),
                    font_for(bold, italic),
                    fg,
                );
            }
            run.clear();
        };
        for p in line {
            let under_cursor = focused_block && cursor_cell == Some((p.col, row));
            let fg = if under_cursor { to_c32(palette.cursor_accent) } else { p.fg };
            let bold = p.flags.contains(Flags::BOLD);
            let italic = p.flags.contains(Flags::ITALIC);
            let span = span_of(p);

            if style.builtin_box_drawing && boxdraw::handles(p.c) {
                flush(&mut run, run_start, run_key.take());
                boxdraw::draw(&mut fg_mesh, &painter, cell_rect(p.col, row, span), p.c, fg, ppp, box_stroke);
                continue;
            }
            if is_simple(p.c) && p.extra.is_none() && span == 1 {
                let key = Some((fg, bold, italic));
                // Runs hold only ASCII, so byte length = cell count.
                let split_ligature =
                    !style.ligatures && ligature_prone(p.c) && run.chars().last().is_some_and(ligature_prone);
                if run_key != key || run_start + run.len() != p.col || split_ligature {
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
            let galley = painter.layout_no_wrap(s, font_for(bold, italic), fg);
            let r = cell_rect(p.col, row, span);
            let x = r.center().x - galley.size().x / 2.0;
            painter.galley(pos2(x.max(r.min.x - cell.x * 0.25), y), galley, fg);
        }
        flush(&mut run, run_start, run_key.take());

        // Underline (SGR or hovered link) and strikeout.
        for p in line {
            if !p.link && !p.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
                continue;
            }
            let r = cell_rect(p.col, row, span_of(p));
            // About one device pixel per 14 pt of font size.
            let t = ((font.size / 14.0) * ppp).round().max(1.0) / ppp;
            if p.link || p.flags.intersects(Flags::ALL_UNDERLINES) {
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
            // Unfocused: hollow block, whatever the shape, as xterm.js does.
            CursorShape::Block | CursorShape::HollowBlock | CursorShape::Beam | CursorShape::Underline
                if !focused =>
            {
                painter.rect_stroke(r.shrink(px / 2.0), 0.0, Stroke::new(px, color), StrokeKind::Inside);
            }
            CursorShape::HollowBlock if cursor_on => {
                painter.rect_stroke(r.shrink(px / 2.0), 0.0, Stroke::new(px, color), StrokeKind::Inside);
            }
            CursorShape::Beam if cursor_on => {
                painter.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), 0.0, color);
            }
            CursorShape::Underline if cursor_on => {
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

    // Visual bell: a short inset ring in the palette's yellow (the
    // webview's ring-warning).
    if let Some(at) = term.view.bell_at {
        let age = at.elapsed();
        if age < BELL_FLASH {
            let a = 1.0 - age.as_secs_f32() / BELL_FLASH.as_secs_f32();
            painter.rect_stroke(
                rect.shrink(1.0),
                0.0,
                Stroke::new(2.0, to_c32(palette.ansi[3]).gamma_multiply(a)),
                StrokeKind::Inside,
            );
            ui.ctx().request_repaint();
        } else {
            term.view.bell_at = None;
        }
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
    fn bold_uses_bright_variant() {
        assert_eq!(adjust_fg(Color::Named(NamedColor::Red), Flags::BOLD), Color::Indexed(9));
        assert_eq!(adjust_fg(Color::Indexed(2), Flags::BOLD), Color::Indexed(10));
        assert_eq!(adjust_fg(Color::Indexed(2), Flags::empty()), Color::Indexed(2));
    }

    #[test]
    fn zoom_steps_every_40px_and_clamps() {
        let mut acc = 0.0;
        // One mouse-wheel notch up (100 px): two steps, 20 px carried.
        assert_eq!(zoom_step(13.0, &mut acc, wheel_px(MouseWheelUnit::Line, 1.0, 16.0, 24)), 15.0);
        assert_eq!(acc, 20.0);
        // A trackpad's small deltas accumulate.
        assert_eq!(zoom_step(15.0, &mut acc, 10.0), 15.0);
        assert_eq!(zoom_step(15.0, &mut acc, 10.0), 16.0);
        assert_eq!(acc, 0.0);
        // Down shrinks; the range is 8–48.
        assert_eq!(zoom_step(9.0, &mut acc, -200.0), MIN_FONT_SIZE);
        assert_eq!(zoom_step(47.0, &mut acc, 400.0), MAX_FONT_SIZE);
    }

    #[test]
    fn cursor_blinks_at_600ms_and_stops_when_idle() {
        let ms = Duration::from_millis;
        assert_eq!(cursor_phase(false, ms(700), ms(0)), (true, None));
        assert_eq!(cursor_phase(true, ms(100), ms(0)), (true, Some(ms(500))));
        assert_eq!(cursor_phase(true, ms(700), ms(0)), (false, Some(ms(500))));
        assert_eq!(cursor_phase(true, ms(1300), BLINK_IDLE_STOP), (true, None));
    }

    #[test]
    fn settings_parse() {
        assert_eq!(RightClick::from_setting("paste"), RightClick::Paste);
        assert_eq!(RightClick::from_setting("menu"), RightClick::Menu);
        assert_eq!(BellMode::from_setting("sound"), BellMode::Sound);
        assert_eq!(BellMode::from_setting("visual"), BellMode::Visual);
        assert_eq!(BellMode::from_setting("none"), BellMode::None);
    }

    #[test]
    fn ligature_prone_is_ascii_punctuation() {
        assert!(ligature_prone('-') && ligature_prone('>') && ligature_prone('='));
        assert!(!ligature_prone('a') && !ligature_prone(' ') && !ligature_prone('0'));
    }
}
