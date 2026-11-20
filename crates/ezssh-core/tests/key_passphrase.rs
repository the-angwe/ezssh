//! A private key and its passphrase are one credential: created together,
//! the passphrase changed or removed on the key, re-wrapped by a vault
//! password change, carried by backups, and folded into the key when a
//! bundle from before the change paired them through a session. Logins go
//! against the in-process test server with an encrypted OpenSSH key.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ezssh_core::api::credentials::PassphraseChange;
use ezssh_core::backup::{self, Bundle, CredentialEntry, SessionEntry};
use ezssh_core::db::sessions::{Session, SessionInput};
use ezssh_core::events::{self, OutputSink};
use ezssh_core::{AppState, api};
use ezssh_testsrv::{Config, TestServer};
use russh::keys::ssh_key::{Algorithm, LineEnding};

const MASTER: &str = "correct horse battery";
const PASSPHRASE: &str = "key pass phrase";

struct Discard;

impl OutputSink for Discard {
    fn send(&self, _: &[u8]) -> bool {
        true
    }
}

struct Env {
    dir: tempfile::TempDir,
    st: AppState,
    srv: TestServer,
    /// The user's key, encrypted with [`PASSPHRASE`], OpenSSH PEM.
    pem: String,
}

async fn env() -> Env {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("remote");
    std::fs::create_dir_all(&root).unwrap();
    let key = russh::keys::PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let pem = key
        .encrypt(&mut rand::rng(), PASSPHRASE)
        .unwrap()
        .to_openssh(LineEnding::LF)
        .unwrap()
        .to_string();
    let mut cfg = Config::new(&root, "u", "not-this");
    cfg.authorized_key = Some(key.public_key().clone());
    let srv = TestServer::start(cfg, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let (sink, _rx) = events::channel(|| {});
    let st = AppState::open(&dir.path().join("ezterm.sqlite"), sink)
        .await
        .unwrap();
    api::vault::vault_init(&st, MASTER.into()).await.unwrap();
    Env { dir, st, srv, pem }
}

fn key_session(port: u16, key: i64) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: "box".into(),
        host: "127.0.0.1".into(),
        port: port.into(),
        username: "u".into(),
        auth_type: "key".into(),
        credential_id: Some(key),
        color: None,
        initial_command: None,
        scrollback_lines: 1000,
        font_size: 14,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 0,
        connect_timeout_secs: 10,
        env: Vec::new(),
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: None,
    }
}

/// Connect `session`; disconnect again on success.
async fn login(st: &AppState, session: i64) -> Result<(), String> {
    match api::ssh::ssh_connect(st, session, 80, 24, true, Some(true), Box::new(Discard)).await {
        Ok(c) => {
            api::ssh::ssh_disconnect(st, c.connection_id).await.unwrap();
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

async fn has_passphrase(st: &AppState, id: i64) -> bool {
    api::credentials::credential_list(st)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .expect("listed")
        .has_passphrase
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_key_logs_in_with_the_passphrase_stored_on_it() {
    let e = env().await;
    let key = api::credentials::credential_create(
        &e.st,
        "private_key".into(),
        "laptop".into(),
        e.pem.clone(),
        Some(PASSPHRASE.into()),
    )
    .await
    .unwrap();
    assert!(key.has_passphrase);
    assert!(has_passphrase(&e.st, key.id).await);
    let s = api::sessions::session_create(&e.st, key_session(e.srv.port(), key.id))
        .await
        .unwrap();
    login(&e.st, s.id)
        .await
        .expect("login with the stored passphrase");

    // Removed: the key no longer decodes.
    api::credentials::credential_update(&e.st, key.id, None, None, PassphraseChange::Clear)
        .await
        .unwrap();
    assert!(!has_passphrase(&e.st, key.id).await);
    let err = login(&e.st, s.id).await.expect_err("no passphrase");
    assert!(err.contains("key parse"), "{err}");

    // Wrong, then right.
    api::credentials::credential_update(
        &e.st,
        key.id,
        None,
        None,
        PassphraseChange::Set("wrong".into()),
    )
    .await
    .unwrap();
    assert!(login(&e.st, s.id).await.is_err());
    api::credentials::credential_update(
        &e.st,
        key.id,
        None,
        None,
        PassphraseChange::Set(PASSPHRASE.into()),
    )
    .await
    .unwrap();
    login(&e.st, s.id)
        .await
        .expect("login after fixing the passphrase");

    // A rename leaves it alone; an empty Set clears it.
    api::credentials::credential_update(
        &e.st,
        key.id,
        Some("renamed".into()),
        None,
        PassphraseChange::Keep,
    )
    .await
    .unwrap();
    assert!(has_passphrase(&e.st, key.id).await);
    api::credentials::credential_update(
        &e.st,
        key.id,
        None,
        None,
        PassphraseChange::Set("".into()),
    )
    .await
    .unwrap();
    assert!(!has_passphrase(&e.st, key.id).await);

    // Only keys take a passphrase; passphrases are not created alone.
    assert!(
        api::credentials::credential_create(
            &e.st,
            "password".into(),
            "pw".into(),
            "x".into(),
            Some("pp".into()),
        )
        .await
        .is_err()
    );
    assert!(
        api::credentials::credential_create(
            &e.st,
            "key_passphrase".into(),
            "pp".into(),
            "x".into(),
            None,
        )
        .await
        .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passphrase_survives_a_vault_password_change_and_a_backup() {
    let e = env().await;
    let key = api::credentials::credential_create(
        &e.st,
        "private_key".into(),
        "laptop".into(),
        e.pem.clone(),
        Some(PASSPHRASE.into()),
    )
    .await
    .unwrap();
    let s = api::sessions::session_create(&e.st, key_session(e.srv.port(), key.id))
        .await
        .unwrap();

    let new_master = "a different master password";
    api::vault::vault_change_password(&e.st, MASTER.into(), new_master.into())
        .await
        .unwrap();
    api::vault::vault_unlock(&e.st, new_master.into())
        .await
        .unwrap();
    login(&e.st, s.id).await.expect("login after the re-key");

    let path = e
        .dir
        .path()
        .join("b.ezbackup")
        .to_string_lossy()
        .into_owned();
    api::backup::backup_create(&e.st, path.clone(), new_master.into(), "backup-pass".into())
        .await
        .unwrap();
    let preview = api::backup::backup_preview(&e.st, path.clone(), "backup-pass".into())
        .await
        .unwrap();
    assert_eq!(preview.credentials.len(), 1);
    assert!(preview.credentials[0].has_passphrase);

    api::sessions::session_delete(&e.st, s.id).await.unwrap();
    api::credentials::credential_delete(&e.st, key.id)
        .await
        .unwrap();
    let sel = api::backup::SelectionSpec {
        session_ids: vec![s.id],
        credential_ids: vec![key.id],
        ..Default::default()
    };
    let r = api::backup::backup_restore(&e.st, path, "backup-pass".into(), sel)
        .await
        .unwrap();
    assert_eq!((r.credentials_created, r.sessions_created), (1, 1));
    let restored = api::sessions::session_list(&e.st)
        .await
        .unwrap()
        .into_iter()
        .find(|x| x.name == "box")
        .unwrap();
    assert_eq!(restored.key_passphrase_credential_id, None);
    assert!(has_passphrase(&e.st, restored.credential_id.unwrap()).await);
    login(&e.st, restored.id)
        .await
        .expect("login with the restored key");
}

/// A key session as an older build exported it: the passphrase is a
/// separate credential the session names.
fn old_key_session(port: u16, key: i64, passphrase: i64) -> Session {
    Session {
        id: 1,
        folder_id: None,
        name: "old box".into(),
        host: "127.0.0.1".into(),
        port: port.into(),
        username: "u".into(),
        auth_type: "key".into(),
        credential_id: Some(key),
        key_passphrase_credential_id: Some(passphrase),
        color: None,
        sort: 0,
        initial_command: None,
        scrollback_lines: 5000,
        font_size: 13,
        font_family: String::new(),
        cursor_style: "block".into(),
        compression: 0,
        keepalive_secs: 0,
        connect_timeout_secs: 10,
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_bundle_folds_the_paired_passphrase_into_the_key() {
    let e = env().await;
    let entry = |id: i64, kind: &str, label: &str, secret: &str| CredentialEntry {
        id,
        kind: kind.into(),
        label: label.into(),
        secret_b64: B64.encode(secret),
        session_id: None,
        passphrase_b64: None,
    };
    let bundle = Bundle {
        version: backup::BACKUP_VERSION,
        created_at: "2026-01-01T00:00:00Z".into(),
        app_version: "1.9.0".into(),
        folders: Vec::new(),
        sessions: vec![SessionEntry {
            session: old_key_session(e.srv.port(), 3, 4),
            env: Vec::new(),
            forwards: Vec::new(),
        }],
        credentials: vec![
            entry(3, "private_key", "laptop", &e.pem),
            entry(4, "key_passphrase", "laptop pp", PASSPHRASE),
        ],
        known_hosts: Vec::new(),
        settings: Vec::new(),
    };
    let json = serde_json::to_string(&bundle).unwrap();
    assert!(
        !json.contains("passphrase_b64"),
        "old bundles lack the field"
    );
    let path = e.dir.path().join("old.ezbackup");
    std::fs::write(
        &path,
        backup::encrypt_bundle(&bundle, "backup-pass").unwrap(),
    )
    .unwrap();
    let path = path.to_string_lossy().into_owned();

    // Only the key is offered, with its passphrase.
    let preview = api::backup::backup_preview(&e.st, path.clone(), "backup-pass".into())
        .await
        .unwrap();
    assert_eq!(preview.credentials.len(), 1);
    assert_eq!(preview.credentials[0].kind, "private_key");
    assert!(preview.credentials[0].has_passphrase);

    // The ezTerm-era restore listed the passphrase too: selecting it adds
    // no separate row.
    let sel = api::backup::SelectionSpec {
        session_ids: vec![1],
        credential_ids: vec![3, 4],
        ..Default::default()
    };
    let r = api::backup::backup_restore(&e.st, path, "backup-pass".into(), sel)
        .await
        .unwrap();
    assert_eq!((r.credentials_created, r.sessions_created), (1, 1));
    let creds = api::credentials::credential_list(&e.st).await.unwrap();
    assert_eq!(creds.len(), 1);
    assert!(creds[0].has_passphrase);
    let s = api::sessions::session_list(&e.st)
        .await
        .unwrap()
        .into_iter()
        .find(|x| x.name == "old box")
        .unwrap();
    assert_eq!(
        (s.credential_id, s.key_passphrase_credential_id),
        (Some(creds[0].id), None)
    );
    login(&e.st, s.id)
        .await
        .expect("login with the folded passphrase");
}
