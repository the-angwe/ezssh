//! The settings dialog: five sections, every control writes through
//! (no Save button). Per-session values override the Terminal / Sessions
//! defaults here.

use std::collections::HashMap;

use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke,
    Ui, vec2,
};
use ezssh_term::palette::{self, Palette};

use crate::app::App;
use crate::font_presets;
use crate::form;
use crate::icons::Icon;
use crate::settings::Value;
use crate::style::{Theme, font, font_semibold, radius, size, tokens};
use crate::widgets::{self, Segment};

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

const W: f32 = 760.0;
const H: f32 = 560.0;
const NAV_W: f32 = 176.0;
/// Width of the control column on the right of a settings row.
const CONTROL_W: f32 = 260.0;

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
    let size_w = W.min(ctx.content_rect().width() - 32.0);
    let size_h = H.min(ctx.content_rect().height() - 32.0);
    let id = Id::unique("settings");
    let resp = egui::Modal::new(id)
        .backdrop_color(Color32::from_black_alpha(128))
        .frame(widgets::dialog_frame(t))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(size_w, size_h), Sense::hover());
            let header = Rect::from_min_size(rect.min, vec2(size_w, widgets::DIALOG_HEAD_H));
            let header_close = widgets::dialog_header(
                ui,
                id,
                header,
                None,
                t.accent,
                "Settings",
                Some("Changes save automatically"),
                false,
                false,
            );
            // Navigation.
            let nav = Rect::from_min_max(
                Pos2::new(rect.left(), header.bottom()),
                Pos2::new(rect.left() + NAV_W, rect.bottom()),
            );
            ui.painter()
                .vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, t.border));
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(nav.shrink2(vec2(8.0, 12.0))),
                |ui| {
                    widgets::nav_rail(ui, NAV_W - 16.0, &SECTIONS, &mut dlg.section);
                },
            );
            // Content.
            let content = Rect::from_min_max(Pos2::new(nav.right(), header.bottom()), rect.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin {
                                left: 24,
                                right: 24,
                                top: 20,
                                bottom: 24,
                            })
                            .show(ui, |ui| {
                                ui.set_width(content.width() - 48.0);
                                let mut s = Sec {
                                    ui,
                                    app,
                                    drafts: &mut dlg.drafts,
                                    font_custom: &mut dlg.font_custom,
                                    changes: &mut changes,
                                };
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
            if header_close.show(ui) {
                close = true;
            }
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
            s.row(
                "Theme",
                Some("Window chrome only; pick the terminal colours below."),
                |s, ui| {
                    let mut chosen = theme;
                    let items = [
                        Segment::text(Theme::Dark, "Dark").with_icon(Icon::Moon),
                        Segment::text(Theme::Light, "Light").with_icon(Icon::Sun),
                    ];
                    if widgets::segmented(ui, Id::unique("settings-theme"), &mut chosen, &items) {
                        s.changes.push(Change::Theme(chosen));
                    }
                },
            );
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
        self.group(
            "Defaults for new sessions",
            Some("Each session can override these in its own dialog."),
            |s| {
                s.int_row(
                    "terminal.scrollback",
                    "Scrollback lines",
                    Some("0 disables scrollback."),
                );
                s.int_row("terminal.fontSize", "Font size", None);
                s.font_family_row("terminal.fontFamily", "Font family");
                s.enum_row(
                    "terminal.cursorStyle",
                    "Cursor style",
                    None,
                    &[
                        ("block", "Block"),
                        ("underline", "Underline"),
                        ("bar", "Bar"),
                    ],
                );
                s.bool_row(
                    "terminal.cursorBlink",
                    "Cursor blink",
                    Some("Applies to tabs opened after the change."),
                );
            },
        );
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
            s.enum_row(
                "terminal.bell",
                "On bell",
                None,
                &[
                    ("none", "Do nothing"),
                    ("visual", "Flash the terminal"),
                    ("sound", "Play a beep"),
                ],
            );
        });
    }

    fn sessions(&mut self) {
        self.group("Tabs", None, |s| {
            s.bool_row(
                "sessions.confirmCloseLive",
                "Confirm before closing a connected tab",
                None,
            );
            s.bool_row(
                "sessions.confirmExit",
                "Confirm before quitting with connected tabs",
                None,
            );
            s.bool_row(
                "sessions.collapseSidebarOnOpen",
                "Hide the sessions sidebar when a tab opens",
                Some("Gives the terminal the room."),
            );
            s.bool_row(
                "sessions.autoOpenSftp",
                "Open the file pane when a session connects",
                Some("SSH and WSL tabs."),
            );
        });
        self.group(
            "Defaults for new sessions",
            Some("Each session can override these in its own dialog."),
            |s| {
                s.int_row(
                    "sessions.keepaliveSecs",
                    "Keepalive interval (seconds)",
                    Some("0 disables. Keeps idle connections alive through NAT and firewalls."),
                );
                s.int_row(
                    "sessions.connectTimeoutSecs",
                    "Connect timeout (seconds)",
                    None,
                );
            },
        );
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
            s.ui.add_space(12.0);
            s.ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                Icon::Monitor.show(ui, 14.0, t.muted);
                ui.label(RichText::new("Inactivity means no keyboard or mouse input anywhere in the ezSsh window.").font(font(size::SM)).color(t.muted));
            });
        });
    }

    // ------------------------------------------------------------ blocks

    /// A titled group of rows; groups are separated by a hairline.
    fn group(&mut self, title: &str, hint: Option<&str>, body: impl FnOnce(&mut Self)) {
        let t = tokens(self.ui.ctx());
        let seq = self.ui.scope_id().with("settings-group-pass");
        let pass = self.ui.ctx().cumulative_pass_nr();
        let first = self.ui.ctx().data(|d| d.get_temp::<u64>(seq)) != Some(pass);
        self.ui.ctx().data_mut(|d| d.insert_temp(seq, pass));
        self.ui.spacing_mut().item_spacing.y = 0.0;
        if !first {
            self.ui.add_space(20.0);
            let r = self.ui.available_rect_before_wrap();
            self.ui
                .painter()
                .hline(r.x_range(), r.top(), Stroke::new(1.0, t.border));
            self.ui.add_space(20.0);
        }
        self.ui.label(
            RichText::new(title)
                .font(font_semibold(size::MD))
                .color(t.fg),
        );
        if let Some(h) = hint {
            self.ui.add_space(2.0);
            self.ui
                .label(RichText::new(h).font(font(size::SM)).color(t.muted));
        }
        self.ui.add_space(6.0);
        body(self);
    }

    /// Label + hint on the left, control on the right, hairline below.
    fn row(&mut self, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Self, &mut Ui)) {
        let t = tokens(self.ui.ctx());
        let width = self.ui.available_width();
        let top = self.ui.cursor().min;
        let text_w = (width - CONTROL_W - 16.0).max(160.0);
        let text = self.ui.allocate_ui_with_layout(
            vec2(text_w, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(text_w);
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(RichText::new(label).font(font(size::BASE)).color(t.fg))
                        .wrap(),
                );
                if let Some(h) = hint {
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(RichText::new(h).font(font(size::SM)).color(t.muted))
                            .wrap(),
                    );
                }
                ui.add_space(10.0);
            },
        );
        let text_h = text.response.rect.height();
        let h = text_h.max(48.0);
        if h > text_h {
            self.ui.add_space(h - text_h);
        }
        let row = Rect::from_min_size(top, vec2(width, h));
        let ctrl_rect = Rect::from_min_max(Pos2::new(row.right() - CONTROL_W, row.top()), row.max);
        let mut child = self.ui.new_child(
            egui::UiBuilder::new()
                .max_rect(ctrl_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        widgets::trailing(&mut child, |ui| control(self, ui));
        self.ui.painter().hline(
            row.x_range(),
            row.bottom() - 0.5,
            Stroke::new(1.0, t.border),
        );
    }

    fn bool_row(&mut self, key: &'static str, label: &str, hint: Option<&str>) {
        let value = self.app.settings.bool(key);
        self.row(label, hint, |s, ui| {
            let mut v = value;
            if widgets::switch(ui, &mut v, label)
                .on_hover_text(label)
                .changed()
            {
                s.changes.push(Change::Set(key, Value::Bool(v)));
            }
        });
    }

    fn int_row(&mut self, key: &'static str, label: &str, hint: Option<&str>) {
        let value = self.app.settings.int(key);
        self.row(label, hint, |s, ui| {
            let draft = s.drafts.entry(key).or_insert_with(|| value.to_string());
            let out = form::number(ui, draft, label, 112.0);
            let resp = out.resp;
            if out.step != 0 {
                // Step from what is typed when it parses, else the stored value.
                let base = draft.trim().parse::<i64>().unwrap_or(value);
                s.changes
                    .push(Change::Set(key, Value::Int(base + out.step)));
                s.drafts.remove(key);
                return;
            }
            let commit = resp.lost_focus();
            if commit {
                match draft.trim().parse::<i64>() {
                    Ok(n) => s.changes.push(Change::Set(key, Value::Int(n))),
                    Err(_) => {
                        tracing::debug!(key, draft = %draft, "settings: not a number, reverted")
                    }
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
        const CUSTOM_FONT: &str = font_presets::CUSTOM_FONT;
        let value = self.app.settings.str(key).to_string();
        let custom = *self.font_custom || (!value.is_empty() && !font_presets::is_preset(&value));
        self.row(label, Some("Installed monospace fonts."), |s, ui| {
            let options = font_presets::choices_with_custom();
            let mut choice = if custom {
                CUSTOM_FONT.to_string()
            } else {
                value.clone()
            };
            let select_w = if custom { 108.0 } else { CONTROL_W };
            if form::select(
                ui,
                Id::unique(("settings", key)),
                &mut choice,
                options,
                Some(select_w),
            ) {
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
                let resp = widgets::text_input(
                    ui,
                    &mut v,
                    "e.g. Iosevka",
                    CONTROL_W - select_w - 8.0,
                    false,
                );
                form::labelled(ui, &resp, "Custom font family");
                if resp.on_hover_text("Custom font family").changed() {
                    s.changes.push(Change::Set(key, Value::Str(v)));
                }
            }
        });
    }

    /// A choice: a segmented control when the labels fit the control
    /// column, else a select.
    fn enum_row(
        &mut self,
        key: &'static str,
        label: &str,
        hint: Option<&str>,
        options: &[(&'static str, &'static str)],
    ) {
        let value = self.app.settings.str(key).to_string();
        let t = tokens(self.ui.ctx());
        let total: f32 = options
            .iter()
            .map(|(_, l)| {
                self.ui
                    .painter()
                    .layout_no_wrap(l.to_string(), font(size::SM), t.fg)
                    .size()
                    .x
                    + 20.0
                    + 2.0
            })
            .sum::<f32>()
            + 4.0;
        let segmented = total <= CONTROL_W;
        self.row(label, hint, |s, ui| {
            if segmented {
                let items: Vec<Segment<'_, &str>> =
                    options.iter().map(|(v, l)| Segment::text(*v, l)).collect();
                let mut chosen = value.as_str();
                if widgets::segmented(ui, Id::unique(("settings-seg", key)), &mut chosen, &items) {
                    s.changes
                        .push(Change::Set(key, Value::Str(chosen.to_string())));
                }
            } else {
                let opts: Vec<(String, String)> = options
                    .iter()
                    .map(|(v, l)| (v.to_string(), l.to_string()))
                    .collect();
                let mut chosen = value.clone();
                if form::select(
                    ui,
                    Id::unique(("settings", key)),
                    &mut chosen,
                    &opts,
                    Some(CONTROL_W),
                ) {
                    s.changes.push(Change::Set(key, Value::Str(chosen)));
                }
            }
        });
    }

    fn palette_row(&mut self) {
        let t = tokens(self.ui.ctx());
        let value = self.app.settings.str("terminal.palette").to_string();
        let current = palette::by_id(&value);
        self.row(
            "Terminal colours",
            Some("ANSI palette for every terminal tab. Changes apply live."),
            |s, ui| {
                ui.scope(|ui| {
                    let v = ui.visuals_mut();
                    v.widgets.inactive.weak_bg_fill = t.surface2;
                    v.widgets.hovered.weak_bg_fill = t.surface3;
                    ui.spacing_mut().button_padding = vec2(10.0, 8.0);
                    egui::ComboBox::from_id_salt("terminal.palette")
                        .width(CONTROL_W)
                        .selected_text(RichText::new(current.label).font(font(size::BASE)))
                        .show_ui(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (heading, light) in [("Dark", false), ("Light", true)] {
                                ui.label(
                                    RichText::new(heading)
                                        .font(font_semibold(size::XS))
                                        .color(t.muted),
                                );
                                for p in palette::PALETTES.iter().filter(|p| p.light == light) {
                                    if ui
                                        .selectable_label(
                                            p.id == value,
                                            RichText::new(p.label).font(font(size::BASE)),
                                        )
                                        .clicked()
                                        && p.id != value
                                    {
                                        s.changes.push(Change::Set(
                                            "terminal.palette",
                                            Value::Str(p.id.to_string()),
                                        ));
                                    }
                                }
                            }
                        });
                });
            },
        );
        self.ui.add_space(12.0);
        swatch(self.ui, current, self.app.mono.clone());
        self.ui.add_space(4.0);
    }
}

fn rgb(c: ezssh_term::palette::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Mock prompt line and the 16 ANSI colours on the palette background.
fn swatch(ui: &mut Ui, p: &Palette, mono: egui::FontFamily) {
    let t = tokens(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 60.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, radius::LG, rgb(p.background));
    ui.painter().rect_stroke(
        rect,
        radius::LG,
        Stroke::new(1.0, t.border_strong),
        egui::StrokeKind::Inside,
    );
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
    let font_id = egui::FontId::new(size::SM, mono);
    for (text, color) in parts {
        job.append(text, 0.0, egui::TextFormat::simple(font_id.clone(), color));
    }
    let galley = ui.painter().layout_job(job);
    let text_pos = Pos2::new(rect.left() + 14.0, rect.top() + 10.0);
    let cursor_x = text_pos.x + galley.size().x + 2.0;
    let line_h = galley.size().y;
    ui.painter().galley(text_pos, galley, fg);
    ui.painter().rect_filled(
        Rect::from_min_size(
            Pos2::new(cursor_x, text_pos.y + 1.0),
            vec2(line_h * 0.55, line_h - 2.0),
        ),
        0.0,
        rgb(p.cursor),
    );
    let gap = 4.0;
    let n = p.ansi.len() as f32;
    let cw = (rect.width() - 28.0 - gap * (n - 1.0)) / n;
    for (i, c) in p.ansi.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(
                rect.left() + 14.0 + i as f32 * (cw + gap),
                rect.bottom() - 24.0,
            ),
            vec2(cw, 14.0),
        );
        ui.painter()
            .rect_filled(r, CornerRadius::same(radius::SM as u8), rgb(*c));
        ui.interact(r, Id::unique(("ansi", i)), Sense::hover())
            .on_hover_text(format!("ANSI {i}: #{:02x}{:02x}{:02x}", c.r, c.g, c.b));
    }
}
