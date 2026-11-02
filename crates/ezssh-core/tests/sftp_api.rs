//! The SFTP API end to end against the in-process test server
//! (`ezssh-testsrv`): connect with a password, open the file browser,
//! list / mkdir / rename / remove, upload and download with progress
//! events, and the final error event of a transfer that cannot start.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ezssh_core::api;
use ezssh_core::db::sessions::SessionInput;
use ezssh_core::events::{self, Event, OutputSink};
use ezssh_core::sftp::transfer::TransferProgress;
use ezssh_core::{AppState, Secret};
use ezssh_testsrv::{Config, TestServer};

struct Discard;

impl OutputSink for Discard {
    fn send(&self, _: &[u8]) -> bool {
        true
    }
}

fn ssh_session(port: u16) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: "testsrv".into(),
        host: "127.0.0.1".into(),
        port: port.into(),
        username: "u".into(),
        auth_type: "password".into(),
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
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: Some(Secret::new("pw".into())),
    }
}

/// The last event of transfer `id`.
fn wait_transfer(rx: &mpsc::Receiver<Event>, id: u64) -> TransferProgress {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Event::Transfer(p)) if p.transfer_id == id && p.done => return p,
            Ok(_) => {}
            Err(e) => panic!("no final event for transfer {id}: {e}"),
        }
    }
}

fn names(list: &[api::sftp::SftpEntry]) -> Vec<String> {
    list.iter().map(|e| format!("{}{}", e.name, if e.is_dir { "/" } else { "" })).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_round_trip() {
    // RUST_LOG=ezssh_testsrv=debug,ezssh_core=debug shows both sides.
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_test_writer().try_init();
    let dir = tempfile::tempdir().unwrap();
    let remote_root = dir.path().join("remote");
    std::fs::create_dir_all(remote_root.join("home/u")).unwrap();
    std::fs::write(remote_root.join("home/u/hello.txt"), b"hello").unwrap();
    let local = dir.path().join("local");
    std::fs::create_dir_all(&local).unwrap();

    let mut cfg = Config::new(&remote_root, "u", "pw");
    cfg.home = "/home/u".into();
    let srv = TestServer::start(cfg, "127.0.0.1:0".parse().unwrap()).await.unwrap();

    let (sink, rx) = events::channel(|| {});
    let state = AppState::open(&dir.path().join("ezterm.sqlite"), sink).await.unwrap();
    api::vault::vault_init(&state, "correct horse battery".into()).await.unwrap();
    let session = api::sessions::session_create(&state, ssh_session(srv.port())).await.unwrap();
    let conn = api::ssh::ssh_connect(&state, session.id, 80, 24, true, Some(true), Box::new(Discard)).await.expect("connect");
    let cid = conn.connection_id;

    api::sftp::sftp_open(&state, cid).await.expect("sftp open");
    let home = api::sftp::sftp_realpath(&state, cid, ".".into()).await.unwrap();
    assert_eq!(home, "/home/u");
    assert_eq!(names(&api::sftp::sftp_list(&state, cid, home.clone()).await.unwrap()), ["hello.txt"]);

    api::sftp::sftp_mkdir(&state, cid, "/home/u/d".into()).await.unwrap();
    // Directories first, then names.
    assert_eq!(names(&api::sftp::sftp_list(&state, cid, home.clone()).await.unwrap()), ["d/", "hello.txt"]);

    // Upload with progress, then download it back.
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(local.join("big.bin"), &big).unwrap();
    let t = api::sftp::sftp_upload(&state, cid, local.join("big.bin").display().to_string(), "/home/u/d/big.bin".into()).await.unwrap();
    let p = wait_transfer(&rx, t.transfer_id);
    assert_eq!((p.error.as_deref(), p.bytes_sent, p.total_bytes), (None, 300_000, 300_000));
    assert_eq!(std::fs::read(remote_root.join("home/u/d/big.bin")).unwrap(), big);

    let back = local.join("back.bin");
    let t = api::sftp::sftp_download(&state, cid, "/home/u/d/big.bin".into(), back.display().to_string()).await.unwrap();
    let p = wait_transfer(&rx, t.transfer_id);
    assert_eq!((p.error.as_deref(), p.bytes_sent), (None, 300_000));
    assert_eq!(std::fs::read(&back).unwrap(), big);

    // A transfer that cannot start still ends with an error event.
    let t = api::sftp::sftp_upload(&state, cid, local.join("missing").display().to_string(), "/home/u/x".into()).await.unwrap();
    assert!(wait_transfer(&rx, t.transfer_id).error.is_some());
    let t = api::sftp::sftp_download(&state, cid, "/home/u/hello.txt".into(), local.join("no/such/dir/f").display().to_string()).await.unwrap();
    assert!(wait_transfer(&rx, t.transfer_id).error.is_some());
    // And one that fails on the server side too.
    let t = api::sftp::sftp_download(&state, cid, "/home/u/nope".into(), local.join("nope").display().to_string()).await.unwrap();
    assert!(wait_transfer(&rx, t.transfer_id).error.is_some());

    // Batches run in order, one ticket each.
    for n in ["a", "b", "c"] {
        std::fs::write(local.join(n), n.repeat(1000)).unwrap();
    }
    let items = |names: &[&str], up: bool| -> Vec<api::sftp::TransferItem> {
        names
            .iter()
            .map(|n| api::sftp::TransferItem {
                local_path: local.join(if up { n.to_string() } else { format!("{n}.back") }).display().to_string(),
                remote_path: format!("/home/u/d/{n}"),
            })
            .collect()
    };
    let up = api::sftp::sftp_upload_many(&state, cid, items(&["a", "b", "c"], true)).await.unwrap();
    for t in &up {
        assert_eq!(wait_transfer(&rx, t.transfer_id).error, None);
    }
    let down = api::sftp::sftp_download_many(&state, cid, items(&["a", "b", "c"], false)).await.unwrap();
    for t in &down {
        assert_eq!(wait_transfer(&rx, t.transfer_id).error, None);
    }
    assert_eq!(std::fs::read_to_string(local.join("c.back")).unwrap(), "c".repeat(1000));
    for n in ["a", "b", "c"] {
        api::sftp::sftp_remove(&state, cid, format!("/home/u/d/{n}")).await.unwrap();
    }

    api::sftp::sftp_rename(&state, cid, "/home/u/hello.txt".into(), "/home/u/d/hi.txt".into()).await.unwrap();
    assert_eq!(names(&api::sftp::sftp_list(&state, cid, "/home/u/d".into()).await.unwrap()), ["big.bin", "hi.txt"]);
    // A non-empty directory cannot be removed.
    assert!(api::sftp::sftp_rmdir(&state, cid, "/home/u/d".into()).await.is_err());
    api::sftp::sftp_remove(&state, cid, "/home/u/d/hi.txt".into()).await.unwrap();
    api::sftp::sftp_remove(&state, cid, "/home/u/d/big.bin".into()).await.unwrap();
    api::sftp::sftp_rmdir(&state, cid, "/home/u/d".into()).await.unwrap();
    assert!(api::sftp::sftp_list(&state, cid, home).await.unwrap().is_empty());
    assert!(!Path::new(&remote_root.join("home/u/d")).exists());

    api::ssh::ssh_disconnect(&state, cid).await.unwrap();
    // The browser goes with the connection.
    assert!(api::sftp::sftp_list(&state, cid, "/".into()).await.is_err());
}

/// First event of transfer `id` with progress, or its end.
fn wait_progress(rx: &mpsc::Receiver<Event>, id: u64) -> TransferProgress {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Event::Transfer(p)) if p.transfer_id == id && (p.bytes_sent > 0 || p.done) => return p,
            Ok(_) => {}
            Err(e) => panic!("no progress for transfer {id}: {e}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_and_walk() {
    let dir = tempfile::tempdir().unwrap();
    let remote_root = dir.path().join("remote");
    std::fs::create_dir_all(remote_root.join("t/a/b")).unwrap();
    std::fs::create_dir_all(remote_root.join("t/empty")).unwrap();
    std::fs::write(remote_root.join("t/top.txt"), b"1").unwrap();
    std::fs::write(remote_root.join("t/a/mid.txt"), b"2").unwrap();
    std::fs::write(remote_root.join("t/a/b/deep.txt"), b"3").unwrap();
    std::fs::write(remote_root.join("big.bin"), vec![9u8; 4 * 1024 * 1024]).unwrap();
    let local = dir.path().join("local");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("up.bin"), vec![5u8; 4 * 1024 * 1024]).unwrap();

    let mut cfg = Config::new(&remote_root, "u", "pw");
    // 128 chunks of 32 KiB at 15 ms each: about two seconds per file.
    cfg.op_delay = Duration::from_millis(15);
    let srv = TestServer::start(cfg, "127.0.0.1:0".parse().unwrap()).await.unwrap();
    let (sink, rx) = events::channel(|| {});
    let state = AppState::open(&dir.path().join("ezterm.sqlite"), sink).await.unwrap();
    api::vault::vault_init(&state, "correct horse battery".into()).await.unwrap();
    let session = api::sessions::session_create(&state, ssh_session(srv.port())).await.unwrap();
    let cid = api::ssh::ssh_connect(&state, session.id, 80, 24, true, Some(true), Box::new(Discard)).await.unwrap().connection_id;
    api::sftp::sftp_open(&state, cid).await.unwrap();

    // Walk: parents first, files with relative paths, empty folders kept.
    let tree = api::sftp::sftp_walk(&state, cid, "/t".into()).await.unwrap();
    // Children are recorded when their parent is listed: parents first.
    assert_eq!(tree.dirs, ["a", "empty", "a/b"]);
    let files: Vec<&str> = tree.files.iter().map(|(rel, _)| rel.as_str()).collect();
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(sorted, ["a/b/deep.txt", "a/mid.txt", "top.txt"]);
    assert!(tree.errors.is_empty());
    assert!(api::sftp::sftp_walk(&state, cid, "/nope".into()).await.is_err(), "an unlistable root is an error");

    // Cancel a running upload: the partial remote file goes away.
    let t = api::sftp::sftp_upload(&state, cid, local.join("up.bin").display().to_string(), "/up.bin".into()).await.unwrap();
    let p = wait_progress(&rx, t.transfer_id);
    assert!(!p.done, "still running when cancelled");
    assert!(api::sftp::sftp_transfer_cancel(&state, t.transfer_id).await.unwrap());
    let p = wait_transfer(&rx, t.transfer_id);
    assert!(p.cancelled && p.error.is_none(), "{p:?}");
    assert!(!remote_root.join("up.bin").exists(), "partial upload removed");
    // Gone from the registry once ended.
    assert!(!api::sftp::sftp_transfer_cancel(&state, t.transfer_id).await.unwrap());

    // Cancel a running download: the partial local file goes away.
    let back = local.join("big.back");
    let t = api::sftp::sftp_download(&state, cid, "/big.bin".into(), back.display().to_string()).await.unwrap();
    wait_progress(&rx, t.transfer_id);
    api::sftp::sftp_transfer_cancel(&state, t.transfer_id).await.unwrap();
    assert!(wait_transfer(&rx, t.transfer_id).cancelled);
    assert!(!back.exists(), "partial download removed");

    // A queued file of a batch is cancelled before it starts; the others run.
    std::fs::write(local.join("s1"), b"one").unwrap();
    std::fs::write(local.join("s2"), b"two").unwrap();
    let items = ["s1", "s2"]
        .map(|n| api::sftp::TransferItem { local_path: local.join(n).display().to_string(), remote_path: format!("/{n}") })
        .to_vec();
    let mut all = vec![api::sftp::TransferItem { local_path: local.join("up.bin").display().to_string(), remote_path: "/slow".into() }];
    all.extend(items);
    let t = api::sftp::sftp_upload_many(&state, cid, all).await.unwrap();
    assert!(api::sftp::sftp_transfer_cancel(&state, t[1].transfer_id).await.unwrap(), "queued s1 cancellable");
    api::sftp::sftp_transfer_cancel(&state, t[0].transfer_id).await.unwrap();
    assert!(wait_transfer(&rx, t[0].transfer_id).cancelled);
    assert!(wait_transfer(&rx, t[1].transfer_id).cancelled);
    assert_eq!(wait_transfer(&rx, t[2].transfer_id).error, None);
    assert!(!remote_root.join("s1").exists() && !remote_root.join("slow").exists());
    assert_eq!(std::fs::read(remote_root.join("s2")).unwrap(), b"two");
}
