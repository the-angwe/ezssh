//! Headless tests of the credentials manager, forwards pane and forward dialog.

use std::time::{Duration, Instant};

use egui::{Key, Modifiers, Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ezssh_core::api;
use ezssh_core::db::forwards::ForwardInput;
use ezssh_core::db::sessions::{Session, SessionInput};
use ezssh_core::ssh::forwards::{ForwardKind, ForwardSpec, ForwardStatus, RuntimeForwardSummary};

use super::{Env, PASSWORD, shot, start, unlock_fresh, wait};
use crate::app::App;
use crate::style::Theme;

/// An SSH session that cannot connect (nothing listens on port 1).
fn ssh_input(name: &str, auth: &str, key: Option<i64>, passphrase: Option<i64>) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: name.into(),
        host: "127.0.0.1".into(),
        port: 1,
        username: "root".into(),
        auth_type: auth.into(),
        credential_id: key,
        key_passphrase_credential_id: passphrase,
        color: None,
        initial_command: None,
        scrollback_lines: 1000,
        font_size: 13,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 60,
        connect_timeout_secs: 5,
        env: Vec::new(),
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: None,
    }
}

fn create_session(app: &App, input: SessionInput) -> Session {
    app.core.block_on(|st| async move { api::sessions::session_create(&st, input).await }).expect("session_create")
}

fn create_credential(app: &App, kind: &str, label: &str, secret: &str) -> i64 {
    let (kind, label, secret) = (kind.to_string(), label.to_string(), secret.to_string());
    app.core.block_on(|st| async move { api::credentials::credential_create(&st, kind, label, secret).await }).expect("credential_create").id
}

fn forward(name: &str, kind: &str, bind_port: i64, dest: &str, dest_port: i64, auto: bool) -> ForwardInput {
    ForwardInput {
        name: name.into(),
        kind: kind.into(),
        bind_addr: "127.0.0.1".into(),
        bind_port,
        dest_addr: dest.into(),
        dest_port,
        auto_start: i64::from(auto),
    }
}

fn create_forward(app: &App, session: i64, input: ForwardInput) -> i64 {
    app.core.block_on(|st| async move { api::forwards::forward_create(&st, session, input).await }).expect("forward_create").id
}

fn saved_forwards(app: &App, session: i64) -> Vec<ezssh_core::db::forwards::Forward> {
    app.core.block_on(|st| async move { api::forwards::forward_list(&st, session).await }).expect("forward_list")
}

fn is_field(role: Role) -> bool {
    matches!(role, Role::TextInput | Role::PasswordInput | Role::MultilineTextInput)
}

/// Click the text field named `label` (not its visible caption) and
/// replace its text with `text`.
fn fill<S: 'static>(h: &mut Harness<'static, S>, label: &str, text: &str) {
    let field = |h: &Harness<'static, S>| {
        h.query_all_by_label(label).find(|n| is_field(n.accesskit_node().role())).unwrap_or_else(|| panic!("no field {label:?}")).rect()
    };
    // Wait out a modal re-centring after its size changed.
    let mut rect = field(h);
    for _ in 0..10 {
        h.step();
        let now = field(h);
        if now == rect {
            break;
        }
        rect = now;
    }
    h.event(egui::Event::PointerMoved(rect.center()));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.to_string()));
    h.run_steps(2);
}

fn listed(app: &App) -> Vec<(String, String)> {
    app.dialogs.credentials.as_ref().and_then(|d| d.listed()).unwrap_or_default()
}

fn has_label(app: &App, label: &str) -> bool {
    listed(app).iter().any(|(_, l)| l == label)
}

fn open_manager(h: &mut Harness<'static, App>) {
    h.state_mut().dialogs.credentials = Some(Default::default());
    wait(h, 10, "credentials loaded", |a| a.dialogs.credentials.as_ref().and_then(|d| d.listed()).is_some());
    // Past the modal's sizing pass, so clicks land where the nodes are.
    h.run_steps(5);
}

fn field_value<S: 'static>(h: &Harness<'static, S>, label: &str) -> Option<String> {
    h.query_all_by_label(label).find(|n| is_field(n.accesskit_node().role())).and_then(|n| n.value())
}

#[test]
fn credentials_manager_add_rename_rotate_delete() {
    let mut env: Env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let key = create_credential(h.state(), "private_key", "Prod key", "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----");
    create_credential(h.state(), "key_passphrase", "Prod passphrase", "hunter2");
    // A leftover from the ezTerm app's password picker: unowned.
    create_credential(h.state(), "password", "Picker password", "pw");
    create_session(h.state(), ssh_input("db-server", "key", Some(key), None));

    open_manager(h);
    let names = listed(h.state());
    assert!(names.contains(&("private_key".into(), "Prod key".into())), "{names:?}");
    assert!(names.contains(&("key_passphrase".into(), "Prod passphrase".into())));
    assert!(h.query_by_label("Add SSH keys").is_some() && h.query_by_label("Add Key passphrases").is_some());
    assert!(h.query_by_label("Add Passwords").is_none(), "no password group to add to");
    // The unattached password: delete only.
    assert!(h.query_by_label("Delete Picker password").is_some());
    assert!(h.query_by_label("Rename Picker password").is_none());
    assert!(h.query_by_label_contains("Session passwords are saved with each session").is_some());
    shot(h, "credentials-dark");

    // Add a key (textarea secret).
    h.get_by_label("Add SSH keys").click();
    h.run_steps(3);
    shot(h, "credentials-add");
    fill(h, "New SSH keys label", "Laptop key");
    fill(h, "New SSH keys secret", "-----BEGIN OPENSSH PRIVATE KEY-----\nxyz");
    h.get_by_label("Save credential").click();
    wait(h, 10, "key added", |a| has_label(a, "Laptop key"));
    wait(h, 10, "shared cache reloaded", |a| a.creds.list.iter().any(|c| c.label == "Laptop key"));
    assert!(!h.state().creds.list.iter().any(|c| c.kind == "password" && c.label == "Laptop key"));

    // Rename with Enter.
    h.get_by_label("Rename Prod key").click();
    h.run_steps(3);
    assert!(h.state().dialogs.credentials.as_ref().is_some_and(|d| d.editing()));
    fill(h, "Rename Prod key", "Prod key 2");
    h.key_press(Key::Enter);
    wait(h, 10, "renamed", |a| has_label(a, "Prod key 2"));
    wait(h, 10, "cache sees the rename", |a| a.creds.list.iter().any(|c| c.label == "Prod key 2"));

    // Rotate a passphrase.
    h.get_by_label("Replace secret for Prod passphrase").click();
    h.run_steps(3);
    fill(h, "New secret for Prod passphrase", "correct-horse");
    shot(h, "credentials-rotate");
    h.get_by_label("Replace secret").click();
    wait(h, 10, "secret updated", |a| a.toasts.items().iter().any(|t| t.title == "Secret updated"));
    assert!(!h.state().dialogs.credentials.as_ref().unwrap().editing(), "inline form closed after success");

    // Delete warns with the sessions that use it.
    h.get_by_label("Delete Prod key 2").click();
    h.run_steps(3);
    assert!(h.state().dialogs.credentials.as_ref().unwrap().confirming());
    assert!(h.query_by_label("Delete \"Prod key 2\"?").is_some());
    assert!(h.query_by_label_contains("Used by 1 session:\n· db-server").is_some(), "usage listed");
    shot(h, "credentials-delete-confirm");
    h.get_by_label("Delete").click();
    wait(h, 10, "deleted", |a| !has_label(a, "Prod key 2"));
    wait(h, 10, "cache sees the delete", |a| !a.creds.list.iter().any(|c| c.label == "Prod key 2"));
    assert!(!h.state().dialogs.credentials.as_ref().unwrap().confirming());

    // Escape unwinds: the add form first, then the dialog.
    h.get_by_label("Add Key passphrases").click();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(2);
    let d = h.state().dialogs.credentials.as_ref().expect("still open");
    assert!(!d.editing(), "Escape closed the add form");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialogs.credentials.is_none(), "second Escape closes");

    h.state_mut().set_theme(Theme::Light);
    open_manager(h);
    shot(h, "credentials-light");
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert!(h.state().dialogs.credentials.is_none());
}

fn status(id: u64, persistent: Option<i64>, spec: ForwardSpec, status: ForwardStatus) -> ezssh_core::Event {
    ezssh_core::Event::ForwardStatus { connection_id: FAKE_CONN, summary: RuntimeForwardSummary { runtime_id: id, persistent_id: persistent, spec, status } }
}

const FAKE_CONN: u64 = 9_999;

#[test]
fn forwards_pane_status_menu_and_dialog() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let session = create_session(h.state(), ssh_input("tunnel-host", "agent", None, None));
    let pg = create_forward(h.state(), session.id, forward("Postgres dev", "local", 15432, "db.internal", 5432, true));
    let socks = create_forward(h.state(), session.id, forward("", "dynamic", 1080, "", 0, false));

    h.state_mut().open_session(session.clone());
    let tab = h.state().tabs.active.expect("tab");
    h.state_mut().tabs.get_mut(tab).unwrap().forwards_open = true;
    wait(h, 10, "saved forwards loaded", move |a| a.forwards.saved_labels(session.id).is_some_and(|l| l.len() == 2));
    // The connect to port 1 fails; wait for it so it does not race the fake below.
    wait(h, 15, "connect attempt over", move |a| a.tabs.get(tab).is_some_and(|t| t.connection_id.is_none() && t.status != crate::tabs::TabStatus::Connecting));
    assert!(h.query_by_role_and_label(Role::ListItem, "Postgres dev").is_some());
    assert!(h.query_by_role_and_label(Role::ListItem, "SOCKS5 @ 127.0.0.1:1080").is_some());
    assert!(h.get_by_label("Start Postgres dev").accesskit_node().is_disabled(), "Start needs a connection");
    assert!(h.get_by_label("Add forward").accesskit_node().is_disabled());
    shot(h, "forwards-pane-offline");

    // Pretend the tab is connected and the core reports statuses.
    h.state_mut().forwards.fake_connection(FAKE_CONN);
    h.state_mut().tabs.get_mut(tab).unwrap().connection_id = Some(FAKE_CONN);
    let spec = |name: &str, kind, port| ForwardSpec { name: name.into(), kind, bind_addr: "127.0.0.1".into(), bind_port: port, dest_addr: "db.internal".into(), dest_port: 5432 };
    let app = h.state_mut();
    crate::views::forwards::on_event(app, status(1, Some(pg), spec("Postgres dev", ForwardKind::Local, 15432), ForwardStatus::Running));
    crate::views::forwards::on_event(
        app,
        status(2, Some(socks), spec("", ForwardKind::Dynamic, 1080), ForwardStatus::Error { message: "bind 127.0.0.1:1080 permission denied".into() }),
    );
    crate::views::forwards::on_event(
        app,
        status(3, None, spec("", ForwardKind::Remote, 8080), ForwardStatus::Conflict { message: "bind 127.0.0.1:8080 in use (another ezSsh tab?)".into() }),
    );
    h.run_steps(3);
    assert!(h.query_by_label("Stop Postgres dev").is_some(), "running row offers Stop");
    assert!(!h.get_by_label("Start SOCKS5 @ 127.0.0.1:1080").accesskit_node().is_disabled());
    assert!(h.query_by_role_and_label(Role::ListItem, "127.0.0.1:8080 → db.internal:5432").is_some(), "tab-only row");
    assert!(h.state().toasts.items().iter().any(|t| t.title.starts_with("Forward failed — SOCKS5")), "errors toast");
    shot(h, "forwards-pane-status");

    // Context menu: Edit opens the saved forward.
    h.get_by_role_and_label(Role::ListItem, "Postgres dev").click_secondary();
    h.run_steps(3);
    assert!(h.state().forwards.menu_open());
    assert!(h.query_by_role_and_label(Role::MenuItem, "Stop").is_some());
    shot(h, "forwards-pane-menu");
    h.get_by_role_and_label(Role::MenuItem, "Edit").click();
    h.run_steps(3);
    assert!(h.query_by_label("Forward (session config)").is_some());
    assert_eq!(field_value(h, "Name (optional)").as_deref(), Some("Postgres dev"));
    assert!(h.query_by_label("Save to this session (persists across reconnects)").is_none());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().forwards.dialog_open(), "Escape closes");

    // Add on this tab: validation and warnings.
    h.get_by_label("Add forward").click();
    h.run_steps(3);
    assert!(h.query_by_label("Forward (this tab)").is_some());
    assert!(h.query_by_label("Save to this session (persists across reconnects)").is_some());
    assert!(h.query_by_label("Auto-start when the session connects").is_none(), "only once persisted");
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert_eq!(h.state().forwards.dialog_error(), Some("Bind port must be 1–65535"));
    fill(h, "Bind address", "0.0.0.0");
    fill(h, "Bind port", "80");
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert_eq!(h.state().forwards.dialog_error(), Some("Destination host is required"));
    assert!(h.query_by_label("This forward will be reachable from other machines on your network.").is_some());
    assert!(h.query_by_label("Ports below 1024 require admin/root on most systems.").is_some());
    h.get_by_label("Save to this session (persists across reconnects)").click();
    h.run_steps(2);
    assert!(h.query_by_label("Auto-start when the session connects").is_some());
    shot(h, "forward-dialog-validation");
    h.get_by_label("Dynamic (-D)").click();
    h.run_steps(2);
    assert!(h.query_by_label("Destination host").is_none(), "dynamic has no destination");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(!h.state().forwards.dialog_open());

    // Delete from the pane.
    h.get_by_label("Delete SOCKS5 @ 127.0.0.1:1080").click();
    wait(h, 10, "deleted", move |a| a.forwards.saved_labels(session.id).is_some_and(|l| l.len() == 1));
    assert_eq!(saved_forwards(h.state(), session.id).len(), 1);

    h.state_mut().set_theme(Theme::Light);
    shot(h, "forwards-pane-light");
}

/// Hosts the session dialog's Forwards tab on its own, since the session
/// dialog is ported elsewhere.
struct TabHost {
    app: App,
    session: Option<i64>,
}

impl eframe::App for TabHost {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.app, ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Frame::new().fill(crate::style::tokens(&ctx).surface).inner_margin(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            crate::views::forwards::session_tab(&mut self.app, ui, self.session);
        });
        crate::views::forwards::dialogs(&mut self.app, &ctx);
        self.app.toasts.show(&ctx);
    }
}

fn host_wait(h: &mut Harness<'static, TabHost>, what: &str, cond: impl Fn(&TabHost) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
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

fn host_shot(h: &mut Harness<'static, TabHost>, name: &str) {
    let Ok(dir) = std::env::var("EZSSH_UI_SHOTS") else {
        return;
    };
    h.run_steps(10);
    let img = h.render().unwrap_or_else(|e| panic!("render failed: {e}"));
    let path = std::path::Path::new(&dir).join(format!("app-{name}.png"));
    img.save(&path).unwrap_or_else(|e| panic!("saving {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

#[test]
fn session_tab_lists_adds_edits_deletes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("ezterm.sqlite");
    let mut h = Harness::builder().with_size(vec2(1000.0, 760.0)).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        cc.egui_ctx.add_font_provider(std::sync::Arc::new(egui_system_font_provider::SystemFontProvider::new()));
        TabHost { app: App::new(&cc.egui_ctx, crate::app::Options { db_path: db_path.clone(), updater: None }).expect("app starts"), session: None }
    });
    h.run_steps(2);
    assert!(h.query_by_label("Save the session first to configure forwards.").is_some());

    let app = &mut h.state_mut().app;
    app.core.block_on(|st| async move { api::vault::vault_init(&st, PASSWORD.into()).await }).expect("vault_init");
    app.on_unlocked();
    let sid = create_session(app, ssh_input("tunnel-host", "agent", None, None)).id;
    create_forward(app, sid, forward("Postgres dev", "local", 15432, "db.internal", 5432, true));
    h.state_mut().session = Some(sid);
    host_wait(&mut h, "list loaded", move |s| s.app.forwards.saved_labels(sid).is_some_and(|l| l.len() == 1));
    assert!(h.query_by_role_and_label(Role::ListItem, "Postgres dev").is_some());
    host_shot(&mut h, "session-forwards-tab");

    // Add.
    h.get_by_label("Add forward").click();
    h.run_steps(3);
    assert!(h.query_by_label("Forward (session config)").is_some());
    assert!(h.query_by_label("Auto-start when the session connects").is_some());
    fill(&mut h, "Bind port", "2222");
    fill(&mut h, "Destination host", "localhost");
    fill(&mut h, "Destination port", "22");
    host_shot(&mut h, "session-forward-dialog");
    h.state_mut().app.set_theme(Theme::Light);
    host_shot(&mut h, "session-forward-dialog-light");
    h.get_by_label("Save").click();
    host_wait(&mut h, "added", move |s| s.app.forwards.saved_labels(sid).is_some_and(|l| l.len() == 2) && !s.app.forwards.dialog_open());
    let rows = saved_forwards(&h.state().app, sid);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|f| f.bind_port == 2222 && f.dest_addr == "localhost" && f.auto_start == 1));

    // Edit: rename the new one.
    h.get_by_label("Edit 127.0.0.1:2222 → localhost:22").click();
    h.run_steps(3);
    fill(&mut h, "Name (optional)", "ssh hop");
    h.get_by_label("Save").click();
    host_wait(&mut h, "edited", move |s| s.app.forwards.saved_labels(sid).is_some_and(|l| l.contains(&"ssh hop".to_string())));

    // Delete.
    h.get_by_label("Delete Postgres dev").click();
    host_wait(&mut h, "deleted", move |s| s.app.forwards.saved_labels(sid).is_some_and(|l| l == ["ssh hop"]));
    assert_eq!(saved_forwards(&h.state().app, sid).len(), 1);
}
