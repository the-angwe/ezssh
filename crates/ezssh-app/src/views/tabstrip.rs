//! The tab strip: rounded tab cards that merge into the terminal, the
//! active tab's pane toggles and the Layout menu.

use egui::{CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, Ui, vec2};

use crate::app::App;
use crate::icons::Icon;
use crate::style::{alpha, font, radius, size, tokens};
use crate::tabs::{TabId, ViewMode};
use crate::views::dialogs::TileGridDraft;
use crate::widgets::{self, Menu, MenuEntry, MenuResult};

/// Tab width bounds (px): the floor clears a tab's fixed chrome (dots,
/// close button) plus a few letters.
const TAB_MIN_W: f32 = 120.0;
const TAB_MAX_W: f32 = 200.0;
/// Height of a tab card inside the strip (the rest is the gap above).
const TAB_H: f32 = 30.0;
const TAB_GAP: f32 = 2.0;
const CHEVRON_W: f32 = 24.0;
/// A drag released further than this above / below the strip is a no-op.
const STRIP_Y_TOLERANCE: f32 = 24.0;
const AUTOSCROLL_EDGE: f32 = 28.0;
const AUTOSCROLL_SPEED: f32 = 10.0;

#[derive(Default)]
pub struct StripState {
    scroll: f32,
    /// Scroll the active tab into view on the next frame.
    pub reveal_active: bool,
    drag: Option<TabId>,
    /// (index, right side) of the insertion indicator.
    drop: Option<(usize, bool)>,
    /// The Layout ▾ menu while open.
    layout_menu: Option<Menu<ViewMode>>,
}

/// Widths for natural sizes `natural` in `avail` px: shrink in proportion
/// down to the minimum, after which the strip scrolls.
pub fn fit_widths(natural: &[f32], avail: f32) -> Vec<f32> {
    let total: f32 = natural.iter().sum();
    if total <= avail || total <= 0.0 {
        return natural.to_vec();
    }
    natural.iter().map(|w| (w * avail / total).max(TAB_MIN_W)).collect()
}

/// Drop target for pointer x among tab rects (index, right side).
pub fn drop_target(xs: &[(f32, f32)], x: f32) -> Option<(usize, bool)> {
    let first = xs.first()?;
    let last = xs.last()?;
    if x < first.0 {
        return Some((0, false));
    }
    if x >= last.1 {
        return Some((xs.len() - 1, true));
    }
    xs.iter().position(|(l, r)| x >= *l && x < *r).map(|i| (i, x >= (xs[i].0 + xs[i].1) / 2.0))
}

const MODE_ICONS: [(ViewMode, Icon); 6] = [
    (ViewMode::Tabs, Icon::LayoutPanelLeft),
    (ViewMode::TileH, Icon::Rows2),
    (ViewMode::TileV, Icon::Columns2),
    (ViewMode::TileGrid, Icon::Grid3x3),
    (ViewMode::Cascade, Icon::SquareStack),
    (ViewMode::Auto, Icon::Sparkles),
];

pub fn show(app: &mut App, ui: &mut Ui, strip: Rect) {
    let t = tokens(ui.ctx());
    ui.painter().rect_filled(strip, 0.0, t.surface);

    // Right end: the active tab's pane toggles, then Layout ▾.
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(8.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    right.spacing_mut().item_spacing.x = 8.0;
    let mode = app.tabs.view_mode;
    let layout_icon = MODE_ICONS.iter().find(|(m, _)| *m == mode).map_or(Icon::LayoutPanelLeft, |(_, i)| *i);
    let layout_btn = widgets::Button::ghost("Layout").icon(layout_icon).chevron().small().tooltip("View mode (Ctrl+Shift+W cycles)").show(&mut right);
    if layout_btn.clicked() {
        let entries = MODE_ICONS
            .iter()
            .map(|(m, icon)| MenuEntry::item(m.label(), *m).icon(*icon).checked(*m == mode))
            .collect();
        app.strip.layout_menu = Some(Menu::new(Pos2::new(layout_btn.rect.right() - 200.0, layout_btn.rect.bottom() + 4.0), entries));
    }
    let mut toggle_sftp = None;
    let mut toggle_fwd = None;
    if let Some(tab) = app.tabs.active_tab() {
        let mut items: Vec<(Icon, bool, &str, u8)> = Vec::new();
        if tab.has_file_pane() {
            items.push((Icon::FolderTree, tab.sftp_open, if tab.sftp_open { "Hide file pane" } else { "Show file pane" }, 0));
        }
        if tab.is_ssh() {
            items.push((Icon::Network, tab.forwards_open, if tab.forwards_open { "Hide forwards pane" } else { "Show forwards pane" }, 1));
        }
        if !items.is_empty() {
            let shown: Vec<(Icon, bool, &str)> = items.iter().map(|(i, on, l, _)| (*i, *on, *l)).collect();
            if let Some(i) = widgets::toggle_group(&mut right, Id::unique("pane-toggles"), &shown) {
                match items[i].3 {
                    0 => toggle_sftp = Some(tab.id),
                    _ => toggle_fwd = Some(tab.id),
                }
            }
        }
    }
    let tools_left = right.min_rect().left() - 8.0;

    let area = Rect::from_min_max(strip.min, Pos2::new(tools_left, strip.bottom()));
    // Hairline where the strip meets the terminal; the active tab's card
    // interrupts it below.
    let mut gap: Option<(f32, f32)> = None;
    if app.tabs.tabs.is_empty() {
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, t.border));
        let r = area.shrink2(vec2(14.0, 0.0));
        Icon::Terminal.paint(ui, Rect::from_center_size(Pos2::new(r.left() + 7.0, r.center().y), vec2(14.0, 14.0)), 14.0, t.muted);
        widgets::paint_text(ui, r.with_min_x(r.left() + 22.0), "No open tabs — double-click a session in the sidebar", font(size::SM), t.muted);
        layout_menu(app, ui.ctx());
        return;
    }

    // Natural widths.
    let userhost = app.settings.str("appearance.tabLabel") == "userhost";
    let labels: Vec<String> = app.tabs.tabs.iter().map(|tab| tab.label(userhost)).collect();
    let natural: Vec<f32> = labels
        .iter()
        .map(|label| {
            let text = ui.painter().layout_no_wrap(label.clone(), font(size::BASE), t.fg).size().x;
            (12.0 + 6.0 + 8.0 + text + 8.0 + 20.0 + 8.0).clamp(TAB_MIN_W, TAB_MAX_W)
        })
        .collect();
    let overflow = fit_widths(&natural, area.width() - 8.0).iter().sum::<f32>() > area.width() - 8.0 + 0.5;
    let inner = if overflow { area.shrink2(vec2(CHEVRON_W, 0.0)) } else { area.shrink2(vec2(4.0, 0.0)) };
    let widths = fit_widths(&natural, inner.width());
    let content_w: f32 = widths.iter().sum();
    let max_scroll = (content_w - inner.width()).max(0.0);

    // Keep the active tab visible.
    let st = &mut app.strip;
    if std::mem::take(&mut st.reveal_active)
        && let Some(i) = app.tabs.active.and_then(|a| app.tabs.index_of(a))
    {
        let left: f32 = widths[..i].iter().sum();
        let right = left + widths[i];
        if left < st.scroll {
            st.scroll = left;
        } else if right > st.scroll + inner.width() {
            st.scroll = right - inner.width();
        }
    }
    // Wheel over the strip scrolls it horizontally.
    let strip_resp = ui.interact(area, Id::unique("tab-strip"), Sense::hover());
    if strip_resp.hovered() {
        let d = ui.ctx().input(|i| if i.smooth_scroll_delta.x != 0.0 { i.smooth_scroll_delta.x } else { i.smooth_scroll_delta.y });
        st.scroll -= d;
    }
    st.scroll = st.scroll.clamp(0.0, max_scroll);

    if overflow {
        for (left_side, icon) in [(true, Icon::ChevronLeft), (false, Icon::ChevronRight)] {
            let r = if left_side {
                Rect::from_min_size(area.min, vec2(CHEVRON_W, area.height()))
            } else {
                Rect::from_min_max(Pos2::new(area.right() - CHEVRON_W, area.top()), area.max)
            };
            let enabled = if left_side { st.scroll > 0.0 } else { st.scroll < max_scroll - 1.0 };
            let resp = ui.interact(r, Id::unique(("tab-chevron", left_side)), if enabled { Sense::click() } else { Sense::hover() });
            let br = Rect::from_center_size(r.center(), vec2(22.0, 22.0));
            if enabled && resp.hovered() {
                ui.painter().rect_filled(br, radius::MD, t.surface2);
            }
            icon.paint(ui, br, 14.0, if !enabled { alpha(t.muted, 0.4) } else if resp.hovered() { t.fg } else { t.muted });
            let label = if left_side { "Scroll tabs left" } else { "Scroll tabs right" };
            widgets::a11y(&resp, egui::Role::Button, label);
            let resp = resp.on_hover_text(label);
            if resp.clicked() {
                let step = 120.0_f32.max(inner.width() * 0.6);
                st.scroll = (st.scroll + if left_side { -step } else { step }).clamp(0.0, max_scroll);
            }
        }
    }

    // Tabs.
    let painter_clip = ui.clip_rect().intersect(inner);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    child.set_clip_rect(painter_clip);
    let mut x = inner.left() - st.scroll;
    let mut spans = Vec::with_capacity(widths.len());
    let mut activate = None;
    let mut close = None;
    let mut menu = None;
    let dragging = st.drag;
    let top_round = CornerRadius { nw: radius::LG as u8, ne: radius::LG as u8, sw: 0, se: 0 };
    for (i, tab) in app.tabs.tabs.iter().enumerate() {
        let slot = Rect::from_min_size(Pos2::new(x, inner.top()), vec2(widths[i], inner.height()));
        spans.push((slot.left(), slot.right()));
        x += widths[i];
        let r = Rect::from_min_max(Pos2::new(slot.left() + TAB_GAP / 2.0, slot.bottom() - TAB_H), Pos2::new(slot.right() - TAB_GAP / 2.0, slot.bottom()));
        let resp = child.interact(r, Id::unique(("tab", tab.id)), Sense::click_and_drag());
        let on = app.tabs.active == Some(tab.id);
        widgets::a11y_selected(&resp, egui::Role::Tab, on, &labels[i]);
        let hovered = resp.hovered();
        if on {
            child.painter().rect_filled(r, top_round, t.bg);
            // The card's sides: hairlines up to the strip's bottom.
            child.painter().vline(r.left() + 0.5, r.y_range(), Stroke::new(1.0, t.border));
            child.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, t.border));
            child.painter().hline((r.left() + 1.0)..=(r.right() - 1.0), r.top() + 0.5, Stroke::new(1.0, t.border));
            gap = Some((r.left().max(inner.left()), r.right().min(inner.right())));
        } else if hovered {
            child.painter().rect_filled(r, top_round, t.surface2);
        }
        let fade = if dragging == Some(tab.id) { 0.5 } else { 1.0 };
        let mut cx = r.left() + 12.0;
        let cy = r.center().y;
        if let Some(c) = tab.color.as_deref().and_then(crate::views::sidebar::parse_color) {
            // The session colour: a 2 px edge along the top of the card.
            child.painter().rect_filled(Rect::from_min_max(Pos2::new(r.left() + 1.0, r.top() + 1.0), Pos2::new(r.right() - 1.0, r.top() + 3.0)), CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, alpha(c, fade));
        }
        widgets::status_dot(&child, Pos2::new(cx + 3.0, cy), tab.status, 11.0);
        cx += 6.0 + 8.0;

        // Close ✕ at the right, shown on the active tab and on hover.
        let close_r = Rect::from_center_size(Pos2::new(r.right() - 8.0 - 10.0, cy), vec2(20.0, 20.0));
        let b = child.interact(close_r, resp.id.with("close"), Sense::click());
        if on || hovered || b.hovered() {
            widgets::paint_icon_button(&child, &b, close_r, Icon::X, 12.0, false, None);
        }
        widgets::a11y(&b, egui::Role::Button, "Close tab");
        if b.on_hover_text("Close tab").clicked() {
            close = Some(tab.id);
        }
        let text_color = alpha(if on || hovered { t.fg } else { t.fg2 }, fade);
        let name_rect = Rect::from_min_max(Pos2::new(cx, r.top()), Pos2::new(close_r.left() - 6.0, r.bottom()));
        widgets::paint_text(&child, name_rect, &labels[i], font(size::BASE), text_color);
        let resp = resp
            .on_hover_ui(|ui| {
                let mut tip = format!("{} ({})", tab.summary(), tab.status.label());
                if let Some(m) = &tab.message {
                    tip = format!("{tip}\n{m}");
                }
                ui.label(tip);
            })
            .on_hover_cursor(if dragging.is_some() { CursorIcon::Grabbing } else { CursorIcon::PointingHand });
        if resp.clicked() {
            activate = Some(tab.id);
        }
        if resp.middle_clicked() {
            close = Some(tab.id);
        }
        if resp.secondary_clicked() {
            menu = Some((tab.id, resp.interact_pointer_pos().unwrap_or(r.left_bottom())));
        }
        if resp.drag_started() {
            st.drag = Some(tab.id);
        }
    }
    // The bottom hairline, interrupted under the active card.
    let y = strip.bottom() - 0.5;
    match gap {
        Some((l, r)) => {
            ui.painter().hline(strip.left()..=l, y, Stroke::new(1.0, t.border));
            ui.painter().hline(r..=strip.right(), y, Stroke::new(1.0, t.border));
        }
        None => {
            ui.painter().hline(strip.x_range(), y, Stroke::new(1.0, t.border));
        }
    }
    // Insertion indicator.
    if let Some((i, right)) = st.drop
        && let Some((l, r)) = spans.get(i)
    {
        let x = if right { *r - 2.0 } else { *l };
        child.painter().rect_filled(Rect::from_min_size(Pos2::new(x, inner.bottom() - TAB_H), vec2(2.0, TAB_H)), 1.0, t.accent);
    }

    // Drag reorder.
    if let Some(src) = st.drag {
        let (pos, down, escape) = ui.ctx().input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.key_pressed(egui::Key::Escape)));
        let target = pos.and_then(|p| {
            if p.y < area.top() - STRIP_Y_TOLERANCE || p.y > area.bottom() + STRIP_Y_TOLERANCE {
                return None;
            }
            drop_target(&spans, p.x.clamp(inner.left(), inner.right() - 1.0))
        });
        let src_index = app.tabs.index_of(src);
        // Dropping onto the source tab is a no-op; no indicator there.
        st.drop = target.filter(|(i, _)| Some(*i) != src_index);
        if let Some(p) = pos
            && down
        {
            let d = if p.x < inner.left() + AUTOSCROLL_EDGE {
                -AUTOSCROLL_SPEED
            } else if p.x > inner.right() - AUTOSCROLL_EDGE {
                AUTOSCROLL_SPEED
            } else {
                0.0
            };
            if d != 0.0 {
                st.scroll = (st.scroll + d).clamp(0.0, max_scroll);
                ui.ctx().request_repaint();
            }
        }
        if escape {
            st.drag = None;
            st.drop = None;
        } else if !down {
            st.drag = None;
            let drop = st.drop.take();
            if let (Some(from), Some((i, right))) = (src_index, drop) {
                let to = if right { i + 1 } else { i };
                tracing::debug!(from, to, "tabstrip: reorder");
                app.tabs.reorder(from, to);
            }
        }
    }

    if let Some(id) = toggle_sftp
        && let Some(tab) = app.tabs.get_mut(id)
    {
        tab.sftp_open = !tab.sftp_open;
    }
    if let Some(id) = toggle_fwd
        && let Some(tab) = app.tabs.get_mut(id)
    {
        tab.forwards_open = !tab.forwards_open;
    }
    if let Some(id) = activate {
        app.activate(id);
    }
    if let Some((id, pos)) = menu {
        crate::views::mdi::open_tab_menu(app, id, pos);
    }
    if let Some(id) = close {
        app.request_close_tab(id);
    }
    layout_menu(app, ui.ctx());
}

/// The Layout ▾ menu: the six view modes, the current one checked.
fn layout_menu(app: &mut App, ctx: &egui::Context) {
    let Some(m) = &mut app.strip.layout_menu else {
        return;
    };
    match m.show(ctx, Id::unique("layout-menu")) {
        MenuResult::Open => {}
        MenuResult::Closed => app.strip.layout_menu = None,
        MenuResult::Chosen(mode) => {
            app.strip.layout_menu = None;
            tracing::debug!(mode = mode.id(), "tabstrip: view mode picked");
            if mode == ViewMode::TileGrid {
                app.dialogs.tile_grid = Some(TileGridDraft::new(app.tabs.tile_grid));
            } else {
                app.tabs.view_mode = mode;
                app.save_layout();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_shrink_proportionally_to_the_floor() {
        assert_eq!(fit_widths(&[200.0, 220.0], 1000.0), vec![200.0, 220.0]);
        let w = fit_widths(&[200.0, 200.0, 200.0], 450.0);
        assert!(w.iter().all(|x| (*x - 150.0).abs() < 0.01));
        // Floor reached: the strip will scroll.
        assert_eq!(fit_widths(&[200.0, 200.0, 200.0], 300.0), vec![120.0; 3]);
    }

    #[test]
    fn drop_targets() {
        let xs = [(0.0, 100.0), (100.0, 200.0)];
        assert_eq!(drop_target(&xs, -5.0), Some((0, false)));
        assert_eq!(drop_target(&xs, 20.0), Some((0, false)));
        assert_eq!(drop_target(&xs, 60.0), Some((0, true)));
        assert_eq!(drop_target(&xs, 120.0), Some((1, false)));
        assert_eq!(drop_target(&xs, 250.0), Some((1, true)));
        assert_eq!(drop_target(&[], 5.0), None);
    }
}
