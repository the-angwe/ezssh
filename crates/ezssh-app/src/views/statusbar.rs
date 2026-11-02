//! The status bar: connection dot and `user@host`, SFTP cwd, X server
//! pill, version, theme / settings / updates / lock buttons.

use egui::{Align, FontId, Layout, Pos2, Rect, RichText, Ui, vec2};

use crate::app::App;
use crate::icons::Icon;
use crate::style::{alpha, font, size, tokens};
use crate::tabs::TabStatus;
use crate::widgets;

pub fn show(app: &mut App, ui: &mut Ui) {
    let t = tokens(ui.ctx());
    let r = ui.max_rect().shrink2(vec2(8.0, 0.0));
    let mono = FontId::new(size::XS, app.mono.clone());
    let active = app.tabs.active_tab().cloned();

    // Left: dot + summary (+ SFTP cwd).
    let (dot, label) = match active.as_ref().map(|a| a.status) {
        Some(TabStatus::Connected) => (t.success, "connected"),
        Some(TabStatus::Connecting) => (alpha(t.warning, widgets::pulse(ui.ctx())), "connecting"),
        Some(TabStatus::Error) => (t.danger, "error"),
        Some(TabStatus::Closed) => (t.muted, "closed"),
        None => (t.muted, "idle"),
    };
    let dot_center = Pos2::new(r.left() + 3.0, r.center().y);
    ui.painter().circle_filled(dot_center, 3.0, dot);
    ui.interact(Rect::from_center_size(dot_center, vec2(10.0, 10.0)), ui.scope_id().with("dot"), egui::Sense::hover()).on_hover_text(label);

    let mut summary = active.as_ref().map_or_else(|| "No active session".to_string(), |a| a.summary());
    if let Some(a) = active.as_ref().filter(|a| a.sftp_open) {
        summary = format!("{summary}  ·  SFTP {}", a.cwd);
    }

    // Right: buttons, laid out from the right edge.
    let mut right = ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(Layout::right_to_left(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if widgets::icon_button(ui, Icon::Lock, 20.0, 12.0, false, "Lock vault").clicked() {
            app.lock();
        }
        updates_button(app, ui);
        if widgets::icon_button(ui, Icon::Settings, 20.0, 12.0, false, "Settings").clicked() {
            app.dialogs.settings = Some(crate::views::settings::SettingsDialog::default());
        }
        let dark = app.theme == crate::style::Theme::Dark;
        if widgets::icon_button(ui, if dark { Icon::Sun } else { Icon::Moon }, 20.0, 12.0, false, if dark { "Light theme" } else { "Dark theme" })
            .clicked()
        {
            app.set_theme(app.theme.toggled());
        }
        sep(ui);
        ui.label(RichText::new(format!("ezSsh v{}", env!("CARGO_PKG_VERSION"))).font(font(size::XS)).color(alpha(t.muted, 0.6)));
        if let Some(x) = &app.xstatus
            && (!x.running_displays.is_empty() || x.installed)
        {
            sep(ui);
            xserver_pill(ui, x);
        }
        ui.min_rect().left()
    });
    let right_edge = right.inner - 8.0;
    right.response.rect.set_left(right_edge);

    let text_left = dot_center.x + 9.0;
    let text_rect = Rect::from_min_max(Pos2::new(text_left, r.top()), Pos2::new(right_edge.max(text_left), r.bottom()));
    let shown = widgets::paint_text(ui, text_rect, &summary, mono, t.muted);
    ui.interact(shown, ui.scope_id().with("summary"), egui::Sense::hover()).on_hover_text(&summary);
}

/// Sparkles: opens the update dialog; a dot while a newer release is known.
fn updates_button(app: &mut App, ui: &mut Ui) {
    let t = tokens(ui.ctx());
    let enabled = app.updater.cfg.is_some();
    let tip = match &app.updater.available {
        Some(u) => format!("Update available: v{}", u.version),
        None => "Check for updates".to_string(),
    };
    let resp = ui.add_enabled_ui(enabled, |ui| widgets::icon_button(ui, Icon::Sparkles, 20.0, 12.0, false, &tip)).inner;
    let resp = resp.on_disabled_hover_text("Update checks are off in this run");
    if app.updater.available.is_some() {
        ui.painter().circle_filled(resp.rect.right_top() + vec2(-4.0, 4.0), 3.0, t.accent);
    }
    if resp.clicked() {
        crate::views::update::open(app);
    }
}

fn sep(ui: &mut Ui) {
    let t = tokens(ui.ctx());
    ui.label(RichText::new("·").font(font(size::XS)).color(alpha(t.muted, 0.4)));
}

fn xserver_pill(ui: &mut Ui, x: &ezssh_core::xserver::XServerStatus) {
    let t = tokens(ui.ctx());
    let running = !x.running_displays.is_empty();
    let (color, label, tip) = if running {
        let displays: Vec<String> = x.running_displays.iter().map(|d| format!(":{d}")).collect();
        (t.success, format!("X:{}", x.running_displays[0]), format!("VcXsrv running on {}", displays.join(", ")))
    } else if x.installed {
        (
            t.muted,
            "X idle".to_string(),
            format!(
                "VcXsrv installed at {} — starts when an SSH session forwards X11",
                x.install_path.as_deref().unwrap_or("?")
            ),
        )
    } else {
        (t.warning, "X off".to_string(), "VcXsrv not installed — required for X11 forwarding".to_string())
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(label).font(font(size::XS)).color(color));
        Icon::Monitor.show(ui, 11.0, color);
    })
    .response
    .on_hover_text(tip);
}
