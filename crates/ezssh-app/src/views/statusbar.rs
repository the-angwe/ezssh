//! The status bar: state only. Connection dot and `user@host`, then chips
//! for the SFTP cwd, the X server and a known update, and the version.
//! The app actions live in the sidebar footer.

use egui::{Align, Layout, Pos2, Rect, RichText, Ui, vec2};

use crate::app::App;
use crate::style::{alpha, font, size, tokens};
use crate::widgets;

pub fn show(app: &mut App, ui: &mut Ui) {
    let t = tokens(ui.ctx());
    let r = ui.max_rect().shrink2(vec2(12.0, 0.0));
    let active = app.tabs.active_tab().cloned();

    // Left: dot + summary.
    let dot_center = Pos2::new(r.left() + 3.0, r.center().y);
    let label = match active.as_ref().map(|a| a.status) {
        Some(status) => {
            widgets::status_dot(ui, dot_center, status, 11.0);
            status.label()
        }
        None => {
            ui.painter().circle_filled(dot_center, 3.0, alpha(t.muted, 0.6));
            "idle"
        }
    };
    ui.interact(Rect::from_center_size(dot_center, vec2(10.0, 10.0)), ui.scope_id().with("dot"), egui::Sense::hover()).on_hover_text(label);
    let summary = active.as_ref().map_or_else(|| "No active session".to_string(), |a| a.summary());

    // Right: chips, laid out from the right edge.
    let mut open_update = false;
    let right = ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(Layout::right_to_left(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).font(font(size::XS)).color(alpha(t.muted, 0.7)));
        if let Some(u) = &app.updater.available {
            let text = format!("v{} available", u.version);
            if widgets::chip_button(ui, &text, Some(t.accent), false, &format!("Update available: v{}", u.version)).on_hover_text("Open the update dialog").clicked() {
                open_update = true;
            }
        }
        if let Some(x) = &app.xstatus
            && (!x.running_displays.is_empty() || x.installed)
        {
            xserver_chip(ui, x);
        }
        if let Some(a) = active.as_ref().filter(|a| a.sftp_open) {
            widgets::chip(ui, &a.cwd, None, true).on_hover_text(format!("File pane: {}", a.cwd));
        }
        ui.min_rect().left()
    });
    if open_update {
        crate::views::update::open(app);
    }
    let right_edge = right.inner - 12.0;

    let text_left = dot_center.x + 11.0;
    let text_rect = Rect::from_min_max(Pos2::new(text_left, r.top()), Pos2::new(right_edge.max(text_left), r.bottom()));
    let shown = widgets::paint_text(ui, text_rect, &summary, font(size::SM), t.fg2);
    ui.interact(shown, ui.scope_id().with("summary"), egui::Sense::hover()).on_hover_text(&summary);
}

fn xserver_chip(ui: &mut Ui, x: &ezssh_core::xserver::XServerStatus) {
    let t = tokens(ui.ctx());
    let running = !x.running_displays.is_empty();
    let (color, label, tip) = if running {
        let displays: Vec<String> = x.running_displays.iter().map(|d| format!(":{d}")).collect();
        (Some(t.success), format!("X :{}", x.running_displays[0]), format!("VcXsrv running on {}", displays.join(", ")))
    } else if x.installed {
        (
            None,
            "X idle".to_string(),
            format!(
                "VcXsrv installed at {} — starts when an SSH session forwards X11",
                x.install_path.as_deref().unwrap_or("?")
            ),
        )
    } else {
        (Some(t.warning), "X off".to_string(), "VcXsrv not installed — required for X11 forwarding".to_string())
    };
    widgets::chip(ui, &label, color, false).on_hover_text(tip);
}
