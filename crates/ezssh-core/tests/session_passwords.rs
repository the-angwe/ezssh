//! Passwords belong to their session: saved with the session, copied by
//! duplicate, dropped with it, never in the shared credentials list, and
//! carried through backup / restore (also from bundles written before the
//! change, where one password row could serve several sessions).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ezssh_core::backup::{self, Bundle, CredentialEntry, SessionEntry};
use ezssh_core::db::sessions::{Session, SessionInput};
use ezssh_core::{AppError, AppState, Secret, api, events, vault};

const MASTER: &str = "correct horse battery";

async fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let (sink, _rx) = events::channel(|| {});
    let state = AppState::open(&dir.path().join("ezterm.sqlite"), sink).await.unwrap();
    api::vault::vault_init(&state, MASTER.into()).await.unwrap();
    (dir, state)
}

fn ssh(name: &str, auth: &str, password: Option<&str>) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: name.into(),
        host: "example.com".into(),
        port: 22,
        username: "root".into(),
        auth_type: auth.into(),
        credential_id: None,
        key_passphrase_credential_id: None,
        color: None,
        initial_command: None,
        scrollback_lines: 5000,
        font_size: 13,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 60,
        connect_timeout_secs: 15,
        env: Vec::new(),
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: password.map(|p| Secret::new(p.into())),
    }
}

/// Decrypt the row `sessions.credential_id` points at, with its owner.
async fn stored_password(state: &AppState, s: &Session) -> Option<(String, Option<i64>)> {
    let id = s.credential_id?;
    let (nonce, ct, owner): (Vec<u8>, Vec<u8>, Option<i64>) =
        sqlx::query_as("SELECT nonce, ciphertext, session_id FROM credentials WHERE id = ?")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let vs = state.vault.read().await;
    let pt = vault::decrypt_with(&vs, &nonce, &ct).unwrap();
    Some((String::from_utf8(pt).unwrap(), owner))
}

/// A session row as an older build exported it.
fn old_session(id: i64, name: &str, credential: i64) -> Session {
    Session {
        id,
        folder_id: None,
        name: name.into(),
        host: "example.com".into(),
        port: 22,
        username: "root".into(),
        auth_type: "password".into(),
        credential_id: Some(credential),
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
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
    }
}

async fn credential_rows(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM credentials").fetch_one(&state.db).await.unwrap()
}

#[tokio::test]
async fn password_lives_and_dies_with_its_session() {
    let (_dir, st) = state().await;
    let s = api::sessions::session_create(&st, ssh("db", "password", Some("hunter2"))).await.unwrap();
    assert_eq!(stored_password(&st, &s).await, Some(("hunter2".into(), Some(s.id))));
    // Not in the shared list.
    assert!(api::credentials::credential_list(&st).await.unwrap().is_empty());
    assert!(api::credentials::credential_list_detailed(&st).await.unwrap().is_empty());
    // Nor reachable through the shared-credential API.
    let own = s.credential_id.unwrap();
    assert!(matches!(api::credentials::credential_delete(&st, own).await, Err(AppError::NotFound)));
    assert!(matches!(
        api::credentials::credential_update(&st, own, Some("x".into()), None).await,
        Err(AppError::NotFound)
    ));

    // Saving without a password keeps it; with one replaces it in place.
    let s2 = api::sessions::session_update(&st, s.id, ssh("db", "password", None)).await.unwrap();
    assert_eq!(s2.credential_id, Some(own));
    let s3 = api::sessions::session_update(&st, s.id, ssh("db", "password", Some("new-pw"))).await.unwrap();
    assert_eq!(s3.credential_id, Some(own));
    assert_eq!(stored_password(&st, &s3).await.unwrap().0, "new-pw");
    // An empty password field also keeps it.
    let s4 = api::sessions::session_update(&st, s.id, ssh("db", "password", Some(""))).await.unwrap();
    assert_eq!(stored_password(&st, &s4).await.unwrap().0, "new-pw");

    // Duplicate gets its own copy.
    let d = api::sessions::session_duplicate(&st, s.id).await.unwrap();
    assert_ne!(d.credential_id, Some(own));
    assert_eq!(stored_password(&st, &d).await, Some(("new-pw".into(), Some(d.id))));
    assert_eq!(credential_rows(&st).await, 2);

    // Deleting the copy drops its password only.
    api::sessions::session_delete(&st, d.id).await.unwrap();
    assert_eq!(credential_rows(&st).await, 1);

    // Switching to agent auth drops the password.
    let a = api::sessions::session_update(&st, s.id, ssh("db", "agent", Some("ignored"))).await.unwrap();
    assert_eq!(a.credential_id, None);
    assert_eq!(credential_rows(&st).await, 0);
}

#[tokio::test]
async fn key_slots_take_shared_rows_only_and_password_sessions_ignore_credential_id() {
    let (_dir, st) = state().await;
    let pw_session = api::sessions::session_create(&st, ssh("a", "password", Some("pw"))).await.unwrap();
    // A key session may not point at another session's password.
    let mut bad = ssh("b", "key", None);
    bad.credential_id = pw_session.credential_id;
    assert!(matches!(api::sessions::session_create(&st, bad).await, Err(AppError::Validation(_))));

    // An unowned password row, as the ezTerm app's picker left them: listed
    // (the credentials manager offers to delete it), never attached by
    // naming it, and deletable.
    let unowned = api::credentials::credential_create(&st, "password".into(), "legacy".into(), "picked".into())
        .await
        .unwrap();
    let mut naming = ssh("c", "password", Some("own"));
    naming.credential_id = Some(unowned.id);
    let c = api::sessions::session_create(&st, naming).await.unwrap();
    assert_ne!(c.credential_id, Some(unowned.id));
    assert_eq!(stored_password(&st, &c).await, Some(("own".into(), Some(c.id))));
    // Naming another session's password neither shares nor copies it.
    let mut other = ssh("c", "password", None);
    other.credential_id = pw_session.credential_id;
    let c2 = api::sessions::session_update(&st, c.id, other).await.unwrap();
    assert_eq!(c2.credential_id, c.credential_id);
    assert_eq!(stored_password(&st, &c2).await.unwrap().0, "own");

    let listed = api::credentials::credential_list_detailed(&st).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].id, listed[0].kind.as_str()), (unowned.id, "password"));
    api::credentials::credential_delete(&st, unowned.id).await.unwrap();
    // One own row per session, no orphans: a, c.
    assert_eq!(credential_rows(&st).await, 2);
}

#[tokio::test]
async fn backup_round_trip_rebinds_passwords() {
    let (dir, st) = state().await;
    let s = api::sessions::session_create(&st, ssh("db", "password", Some("hunter2"))).await.unwrap();
    let key = api::credentials::credential_create(&st, "private_key".into(), "k".into(), "PEM".into()).await.unwrap();
    let mut k = ssh("box", "key", None);
    k.credential_id = Some(key.id);
    let k = api::sessions::session_create(&st, k).await.unwrap();

    let path = dir.path().join("b.ezbackup").to_string_lossy().into_owned();
    let summary = api::backup::backup_create(&st, path.clone(), MASTER.into(), "backup-pass".into()).await.unwrap();
    assert_eq!(summary.credentials, 2);

    let preview = api::backup::backup_preview(&st, path.clone(), "backup-pass".into()).await.unwrap();
    // Only the key is offered as a credential; the password rides on its session.
    assert_eq!(preview.credentials.len(), 1);
    assert_eq!(preview.credentials[0].kind, "private_key");
    let pv = preview.sessions.iter().find(|p| p.id == s.id).unwrap();
    assert!(pv.has_password);
    assert_eq!(pv.credential_id, None);
    assert_eq!(preview.sessions.iter().find(|p| p.id == k.id).unwrap().credential_id, Some(key.id));

    api::sessions::session_delete(&st, s.id).await.unwrap();
    let sel = api::backup::SelectionSpec { session_ids: vec![s.id], ..Default::default() };
    let r = api::backup::backup_restore(&st, path, "backup-pass".into(), sel).await.unwrap();
    assert_eq!(r.sessions_created, 1);
    assert_eq!(r.credentials_created, 0);
    let restored = api::sessions::session_list(&st).await.unwrap().into_iter().find(|x| x.name == "db").unwrap();
    assert_eq!(stored_password(&st, &restored).await, Some(("hunter2".into(), Some(restored.id))));
}

#[tokio::test]
async fn legacy_bundle_with_a_shared_password_gives_each_session_a_copy() {
    let (dir, st) = state().await;
    let one = old_session(1, "one", 7);
    let two = old_session(2, "two", 7);
    let bundle = Bundle {
        version: backup::BACKUP_VERSION,
        created_at: "2026-01-01T00:00:00Z".into(),
        app_version: "1.14.0".into(),
        folders: Vec::new(),
        sessions: vec![SessionEntry { session: one, env: Vec::new() }, SessionEntry { session: two, env: Vec::new() }],
        credentials: vec![CredentialEntry {
            id: 7,
            kind: "password".into(),
            label: "shared".into(),
            secret_b64: B64.encode("s3cret"),
            session_id: None,
        }],
        known_hosts: Vec::new(),
        settings: Vec::new(),
    };
    // Bundles from before the change have no session_id field at all.
    let json = serde_json::to_string(&bundle).unwrap();
    assert!(!json.contains("session_id"));
    let path = dir.path().join("old.ezbackup");
    std::fs::write(&path, backup::encrypt_bundle(&bundle, "backup-pass").unwrap()).unwrap();
    let path = path.to_string_lossy().into_owned();

    let preview = api::backup::backup_preview(&st, path.clone(), "backup-pass".into()).await.unwrap();
    assert!(preview.credentials.is_empty());
    assert!(preview.sessions.iter().all(|s| s.has_password));

    // A selection naming the password credential (the ezTerm app's restore
    // dialog listed it) must not land it as a shared row.
    let sel = api::backup::SelectionSpec { session_ids: vec![1, 2], credential_ids: vec![7], ..Default::default() };
    api::backup::backup_restore(&st, path, "backup-pass".into(), sel).await.unwrap();
    let sessions = api::sessions::session_list(&st).await.unwrap();
    assert_eq!(sessions.len(), 2);
    for s in &sessions {
        assert_eq!(stored_password(&st, s).await, Some(("s3cret".into(), Some(s.id))));
    }
    assert_ne!(sessions[0].credential_id, sessions[1].credential_id);
    assert!(api::credentials::credential_list(&st).await.unwrap().is_empty());
}
