//! Headless tests of the backup, restore, cloud sync, MobaXterm import.

use std::path::Path;

use egui::{Key, Modifiers, Role};
use egui_kittest::kittest::Queryable;
use ezssh_core::db::sessions::SessionInput;
use ezssh_core::{Secret, api};

use super::*;
use crate::style::Theme;
use crate::views::backup::{self, BackupDialog};
use crate::views::import::ImportDialog;
use crate::views::restore::{self, RestoreDialog};
use crate::views::sync::{self, SyncDialog};

const BACKUP_PASS: &str = "backup phrase 1";

fn session(
    name: &str,
    folder: Option<i64>,
    auth: &str,
    credential_id: Option<i64>,
    password: Option<&str>,
) -> SessionInput {
    SessionInput {
        folder_id: folder,
        name: name.into(),
        host: format!("{name}.example.com"),
        port: 22,
        username: "ops".into(),
        auth_type: auth.into(),
        credential_id,
        key_passphrase_credential_id: None,
        color: None,
        initial_command: None,
        scrollback_lines: 5000,
        font_size: 14,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 30,
        connect_timeout_secs: 15,
        env: vec![],
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: password.map(|p| Secret::new(p.into())),
    }
}

struct Seeded {
    pw_session: i64,
    pw_credential: i64,
}

/// Replace the seeded local shells with a folder "Prod" holding a password
/// session "web" and a key session "deploy" (key "id_test"), so the
/// restore lists stay short and fully visible.
fn seed(h: &mut Harness<'static, App>) -> Seeded {
    let (pw_session, pw_credential) = h.state().core.block_on(|st| async move {
        for s in api::sessions::session_list(&st).await.expect("sessions") {
            api::sessions::session_delete(&st, s.id)
                .await
                .expect("delete session");
        }
        for f in api::folders::folder_list(&st).await.expect("folders") {
            // Children go with their parent.
            let _ = api::folders::folder_delete(&st, f.id).await;
        }
        let folder = api::folders::folder_create(&st, None, "Prod".into())
            .await
            .expect("folder");
        let key = api::credentials::credential_create(
            &st,
            "private_key".into(),
            "id_test".into(),
            "-----BEGIN KEY-----\nx\n".into(),
        )
        .await
        .expect("key");
        let web = api::sessions::session_create(
            &st,
            session("web", Some(folder.id), "password", None, Some("hunter22")),
        )
        .await
        .expect("web");
        api::sessions::session_create(
            &st,
            session("deploy", Some(folder.id), "key", Some(key.id), None),
        )
        .await
        .expect("deploy");
        (
            web.id,
            web.credential_id
                .expect("a password session owns a credentials row"),
        )
    });
    let app = h.state_mut();
    app.sidebar.reload(&app.core);
    app.creds.reload(&app.core);
    wait_s(h, 10, "seeded tree", |a| a.sidebar.sessions.len() == 2);
    Seeded {
        pw_session,
        pw_credential,
    }
}

/// Let a dialog that grew or shrank re-centre before clicking into it
/// (the modal is centred, so its rows move with its height).
fn settle(h: &mut Harness<'static, App>) {
    h.run_steps(4);
}

/// [`wait`], then [`settle`].
fn wait_s(h: &mut Harness<'static, App>, secs: u64, what: &str, cond: impl Fn(&App) -> bool) {
    wait(h, secs, what, cond);
    settle(h);
}

/// Replace what a password field holds.
fn retype(h: &mut Harness<'static, App>, index: usize, text: &str) {
    settle(h);
    let field = h
        .query_all_by_role(Role::PasswordInput)
        .nth(index)
        .expect("password field");
    field.click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    h.query_all_by_role(Role::PasswordInput)
        .nth(index)
        .expect("password field")
        .type_text(text);
    h.run_steps(2);
}

fn last_toast(app: &App) -> (String, String) {
    let t = app.toasts.items().last().expect("a toast");
    (t.title.clone(), t.body.clone().unwrap_or_default())
}

/// Both themes of one screen.
fn shots(h: &mut Harness<'static, App>, name: &str) {
    shot(h, name);
    h.state_mut().set_theme(Theme::Light);
    shot(h, &format!("{name}-light"));
    h.state_mut().set_theme(Theme::Dark);
    h.run_steps(2);
}

/// Back up through the dialog into `file`.
fn run_backup(h: &mut Harness<'static, App>, file: &Path) {
    h.state_mut().dialogs.backup = Some(BackupDialog::default());
    settle(h);
    assert!(h.query_by_label("Export encrypted backup").is_some());
    type_into(h, 0, "wrong password");
    shots(h, "backup-reauth");
    h.get_by_label("Continue").click();
    wait_s(h, 20, "reauth refused", |a| {
        a.dialogs.backup.as_ref().is_some_and(|d| d.err.is_some())
    });
    assert!(h.query_by_label("Incorrect master password").is_some());
    retype(h, 0, PASSWORD);
    h.get_by_label("Continue").click();
    wait_s(h, 20, "passphrase step", |a| {
        a.dialogs
            .backup
            .as_ref()
            .is_some_and(|d| d.step == backup::Step::Passphrase)
    });
    // Too short first: refused before any file dialog.
    type_into(h, 0, "short");
    type_into(h, 1, "short");
    h.get_by_label("Save backup…").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Passphrase must be at least 8 characters")
            .is_some()
    );
    shots(h, "backup-passphrase");
    retype(h, 0, BACKUP_PASS);
    retype(h, 1, BACKUP_PASS);
    h.state_mut().dialogs.backup.as_mut().unwrap().save_to = Some(file.to_path_buf());
    h.get_by_label("Save backup…").click();
    wait_s(h, 20, "backup written", |a| a.dialogs.backup.is_none());
    assert!(file.exists(), "backup file at {}", file.display());
    let (title, body) = last_toast(h.state());
    assert_eq!(title, "Backup saved");
    assert!(
        body.contains("2 sessions") && body.contains("1 folder"),
        "{body}"
    );
    // One key + one session password.
    assert!(
        body.contains("2 credentials (keys and session passwords)"),
        "{body}"
    );
}

/// Open `file` in the restore dialog and unlock it.
fn open_restore(h: &mut Harness<'static, App>, file: &Path, pass: &str) {
    h.state_mut().dialogs.restore = Some(RestoreDialog::from_path(file.to_path_buf()));
    settle(h);
    assert!(h.query_by_label("Restore from backup").is_some());
    type_into(h, 0, pass);
    h.get_by_label("Unlock backup").click();
    wait_s(h, 20, "backup preview", |a| {
        a.dialogs
            .restore
            .as_ref()
            .is_some_and(|d| d.step == restore::Step::Select || d.err.is_some())
    });
    let err = h
        .state()
        .dialogs
        .restore
        .as_ref()
        .and_then(|d| d.err.clone());
    assert!(err.is_none(), "preview failed: {err:?}");
}

#[test]
fn backup_then_restore_brings_session_passwords() {
    let mut env = start();
    unlock_fresh(&mut env);
    let file = env._dir.path().join("backup.json");
    let h = &mut env.h;
    let seeded = seed(h);
    run_backup(h, &file);

    // A wrong passphrase is refused inline.
    h.state_mut().dialogs.restore = Some(RestoreDialog::from_path(file.clone()));
    settle(h);
    type_into(h, 0, "not the phrase");
    h.get_by_label("Unlock backup").click();
    wait_s(h, 20, "bad passphrase", |a| {
        a.dialogs.restore.as_ref().is_some_and(|d| d.err.is_some())
    });
    assert!(h.query_by_label("Incorrect backup passphrase").is_some());
    shots(h, "restore-passphrase-error");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().dialogs.restore.is_none());

    open_restore(h, &file, BACKUP_PASS);
    {
        let d = h.state().dialogs.restore.as_ref().unwrap();
        let p = d.preview.as_ref().unwrap();
        let web = p.sessions.iter().find(|s| s.name == "web").unwrap();
        assert!(
            web.has_password && web.credential_id.is_none(),
            "the password travels with its session"
        );
        assert!(
            p.credentials.iter().all(|c| c.kind != "password"),
            "no password in the shared list"
        );
        assert_eq!(p.credentials.len(), 1);
    }
    assert!(
        h.query_by_label("password included").is_some(),
        "marker on the password session"
    );
    shots(h, "restore-select");

    // Tree selection: drop everything, then pick the folder and both
    // sessions, then drop the key to see the warning.
    h.get_by_label("Select no sessions").click();
    h.get_by_label("Select no folders").click();
    h.run_steps(2);
    assert_eq!(
        h.state()
            .dialogs
            .restore
            .as_ref()
            .unwrap()
            .sel
            .sessions
            .len(),
        0
    );
    h.get_by_role_and_label(Role::CheckBox, "web").click();
    h.get_by_role_and_label(Role::CheckBox, "deploy").click();
    h.get_by_role_and_label(Role::CheckBox, "Prod").click();
    h.get_by_label("Select no credentials").click();
    settle(h);
    let sel = &h.state().dialogs.restore.as_ref().unwrap().sel;
    assert_eq!(
        (sel.sessions.len(), sel.folders.len(), sel.creds.len()),
        (2, 1, 0)
    );
    assert!(
        h.query_by_label(
            "deploy: the session's key is not selected, so it is restored without one."
        )
        .is_some(),
        "warning for the key session only"
    );
    shot(h, "restore-key-warning");
    h.get_by_label("Select all credentials").click();
    h.run_steps(2);
    assert!(
        h.query_by_label(
            "deploy: the session's key is not selected, so it is restored without one."
        )
        .is_none()
    );

    settle(h);
    h.get_by_label("Import selected").click();
    wait_s(h, 20, "restored", |a| {
        a.dialogs.restore.as_ref().is_none_or(|d| d.err.is_some())
    });
    let err = h
        .state()
        .dialogs
        .restore
        .as_ref()
        .and_then(|d| d.err.clone());
    assert!(err.is_none(), "restore failed: {err:?}");
    let (title, body) = last_toast(h.state());
    assert_eq!(title, "Restore complete");
    assert!(
        body.contains("2 sessions")
            && body.contains("1 credential")
            && body.contains("Imported as: id_test (2), Prod (2)"),
        "{body}"
    );
    wait_s(h, 10, "tree reloaded", |a| {
        a.sidebar.sessions.len() == 4 && a.sidebar.folders.iter().any(|f| f.name == "Prod (2)")
    });

    // The restored password session owns a new password row.
    let restored = {
        let a = h.state();
        let folder = a
            .sidebar
            .folders
            .iter()
            .find(|f| f.name == "Prod (2)")
            .unwrap()
            .id;
        a.sidebar
            .sessions
            .iter()
            .find(|s| s.name == "web" && s.folder_id == Some(folder))
            .unwrap()
            .clone()
    };
    assert_ne!(restored.id, seeded.pw_session);
    let cred = restored.credential_id.expect("restored with its password");
    assert_ne!(cred, seeded.pw_credential, "its own copy");
    let rows = h
        .state()
        .core
        .block_on(|st| async move { ezssh_core::db::credentials::list_for_export(&st.db).await })
        .unwrap();
    let row = rows.iter().find(|r| r.id == cred).expect("password row");
    assert_eq!(
        (row.kind.as_str(), row.session_id),
        ("password", Some(restored.id))
    );
    // The key came back as a shared credential (renamed) and the restored
    // key session points at it.
    let key = rows
        .iter()
        .find(|r| r.label == "id_test (2)")
        .expect("restored key");
    assert!(key.session_id.is_none());
    let deploy = h
        .state()
        .sidebar
        .sessions
        .iter()
        .find(|s| s.name == "deploy" && s.folder_id == restored.folder_id)
        .unwrap()
        .clone();
    assert_eq!(deploy.credential_id, Some(key.id));
    wait_s(h, 10, "credentials reloaded", |a| {
        a.creds.list.iter().any(|c| c.label == "id_test (2)")
    });
}

#[test]
fn sync_local_folder_push_status_and_pull_handoff() {
    let mut env = start();
    unlock_fresh(&mut env);
    let target = env._dir.path().join("sync-target");
    let h = &mut env.h;

    h.state_mut().dialogs.sync = Some(SyncDialog::default());
    wait_s(h, 10, "sync status", |a| {
        a.dialogs.sync.as_ref().is_some_and(|d| d.status.is_some())
    });
    assert!(h.query_by_label("Not configured").is_some());
    h.get_by_role_and_label(Role::RadioButton, "S3-compatible")
        .click();
    h.run_steps(2);
    assert_eq!(
        h.state().dialogs.sync.as_ref().unwrap().kind,
        sync::Kind::S3
    );
    assert!(h.query_by_label("Endpoint URL").is_some());
    shots(h, "sync-s3");
    h.get_by_role_and_label(Role::RadioButton, "Local folder")
        .click();
    settle(h);

    // Validation before any call.
    h.get_by_label("Enable sync").click();
    settle(h);
    assert!(h.query_by_label("Pick a folder first").is_some());

    settle(h);
    let folder = h.get_by_role_and_label(Role::TextInput, "Sync folder");
    folder.click();
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Sync folder")
        .type_text(&target.to_string_lossy());
    h.run_steps(2);
    assert_eq!(
        h.state().dialogs.sync.as_ref().unwrap().path,
        target.to_string_lossy()
    );
    type_into(h, 0, "sync phrase 1");
    type_into(h, 1, "sync phrase 1");
    h.get_by_label("Enable sync").click();
    wait_s(h, 20, "local sync enabled", |a| {
        a.dialogs.sync.as_ref().is_some_and(|d| {
            d.msg.as_deref() == Some("Local-folder sync enabled. Initial push queued.")
                && d.status.as_ref().is_some_and(|s| s.kind == "local")
        })
    });
    // Debounce 3 s + the 3 s poll.
    wait_s(h, 30, "first push recorded", |a| {
        a.dialogs
            .sync
            .as_ref()
            .and_then(|d| d.status.as_ref())
            .is_some_and(|s| s.last_success_at.is_some())
    });
    let bundle = target.join("ezterm-sync.json");
    assert!(bundle.exists(), "bundle written to {}", bundle.display());
    assert!(h.query_by_label("Enabled — local").is_some());
    assert!(
        h.query_by_label("Update & push").is_some(),
        "same target → update"
    );
    shots(h, "sync-local-enabled");

    h.get_by_label("Push now").click();
    wait_s(h, 10, "push queued", |a| {
        a.dialogs.sync.as_ref().is_some_and(|d| {
            d.msg.as_deref() == Some("Push queued — will write within a few seconds.")
        })
    });
    assert!(
        h.query_by_label("Pull from cloud").is_none(),
        "pull is S3 only"
    );

    // Pull hands its temp file to the restore dialog; a local bundle is the
    // same format, so it previews with the sync passphrase.
    sync::hand_off_pull(h.state_mut(), bundle.clone());
    h.run_steps(3);
    assert!(h.state().dialogs.sync.is_none());
    assert_eq!(
        h.state()
            .dialogs
            .restore
            .as_ref()
            .and_then(|d| d.path.clone()),
        Some(bundle.clone())
    );
    h.state_mut().dialogs.restore = None;
    open_restore(h, &bundle, "sync phrase 1");
    assert!(
        h.state()
            .dialogs
            .restore
            .as_ref()
            .unwrap()
            .preview
            .as_ref()
            .is_some_and(|p| !p.sessions.is_empty())
    );
    h.get_by_label("Cancel").click();
    h.run_steps(2);

    // Disable.
    h.state_mut().dialogs.sync = Some(SyncDialog::default());
    wait_s(h, 10, "sync status", |a| {
        a.dialogs.sync.as_ref().is_some_and(|d| d.status.is_some())
    });
    h.get_by_label("Disable").click();
    wait_s(h, 10, "disabled", |a| {
        a.dialogs.sync.as_ref().is_some_and(|d| {
            d.msg.as_deref() == Some("Sync disabled.")
                && d.status.as_ref().is_some_and(|s| s.kind == "none")
        })
    });
    assert!(h.query_by_label("Disable").is_none());
}

pub(super) const MOBA_FIXTURE: &str = "\
[Bookmarks]
SubRep=
ImgNum=42

[Bookmarks_1]
SubRep=Production
ImgNum=41
web-01=#109#0%web01.example.com%22%root%%-1%-1%%%%%0%0%0%%%
db-01=#109#0%db01.example.com%2222%postgres%%-1%-1
keyed=#109#0%k.example.com%22%ops%%-1%-1%%%%%0%0%0%_MyDocuments_\\ssh-keys\\ezssh-test-missing.pem%%

[Bookmarks_2]
SubRep=
rdp-box=#0#4%rdp.example.com%3389%admin%%
";

#[test]
fn mobaxterm_import_preview_commit_and_duplicates() {
    let mut env = start();
    unlock_fresh(&mut env);
    let file = env._dir.path().join("sessions.mxtsessions");
    std::fs::write(&file, MOBA_FIXTURE).unwrap();
    let h = &mut env.h;

    h.state_mut().dialogs.import = Some(ImportDialog::from_path(file.clone()));
    wait_s(h, 10, "import preview", |a| {
        a.dialogs
            .import
            .as_ref()
            .is_some_and(|d| d.preview.is_some() || d.error.is_some())
    });
    assert!(h.state().dialogs.import.as_ref().unwrap().error.is_none());
    for label in [
        "Sessions: 3",
        "New folders: 1",
        "Key files: 1",
        "Duplicates: 0",
        "Skipped: 1",
    ] {
        assert!(h.query_by_label(label).is_some(), "stat {label}");
    }
    assert!(h.query_by_label("3 SESSIONS TO IMPORT").is_some());
    shots(h, "import-preview");

    h.get_by_label("Import").click();
    wait_s(h, 20, "imported", |a| a.dialogs.import.is_none());
    let (title, body) = last_toast(h.state());
    assert_eq!(title, "Import complete with missing keys");
    assert!(
        body.starts_with("3 created, 1 folder") && body.contains("ezssh-test-missing.pem"),
        "{body}"
    );
    wait_s(h, 10, "tree reloaded", |a| {
        let prod = a
            .sidebar
            .folders
            .iter()
            .find(|f| f.name == "Production")
            .map(|f| f.id);
        prod.is_some()
            && a.sidebar
                .sessions
                .iter()
                .filter(|s| s.folder_id == prod)
                .count()
                == 3
    });

    // Again: every row is a duplicate; Rename imports copies.
    h.state_mut().dialogs.import = Some(ImportDialog::from_path(file.clone()));
    wait_s(h, 10, "import preview", |a| {
        a.dialogs
            .import
            .as_ref()
            .is_some_and(|d| d.preview.is_some())
    });
    assert!(h.query_by_label("Duplicates: 3").is_some());
    shots(h, "import-duplicates");
    h.get_by_role_and_label(Role::RadioButton, "Rename").click();
    h.run_steps(2);
    assert_eq!(
        h.state().dialogs.import.as_ref().unwrap().strategy,
        crate::views::import::Strategy::Rename
    );
    h.get_by_label("Import").click();
    wait_s(h, 20, "imported again", |a| a.dialogs.import.is_none());
    wait_s(h, 10, "renamed copies", |a| {
        a.sidebar
            .sessions
            .iter()
            .any(|s| s.name == "web-01 (imported)")
    });
}
