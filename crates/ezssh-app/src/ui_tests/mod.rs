//! Headless end-to-end tests of the shell: the real [`App`] on a fresh
//! database in a temp dir, driven through egui_kittest (AccessKit queries,
//! synthetic input). Local shell tabs run real PTYs, so these are Unix
//! only, like `ezssh-core/tests/api_local.rs`.
//!
//! `EZSSH_UI_SHOTS=<dir> cargo test -p ezssh-app ui_tests` also renders the
//! screens to `<dir>/<name>.png` with wgpu.

#![cfg(unix)]

// Phase 5 dialog tests, one file per dialog group; they use the helpers
// below.
mod credentials;
mod data;
mod sessions;
mod vault;

use std::time::{Duration, Instant};

use egui::{Event, Modifiers, PointerButton, Pos2, Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::app::{App, Options, Screen};
use crate::tabs::{TabStatus, ViewMode};

pub(crate) const PASSWORD: &str = "correct horse battery";

pub(crate) struct Env {
    pub(crate) _dir: tempfile::TempDir,
    pub(crate) h: Harness<'static, App>,
}

pub(crate) fn start() -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("ezterm.sqlite");
    let h = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        // Real-time-ish frames: egui counts two clicks as a double-click
        // only within 0.3 s, and kittest's default step is 0.25 s.
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            cc.egui_ctx.add_font_provider(std::sync::Arc::new(egui_system_font_provider::SystemFontProvider::new()));
            App::new(&cc.egui_ctx, Options { db_path: db_path.clone() }).expect("app starts")
        });
    Env { _dir: dir, h }
}

/// Step frames until `cond` holds (async core calls finish on other
/// threads), or fail after `secs`.
pub(crate) fn wait(h: &mut Harness<'static, App>, secs: u64, what: &str, cond: impl Fn(&App) -> bool) {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        h.step();
        if cond(h.state()) {
            h.step();
            return;
        }
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(15));
    }
}

pub(crate) fn shot(h: &mut Harness<'static, App>, name: &str) {
    let Ok(dir) = std::env::var("EZSSH_UI_SHOTS") else {
        return;
    };
    h.run_steps(10);
    match h.render() {
        Ok(img) => {
            let path = std::path::Path::new(&dir).join(format!("app-{name}.png"));
            img.save(&path).unwrap_or_else(|e| panic!("saving {}: {e}", path.display()));
            eprintln!("wrote {}", path.display());
        }
        Err(e) => panic!("render failed: {e}"),
    }
}

pub(crate) fn type_into(h: &mut Harness<'static, App>, index: usize, text: &str) {
    let fields: Vec<_> = h.query_all_by_role(Role::PasswordInput).collect();
    let field = fields.get(index).unwrap_or_else(|| panic!("password field {index} missing ({} found)", fields.len()));
    field.click();
    field.type_text(text);
    h.run_steps(2);
}

/// Create the vault and wait for the workspace with the seeded sessions.
pub(crate) fn unlock_fresh(env: &mut Env) {
    let h = &mut env.h;
    h.run_steps(3);
    assert!(h.query_by_label("Set master password").is_some(), "first run shows vault creation");
    shot(h, "first-run");
    type_into(h, 0, PASSWORD);
    type_into(h, 1, PASSWORD);
    h.get_by_label("Create vault").click();
    wait(h, 20, "main screen", |a| matches!(a.screen, Screen::Main));
    wait(h, 20, "seeded local shells", |a| a.sidebar.sessions.iter().any(|s| s.session_kind == "local"));
}

/// Everything in the tab's terminal, scrollback included.
pub(crate) fn screen_text(h: &mut Harness<'static, App>, tab: u64) -> String {
    let term = &mut h.state_mut().terms.get_mut(&tab).expect("terminal").term;
    term.select_all();
    term.selection_text().unwrap_or_default()
}

pub(crate) fn first_local(h: &Harness<'static, App>) -> String {
    h.state().sidebar.sessions.iter().find(|s| s.session_kind == "local").expect("a local shell session").name.clone()
}

pub(crate) fn open_local(h: &mut Harness<'static, App>) -> u64 {
    // Out of egui's multi-click window, so the next two clicks are a
    // double-click and not the end of a triple.
    h.run_steps(30);
    let name = first_local(h);
    let before = h.state().tabs.tabs.len();
    h.get_by_role_and_label(Role::TreeItem, &name).click(); // select
    // Double-click opens a tab.
    let rect = h.get_by_role_and_label(Role::TreeItem, &name).rect();
    for _ in 0..2 {
        h.event(Event::PointerMoved(rect.center()));
        h.event(Event::PointerButton { pos: rect.center(), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.event(Event::PointerButton { pos: rect.center(), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
    wait(h, 5, "tab opened", |a| a.tabs.tabs.len() == before + 1);
    let id = h.state().tabs.active.expect("new tab is active");
    wait(h, 15, "local shell connected", move |a| a.tabs.get(id).is_some_and(|t| t.status == TabStatus::Connected));
    id
}

#[test]
fn first_run_unlock_open_tab_lock_and_unlock() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    assert!(h.query_by_role_and_label(Role::TreeItem, "Local Shells").is_some(), "seeded folder in the tree");
    shot(h, "main-empty");

    let tab = open_local(h);
    assert!(h.state().tabs.sidebar_collapsed, "opening a tab collapses the sidebar");
    assert!(h.query_by_role(Role::Tab).is_some());
    // The shell answers a command through the PTY path.
    h.state().terms[&tab].term.write(b"echo ezssh-$((40+2))\r".to_vec());
    let until = Instant::now() + Duration::from_secs(10);
    while !screen_text(h, tab).contains("ezssh-42") {
        assert!(Instant::now() < until, "no command output");
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    shot(h, "tab-local-shell");

    // Lock: the unlock screen covers the window, the tab stays connected.
    let conn = h.state().tabs.get(tab).and_then(|t| t.connection_id);
    h.get_by_label("Lock vault").click();
    wait(h, 5, "unlock screen", |a| matches!(&a.screen, Screen::Unlock(u) if !u.first_run));
    assert!(h.query_by_label("Unlock ezSsh").is_some());
    shot(h, "locked");
    type_into(h, 0, "wrong password");
    h.get_by_label("Unlock").click();
    wait(h, 10, "bad password message", |a| matches!(&a.screen, Screen::Unlock(u) if u.error().is_some()));
    assert!(h.query_by_label("Incorrect master password").is_some());
    // Select the wrong text and type the right password over it.
    h.get_by_role(Role::PasswordInput).click();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    // No click here: it would drop the selection.
    h.get_by_role(Role::PasswordInput).type_text(PASSWORD);
    h.run_steps(2);
    h.get_by_label("Unlock").click();
    wait(h, 10, "main again", |a| matches!(a.screen, Screen::Main));
    let still = h.state().tabs.get(tab).map(|t| (t.status, t.connection_id));
    assert_eq!(still, Some((TabStatus::Connected, conn)), "the connection survived the lock");
}

#[test]
fn view_modes_cascade_and_minimize() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let tabs: Vec<u64> = (0..3).map(|_| {
        h.state_mut().tabs.sidebar_collapsed = false;
        h.run_steps(2);
        open_local(h)
    }).collect();
    assert_eq!(h.state().tabs.active, Some(tabs[2]));

    for (label, mode) in [("Tile horizontal", ViewMode::TileH), ("Tile vertical", ViewMode::TileV), ("Auto-arrange", ViewMode::Auto)] {
        h.get_by_label(label).click();
        h.run_steps(3);
        assert_eq!(h.state().tabs.view_mode, mode);
        shot(h, &format!("mode-{}", mode.id()));
    }

    // Tile grid asks for the layout first.
    h.get_by_label("Tile grid…").click();
    h.run_steps(3);
    assert!(h.query_by_label("Tile grid").is_some(), "grid dialog open");
    shot(h, "tile-grid-dialog");
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert_eq!(h.state().tabs.view_mode, ViewMode::TileGrid);

    // Cascade: minimise the top frame, restore it from the strip.
    h.get_by_label("Cascade").click();
    h.run_steps(3);
    shot(h, "mode-cascade");
    let minimize: Vec<_> = h.query_all_by_label("Minimize").collect();
    assert_eq!(minimize.len(), 3, "one chrome per frame");
    let top = *h.state().tabs.z_order().last().unwrap();
    let top_min = minimize.iter().max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap();
    top_min.click();
    h.run_steps(3);
    assert!(h.state().tabs.is_minimized(top), "the bottom-most chrome belongs to the top frame (staircase)");
    shot(h, "cascade-minimized");
    let name = h.state().tabs.get(top).unwrap().session.name.clone();
    h.get_by_label(&format!("Restore {name}")).click();
    h.run_steps(3);
    assert!(!h.state().tabs.is_minimized(top));

    // Drag the top frame by its title bar.
    let before = h.state().tabs.cascade[&top];
    let maxes: Vec<_> = h.query_all_by_label("Maximize").map(|n| n.rect()).collect();
    let bar = maxes.iter().max_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    let from = Pos2::new(bar.left() - 200.0, bar.center().y);
    drag(h, from, from + vec2(120.0, 80.0));
    let after = h.state().tabs.cascade[&top];
    assert_eq!((after.x - before.x, after.y - before.y), (120.0, 80.0), "frame moved with the pointer");
    shot(h, "cascade-dragged");

    // Ctrl+Shift+W cycles (cascade → auto) while no terminal has focus.
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::W);
    h.run_steps(3);
    assert_eq!(h.state().tabs.view_mode, ViewMode::Auto);
}

/// Press at `from`, move in steps to `to`, release.
pub(crate) fn drag(h: &mut Harness<'static, App>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=10 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn sidebar_resize_persists() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let strip = h.get_by_label("Resize sidebar").rect();
    assert!((strip.right() - crate::layout::SIDEBAR_DEFAULT).abs() <= 1.0, "default width, got {}", strip.right());
    drag(h, strip.center(), strip.center() + vec2(100.0, 0.0));
    assert_eq!(h.state().layout.sidebar_width, crate::layout::SIDEBAR_DEFAULT + 100.0);
    let path = crate::layout::path_for(&env._dir.path().join("ezterm.sqlite"));
    assert_eq!(crate::layout::load(&path).sidebar_width, crate::layout::SIDEBAR_DEFAULT + 100.0, "saved on release");
    // Never past the maximum.
    let strip = env.h.get_by_label("Resize sidebar").rect();
    drag(&mut env.h, strip.center(), strip.center() + vec2(900.0, 0.0));
    assert_eq!(env.h.state().layout.sidebar_width, crate::layout::SIDEBAR_MAX);
}

#[test]
fn folder_create_drag_drop_and_close_confirm() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;

    h.get_by_label("New folder").click();
    h.run_steps(3);
    h.get_by_role(Role::TextInput).type_text("Servers");
    h.get_by_label("Create").click();
    wait(h, 10, "folder created", |a| a.sidebar.folders.iter().any(|f| f.name == "Servers"));
    h.run_steps(2);
    let folder = h.state().sidebar.folders.iter().find(|f| f.name == "Servers").unwrap().id;

    // Drag the first local shell onto the folder row.
    let name = first_local(h);
    let from = h.get_by_role_and_label(Role::TreeItem, &name).rect().center();
    let to = h.get_by_role_and_label(Role::TreeItem, "Servers").rect().center();
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=10 {
        let p = from + (to - from) * (i as f32 / 10.0);
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.run_steps(2);
    assert_eq!(h.state().sidebar.target_for_test(), Some(crate::views::sidebar::DropSlot::IntoFolder(folder)));
    shot(h, "sidebar-drag");
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    let n = name.clone();
    wait(h, 10, "session moved", move |a| a.sidebar.sessions.iter().any(|s| s.name == n && s.folder_id == Some(folder)));

    // Root context menu: every item is live now.
    let tree = h.get_by_role_and_label(Role::TreeItem, "Servers").rect();
    let below = Pos2::new(tree.center().x, 760.0);
    h.event(Event::PointerMoved(below));
    h.event(Event::PointerButton { pos: below, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: below, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(!h.get_by_label("Backup…").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Settings…").accesskit_node().is_disabled());
    shot(h, "sidebar-root-menu");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    // Closing a connected tab asks first.
    let tab = open_local(h);
    h.state_mut().request_close_tab(tab);
    h.run_steps(3);
    // The tab's ✕ and the dialog's button share the label "Close tab", as
    // in the webview; the dialog's is the one below the strip.
    assert!(h.query_by_label("This tab is connected. Closing it ends the session.").is_some(), "confirmation shown");
    shot(h, "close-confirm");
    let buttons: Vec<_> = h.query_all_by_label("Close tab").collect();
    buttons.iter().find(|n| n.rect().top() > 100.0).expect("dialog button").click();
    h.run_steps(3);
    assert!(h.state().tabs.get(tab).is_none());
}

#[test]
fn settings_write_through_and_theme() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    h.get_by_label("Settings").click();
    h.run_steps(3);
    shot(h, "settings-appearance");
    h.get_by_label("Terminal").click();
    h.run_steps(3);
    shot(h, "settings-terminal");
    h.get_by_role_and_label(Role::CheckBox, "Copy on select").click();
    h.run_steps(2);
    assert!(h.state().settings.bool("terminal.copyOnSelect"));
    assert!(h.state().term_style.copy_on_select, "applied to the terminal style");
    wait(h, 10, "stored", |a| {
        a.core.block_on(|st| async move { ezssh_core::api::settings::settings_get(&st, "terminal.copyOnSelect".into()).await })
            .ok()
            .flatten()
            .as_deref()
            == Some("true")
    });
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialogs.settings.is_none(), "Escape closes");

    h.get_by_label("Light theme").click();
    h.run_steps(3);
    assert_eq!(h.state().theme, crate::style::Theme::Light);
    shot(h, "main-light");
    wait(h, 10, "theme stored", |a| {
        a.core.block_on(|st| async move { ezssh_core::api::settings::settings_get(&st, "theme".into()).await }).ok().flatten().as_deref()
            == Some("light")
    });
}

#[test]
fn idle_lock_host_key_and_paste_confirm() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let tab = open_local(h);

    // Host-key prompt (an SSH tab would get it from the connect error).
    h.state_mut().terms.get_mut(&tab).unwrap().host_key = Some(crate::conn::HostKeyPrompt {
        mismatch: true,
        fingerprint: "SHA256:newnewnew".into(),
        expected: Some("SHA256:oldoldold".into()),
    });
    h.run_steps(3);
    assert!(h.query_by_label("Host key changed!").is_some());
    assert!(h.query_by_label("SHA256:oldoldold").is_some() && h.query_by_label("SHA256:newnewnew").is_some());
    shot(h, "host-key-mismatch");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().terms[&tab].host_key.is_none());
    assert_eq!(h.state().tabs.get(tab).unwrap().status, TabStatus::Closed);

    // Reconnect from the overlay card.
    h.get_by_label("Reconnect").click();
    wait(h, 15, "reconnected", move |a| a.tabs.get(tab).is_some_and(|t| t.status == TabStatus::Connected));

    // Multi-line paste waits for a yes, then reaches the shell.
    h.state_mut().dialogs.paste = Some((tab, "echo one-$((1+1))\necho two-$((1+2))\n".into(), 2));
    h.run_steps(3);
    assert!(h.query_by_label("Paste multiple lines?").is_some());
    shot(h, "paste-confirm");
    h.get_by_label("Paste").click();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let text = screen_text(h, tab);
        if text.contains("one-2") && text.contains("two-3") {
            break;
        }
        assert!(Instant::now() < until, "pasted lines did not run");
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }

    // Idle auto-lock: one minute of silence with the setting at 1.
    h.state_mut().settings.set("security.idleLockMinutes", crate::settings::Value::Int(1));
    h.state_mut().last_input = Instant::now() - Duration::from_secs(61);
    h.run_steps(2);
    assert!(matches!(h.state().screen, Screen::Unlock(_)), "locked after the idle limit");
    assert!(h.state().tabs.get(tab).is_some_and(|t| t.status == TabStatus::Connected), "tab kept");
}
