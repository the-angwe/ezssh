//! Open tabs and the workspace layout around them: a port of
//! `ui/lib/tabs-store.ts` plus the cascade geometry of `use-mdi-drag.ts` /
//! `use-mdi-resize.ts`. Pure state; the terminals live in the app, keyed by
//! [`TabId`].

use std::collections::{HashMap, HashSet};

use ezssh_core::db::sessions::Session;

use crate::layout::TileGrid;

pub type TabId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStatus {
    Connecting,
    Connected,
    Closed,
    Error,
}

impl TabStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Closed => "closed",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Tabs,
    TileH,
    TileV,
    TileGrid,
    Cascade,
    Auto,
}

impl ViewMode {
    pub const ALL: [ViewMode; 6] =
        [ViewMode::Tabs, ViewMode::TileH, ViewMode::TileV, ViewMode::TileGrid, ViewMode::Cascade, ViewMode::Auto];

    /// The id the webview stored (`ezterm.viewMode`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Tabs => "tabs",
            Self::TileH => "tile-h",
            Self::TileV => "tile-v",
            Self::TileGrid => "tile-grid",
            Self::Cascade => "cascade",
            Self::Auto => "auto",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tabs => "Tabs view",
            Self::TileH => "Tile horizontal",
            Self::TileV => "Tile vertical",
            Self::TileGrid => "Tile grid…",
            Self::Cascade => "Cascade",
            Self::Auto => "Auto-arrange",
        }
    }

    /// Ctrl+Shift+W order.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Floating-frame geometry of one tab in cascade mode, relative to the
/// MDI area's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Higher = on top.
    pub z: u64,
    pub maximized: bool,
    pub prev: Option<(f32, f32, f32, f32)>,
}

pub const CASCADE_MIN_W: f32 = 200.0;
pub const CASCADE_MIN_H: f32 = 120.0;
const STAIRCASE_STEP: f32 = 30.0;
const DEFAULT_FRAME_W: f32 = 640.0;
const DEFAULT_FRAME_H: f32 = 400.0;

impl CascadeGeom {
    fn staircase(count: usize, z: u64) -> Self {
        let offset = (count % 10) as f32 * STAIRCASE_STEP;
        Self { x: offset, y: offset, w: DEFAULT_FRAME_W, h: DEFAULT_FRAME_H, z, maximized: false, prev: None }
    }

    /// Where the frame is drawn in an area of `area_w` × `area_h`: stored
    /// geometry clamped so it never leaks off-screen after a window
    /// resize. Display only; nothing is written back (as in `tab-slot.tsx`).
    pub fn displayed(&self, area_w: f32, area_h: f32) -> (f32, f32, f32, f32) {
        let w = self.w.min(CASCADE_MIN_W.max(area_w));
        let h = self.h.min(CASCADE_MIN_H.max(area_h));
        let x = self.x.min(area_w - w).max(0.0);
        let y = self.y.min(area_h - h).max(0.0);
        (x, y, w, h)
    }

    /// Title-bar drag: `start` moved by (dx, dy), kept inside the area.
    pub fn dragged(start: (f32, f32, f32, f32), dx: f32, dy: f32, area_w: f32, area_h: f32) -> (f32, f32) {
        let (sx, sy, sw, sh) = start;
        ((sx + dx).min(area_w - sw).max(0.0), (sy + dy).min(area_h - sh).max(0.0))
    }

    /// Edge / corner resize from `start` by (dx, dy). Returns (x, y, w, h).
    pub fn resized(start: (f32, f32, f32, f32), edge: Edge, dx: f32, dy: f32, area_w: f32, area_h: f32) -> (f32, f32, f32, f32) {
        let (sx, sy, sw, sh) = start;
        let (mut x, mut y, mut w, mut h) = start;
        if edge.east() {
            w = (sw + dx).min(area_w - sx).max(CASCADE_MIN_W);
        }
        if edge.west() {
            let nx = (sx + dx).min(sx + sw - CASCADE_MIN_W).max(0.0);
            w = sw + (sx - nx);
            x = nx;
        }
        if edge.south() {
            h = (sh + dy).min(area_h - sy).max(CASCADE_MIN_H);
        }
        if edge.north() {
            let ny = (sy + dy).min(sy + sh - CASCADE_MIN_H).max(0.0);
            h = sh + (sy - ny);
            y = ny;
        }
        (x, y, w, h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Edge {
    pub const ALL: [Edge; 8] = [Edge::N, Edge::S, Edge::E, Edge::W, Edge::NE, Edge::NW, Edge::SE, Edge::SW];

    fn north(self) -> bool {
        matches!(self, Edge::N | Edge::NE | Edge::NW)
    }
    fn south(self) -> bool {
        matches!(self, Edge::S | Edge::SE | Edge::SW)
    }
    fn east(self) -> bool {
        matches!(self, Edge::E | Edge::NE | Edge::SE)
    }
    fn west(self) -> bool {
        matches!(self, Edge::W | Edge::NW | Edge::SW)
    }
}

/// One open tab: the session it was opened from and its connection state.
#[derive(Clone, Debug)]
pub struct Tab {
    pub id: TabId,
    pub session: Session,
    pub connection_id: Option<u64>,
    pub status: TabStatus,
    /// Error text (status Error) or close reason (status Closed).
    pub message: Option<String>,
    pub sftp_open: bool,
    pub forwards_open: bool,
    pub cwd: String,
    /// The font family and tab colour this tab shows. They start as the
    /// session's and differ from `session` after "Apply to tab" in the
    /// Appearance dialog (size, cursor and scrollback live in the terminal).
    pub font_family: String,
    pub color: Option<String>,
}

impl Tab {
    /// `user@host[:port]`, the overlay / status-bar summary.
    pub fn summary(&self) -> String {
        session_summary(&self.session)
    }

    /// SSH and WSL tabs have a file pane (SFTP / the WSL share).
    pub fn has_file_pane(&self) -> bool {
        matches!(self.session.session_kind.as_str(), "ssh" | "wsl")
    }

    pub fn is_ssh(&self) -> bool {
        self.session.session_kind == "ssh"
    }

    /// Strip / title label for `appearance.tabLabel`. Local shells always
    /// show their name (their `username` column holds a start directory).
    pub fn label(&self, userhost: bool) -> String {
        let s = &self.session;
        if userhost && s.session_kind != "local" && !s.username.is_empty() {
            format!("{}@{}", s.username, s.host)
        } else {
            s.name.clone()
        }
    }
}

/// One-line description of where a session goes: `user@host[:port]` for
/// SSH, `user@distro` (or the distro) for WSL, the program for local
/// shells. The webview formatted every kind as SSH, which showed local
/// shells as `@/bin/sh:0`.
pub fn session_summary(s: &Session) -> String {
    match s.session_kind.as_str() {
        "local" => s.host.clone(),
        "wsl" if s.username.is_empty() => s.host.clone(),
        "wsl" => format!("{}@{}", s.username, s.host),
        _ if s.port != 22 => format!("{}@{}:{}", s.username, s.host, s.port),
        _ => format!("{}@{}", s.username, s.host),
    }
}

#[derive(Debug, Default)]
pub struct Tabs {
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    pub sidebar_collapsed: bool,
    /// Tab awaiting the "close a connected tab?" confirmation.
    pub pending_close: Option<TabId>,
    pub view_mode: ViewMode,
    pub tile_grid: TileGrid,
    pub cascade: HashMap<TabId, CascadeGeom>,
    pub minimized: HashSet<TabId>,
    next_z: u64,
    next_id: TabId,
}

impl Tabs {
    pub fn new(view_mode: ViewMode, tile_grid: TileGrid) -> Self {
        Self { view_mode, tile_grid, next_z: 1, next_id: 1, ..Self::default() }
    }

    pub fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn by_connection(&self, connection_id: u64) -> Option<TabId> {
        self.tabs.iter().find(|t| t.connection_id == Some(connection_id)).map(|t| t.id)
    }

    /// Open a tab for `session`; it becomes active and the sidebar collapses
    /// (MobaXterm: the terminal gets the room).
    pub fn open(&mut self, session: Session) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        self.next_z += 1;
        self.cascade.insert(id, CascadeGeom::staircase(self.cascade.len(), self.next_z));
        self.tabs.push(Tab {
            id,
            font_family: session.font_family.clone(),
            color: session.color.clone(),
            session,
            connection_id: None,
            status: TabStatus::Connecting,
            message: None,
            sftp_open: false,
            forwards_open: false,
            cwd: "/".into(),
        });
        self.active = Some(id);
        self.sidebar_collapsed = true;
        id
    }

    pub fn set_status(&mut self, id: TabId, status: TabStatus, message: Option<String>) {
        if let Some(t) = self.get_mut(id) {
            tracing::debug!(tab = id, from = t.status.label(), to = status.label(), ?message, "tabs.status");
            t.status = status;
            t.message = message;
        }
    }

    /// Remove a tab. The last tab becomes active if the closed one was.
    pub fn close(&mut self, id: TabId) {
        self.tabs.retain(|t| t.id != id);
        if self.active == Some(id) {
            self.active = self.tabs.last().map(|t| t.id);
        }
        self.cascade.remove(&id);
        self.minimized.remove(&id);
        if self.pending_close == Some(id) {
            self.pending_close = None;
        }
    }

    /// Close, or park in `pending_close` when the tab is connected and
    /// `confirm_live` (`sessions.confirmCloseLive`) is on. Returns true
    /// when the tab was closed right away.
    pub fn request_close(&mut self, id: TabId, confirm_live: bool) -> bool {
        match self.get(id) {
            Some(t) if t.status == TabStatus::Connected && confirm_live => {
                self.pending_close = Some(id);
                false
            }
            Some(_) => {
                self.close(id);
                true
            }
            None => false,
        }
    }

    /// Move the tab at `from` so it lands before the tab now at `to`
    /// (`to == len` appends). Same contract as the store's `reorder`.
    pub fn reorder(&mut self, from: usize, to: usize) {
        let n = self.tabs.len();
        if from == to || from >= n || to > n {
            return;
        }
        let moved = self.tabs.remove(from);
        let insert_at = if to > from { to - 1 } else { to };
        self.tabs.insert(insert_at, moved);
    }

    pub fn any_connected(&self) -> bool {
        self.tabs.iter().any(|t| t.status == TabStatus::Connected)
    }

    pub fn is_minimized(&self, id: TabId) -> bool {
        self.minimized.contains(&id)
    }

    /// Tabs that get a cell in tile modes / a frame in cascade.
    pub fn visible(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| !self.minimized.contains(&t.id))
    }

    /// Grid for tile-grid / auto: (rows, cols). Auto picks the squarest
    /// grid for the visible tab count.
    pub fn grid_dims(&self) -> (usize, usize) {
        match self.view_mode {
            ViewMode::Auto => {
                let n = self.visible().count().max(1);
                let cols = (n as f64).sqrt().ceil() as usize;
                (n.div_ceil(cols), cols)
            }
            _ => (self.tile_grid.rows as usize, self.tile_grid.cols as usize),
        }
    }

    pub fn bring_to_front(&mut self, id: TabId) {
        if let Some(g) = self.cascade.get_mut(&id) {
            self.next_z += 1;
            g.z = self.next_z;
        }
    }

    pub fn minimize(&mut self, id: TabId) {
        self.minimized.insert(id);
    }

    pub fn restore(&mut self, id: TabId) {
        self.minimized.remove(&id);
        self.bring_to_front(id);
    }

    pub fn toggle_maximize(&mut self, id: TabId, area_w: f32, area_h: f32) {
        let Some(g) = self.cascade.get_mut(&id) else {
            return;
        };
        if g.maximized
            && let Some((x, y, w, h)) = g.prev.take()
        {
            *g = CascadeGeom { x, y, w, h, maximized: false, ..*g };
        } else {
            g.prev = Some((g.x, g.y, g.w, g.h));
            g.x = 0.0;
            g.y = 0.0;
            g.w = area_w;
            g.h = area_h;
            g.maximized = true;
        }
    }

    /// Cascade frames bottom to top.
    pub fn z_order(&self) -> Vec<TabId> {
        let mut ids: Vec<(u64, TabId)> =
            self.visible().map(|t| (self.cascade.get(&t.id).map_or(0, |g| g.z), t.id)).collect();
        ids.sort();
        ids.into_iter().map(|(_, id)| id).collect()
    }
}

#[cfg(test)]
pub(crate) fn test_session(id: i64, name: &str, kind: &str) -> Session {
    Session {
        id,
        folder_id: None,
        name: name.into(),
        host: "example.org".into(),
        port: 22,
        username: "root".into(),
        auth_type: "agent".into(),
        credential_id: None,
        key_passphrase_credential_id: None,
        color: None,
        sort: 0,
        initial_command: None,
        scrollback_lines: 5000,
        font_size: 13,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 60,
        connect_timeout_secs: 15,
        session_kind: kind.into(),
        forward_x11: 0,
        starting_dir: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs_with(n: usize) -> (Tabs, Vec<TabId>) {
        let mut t = Tabs::new(ViewMode::Tabs, TileGrid::default());
        let ids = (0..n).map(|i| t.open(test_session(i as i64, &format!("s{i}"), "ssh"))).collect();
        (t, ids)
    }

    fn order(t: &Tabs) -> Vec<TabId> {
        t.tabs.iter().map(|x| x.id).collect()
    }

    #[test]
    fn open_activates_collapses_sidebar_and_staircases() {
        let (t, ids) = tabs_with(3);
        assert_eq!(t.active, Some(ids[2]));
        assert!(t.sidebar_collapsed);
        let g: Vec<_> = ids.iter().map(|id| t.cascade[id]).collect();
        assert_eq!((g[0].x, g[1].x, g[2].x), (0.0, 30.0, 60.0));
        assert!(g[0].z < g[1].z && g[1].z < g[2].z);
        assert_eq!(t.get(ids[0]).unwrap().status, TabStatus::Connecting);
    }

    #[test]
    fn close_picks_last_tab_and_drops_state() {
        let (mut t, ids) = tabs_with(3);
        t.set_active_for_test(ids[1]);
        t.minimize(ids[1]);
        t.close(ids[1]);
        assert_eq!(order(&t), vec![ids[0], ids[2]]);
        assert_eq!(t.active, Some(ids[2]));
        assert!(!t.cascade.contains_key(&ids[1]) && !t.minimized.contains(&ids[1]));
        t.close(ids[0]);
        assert_eq!(t.active, Some(ids[2]), "closing an inactive tab keeps the active one");
    }

    #[test]
    fn request_close_confirms_only_live_tabs() {
        let (mut t, ids) = tabs_with(2);
        t.set_status(ids[0], TabStatus::Connected, None);
        assert!(!t.request_close(ids[0], true));
        assert_eq!(t.pending_close, Some(ids[0]));
        assert!(t.request_close(ids[1], true), "connecting tabs close at once");
        assert!(t.request_close(ids[0], false), "setting off");
        assert_eq!(t.pending_close, None);
    }

    #[test]
    fn reorder_matches_the_store_contract() {
        let (mut t, ids) = tabs_with(4);
        t.reorder(0, 2); // before the tab at 2
        assert_eq!(order(&t), vec![ids[1], ids[0], ids[2], ids[3]]);
        t.reorder(3, 0);
        assert_eq!(order(&t), vec![ids[3], ids[1], ids[0], ids[2]]);
        t.reorder(1, 4); // append
        assert_eq!(order(&t), vec![ids[3], ids[0], ids[2], ids[1]]);
        let before = order(&t);
        t.reorder(1, 1);
        t.reorder(9, 0);
        t.reorder(0, 9);
        assert_eq!(order(&t), before, "no-ops");
    }

    #[test]
    fn view_mode_cycle_and_ids() {
        let mut m = ViewMode::Tabs;
        let mut seen = vec![];
        for _ in 0..6 {
            seen.push(m.id());
            m = m.next();
        }
        assert_eq!(m, ViewMode::Tabs);
        assert_eq!(seen, ["tabs", "tile-h", "tile-v", "tile-grid", "cascade", "auto"]);
        for m in ViewMode::ALL {
            assert_eq!(ViewMode::from_id(m.id()), Some(m));
        }
        assert_eq!(ViewMode::from_id("x"), None);
    }

    #[test]
    fn auto_grid_is_squarest() {
        let (mut t, ids) = tabs_with(5);
        t.view_mode = ViewMode::Auto;
        assert_eq!(t.grid_dims(), (2, 3));
        t.minimize(ids[0]);
        assert_eq!(t.grid_dims(), (2, 2));
        t.view_mode = ViewMode::TileGrid;
        t.tile_grid = TileGrid { rows: 1, cols: 4 };
        assert_eq!(t.grid_dims(), (1, 4));
        let (mut empty, _) = tabs_with(0);
        empty.view_mode = ViewMode::Auto;
        assert_eq!(empty.grid_dims(), (1, 1));
    }

    #[test]
    fn maximize_toggles_and_restores() {
        let (mut t, ids) = tabs_with(2);
        let before = t.cascade[&ids[1]];
        t.toggle_maximize(ids[1], 1000.0, 700.0);
        let g = t.cascade[&ids[1]];
        assert!(g.maximized && (g.x, g.y, g.w, g.h) == (0.0, 0.0, 1000.0, 700.0));
        t.toggle_maximize(ids[1], 1000.0, 700.0);
        let g = t.cascade[&ids[1]];
        assert!(!g.maximized);
        assert_eq!((g.x, g.y, g.w, g.h), (before.x, before.y, before.w, before.h));
    }

    #[test]
    fn restore_brings_to_front_and_z_order_skips_minimized() {
        let (mut t, ids) = tabs_with(3);
        t.minimize(ids[0]);
        assert_eq!(t.z_order(), vec![ids[1], ids[2]]);
        t.restore(ids[0]);
        assert_eq!(t.z_order(), vec![ids[1], ids[2], ids[0]]);
        t.bring_to_front(ids[1]);
        assert_eq!(t.z_order().last(), Some(&ids[1]));
    }

    #[test]
    fn cascade_geometry_clamps() {
        let g = CascadeGeom { x: 900.0, y: 500.0, w: 640.0, h: 400.0, z: 1, maximized: false, prev: None };
        assert_eq!(g.displayed(1000.0, 600.0), (360.0, 200.0, 640.0, 400.0));
        // Area smaller than the frame: frame shrinks to the area (not below the minimums).
        assert_eq!(g.displayed(500.0, 300.0), (0.0, 0.0, 500.0, 300.0));
        assert_eq!(g.displayed(100.0, 50.0), (0.0, 0.0, 200.0, 120.0));

        let start = (100.0, 100.0, 300.0, 200.0);
        assert_eq!(CascadeGeom::dragged(start, -500.0, 50.0, 800.0, 600.0), (0.0, 150.0));
        assert_eq!(CascadeGeom::dragged(start, 900.0, 900.0, 800.0, 600.0), (500.0, 400.0));

        assert_eq!(CascadeGeom::resized(start, Edge::SE, 50.0, 20.0, 800.0, 600.0), (100.0, 100.0, 350.0, 220.0));
        assert_eq!(CascadeGeom::resized(start, Edge::E, 1000.0, 0.0, 800.0, 600.0), (100.0, 100.0, 700.0, 200.0));
        // West edge: x and w move opposite, never below the minimum width.
        assert_eq!(CascadeGeom::resized(start, Edge::W, -40.0, 0.0, 800.0, 600.0), (60.0, 100.0, 340.0, 200.0));
        assert_eq!(CascadeGeom::resized(start, Edge::W, 250.0, 0.0, 800.0, 600.0), (200.0, 100.0, 200.0, 200.0));
        assert_eq!(CascadeGeom::resized(start, Edge::NW, -200.0, -200.0, 800.0, 600.0), (0.0, 0.0, 400.0, 300.0));
        assert_eq!(CascadeGeom::resized(start, Edge::N, 0.0, 150.0, 800.0, 600.0), (100.0, 180.0, 300.0, 120.0));
    }

    #[test]
    fn labels_and_summary() {
        let mut s = test_session(1, "web", "ssh");
        let tab = Tab {
            id: 1,
            session: s.clone(),
            connection_id: None,
            status: TabStatus::Connected,
            message: None,
            sftp_open: false,
            forwards_open: false,
            cwd: "/".into(),
            font_family: String::new(),
            color: None,
        };
        assert_eq!(tab.label(false), "web");
        assert_eq!(tab.label(true), "root@example.org");
        assert_eq!(tab.summary(), "root@example.org");
        s.port = 2222;
        assert_eq!(session_summary(&s), "root@example.org:2222");
        let local = Tab { session: Session { host: "/bin/bash".into(), port: 0, username: String::new(), ..test_session(2, "bash", "local") }, ..tab };
        assert_eq!(local.label(true), "bash", "local shells keep their name");
        assert_eq!(local.summary(), "/bin/bash");
        let wsl = Session { host: "Ubuntu".into(), port: 0, ..test_session(3, "Ubuntu", "wsl") };
        assert_eq!(session_summary(&wsl), "root@Ubuntu");
        assert_eq!(session_summary(&Session { username: String::new(), ..wsl }), "Ubuntu");
    }

    impl Tabs {
        fn set_active_for_test(&mut self, id: TabId) {
            self.active = Some(id);
        }
    }
}
