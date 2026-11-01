//! `sessions-sidebar.tsx`: the folder / session tree with drag and drop,
//! context menus, and the small folder / session dialogs. The session
//! editor and the root menu's dialogs live in their own modules.

use std::collections::HashSet;

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use ezssh_core::api;
use ezssh_core::db::folders::Folder;
use ezssh_core::db::sessions::Session;

use crate::app::App;
use crate::core::Core;
use crate::icons::Icon;
use crate::style::{ROW_H, alpha, font, font_semibold, size, tokens};
use crate::tabs::session_summary;
use crate::views::dialogs::{self, MoveState, PromptAnswer, PromptState};
use crate::widgets::{self, Menu, MenuEntry, MenuResult};

// ---------------------------------------------------------------- drop rules

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Session,
    Folder,
}

/// Where a drag in progress will land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropSlot {
    IntoRoot,
    IntoFolder(i64),
    BeforeSession(i64),
    AfterSession(i64),
    BeforeFolder(i64),
    AfterFolder(i64),
}

/// Slot for the pointer at `ratio` (0 = row top, 1 = bottom) of a row.
/// Sessions split 50/50. Folder rows split 25/50/25 for a folder drag
/// (nest or reorder from the same row) but are wholly "into" for a session
/// drag (the narrow middle band made that feel broken, #72).
pub fn slot_for_row(ratio: f32, row: RowKind, id: i64, source: RowKind) -> DropSlot {
    match row {
        RowKind::Session => {
            if ratio < 0.5 {
                DropSlot::BeforeSession(id)
            } else {
                DropSlot::AfterSession(id)
            }
        }
        RowKind::Folder if source == RowKind::Session => DropSlot::IntoFolder(id),
        RowKind::Folder if ratio < 0.25 => DropSlot::BeforeFolder(id),
        RowKind::Folder if ratio > 0.75 => DropSlot::AfterFolder(id),
        RowKind::Folder => DropSlot::IntoFolder(id),
    }
}

/// A row cannot be dropped onto itself.
pub fn slot_rejected(slot: DropSlot, source: (RowKind, i64)) -> bool {
    match (source.0, slot) {
        (RowKind::Folder, DropSlot::IntoFolder(f) | DropSlot::BeforeFolder(f) | DropSlot::AfterFolder(f)) => f == source.1,
        (RowKind::Session, DropSlot::BeforeSession(s) | DropSlot::AfterSession(s)) => s == source.1,
        _ => false,
    }
}

/// Destination folder of a drop (`Some(None)` = root, `None` = unknown
/// anchor).
fn resolve_dest(slot: DropSlot, folders: &[Folder], sessions: &[Session]) -> Option<Option<i64>> {
    match slot {
        DropSlot::IntoRoot => Some(None),
        DropSlot::IntoFolder(f) => Some(Some(f)),
        DropSlot::BeforeSession(s) | DropSlot::AfterSession(s) => sessions.iter().find(|x| x.id == s).map(|x| x.folder_id),
        DropSlot::BeforeFolder(f) | DropSlot::AfterFolder(f) => folders.iter().find(|x| x.id == f).map(|x| x.parent_id),
    }
}

/// Insert `dragged` next to the slot's anchor among `existing`; prepend
/// when the anchor is not a sibling of the dragged kind.
fn order_for_drop(existing: &[i64], dragged: i64, slot: DropSlot, kind: RowKind) -> Vec<i64> {
    let anchor = match (kind, slot) {
        (RowKind::Session, DropSlot::BeforeSession(a)) | (RowKind::Folder, DropSlot::BeforeFolder(a)) => Some((a, 0)),
        (RowKind::Session, DropSlot::AfterSession(a)) | (RowKind::Folder, DropSlot::AfterFolder(a)) => Some((a, 1)),
        _ => None,
    };
    let mut out = existing.to_vec();
    match anchor.and_then(|(a, off)| existing.iter().position(|x| *x == a).map(|i| i + off)) {
        Some(at) => out.insert(at, dragged),
        None => out.insert(0, dragged),
    }
    out
}

/// A core call a drop turns into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropOp {
    MoveSession { id: i64, folder: Option<i64> },
    ReorderSessions { folder: Option<i64>, ids: Vec<i64> },
    MoveFolder { id: i64, parent: Option<i64> },
    ReorderFolders { parent: Option<i64>, ids: Vec<i64> },
}

/// What dropping `source` on `slot` does: the calls, in order, and the
/// folder to expand so the user sees where it landed. `into` drops put
/// the item first in the folder (sort 0); before / after drops reorder
/// the whole sibling list.
pub fn plan_drop(source: (RowKind, i64), slot: DropSlot, folders: &[Folder], sessions: &[Session]) -> (Vec<DropOp>, Option<i64>) {
    let (kind, id) = source;
    let into = matches!(slot, DropSlot::IntoRoot | DropSlot::IntoFolder(_));
    let Some(dest) = resolve_dest(slot, folders, sessions) else {
        return (vec![], None);
    };
    match kind {
        RowKind::Session => {
            let Some(cur) = sessions.iter().find(|s| s.id == id) else {
                return (vec![], None);
            };
            if into {
                if cur.folder_id == dest {
                    return (vec![], None);
                }
                return (vec![DropOp::MoveSession { id, folder: dest }], dest);
            }
            let cross = cur.folder_id != dest;
            let mut ops = if cross { vec![DropOp::MoveSession { id, folder: dest }] } else { vec![] };
            let mut existing: Vec<&Session> = sessions.iter().filter(|s| s.folder_id == dest && s.id != id).collect();
            existing.sort_by_key(|s| (s.sort, s.id));
            let existing: Vec<i64> = existing.iter().map(|s| s.id).collect();
            let ordered = order_for_drop(&existing, id, slot, RowKind::Session);
            if ordered.len() > 1 || cross {
                ops.push(DropOp::ReorderSessions { folder: dest, ids: ordered });
            }
            (ops, dest)
        }
        RowKind::Folder => {
            let Some(cur) = folders.iter().find(|f| f.id == id) else {
                return (vec![], None);
            };
            if slot == DropSlot::IntoFolder(id) || dest == Some(id) {
                return (vec![], None);
            }
            if into {
                if cur.parent_id == dest {
                    return (vec![], None);
                }
                return (vec![DropOp::MoveFolder { id, parent: dest }], dest);
            }
            let cross = cur.parent_id != dest;
            let mut ops = if cross { vec![DropOp::MoveFolder { id, parent: dest }] } else { vec![] };
            let mut existing: Vec<&Folder> = folders.iter().filter(|f| f.parent_id == dest && f.id != id).collect();
            existing.sort_by_key(|f| (f.sort, f.id));
            let existing: Vec<i64> = existing.iter().map(|f| f.id).collect();
            let ordered = order_for_drop(&existing, id, slot, RowKind::Folder);
            if ordered.len() > 1 || cross {
                ops.push(DropOp::ReorderFolders { parent: dest, ids: ordered });
            }
            (ops, None)
        }
    }
}

/// `f` and every folder below it: invalid destinations for moving `f`.
pub fn descendants(folders: &[Folder], f: i64) -> HashSet<i64> {
    let mut out = HashSet::from([f]);
    let mut frontier = vec![f];
    while let Some(p) = frontier.pop() {
        for c in folders.iter().filter(|c| c.parent_id == Some(p)) {
            if out.insert(c.id) {
                frontier.push(c.id);
            }
        }
    }
    out
}

/// Case-insensitive A→Z (or Z→A) order of a folder's direct children:
/// (folder ids, session ids), each `None` when there is nothing to sort.
pub fn sorted_children(folders: &[Folder], sessions: &[Session], parent: Option<i64>, asc: bool) -> (Option<Vec<i64>>, Option<Vec<i64>>) {
    let cmp = |a: &str, b: &str| {
        let o = a.to_lowercase().cmp(&b.to_lowercase());
        if asc { o } else { o.reverse() }
    };
    let mut fs: Vec<&Folder> = folders.iter().filter(|f| f.parent_id == parent).collect();
    fs.sort_by(|a, b| cmp(&a.name, &b.name));
    let mut ss: Vec<&Session> = sessions.iter().filter(|s| s.folder_id == parent).collect();
    ss.sort_by(|a, b| cmp(&a.name, &b.name));
    (
        (fs.len() > 1).then(|| fs.iter().map(|f| f.id).collect()),
        (ss.len() > 1).then(|| ss.iter().map(|s| s.id).collect()),
    )
}

async fn run_ops(st: &ezssh_core::AppState, ops: Vec<DropOp>) -> ezssh_core::Result<()> {
    for op in ops {
        tracing::debug!(?op, "sidebar: drop op");
        match op {
            DropOp::MoveSession { id, folder } => api::sessions::session_move(st, id, folder, 0).await?,
            DropOp::ReorderSessions { folder, ids } => api::sessions::session_reorder(st, folder, ids).await?,
            DropOp::MoveFolder { id, parent } => api::folders::folder_move(st, id, parent, 0).await?,
            DropOp::ReorderFolders { parent, ids } => api::folders::folder_reorder(st, parent, ids).await?,
        }
    }
    Ok(())
}

/// `#rgb` / `#rrggbb` session colours.
pub fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    let v = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
    match h.len() {
        3 => Some(Color32::from_rgb(v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17)),
        6 => Some(Color32::from_rgb(v(0, 2)?, v(2, 2)?, v(4, 2)?)),
        _ => None,
    }
}

// ---------------------------------------------------------------- state

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    NewSession(Option<i64>),
    Edit(i64),
    Import,
    Backup,
    Restore,
    CloudSync,
    Credentials,
    ChangePassword,
    RecoveryCode,
    NewFolder(Option<i64>),
    Sort(Option<i64>, bool),
    MoveFolder(i64),
    RenameFolder(i64),
    DeleteFolder(i64),
    Settings,
    Connect(i64),
    Duplicate(i64),
    MoveSession(i64),
    DeleteSession(i64),
}

enum Prompt {
    NewFolder(Option<i64>, PromptState),
    RenameFolder(i64, PromptState),
}

enum Pending {
    DeleteFolder(Folder),
    DeleteSession(Box<Session>),
}

enum Moving {
    Session(Box<Session>, Box<MoveState>),
    Folder(Folder, Box<MoveState>),
}

#[derive(Default)]
pub struct Sidebar {
    pub folders: Vec<Folder>,
    pub sessions: Vec<Session>,
    expanded: HashSet<i64>,
    selected: Option<i64>,
    menu: Option<Menu<Action>>,
    prompt: Option<Prompt>,
    confirm: Option<Pending>,
    moving: Option<Moving>,
    drag: Option<(RowKind, i64)>,
    target: Option<DropSlot>,
    /// Row rectangles of the last frame, for hit-testing a drag.
    rows: Vec<(RowKind, i64, Rect)>,
}

impl Sidebar {
    pub fn reload(&self, core: &Core) {
        core.call(
            "folder_list+session_list",
            |st| async move {
                let (f, s) = tokio::join!(api::folders::folder_list(&st), api::sessions::session_list(&st));
                Ok::<_, ezssh_core::AppError>((f?, s?))
            },
            |app, r| match r {
                Ok((folders, sessions)) => {
                    // First time there are folders: open them all so new users
                    // see what is inside. Never re-open what was closed.
                    if app.sidebar.expanded.is_empty() && app.sidebar.folders.is_empty() {
                        app.sidebar.expanded = folders.iter().map(|f| f.id).collect();
                    }
                    app.sidebar.folders = folders;
                    app.sidebar.sessions = sessions;
                }
                Err(e) => app.toasts.danger("Could not load sessions", e.to_string()),
            },
        );
    }

    #[cfg(test)]
    pub fn target_for_test(&self) -> Option<DropSlot> {
        self.target
    }

    pub fn dialog_open(&self) -> bool {
        self.prompt.is_some() || self.confirm.is_some() || self.moving.is_some()
    }

    pub fn close_dialogs(&mut self) {
        self.prompt = None;
        self.confirm = None;
        self.moving = None;
        self.menu = None;
        self.drag = None;
        self.target = None;
    }

    fn folder_menu(&self, pos: Pos2, folder: Option<&Folder>) -> Menu<Action> {
        let id = folder.map(|f| f.id);
        let children = self.folders.iter().filter(|f| f.parent_id == id).count() + self.sessions.iter().filter(|s| s.folder_id == id).count();
        let mut items = vec![
            MenuEntry::item("New Session", Action::NewSession(id)),
            MenuEntry::item("New Folder", Action::NewFolder(id)),
            MenuEntry::Separator,
            MenuEntry::item("Sort A → Z", Action::Sort(id, true)).enabled(children > 1),
            MenuEntry::item("Sort Z → A", Action::Sort(id, false)).enabled(children > 1),
        ];
        if let Some(f) = folder {
            items.extend([
                MenuEntry::Separator,
                MenuEntry::item("Move to folder…", Action::MoveFolder(f.id)),
                MenuEntry::item("Rename", Action::RenameFolder(f.id)),
                MenuEntry::danger("Delete", Action::DeleteFolder(f.id)),
            ]);
        } else {
            items.extend([
                MenuEntry::Separator,
                MenuEntry::item("Import from MobaXterm…", Action::Import),
                MenuEntry::Separator,
                MenuEntry::item("Backup…", Action::Backup),
                MenuEntry::item("Restore…", Action::Restore),
                MenuEntry::Separator,
                MenuEntry::item("Cloud sync…", Action::CloudSync),
                MenuEntry::Separator,
                MenuEntry::item("Settings…", Action::Settings),
                MenuEntry::Separator,
                MenuEntry::item("Credentials…", Action::Credentials),
                MenuEntry::item("Change master password…", Action::ChangePassword),
                MenuEntry::item("Recovery code…", Action::RecoveryCode),
            ]);
        }
        Menu::new(pos, items)
    }

    fn session_menu(pos: Pos2, s: &Session) -> Menu<Action> {
        Menu::new(
            pos,
            vec![
                MenuEntry::item("Connect", Action::Connect(s.id)),
                MenuEntry::item("Edit", Action::Edit(s.id)),
                MenuEntry::item("Duplicate", Action::Duplicate(s.id)),
                MenuEntry::Separator,
                MenuEntry::item("Move to folder…", Action::MoveSession(s.id)),
                MenuEntry::Separator,
                MenuEntry::danger("Delete", Action::DeleteSession(s.id)),
            ],
        )
    }
}

// ---------------------------------------------------------------- view

const AUTOSCROLL_EDGE: f32 = 28.0;
const AUTOSCROLL_SPEED: f32 = 10.0;

pub fn show(app: &mut App, ui: &mut Ui) {
    let t = tokens(ui.ctx());
    let pane = ui.max_rect();
    let width = pane.width() - 1.0;

    // Toolbar.
    let bar = Rect::from_min_size(pane.min, vec2(width, ROW_H));
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, t.border));
    let mut toolbar = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(6.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    toolbar.spacing_mut().item_spacing.x = 2.0;
    if widgets::icon_button(&mut toolbar, Icon::Plus, 24.0, 14.0, false, "New session").clicked() {
        act(app, Action::NewSession(None));
    }
    if widgets::icon_button(&mut toolbar, Icon::FolderPlus, 24.0, 14.0, false, "New folder").clicked() {
        app.sidebar.prompt = Some(Prompt::NewFolder(None, PromptState::new("New folder", "Folder name", "", "My servers", "Create")));
    }
    if widgets::icon_button(&mut toolbar, Icon::FileInput, 24.0, 14.0, false, "Import from MobaXterm").clicked() {
        act(app, Action::Import);
    }
    toolbar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::icon_button(ui, Icon::PanelLeftClose, 24.0, 14.0, false, "Hide sessions (Ctrl+B)").clicked() {
            app.tabs.sidebar_collapsed = true;
        }
    });

    // Tree.
    let tree_rect = Rect::from_min_max(Pos2::new(pane.left(), bar.bottom()), Pos2::new(pane.left() + width, pane.bottom()));
    let mut tree = ui.new_child(egui::UiBuilder::new().max_rect(tree_rect).id_salt("sidebar-tree"));
    let dragging = app.sidebar.drag.is_some();
    if dragging && app.sidebar.target == Some(DropSlot::IntoRoot) {
        tree.painter().rect_filled(tree_rect, 0.0, alpha(t.accent, 0.1));
        tree.painter().rect_stroke(tree_rect.shrink(1.0), 0.0, Stroke::new(1.0, alpha(t.accent, 0.4)), StrokeKind::Inside);
    }
    // Background: root context menu, and the "into root" drop zone.
    let bg = tree.interact(tree_rect, Id::unique("sidebar-tree-bg"), Sense::click());
    if bg.secondary_clicked()
        && let Some(p) = bg.interact_pointer_pos()
    {
        app.sidebar.menu = Some(app.sidebar.folder_menu(p, None));
    }

    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    let mut rows = Vec::new();
    let mut opened: Option<Session> = None;
    let mut new_session = false;
    let mut actions: Vec<(Pos2, MenuOpen)> = Vec::new();
    egui::ScrollArea::vertical().id_salt("sidebar-scroll").auto_shrink(false).show(&mut tree, |ui| {
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        if app.sidebar.folders.is_empty() && app.sidebar.sessions.is_empty() {
            if widgets::empty_state(ui, Icon::Terminal, "No sessions yet", "Create your first saved connection.", Some(("New session", true)), true)
                .is_some_and(|r| r.clicked())
            {
                new_session = true;
            }
            return;
        }
        let mut ctx = Walk { rows: &mut rows, opened: &mut opened, menus: &mut actions };
        walk(ui, &mut app.sidebar, None, 0, &mut ctx);
        // Auto-scroll near the edges while dragging.
        if dragging && let Some(p) = pointer {
            let clip = ui.clip_rect();
            if clip.x_range().contains(p.x) {
                let d = if p.y < clip.top() + AUTOSCROLL_EDGE {
                    AUTOSCROLL_SPEED
                } else if p.y > clip.bottom() - AUTOSCROLL_EDGE {
                    -AUTOSCROLL_SPEED
                } else {
                    0.0
                };
                if d != 0.0 {
                    ui.scroll_with_delta(vec2(0.0, d));
                    ui.ctx().request_repaint();
                }
            }
        }
    });
    app.sidebar.rows = rows;
    if new_session {
        act(app, Action::NewSession(None));
    }
    for (pos, m) in actions {
        app.sidebar.menu = Some(match m {
            MenuOpen::Folder(f) => {
                let f = app.sidebar.folders.iter().find(|x| x.id == f).cloned();
                app.sidebar.folder_menu(pos, f.as_ref())
            }
            MenuOpen::Session(s) => match app.sidebar.sessions.iter().find(|x| x.id == s) {
                Some(s) => Sidebar::session_menu(pos, s),
                None => continue,
            },
        });
    }
    if let Some(s) = opened {
        app.open_session(s);
    }

    drag_update(app, ui, tree_rect);
    menu(app, ui.ctx());
}

enum MenuOpen {
    Folder(i64),
    Session(i64),
}

struct Walk<'a> {
    rows: &'a mut Vec<(RowKind, i64, Rect)>,
    opened: &'a mut Option<Session>,
    menus: &'a mut Vec<(Pos2, MenuOpen)>,
}

/// Folders before sessions at every depth.
fn walk(ui: &mut Ui, sb: &mut Sidebar, parent: Option<i64>, depth: usize, w: &mut Walk<'_>) {
    let folders: Vec<Folder> = sb.folders.iter().filter(|f| f.parent_id == parent).cloned().collect();
    for f in folders {
        let open = sb.expanded.contains(&f.id);
        folder_row(ui, sb, &f, depth, open, w);
        if open {
            walk(ui, sb, Some(f.id), depth + 1, w);
        }
    }
    let sessions: Vec<Session> = sb.sessions.iter().filter(|s| s.folder_id == parent).cloned().collect();
    for s in sessions {
        session_row(ui, sb, &s, depth, w);
    }
}

fn count_sessions(sb: &Sidebar, folder: i64) -> usize {
    sb.sessions.iter().filter(|s| s.folder_id == Some(folder)).count()
        + sb.folders.iter().filter(|f| f.parent_id == Some(folder)).map(|f| count_sessions(sb, f.id)).sum::<usize>()
}

fn drop_line(ui: &Ui, rect: Rect, top: bool) {
    let t = tokens(ui.ctx());
    let y = if top { rect.top() } else { rect.bottom() };
    ui.painter().rect_filled(Rect::from_min_max(Pos2::new(rect.left(), y - 1.0), Pos2::new(rect.right(), y + 1.0)), 1.0, t.accent);
}

fn folder_row(ui: &mut Ui, sb: &mut Sidebar, f: &Folder, depth: usize, open: bool, w: &mut Walk<'_>) {
    let t = tokens(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    w.rows.push((RowKind::Folder, f.id, rect));
    let source = sb.drag == Some((RowKind::Folder, f.id));
    let into = sb.drag.is_some() && sb.target == Some(DropSlot::IntoFolder(f.id));
    let hovered = resp.hovered() || resp.contains_pointer();
    if into {
        ui.painter().rect_filled(rect, 0.0, alpha(t.accent, 0.2));
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, alpha(t.accent, 0.6)), StrokeKind::Inside);
    } else if hovered && sb.drag.is_none() {
        ui.painter().rect_filled(rect, 0.0, alpha(t.surface2, 0.7));
    }
    widgets::a11y_selected(&resp, egui::Role::TreeItem, open, &f.name);
    let fade = if source { 0.5 } else { 1.0 };
    let x0 = rect.left() + 6.0 + depth as f32 * 12.0;
    let cy = rect.center().y;
    (if open { Icon::ChevronDown } else { Icon::ChevronRight }).paint(
        ui,
        Rect::from_center_size(Pos2::new(x0 + 6.0, cy), vec2(12.0, 12.0)),
        11.0,
        alpha(t.muted, 0.8 * fade),
    );
    let (icon, ic) = if open { (Icon::FolderOpen, t.accent) } else { (Icon::Folder, alpha(t.accent, 0.7)) };
    icon.paint(ui, Rect::from_center_size(Pos2::new(x0 + 12.0 + 6.0 + 7.0, cy), vec2(14.0, 14.0)), 14.0, alpha(ic, ic.a() as f32 / 255.0 * fade));

    // Right side: trash on hover, then the session count badge.
    let mut right = rect.right() - 8.0;
    let trash_rect = Rect::from_center_size(Pos2::new(right - 10.0, cy), vec2(20.0, 20.0));
    let trash = ui.interact(trash_rect, resp.id.with("trash"), Sense::click());
    if hovered && sb.drag.is_none() {
        widgets::paint_icon_button(ui, &trash, trash_rect, Icon::Trash, 11.0, false, Some(t.danger));
        right -= 22.0;
    }
    widgets::a11y(&trash, egui::Role::Button, &format!("Delete folder {}", f.name));
    if trash.on_hover_text("Delete folder").clicked() {
        sb.confirm = Some(Pending::DeleteFolder(f.clone()));
    }
    let count = count_sessions(sb, f.id);
    if count > 0 {
        let g = ui.painter().layout_no_wrap(count.to_string(), egui::FontId::monospace(10.0), t.muted);
        let badge = Rect::from_min_max(Pos2::new(right - g.size().x - 12.0, cy - 8.0), Pos2::new(right, cy + 8.0));
        ui.painter().rect_filled(badge, 2.0, alpha(t.surface2, if hovered { 1.0 } else { 0.8 }));
        let color = if hovered { alpha(t.fg, 0.8) } else { t.muted };
        ui.painter().galley_with_override_text_color(badge.center() - g.size() / 2.0, g, color);
        right = badge.left() - 6.0;
        ui.interact(badge, resp.id.with("badge"), Sense::hover()).on_hover_text(format!("{count} session{}", if count == 1 { "" } else { "s" }));
    }
    let name_left = x0 + 12.0 + 6.0 + 14.0 + 6.0;
    let color = alpha(if hovered || into { t.fg } else { alpha(t.fg, 0.8) }, fade);
    widgets::paint_text(ui, Rect::from_min_max(Pos2::new(name_left, rect.top()), Pos2::new(right, rect.bottom())), &f.name, font_semibold(size::XS), color);

    if sb.target == Some(DropSlot::BeforeFolder(f.id)) {
        drop_line(ui, rect, true);
    }
    if sb.target == Some(DropSlot::AfterFolder(f.id)) {
        drop_line(ui, rect, false);
    }
    if resp.drag_started() {
        sb.drag = Some((RowKind::Folder, f.id));
    }
    if resp.clicked() && !sb.expanded.remove(&f.id) {
        sb.expanded.insert(f.id);
    }
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        w.menus.push((p, MenuOpen::Folder(f.id)));
    }
}

fn session_row(ui: &mut Ui, sb: &mut Sidebar, s: &Session, depth: usize, w: &mut Walk<'_>) {
    let t = tokens(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    w.rows.push((RowKind::Session, s.id, rect));
    let selected = sb.selected == Some(s.id);
    let source = sb.drag == Some((RowKind::Session, s.id));
    let hovered = resp.hovered() && sb.drag.is_none();
    if selected {
        ui.painter().rect_filled(rect, 0.0, alpha(t.accent, 0.18));
    } else if hovered {
        ui.painter().rect_filled(rect, 0.0, alpha(t.surface2, 0.7));
    }
    widgets::a11y_selected(&resp, egui::Role::TreeItem, selected, &s.name);
    let fade = if source { 0.5 } else { 1.0 };
    let color = parse_color(s.color.as_deref().unwrap_or(""));
    // Left rail: selected accent > session colour.
    let rail = if selected { Some(t.accent) } else { color };
    if let Some(c) = rail {
        ui.painter().rect_filled(Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 4.0), Pos2::new(rect.left() + 3.0, rect.bottom() - 4.0)), 1.0, c);
    }
    let x0 = rect.left() + 6.0 + (depth + 1) as f32 * 12.0;
    let tile = Rect::from_center_size(Pos2::new(x0 + 9.0, rect.center().y), vec2(18.0, 18.0));
    let kind_default = match s.session_kind.as_str() {
        "wsl" => t.warning,
        "local" => t.success,
        _ => t.accent,
    };
    ui.painter().rect_filled(tile, 2.0, alpha(color.unwrap_or(kind_default), fade));
    let icon = match s.session_kind.as_str() {
        "wsl" => Icon::SquareTerminal,
        "local" => Icon::MonitorDot,
        _ => Icon::Server,
    };
    icon.paint(ui, tile, 11.0, alpha(Color32::WHITE, fade));
    let name_rect = Rect::from_min_max(Pos2::new(tile.right() + 8.0, rect.top()), Pos2::new(rect.right() - 8.0, rect.bottom()));
    let f = if selected { font_semibold(size::XS) } else { font(size::XS) };
    widgets::paint_text(ui, name_rect, &s.name, f, alpha(if selected || hovered { t.fg } else { alpha(t.fg, 0.9) }, fade));

    if sb.target == Some(DropSlot::BeforeSession(s.id)) {
        drop_line(ui, rect, true);
    }
    if sb.target == Some(DropSlot::AfterSession(s.id)) {
        drop_line(ui, rect, false);
    }
    let resp = resp.on_hover_text(session_summary(s));
    if resp.drag_started() {
        sb.drag = Some((RowKind::Session, s.id));
    }
    if resp.clicked() {
        sb.selected = Some(s.id);
    }
    if resp.double_clicked() {
        *w.opened = Some(s.clone());
    }
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        sb.selected = Some(s.id);
        w.menus.push((p, MenuOpen::Session(s.id)));
    }
}

/// Track the drop target while dragging; drop on release, cancel on Escape.
fn drag_update(app: &mut App, ui: &Ui, tree_rect: Rect) {
    let Some(source) = app.sidebar.drag else {
        return;
    };
    let (pos, down, escape) = ui.ctx().input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.key_pressed(egui::Key::Escape)));
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let slot = pos.and_then(|p| {
        if !tree_rect.contains(p) {
            return None;
        }
        let hit = app.sidebar.rows.iter().find(|(_, _, r)| r.contains(p));
        let slot = match hit {
            Some((kind, id, r)) => slot_for_row((p.y - r.top()) / r.height().max(1.0), *kind, *id, source.0),
            None => DropSlot::IntoRoot,
        };
        (!slot_rejected(slot, source)).then_some(slot)
    });
    app.sidebar.target = slot;
    if escape {
        tracing::debug!("sidebar: drag cancelled");
        app.sidebar.drag = None;
        app.sidebar.target = None;
        return;
    }
    if down {
        return;
    }
    app.sidebar.drag = None;
    app.sidebar.target = None;
    let Some(slot) = slot else {
        return;
    };
    let (ops, expand) = plan_drop(source, slot, &app.sidebar.folders, &app.sidebar.sessions);
    tracing::info!(?source, ?slot, ops = ops.len(), "sidebar: drop");
    if let Some(f) = expand {
        app.sidebar.expanded.insert(f);
    }
    if ops.is_empty() {
        return;
    }
    app.core.call(
        "sidebar drop",
        move |st| async move { run_ops(&st, ops).await },
        |app, r| {
            if let Err(e) = r {
                app.toasts.danger("Move failed", e.to_string());
            }
            app.sidebar.reload(&app.core);
        },
    );
}

fn menu(app: &mut App, ctx: &egui::Context) {
    let Some(m) = &mut app.sidebar.menu else {
        return;
    };
    match m.show(ctx, Id::unique("sidebar-menu")) {
        MenuResult::Open => {}
        MenuResult::Closed => app.sidebar.menu = None,
        MenuResult::Chosen(a) => {
            app.sidebar.menu = None;
            act(app, a);
        }
    }
}

fn act(app: &mut App, a: Action) {
    tracing::debug!(?a, "sidebar: action");
    let sb = &mut app.sidebar;
    match a {
        Action::NewSession(folder) => app.dialogs.session = Some(crate::views::session_dialog::SessionDialog::new_session(folder)),
        Action::Edit(id) => {
            if let Some(s) = sb.sessions.iter().find(|s| s.id == id).cloned() {
                app.dialogs.session = Some(crate::views::session_dialog::SessionDialog::edit(s));
            }
        }
        Action::Import => app.dialogs.import = Some(Default::default()),
        Action::Backup => app.dialogs.backup = Some(Default::default()),
        Action::Restore => app.dialogs.restore = Some(Default::default()),
        Action::CloudSync => app.dialogs.sync = Some(Default::default()),
        Action::Credentials => app.dialogs.credentials = Some(Default::default()),
        Action::ChangePassword => app.dialogs.change_password = Some(Default::default()),
        Action::RecoveryCode => app.dialogs.recovery_code = Some(Default::default()),
        Action::NewFolder(parent) => sb.prompt = Some(Prompt::NewFolder(parent, PromptState::new("New folder", "Folder name", "", "My servers", "Create"))),
        Action::RenameFolder(id) => {
            if let Some(f) = sb.folders.iter().find(|f| f.id == id) {
                sb.prompt = Some(Prompt::RenameFolder(id, PromptState::new(format!("Rename \"{}\"", f.name), "Folder name", &f.name, "", "Rename")));
            }
        }
        Action::DeleteFolder(id) => sb.confirm = sb.folders.iter().find(|f| f.id == id).cloned().map(Pending::DeleteFolder),
        Action::DeleteSession(id) => sb.confirm = sb.sessions.iter().find(|s| s.id == id).cloned().map(|s| Pending::DeleteSession(Box::new(s))),
        Action::MoveFolder(id) => {
            if let Some(f) = sb.folders.iter().find(|f| f.id == id).cloned() {
                let st = MoveState::new(format!("Move folder \"{}\" to…", f.name), &sb.folders, f.parent_id, descendants(&sb.folders, f.id));
                sb.moving = Some(Moving::Folder(f, Box::new(st)));
            }
        }
        Action::MoveSession(id) => {
            if let Some(s) = sb.sessions.iter().find(|s| s.id == id).cloned() {
                let st = MoveState::new(format!("Move \"{}\" to…", s.name), &sb.folders, s.folder_id, HashSet::new());
                sb.moving = Some(Moving::Session(Box::new(s), Box::new(st)));
            }
        }
        Action::Sort(parent, asc) => {
            let (fs, ss) = sorted_children(&sb.folders, &sb.sessions, parent, asc);
            if fs.is_none() && ss.is_none() {
                return;
            }
            app.core.call(
                "sort",
                move |st| async move {
                    if let Some(ids) = fs {
                        api::folders::folder_reorder(&st, parent, ids).await?;
                    }
                    if let Some(ids) = ss {
                        api::sessions::session_reorder(&st, parent, ids).await?;
                    }
                    Ok::<_, ezssh_core::AppError>(())
                },
                move |app, r| {
                    match r {
                        Ok(()) => app.toasts.success("Sorted", (if asc { "A → Z" } else { "Z → A" }).to_string()),
                        Err(e) => app.toasts.danger("Action failed", e.to_string()),
                    }
                    app.sidebar.reload(&app.core);
                },
            );
        }
        Action::Settings => app.dialogs.settings = Some(crate::views::settings::SettingsDialog::default()),
        Action::Connect(id) => {
            if let Some(s) = sb.sessions.iter().find(|s| s.id == id).cloned() {
                app.open_session(s);
            }
        }
        Action::Duplicate(id) => app.core.call(
            "session_duplicate",
            move |st| async move { api::sessions::session_duplicate(&st, id).await },
            |app, r| {
                if let Err(e) = r {
                    app.toasts.danger("Action failed", e.to_string());
                }
                app.sidebar.reload(&app.core);
            },
        ),
    }
}

/// Folder / session dialogs, drawn after the main layout.
pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    let mut done = false;
    if let Some(p) = &mut app.sidebar.prompt {
        let (state, parent, rename) = match p {
            Prompt::NewFolder(parent, s) => (s, *parent, None),
            Prompt::RenameFolder(id, s) => (s, None, Some(*id)),
        };
        match state.show(ctx, Id::unique("sidebar-prompt")) {
            Some(PromptAnswer::Cancel) => done = true,
            Some(PromptAnswer::Submit(name)) => {
                done = true;
                let n = name.clone();
                match rename {
                    None => app.core.call(
                        "folder_create",
                        move |st| async move { api::folders::folder_create(&st, parent, n).await },
                        move |app, r| {
                            match r {
                                Ok(_) => app.toasts.success("Folder created", name),
                                Err(e) => app.toasts.danger("Action failed", e.to_string()),
                            }
                            app.sidebar.reload(&app.core);
                        },
                    ),
                    Some(id) => app.core.call(
                        "folder_rename",
                        move |st| async move { api::folders::folder_rename(&st, id, n).await },
                        |app, r| {
                            match r {
                                Ok(()) => app.toasts.success("Folder renamed", None),
                                Err(e) => app.toasts.danger("Action failed", e.to_string()),
                            }
                            app.sidebar.reload(&app.core);
                        },
                    ),
                }
            }
            None => {}
        }
    }
    if done {
        app.sidebar.prompt = None;
    }

    if let Some(c) = &app.sidebar.confirm {
        let (title, body) = match c {
            Pending::DeleteFolder(f) => {
                (format!("Delete folder \"{}\"?", f.name), "Subfolders will be deleted. Sessions inside will be moved to the root.")
            }
            Pending::DeleteSession(s) => {
                (format!("Delete \"{}\"?", s.name), "This also removes any saved credentials attached to this session.")
            }
        };
        let answer = dialogs::confirm(
            ctx,
            Id::unique("sidebar-confirm"),
            dialogs::Confirm { title: &title, body, confirm_text: "Delete", danger: true },
        );
        if let Some(yes) = answer
            && let Some(c) = app.sidebar.confirm.take()
            && yes
        {
            match c {
                Pending::DeleteFolder(f) => app.core.call(
                    "folder_delete",
                    move |st| async move { api::folders::folder_delete(&st, f.id).await },
                    move |app, r| {
                        match r {
                            Ok(()) => app.toasts.success("Folder deleted", f.name),
                            Err(e) => app.toasts.danger("Action failed", e.to_string()),
                        }
                        app.sidebar.reload(&app.core);
                    },
                ),
                Pending::DeleteSession(s) => app.core.call(
                    "session_delete",
                    {
                        let id = s.id;
                        move |st| async move { api::sessions::session_delete(&st, id).await }
                    },
                    move |app, r| {
                        match r {
                            Ok(()) => app.toasts.success("Session deleted", s.name),
                            Err(e) => app.toasts.danger("Action failed", e.to_string()),
                        }
                        app.sidebar.reload(&app.core);
                    },
                ),
            }
        }
    }

    if let Some(m) = &mut app.sidebar.moving {
        let folders = app.sidebar.folders.clone();
        let state = match m {
            Moving::Session(_, s) | Moving::Folder(_, s) => s,
        };
        let answer = state.show(ctx, Id::unique("sidebar-move"), &folders);
        if let Some(choice) = answer
            && let Some(m) = app.sidebar.moving.take()
            && let Some(dest) = choice
        {
            if let Some(d) = dest {
                app.sidebar.expanded.insert(d);
            }
            match m {
                Moving::Session(s, _) => app.core.call(
                    "session_move",
                    {
                        let id = s.id;
                        move |st| async move { api::sessions::session_move(&st, id, dest, 0).await }
                    },
                    move |app, r| {
                        match r {
                            Ok(()) => app.toasts.success("Session moved", s.name),
                            Err(e) => app.toasts.danger("Action failed", e.to_string()),
                        }
                        app.sidebar.reload(&app.core);
                    },
                ),
                Moving::Folder(f, _) => app.core.call(
                    "folder_move",
                    move |st| async move { api::folders::folder_move(&st, f.id, dest, 0).await },
                    move |app, r| {
                        match r {
                            Ok(()) => app.toasts.success("Folder moved", f.name),
                            Err(e) => app.toasts.danger("Action failed", e.to_string()),
                        }
                        app.sidebar.reload(&app.core);
                    },
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::test_session;

    fn folder(id: i64, parent: Option<i64>, sort: i64, name: &str) -> Folder {
        Folder { id, parent_id: parent, name: name.into(), sort }
    }

    fn session(id: i64, folder: Option<i64>, sort: i64, name: &str) -> Session {
        Session { folder_id: folder, sort, ..test_session(id, name, "ssh") }
    }

    #[test]
    fn row_slots() {
        use DropSlot::*;
        assert_eq!(slot_for_row(0.2, RowKind::Session, 5, RowKind::Session), BeforeSession(5));
        assert_eq!(slot_for_row(0.6, RowKind::Session, 5, RowKind::Folder), AfterSession(5));
        // A session dragged over a folder always goes into it.
        assert_eq!(slot_for_row(0.1, RowKind::Folder, 7, RowKind::Session), IntoFolder(7));
        assert_eq!(slot_for_row(0.9, RowKind::Folder, 7, RowKind::Session), IntoFolder(7));
        // A folder drag splits 25 / 50 / 25.
        assert_eq!(slot_for_row(0.2, RowKind::Folder, 7, RowKind::Folder), BeforeFolder(7));
        assert_eq!(slot_for_row(0.5, RowKind::Folder, 7, RowKind::Folder), IntoFolder(7));
        assert_eq!(slot_for_row(0.8, RowKind::Folder, 7, RowKind::Folder), AfterFolder(7));
    }

    #[test]
    fn self_drops_rejected() {
        use DropSlot::*;
        assert!(slot_rejected(IntoFolder(3), (RowKind::Folder, 3)));
        assert!(slot_rejected(AfterFolder(3), (RowKind::Folder, 3)));
        assert!(!slot_rejected(IntoFolder(4), (RowKind::Folder, 3)));
        assert!(slot_rejected(BeforeSession(9), (RowKind::Session, 9)));
        assert!(!slot_rejected(IntoFolder(9), (RowKind::Session, 9)));
    }

    fn data() -> (Vec<Folder>, Vec<Session>) {
        let folders = vec![folder(1, None, 0, "A"), folder(2, None, 1, "B"), folder(3, Some(1), 0, "A1")];
        let sessions = vec![session(10, None, 0, "r0"), session(11, Some(1), 0, "a0"), session(12, Some(1), 1, "a1"), session(13, Some(2), 0, "b0")];
        (folders, sessions)
    }

    #[test]
    fn session_into_folder_moves_to_top() {
        let (f, s) = data();
        let (ops, expand) = plan_drop((RowKind::Session, 10), DropSlot::IntoFolder(2), &f, &s);
        assert_eq!(ops, vec![DropOp::MoveSession { id: 10, folder: Some(2) }]);
        assert_eq!(expand, Some(2));
        // Into the folder it is already in: nothing.
        assert_eq!(plan_drop((RowKind::Session, 11), DropSlot::IntoFolder(1), &f, &s).0, vec![]);
        assert_eq!(plan_drop((RowKind::Session, 10), DropSlot::IntoRoot, &f, &s).0, vec![]);
    }

    #[test]
    fn session_reorder_within_and_across_folders() {
        let (f, s) = data();
        // a0 after a1, same folder.
        let (ops, _) = plan_drop((RowKind::Session, 11), DropSlot::AfterSession(12), &f, &s);
        assert_eq!(ops, vec![DropOp::ReorderSessions { folder: Some(1), ids: vec![12, 11] }]);
        // r0 before a1: cross-folder move, then reorder there.
        let (ops, expand) = plan_drop((RowKind::Session, 10), DropSlot::BeforeSession(12), &f, &s);
        assert_eq!(
            ops,
            vec![DropOp::MoveSession { id: 10, folder: Some(1) }, DropOp::ReorderSessions { folder: Some(1), ids: vec![11, 10, 12] }]
        );
        assert_eq!(expand, Some(1));
        // Alone in its folder, dropped next to nobody new: no calls.
        let (ops, _) = plan_drop((RowKind::Session, 13), DropSlot::BeforeSession(99), &f, &s);
        assert_eq!(ops, vec![], "unknown anchor resolves to no destination");
    }

    #[test]
    fn folder_moves_and_reorders() {
        let (f, s) = data();
        let (ops, expand) = plan_drop((RowKind::Folder, 2), DropSlot::IntoFolder(1), &f, &s);
        assert_eq!(ops, vec![DropOp::MoveFolder { id: 2, parent: Some(1) }]);
        assert_eq!(expand, Some(1));
        assert_eq!(plan_drop((RowKind::Folder, 3), DropSlot::IntoFolder(1), &f, &s).0, vec![], "already there");
        assert_eq!(plan_drop((RowKind::Folder, 1), DropSlot::IntoFolder(1), &f, &s).0, vec![], "into itself");
        // B before A at the root.
        let (ops, _) = plan_drop((RowKind::Folder, 2), DropSlot::BeforeFolder(1), &f, &s);
        assert_eq!(ops, vec![DropOp::ReorderFolders { parent: None, ids: vec![2, 1] }]);
        // A1 after B: out of A, to the root, after B.
        let (ops, _) = plan_drop((RowKind::Folder, 3), DropSlot::AfterFolder(2), &f, &s);
        assert_eq!(ops, vec![DropOp::MoveFolder { id: 3, parent: None }, DropOp::ReorderFolders { parent: None, ids: vec![1, 2, 3] }]);
    }

    #[test]
    fn descendants_and_sorting() {
        let (f, s) = data();
        assert_eq!(descendants(&f, 1), HashSet::from([1, 3]));
        assert_eq!(descendants(&f, 2), HashSet::from([2]));
        let (fs, ss) = sorted_children(&f, &s, None, false);
        assert_eq!(fs, Some(vec![2, 1]));
        assert_eq!(ss, None, "one root session: nothing to sort");
        let mixed = vec![session(1, None, 0, "beta"), session(2, None, 1, "Alpha"), session(3, None, 2, "alpha2")];
        assert_eq!(sorted_children(&[], &mixed, None, true).1, Some(vec![2, 3, 1]), "case-insensitive");
    }

    #[test]
    fn colours() {
        assert_eq!(parse_color("#e06c75"), Some(Color32::from_rgb(0xe0, 0x6c, 0x75)));
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#12345"), None);
    }
}
