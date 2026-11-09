//! Port forward lifecycle against the in-process test server.
//!
//! - Across a dropped connection: when the SSH link dies (peer gone, or
//!   silent and caught by keepalives) the connection's forwards must stop
//!   and release their local ports, so the reconnect's auto-start can bind
//!   them again.
//! - A local port already taken: the amber conflict names the tab holding
//!   it, or says the holder is outside this app's tabs.
//!
//! A TCP proxy sits between ezssh-core and the in-process test server so
//! the link can be cut (sockets closed) or blackholed (sockets kept open,
//! nothing relayed — a suspended laptop or a dead NAT mapping).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ezssh_core::api;
use ezssh_core::db::forwards::ForwardInput;
use ezssh_core::db::sessions::SessionInput;
use ezssh_core::events::{self, Event, OutputSink};
use ezssh_core::ssh::forwards::ForwardStatus;
use ezssh_core::{AppState, Secret};
use ezssh_testsrv::{Config, TestServer};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

struct Discard;

impl OutputSink for Discard {
    fn send(&self, _: &[u8]) -> bool {
        true
    }
}

const PASS: u8 = 0;
const BLACKHOLE: u8 = 1;
const CUT: u8 = 2;

/// Relays every accepted connection to `upstream` while the mode is PASS.
/// On BLACKHOLE it stops relaying but keeps both sockets open; on CUT it
/// closes them. Connections accepted later relay normally again once the
/// mode is back to PASS.
async fn proxy(upstream: u16) -> (u16, watch::Sender<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (mode_tx, _) = watch::channel(PASS);
    let mode = mode_tx.clone();
    tokio::spawn(async move {
        loop {
            let (client, _) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => panic!("proxy accept: {e}"),
            };
            let up = TcpStream::connect(("127.0.0.1", upstream)).await.unwrap();
            let mut rx = mode.subscribe();
            tokio::spawn(async move {
                let (mut cr, mut cw) = client.into_split();
                let (mut ur, mut uw) = up.into_split();
                let changed = {
                    let a = tokio::io::copy(&mut cr, &mut uw);
                    let b = tokio::io::copy(&mut ur, &mut cw);
                    tokio::select! {
                        _ = a => None,
                        _ = b => None,
                        m = rx.wait_for(|m| *m != PASS) => m.ok().map(|m| *m),
                    }
                };
                if changed == Some(BLACKHOLE) {
                    // Keep all four halves alive, relay nothing.
                    let _hold = (cr, cw, ur, uw);
                    std::future::pending::<()>().await;
                }
            });
        }
    });
    (port, mode_tx)
}

fn ssh_session(port: u16, keepalive_secs: i64) -> SessionInput {
    named_session("testsrv", port, keepalive_secs)
}

fn named_session(name: &str, port: u16, keepalive_secs: i64) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: name.into(),
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
        keepalive_secs,
        connect_timeout_secs: 10,
        env: Vec::new(),
        session_kind: "ssh".into(),
        forward_x11: 0,
        starting_dir: None,
        password: Some(Secret::new("pw".into())),
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Waits (off the runtime thread) for an event matching `f`.
async fn wait_event(
    rx: &std::sync::Arc<std::sync::Mutex<mpsc::Receiver<Event>>>,
    secs: u64,
    what: &'static str,
    f: impl Fn(&Event) -> bool + Send + 'static,
) -> Event {
    let rx = rx.clone();
    tokio::task::spawn_blocking(move || {
        let rx = rx.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(e) if f(&e) => return e,
                Ok(e) => eprintln!("[{what}] skipped {e:?}"),
                Err(e) => panic!("no event `{what}` within {secs}s: {e}"),
            }
        }
    })
    .await
    .unwrap()
}

fn forward_status(cid: u64) -> impl Fn(&Event) -> bool {
    move |e| matches!(e, Event::ForwardStatus { connection_id, .. } if *connection_id == cid)
}

async fn drop_and_reconnect(mode: u8, keepalive_secs: i64) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().unwrap();
    let remote_root = dir.path().join("remote");
    std::fs::create_dir_all(remote_root.join("home/u")).unwrap();
    let mut cfg = Config::new(&remote_root, "u", "pw");
    cfg.home = "/home/u".into();
    let srv = TestServer::start(cfg, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let (proxy_port, proxy_mode) = proxy(srv.port()).await;

    let (sink, rx) = events::channel(|| {});
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let state = AppState::open(&dir.path().join("ezterm.sqlite"), sink)
        .await
        .unwrap();
    api::vault::vault_init(&state, "correct horse battery".into())
        .await
        .unwrap();
    let session = api::sessions::session_create(&state, ssh_session(proxy_port, keepalive_secs))
        .await
        .unwrap();
    let fwd_port = free_port();
    api::forwards::forward_create(
        &state,
        session.id,
        ForwardInput {
            name: "l".into(),
            kind: "local".into(),
            bind_addr: "127.0.0.1".into(),
            bind_port: fwd_port.into(),
            dest_addr: "127.0.0.1".into(),
            dest_port: 9,
            auto_start: 1,
        },
    )
    .await
    .unwrap();

    let connect = || {
        api::ssh::ssh_connect(
            &state,
            session.id,
            80,
            24,
            true,
            Some(true),
            Box::new(Discard),
        )
    };
    let cid = connect().await.expect("connect").connection_id;
    let e = wait_event(&rx, 10, "first Running", forward_status(cid)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    assert!(
        matches!(summary.status, ForwardStatus::Running),
        "{:?}",
        summary.status
    );
    assert!(!port_free(fwd_port), "forward port not bound");

    // Kill the link.
    proxy_mode.send_replace(mode);
    let started = Instant::now();
    wait_event(
        &rx,
        30,
        "Closed",
        move |e| matches!(e, Event::Closed { connection_id, .. } if *connection_id == cid),
    )
    .await;
    eprintln!("closed after {:?}", started.elapsed());
    let e = wait_event(&rx, 10, "Stopped", forward_status(cid)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    assert!(
        matches!(summary.status, ForwardStatus::Stopped),
        "{:?}",
        summary.status
    );
    assert!(
        port_free(fwd_port),
        "forward port still bound after the drop"
    );

    // Reconnect: the auto-start forward comes back Running, not Conflict.
    proxy_mode.send_replace(PASS);
    let cid2 = connect().await.expect("reconnect").connection_id;
    let e = wait_event(&rx, 10, "second Running", forward_status(cid2)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    assert!(
        matches!(summary.status, ForwardStatus::Running),
        "{:?}",
        summary.status
    );
    api::ssh::ssh_disconnect(&state, cid2).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_released_when_link_cut() {
    drop_and_reconnect(CUT, 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_released_when_link_goes_silent() {
    drop_and_reconnect(BLACKHOLE, 1).await;
}

/// The link goes silent and nothing notices (no keepalives); the user
/// reconnects by hand. The app fires the disconnect and the new connect
/// at the same time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_released_on_manual_reconnect_of_silent_link() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().unwrap();
    let remote_root = dir.path().join("remote");
    std::fs::create_dir_all(remote_root.join("home/u")).unwrap();
    let mut cfg = Config::new(&remote_root, "u", "pw");
    cfg.home = "/home/u".into();
    let srv = TestServer::start(cfg, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let (proxy_port, proxy_mode) = proxy(srv.port()).await;
    let (sink, rx) = events::channel(|| {});
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let state = std::sync::Arc::new(
        AppState::open(&dir.path().join("ezterm.sqlite"), sink)
            .await
            .unwrap(),
    );
    api::vault::vault_init(&state, "correct horse battery".into())
        .await
        .unwrap();
    let session = api::sessions::session_create(&state, ssh_session(proxy_port, 0))
        .await
        .unwrap();
    let fwd_port = free_port();
    api::forwards::forward_create(
        &state,
        session.id,
        ForwardInput {
            name: "l".into(),
            kind: "local".into(),
            bind_addr: "127.0.0.1".into(),
            bind_port: fwd_port.into(),
            dest_addr: "127.0.0.1".into(),
            dest_port: 9,
            auto_start: 1,
        },
    )
    .await
    .unwrap();
    let sid = session.id;
    let cid = api::ssh::ssh_connect(&state, sid, 80, 24, true, Some(true), Box::new(Discard))
        .await
        .expect("connect")
        .connection_id;
    wait_event(&rx, 10, "first Running", forward_status(cid)).await;

    proxy_mode.send_replace(BLACKHOLE);
    tokio::time::sleep(Duration::from_millis(200)).await;
    proxy_mode.send_replace(PASS);
    let st = state.clone();
    let disc = tokio::spawn(async move { api::ssh::ssh_disconnect(&st, cid).await });
    let cid2 = api::ssh::ssh_connect(&state, sid, 80, 24, true, Some(true), Box::new(Discard))
        .await
        .expect("reconnect")
        .connection_id;
    tokio::time::timeout(Duration::from_secs(10), disc)
        .await
        .expect("disconnect hung")
        .unwrap()
        .unwrap();
    let e = wait_event(&rx, 10, "second forward status", forward_status(cid2)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    assert!(
        matches!(summary.status, ForwardStatus::Running),
        "{:?}",
        summary.status
    );
}

fn local_forward(port: u16) -> ForwardInput {
    ForwardInput {
        name: String::new(),
        kind: "local".into(),
        bind_addr: "127.0.0.1".into(),
        bind_port: port.into(),
        dest_addr: "127.0.0.1".into(),
        dest_port: 80,
        auto_start: 1,
    }
}

/// Two sessions auto-start the same local port: the second tab's conflict
/// names the first tab. A port held outside the app's tabs says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conflict_names_the_holding_tab() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().unwrap();
    let remote_root = dir.path().join("remote");
    std::fs::create_dir_all(remote_root.join("home/u")).unwrap();
    let mut cfg = Config::new(&remote_root, "u", "pw");
    cfg.home = "/home/u".into();
    let srv = TestServer::start(cfg, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let (sink, rx) = events::channel(|| {});
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let state = AppState::open(&dir.path().join("ezterm.sqlite"), sink)
        .await
        .unwrap();
    api::vault::vault_init(&state, "correct horse battery".into())
        .await
        .unwrap();
    let dn = api::sessions::session_create(&state, named_session("dn", srv.port(), 0))
        .await
        .unwrap();
    let avoro = api::sessions::session_create(&state, named_session("avoro", srv.port(), 0))
        .await
        .unwrap();
    let port = free_port();
    for s in [dn.id, avoro.id] {
        api::forwards::forward_create(&state, s, local_forward(port))
            .await
            .unwrap();
    }
    let connect =
        |sid| api::ssh::ssh_connect(&state, sid, 80, 24, true, Some(true), Box::new(Discard));

    let c_dn = connect(dn.id).await.unwrap().connection_id;
    let e = wait_event(&rx, 10, "dn Running", forward_status(c_dn)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    assert!(
        matches!(summary.status, ForwardStatus::Running),
        "{:?}",
        summary.status
    );

    let c_av = connect(avoro.id).await.unwrap().connection_id;
    let e = wait_event(&rx, 10, "avoro Conflict", forward_status(c_av)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    match &summary.status {
        ForwardStatus::Conflict { message } => assert_eq!(
            message,
            &format!("127.0.0.1:{port} is in use by the forward of tab \"dn\"")
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
    api::ssh::ssh_disconnect(&state, c_av).await.unwrap();
    api::ssh::ssh_disconnect(&state, c_dn).await.unwrap();
    wait_event(&rx, 10, "dn Stopped", move |e| {
        matches!(e, Event::ForwardStatus { connection_id, summary }
            if *connection_id == c_dn && matches!(summary.status, ForwardStatus::Stopped))
    })
    .await;

    // Held by something that is not a tab of this app.
    let _outside = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let c_av = connect(avoro.id).await.unwrap().connection_id;
    let e = wait_event(&rx, 10, "outside Conflict", forward_status(c_av)).await;
    let Event::ForwardStatus { summary, .. } = e else {
        unreachable!()
    };
    match &summary.status {
        ForwardStatus::Conflict { message } => assert_eq!(
            message,
            &format!("127.0.0.1:{port} is in use by another program or ezSsh window")
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
    api::ssh::ssh_disconnect(&state, c_av).await.unwrap();
}
