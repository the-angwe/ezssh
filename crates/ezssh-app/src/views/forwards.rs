//! `forwards-pane.tsx` + `forward-dialog.tsx` + `lib/forwards.ts`: the
//! 280 px port-forward pane docked in an SSH tab (saved and tab-only
//! forwards with live status), its add / edit dialog, and the session
//! dialog's Forwards tab (`ForwardsConfigPane`).
//!
//! Saved forwards are kept per session and shared by the pane and the
//! session tab; runtime forwards are kept per connection and fed by core
//! `ForwardStatus` events even while no pane is open, so a pane opened
//! later shows auto-started forwards at once.

use std::collections::HashMap;

use egui::{Align, Color32, CursorIcon, Id, Layout, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, vec2};
use ezssh_core::api;
use ezssh_core::api::forwards::ForwardStartTarget;
use ezssh_core::db::forwards::{Forward, ForwardInput};
use ezssh_core::ssh::forwards::{ForwardKind, ForwardSpec, ForwardStatus, RuntimeForwardSummary};

use crate::app::App;
use crate::form::{self, InputOpts};
use crate::icons::Icon;
use crate::style::{alpha, font, font_semibold, size, tokens};
use crate::tabs::TabId;
use crate::widgets::{self, Menu, MenuEntry, MenuResult, Tone};

// ---------------------------------------------------------------- lib/forwards.ts

/// The fields a label is made of, from a saved row or a runtime spec.
struct LabelParts<'a> {
    name: &'a str,
    kind: ForwardKind,
    bind_addr: &'a str,
    bind_port: i64,
    dest_addr: &'a str,
    dest_port: i64,
}

/// `forwardLabel`: the user's name, else an automatic label from kind and
/// addresses so a row is never blank.
fn label_of(p: LabelParts<'_>) -> String {
    if !p.name.is_empty() {
        return p.name.to_string();
    }
    match p.kind {
        ForwardKind::Dynamic => format!("SOCKS5 @ {}:{}", p.bind_addr, p.bind_port),
        _ => format!("{}:{} → {}:{}", p.bind_addr, p.bind_port, p.dest_addr, p.dest_port),
    }
}

fn saved_label(f: &Forward) -> String {
    label_of(LabelParts {
        name: &f.name,
        kind: parse_kind(&f.kind),
        bind_addr: &f.bind_addr,
        bind_port: f.bind_port,
        dest_addr: &f.dest_addr,
        dest_port: f.dest_port,
    })
}

fn spec_label(s: &ForwardSpec) -> String {
    label_of(LabelParts {
        name: &s.name,
        kind: s.kind,
        bind_addr: &s.bind_addr,
        bind_port: s.bind_port.into(),
        dest_addr: &s.dest_addr,
        dest_port: s.dest_port.into(),
    })
}

/// The DB stores the kind as text; anything unknown reads as local.
fn parse_kind(s: &str) -> ForwardKind {
    match s {
        "local" => ForwardKind::Local,
        "remote" => ForwardKind::Remote,
        "dynamic" => ForwardKind::Dynamic,
        other => {
            tracing::warn!(kind = other, "forwards: unknown kind in the database, shown as local");
            ForwardKind::Local
        }
    }
}

fn kind_str(k: ForwardKind) -> &'static str {
    match k {
        ForwardKind::Local => "local",
        ForwardKind::Remote => "remote",
        ForwardKind::Dynamic => "dynamic",
    }
}

fn kind_icon(k: ForwardKind) -> Icon {
    match k {
        ForwardKind::Local => Icon::ArrowLeftRight,
        ForwardKind::Remote => Icon::Server,
        ForwardKind::Dynamic => Icon::Globe2,
    }
}

/// `KIND_TONE`: Tailwind blue-400 / amber-400 / emerald-400 (both themes).
fn kind_tone(k: ForwardKind) -> Color32 {
    match k {
        ForwardKind::Local => Color32::from_rgb(96, 165, 250),
        ForwardKind::Remote => Color32::from_rgb(251, 191, 36),
        ForwardKind::Dynamic => Color32::from_rgb(52, 211, 153),
    }
}

/// Tailwind amber-500: the neutral "conflict" tone.
const AMBER_500: Color32 = Color32::from_rgb(245, 158, 11);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowStatus {
    Idle,
    Running,
    Error,
    Conflict,
}

fn row_status(rf: Option<&RuntimeForwardSummary>) -> RowStatus {
    match rf.map(|r| &r.status) {
        Some(ForwardStatus::Running) => RowStatus::Running,
        Some(ForwardStatus::Error { .. }) => RowStatus::Error,
        Some(ForwardStatus::Conflict { .. }) => RowStatus::Conflict,
        _ => RowStatus::Idle,
    }
}

/// The message under a row (errors and conflicts only).
fn row_note(rf: Option<&RuntimeForwardSummary>) -> Option<String> {
    match rf.map(|r| &r.status) {
        Some(ForwardStatus::Error { message } | ForwardStatus::Conflict { message }) => Some(message.clone()),
        _ => None,
    }
}

/// Insert or replace by runtime id: a manual Start's return value and its
/// status event carry the same id and coalesce into one row.
fn upsert(rows: &mut Vec<RuntimeForwardSummary>, rf: RuntimeForwardSummary) {
    match rows.iter().position(|x| x.runtime_id == rf.runtime_id) {
        Some(i) => rows[i] = rf,
        None => rows.push(rf),
    }
}

/// A status event: a stopped saved forward drops its runtime row (the
/// saved row stays, idle); a stopped tab-only forward keeps its row with
/// the new status, as the webview did.
fn apply_status(rows: &mut Vec<RuntimeForwardSummary>, rf: RuntimeForwardSummary) {
    if matches!(rf.status, ForwardStatus::Stopped) {
        if rf.persistent_id.is_some() {
            rows.retain(|x| x.runtime_id != rf.runtime_id);
        } else if let Some(i) = rows.iter().position(|x| x.runtime_id == rf.runtime_id) {
            rows[i] = rf;
        }
        return;
    }
    upsert(rows, rf);
}

// ---------------------------------------------------------------- state

#[derive(Default)]
struct SavedList {
    rows: Vec<Forward>,
    loaded: bool,
}

#[derive(Default)]
struct RuntimeList {
    rows: Vec<RuntimeForwardSummary>,
    requested: bool,
}

/// A view that loads on "mount" (first frame drawn after not being drawn).
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum ViewKey {
    Pane(TabId),
    SessionTab(i64),
}

/// Everything the panes keep between frames.
#[derive(Default)]
pub struct ForwardsState {
    /// Saved forwards per session id.
    saved: HashMap<i64, SavedList>,
    /// Runtime forwards per connection id.
    runtime: HashMap<u64, RuntimeList>,
    /// Frame each view last drew; a view not drawn the frame before
    /// reloads its saved list, as the webview's remount did.
    seen: HashMap<ViewKey, u64>,
    dialog: Option<ForwardDialog>,
    menu: Option<(TabId, Menu<Action>)>,
    next_token: u64,
}

impl ForwardsState {
    /// Drop the dialog and menu (the vault locked).
    pub fn close_dialogs(&mut self) {
        self.dialog = None;
        self.menu = None;
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// True when `key` was not drawn last frame (and marks it drawn now).
    fn mounted(&mut self, key: ViewKey, frame: u64) -> bool {
        let prev = self.seen.insert(key, frame);
        !matches!(prev, Some(p) if p + 1 >= frame)
    }

    fn open_dialog(&mut self, mode: Mode) {
        self.next_token += 1;
        tracing::debug!(mode = mode.name(), token = self.next_token, "forwards: dialog open");
        self.dialog = Some(ForwardDialog::new(mode, self.next_token));
    }
}

#[cfg(test)]
impl ForwardsState {
    /// Treat `connection_id` as a live connection whose runtime list is
    /// loaded (the UI tests have no sshd).
    pub(crate) fn fake_connection(&mut self, connection_id: u64) {
        self.runtime.entry(connection_id).or_default().requested = true;
    }

    /// Labels of the saved forwards of `session_id`, once loaded.
    pub(crate) fn saved_labels(&self, session_id: i64) -> Option<Vec<String>> {
        self.saved.get(&session_id).filter(|l| l.loaded).map(|l| l.rows.iter().map(saved_label).collect())
    }

    pub(crate) fn dialog_error(&self) -> Option<&str> {
        self.dialog.as_ref().and_then(|d| d.err.as_deref())
    }

    pub(crate) fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
}

/// What a click in the pane or the session tab asks for.
#[derive(Clone)]
enum Action {
    AddTab,
    AddSaved(i64),
    EditSaved(Forward),
    Start(i64),
    /// Stop a running saved forward (runtime id).
    Stop(u64),
    DeleteSaved { session_id: i64, id: i64 },
    EditTab(RuntimeForwardSummary),
    /// Stop a tab-only forward and drop its row (runtime id).
    StopTab(u64),
}

// ---------------------------------------------------------------- loading

fn load_saved(app: &mut App, session_id: i64) {
    tracing::debug!(session = session_id, "forwards: loading saved forwards");
    app.core.call(
        "forward_list",
        move |st| async move { api::forwards::forward_list(&st, session_id).await },
        move |app, r| match r {
            Ok(rows) => {
                tracing::debug!(session = session_id, n = rows.len(), "forwards: saved loaded");
                let list = app.forwards.saved.entry(session_id).or_default();
                list.rows = rows;
                list.loaded = true;
            }
            Err(e) => {
                tracing::warn!(session = session_id, "forwards: forward_list failed: {e}");
                app.toasts.danger("Could not load forwards", e.to_string());
            }
        },
    );
}

fn load_runtime(app: &mut App, connection_id: u64) {
    app.forwards.runtime.entry(connection_id).or_default().requested = true;
    tracing::debug!(connection_id, "forwards: loading runtime forwards");
    app.core.call(
        "forward_runtime_list",
        move |st| async move { api::forwards::forward_runtime_list(&st, connection_id).await },
        move |app, r| match r {
            Ok(rows) => {
                tracing::debug!(connection_id, n = rows.len(), "forwards: runtime loaded");
                let list = app.forwards.runtime.entry(connection_id).or_default();
                // Events that arrived meanwhile are newer than the snapshot.
                for rf in rows {
                    if !list.rows.iter().any(|x| x.runtime_id == rf.runtime_id) {
                        list.rows.push(rf);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(connection_id, "forwards: forward_runtime_list failed: {e}");
                app.toasts.danger("Could not load running forwards", e.to_string());
            }
        },
    );
}

/// A core `Event::ForwardStatus`.
pub fn on_event(app: &mut App, event: ezssh_core::Event) {
    let ezssh_core::Event::ForwardStatus { connection_id, summary } = event else {
        tracing::debug!(?event, "forwards: not a forward event, ignored");
        return;
    };
    tracing::debug!(
        connection_id,
        runtime_id = summary.runtime_id,
        persistent_id = ?summary.persistent_id,
        status = ?summary.status,
        "forwards: status"
    );
    // Runtime errors (bind, server reject, auto-start) are async: nobody is
    // waiting on a call, so they get a toast as well as the row note.
    if let ForwardStatus::Error { message } = &summary.status {
        app.toasts.danger(format!("Forward failed — {}", spec_label(&summary.spec)), message.clone());
    }
    apply_status(&mut app.forwards.runtime.entry(connection_id).or_default().rows, summary);
}

// ---------------------------------------------------------------- actions

fn apply(app: &mut App, tab: Option<TabId>, action: Action) {
    let (connection_id, session_id) = match tab.and_then(|t| app.tabs.get(t)) {
        Some(t) => (t.connection_id, Some(t.session.id)),
        None => (None, None),
    };
    match action {
        Action::AddTab => match connection_id {
            Some(cid) => app.forwards.open_dialog(Mode::TabCreate { connection_id: cid, session_id }),
            None => tracing::debug!(?tab, "forwards: add ignored, not connected"),
        },
        Action::AddSaved(sid) => app.forwards.open_dialog(Mode::SavedCreate { session_id: sid }),
        Action::EditSaved(forward) => app.forwards.open_dialog(Mode::SavedEdit { forward }),
        Action::EditTab(existing) => match connection_id {
            Some(cid) => app.forwards.open_dialog(Mode::TabEdit { connection_id: cid, existing }),
            None => tracing::debug!(?tab, "forwards: edit ignored, not connected"),
        },
        Action::Start(id) => {
            let Some(cid) = connection_id else {
                tracing::debug!(?tab, id, "forwards: start ignored, not connected");
                return;
            };
            tracing::debug!(connection_id = cid, id, "forwards: start saved");
            app.core.call(
                "forward_start",
                move |st| async move { api::forwards::forward_start(&st, cid, ForwardStartTarget::Persistent { id }).await },
                move |app, r| match r {
                    Ok(rf) => upsert(&mut app.forwards.runtime.entry(cid).or_default().rows, rf),
                    Err(e) => {
                        tracing::warn!(connection_id = cid, id, "forwards: start failed: {e}");
                        app.toasts.danger("Start failed", e.to_string());
                    }
                },
            );
        }
        Action::Stop(rid) | Action::StopTab(rid) => {
            let Some(cid) = connection_id else {
                tracing::debug!(?tab, rid, "forwards: stop ignored, not connected");
                return;
            };
            let drop_row = matches!(action, Action::StopTab(_));
            tracing::debug!(connection_id = cid, runtime_id = rid, drop_row, "forwards: stop");
            app.core.call(
                "forward_stop",
                move |st| async move { api::forwards::forward_stop(&st, cid, rid).await },
                move |app, r| match r {
                    // A stopped tab-only row would linger with nothing to
                    // stop; Stop removes it (the webview kept it forever).
                    Ok(()) if drop_row => {
                        if let Some(list) = app.forwards.runtime.get_mut(&cid) {
                            list.rows.retain(|x| x.runtime_id != rid);
                        }
                    }
                    Ok(()) => {}
                    Err(e) => {
                        tracing::warn!(connection_id = cid, runtime_id = rid, "forwards: stop failed: {e}");
                        app.toasts.danger("Stop failed", e.to_string());
                    }
                },
            );
        }
        Action::DeleteSaved { session_id, id } => {
            // forward_delete also stops any running runtime of this row.
            tracing::debug!(session = session_id, id, "forwards: delete saved");
            app.core.call(
                "forward_delete",
                move |st| async move { api::forwards::forward_delete(&st, id).await },
                move |app, r| match r {
                    Ok(()) => {
                        if let Some(list) = app.forwards.saved.get_mut(&session_id) {
                            list.rows.retain(|x| x.id != id);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(id, "forwards: delete failed: {e}");
                        app.toasts.danger("Delete failed", e.to_string());
                    }
                },
            );
        }
    }
}

// ---------------------------------------------------------------- pane

const HEADER_H: f32 = 37.0;

/// Paint the pane of `tab` into `rect`.
pub fn pane(app: &mut App, ui: &mut Ui, tab: TabId, rect: Rect) {
    let Some(t) = app.tabs.get(tab) else {
        return;
    };
    let (session_id, connection_id) = (t.session.id, t.connection_id);
    let frame = ui.ctx().cumulative_frame_nr();
    if app.forwards.mounted(ViewKey::Pane(tab), frame) {
        load_saved(app, session_id);
        // Drop what belongs to closed tabs and old connections.
        let live_tabs: Vec<TabId> = app.tabs.tabs.iter().map(|t| t.id).collect();
        let live_conns: Vec<u64> = app.tabs.tabs.iter().filter_map(|t| t.connection_id).collect();
        app.forwards.seen.retain(|k, _| !matches!(k, ViewKey::Pane(id) if !live_tabs.contains(id)));
        app.forwards.runtime.retain(|cid, _| live_conns.contains(cid));
    }
    if let Some(cid) = connection_id
        && !app.forwards.runtime.get(&cid).is_some_and(|l| l.requested)
    {
        load_runtime(app, cid);
    }

    let tk = tokens(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, tk.surface);
    ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, tk.border));
    let mut actions: Vec<Action> = Vec::new();
    let mut menu: Option<Menu<Action>> = None;

    // Header.
    let header = Rect::from_min_size(rect.min, vec2(rect.width() - 1.0, HEADER_H));
    ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, tk.border));
    ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(12.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.label(RichText::new("Forwards").font(font_semibold(size::SM)).color(tk.fg));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let tip = if connection_id.is_some() { "Add forward to this tab" } else { "Connect first to add a forward" };
            if ghost_icon(ui, Some(Icon::Plus), "Add", connection_id.is_some(), Some(tip), "Add forward").clicked() {
                actions.push(Action::AddTab);
            }
        });
    });

    // Rows.
    let body = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), Pos2::new(rect.right() - 1.0, rect.bottom()));
    let saved: &[Forward] = app.forwards.saved.get(&session_id).map_or(&[], |l| &l.rows);
    let runtime: &[RuntimeForwardSummary] = connection_id.and_then(|c| app.forwards.runtime.get(&c)).map_or(&[], |l| &l.rows);
    // The last runtime row of a saved forward wins (a retried start
    // leaves its failed attempt behind).
    let by_saved = |id: i64| runtime.iter().rev().find(|r| r.persistent_id == Some(id));
    let tab_only: Vec<&RuntimeForwardSummary> = runtime.iter().filter(|r| r.persistent_id.is_none()).collect();
    let connected = connection_id.is_some();
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical().id_salt(("forwards-pane", tab)).auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if saved.is_empty() && tab_only.is_empty() {
                egui::Frame::new().inner_margin(16).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("No forwards yet. Add one to tunnel a port through this session.").font(font(size::SM)).color(tk.muted),
                        )
                        .wrap(),
                    );
                });
            }
            for p in saved {
                let rt = by_saved(p.id);
                let label = saved_label(p);
                let running = matches!(rt.map(|r| &r.status), Some(ForwardStatus::Running));
                let mut btns = Vec::with_capacity(3);
                if running {
                    btns.push(IconBtn::new(Icon::Square, format!("Stop {label}"), "Stop").tone(tk.danger));
                } else {
                    let tip = if connected { "Start" } else { "Connect first to start" };
                    btns.push(IconBtn::new(Icon::Play, format!("Start {label}"), tip).tone(tk.success).enabled(connected));
                }
                btns.push(IconBtn::new(Icon::Pencil, format!("Edit {label}"), "Edit"));
                btns.push(IconBtn::new(Icon::Trash, format!("Delete {label}"), "Delete"));
                let stop_or_start = match rt {
                    Some(r) if running => Action::Stop(r.runtime_id),
                    _ => Action::Start(p.id),
                };
                let out = pane_row(ui, Id::unique(("forward-row", tab, "saved", p.id)), parse_kind(&p.kind), &label, row_status(rt), row_note(rt).as_deref(), &btns);
                let delete = Action::DeleteSaved { session_id, id: p.id };
                match out.clicked {
                    Some(0) => actions.push(stop_or_start.clone()),
                    Some(1) => actions.push(Action::EditSaved(p.clone())),
                    Some(2) => actions.push(delete.clone()),
                    _ => {}
                }
                if let Some(pos) = out.menu_at {
                    let toggle = if running {
                        MenuEntry::item("Stop", stop_or_start)
                    } else {
                        MenuEntry::item("Start", stop_or_start).enabled(connected)
                    };
                    menu = Some(Menu::new(
                        pos,
                        vec![
                            MenuEntry::item("Edit", Action::EditSaved(p.clone())),
                            toggle,
                            MenuEntry::Separator,
                            MenuEntry::danger("Delete", delete),
                        ],
                    ));
                }
            }
            for rf in &tab_only {
                let label = spec_label(&rf.spec);
                let btns = [
                    IconBtn::new(Icon::Pencil, format!("Edit {label}"), "Edit"),
                    IconBtn::new(Icon::Square, format!("Stop {label}"), "Stop").tone(tk.danger),
                ];
                let out =
                    pane_row(ui, Id::unique(("forward-row", tab, "tab", rf.runtime_id)), rf.spec.kind, &label, row_status(Some(rf)), row_note(Some(rf)).as_deref(), &btns);
                match out.clicked {
                    Some(0) => actions.push(Action::EditTab((*rf).clone())),
                    Some(1) => actions.push(Action::StopTab(rf.runtime_id)),
                    _ => {}
                }
                if let Some(pos) = out.menu_at {
                    menu = Some(Menu::new(
                        pos,
                        vec![MenuEntry::item("Edit", Action::EditTab((*rf).clone())), MenuEntry::item("Stop", Action::StopTab(rf.runtime_id))],
                    ));
                }
            }
        });
    });

    if let Some(m) = menu {
        app.forwards.menu = Some((tab, m));
    }
    for a in actions {
        apply(app, Some(tab), a);
    }
}

struct RowOut {
    /// Index of the button clicked this frame.
    clicked: Option<usize>,
    /// Right-clicked here: open the context menu.
    menu_at: Option<Pos2>,
}

/// `Row`: status dot, kind icon, mono label, buttons; an error or
/// conflict note below.
fn pane_row(ui: &mut Ui, id: Id, kind: ForwardKind, label: &str, status: RowStatus, note: Option<&str>, btns: &[IconBtn<'_>]) -> RowOut {
    let tk = tokens(ui.ctx());
    let width = ui.available_width();
    let note_color = if status == RowStatus::Conflict { AMBER_500 } else { tk.danger };
    let note_g = note.map(|n| ui.painter().layout(n.to_string(), font(size::XS), note_color, width - 24.0 - 24.0));
    let h = 8.0 + 20.0 + note_g.as_ref().map_or(0.0, |g| 2.0 + g.size().y) + 8.0;
    let (rect, row) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    widgets::a11y(&row, egui::Role::ListItem, label);
    let mut out = RowOut { clicked: None, menu_at: None };
    if row.secondary_clicked() {
        out.menu_at = row.interact_pointer_pos().or_else(|| ui.ctx().pointer_latest_pos());
    }
    if ui.rect_contains_pointer(rect) {
        ui.painter().rect_filled(rect, 0.0, tk.surface2);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, tk.border));
    let line = Rect::from_min_size(Pos2::new(rect.left() + 12.0, rect.top() + 8.0), vec2(width - 24.0, 20.0));
    let dot = match status {
        RowStatus::Running => tk.success,
        RowStatus::Error => tk.danger,
        RowStatus::Conflict => AMBER_500,
        RowStatus::Idle => alpha(tk.muted, 0.6),
    };
    ui.painter().circle_filled(Pos2::new(line.left() + 3.0, line.center().y), 3.0, dot);
    kind_icon(kind).paint(ui, Rect::from_center_size(Pos2::new(line.left() + 20.0, line.center().y), vec2(12.0, 12.0)), 12.0, kind_tone(kind));
    let btn_w = 22.0;
    let btns_left = line.right() - btn_w * btns.len() as f32;
    let text = Rect::from_min_max(Pos2::new(line.left() + 34.0, line.top()), Pos2::new(btns_left - 6.0, line.bottom()));
    widgets::paint_text(ui, text, label, egui::FontId::monospace(size::XS), tk.fg);
    for (i, b) in btns.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(btns_left + btn_w * i as f32 + 1.0, line.top()), vec2(20.0, 20.0));
        if b.show(ui, r, id.with(i)).clicked() {
            out.clicked = Some(i);
        }
    }
    if let Some(g) = note_g {
        ui.painter().galley(Pos2::new(rect.left() + 12.0 + 24.0, line.bottom() + 2.0), g, note_color);
    }
    out
}

// ---------------------------------------------------------------- shared bits

/// An icon button (the webview's `.icon-btn` / `IconBtn`) painted at a
/// given rect, with a screen-reader `label` apart from its `tooltip`.
pub(super) struct IconBtn<'a> {
    icon: Icon,
    icon_size: f32,
    label: String,
    tooltip: &'a str,
    /// Icon colour at rest (muted when `None`).
    tone: Option<Color32>,
    hover_tone: Option<Color32>,
    hover_bg: Option<Color32>,
    enabled: bool,
    /// When false it is painted only while hovered or focused (the
    /// webview's `opacity-0 group-hover:opacity-100 focus:opacity-100`).
    visible: bool,
}

impl<'a> IconBtn<'a> {
    pub(super) fn new(icon: Icon, label: String, tooltip: &'a str) -> Self {
        Self { icon, icon_size: 12.0, label, tooltip, tone: None, hover_tone: None, hover_bg: None, enabled: true, visible: true }
    }

    pub(super) fn tone(mut self, c: Color32) -> Self {
        self.tone = Some(c);
        self
    }

    pub(super) fn hover(mut self, tone: Color32, bg: Color32) -> Self {
        self.hover_tone = Some(tone);
        self.hover_bg = Some(bg);
        self
    }

    pub(super) fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub(super) fn visible(mut self, on: bool) -> Self {
        self.visible = on;
        self
    }

    pub(super) fn show(&self, ui: &mut Ui, rect: Rect, id: Id) -> Response {
        let tk = tokens(ui.ctx());
        let enabled = self.enabled;
        let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &self.label));
        let hovered = enabled && resp.hovered();
        if (self.visible || resp.has_focus() || hovered) && ui.is_rect_visible(rect) {
            if hovered {
                ui.painter().rect_filled(rect, 2.0, self.hover_bg.unwrap_or(tk.surface2));
            }
            let mut color = if hovered { self.hover_tone.or(self.tone).unwrap_or(tk.fg) } else { self.tone.unwrap_or(tk.muted) };
            if !enabled {
                color = alpha(color, 0.4);
            }
            self.icon.paint(ui, rect, self.icon_size, color);
            widgets::focus_ring(ui, &resp, rect);
        }
        let resp = resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed });
        resp.on_hover_text(self.tooltip)
    }
}

/// `.btn-ghost` with an optional leading icon: bordered, `text-xs`.
/// `a11y_label` names it to screen readers when the visible text is
/// ambiguous ("Add" in several groups).
pub(super) fn ghost_icon(ui: &mut Ui, icon: Option<Icon>, text: &str, enabled: bool, tooltip: Option<&str>, a11y_label: &str) -> Response {
    let tk = tokens(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font(size::XS), tk.fg);
    let icon_w = if icon.is_some() { 12.0 + 6.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 24.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, a11y_label));
    if ui.is_rect_visible(rect) {
        if enabled && resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, tk.surface2);
        }
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, tk.border), StrokeKind::Inside);
        let color = if enabled { tk.fg } else { alpha(tk.fg, 0.5) };
        let mut x = rect.left() + 12.0;
        if let Some(icon) = icon {
            icon.paint(ui, Rect::from_center_size(Pos2::new(x + 6.0, rect.center().y), vec2(12.0, 12.0)), 12.0, color);
            x += icon_w;
        }
        ui.painter().galley_with_override_text_color(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, color);
        widgets::focus_ring(ui, &resp, rect);
    }
    let resp = resp.on_hover_cursor(if enabled { CursorIcon::PointingHand } else { CursorIcon::NotAllowed });
    match tooltip {
        Some(t) => resp.on_hover_text(t),
        None => resp,
    }
}

// ---------------------------------------------------------------- session tab

/// The session dialog's Forwards tab (`ForwardsConfigPane`): the saved
/// forwards of `session_id` with add / edit / delete. `None` = a session
/// not saved yet ("Save the session first…").
pub fn session_tab(app: &mut App, ui: &mut Ui, session_id: Option<i64>) {
    let tk = tokens(ui.ctx());
    let Some(sid) = session_id else {
        ui.label(RichText::new("Save the session first to configure forwards.").font(font(size::SM)).color(tk.muted));
        return;
    };
    if app.forwards.mounted(ViewKey::SessionTab(sid), ui.ctx().cumulative_frame_nr()) {
        load_saved(app, sid);
    }
    let mut actions: Vec<Action> = Vec::new();
    let list = app.forwards.saved.get(&sid);
    let rows: &[Forward] = list.map_or(&[], |l| &l.rows);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if rows.is_empty() {
            let text = if list.is_some_and(|l| l.loaded) { "No forwards configured for this session." } else { "Loading…" };
            ui.label(RichText::new(text).font(font(size::SM)).color(tk.muted));
        }
        for r in rows {
            match config_row(ui, r) {
                Some(0) => actions.push(Action::EditSaved(r.clone())),
                Some(1) => actions.push(Action::DeleteSaved { session_id: sid, id: r.id }),
                _ => {}
            }
        }
        if ghost_icon(ui, Some(Icon::Plus), "Add forward", true, None, "Add forward").clicked() {
            actions.push(Action::AddSaved(sid));
        }
    });
    for a in actions {
        apply(app, None, a);
    }
}

/// One saved forward in the session tab: kind icon, label, `auto` badge,
/// Edit / Delete. Returns the index of the button clicked.
fn config_row(ui: &mut Ui, r: &Forward) -> Option<usize> {
    let tk = tokens(ui.ctx());
    let label = saved_label(r);
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    widgets::a11y(&row, egui::Role::ListItem, &label);
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, tk.border), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(8.0, 6.0));
    kind_icon(parse_kind(&r.kind)).paint(ui, Rect::from_center_size(Pos2::new(inner.left() + 6.0, inner.center().y), vec2(12.0, 12.0)), 12.0, tk.muted);
    let btns_left = inner.right() - 44.0;
    let mut text_right = btns_left - 8.0;
    if r.auto_start == 1 {
        let g = ui.painter().layout_no_wrap("AUTO".into(), font(size::XS - 1.0), alpha(tk.success, 0.8));
        let badge = Rect::from_min_size(Pos2::new(text_right - g.size().x - 8.0, inner.center().y - 8.0), vec2(g.size().x + 8.0, 16.0));
        ui.painter().rect_stroke(badge, 2.0, Stroke::new(1.0, alpha(tk.success, 0.3)), StrokeKind::Inside);
        ui.painter().galley(badge.center() - g.size() / 2.0, g, alpha(tk.success, 0.8));
        text_right = badge.left() - 8.0;
    }
    let text = Rect::from_min_max(Pos2::new(inner.left() + 20.0, inner.top()), Pos2::new(text_right, inner.bottom()));
    widgets::paint_text(ui, text, &label, egui::FontId::monospace(size::XS), tk.fg);
    let mut clicked = None;
    for (i, (icon, verb)) in [(Icon::Pencil, "Edit"), (Icon::Trash, "Delete")].into_iter().enumerate() {
        let b = Rect::from_min_size(Pos2::new(btns_left + 22.0 * i as f32 + 2.0, inner.center().y - 10.0), vec2(20.0, 20.0));
        let id = Id::unique(("forward-config-btn", r.id, i));
        if IconBtn::new(icon, format!("{verb} {label}"), verb).show(ui, b, id).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

// ---------------------------------------------------------------- dialog

/// `ForwardDialog`'s modes.
enum Mode {
    /// Session tab "Add forward".
    SavedCreate { session_id: i64 },
    /// Edit a saved forward (a running one is restarted by the core).
    SavedEdit { forward: Forward },
    /// The pane's "Add": start on this tab, optionally also save it to the
    /// session.
    TabCreate { connection_id: u64, session_id: Option<i64> },
    /// Edit a running tab-only forward (atomic replace).
    TabEdit { connection_id: u64, existing: RuntimeForwardSummary },
}

impl Mode {
    fn name(&self) -> &'static str {
        match self {
            Mode::SavedCreate { .. } => "persistent-create",
            Mode::SavedEdit { .. } => "persistent-edit",
            Mode::TabCreate { .. } => "ephemeral-create",
            Mode::TabEdit { .. } => "ephemeral-edit",
        }
    }

    fn is_saved(&self) -> bool {
        matches!(self, Mode::SavedCreate { .. } | Mode::SavedEdit { .. })
    }

    /// The session a tab forward may also be saved to.
    fn persist_session(&self) -> Option<i64> {
        match self {
            Mode::TabCreate { session_id: Some(s), .. } if *s > 0 => Some(*s),
            _ => None,
        }
    }
}

pub(crate) struct ForwardDialog {
    mode: Mode,
    /// Identifies this dialog to its pending save.
    token: u64,
    name: String,
    kind: ForwardKind,
    bind_addr: String,
    bind_port: String,
    dest_addr: String,
    dest_port: String,
    auto_start: bool,
    persist: bool,
    busy: bool,
    err: Option<String>,
}

/// The parsed, validated form.
#[derive(Debug, PartialEq)]
struct Valid {
    bind_port: u16,
    dest_addr: String,
    dest_port: u16,
}

fn port_text(p: i64) -> String {
    if p > 0 { p.to_string() } else { String::new() }
}

/// A port field's value; blank or garbage is 0 (fails the range check).
fn parse_port(s: &str) -> i64 {
    s.trim().parse::<i64>().unwrap_or(0)
}

impl ForwardDialog {
    fn new(mode: Mode, token: u64) -> Self {
        let mut d = Self {
            mode,
            token,
            name: String::new(),
            kind: ForwardKind::Local,
            bind_addr: "127.0.0.1".into(),
            bind_port: String::new(),
            dest_addr: String::new(),
            dest_port: String::new(),
            auto_start: true,
            persist: false,
            busy: false,
            err: None,
        };
        match &d.mode {
            Mode::SavedEdit { forward: f } => {
                d.name = f.name.clone();
                d.kind = parse_kind(&f.kind);
                d.bind_addr = f.bind_addr.clone();
                d.bind_port = port_text(f.bind_port);
                d.dest_addr = f.dest_addr.clone();
                d.dest_port = port_text(f.dest_port);
                d.auto_start = f.auto_start == 1;
            }
            Mode::TabEdit { existing, .. } => {
                let s = &existing.spec;
                d.name = s.name.clone();
                d.kind = s.kind;
                d.bind_addr = s.bind_addr.clone();
                d.bind_port = port_text(s.bind_port.into());
                d.dest_addr = s.dest_addr.clone();
                d.dest_port = port_text(s.dest_port.into());
                d.auto_start = false;
            }
            Mode::SavedCreate { .. } | Mode::TabCreate { .. } => {}
        }
        d
    }

    fn non_loopback(&self) -> bool {
        let a = self.bind_addr.trim();
        !a.is_empty() && !["127.0.0.1", "localhost", "::1"].contains(&a)
    }

    fn privileged(&self) -> bool {
        (1..1024).contains(&parse_port(&self.bind_port))
    }

    /// `submit`'s checks, in the webview's order and wording.
    fn validate(&self) -> Result<Valid, &'static str> {
        if self.bind_addr.trim().is_empty() {
            return Err("Bind address is required");
        }
        let bind_port = parse_port(&self.bind_port);
        if !(1..=65535).contains(&bind_port) {
            return Err("Bind port must be 1–65535");
        }
        if self.kind == ForwardKind::Dynamic {
            return Ok(Valid { bind_port: bind_port as u16, dest_addr: String::new(), dest_port: 0 });
        }
        if self.dest_addr.trim().is_empty() {
            return Err("Destination host is required");
        }
        let dest_port = parse_port(&self.dest_port);
        if !(1..=65535).contains(&dest_port) {
            return Err("Destination port must be 1–65535");
        }
        Ok(Valid { bind_port: bind_port as u16, dest_addr: self.dest_addr.clone(), dest_port: dest_port as u16 })
    }

    fn input(&self, v: &Valid) -> ForwardInput {
        ForwardInput {
            name: self.name.clone(),
            kind: kind_str(self.kind).into(),
            bind_addr: self.bind_addr.clone(),
            bind_port: v.bind_port.into(),
            dest_addr: v.dest_addr.clone(),
            dest_port: v.dest_port.into(),
            auto_start: i64::from(self.auto_start),
        }
    }

    fn spec(&self, v: &Valid) -> ForwardSpec {
        ForwardSpec {
            name: self.name.clone(),
            kind: self.kind,
            bind_addr: self.bind_addr.clone(),
            bind_port: v.bind_port,
            dest_addr: v.dest_addr.clone(),
            dest_port: v.dest_port,
        }
    }
}

/// What a save produced, applied to the lists.
#[derive(Default)]
struct Saved {
    /// A saved row, created or updated.
    persistent: Option<Forward>,
    /// A runtime forward started on `connection`.
    runtime: Option<(u64, RuntimeForwardSummary)>,
    /// The runtime row a replace superseded.
    replaced: Option<u64>,
}

fn submit(app: &mut App) {
    let Some(d) = app.forwards.dialog.as_mut() else {
        return;
    };
    if d.busy {
        return;
    }
    d.err = None;
    let v = match d.validate() {
        Ok(v) => v,
        Err(msg) => {
            tracing::debug!(msg, "forwards: dialog validation");
            d.err = Some(msg.into());
            return;
        }
    };
    d.busy = true;
    let token = d.token;
    tracing::debug!(mode = d.mode.name(), kind = kind_str(d.kind), persist = d.persist, "forwards: save");
    let input = d.input(&v);
    let spec = d.spec(&v);
    match &d.mode {
        Mode::SavedCreate { session_id } => {
            let sid = *session_id;
            app.core.call(
                "forward_create",
                move |st| async move { api::forwards::forward_create(&st, sid, input).await },
                move |app, r| finish(app, token, r.map(|f| Saved { persistent: Some(f), ..Saved::default() }).map_err(|e| e.to_string())),
            );
        }
        Mode::SavedEdit { forward } => {
            let id = forward.id;
            app.core.call(
                "forward_update",
                move |st| async move { api::forwards::forward_update(&st, id, input).await },
                move |app, r| finish(app, token, r.map(|f| Saved { persistent: Some(f), ..Saved::default() }).map_err(|e| e.to_string())),
            );
        }
        Mode::TabCreate { connection_id, .. } if d.persist && d.mode.persist_session().is_some() => {
            // Save to the session and start it here: live now, and back
            // on the next connect.
            let (cid, sid) = (*connection_id, d.mode.persist_session().unwrap_or_default());
            app.core.call(
                "forward_create+start",
                move |st| async move {
                    let created = api::forwards::forward_create(&st, sid, input).await?;
                    let started = api::forwards::forward_start(&st, cid, ForwardStartTarget::Persistent { id: created.id }).await;
                    Ok::<_, ezssh_core::AppError>((created, started))
                },
                move |app, r| match r {
                    Ok((created, Ok(rf))) => finish(app, token, Ok(Saved { persistent: Some(created), runtime: Some((cid, rf)), replaced: None })),
                    Ok((created, Err(e))) => {
                        // Saved but not started: keep the row (its error
                        // shows in the pane through the status event) and
                        // close, so a second Save cannot duplicate it.
                        tracing::warn!(connection_id = cid, id = created.id, "forwards: saved but start failed: {e}");
                        app.toasts.danger("Forward saved, start failed", e.to_string());
                        finish(app, token, Ok(Saved { persistent: Some(created), ..Saved::default() }));
                    }
                    Err(e) => finish(app, token, Err(e.to_string())),
                },
            );
        }
        Mode::TabCreate { connection_id, .. } => {
            let cid = *connection_id;
            app.core.call(
                "forward_start",
                move |st| async move { api::forwards::forward_start(&st, cid, ForwardStartTarget::Ephemeral { spec }).await },
                move |app, r| finish(app, token, r.map(|rf| Saved { runtime: Some((cid, rf)), ..Saved::default() }).map_err(|e| e.to_string())),
            );
        }
        Mode::TabEdit { connection_id, existing } => {
            // Replace atomically on the core (stop, wait for the bind to be
            // released, start) so re-using the address cannot race.
            let (cid, old) = (*connection_id, existing.runtime_id);
            app.core.call(
                "forward_replace",
                move |st| async move { api::forwards::forward_replace(&st, cid, old, spec).await },
                move |app, r| {
                    finish(app, token, r.map(|rf| Saved { runtime: Some((cid, rf)), replaced: Some(old), ..Saved::default() }).map_err(|e| e.to_string()))
                },
            );
        }
    }
}

/// A save finished: update the lists; close the dialog it came from, or
/// show the error in it.
fn finish(app: &mut App, token: u64, r: Result<Saved, String>) {
    let ours = app.forwards.dialog.as_ref().is_some_and(|d| d.token == token);
    match r {
        Ok(s) => {
            tracing::debug!(token, saved = s.persistent.is_some(), started = s.runtime.is_some(), "forwards: saved");
            if let Some(f) = s.persistent {
                let list = app.forwards.saved.entry(f.session_id).or_default();
                match list.rows.iter().position(|x| x.id == f.id) {
                    Some(i) => list.rows[i] = f,
                    None => list.rows.push(f),
                }
            }
            if let Some((cid, rf)) = s.runtime {
                let rows = &mut app.forwards.runtime.entry(cid).or_default().rows;
                if let Some(old) = s.replaced {
                    rows.retain(|x| x.runtime_id != old);
                }
                upsert(rows, rf);
            }
            if ours {
                app.forwards.dialog = None;
            }
        }
        Err(msg) => {
            tracing::warn!(token, "forwards: save failed: {msg}");
            app.toasts.danger("Forward save failed", msg.clone());
            if ours && let Some(d) = app.forwards.dialog.as_mut() {
                d.busy = false;
                d.err = Some(msg);
            }
        }
    }
}

const KIND_TILES: [(ForwardKind, &str, &str); 3] = [
    (ForwardKind::Local, "Local (-L)", "localhost → remote target"),
    (ForwardKind::Remote, "Remote (-R)", "remote bind → local target"),
    (ForwardKind::Dynamic, "Dynamic (-D)", "local SOCKS5 proxy"),
];

/// A labelled field of `width` in a row of fields.
fn field_at<R>(ui: &mut Ui, width: f32, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        form::field(ui, label, None, add)
    })
    .inner
}

/// The pane's context menu and the add / edit dialog.
pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    if let Some((tab, menu)) = app.forwards.menu.as_mut() {
        let tab = *tab;
        match menu.show(ctx, Id::unique("forwards-menu")) {
            MenuResult::Open => {}
            MenuResult::Closed => app.forwards.menu = None,
            MenuResult::Chosen(a) => {
                app.forwards.menu = None;
                apply(app, Some(tab), a);
            }
        }
    }

    let Some(d) = app.forwards.dialog.as_mut() else {
        return;
    };
    let tk = tokens(ctx);
    let busy = d.busy;
    let (mut save, mut cancel) = (false, false);
    let title = if d.mode.is_saved() { "Forward (session config)" } else { "Forward (this tab)" };
    let out = widgets::dialog(
        ctx,
        Id::unique("forward-dialog"),
        560.0,
        |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new(title).font(font_semibold(size::BASE)).color(tk.fg));
            // The header strip's border, across the whole card.
            let r = ui.available_rect_before_wrap();
            ui.painter().hline((r.left() - 16.0)..=(r.right() + 16.0), r.top() - 3.0, Stroke::new(1.0, tk.border));
            ui.add_space(8.0);
            let w = ui.available_width();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let tile_w = (w - 16.0) / 3.0;
                for (kind, label, hint) in KIND_TILES {
                    if form::option_card(ui, d.kind == kind, kind_icon(kind), label, hint, tile_w).clicked() {
                        d.kind = kind;
                    }
                }
            });
            form::field(ui, "Name (optional)", None, |ui| {
                form::input(ui, &mut d.name, InputOpts::new("e.g. Postgres dev")).accessible_name("Name (optional)");
            });
            let first_w = w - 120.0 - 8.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                field_at(ui, first_w, "Bind address", |ui| {
                    form::input(ui, &mut d.bind_addr, InputOpts::new("127.0.0.1").mono().width(first_w)).accessible_name("Bind address");
                });
                field_at(ui, 120.0, "Bind port", |ui| {
                    form::input(ui, &mut d.bind_port, InputOpts::new("").width(120.0)).accessible_name("Bind port");
                });
            });
            if d.non_loopback() {
                form::note(ui, Icon::AlertTriangle, tk.warning, "This forward will be reachable from other machines on your network.");
            }
            if d.privileged() {
                form::hint(ui, "Ports below 1024 require admin/root on most systems.");
            }
            if d.kind != ForwardKind::Dynamic {
                let hint = if d.kind == ForwardKind::Remote { "localhost" } else { "db.internal" };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    field_at(ui, first_w, "Destination host", |ui| {
                        form::input(ui, &mut d.dest_addr, InputOpts::new(hint).mono().width(first_w)).accessible_name("Destination host");
                    });
                    field_at(ui, 120.0, "Destination port", |ui| {
                        form::input(ui, &mut d.dest_port, InputOpts::new("").width(120.0)).accessible_name("Destination port");
                    });
                });
            }
            if d.mode.persist_session().is_some() {
                form::checkbox(ui, &mut d.persist, "Save to this session (persists across reconnects)", None);
            }
            if d.mode.is_saved() || d.persist {
                form::checkbox(ui, &mut d.auto_start, "Auto-start when the session connects", None);
            }
            if let Some(err) = &d.err {
                ui.label(RichText::new(err).font(font(size::SM)).color(tk.danger));
            }
        },
        Some(&mut |ui: &mut Ui| {
            if widgets::primary(ui, if busy { "Saving…" } else { "Save" }, Tone::Accent, !busy, size::SM).clicked() {
                save = true;
            }
            if widgets::ghost(ui, "Cancel", size::SM).clicked() {
                cancel = true;
            }
        }),
    );
    if cancel || out.dismissed {
        tracing::debug!("forwards: dialog closed");
        app.forwards.dialog = None;
    } else if save {
        submit(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: ForwardKind, name: &str) -> ForwardSpec {
        ForwardSpec { name: name.into(), kind, bind_addr: "127.0.0.1".into(), bind_port: 8080, dest_addr: "db".into(), dest_port: 5432 }
    }

    fn rf(id: u64, persistent: Option<i64>, status: ForwardStatus) -> RuntimeForwardSummary {
        RuntimeForwardSummary { runtime_id: id, persistent_id: persistent, spec: spec(ForwardKind::Local, ""), status }
    }

    #[test]
    fn labels_match_lib_forwards() {
        assert_eq!(spec_label(&spec(ForwardKind::Local, "")), "127.0.0.1:8080 → db:5432");
        assert_eq!(spec_label(&spec(ForwardKind::Dynamic, "")), "SOCKS5 @ 127.0.0.1:8080");
        assert_eq!(spec_label(&spec(ForwardKind::Remote, "pg")), "pg");
    }

    #[test]
    fn status_events_upsert_and_stop() {
        let mut rows = Vec::new();
        apply_status(&mut rows, rf(1, Some(7), ForwardStatus::Running));
        apply_status(&mut rows, rf(1, Some(7), ForwardStatus::Running));
        apply_status(&mut rows, rf(2, None, ForwardStatus::Conflict { message: "taken".into() }));
        assert_eq!(rows.len(), 2, "same runtime id coalesces");
        assert_eq!(row_status(rows.iter().find(|r| r.runtime_id == 2)), RowStatus::Conflict);
        assert_eq!(row_note(rows.iter().find(|r| r.runtime_id == 2)).as_deref(), Some("taken"));
        // A saved forward's runtime row goes; a tab-only one stays, idle.
        apply_status(&mut rows, rf(1, Some(7), ForwardStatus::Stopped));
        apply_status(&mut rows, rf(2, None, ForwardStatus::Stopped));
        assert_eq!(rows.len(), 1);
        assert_eq!(row_status(rows.first()), RowStatus::Idle);
        // Stopped for an unknown tab-only id adds nothing.
        apply_status(&mut rows, rf(9, None, ForwardStatus::Stopped));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn dialog_validation_order_and_warnings() {
        let mut d = ForwardDialog::new(Mode::SavedCreate { session_id: 1 }, 1);
        assert_eq!(d.validate(), Err("Bind port must be 1–65535"));
        d.bind_addr = "  ".into();
        assert_eq!(d.validate(), Err("Bind address is required"));
        d.bind_addr = "0.0.0.0".into();
        assert!(d.non_loopback());
        d.bind_port = "80".into();
        assert!(d.privileged());
        assert_eq!(d.validate(), Err("Destination host is required"));
        d.dest_addr = "db".into();
        d.dest_port = "70000".into();
        assert_eq!(d.validate(), Err("Destination port must be 1–65535"));
        d.dest_port = "5432".into();
        assert_eq!(d.validate(), Ok(Valid { bind_port: 80, dest_addr: "db".into(), dest_port: 5432 }));
        // Dynamic ignores (and clears) the destination.
        d.kind = ForwardKind::Dynamic;
        d.dest_port = "x".into();
        let v = d.validate().expect("dynamic is valid");
        assert_eq!((v.dest_addr.as_str(), v.dest_port), ("", 0));
        assert_eq!(d.input(&v).kind, "dynamic");
        d.bind_addr = "localhost".into();
        assert!(!d.non_loopback());
    }

    #[test]
    fn persist_offered_only_for_a_saved_session() {
        assert_eq!(Mode::TabCreate { connection_id: 1, session_id: Some(4) }.persist_session(), Some(4));
        assert_eq!(Mode::TabCreate { connection_id: 1, session_id: Some(0) }.persist_session(), None);
        assert_eq!(Mode::SavedCreate { session_id: 4 }.persist_session(), None);
    }
}
