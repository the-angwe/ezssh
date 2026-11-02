//! `tabs-shell.tsx` strip + `view-mode-toolbar.tsx`.

use egui::{CursorIcon, Id, Pos2, Rect, Sense, Stroke, Ui, vec2};

use crate::app::App;
use crate::icons::Icon;
use crate::style::{alpha, font, size, tokens};
use crate::tabs::{TabId, ViewMode};
use crate::views::dialogs::TileGridDraft;
use crate::widgets;

/// Tab width bounds (px): the floor clears an SSH tab's fixed chrome.
const TAB_MIN_W: f32 = 180.0;
const TAB_MAX_W: f32 = 260.0;
const CHEVRON_W: f32 = 24.0;
const MODE_W: f32 = 32.0;
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

pub fn show(app: &mut App, ui: &mut Ui, strip: Rect) {
    let t = tokens(ui.ctx());
    ui.painter().rect_filled(strip, 0.0, t.surface);
    ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, t.border));

    // View-mode toolbar on the right.
    let toolbar = Rect::from_min_max(Pos2::new(strip.right() - MODE_W * 6.0, strip.top()), Pos2::new(strip.right(), strip.bottom() - 1.0));
    ui.painter().vline(toolbar.left(), toolbar.y_range(), Stroke::new(1.0, t.border));
    let icons = [Icon::LayoutGrid, Icon::Rows2, Icon::Columns2, Icon::Grid3x3, Icon::SquareStack, Icon::Sparkles];
    for (i, (mode, icon)) in ViewMode::ALL.into_iter().zip(icons).enumerate() {
        let r = Rect::from_min_size(Pos2::new(toolbar.left() + i as f32 * MODE_W, toolbar.top()), vec2(MODE_W, toolbar.height()));
        let resp = ui.interact(r, Id::unique(("view-mode", mode.id())), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let on = app.tabs.view_mode == mode;
        widgets::a11y_selected(&resp, egui::Role::Button, on, mode.label());
        if resp.hovered() && !on {
            ui.painter().rect_filled(r, 0.0, alpha(t.surface2, 0.4));
        }
        icon.paint(ui, r, 14.0, if on || resp.hovered() { t.fg } else { t.muted });
        if on {
            ui.painter().rect_filled(Rect::from_min_max(Pos2::new(r.left() + 4.0, r.bottom() - 2.0), Pos2::new(r.right() - 4.0, r.bottom())), 0.0, t.accent);
        }
        if resp.on_hover_text(mode.label()).clicked() {
            if mode == ViewMode::TileGrid {
                app.dialogs.tile_grid = Some(TileGridDraft::new(app.tabs.tile_grid));
            } else {
                app.tabs.view_mode = mode;
                app.save_layout();
            }
        }
    }

    let area = Rect::from_min_max(strip.min, Pos2::new(toolbar.left(), strip.bottom() - 1.0));
    if app.tabs.tabs.is_empty() {
        let r = area.shrink2(vec2(12.0, 0.0));
        Icon::Terminal.paint(ui, Rect::from_center_size(Pos2::new(r.left() + 6.0, r.center().y), vec2(12.0, 12.0)), 12.0, t.muted);
        widgets::paint_text(ui, r.with_min_x(r.left() + 20.0), "No open tabs — double-click a session in the sidebar", font(size::XS), t.muted);
        return;
    }

    // Natural widths.
    let userhost = app.settings.str("appearance.tabLabel") == "userhost";
    let labels: Vec<String> = app.tabs.tabs.iter().map(|tab| tab.label(userhost)).collect();
    let natural: Vec<f32> = app
        .tabs
        .tabs
        .iter()
        .zip(&labels)
        .map(|(tab, label)| {
            let text = ui.painter().layout_no_wrap(label.clone(), font(size::BASE), t.fg).size().x;
            let mut w = 24.0 + 11.0 + 8.0 + text + 8.0 + 20.0;
            if tab.session.color.is_some() {
                w += 6.0 + 8.0;
            }
            if tab.has_file_pane() {
                w += 4.0 + 20.0 + 8.0;
            }
            if tab.is_ssh() {
                w += 20.0 + 8.0;
            }
            w.clamp(TAB_MIN_W, TAB_MAX_W)
        })
        .collect();
    let overflow = fit_widths(&natural, area.width()).iter().sum::<f32>() > area.width() + 0.5;
    let inner = if overflow { area.shrink2(vec2(CHEVRON_W, 0.0)) } else { area };
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
            let x = if left_side { r.right() } else { r.left() };
            ui.painter().vline(x, r.y_range(), Stroke::new(1.0, t.border));
            let resp = ui.interact(r, Id::unique(("tab-chevron", left_side)), if enabled { Sense::click() } else { Sense::hover() });
            if enabled && resp.hovered() {
                ui.painter().rect_filled(r, 0.0, alpha(t.surface2, 0.4));
            }
            icon.paint(ui, r, 14.0, if !enabled { alpha(t.muted, 0.4) } else if resp.hovered() { t.fg } else { t.muted });
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
    let mut toggle_sftp = None;
    let mut toggle_fwd = None;
    let dragging = st.drag;
    for (i, tab) in app.tabs.tabs.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(x, inner.top()), vec2(widths[i], inner.height()));
        spans.push((r.left(), r.right()));
        x += widths[i];
        let resp = child.interact(r, Id::unique(("tab", tab.id)), Sense::click_and_drag());
        let on = app.tabs.active == Some(tab.id);
        widgets::a11y_selected(&resp, egui::Role::Tab, on, &labels[i]);
        let hovered = resp.hovered();
        if on {
            child.painter().rect_filled(r, 0.0, t.bg);
        } else if hovered {
            child.painter().rect_filled(r, 0.0, alpha(t.surface2, 0.4));
        }
        child.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, t.border));
        if on {
            child.painter().rect_filled(Rect::from_min_max(Pos2::new(r.left(), r.bottom() - 2.0), r.max), 0.0, t.accent);
        }
        let fade = if dragging == Some(tab.id) { 0.5 } else { 1.0 };
        let mut cx = r.left() + 12.0;
        let cy = r.center().y;
        if let Some(c) = tab.session.color.as_deref().and_then(crate::views::sidebar::parse_color) {
            child.painter().circle_filled(Pos2::new(cx + 3.0, cy), 3.0, alpha(c, fade));
            cx += 6.0 + 8.0;
        }
        widgets::status_dot(&child, Pos2::new(cx + 5.5, cy), tab.status, 11.0);
        cx += 11.0 + 8.0;

        // Buttons from the right: close, forwards, file pane.
        let mut bx = r.right() - 12.0;
        let mut button = |icon: Icon, pressed: bool, tip: &str, always: bool, ui: &mut Ui| -> bool {
            let br = Rect::from_center_size(Pos2::new(bx - 10.0, cy), vec2(20.0, 20.0));
            bx -= 20.0;
            let b = ui.interact(br, resp.id.with(tip), Sense::click());
            if always || on || hovered || b.hovered() {
                widgets::paint_icon_button(ui, &b, br, icon, 12.0, pressed, None);
            }
            widgets::a11y_selected(&b, egui::Role::Button, pressed, tip);
            b.on_hover_text(tip).clicked()
        };
        if button(Icon::X, false, "Close tab", false, &mut child) {
            close = Some(tab.id);
        }
        if tab.is_ssh() {
            let tip = if tab.forwards_open { "Hide forwards pane" } else { "Show forwards pane" };
            if button(Icon::Network, tab.forwards_open, tip, true, &mut child) {
                toggle_fwd = Some(tab.id);
            }
        }
        if tab.has_file_pane() {
            let tip = if tab.sftp_open { "Hide file pane" } else { "Show file pane" };
            if button(Icon::FolderTree, tab.sftp_open, tip, true, &mut child) {
                toggle_sftp = Some(tab.id);
            }
            bx -= 4.0;
        }
        let text_color = alpha(if on || hovered { t.fg } else { t.muted }, fade);
        let name_rect = Rect::from_min_max(Pos2::new(cx, r.top()), Pos2::new(bx - 8.0, r.bottom()));
        widgets::paint_text(&child, name_rect, &labels[i], font(size::BASE), text_color);
        let mut tip = format!("{} ({})", tab.summary(), tab.status.label());
        if let Some(m) = &tab.message {
            tip = format!("{tip}\n{m}");
        }
        let resp = resp.on_hover_text(tip).on_hover_cursor(if dragging.is_some() { CursorIcon::Grabbing } else { CursorIcon::Grab });
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
    // Insertion indicator.
    if let Some((i, right)) = st.drop
        && let Some((l, r)) = spans.get(i)
    {
        let x = if right { *r - 2.0 } else { *l };
        child.painter().rect_filled(Rect::from_min_size(Pos2::new(x, inner.top()), vec2(2.0, inner.height())), 0.0, t.accent);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_shrink_proportionally_to_the_floor() {
        assert_eq!(fit_widths(&[200.0, 220.0], 1000.0), vec![200.0, 220.0]);
        let w = fit_widths(&[260.0, 260.0, 260.0], 600.0);
        assert!(w.iter().all(|x| (*x - 200.0).abs() < 0.01));
        // Floor reached: the strip will scroll.
        assert_eq!(fit_widths(&[260.0, 260.0, 260.0], 300.0), vec![180.0; 3]);
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
