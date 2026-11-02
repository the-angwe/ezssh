//! `main-shell.tsx` + `tabs-shell.tsx` layout: sidebar (or its collapsed
//! rail) on the left, tab strip and MDI area in the middle, status bar at
//! the bottom, and the shell-level dialogs on top.

use egui::{CursorIcon, Frame, Id, Pos2, Rect, Sense, Stroke, Ui, vec2};

use crate::app::App;
use crate::icons::Icon;
use crate::layout::{FORWARDS_PANE_W, SIDEBAR_DEFAULT, SIDEBAR_MAX, SIDEBAR_MIN, TERMINAL_MIN_W};
use crate::style::{STATUS_BAR_H, tokens};
use crate::views::{
    appearance, backup, credentials, dialogs, forwards, import, mdi, restore, session_dialog, settings, sftp, sidebar, statusbar, sync, tabstrip,
    vault_dialogs, xserver,
};
use crate::widgets;

pub fn show(app: &mut App, ui: &mut Ui) {
    let t = tokens(ui.ctx());
    let ctx = ui.ctx().clone();

    egui::Panel::bottom("status-bar").exact_size(STATUS_BAR_H).show_separator_line(false).resizable(false).frame(Frame::NONE.fill(t.surface)).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, t.border));
        statusbar::show(app, ui);
    });

    if app.tabs.sidebar_collapsed {
        egui::Panel::left("sidebar-rail").exact_size(24.0).show_separator_line(false).resizable(false).frame(Frame::NONE.fill(t.surface)).show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, t.border));
            let resp = ui.interact(r, Id::unique("sidebar-rail"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            widgets::a11y(&resp, egui::Role::Button, "Show sessions sidebar");
            if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.surface2);
            }
            Icon::PanelLeftOpen.paint(ui, r, 14.0, if resp.hovered() { t.fg } else { t.muted });
            if resp.on_hover_text("Show sessions (Ctrl+B)").clicked() {
                app.tabs.sidebar_collapsed = false;
            }
        });
    } else {
        let width = app.layout.sidebar_width;
        egui::Panel::left("sidebar").exact_size(width).show_separator_line(false).resizable(false).frame(Frame::NONE.fill(t.surface)).show(ui, |ui| {
            let pane = ui.max_rect();
            ui.painter().vline(pane.right() - 0.5, pane.y_range(), Stroke::new(1.0, t.border));
            sidebar::show(app, ui);
            // The sidebar must leave a usable terminal plus the active
            // tab's docked panes.
            let reserve = TERMINAL_MIN_W
                + app.tabs.active_tab().map_or(0.0, |tab| {
                    (if tab.sftp_open && tab.has_file_pane() { app.layout.sftp_pane_width } else { 0.0 })
                        + (if tab.forwards_open && tab.is_ssh() { FORWARDS_PANE_W } else { 0.0 })
                });
            let max = SIDEBAR_MAX.min(ctx.content_rect().width() - pane.left() - reserve);
            let out = widgets::resize_strip(
                ui,
                Id::unique("sidebar-resize"),
                pane,
                &mut app.layout.sidebar_width,
                widgets::Bounds { min: SIDEBAR_MIN, max, default: SIDEBAR_DEFAULT },
                "Resize sidebar",
            );
            if out.ended {
                app.save_layout();
            }
        });
    }

    egui::CentralPanel::no_frame().show(ui, |ui| {
        let r = ui.max_rect();
        let strip = Rect::from_min_size(r.min, vec2(r.width(), crate::style::TAB_BAR_H));
        let area = Rect::from_min_max(Pos2::new(r.left(), strip.bottom()), r.max);
        tabstrip::show(app, ui, strip);
        mdi::show(app, ui, area);
        sftp::handle_drops(app, ui.ctx());
    });

    shell_dialogs(app, &ctx);
    sidebar::dialogs(app, &ctx);
    if !app.mdi.term_focused {
        app.global_shortcuts(&ctx);
    }
}

fn shell_dialogs(app: &mut App, ctx: &egui::Context) {
    if let Some(id) = app.tabs.pending_close {
        let name = app.tabs.get(id).map(|t| t.session.name.clone()).unwrap_or_default();
        let answer = dialogs::confirm(
            ctx,
            Id::unique("confirm-close-tab"),
            dialogs::Confirm {
                title: &format!("Close \"{name}\"?"),
                body: "This tab is connected. Closing it ends the session.",
                confirm_text: "Close tab",
                danger: true,
            },
        );
        match answer {
            Some(true) => {
                app.tabs.pending_close = None;
                app.close_tab(id);
            }
            Some(false) => app.tabs.pending_close = None,
            None => {}
        }
    }

    if app.dialogs.quit_confirm {
        let answer = dialogs::confirm(
            ctx,
            Id::unique("confirm-quit"),
            dialogs::Confirm { title: "Quit ezSsh?", body: "Connected sessions will be closed.", confirm_text: "Quit", danger: true },
        );
        match answer {
            Some(true) => app.quit(),
            Some(false) => app.dialogs.quit_confirm = false,
            None => {}
        }
    }

    if let Some((tab, text, lines)) = &app.dialogs.paste {
        let (tab, lines) = (*tab, *lines);
        let answer = dialogs::confirm(
            ctx,
            Id::unique("confirm-paste"),
            dialogs::Confirm {
                title: "Paste multiple lines?",
                body: &format!("The clipboard holds {lines} lines. Each one will run as you paste it."),
                confirm_text: "Paste",
                danger: true,
            },
        );
        if let Some(yes) = answer {
            let text = text.clone();
            app.dialogs.paste = None;
            if let Some(slot) = app.terms.get_mut(&tab) {
                if yes {
                    slot.term.paste(&text);
                }
                slot.focus = true;
            }
        }
    }

    if let Some(draft) = &mut app.dialogs.tile_grid {
        match draft.show(ctx, Id::unique("tile-grid")) {
            Some(Some(grid)) => {
                app.dialogs.tile_grid = None;
                app.tabs.tile_grid = grid;
                app.tabs.view_mode = crate::tabs::ViewMode::TileGrid;
                app.save_layout();
            }
            Some(None) => app.dialogs.tile_grid = None,
            None => {}
        }
    }

    // Host-key prompts, one at a time, the active tab's first.
    let prompt_tab = app
        .tabs
        .active
        .filter(|id| app.terms.get(id).is_some_and(|s| s.host_key.is_some()))
        .or_else(|| app.tabs.tabs.iter().map(|t| t.id).find(|id| app.terms.get(id).is_some_and(|s| s.host_key.is_some())));
    if let Some(tab) = prompt_tab
        && let (Some(t), Some(slot)) = (app.tabs.get(tab), app.terms.get(&tab))
        && let Some(prompt) = &slot.host_key
    {
        let answer = dialogs::host_key(ctx, Id::unique(("host-key", tab)), &t.session.host, t.session.port, prompt);
        match answer {
            Some(true) => app.connect_tab(tab, true, false),
            Some(false) => {
                if let Some(slot) = app.terms.get_mut(&tab) {
                    slot.host_key = None;
                }
                app.tabs.set_status(tab, crate::tabs::TabStatus::Closed, None);
            }
            None => {}
        }
    }

    if app.dialogs.settings.is_some() {
        settings::show(app, ctx);
    }
    // Phase 5 dialogs. Each shows its `app.dialogs` slot when set; the
    // credentials manager comes after the session dialog so it stacks on
    // top when opened from a picker.
    session_dialog::show(app, ctx);
    credentials::show(app, ctx);
    forwards::dialogs(app, ctx);
    sftp::dialogs(app, ctx);
    vault_dialogs::show(app, ctx);
    xserver::show(app, ctx);
    appearance::show(app, ctx);
    backup::show(app, ctx);
    sync::show(app, ctx);
    restore::show(app, ctx);
    import::show(app, ctx);
}
