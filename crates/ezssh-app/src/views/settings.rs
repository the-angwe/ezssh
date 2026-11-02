//! `settings-dialog.tsx`: five sections, every control writes through
//! (no Save button). Per-session values override the Terminal / Sessions
//! defaults here.

use std::collections::HashMap;

use egui::{Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use ezssh_term::palette::{self, Palette};

use crate::app::App;
use crate::font_presets;
use crate::form;
use crate::icons::Icon;
use crate::settings::Value;
use crate::style::{Theme, alpha, font, font_semibold, size, tokens};
use crate::widgets;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Terminal,
    Sessions,
    Updates,
    Security,
}

const SECTIONS: [(Section, &str, Icon); 5] = [
    (Section::Appearance, "Appearance", Icon::Palette),
    (Section::Terminal, "Terminal", Icon::SquareTerminal),
    (Section::Sessions, "Sessions", Icon::Layers),
    (Section::Updates, "Updates", Icon::RefreshCw),
    (Section::Security, "Security", Icon::ShieldCheck),
];

#[derive(Default)]
pub struct SettingsDialog {
    pub section: Section,
    /// Number fields being typed into; written on Enter / blur so an
    /// intermediate prefix ("1" on the way to "13") is never clamped.
    drafts: HashMap<&'static str, String>,
    /// "Custom…" picked in the font family select (a value that is not an
    /// installed font shows as custom without it).
    font_custom: bool,
}

/// A change made this frame.
enum Change {
    Set(&'static str, Value),
    Theme(Theme),
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.settings.take() else {
        return;
    };
    let t = tokens(ctx);
    let mut changes: Vec<Change> = Vec::new();
    let mut close = false;
    let size_w = 720.0_f32.min(ctx.content_rect().width() - 32.0);
    let size_h = 520.0_f32.min(ctx.content_rect().height() - 32.0);
    let frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(3))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(115) });
    let resp = egui::Modal::new(Id::unique("settings")).backdrop_color(Color32::from_black_alpha(128)).frame(frame).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(size_w, size_h), Sense::hover());
        // Header.
        let header = Rect::from_min_size(rect.min, vec2(size_w, 40.0));
        ui.painter().hline(header.x_range(), header.bottom(), Stroke::new(1.0, t.border));
        ui.scope_builder(egui::UiBuilder::new().max_rect(header.shrink2(vec2(16.0, 0.0))), |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new("Settings").font(font_semibold(size::BASE)).color(t.fg));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::X, 24.0, 14.0, false, "Close settings").clicked() {
                        close = true;
                    }
                    ui.label(RichText::new("Changes save automatically").font(font(size::XS)).color(t.muted));
                });
            });
        });
        // Navigation.
        let nav = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), Pos2::new(rect.left() + 176.0, rect.bottom()));
        ui.painter().vline(nav.right(), nav.y_range(), Stroke::new(1.0, t.border));
        let mut y = nav.top() + 8.0;
        for (section, label, icon) in SECTIONS {
            let row = Rect::from_min_size(Pos2::new(nav.left(), y), vec2(nav.width(), 32.0));
            let r = ui.interact(row, Id::unique(("settings-nav", label)), Sense::click());
            widgets::a11y_selected(&r, egui::Role::Tab, dlg.section == section, label);
            let on = dlg.section == section;
            if on {
                ui.painter().rect_filled(row, 0.0, t.bg);
                ui.painter().rect_filled(Rect::from_min_size(row.min, vec2(2.0, row.height())), 0.0, t.accent);
            } else if r.hovered() {
                ui.painter().rect_filled(row, 0.0, alpha(t.surface2, 0.4));
            }
            let color = if on || r.hovered() { t.fg } else { t.muted };
            icon.paint(ui, Rect::from_center_size(Pos2::new(row.left() + 23.0, row.center().y), vec2(14.0, 14.0)), 14.0, color);
            widgets::paint_text(ui, row.with_min_x(row.left() + 38.0), label, font(size::SM), color);
            if r.clicked() {
                dlg.section = section;
            }
            y += 32.0;
        }
        // Content.
        let content = Rect::from_min_max(Pos2::new(nav.right() + 1.0, header.bottom() + 1.0), rect.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| {
                    ui.set_width(content.width() - 40.0);
                    let mut s = Sec { ui, app, drafts: &mut dlg.drafts, font_custom: &mut dlg.font_custom, changes: &mut changes };
                    match dlg.section {
                        Section::Appearance => s.appearance(),
                        Section::Terminal => s.terminal(),
                        Section::Sessions => s.sessions(),
                        Section::Updates => s.updates(),
                        Section::Security => s.security(),
                    }
                });
            });
        });
    });
    if resp.should_close() {
        close = true;
    }
    for c in changes {
        match c {
            Change::Set(k, v) => app.set_setting(k, v),
            Change::Theme(theme) => app.set_theme(theme),
        }
    }
    if !close {
        app.dialogs.settings = Some(dlg);
    }
}

/// Section builder: rows read the current value from `app` and push
/// changes to apply after the frame.
struct Sec<'a, 'u> {
    ui: &'u mut Ui,
    app: &'a App,
    drafts: &'a mut HashMap<&'static str, String>,
    font_custom: &'a mut bool,
    changes: &'a mut Vec<Change>,
}

impl Sec<'_, '_> {
    fn appearance(&mut self) {
        self.group("Appearance", None, |s| {
            let theme = s.app.theme;
            s.row("Theme", Some("Window chrome only; pick the terminal colours below."), |s, ui| {
                let mut chosen = theme;
                egui::ComboBox::from_id_salt("theme").width(160.0).selected_text(if theme == Theme::Dark { "Dark" } else { "Light" }).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(&mut chosen, Theme::Dark, "Dark");
                        ui.selectable_value(&mut chosen, Theme::Light, "Light");
                    },
                );
                if chosen != theme {
                    s.changes.push(Change::Theme(chosen));
                }
            });
            s.enum_row(
                "appearance.tabLabel",
                "Tab label",
                Some("What each tab shows. Local shells always show their name."),
                &[("name", "Session name"), ("userhost", "user@host")],
            );
            s.palette_row();
        });
    }

    fn terminal(&mut self) {
        self.group("Defaults for new sessions", Some("Each session can override these in its own dialog."), |s| {
            s.int_row("terminal.scrollback", "Scrollback lines", Some("0 disables scrollback."));
            s.int_row("terminal.fontSize", "Font size", None);
            s.font_family_row("terminal.fontFamily", "Font family");
            s.enum_row("terminal.cursorStyle", "Cursor style", None, &[("block", "Block"), ("underline", "Underline"), ("bar", "Bar")]);
            s.bool_row("terminal.cursorBlink", "Cursor blink", Some("Applies to tabs opened after the change."));
        });
        self.group("Mouse and clipboard", None, |s| {
            s.bool_row("terminal.copyOnSelect", "Copy on select", Some("Selecting text copies it immediately."));
            s.enum_row(
                "terminal.rightClick",
                "Right-click",
                Some("With Paste selected, Ctrl + right-click still opens the menu."),
                &[("menu", "Open context menu"), ("paste", "Paste clipboard")],
            );
            s.bool_row(
                "terminal.warnMultilinePaste",
                "Warn before pasting multiple lines",
                Some("Catches a whole script landing in a shell by accident."),
            );
            s.bool_row(
                "terminal.osc52Clipboard",
                "Programs may write the clipboard (OSC 52)",
                Some("Lets Claude Code, neovim and tmux copy to your clipboard over SSH. Programs can never read it."),
            );
        });
        self.group("Bell", None, |s| {
            s.enum_row("terminal.bell", "On bell", None, &[("none", "Do nothing"), ("visual", "Flash the terminal"), ("sound", "Play a beep")]);
        });
    }

    fn sessions(&mut self) {
        self.group("Tabs", None, |s| {
            s.bool_row("sessions.confirmCloseLive", "Confirm before closing a connected tab", None);
            s.bool_row("sessions.confirmExit", "Confirm before quitting with connected tabs", None);
            s.bool_row("sessions.autoOpenSftp", "Open the file pane when a session connects", Some("SSH and WSL tabs."));
        });
        self.group("Defaults for new sessions", Some("Each session can override these in its own dialog."), |s| {
            s.int_row(
                "sessions.keepaliveSecs",
                "Keepalive interval (seconds)",
                Some("0 disables. Keeps idle connections alive through NAT and firewalls."),
            );
            s.int_row("sessions.connectTimeoutSecs", "Connect timeout (seconds)", None);
        });
    }

    fn updates(&mut self) {
        self.group("Updates", None, |s| {
            s.bool_row("updates.checkOnLaunch", "Check for updates on launch", None);
            s.int_row("updates.intervalDays", "Check at most every (days)", None);
            s.bool_row(
                "updates.preRelease",
                "Include pre-releases",
                Some("Track the newest build, including betas, to test versions before they ship publicly."),
            );
        });
    }

    fn security(&mut self) {
        self.group("Vault", None, |s| {
            s.int_row(
                "security.idleLockMinutes",
                "Lock the vault after inactivity (minutes)",
                Some("0 keeps it unlocked until you lock it. Open connections stay up; reopening tabs needs the master password."),
            );
            let t = tokens(s.ui.ctx());
            s.ui.add_space(6.0);
            s.ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                Icon::Monitor.show(ui, 13.0, t.muted);
                ui.label(RichText::new("Inactivity means no keyboard or mouse input anywhere in the ezSsh window.").font(font(size::XS)).color(t.muted));
            });
        });
    }

    // ------------------------------------------------------------ blocks

    fn group(&mut self, title: &str, hint: Option<&str>, body: impl FnOnce(&mut Self)) {
        let t = tokens(self.ui.ctx());
        self.ui.label(RichText::new(title.to_uppercase()).font(font_semibold(size::XS)).color(t.muted));
        if let Some(h) = hint {
            self.ui.label(RichText::new(h).font(font(size::XS)).color(t.muted));
        }
        self.ui.add_space(4.0);
        let top = self.ui.cursor().top();
        let x = self.ui.max_rect().x_range();
        self.ui.painter().hline(x, top, Stroke::new(1.0, t.border));
        self.ui.spacing_mut().item_spacing.y = 0.0;
        body(self);
        self.ui.spacing_mut().item_spacing.y = 4.0;
        self.ui.add_space(24.0);
    }

    /// Label + hint on the left, control on the right, divider below.
    fn row(&mut self, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Self, &mut Ui)) {
        let t = tokens(self.ui.ctx());
        let width = self.ui.available_width();
        let top = self.ui.cursor().min;
        let text_w = (width - 260.0).max(160.0);
        let text = self.ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(text_w);
            ui.add_space(10.0);
            ui.add(egui::Label::new(RichText::new(label).font(font(size::SM)).color(t.fg)).wrap());
            if let Some(h) = hint {
                ui.add_space(2.0);
                ui.add(egui::Label::new(RichText::new(h).font(font(size::XS)).color(t.muted)).wrap());
            }
            ui.add_space(10.0);
        });
        let text_h = text.response.rect.height();
        let h = text_h.max(44.0);
        if h > text_h {
            self.ui.add_space(h - text_h);
        }
        let row = Rect::from_min_size(top, vec2(width, h));
        let ctrl_rect = Rect::from_min_max(Pos2::new(row.right() - 250.0, row.top()), row.max);
        let mut child = self.ui.new_child(egui::UiBuilder::new().max_rect(ctrl_rect).layout(Layout::right_to_left(Align::Center)));
        control(self, &mut child);
        self.ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, t.border));
    }

    fn bool_row(&mut self, key: &'static str, label: &str, hint: Option<&str>) {
        let value = self.app.settings.bool(key);
        self.row(label, hint, |s, ui| {
            let mut v = value;
            let cb = ui.add(egui::Checkbox::without_text(&mut v));
            widgets::a11y_selected(&cb, egui::Role::CheckBox, v, label);
            if cb.on_hover_text(label).changed() {
                s.changes.push(Change::Set(key, Value::Bool(v)));
            }
        });
    }

    fn int_row(&mut self, key: &'static str, label: &str, hint: Option<&str>) {
        let value = self.app.settings.int(key);
        self.row(label, hint, |s, ui| {
            let draft = s.drafts.entry(key).or_insert_with(|| value.to_string());
            let t = tokens(ui.ctx());
            let resp = ui.add(
                egui::TextEdit::singleline(draft)
                    .desired_width(96.0)
                    .align(egui::Align2::RIGHT_CENTER)
                    .font(font(size::BASE))
                    .margin(Margin::symmetric(8, 6))
                    .background_color(t.surface2),
            );
            let commit = resp.lost_focus();
            if commit {
                match draft.trim().parse::<i64>() {
                    Ok(n) => s.changes.push(Change::Set(key, Value::Int(n))),
                    Err(_) => tracing::debug!(key, draft = %draft, "settings: not a number, reverted"),
                }
                // Show the stored (clamped) value next frame.
                s.drafts.remove(key);
            } else if !resp.has_focus() && draft.trim().parse::<i64>().ok() != Some(value) {
                // The store changed underneath (rollback): follow it.
                *draft = value.to_string();
            }
        });
    }

    /// The session dialog's family list: "(default)", the installed
    /// monospace fonts, and "Custom…", which adds a free-text box.
    fn font_family_row(&mut self, key: &'static str, label: &str) {
        const CUSTOM_FONT: &str = "\u{0}custom";
        let value = self.app.settings.str(key).to_string();
        let custom = *self.font_custom || (!value.is_empty() && !font_presets::is_preset(&value));
        self.row(label, Some("Installed monospace fonts."), |s, ui| {
            let mut options: Vec<(String, String)> = font_presets::choices().to_vec();
            options.push((CUSTOM_FONT.into(), "Custom…".into()));
            let mut choice = if custom { CUSTOM_FONT.to_string() } else { value.clone() };
            let select_w = if custom { 100.0 } else { 210.0 };
            if form::select(ui, Id::unique(("settings", key)), &mut choice, &options, Some(select_w)) {
                tracing::debug!(key, font = %choice, "settings: font family picked");
                if choice == CUSTOM_FONT {
                    *s.font_custom = true;
                } else {
                    *s.font_custom = false;
                    if choice != value {
                        s.changes.push(Change::Set(key, Value::Str(choice.clone())));
                    }
                }
            }
            if choice == CUSTOM_FONT {
                let mut v = value.clone();
                let resp = widgets::text_input(ui, &mut v, "e.g. Iosevka", 140.0, false);
                form::labelled(ui, &resp, "Custom font family");
                if resp.on_hover_text("Custom font family").changed() {
                    s.changes.push(Change::Set(key, Value::Str(v)));
                }
            }
        });
    }

    fn enum_row(&mut self, key: &'static str, label: &str, hint: Option<&str>, options: &[(&'static str, &'static str)]) {
        let value = self.app.settings.str(key).to_string();
        let shown = options.iter().find(|(v, _)| *v == value).map_or(value.as_str(), |(_, l)| l).to_string();
        self.row(label, hint, |s, ui| {
            egui::ComboBox::from_id_salt(key).width(176.0).selected_text(shown).show_ui(ui, |ui| {
                for (v, text) in options {
                    if ui.selectable_label(value == *v, *text).clicked() && value != *v {
                        s.changes.push(Change::Set(key, Value::Str((*v).to_string())));
                    }
                }
            });
        });
    }

    fn palette_row(&mut self) {
        let t = tokens(self.ui.ctx());
        let value = self.app.settings.str("terminal.palette").to_string();
        let current = palette::by_id(&value);
        self.row("Terminal colours", Some("ANSI palette for every terminal tab. Changes apply live."), |s, ui| {
            egui::ComboBox::from_id_salt("terminal.palette").width(176.0).selected_text(current.label).show_ui(ui, |ui| {
                for (heading, light) in [("Dark", false), ("Light", true)] {
                    ui.label(RichText::new(heading).font(font_semibold(size::XS)).color(t.muted));
                    for p in palette::PALETTES.iter().filter(|p| p.light == light) {
                        if ui.selectable_label(p.id == value, p.label).clicked() && p.id != value {
                            s.changes.push(Change::Set("terminal.palette", Value::Str(p.id.to_string())));
                        }
                    }
                }
            });
        });
        self.ui.add_space(8.0);
        swatch(self.ui, current, self.app.mono.clone());
        self.ui.add_space(10.0);
    }
}

fn rgb(c: ezssh_term::palette::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Mock prompt line and the 16 ANSI colours on the palette background.
fn swatch(ui: &mut Ui, p: &Palette, mono: egui::FontFamily) {
    let t = tokens(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 52.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, rgb(p.background));
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let a = |i: usize| rgb(p.ansi[i]);
    let fg = rgb(p.foreground);
    let parts: [(&str, Color32); 11] = [
        ("user@host", a(2)),
        (":", fg),
        ("~/src", a(4)),
        ("$ ", fg),
        ("ls", a(3)),
        (" -la ", fg),
        ("docs/", a(6)),
        (" ", fg),
        ("*.md", a(5)),
        (" ", fg),
        ("error", a(1)),
    ];
    let mut job = egui::text::LayoutJob::default();
    let font_id = egui::FontId::new(size::XS, mono);
    for (text, color) in parts {
        job.append(text, 0.0, egui::TextFormat::simple(font_id.clone(), color));
    }
    let galley = ui.painter().layout_job(job);
    let text_pos = Pos2::new(rect.left() + 12.0, rect.top() + 8.0);
    let cursor_x = text_pos.x + galley.size().x + 2.0;
    let line_h = galley.size().y;
    ui.painter().galley(text_pos, galley, fg);
    ui.painter().rect_filled(Rect::from_min_size(Pos2::new(cursor_x, text_pos.y + 1.0), vec2(line_h * 0.55, line_h - 2.0)), 0.0, rgb(p.cursor));
    let gap = 4.0;
    let n = p.ansi.len() as f32;
    let cw = (rect.width() - 24.0 - gap * (n - 1.0)) / n;
    for (i, c) in p.ansi.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.left() + 12.0 + i as f32 * (cw + gap), rect.bottom() - 20.0), vec2(cw, 14.0));
        ui.painter().rect_filled(r, CornerRadius::same(2), rgb(*c));
        ui.interact(r, Id::unique(("ansi", i)), Sense::hover()).on_hover_text(format!("ANSI {i}: #{:02x}{:02x}{:02x}", c.r, c.g, c.b));
    }
}
