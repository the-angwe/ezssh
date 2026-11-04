//! Headless tests of the session dialog, credential picker, auth-fix overlay.

use egui::{Modifiers, Role};
use egui_kittest::kittest::Queryable;
use ezssh_core::api;
use ezssh_core::db::sessions::{Session, SessionInput};

use super::*;
use crate::style::Theme;
use crate::views::auth_fix::AuthFix;
use crate::views::session_dialog::SessionDialog;

type H = Harness<'static, App>;

/// Scroll `label` (of `role`) into view, click it, type `text`.
fn fill(h: &mut H, role: Role, label: &str, text: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(role, label).click();
    h.run_steps(1);
    h.get_by_role_and_label(role, label).type_text(text);
    h.run_steps(2);
}

/// Select everything in a text field and type over it, then Enter (number
/// fields commit on Enter / blur).
fn replace(h: &mut H, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, label).click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn click(h: &mut H, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

fn session_named(h: &H, name: &str) -> Session {
    h.state()
        .sidebar
        .sessions
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("session {name} in the sidebar"))
        .clone()
}

fn db_session(h: &H, id: i64) -> Session {
    h.state()
        .core
        .block_on(|st| async move { api::sessions::session_get(&st, id).await })
        .expect("session row")
}

/// Every credentials row (session passwords included) as (id, owner).
fn all_rows(h: &H) -> Vec<(i64, Option<i64>)> {
    h.state()
        .core
        .block_on(|st| async move { ezssh_core::db::credentials::list_for_export(&st.db).await })
        .expect("credential rows")
        .into_iter()
        .map(|r| (r.id, r.session_id))
        .collect()
}

/// Owner (`credentials.session_id`) of credential `id`.
fn owner_of(h: &H, id: i64) -> Option<i64> {
    all_rows(h)
        .into_iter()
        .find(|(c, _)| *c == id)
        .unwrap_or_else(|| panic!("credential {id} exists"))
        .1
}

fn owned_rows(h: &H, session: i64) -> usize {
    all_rows(h)
        .into_iter()
        .filter(|(_, o)| *o == Some(session))
        .count()
}

/// Plaintext of credential `id`, base64 (as a backup writes it).
fn secret_b64(h: &H, id: i64) -> String {
    h.state()
        .core
        .block_on(|st| async move {
            let vs = st.vault.read().await;
            ezssh_core::backup::export_credentials(&st.db, &vs).await
        })
        .expect("export")
        .into_iter()
        .find(|c| c.id == id)
        .expect("exported")
        .secret_b64
}

fn shared_list(h: &H) -> Vec<(String, String)> {
    h.state()
        .core
        .block_on(|st| async move { api::credentials::credential_list(&st).await })
        .expect("credential_list")
        .into_iter()
        .map(|c| (c.kind, c.label))
        .collect()
}

fn alert(h: &H, text: &str) -> bool {
    h.query_by_label(text).is_some()
}

/// Toasts sit over the dialog footer; clear them before clicking there.
fn clear_toasts(h: &mut H) {
    h.state_mut().toasts = crate::toasts::Toasts::default();
    h.run_steps(1);
}

fn open_new(h: &mut H) {
    clear_toasts(h);
    super::sidebar_new(h, "New session");
    wait(h, 5, "dialog open", |a| {
        a.dialogs
            .session
            .as_ref()
            .is_some_and(|d| d.shells_loaded())
    });
}

#[test]
fn ssh_password_session_create_edit_and_switch_to_key() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    h.state_mut().tabs.sidebar_collapsed = false;
    h.run_steps(2);
    open_new(h);
    shot(h, "session-new");

    // Validation: name first.
    h.get_by_label("Create session").click();
    h.run_steps(3);
    assert!(alert(h, "Name is required"));
    fill(h, Role::TextInput, "Name", "prod");
    fill(h, Role::TextInput, "Host", "example.com");
    fill(h, Role::TextInput, "Username", "root");
    click(h, Role::RadioButton, "Password");
    h.get_by_label("Create session").click();
    h.run_steps(3);
    assert!(
        alert(h, "Password is required"),
        "a new password session needs one"
    );
    fill(h, Role::PasswordInput, "Password", "hunter2");
    shot(h, "session-new-password");
    // Ctrl+Enter saves.
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Enter);
    wait(h, 10, "saved", |a| {
        a.dialogs.session.is_none() && a.sidebar.sessions.iter().any(|s| s.name == "prod")
    });

    let s = session_named(h, "prod");
    assert_eq!(
        (
            s.auth_type.as_str(),
            s.host.as_str(),
            s.port,
            s.username.as_str()
        ),
        ("password", "example.com", 22, "root")
    );
    let cred = s.credential_id.expect("password bound");
    assert_eq!(
        owner_of(h, cred),
        Some(s.id),
        "the password row belongs to the session"
    );
    assert!(
        shared_list(h).is_empty(),
        "passwords are not in the shared list"
    );

    // Edit with the password left blank: kept.
    clear_toasts(h);
    h.state_mut().dialogs.session = Some(SessionDialog::edit(s.clone()));
    h.run_steps(3);
    shot(h, "session-edit-password");
    fill(h, Role::TextInput, "Name", "-2");
    h.get_by_label("Save changes").click();
    wait(h, 10, "edit saved", |a| {
        a.dialogs.session.is_none() && a.sidebar.sessions.iter().any(|s| s.name == "prod-2")
    });
    let s2 = db_session(h, s.id);
    assert_eq!(
        s2.credential_id,
        Some(cred),
        "blank password keeps the stored row"
    );
    assert_eq!(owner_of(h, cred), Some(s.id));

    // Switch to a key added inline through the picker.
    clear_toasts(h);
    h.state_mut().dialogs.session = Some(SessionDialog::edit(s2.clone()));
    h.run_steps(3);
    click(h, Role::RadioButton, "Private Key");
    let adds: Vec<_> = h.query_all_by_label("Add new").collect();
    assert_eq!(adds.len(), 2, "key and passphrase pickers");
    h.get_all_by_label("Add new").next().unwrap().click();
    h.run_steps(3);
    fill(h, Role::TextInput, "New credential label", "prod key");
    fill(
        h,
        Role::MultilineTextInput,
        "New credential secret",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----",
    );
    shot(h, "session-key-add");
    click(h, Role::Button, "Save credential");
    wait(h, 10, "key created and selected", |a| {
        a.creds.list.iter().any(|c| c.label == "prod key")
            && a.dialogs
                .session
                .as_ref()
                .is_some_and(|d| d.key_credential().is_some())
    });
    let key_id = h
        .state()
        .creds
        .list
        .iter()
        .find(|c| c.label == "prod key")
        .unwrap()
        .id;
    assert_eq!(
        h.state().dialogs.session.as_ref().unwrap().key_credential(),
        Some(key_id)
    );
    // Generation bumps (the reload) do not clear the fresh selection.
    wait(h, 5, "reload done", |a| !a.creds.loading);
    assert_eq!(
        h.state().dialogs.session.as_ref().unwrap().key_credential(),
        Some(key_id)
    );
    shot(h, "session-key-picked");
    // The manager button opens the manager over the dialog.
    h.get_all_by_label("Manage credentials")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert!(h.state().dialogs.credentials.is_some() && h.state().dialogs.session.is_some());
    h.state_mut().dialogs.credentials = None;
    h.run_steps(2);
    h.get_by_label("Save changes").click();
    wait(h, 10, "key saved", |a| a.dialogs.session.is_none());
    let s3 = db_session(h, s.id);
    assert_eq!(
        (s3.auth_type.as_str(), s3.credential_id),
        ("key", Some(key_id))
    );
    assert_eq!(
        owned_rows(h, s.id),
        0,
        "switching to a key drops the session password"
    );
    assert_eq!(
        shared_list(h),
        vec![("private_key".to_string(), "prod key".to_string())]
    );

    // A credential that vanished is cleared from the picker.
    clear_toasts(h);
    h.state_mut().dialogs.session = Some(SessionDialog::edit(s3));
    h.run_steps(3);
    assert_eq!(
        h.state().dialogs.session.as_ref().unwrap().key_credential(),
        Some(key_id)
    );
    h.state()
        .core
        .block_on(|st| async move { api::credentials::credential_delete(&st, key_id).await })
        .expect("delete");
    {
        let app = h.state_mut();
        app.creds.reload(&app.core);
    }
    wait(h, 5, "selection cleared", |a| {
        a.dialogs
            .session
            .as_ref()
            .is_some_and(|d| d.key_credential().is_none())
    });
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().dialogs.session.is_none(), "Escape closes");
}

#[test]
fn wsl_local_validation_panes_and_escape() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    h.state_mut().tabs.sidebar_collapsed = false;
    h.run_steps(2);

    // WSL.
    open_new(h);
    click(h, Role::RadioButton, "WSL");
    fill(h, Role::TextInput, "Name", "My WSL");
    fill(h, Role::TextInput, "Distro", "Ubuntu-24.04");
    shot(h, "session-wsl");
    h.get_by_label("Create session").click();
    wait(h, 10, "wsl saved", |a| {
        a.dialogs.session.is_none() && a.sidebar.sessions.iter().any(|s| s.name == "My WSL")
    });
    let s = session_named(h, "My WSL");
    assert_eq!(
        (
            s.session_kind.as_str(),
            s.host.as_str(),
            s.auth_type.as_str(),
            s.port
        ),
        ("wsl", "Ubuntu-24.04", "agent", 0)
    );

    // Local: the first detected shell is preselected.
    open_new(h);
    click(h, Role::RadioButton, "Local Shell");
    fill(h, Role::TextInput, "Name", "My shell");
    shot(h, "session-local");
    h.get_by_label("Create session").click();
    wait(h, 10, "local saved", |a| {
        a.dialogs.session.is_none() && a.sidebar.sessions.iter().any(|s| s.name == "My shell")
    });
    let s = session_named(h, "My shell");
    assert_eq!(s.session_kind, "local");
    assert!(!s.host.is_empty(), "a detected shell program");

    // SSH validation, the other panes, env rows, Escape.
    open_new(h);
    fill(h, Role::TextInput, "Name", "bad");
    h.get_by_label("Create session").click();
    h.run_steps(3);
    assert!(alert(h, "Host is required"));
    fill(h, Role::TextInput, "Host", "h");
    fill(h, Role::TextInput, "Username", "u");
    replace(h, "Port", "70000");
    h.get_by_label("Create session").click();
    h.run_steps(3);
    assert!(alert(h, "Port must be between 1 and 65535"));
    shot(h, "session-validation");
    replace(h, "Port", "2222");

    h.get_by_role_and_label(Role::Tab, "Terminal").click();
    h.run_steps(3);
    shot(h, "session-terminal");
    replace(h, "Font size", "7");
    h.get_by_label("Create session").click();
    h.run_steps(3);
    assert!(alert(h, "Font size must be between 8 and 48"));
    replace(h, "Font size", "14");
    click(h, Role::Button, "Add variable");
    h.get_by_label("Create session").click();
    h.run_steps(3);
    assert!(alert(h, "Env var name cannot be empty"));
    fill(h, Role::TextInput, "env name 1", "LANG");
    fill(h, Role::TextInput, "env value 1", "C.UTF-8");
    click(h, Role::Button, "Bar");
    shot(h, "session-terminal-env");

    h.get_by_role_and_label(Role::Tab, "Advanced").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::CheckBox, "Enable SSH compression (zlib)")
        .click();
    h.run_steps(2);
    shot(h, "session-advanced");
    h.get_by_role_and_label(Role::Tab, "Forwards").click();
    h.run_steps(3);
    shot(h, "session-forwards");

    // Light theme.
    h.state_mut().set_theme(Theme::Light);
    h.get_by_role_and_label(Role::Tab, "General").click();
    h.run_steps(3);
    shot(h, "session-general-light");
    h.get_by_role_and_label(Role::Tab, "Terminal").click();
    h.run_steps(3);
    shot(h, "session-terminal-light");

    h.get_by_label("Create session").click();
    wait(h, 10, "ssh saved", |a| {
        a.dialogs.session.is_none() && a.sidebar.sessions.iter().any(|s| s.name == "bad")
    });
    let s = session_named(h, "bad");
    assert_eq!(
        (s.port, s.font_size, s.cursor_style.as_str(), s.compression),
        (2222, 14, "bar", 1)
    );
    let id = s.id;
    let env_rows = h
        .state()
        .core
        .block_on(|st| async move { api::sessions::session_env_get(&st, id).await })
        .expect("env");
    assert_eq!(
        env_rows
            .iter()
            .map(|p| (p.key.as_str(), p.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("LANG", "C.UTF-8")]
    );

    // Escape and Cancel close without saving.
    open_new(h);
    fill(h, Role::TextInput, "Name", "never");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    if h.state().dialogs.session.is_some() {
        // The first Escape may only blur the text field.
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
    }
    assert!(h.state().dialogs.session.is_none(), "Escape closes");
    open_new(h);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialogs.session.is_none(), "Cancel closes");
    assert!(!h.state().sidebar.sessions.iter().any(|s| s.name == "never"));
}

fn ssh_input(name: &str, password: Option<&str>) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: name.into(),
        // Nothing listens on port 1: connects fail fast.
        host: "127.0.0.1".into(),
        port: 1,
        username: "root".into(),
        auth_type: "password".into(),
        credential_id: None,
        key_passphrase_credential_id: None,
        color: None,
        initial_command: None,
        scrollback_lines: 1000,
        font_size: 13,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 60,
        connect_timeout_secs: 5,
        env: vec![ezssh_core::db::sessions::EnvPair {
            key: "LANG".into(),
            value: "C".into(),
        }],
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: password.map(|p| ezssh_core::Secret::new(p.into())),
    }
}

/// Open a tab for an SSH session that fails to connect, then put up the
/// overlay as an auth failure would.
fn tab_with_auth_fix(h: &mut H, session: Session) -> u64 {
    h.state_mut().open_session(session.clone());
    let tab = h.state().tabs.active.expect("tab");
    wait(h, 15, "connect failed", move |a| {
        a.tabs
            .get(tab)
            .is_some_and(|t| t.status == TabStatus::Error)
    });
    h.state_mut().terms.get_mut(&tab).unwrap().auth_fix = Some(AuthFix::new(
        tab,
        session,
        "Authentication failed (password)".into(),
    ));
    h.run_steps(3);
    tab
}

#[test]
fn auth_fix_overlay_saves_password_and_reconnects() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let s = h
        .state()
        .core
        .block_on(|st| async move {
            api::sessions::session_create(&st, ssh_input("box", Some("old-pw"))).await
        })
        .expect("session");
    let old_cred = s.credential_id.expect("password stored");
    assert_eq!(secret_b64(h, old_cred), "b2xkLXB3", "\"old-pw\"");
    let tab = tab_with_auth_fix(h, s.clone());
    assert!(h.query_by_label("Fix authentication").is_some());
    assert!(
        h.query_by_label("Authentication failed (password)")
            .is_some()
    );
    shot(h, "auth-fix");

    // Username + a new password, then Save & reconnect.
    replace(h, "Username", "admin");
    fill(h, Role::PasswordInput, "Password", "new-pw");
    h.get_by_label("Save & reconnect").click();
    wait(h, 10, "saved and reconnecting", move |a| {
        a.terms.get(&tab).is_some_and(|s| s.auth_fix.is_none())
            && a.tabs
                .get(tab)
                .is_some_and(|t| t.session.username == "admin")
    });
    let row = db_session(h, s.id);
    assert_eq!(row.username, "admin");
    let cred = row.credential_id.expect("still a password session");
    assert_eq!(
        cred, old_cred,
        "the session's own row was rewritten in place"
    );
    assert_eq!(owner_of(h, cred), Some(s.id));
    let id = s.id;
    let env_rows = h
        .state()
        .core
        .block_on(|st| async move { api::sessions::session_env_get(&st, id).await })
        .expect("env");
    assert_eq!(env_rows.len(), 1, "env kept through session_update");
    assert_eq!(
        secret_b64(h, cred),
        "bmV3LXB3",
        "the new password (\"new-pw\") is stored"
    );

    // The retry fails again (nothing listens); the overlay can come back
    // and Close tab closes the tab.
    wait(h, 15, "retry failed", move |a| {
        a.tabs
            .get(tab)
            .is_some_and(|t| t.status == TabStatus::Error)
    });
    let updated = h.state().tabs.get(tab).unwrap().session.clone();
    h.state_mut().terms.get_mut(&tab).unwrap().auth_fix =
        Some(AuthFix::new(tab, updated, "Authentication failed".into()));
    h.run_steps(3);
    click(h, Role::RadioButton, "Private Key");
    h.get_by_label("Save & reconnect").click();
    h.run_steps(3);
    assert!(alert(h, "Pick or add a credential"));
    h.state_mut().set_theme(Theme::Light);
    h.run_steps(2);
    shot(h, "auth-fix-light-key");
    // The tab strip's ✕ shares the label; the overlay's button is lower.
    let buttons: Vec<_> = h.query_all_by_label("Close tab").collect();
    buttons
        .iter()
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("overlay button")
        .click();
    h.run_steps(3);
    assert!(h.state().tabs.get(tab).is_none(), "Close tab closes it");
}

/// An auth-fix overlay belongs to its tab: other tabs keep the keyboard,
/// and locking drops the overlay (it holds typed secrets).
#[test]
fn auth_fix_overlay_is_per_tab_and_cleared_by_lock() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let a = open_local(h);
    // A second tab of the same session (the sidebar collapsed on the first).
    let session = h.state().tabs.get(a).unwrap().session.clone();
    h.state_mut().open_session(session);
    let b = h.state().tabs.active.unwrap();
    h.run_steps(3);
    let session = h.state().tabs.get(a).unwrap().session.clone();
    h.state_mut().terms.get_mut(&a).unwrap().auth_fix =
        Some(AuthFix::new(a, session, "Authentication failed".into()));
    h.run_steps(3);
    assert!(
        !h.state().any_modal_open(),
        "another tab's overlay is not a window-wide modal"
    );
    assert_eq!(h.state().tabs.active, Some(b));
    h.state_mut().lock();
    h.run_steps(2);
    assert!(
        h.state().terms.values().all(|s| s.auth_fix.is_none()),
        "lock clears overlays"
    );
}

#[test]
fn session_terminal_options_saved_apply_to_open_tab() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    h.state_mut().tabs.sidebar_collapsed = false;
    h.run_steps(2);
    let tab = open_local(h);
    let s = h.state().tabs.get(tab).expect("tab").session.clone();
    assert_ne!(s.font_size, 20);

    // Font size through the dialog: the open tab follows on save.
    clear_toasts(h);
    h.state_mut().dialogs.session = Some(SessionDialog::edit(s.clone()));
    h.run_steps(3);
    h.get_by_role_and_label(Role::Tab, "Terminal").click();
    h.run_steps(3);
    replace(h, "Font size", "20");
    replace(h, "Scrollback lines", "123");
    click(h, Role::Button, "Bar");
    let blink = h.state().terms[&tab].term.options().cursor_blink;
    h.get_by_label("Save changes").click();
    wait(h, 10, "edit saved", |a| a.dialogs.session.is_none());
    let row = db_session(h, s.id);
    assert_eq!(
        (
            row.font_size,
            row.scrollback_lines,
            row.cursor_style.as_str()
        ),
        (20, 123, "bar")
    );
    let ts = &h.state().tabs.get(tab).unwrap().session;
    assert_eq!(
        (ts.font_size, ts.scrollback_lines, ts.cursor_style.as_str()),
        (20, 123, "bar")
    );
    let term = &h.state().terms[&tab].term;
    assert_eq!(term.font_size(), 20.0, "terminal took the saved size");
    assert_eq!(
        term.options().scrollback,
        123,
        "terminal took the saved scrollback"
    );
    assert_eq!(
        term.options().cursor,
        ezssh_term::CursorKind::Bar,
        "terminal took the saved cursor"
    );
    assert_eq!(
        term.options().cursor_blink,
        blink,
        "blink is the tab's own setting"
    );

    // Family: the terminal gets the saved family's faces.
    let family = crate::font_presets::choices()
        .iter()
        .map(|(v, _)| v.clone())
        .find(|v| !v.is_empty());
    if let Some(family) = family {
        let saved = Session {
            font_family: family.clone(),
            ..db_session(h, s.id)
        };
        let want = {
            let app = h.state_mut();
            let ctx = app.ctx.clone();
            app.fonts.faces(&ctx, &family)
        };
        h.state_mut().apply_session_terminal(&saved);
        h.run_steps(2);
        assert_eq!(h.state().tabs.get(tab).unwrap().session.font_family, family);
        assert_eq!(
            h.state().terms[&tab].term.faces(),
            &want,
            "terminal took the saved family"
        );
    }

    // A save that leaves the font alone keeps a tab-only change.
    h.state_mut()
        .terms
        .get_mut(&tab)
        .unwrap()
        .term
        .set_font_size(11.0);
    let same = h.state().tabs.get(tab).unwrap().session.clone();
    h.state_mut().apply_session_terminal(&Session {
        name: "renamed".into(),
        ..same
    });
    h.run_steps(2);
    assert_eq!(
        h.state().terms[&tab].term.font_size(),
        11.0,
        "unchanged font keeps the live size"
    );
}
