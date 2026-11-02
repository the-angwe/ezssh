//! End-to-end through the public API with no UI: open a fresh database,
//! create the vault, save a local-shell session, connect it through
//! `api::local`, type into it and watch output and the close event arrive
//! through the sinks a UI would supply.

#![cfg(unix)]

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use ezssh_core::api;
use ezssh_core::db::sessions::SessionInput;
use ezssh_core::events::{self, CloseReason, Event, OutputSink};
use ezssh_core::{AppError, AppState};

/// Collects every output chunk into a shared buffer.
struct Collect(Arc<Mutex<Vec<u8>>>);

impl OutputSink for Collect {
    fn send(&self, bytes: &[u8]) -> bool {
        self.0.lock().unwrap().extend_from_slice(bytes);
        true
    }
}

fn local_session(name: &str) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: name.into(),
        host: "/bin/sh".into(),
        port: 0,
        username: String::new(),
        auth_type: "agent".into(),
        credential_id: None,
        key_passphrase_credential_id: None,
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
        session_kind: "local".into(),
        forward_x11: 0,
        starting_dir: None,
        password: None,
    }
}

fn wait_closed(rx: &mpsc::Receiver<Event>, id: u64, timeout: Duration) -> CloseReason {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Event::Closed { connection_id, reason }) if connection_id == id => return reason,
            Ok(other) => eprintln!("ignoring {other:?}"),
            Err(e) => panic!("no close event for {id} within {timeout:?}: {e}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_shell_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ezterm.sqlite");
    let (sink, rx) = events::channel(|| {});
    let state = AppState::open(&db, sink).await.expect("open state");

    assert_eq!(api::vault::vault_status(&state).await.unwrap(), "uninitialized");
    // Everything behind the vault is refused until it exists and is unlocked.
    assert!(matches!(
        api::sessions::session_list(&state).await,
        Err(AppError::VaultLocked)
    ));
    api::vault::vault_init(&state, "correct horse battery".into()).await.unwrap();
    assert_eq!(api::vault::vault_status(&state).await.unwrap(), "unlocked");

    let session = api::sessions::session_create(&state, local_session("sh")).await.unwrap();
    assert_eq!(api::sessions::session_list(&state).await.unwrap().len(), 1);

    let buf = Arc::new(Mutex::new(Vec::new()));
    let conn = api::local::local_connect(&state, session.id, 80, 24, Box::new(Collect(buf.clone())))
        .await
        .expect("local connect");
    assert!(state.local.get(conn.connection_id).await.is_some());

    api::local::local_resize(&state, conn.connection_id, 100, 30).await.unwrap();
    api::local::local_write(&state, conn.connection_id, b"echo ezssh-$((40+2)); exit\n".to_vec())
        .await
        .unwrap();

    let reason = wait_closed(&rx, conn.connection_id, Duration::from_secs(10));
    assert_eq!(reason, CloseReason::Ended);
    let text = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    assert!(text.contains("ezssh-42"), "shell output was {text:?}");

    api::local::local_disconnect(&state, conn.connection_id).await.unwrap();

    // Reopening the same file sees an initialised, locked vault.
    drop(state);
    let (sink, _rx) = events::channel(|| {});
    let state = AppState::open(&db, sink).await.expect("reopen state");
    assert_eq!(api::vault::vault_status(&state).await.unwrap(), "locked");
    api::vault::vault_unlock(&state, "correct horse battery".into()).await.unwrap();
    assert_eq!(api::sessions::session_list(&state).await.unwrap()[0].name, "sh");
}
