//! The small generic dialogs: confirm, prompt, move to folder, tile grid
//! and host key.

use std::collections::HashSet;

use egui::{Id, Key, RichText, Sense, Ui, vec2};
use ezssh_core::db::folders::Folder;

use crate::conn::HostKeyPrompt;
use crate::icons::Icon;
use crate::layout::TileGrid;
use crate::style::{alpha, font, font_semibold, radius, size, tokens};
use crate::widgets::{self, dialog, dialog_heading};

/// Ask once and get Some(true / false) when answered.
pub struct Confirm<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub confirm_text: &'a str,
    pub danger: bool,
}

/// A confirmation. The primary button has focus, so Enter confirms;
/// Escape and the backdrop cancel.
pub fn confirm(ctx: &egui::Context, id: Id, c: Confirm<'_>) -> Option<bool> {
    let t = tokens(ctx);
    let mut answer = None;
    let first = first_frame(ctx, id);
    let out = dialog(
        ctx,
        id,
        420.0,
        |ui| {
            let (icon, color) = if c.danger { (Icon::AlertTriangle, t.danger) } else { (Icon::Info, t.accent) };
            dialog_heading(ui, icon, color, c.title, c.body);
        },
        Some(&mut |ui: &mut Ui| {
            let ok = if c.danger { widgets::Button::danger(c.confirm_text) } else { widgets::Button::primary(c.confirm_text) }.show(ui);
            if first {
                ok.request_focus();
            }
            if ok.clicked() {
                answer = Some(true);
            }
            if widgets::Button::secondary("Cancel").show(ui).clicked() {
                answer = Some(false);
            }
        }),
    );
    if out.dismissed() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// True on the first frame a dialog with `id` is shown (for initial
/// focus). Keyed on the frame counter: a dialog abandoned without an
/// answer (the vault locked over it) must not skip the focus next time.
fn first_frame(ctx: &egui::Context, id: Id) -> bool {
    let key = id.with("shown");
    let now = ctx.cumulative_frame_nr();
    let last = ctx.data(|d| d.get_temp::<u64>(key));
    ctx.data_mut(|d| d.insert_temp(key, now));
    // Repeat passes of one frame share its number.
    !last.is_some_and(|l| l + 1 >= now)
}

// ---------------------------------------------------------------- prompt

pub struct PromptState {
    pub title: String,
    pub label: String,
    pub value: String,
    pub placeholder: String,
    pub confirm_text: String,
    error: Option<String>,
    focused: bool,
}

pub enum PromptAnswer {
    Cancel,
    Submit(String),
}

impl PromptState {
    pub fn new(title: impl Into<String>, label: &str, value: &str, placeholder: &str, confirm_text: &str) -> Self {
        Self {
            title: title.into(),
            label: label.into(),
            value: value.into(),
            placeholder: placeholder.into(),
            confirm_text: confirm_text.into(),
            error: None,
            focused: false,
        }
    }

    /// Show `e` under the input and focus it again (a submit failed).
    pub fn set_error(&mut self, e: impl Into<String>) {
        self.error = Some(e.into());
        self.focused = false;
    }

    /// Input focused with its text selected, Enter
    /// submits, an empty value is refused in place.
    pub fn show(&mut self, ctx: &egui::Context, id: Id) -> Option<PromptAnswer> {
        let t = tokens(ctx);
        let confirm_text = self.confirm_text.clone();
        let submit = std::cell::Cell::new(false);
        let mut cancel = false;
        let out = dialog(
            ctx,
            id,
            440.0,
            |ui| {
                ui.label(RichText::new(&self.title).font(font_semibold(size::MD)).color(t.fg));
                ui.add_space(14.0);
                if !self.label.is_empty() {
                    ui.label(RichText::new(&self.label).font(font(size::SM)).color(t.fg2));
                    ui.add_space(6.0);
                }
                let w = ui.available_width();
                let resp = widgets::text_input(ui, &mut self.value, &self.placeholder, w, false);
                crate::form::labelled(ui, &resp, if self.label.is_empty() { "Value" } else { &self.label });
                if !self.focused {
                    self.focused = true;
                    resp.request_focus();
                    select_all(ui, &resp, &self.value);
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    submit.set(true);
                }
                if let Some(e) = &self.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(e).font(font(size::SM)).color(t.danger));
                }
            },
            Some(&mut |ui: &mut Ui| {
                if widgets::Button::primary(&confirm_text).show(ui).clicked() {
                    submit.set(true);
                }
                if widgets::Button::secondary("Cancel").show(ui).clicked() {
                    cancel = true;
                }
            }),
        );
        if cancel || out.dismissed() {
            return Some(PromptAnswer::Cancel);
        }
        if submit.get() {
            let v = self.value.trim();
            if v.is_empty() {
                self.error = Some(format!("{} is required", if self.label.is_empty() { "Value" } else { &self.label }));
                self.focused = false;
                return None;
            }
            return Some(PromptAnswer::Submit(v.to_string()));
        }
        None
    }
}

fn select_all(ui: &Ui, resp: &egui::Response, value: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        let range = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(value.chars().count()));
        state.cursor.set_char_range(Some(range));
        state.store(ui.ctx(), resp.id);
    }
}

// ---------------------------------------------------------------- move to folder

pub struct MoveState {
    pub title: String,
    /// Folder the item lives in now (not selectable). `None` = root.
    current: Option<i64>,
    excluded: HashSet<i64>,
    /// `Some(None)` = root picked.
    pick: Option<Option<i64>>,
    expanded: HashSet<i64>,
}

impl MoveState {
    pub fn new(title: String, folders: &[Folder], current: Option<i64>, excluded: HashSet<i64>) -> Self {
        Self { title, current, excluded, pick: None, expanded: folders.iter().map(|f| f.id).collect() }
    }

    /// `Some(Some(dest))` on Move, `Some(None)` on cancel. `dest` is the
    /// folder id, `None` meaning the root.
    pub fn show(&mut self, ctx: &egui::Context, id: Id, folders: &[Folder]) -> Option<Option<Option<i64>>> {
        let t = tokens(ctx);
        let mut done: Option<Option<Option<i64>>> = None;
        let pick = self.pick;
        let out = dialog(
            ctx,
            id,
            440.0,
            |ui| {
                ui.label(RichText::new(&self.title).font(font_semibold(size::MD)).color(t.fg));
                ui.add_space(14.0);
                let frame = egui::Frame::new()
                    .fill(t.bg)
                    .corner_radius(radius::LG as u8)
                    .inner_margin(egui::Margin::symmetric(0, 4));
                frame.show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.set_width(ui.available_width());
                        let root_disabled = self.current.is_none();
                        if folder_row(ui, "(Root)", root_disabled, self.pick == Some(None), true) {
                            self.pick = Some(None);
                        }
                        // Depth-first over the folder tree.
                        fn walk(s: &mut MoveState, ui: &mut Ui, folders: &[Folder], parent: Option<i64>, depth: usize) {
                            let mut kids: Vec<&Folder> = folders.iter().filter(|f| f.parent_id == parent).collect();
                            kids.sort_by_key(|f| (f.sort, f.id));
                            for f in kids {
                                let has_kids = folders.iter().any(|c| c.parent_id == Some(f.id));
                                let open = s.expanded.contains(&f.id);
                                let disabled = s.current == Some(f.id) || s.excluded.contains(&f.id);
                                let (picked, toggled) =
                                    folder_row_tree(ui, depth, &f.name, has_kids, open, disabled, s.pick == Some(Some(f.id)));
                                if picked {
                                    s.pick = Some(Some(f.id));
                                }
                                if toggled && !s.expanded.remove(&f.id) {
                                    s.expanded.insert(f.id);
                                }
                                if open {
                                    walk(s, ui, folders, Some(f.id), depth + 1);
                                }
                            }
                        }
                        walk(self, ui, folders, None, 0);
                    });
                });
            },
            Some(&mut |ui: &mut Ui| {
                if widgets::Button::primary("Move").enabled(pick.is_some()).show(ui).clicked()
                    && let Some(p) = pick
                {
                    done = Some(Some(p));
                }
                if widgets::Button::secondary("Cancel").show(ui).clicked() {
                    done = Some(None);
                }
            }),
        );
        if out.dismissed() && done.is_none() {
            done = Some(None);
        }
        done
    }
}

/// A 28 px picker row. Returns true when picked.
fn folder_row(ui: &mut Ui, name: &str, disabled: bool, selected: bool, root: bool) -> bool {
    let t = tokens(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), if disabled { Sense::hover() } else { Sense::click() });
    paint_pick_row(ui, &resp, rect, selected, disabled);
    let x = rect.left() + 6.0 + 12.0 + 8.0;
    let icon_rect = egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0));
    Icon::Folder.paint(ui, icon_rect, 16.0, if root { alpha(t.muted, 0.7) } else { t.muted });
    let color = if disabled { alpha(t.muted, 0.6) } else { t.fg };
    widgets::paint_text(ui, egui::Rect::from_min_max(egui::pos2(x + 24.0, rect.top()), egui::pos2(rect.right() - 8.0, rect.bottom())), name, font(size::BASE), color);
    let resp = if disabled { resp.on_hover_text("Already here") } else { resp };
    resp.clicked()
}

/// A folder row with a disclosure chevron. Returns (picked, toggled).
fn folder_row_tree(ui: &mut Ui, depth: usize, name: &str, has_kids: bool, open: bool, disabled: bool, selected: bool) -> (bool, bool) {
    let t = tokens(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), if disabled { Sense::hover() } else { Sense::click() });
    paint_pick_row(ui, &resp, rect, selected, disabled);
    let x0 = rect.left() + 6.0 + (depth + 1) as f32 * 14.0;
    let chevron = egui::Rect::from_center_size(egui::pos2(x0 + 6.0, rect.center().y), vec2(14.0, 14.0));
    let mut toggled = false;
    if has_kids {
        let c = ui.interact(chevron, resp.id.with("chevron"), Sense::click());
        (if open { Icon::ChevronDown } else { Icon::ChevronRight }).paint(ui, chevron, 12.0, alpha(t.muted, 0.9));
        toggled = c.clicked();
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(x0 + 12.0 + 8.0 + 8.0, rect.center().y), vec2(16.0, 16.0));
    let icon = if open { Icon::FolderOpen } else { Icon::Folder };
    icon.paint(ui, icon_rect, 16.0, t.muted);
    let color = if disabled { alpha(t.muted, 0.6) } else { t.fg2 };
    widgets::paint_text(
        ui,
        egui::Rect::from_min_max(egui::pos2(icon_rect.right() + 8.0, rect.top()), egui::pos2(rect.right() - 8.0, rect.bottom())),
        name,
        font(size::BASE),
        color,
    );
    let picked = !toggled && resp.clicked();
    if disabled {
        resp.on_hover_text("Already here");
    }
    (picked, toggled)
}

fn paint_pick_row(ui: &Ui, resp: &egui::Response, rect: egui::Rect, selected: bool, disabled: bool) {
    widgets::row_fill(ui, rect, !disabled && resp.hovered(), selected);
}

// ---------------------------------------------------------------- tile grid

pub struct TileGridDraft {
    pub rows: i64,
    pub cols: i64,
    focused: bool,
}

impl TileGridDraft {
    pub fn new(grid: TileGrid) -> Self {
        Self { rows: grid.rows as i64, cols: grid.cols as i64, focused: false }
    }

    /// `Some(Some(grid))` on Apply, `Some(None)` on cancel.
    pub fn show(&mut self, ctx: &egui::Context, id: Id) -> Option<Option<TileGrid>> {
        let t = tokens(ctx);
        let apply = std::cell::Cell::new(false);
        let mut cancel = false;
        let first = !self.focused;
        self.focused = true;
        let out = dialog(
            ctx,
            id,
            360.0,
            |ui| {
                dialog_heading(ui, Icon::Grid3x3, t.accent, "Tile grid", "Pick the rows × columns layout (1–8 each).");
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.add_space(50.0);
                    for (label, v) in [("Rows", &mut self.rows), ("Columns", &mut self.cols)] {
                        ui.vertical(|ui| {
                            ui.set_width(100.0);
                            crate::form::field(ui, label, None, |ui| super::session_dialog::number_field(ui, label, v, 100.0));
                        });
                    }
                });
            },
            Some(&mut |ui: &mut Ui| {
                let ok = widgets::Button::primary("Apply").show(ui);
                if first {
                    ok.request_focus();
                }
                if ok.clicked() {
                    apply.set(true);
                }
                if widgets::Button::secondary("Cancel").show(ui).clicked() {
                    cancel = true;
                }
            }),
        );
        if apply.get() {
            Some(Some(TileGrid::clamped(self.rows, self.cols)))
        } else if cancel || out.dismissed() {
            Some(None)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- host key

/// An unknown or changed host key. Some(true) = trust and connect, Some(false) =
/// cancel. No backdrop dismissal: the choice matters.
pub fn host_key(ctx: &egui::Context, id: Id, host: &str, port: i64, p: &HostKeyPrompt) -> Option<bool> {
    let t = tokens(ctx);
    let mut answer = None;
    let (icon, icon_color, title, body) = if p.mismatch {
        (
            Icon::ShieldX,
            t.danger,
            "Host key changed!",
            format!(
                "The host key for {host}:{port} differs from the stored record. This may indicate interception — do NOT continue unless you know why."
            ),
        )
    } else {
        (Icon::ShieldAlert, t.warning, "Trust this host?", format!("No previous record for {host}:{port}. Verify the fingerprint out-of-band before trusting."))
    };
    let out = dialog(
        ctx,
        id,
        520.0,
        |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                widgets::icon_tile(ui, r, icon, icon_color, 18.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.add_space(1.0);
                    ui.label(RichText::new(title).font(font_semibold(size::MD)).color(t.fg));
                    ui.add(egui::Label::new(RichText::new(body).font(font(size::BASE)).color(t.fg2)).wrap());
                    ui.add_space(6.0);
                    let mono = |s: &str| RichText::new(s).font(egui::FontId::monospace(size::SM)).color(t.fg);
                    let caption = |s: &str| RichText::new(s.to_uppercase()).font(font_semibold(size::XS)).color(t.muted);
                    if let Some(expected) = &p.expected {
                        ui.label(caption("Expected SHA256"));
                        ui.add(egui::Label::new(mono(expected)).wrap());
                    }
                    ui.label(caption(if p.expected.is_some() { "Actual SHA256" } else { "SHA256" }));
                    ui.add(egui::Label::new(mono(&p.fingerprint)).wrap());
                });
            });
        },
        Some(&mut |ui: &mut Ui| {
            let ok = if p.mismatch { widgets::Button::danger("Replace and connect") } else { widgets::Button::primary("Trust and connect") }.show(ui);
            if ok.clicked() {
                answer = Some(true);
            }
            if widgets::Button::secondary("Cancel").show(ui).clicked() {
                answer = Some(false);
            }
        }),
    );
    if answer.is_none() && out.dismissed() {
        answer = Some(false);
    }
    answer
}
