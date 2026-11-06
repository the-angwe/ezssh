//! Tab order of the dialogs: press Tab round each one and check that the
//! keyboard follows the layout. egui moves focus in creation order, so a
//! right-to-left row (the old footers), a header ✕ created before the
//! body or a toast above the modal shows up here.
//!
//! Every cycle must: stay inside the dialog (no toast "Dismiss"), give a
//! number field one stop (no "Increase …" / "Decrease …" stepper), and
//! visit stops on one row left to right. A few dialogs also pin the exact
//! sequence: footer buttons in visual order, the ✕ last.

use egui::{Key, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};

use super::*;
use crate::app::App;

/// One Tab stop: role, label (or value) and top-left corner.
#[derive(Clone, Debug, PartialEq)]
struct Stop {
    role: Role,
    label: String,
    x: f32,
    y: f32,
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} '{}' @({:.0},{:.0})",
            self.role, self.label, self.x, self.y
        )
    }
}

fn focused(h: &Harness<'static, App>) -> Option<Stop> {
    let n = h.query(by().predicate(|n| n.is_focused()))?;
    let r = n.rect();
    let ak = n.accesskit_node();
    Some(Stop {
        role: ak.role(),
        label: ak.label().or_else(|| n.value()).unwrap_or_default(),
        x: r.min.x,
        y: r.min.y,
    })
}

/// Press Tab until the focus comes back round (at most 60 presses).
fn tab_cycle(h: &mut Harness<'static, App>) -> Vec<Stop> {
    let mut seen: Vec<Stop> = Vec::new();
    for _ in 0..60 {
        h.key_press(Key::Tab);
        h.run_steps(2);
        let Some(f) = focused(h) else {
            break;
        };
        if seen.first() == Some(&f) {
            break;
        }
        seen.push(f);
    }
    seen
}

fn show(cycle: &[Stop]) -> String {
    cycle
        .iter()
        .enumerate()
        .map(|(i, s)| format!("\n  {i:2} {s}"))
        .collect()
}

/// Walk the open dialog's Tab cycle and check the general rules. The
/// cycle is rotated to start after the ✕ when there is one, so it reads
/// first control → … → ✕.
fn check(h: &mut Harness<'static, App>, name: &str) -> Vec<Stop> {
    h.run_steps(4);
    // Start from nothing focused: the cycle then begins at the first stop.
    h.state().ctx.memory_mut(|m| {
        if let Some(f) = m.focused() {
            m.surrender_focus(f)
        }
    });
    h.run_steps(2);
    shot(h, &format!("focus-{name}"));
    let mut cycle = tab_cycle(h);
    if let Some(i) = cycle.iter().position(|s| s.label == "Close dialog") {
        cycle.rotate_left(i + 1);
    }
    let dump = show(&cycle);
    tracing::debug!("{name}: tab cycle{dump}");
    assert!(!cycle.is_empty(), "{name}: nothing takes the keyboard");
    for s in &cycle {
        assert!(
            s.label != "Dismiss",
            "{name}: Tab leaves the dialog for a toast:{dump}"
        );
        assert!(
            !s.label.starts_with("Increase ") && !s.label.starts_with("Decrease "),
            "{name}: a number stepper is a Tab stop:{dump}"
        );
    }
    for w in cycle.windows(2) {
        if (w[0].y - w[1].y).abs() < 2.0 {
            assert!(
                w[1].x > w[0].x,
                "{name}: '{}' comes after '{}' but sits left of it:{dump}",
                w[1].label,
                w[0].label
            );
        }
    }
    cycle
}

fn labels(cycle: &[Stop]) -> Vec<&str> {
    cycle.iter().map(|s| s.label.as_str()).collect()
}

/// `cycle` ends with exactly `tail`.
fn assert_tail(name: &str, cycle: &[Stop], tail: &[&str]) {
    let l = labels(cycle);
    assert!(
        l.ends_with(tail),
        "{name}: expected the cycle to end {tail:?}:{}",
        show(cycle)
    );
}

fn close_all(h: &mut Harness<'static, App>) {
    h.state_mut().dialogs = Default::default();
    h.run_steps(3);
}

#[test]
fn dialogs_tab_in_visual_order() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    assert!(
        !h.state().toasts.items().is_empty(),
        "start-up toasts are up (the cycles must not reach them)"
    );

    for section in ["Appearance", "Terminal", "Sessions", "Updates", "Security"] {
        h.state_mut().dialogs.settings = Some(Default::default());
        h.run_steps(3);
        h.get_by_role_and_label(Role::Tab, section).click();
        h.run_steps(3);
        let c = check(h, &format!("settings-{section}"));
        assert_eq!(
            labels(&c)[..5],
            ["Appearance", "Terminal", "Sessions", "Updates", "Security"],
            "settings: the rail comes first:{}",
            show(&c)
        );
        assert_tail("settings", &c, &["Close dialog"]);
        close_all(h);
    }

    // The stepper is out of the Tab order; ↑ / ↓ in the field step instead.
    h.state_mut().dialogs.settings = Some(Default::default());
    h.run_steps(3);
    h.get_by_role_and_label(Role::Tab, "Terminal").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Scrollback lines")
        .focus();
    h.run_steps(2);
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(
        h.state().settings.int("terminal.scrollback"),
        5001,
        "↑ steps"
    );
    h.key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(
        h.state().settings.int("terminal.scrollback"),
        4991,
        "Shift+↓ steps by 10"
    );
    close_all(h);

    for pane in ["General", "Terminal", "Advanced", "Forwards"] {
        h.state_mut().dialogs.session = Some(
            crate::views::session_dialog::SessionDialog::new_session(None),
        );
        h.run_steps(3);
        h.get_by_role_and_label(Role::Tab, pane).click();
        h.run_steps(3);
        let c = check(h, &format!("session-{pane}"));
        assert_eq!(labels(&c)[0], "General", "session: rail first:{}", show(&c));
        assert_tail("session", &c, &["Cancel", "Create session", "Close dialog"]);
        close_all(h);
    }

    type Opener = fn(&mut App);
    let simple: Vec<(&str, Opener, &[&str])> = vec![
        (
            "credentials",
            |a| a.dialogs.credentials = Some(Default::default()),
            &[
                "Add SSH keys",
                "Add Key passphrases",
                "Close",
                "Close dialog",
            ],
        ),
        (
            "change-password",
            |a| a.dialogs.change_password = Some(Default::default()),
            &[
                "Current password",
                "Show secret",
                "New password",
                "Confirm new password",
                "Cancel",
                "Close dialog",
            ],
        ),
        (
            "recovery-code",
            |a| a.dialogs.recovery_code = Some(Default::default()),
            &["Cancel", "Close dialog"],
        ),
        (
            "backup",
            |a| a.dialogs.backup = Some(Default::default()),
            &["Master password", "Show secret", "Cancel", "Close dialog"],
        ),
        (
            "sync",
            |a| a.dialogs.sync = Some(Default::default()),
            &["Confirm passphrase", "Enable sync", "Close dialog"],
        ),
        (
            "tile-grid",
            |a| {
                a.dialogs.tile_grid =
                    Some(crate::views::dialogs::TileGridDraft::new(a.tabs.tile_grid))
            },
            &["Rows", "Columns", "Cancel", "Apply"],
        ),
        (
            "quit-confirm",
            |a| a.dialogs.quit_confirm = true,
            &["Cancel", "Quit"],
        ),
    ];
    for (name, open, tail) in simple {
        open(h.state_mut());
        let c = check(h, name);
        assert_tail(name, &c, tail);
        close_all(h);
    }

    sidebar_new(h, "New folder");
    let c = check(h, "new-folder");
    assert_eq!(labels(&c), ["Folder name", "Cancel", "Create"]);
    h.key_press(Key::Escape);
    h.run_steps(3);

    let tab = open_local(h);
    crate::views::appearance::open(h.state_mut(), tab);
    let c = check(h, "appearance");
    assert_tail(
        "appearance",
        &c,
        &["Cancel", "Apply to tab", "Save to session", "Close dialog"],
    );
    close_all(h);
    h.state_mut().dialogs.paste = Some((tab, "a\nb".into(), 2));
    let c = check(h, "paste");
    assert_eq!(labels(&c), ["Cancel", "Paste"]);
    close_all(h);
}

fn ssh_input(name: &str) -> ezssh_core::db::sessions::SessionInput {
    ezssh_core::db::sessions::SessionInput {
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
        env: vec![],
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: Some(ezssh_core::Secret::new("pw".into())),
    }
}

/// The dialogs that need data: credential rows, files to import and
/// restore, a saved SSH session, sidebar folders.
#[test]
fn data_dialogs_tab_in_visual_order() {
    use ezssh_core::api;
    let mut env = start();
    unlock_fresh(&mut env);
    let dir = env._dir.path().to_path_buf();
    let h = &mut env.h;

    h.state()
        .core
        .block_on(|st| async move {
            api::credentials::credential_create(
                &st,
                "private_key".into(),
                "Prod key".into(),
                "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n".into(),
            )
            .await
        })
        .unwrap();
    h.state_mut().dialogs.credentials = Some(Default::default());
    h.run_steps(10);
    let c = check(h, "credentials-row");
    assert_eq!(
        labels(&c),
        [
            "Add SSH keys",
            "Rename Prod key",
            "Replace secret for Prod key",
            "Delete Prod key",
            "Add Key passphrases",
            "Close",
            "Close dialog"
        ]
    );
    h.get_by_label("Add SSH keys").click();
    h.run_steps(4);
    let c = check(h, "credentials-add");
    let l = labels(&c);
    let at = |s: &str| l.iter().position(|x| *x == s).unwrap_or(usize::MAX);
    assert!(
        at("New SSH keys label") < at("New SSH keys secret")
            && at("New SSH keys secret") < at("Cancel")
            && at("Cancel") < at("Add Key passphrases"),
        "credentials add form:{}",
        show(&c)
    );
    close_all(h);

    let moba = dir.join("s.mxtsessions");
    std::fs::write(&moba, super::data::MOBA_FIXTURE).unwrap();
    h.state_mut().dialogs.import = Some(crate::views::import::ImportDialog::from_path(moba));
    wait(h, 10, "import preview", |a| {
        a.dialogs
            .import
            .as_ref()
            .is_some_and(|d| d.preview.is_some())
    });
    let c = check(h, "import");
    assert_tail("import", &c, &["Cancel", "Import", "Close dialog"]);
    close_all(h);

    h.state_mut().dialogs.backup = Some(Default::default());
    h.run_steps(4);
    type_into(h, 0, PASSWORD);
    h.get_by_label("Continue").click();
    wait(h, 20, "passphrase step", |a| {
        a.dialogs
            .backup
            .as_ref()
            .is_some_and(|d| d.step == crate::views::backup::Step::Passphrase)
    });
    let c = check(h, "backup-passphrase");
    assert_tail(
        "backup-passphrase",
        &c,
        &["Confirm passphrase", "Cancel", "Close dialog"],
    );
    let file = dir.join("b.ezbackup");
    type_into(h, 0, "backup-pass-123");
    type_into(h, 1, "backup-pass-123");
    h.state_mut().dialogs.backup.as_mut().unwrap().save_to = Some(file.clone());
    h.get_by_label("Save backup…").click();
    wait(h, 20, "backup written", |a| a.dialogs.backup.is_none());
    h.state_mut().dialogs.restore = Some(crate::views::restore::RestoreDialog::from_path(file));
    h.run_steps(4);
    type_into(h, 0, "backup-pass-123");
    h.get_by_label("Unlock backup").click();
    wait(h, 20, "restore preview", |a| {
        a.dialogs
            .restore
            .as_ref()
            .is_some_and(|d| d.step == crate::views::restore::Step::Select)
    });
    let c = check(h, "restore-select");
    assert_eq!(labels(&c)[0], "Select no folders", "{}", show(&c));
    assert_tail(
        "restore-select",
        &c,
        &["Cancel", "Import selected", "Close dialog"],
    );
    close_all(h);

    let s = h
        .state()
        .core
        .block_on(|st| async move { api::sessions::session_create(&st, ssh_input("box")).await })
        .unwrap();
    h.state_mut().dialogs.session = Some(crate::views::session_dialog::SessionDialog::edit(s));
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Forwards").click();
    h.run_steps(6);
    let c = check(h, "session-edit-forwards");
    assert_tail(
        "session-edit-forwards",
        &c,
        &["Add forward", "Cancel", "Save changes", "Close dialog"],
    );
    h.get_by_label("Add forward").click();
    h.run_steps(6);
    let c = check(h, "forward-dialog");
    assert_eq!(labels(&c)[0], "Local (-L)", "{}", show(&c));
    assert_tail("forward-dialog", &c, &["Cancel", "Save", "Close dialog"]);
    h.state_mut().forwards.close_dialogs();
    close_all(h);

    sidebar_new(h, "New folder");
    h.get_by_role_and_label(Role::TextInput, "Folder name")
        .type_text("Work");
    h.key_press(Key::Enter);
    h.run_steps(6);
    right_click(
        h,
        h.get_by_role_and_label(Role::TreeItem, "Work")
            .rect()
            .center(),
    );
    h.get_by_label("Rename").click();
    h.run_steps(3);
    let c = check(h, "rename-folder");
    assert_eq!(labels(&c), ["Folder name", "Cancel", "Rename"]);
    h.key_press(Key::Escape);
    h.run_steps(3);
    right_click(
        h,
        h.get_by_role_and_label(Role::TreeItem, "box")
            .rect()
            .center(),
    );
    h.get_by_label("Move to folder…").click();
    h.run_steps(3);
    let c = check(h, "move-session");
    // "box" is at the root already: that row is disabled, no stop.
    assert_eq!(
        labels(&c),
        ["Local Shells", "Work", "Cancel"],
        "{}",
        show(&c)
    );
    h.key_press(Key::Escape);
    h.run_steps(3);
}
