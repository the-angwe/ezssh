//! The tab area in its six view modes (tabs, tiled, grid, cascade with
//! its window chrome and minimized strip, auto). Hidden tabs are not painted,
//! but their terminals keep running, so connections survive mode switches.

use egui::{CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use ezssh_term::{Overlay, TermAction, TerminalView};

use crate::app::App;
use crate::icons::Icon;
use crate::layout::{FORWARDS_PANE_W, SFTP_PANE_MAX, TERMINAL_MIN_W};
use crate::style::{self, font, radius, size, tokens};
use crate::tabs::{CascadeGeom, Edge, TabId, TabStatus, ViewMode};
use crate::widgets;

/// Cascade title bar height (px).
const CHROME_H: f32 = 28.0;
/// Minimized strip height (px).
const STRIP_H: f32 = 36.0;
/// Gutter between tiles (px): the `bg` ground shows through.
const GUTTER: f32 = 4.0;
/// Tile grid rows never get shorter than this; past it the grid scrolls.
const GRID_ROW_MIN: f32 = 160.0;

#[derive(Default)]
pub struct MdiState {
    /// A terminal had keyboard focus this frame (global shortcuts yield).
    pub term_focused: bool,
    cascade_drag: Option<CascadeDrag>,
    /// The cascade frame area of this frame (the MDI area minus the
    /// minimised strip), for the chrome's maximise / drag maths.
    frames: Option<Rect>,
}

#[derive(Clone, Copy)]
struct CascadeDrag {
    tab: TabId,
    /// `None` = moving by the title bar.
    edge: Option<Edge>,
    start: (f32, f32, f32, f32),
}

/// Where each visible tab goes and how it is framed.
enum Slot {
    /// Tabs mode: the active tab with its docked panes.
    Full,
    /// Tile modes: a cell with an accent ring when active.
    Cell,
    /// Cascade: a floating frame with chrome.
    Frame,
}

pub fn show(app: &mut App, ui: &mut Ui, area: Rect) {
    let t = tokens(ui.ctx());
    app.mdi.term_focused = false;
    if app.tabs.tabs.is_empty() {
        ui.painter().rect_filled(area, 0.0, t.bg);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        widgets::empty_state(
            &mut child,
            Icon::Terminal,
            "Ready to connect",
            "Double-click a session in the sidebar, or create a new one to get started.",
            None,
            false,
        );
        return;
    }
    let mode = app.tabs.view_mode;
    let visible: Vec<TabId> = app.tabs.visible().map(|t| t.id).collect();
    match mode {
        ViewMode::Tabs => {
            ui.painter().rect_filled(area, 0.0, t.bg);
            if let Some(id) = app.tabs.active.filter(|id| !app.tabs.is_minimized(*id)) {
                slot(app, ui, id, area, Slot::Full);
            }
        }
        ViewMode::TileH | ViewMode::TileV => {
            ui.painter().rect_filled(area, 0.0, t.bg);
            let n = visible.len().max(1) as f32;
            let horizontal = mode == ViewMode::TileH;
            let span = if horizontal { area.height() } else { area.width() };
            let each = (span - GUTTER * (n - 1.0)) / n;
            for (i, id) in visible.iter().enumerate() {
                let off = i as f32 * (each + GUTTER);
                let r = if horizontal {
                    Rect::from_min_size(Pos2::new(area.left(), area.top() + off), vec2(area.width(), each))
                } else {
                    Rect::from_min_size(Pos2::new(area.left() + off, area.top()), vec2(each, area.height()))
                };
                slot(app, ui, *id, r, Slot::Cell);
            }
        }
        ViewMode::TileGrid | ViewMode::Auto => {
            ui.painter().rect_filled(area, 0.0, t.bg);
            let (rows, cols) = app.tabs.grid_dims();
            // Extra tabs get extra rows (CSS grid-auto-rows).
            let rows = rows.max(visible.len().div_ceil(cols.max(1)));
            let row_h = ((area.height() - GUTTER * (rows as f32 - 1.0)) / rows as f32).max(GRID_ROW_MIN);
            let col_w = (area.width() - GUTTER * (cols as f32 - 1.0)) / cols as f32;
            let content_h = rows as f32 * row_h + GUTTER * (rows as f32 - 1.0);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).id_salt("mdi-grid"));
            egui::ScrollArea::vertical().id_salt("mdi-grid-scroll").auto_shrink(false).show(&mut child, |ui| {
                let (content, _) = ui.allocate_exact_size(vec2(area.width(), content_h), Sense::hover());
                for (i, id) in visible.iter().enumerate() {
                    let (r, c) = (i / cols, i % cols);
                    let cell = Rect::from_min_size(
                        Pos2::new(content.left() + c as f32 * (col_w + GUTTER), content.top() + r as f32 * (row_h + GUTTER)),
                        vec2(col_w, row_h),
                    );
                    slot(app, ui, *id, cell, Slot::Cell);
                }
            });
        }
        ViewMode::Cascade => {
            ui.painter().rect_filled(area, 0.0, t.bg);
            let any_min = !app.tabs.minimized.is_empty();
            let frames = if any_min { area.with_max_y(area.bottom() - STRIP_H) } else { area };
            app.mdi.frames = Some(frames);
            cascade_drag(app, ui, frames);
            // A press raises and activates the topmost frame under it.
            let rects: Vec<(TabId, Rect)> = app
                .tabs
                .z_order()
                .into_iter()
                .filter_map(|id| {
                    let (x, y, w, h) = app.tabs.cascade.get(&id)?.displayed(frames.width(), frames.height());
                    Some((id, Rect::from_min_size(frames.min + vec2(x, y), vec2(w, h))))
                })
                .collect();
            if !app.any_modal_open()
                && let Some((id, _)) = rects.iter().rev().find(|(_, r)| pressed_in(ui, *r))
            {
                let id = *id;
                app.tabs.bring_to_front(id);
                app.activate(id);
            }
            for id in app.tabs.z_order() {
                let Some(g) = app.tabs.cascade.get(&id) else {
                    continue;
                };
                let (x, y, w, h) = g.displayed(frames.width(), frames.height());
                let r = Rect::from_min_size(frames.min + vec2(x, y), vec2(w, h));
                slot(app, ui, id, r, Slot::Frame);
            }
            if any_min {
                minimized_strip(app, ui, Rect::from_min_max(Pos2::new(area.left(), frames.bottom()), area.max));
            }
        }
    }
}

/// The primary button went down inside `rect` this frame.
fn pressed_in(ui: &Ui, rect: Rect) -> bool {
    ui.ctx().input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p)))
}

/// One tab's slot: activation on press, frame / ring, panes, terminal.
fn slot(app: &mut App, ui: &mut Ui, id: TabId, rect: Rect, kind: Slot) {
    let t = tokens(ui.ctx());
    let Some(tab) = app.tabs.get(id).cloned() else {
        return;
    };
    let active = app.tabs.active == Some(id);
    // A press anywhere in a tile activates it (frames: see `show`).
    if matches!(kind, Slot::Cell) && !active && pressed_in(ui, rect) && !app.any_modal_open() {
        app.activate(id);
    }
    let mut body = rect;
    match kind {
        Slot::Full => {}
        Slot::Cell => {
            ui.painter().rect_filled(rect, 0.0, t.bg);
        }
        Slot::Frame => {
            ui.painter().add(style::popup_shadow().as_shape(rect, radius::LG));
            ui.painter().rect_filled(rect, radius::LG, t.surface);
            body = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + CHROME_H), rect.max).shrink(1.0);
            chrome(app, ui, id, rect);
        }
    }

    // Docked panes (tabs mode only): file pane, then forwards, then the
    // terminal.
    if matches!(kind, Slot::Full) {
        if tab.sftp_open && tab.has_file_pane() {
            let forwards = if tab.forwards_open && tab.is_ssh() { FORWARDS_PANE_W } else { 0.0 };
            let max = (body.width() - TERMINAL_MIN_W - forwards).min(SFTP_PANE_MAX);
            let w = app.layout.sftp_pane_width.min(body.width() - 120.0).max(0.0);
            let pane = Rect::from_min_size(body.min, vec2(w, body.height()));
            crate::views::sftp::pane(app, ui, id, pane, max);
            body.min.x += w;
        }
        if tab.forwards_open && tab.is_ssh() {
            let w = FORWARDS_PANE_W.min(body.width() - 120.0).max(0.0);
            let pane = Rect::from_min_size(body.min, vec2(w, body.height()));
            crate::views::forwards::pane(app, ui, id, pane);
            body.min.x += w;
        }
    }

    terminal(app, ui, id, body);
    if app.terms.get(&id).is_some_and(|s| s.auth_fix.is_some()) {
        crate::views::auth_fix::show(app, ui, id, body);
    }

    match kind {
        Slot::Cell if active => {
            ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        }
        Slot::Frame => {
            let border = if active { t.accent } else { t.border_strong };
            ui.painter().rect_stroke(rect, radius::LG, Stroke::new(1.0, border), StrokeKind::Inside);
        }
        _ => {}
    }
}

/// Right-click on a tab header (tab strip, cascade title bar): activate
/// the tab and open its terminal's context menu at `pos`, under the same
/// rule as a right-click inside the terminal: not while a dialog, the
/// auth-fix card or a connection overlay covers it.
pub(crate) fn open_tab_menu(app: &mut App, id: TabId, pos: Pos2) {
    app.activate(id);
    let modal = app.any_modal_open();
    let Some(tab) = app.tabs.get(id) else {
        tracing::warn!(tab = id, "mdi: tab menu for a closed tab");
        return;
    };
    let covered = overlay_for(tab).is_some();
    let Some(slot) = app.terms.get_mut(&id) else {
        tracing::error!(tab = id, "mdi: tab without a terminal, no menu");
        return;
    };
    if modal || covered || slot.auth_fix.is_some() || slot.host_key.is_some() {
        tracing::debug!(tab = id, modal, covered, auth_fix = slot.auth_fix.is_some(), host_key = slot.host_key.is_some(), "mdi: tab menu blocked");
        return;
    }
    tracing::debug!(tab = id, ?pos, "mdi: tab menu from the header");
    slot.term.open_menu(pos);
}

fn overlay_for(tab: &crate::tabs::Tab) -> Option<Overlay> {
    let summary = tab.summary();
    match tab.status {
        TabStatus::Connected => None,
        TabStatus::Connecting => Some(Overlay::Connecting { summary }),
        TabStatus::Error => Some(Overlay::Failed { message: tab.message.clone().unwrap_or_else(|| "Unknown error".into()) }),
        TabStatus::Closed => Some(Overlay::Closed { reason: tab.message.clone(), summary }),
    }
}

fn terminal(app: &mut App, ui: &mut Ui, id: TabId, rect: Rect) {
    let modal = app.any_modal_open();
    let Some(tab) = app.tabs.get(id) else {
        return;
    };
    let active = app.tabs.active == Some(id);
    let Some(slot) = app.terms.get_mut(&id) else {
        tracing::error!(tab = id, "mdi: tab without a terminal");
        return;
    };
    // No overlay while a host-key prompt is up (the dialog says it all).
    let overlay = if slot.host_key.is_some() || slot.auth_fix.is_some() { None } else { overlay_for(tab) };
    let bg = {
        let c = app.term_style.palette.background;
        egui::Color32::from_rgb(c.r, c.g, c.b)
    };
    ui.painter().rect_filled(rect, 0.0, bg);
    // An auth-fix overlay owns its own tab's keyboard (not the others').
    let blocked = modal || slot.auth_fix.is_some();
    let focus = active && !blocked && tab.status == TabStatus::Connected && std::mem::take(&mut slot.focus);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("term", id)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let out = TerminalView::new(&mut slot.term, &app.term_style).focus(focus).overlay(overlay.as_ref()).input(!blocked).show(&mut child);
    if out.response.has_focus() {
        app.mdi.term_focused = true;
    }
    for action in out.actions {
        tracing::debug!(tab = id, ?action, "mdi: terminal action");
        match action {
            TermAction::Reconnect => app.connect_tab(id, false, false),
            TermAction::ConfirmPaste { text, lines } => app.dialogs.paste = Some((id, text, lines)),
            TermAction::OpenUrl(url) => {
                if let Err(e) = open::that_detached(&url) {
                    app.toasts.danger("Could not open link", format!("{url}: {e}"));
                }
            }
            TermAction::Bell => crate::bell::ring(),
            TermAction::OpenAppearance => crate::views::appearance::open(app, id),
        }
    }
}

/// Cascade title bar: drag to move, double-click to maximise, buttons.
fn chrome(app: &mut App, ui: &mut Ui, id: TabId, rect: Rect) {
    let t = tokens(ui.ctx());
    let Some(tab) = app.tabs.get(id).cloned() else {
        return;
    };
    let maximized = app.tabs.cascade.get(&id).is_some_and(|g| g.maximized);
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), CHROME_H));
    ui.painter().rect_filled(bar, egui::CornerRadius { nw: radius::LG as u8, ne: radius::LG as u8, sw: 0, se: 0 }, t.surface);
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, t.border));
    let resp = ui.interact(bar, Id::unique(("cascade-bar", id)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::Move);
    // The CentralPanel rect includes the tab strip; frames are laid out in
    // the area `show` computed.
    let area = app.mdi.frames.unwrap_or_else(|| ui.max_rect());
    widgets::status_dot(ui, Pos2::new(bar.left() + 14.0, bar.center().y), tab.status, 11.0);
    let mut bx = bar.right() - 6.0;
    let mut clicked = None;
    for (icon, tip, what) in [
        (Icon::X, "Close", 2),
        (if maximized { Icon::Minimize2 } else { Icon::Maximize2 }, if maximized { "Restore" } else { "Maximize" }, 1),
        (Icon::Minus, "Minimize", 0),
    ] {
        let r = Rect::from_center_size(Pos2::new(bx - 11.0, bar.center().y), vec2(22.0, 22.0));
        bx -= 24.0;
        let b = ui.interact(r, resp.id.with(tip), Sense::click());
        widgets::paint_icon_button(ui, &b, r, icon, 12.0, false, if what == 2 { Some(t.danger) } else { None });
        widgets::a11y(&b, egui::Role::Button, tip);
        if b.on_hover_text(tip).clicked() {
            clicked = Some(what);
        }
    }
    let name_rect = Rect::from_min_max(Pos2::new(bar.left() + 26.0, bar.top()), Pos2::new(bx - 4.0, bar.bottom()));
    widgets::paint_text(ui, name_rect, &tab.session.name, font(size::SM), t.fg);
    let resp = resp.on_hover_text(tab.summary());

    let frames_h = area.height();
    match clicked {
        Some(0) => app.tabs.minimize(id),
        Some(1) => app.tabs.toggle_maximize(id, area.width(), frames_h),
        Some(2) => app.request_close_tab(id),
        _ => {}
    }
    if resp.double_clicked() {
        app.tabs.toggle_maximize(id, area.width(), frames_h);
    }
    if resp.secondary_clicked() {
        let pos = resp.interact_pointer_pos().unwrap_or(bar.left_bottom());
        open_tab_menu(app, id, pos);
    }
    if resp.drag_started() && !maximized {
        start_drag(app, id, None, area.width(), frames_h);
    }
    // Resize handles (not while maximised).
    if !maximized {
        for edge in Edge::ALL {
            let r = handle_rect(rect, edge);
            let h = ui.interact(r, Id::unique(("cascade-edge", id, edge as u8)), Sense::drag()).on_hover_cursor(edge_cursor(edge));
            if h.drag_started() {
                start_drag(app, id, Some(edge), area.width(), frames_h);
            }
        }
    }
}

fn start_drag(app: &mut App, id: TabId, edge: Option<Edge>, area_w: f32, area_h: f32) {
    if let Some(g) = app.tabs.cascade.get(&id) {
        let (x, y, w, h) = g.displayed(area_w, area_h);
        app.mdi.cascade_drag = Some(CascadeDrag { tab: id, edge, start: (x, y, w, h) });
    }
}

fn handle_rect(r: Rect, edge: Edge) -> Rect {
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    match edge {
        Edge::N => Rect::from_min_max(Pos2::new(l + 8.0, t), Pos2::new(rr - 8.0, t + 4.0)),
        Edge::S => Rect::from_min_max(Pos2::new(l + 8.0, b - 4.0), Pos2::new(rr - 8.0, b)),
        Edge::E => Rect::from_min_max(Pos2::new(rr - 4.0, t + 8.0), Pos2::new(rr, b - 8.0)),
        Edge::W => Rect::from_min_max(Pos2::new(l, t + 8.0), Pos2::new(l + 4.0, b - 8.0)),
        Edge::NE => Rect::from_min_max(Pos2::new(rr - 8.0, t), Pos2::new(rr, t + 8.0)),
        Edge::NW => Rect::from_min_max(Pos2::new(l, t), Pos2::new(l + 8.0, t + 8.0)),
        Edge::SE => Rect::from_min_max(Pos2::new(rr - 8.0, b - 8.0), Pos2::new(rr, b)),
        Edge::SW => Rect::from_min_max(Pos2::new(l, b - 8.0), Pos2::new(l + 8.0, b)),
    }
}

fn edge_cursor(edge: Edge) -> CursorIcon {
    match edge {
        Edge::N | Edge::S => CursorIcon::ResizeVertical,
        Edge::E | Edge::W => CursorIcon::ResizeHorizontal,
        Edge::NE | Edge::SW => CursorIcon::ResizeNeSw,
        Edge::NW | Edge::SE => CursorIcon::ResizeNwSe,
    }
}

/// Apply an in-flight move / resize, clamped to the frame area. The
/// terminals under the pointer are not hit while it runs (a drag owns the
/// pointer in egui).
fn cascade_drag(app: &mut App, ui: &Ui, frames: Rect) {
    let Some(d) = app.mdi.cascade_drag else {
        return;
    };
    let (origin, now, down) = ui.ctx().input(|i| (i.pointer.press_origin(), i.pointer.latest_pos(), i.pointer.primary_down()));
    if !down {
        app.mdi.cascade_drag = None;
        return;
    }
    let (Some(o), Some(p)) = (origin, now) else {
        return;
    };
    let (dx, dy) = (p.x - o.x, p.y - o.y);
    let Some(g) = app.tabs.cascade.get_mut(&d.tab) else {
        app.mdi.cascade_drag = None;
        return;
    };
    match d.edge {
        None => {
            let (x, y) = CascadeGeom::dragged(d.start, dx, dy, frames.width(), frames.height());
            g.x = x;
            g.y = y;
        }
        Some(edge) => {
            let (x, y, w, h) = CascadeGeom::resized(d.start, edge, dx, dy, frames.width(), frames.height());
            *g = CascadeGeom { x, y, w, h, ..*g };
        }
    }
    if let Some(edge) = d.edge {
        ui.ctx().set_cursor_icon(edge_cursor(edge));
    }
}

fn minimized_strip(app: &mut App, ui: &mut Ui, rect: Rect) {
    let t = tokens(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.surface);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, t.border));
    let mut x = rect.left() + 8.0;
    let mut restore = None;
    let items: Vec<(TabId, String, TabStatus)> =
        app.tabs.tabs.iter().filter(|tab| app.tabs.is_minimized(tab.id)).map(|tab| (tab.id, tab.session.name.clone(), tab.status)).collect();
    for (id, name, status) in items {
        let g = widgets::elide(ui, &name, font(size::SM), 160.0);
        let w = 10.0 + 6.0 + 8.0 + g.size().x + 12.0;
        let r = Rect::from_min_size(Pos2::new(x, rect.center().y - 13.0), vec2(w, 26.0));
        x += w + 6.0;
        let resp = ui.interact(r, Id::unique(("minimized", id)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        ui.painter().rect_filled(r, radius::MD, if resp.hovered() { t.surface3 } else { t.surface2 });
        widgets::status_dot(ui, Pos2::new(r.left() + 13.0, r.center().y), status, 10.0);
        ui.painter().galley_with_override_text_color(Pos2::new(r.left() + 24.0, r.center().y - g.size().y / 2.0), g, if resp.hovered() { t.fg } else { t.fg2 });
        widgets::focus_ring(ui, &resp, r);
        widgets::a11y(&resp, egui::Role::Button, &format!("Restore {name}"));
        if resp.on_hover_text(format!("Restore {name}")).clicked() {
            restore = Some(id);
        }
    }
    if let Some(id) = restore {
        app.tabs.restore(id);
        app.activate(id);
    }
}
