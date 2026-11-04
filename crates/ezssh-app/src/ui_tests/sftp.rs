//! The file pane against the in-process SSH / SFTP server
//! (`ezssh-testsrv`): open, list, select, navigate (double-click,
//! breadcrumb), OS drops of files and folders with progress, mkdir,
//! rename, delete, download, a listing error, and reopening in the same
//! folder after a reconnect.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ezssh_core::api;
use ezssh_core::db::sessions::SessionInput;
use ezssh_testsrv::{Config, TestServer};

use super::{shot, start, unlock_fresh, wait};
use crate::app::App;
use crate::tabs::TabStatus;

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn ssh_input(port: u16) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: "files".into(),
        host: "127.0.0.1".into(),
        port: port.into(),
        username: "u".into(),
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
        keepalive_secs: 0,
        connect_timeout_secs: 5,
        env: Vec::new(),
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: Some(ezssh_core::Secret::new("pw".into())),
    }
}

fn press(h: &mut Harness<'static, App>, pos: Pos2, button: PointerButton) {
    press_with(h, pos, button, Modifiers::NONE);
}

fn press_with(
    h: &mut Harness<'static, App>,
    pos: Pos2,
    button: PointerButton,
    modifiers: Modifiers,
) {
    // egui 0.37 takes the held modifiers from ModifiersChanged events.
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button,
        pressed: true,
        modifiers,
    });
    h.event(Event::PointerButton {
        pos,
        button,
        pressed: false,
        modifiers,
    });
    h.step();
    if modifiers != Modifiers::NONE {
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.step();
    }
}

fn row_center(h: &Harness<'static, App>, name: &str) -> Pos2 {
    h.get_by_role_and_label(Role::Row, name).rect().center()
}

fn double_click_row(h: &mut Harness<'static, App>, name: &str) {
    h.run_steps(30); // out of the previous multi-click window
    let c = row_center(h, name);
    press(h, c, PointerButton::Primary);
    press(h, c, PointerButton::Primary);
}

fn click_row(h: &mut Harness<'static, App>, name: &str, mods: Modifiers) {
    h.run_steps(30);
    let c = row_center(h, name);
    press_with(h, c, PointerButton::Primary, mods);
}

fn menu(h: &mut Harness<'static, App>, row: &str, item: &str) {
    h.run_steps(30);
    let c = row_center(h, row);
    press(h, c, PointerButton::Secondary);
    h.run_steps(2);
    h.get_by_role_and_label(Role::MenuItem, item).click();
    h.run_steps(2);
}

fn listing(h: &Harness<'static, App>, tab: u64) -> Option<(String, Vec<String>)> {
    h.state().sftp.listing(tab)
}

fn wait_listing(h: &mut Harness<'static, App>, tab: u64, dir: &str, names: &[&str]) {
    let want = (
        dir.to_string(),
        names.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    );
    let until = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if listing(h, tab).as_ref() == Some(&want) && !h.state().sftp.loading(tab) {
            h.step();
            return;
        }
        assert!(
            std::time::Instant::now() < until,
            "listing never became {want:?}; is {:?}",
            listing(h, tab)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn file_pane_end_to_end() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let dir = tempfile::tempdir().expect("tempdir");
    let remote = dir.path().join("remote");
    std::fs::create_dir_all(remote.join("home/u/sub")).unwrap();
    std::fs::write(remote.join("home/u/hello.txt"), b"hello").unwrap();
    std::fs::write(remote.join("home/u/notes.md"), vec![b'n'; 2048]).unwrap();
    std::fs::write(remote.join("home/u/sub/inner.txt"), b"inner").unwrap();
    let local = dir.path().join("local");
    std::fs::create_dir_all(local.join("tree/deep")).unwrap();
    std::fs::write(local.join("one.bin"), vec![1u8; 4096]).unwrap();
    std::fs::write(local.join("tree/a.txt"), b"a").unwrap();
    std::fs::write(local.join("tree/deep/b.txt"), b"b").unwrap();
    // Slow enough writes to see a transfer mid-way.
    std::fs::write(local.join("big.bin"), vec![7u8; 3 * 1024 * 1024]).unwrap();

    let mut cfg = Config::new(&remote, "u", "pw");
    cfg.home = "/home/u".into();
    cfg.op_delay = Duration::from_millis(8);
    let srv = rt
        .block_on(TestServer::start(cfg, "127.0.0.1:0".parse().unwrap()))
        .expect("test server");

    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let input = ssh_input(srv.port());
    let session = h
        .state()
        .core
        .block_on(|st| async move { api::sessions::session_create(&st, input).await })
        .expect("session");
    h.state_mut().open_session(session);
    let tab = h.state().tabs.active.expect("tab");
    wait(h, 20, "host key prompt", move |a| {
        a.terms.get(&tab).is_some_and(|s| s.host_key.is_some())
    });
    h.get_by_label("Trust and connect").click();
    wait(h, 20, "connected", move |a| {
        a.tabs
            .get(tab)
            .is_some_and(|t| t.status == TabStatus::Connected)
    });

    // Open the pane from the tab strip.
    h.get_by_label("Show file pane").click();
    wait_listing(h, tab, "/home/u", &["sub", "hello.txt", "notes.md"]);
    assert_eq!(h.state().tabs.get(tab).unwrap().cwd, "/home/u");
    shot(h, "sftp-pane");

    // Selection: click, ctrl-click, shift-click.
    click_row(h, "hello.txt", Modifiers::NONE);
    assert_eq!(h.state().sftp.selected(tab), ["/home/u/hello.txt"]);
    click_row(h, "notes.md", Modifiers::COMMAND);
    assert_eq!(
        h.state().sftp.selected(tab),
        ["/home/u/hello.txt", "/home/u/notes.md"]
    );
    // Shift extends from the anchor (the ctrl-clicked notes.md) up to sub.
    click_row(h, "sub", Modifiers::SHIFT);
    assert_eq!(
        h.state().sftp.selected(tab),
        ["/home/u/hello.txt", "/home/u/notes.md", "/home/u/sub"]
    );
    shot(h, "sftp-selection");

    // Two files selected: one Download entry for both, run through the
    // test hook (no native folder dialog headless).
    // A plain click inside the selection keeps it; ctrl-click drops sub.
    click_row(h, "hello.txt", Modifiers::NONE);
    assert_eq!(h.state().sftp.selected(tab).len(), 3);
    click_row(h, "sub", Modifiers::COMMAND);
    assert_eq!(
        h.state().sftp.selected(tab),
        ["/home/u/hello.txt", "/home/u/notes.md"]
    );
    h.run_steps(30);
    press(h, row_center(h, "notes.md"), PointerButton::Secondary);
    h.run_steps(2);
    assert!(
        h.query_by_role_and_label(Role::MenuItem, "Download 2 files…")
            .is_some(),
        "multi download entry"
    );
    shot(h, "sftp-menu");
    h.event(Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    assert!(!h.state().sftp.menu_open());
    let downloads = dir.path().join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    crate::views::sftp::test_download(h.state_mut(), tab, &["hello.txt", "notes.md"], &downloads);
    wait(h, 20, "downloads done", |a| a.sftp.transfer_counts().1 == 2);
    assert_eq!(
        std::fs::read(downloads.join("hello.txt")).unwrap(),
        b"hello"
    );
    assert_eq!(
        std::fs::read(downloads.join("notes.md")).unwrap().len(),
        2048
    );

    // Navigate: double-click a folder, then the breadcrumb back.
    double_click_row(h, "sub");
    wait_listing(h, tab, "/home/u/sub", &["inner.txt"]);
    assert!(
        h.state().sftp.selected(tab).is_empty(),
        "selection belongs to the old folder"
    );
    h.get_by_role_and_label(Role::Link, "/home/u").click();
    wait_listing(h, tab, "/home/u", &["sub", "hello.txt", "notes.md"]);

    // OS drag over the window shows the drop overlay; a drop uploads a
    // file and a folder tree into the current folder.
    h.input_mut().hovered_files.push(egui::HoveredFile {
        path: Some(local.join("one.bin")),
        mime: String::new(),
    });
    h.run_steps(2);
    shot(h, "sftp-drop-overlay");
    h.input_mut().hovered_files.clear();
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(local.join("one.bin"))));
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(local.join("tree"))));
    h.step();
    wait(h, 20, "folder job finished", |a| {
        a.sftp.jobs().iter().any(|j| j.0 == "tree" && j.2 == 2)
    });
    assert_eq!(h.state().sftp.jobs(), [("tree".to_string(), 2, 2, 0)]);
    wait_listing(
        h,
        tab,
        "/home/u",
        &["sub", "tree", "hello.txt", "notes.md", "one.bin"],
    );
    assert_eq!(
        std::fs::read(remote.join("home/u/tree/deep/b.txt")).unwrap(),
        b"b"
    );
    assert_eq!(
        std::fs::read(remote.join("home/u/one.bin")).unwrap().len(),
        4096
    );
    shot(h, "sftp-after-drop");
    h.get_by_label("Dismiss upload tree/").click();
    h.run_steps(2);
    assert!(h.state().sftp.jobs().is_empty());

    // A folder of many files keeps the strip short.
    let many = local.join("many");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..40 {
        std::fs::write(many.join(format!("f{i:02}")), vec![0u8; 64 * 1024]).unwrap();
    }
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(many.clone())));
    h.step();
    wait(h, 20, "many-files job registered", |a| {
        a.sftp.jobs().iter().any(|j| j.0 == "many")
    });
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("more waiting").is_none(),
        "job files collapse into the job row"
    );
    shot(h, "sftp-many");
    wait(h, 60, "many-files job done", |a| {
        a.sftp.jobs().iter().any(|j| j.0 == "many" && j.2 == 40)
    });
    h.get_by_label("Dismiss upload many/").click();
    h.run_steps(2);
    std::fs::remove_dir_all(remote.join("home/u/many")).unwrap();
    // Many plain files: six rows, then one "more waiting" line.
    for i in 0..12 {
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped(many.join(format!("f{i:02}")))));
    }
    h.step();
    wait(h, 20, "plain files registered", |a| {
        a.sftp.transfer_counts().0 >= 6
    });
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("more waiting").is_some(),
        "long queues are summarised"
    );
    shot(h, "sftp-queue");
    wait(h, 60, "plain files done", |a| {
        a.sftp.transfer_counts().0 == 0
    });
    for i in 0..12 {
        std::fs::remove_file(remote.join(format!("home/u/f{i:02}"))).unwrap();
    }

    // A big upload shows its progress bar.
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(local.join("big.bin"))));
    h.step();
    wait(h, 20, "big upload reports progress", |a| {
        a.sftp.progress().first().is_some_and(|&n| n > 0)
    });
    shot(h, "sftp-progress");
    wait(h, 60, "big upload done", |a| {
        a.sftp.transfer_counts().0 == 0
    });
    assert_eq!(
        std::fs::metadata(remote.join("home/u/big.bin"))
            .unwrap()
            .len(),
        3 * 1024 * 1024
    );
    wait_listing(
        h,
        tab,
        "/home/u",
        &["sub", "tree", "big.bin", "hello.txt", "notes.md", "one.bin"],
    );

    // New directory: a name with / is refused in place, then created.
    h.get_by_label("New directory").click();
    h.run_steps(3);
    h.event(Event::Text("a/b".into()));
    h.get_by_role_and_label(Role::Button, "Create").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("A name cannot contain /").is_some(),
        "invalid name refused"
    );
    shot(h, "sftp-mkdir-invalid");
    // The field is focused with its text selected again: replace it.
    h.event(Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.event(Event::Text("made".into()));
    h.get_by_role_and_label(Role::Button, "Create").click();
    wait_listing(
        h,
        tab,
        "/home/u",
        &[
            "made",
            "sub",
            "tree",
            "big.bin",
            "hello.txt",
            "notes.md",
            "one.bin",
        ],
    );
    assert!(remote.join("home/u/made").is_dir());

    // Rename from the row menu (the name is preselected: typing replaces it).
    menu(h, "hello.txt", "Rename…");
    h.event(Event::Text("greeting.txt".into()));
    h.get_by_role_and_label(Role::Button, "Rename").click();
    wait_listing(
        h,
        tab,
        "/home/u",
        &[
            "made",
            "sub",
            "tree",
            "big.bin",
            "greeting.txt",
            "notes.md",
            "one.bin",
        ],
    );

    // Delete a file, and refuse to delete a non-empty folder.
    menu(h, "greeting.txt", "Delete");
    shot(h, "sftp-delete-confirm");
    h.get_by_role_and_label(Role::Button, "Delete").click();
    wait_listing(
        h,
        tab,
        "/home/u",
        &["made", "sub", "tree", "big.bin", "notes.md", "one.bin"],
    );
    menu(h, "sub", "Delete");
    h.get_by_role_and_label(Role::Button, "Delete").click();
    wait(h, 10, "delete failure toast", |a| {
        a.toasts.items().iter().any(|t| t.title == "Delete failed")
    });
    assert!(remote.join("home/u/sub/inner.txt").exists());

    // A folder that cannot be listed: the pane keeps its folder and shows
    // the error, which can be dismissed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            remote.join("home/u/made"),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        double_click_row(h, "made");
        wait(h, 10, "list error", move |a| {
            a.sftp.pane_error(tab).is_some()
        });
        assert_eq!(listing(h, tab).unwrap().0, "/home/u");
        assert_eq!(h.state().tabs.get(tab).unwrap().cwd, "/home/u");
        shot(h, "sftp-list-error");
        h.get_by_label("Dismiss error").click();
        h.run_steps(2);
        assert!(h.state().sftp.pane_error(tab).is_none());
        std::fs::set_permissions(
            remote.join("home/u/made"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    // Reconnect: the pane reopens on the new connection in the same folder.
    double_click_row(h, "sub");
    wait_listing(h, tab, "/home/u/sub", &["inner.txt"]);
    let old = h.state().tabs.get(tab).unwrap().connection_id;
    h.state_mut().connect_tab(tab, false, false);
    wait(h, 20, "reconnected", move |a| {
        a.tabs
            .get(tab)
            .is_some_and(|t| t.status == TabStatus::Connected && t.connection_id != old)
    });
    wait_listing(h, tab, "/home/u/sub", &["inner.txt"]);

    // Hiding and showing the pane lists again (picks up remote changes).
    std::fs::write(remote.join("home/u/sub/new.txt"), b"x").unwrap();
    h.get_by_label("Hide file pane").click();
    h.run_steps(3);
    h.get_by_label("Show file pane").click();
    wait_listing(h, tab, "/home/u/sub", &["inner.txt", "new.txt"]);
    drop(srv);
}

/// Runtime, server and an app with `tab` connected and its pane listing
/// `/home/u`.
struct Fixture {
    _rt: tokio::runtime::Runtime,
    _srv: TestServer,
    env: super::Env,
    tab: u64,
}

fn fixture(remote: &Path, op_delay: Duration) -> Fixture {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mut cfg = Config::new(remote, "u", "pw");
    cfg.home = "/home/u".into();
    cfg.op_delay = op_delay;
    let srv = rt
        .block_on(TestServer::start(cfg, "127.0.0.1:0".parse().unwrap()))
        .expect("test server");
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let input = ssh_input(srv.port());
    let session = h
        .state()
        .core
        .block_on(|st| async move { api::sessions::session_create(&st, input).await })
        .expect("session");
    h.state_mut().open_session(session);
    let tab = h.state().tabs.active.expect("tab");
    wait(h, 20, "host key prompt", move |a| {
        a.terms.get(&tab).is_some_and(|s| s.host_key.is_some())
    });
    h.get_by_label("Trust and connect").click();
    wait(h, 20, "connected", move |a| {
        a.tabs
            .get(tab)
            .is_some_and(|t| t.status == TabStatus::Connected)
    });
    h.get_by_label("Show file pane").click();
    wait(h, 20, "pane open", move |a| {
        a.sftp.listing(tab).is_some_and(|l| l.0 == "/home/u")
    });
    Fixture {
        _rt: rt,
        _srv: srv,
        env,
        tab,
    }
}

#[test]
fn folder_download_multi_delete_and_cancel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let remote = dir.path().join("remote");
    let home = remote.join("home/u");
    std::fs::create_dir_all(home.join("proj/src/lib")).unwrap();
    std::fs::create_dir_all(home.join("proj/empty")).unwrap();
    std::fs::create_dir_all(home.join("d1")).unwrap();
    std::fs::create_dir_all(home.join("d2")).unwrap();
    std::fs::write(home.join("d2/keep"), b"k").unwrap();
    std::fs::write(home.join("proj/a.txt"), b"a").unwrap();
    std::fs::write(home.join("proj/src/main.rs"), b"fn main() {}").unwrap();
    std::fs::write(home.join("proj/src/lib/x.rs"), b"x").unwrap();
    for f in ["f1", "f2", "f3"] {
        std::fs::write(home.join(f), f).unwrap();
    }
    std::fs::write(home.join("big.bin"), vec![3u8; 4 * 1024 * 1024]).unwrap();
    let local = dir.path().join("local");
    std::fs::create_dir_all(local.join("many")).unwrap();
    std::fs::write(local.join("up.bin"), vec![4u8; 4 * 1024 * 1024]).unwrap();
    for i in 0..5 {
        std::fs::write(local.join(format!("many/m{i}")), vec![1u8; 1024 * 1024]).unwrap();
    }
    for i in 0..10 {
        std::fs::write(local.join(format!("q{i}")), vec![2u8; 512 * 1024]).unwrap();
    }
    let downloads = dir.path().join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();

    let mut fx = fixture(&remote, Duration::from_millis(10));
    let (h, tab) = (&mut fx.env.h, fx.tab);
    wait_listing(
        h,
        tab,
        "/home/u",
        &["d1", "d2", "proj", "big.bin", "f1", "f2", "f3"],
    );

    // A folder has Download… too; it is recreated under the chosen folder.
    // (Choosing it opens the native folder dialog, which cannot run
    // headless: check the entry, then use the hook.)
    h.run_steps(30);
    press(h, row_center(h, "proj"), PointerButton::Secondary);
    h.run_steps(2);
    assert!(
        h.query_by_role_and_label(Role::MenuItem, "Download…")
            .is_some(),
        "folders can be downloaded"
    );
    h.event(Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    crate::views::sftp::test_download(h.state_mut(), tab, &["proj"], &downloads);
    wait(h, 20, "folder download done", |a| {
        a.sftp.job_states().iter().any(|j| j.0 == "proj" && j.4)
    });
    assert_eq!(
        h.state().sftp.job_states()[0],
        ("proj".to_string(), 3, 3, 0, true)
    );
    assert_eq!(
        std::fs::read(downloads.join("proj/src/lib/x.rs")).unwrap(),
        b"x"
    );
    assert_eq!(std::fs::read(downloads.join("proj/a.txt")).unwrap(), b"a");
    assert!(
        downloads.join("proj/empty").is_dir(),
        "empty folders come along"
    );
    shot(h, "sftp-folder-download");
    h.get_by_label("Dismiss download proj/").click();
    h.run_steps(2);

    // Multi-selection delete: two files and an empty folder.
    click_row(h, "f1", Modifiers::NONE);
    click_row(h, "f2", Modifiers::COMMAND);
    click_row(h, "d1", Modifiers::COMMAND);
    h.run_steps(30);
    press(h, row_center(h, "f2"), PointerButton::Secondary);
    h.run_steps(2);
    assert!(
        h.query_by_role_and_label(Role::MenuItem, "Download 3 items…")
            .is_some()
    );
    h.get_by_role_and_label(Role::MenuItem, "Delete 3 items")
        .click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Delete 3 items?").is_some(),
        "one confirmation for all"
    );
    shot(h, "sftp-delete-many");
    h.get_by_role_and_label(Role::Button, "Delete").click();
    wait_listing(h, tab, "/home/u", &["d2", "proj", "big.bin", "f3"]);
    // A non-empty folder fails, the rest is deleted.
    click_row(h, "d2", Modifiers::NONE);
    click_row(h, "f3", Modifiers::COMMAND);
    menu(h, "f3", "Delete 2 items");
    h.get_by_role_and_label(Role::Button, "Delete").click();
    wait(h, 10, "partial delete toast", |a| {
        a.toasts.items().iter().any(|t| t.title == "Deleted 1 of 2")
    });
    wait_listing(h, tab, "/home/u", &["d2", "proj", "big.bin"]);

    // Cancel a running download: its partial file goes.
    crate::views::sftp::test_download(h.state_mut(), tab, &["big.bin"], &downloads);
    wait(h, 20, "download running", |a| {
        a.sftp.progress().first().is_some_and(|&n| n > 0)
    });
    h.get_by_role_and_label(Role::Button, "Cancel download big.bin")
        .click();
    h.run_steps(2);
    shot(h, "sftp-cancelling");
    wait(h, 20, "download cancelled", |a| {
        a.sftp.cancelled_count() == 1
    });
    assert!(
        !downloads.join("big.bin").exists(),
        "partial download removed"
    );
    assert!(
        h.state()
            .toasts
            .items()
            .iter()
            .all(|t| !t.title.starts_with("Download failed")),
        "a cancel is not a failure"
    );

    // Cancel a running upload: the partial remote file goes.
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(local.join("up.bin"))));
    h.step();
    wait(h, 20, "upload running", |a| {
        a.sftp.progress().first().is_some_and(|&n| n > 0)
    });
    h.get_by_role_and_label(Role::Button, "Cancel upload up.bin")
        .click();
    wait(h, 20, "upload cancelled", |a| {
        a.sftp.cancelled_count() >= 1 && a.sftp.transfer_counts().0 == 0
    });
    assert!(!home.join("up.bin").exists(), "partial upload removed");

    // Cancel a folder job: what was sent stays, the rest never starts.
    h.input_mut()
        .dropped_files
        .push(Arc::new(Dropped(local.join("many"))));
    h.step();
    wait(h, 20, "folder job sending", |a| {
        a.sftp.job_states().iter().any(|j| j.0 == "many") && !a.sftp.progress().is_empty()
    });
    h.get_by_role_and_label(Role::Button, "Cancel upload many/")
        .click();
    wait(h, 20, "folder job cancelled", |a| {
        a.sftp.job_states().iter().any(|j| j.0 == "many" && j.4)
    });
    let (_, total, done, cancelled, _) = h
        .state()
        .sftp
        .job_states()
        .into_iter()
        .find(|j| j.0 == "many")
        .unwrap();
    assert_eq!(total, 5);
    assert!(
        cancelled >= 4 && done + cancelled == 5,
        "done {done} cancelled {cancelled}"
    );
    let sent = std::fs::read_dir(home.join("many")).unwrap().count();
    assert_eq!(sent, done, "only completed files remain");
    shot(h, "sftp-job-cancelled");
    h.get_by_label("Dismiss upload many/").click();
    h.run_steps(2);

    // Cancel what waits beyond the strip's six rows (once the finished rows
    // above have expired, so the count is exact).
    wait(h, 10, "strip empty", |a| {
        a.sftp.transfer_counts() == (0, 0, 0) && a.sftp.cancelled_count() == 0
    });
    for i in 0..10 {
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped(local.join(format!("q{i}")))));
    }
    h.step();
    wait(h, 20, "queue registered", |a| {
        a.sftp.transfer_counts().0 == 10
    });
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Cancel 4 waiting")
        .click();
    wait(h, 30, "queue drained", |a| a.sftp.transfer_counts().0 == 0);
    let uploaded = (0..10)
        .filter(|i| home.join(format!("q{i}")).exists())
        .count();
    assert_eq!(uploaded, 6, "the four waiting files were cancelled");
}
