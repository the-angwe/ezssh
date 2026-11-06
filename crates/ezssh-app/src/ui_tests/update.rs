//! The update dialog against a local release: the launch check finds a
//! signed update, Install downloads and verifies it, replaces the
//! (AppImage) file and starts it; the status-bar button reports up to date
//! and refuses a tampered installer.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Cursor;
use std::path::Path;

use ezssh_core::updater::{Config, Install, InstallKind, Version};

use super::*;

const NEW: &str = "9.9.9";

struct Release {
    _rt: tokio::runtime::Runtime,
    base: String,
    pubkey: String,
}

/// Serve `latest.json` (announcing `version`) and the AppImage `body`,
/// signed for `NEW`; `tamper` flips a byte after signing.
fn release(version: &str, body: &[u8], tamper: bool) -> Release {
    let keys = ezssh_release::keygen(None).unwrap();
    let sk = ezssh_release::load_secret_key(&keys.secret, None).unwrap();
    let name = format!("ezssh_{NEW}_x86_64.AppImage");
    let sig = ezssh_release::sign(&sk, Cursor::new(body), &name, NEW, 1_700_000_000).unwrap();
    let mut served = body.to_vec();
    if tamper {
        served[0] ^= 1;
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut files = HashMap::new();
    files.insert(format!("/dl/{name}"), served);
    let base = rt
        .block_on(ezssh_testsrv::http::serve(files.clone()))
        .unwrap();
    let assets = [ezssh_release::Signed {
        file_name: name,
        signature: sig,
    }];
    let m = ezssh_release::manifest(
        version,
        "Faster.\nSafer.",
        "2026-11-02T10:00:00Z",
        &format!("{base}/dl"),
        &assets,
    )
    .unwrap();
    files.insert("/latest.json".into(), serde_json::to_vec(&m).unwrap());
    let base = rt.block_on(ezssh_testsrv::http::serve(files)).unwrap();
    Release {
        _rt: rt,
        base,
        pubkey: ezssh_release::public_key_line(&keys.public),
    }
}

fn config(r: &Release, appimage: &Path) -> Config {
    Config {
        repo: "owner/repo".into(),
        pubkey: r.pubkey.clone(),
        current: Version::parse("1.0.0").unwrap(),
        install: Install {
            kind: InstallKind::AppImage,
            target: appimage.to_path_buf(),
            exe: appimage.to_path_buf(),
            args: vec![OsString::from("--flag")],
        },
        manifest_override: Some(format!("{}/latest.json", r.base)),
        os: "linux",
        arch: "x86_64",
    }
}

#[test]
fn launch_check_install_and_restart() {
    use std::os::unix::fs::PermissionsExt as _;
    let work = tempfile::tempdir().unwrap();
    let appimage = work.path().join("ezSsh.AppImage");
    std::fs::write(&appimage, b"#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&appimage, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The "new version" leaves a marker with its arguments when started.
    let marker = work.path().join("started");
    let body = format!("#!/bin/sh\necho \"$@\" > '{}'\n", marker.display());
    let r = release(NEW, body.as_bytes(), false);

    let mut env = start_with(Some(config(&r, &appimage)));
    unlock_fresh(&mut env);
    let h = &mut env.h;
    // Settings load → launch check → the dialog opens on its own.
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while h.query_by_label("Install & restart").is_none() {
        assert!(
            std::time::Instant::now() < until,
            "the launch check opened no dialog"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(h.query_by_label(NEW).is_some(), "new version shown");
    assert!(h.query_by_label("1.0.0").is_some(), "current version shown");
    assert!(
        h.state().layout.last_update_check.is_some(),
        "check time kept"
    );
    assert!(h.state().updater.available.is_some());
    shot(h, "update-available");

    // The card re-centres for a few frames after its body grew.
    h.run_steps(5);
    h.get_by_label("Install & restart").click();
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < until,
            "the new version was not started"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(
        std::fs::read_to_string(&appimage).unwrap(),
        body,
        "AppImage replaced"
    );
    assert_eq!(
        std::fs::metadata(&appimage).unwrap().permissions().mode() & 0o777,
        0o755
    );
    // Started with the same arguments.
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(&marker).unwrap_or_default().trim() != "--flag" {
        assert!(
            std::time::Instant::now() < until,
            "marker: {:?}",
            std::fs::read_to_string(&marker)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn up_to_date_then_a_tampered_installer_is_refused() {
    let work = tempfile::tempdir().unwrap();
    let appimage = work.path().join("ezSsh.AppImage");
    std::fs::write(&appimage, b"old").unwrap();

    // Same version as the running one: nothing to do; launch check silent.
    let same = release("1.0.0", b"x", false);
    let mut env = start_with(Some(config(&same, &appimage)));
    unlock_fresh(&mut env);
    let h = &mut env.h;
    h.run_steps(30);
    assert!(h.query_by_label("Install & restart").is_none());
    h.get_by_label("Check for updates").click();
    let until = std::time::Instant::now() + Duration::from_secs(20);
    let latest = format!(
        "You\u{2019}re on the latest version (v{}).",
        env!("CARGO_PKG_VERSION")
    );
    while h.query_by_label(&latest).is_none() {
        assert!(std::time::Instant::now() < until, "no up-to-date line");
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    shot(h, "update-latest");
    // The footer just gained "Check again": its right-aligned group may
    // take a frame to settle.
    h.run_steps(2);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(!h.state().updater.dialog_open());

    // A newer release whose installer was changed after signing.
    let bad = release(NEW, b"new bytes", true);
    h.state_mut().updater.cfg = Some(config(&bad, &appimage));
    h.get_by_label("Check for updates").click();
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while h.query_by_label("Install & restart").is_none() {
        assert!(std::time::Instant::now() < until, "no update offered");
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    // The card re-centres for a few frames after its body grew.
    h.run_steps(5);
    h.get_by_label("Install & restart").click();
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while h.query_by_label_contains("does not verify").is_none() {
        assert!(std::time::Instant::now() < until, "no signature error");
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    shot(h, "update-refused");
    assert_eq!(
        std::fs::read(&appimage).unwrap(),
        b"old",
        "nothing installed"
    );
    assert!(h.query_by_label("Check again").is_some());
}
