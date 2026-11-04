//! Updater tests: manifests and signatures made by `ezssh-release` (as the
//! release job makes them), served by a small local HTTP server.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::install::{self, AfterInstall};
use super::*;

async fn serve(files: HashMap<String, Vec<u8>>) -> String {
    ezssh_testsrv::http::serve(files)
        .await
        .expect("http test server")
}

struct Keys {
    sk: ezssh_release::minisign::SecretKey,
    pubkey: String,
}

fn keys() -> Keys {
    let k = ezssh_release::keygen(None).unwrap();
    Keys {
        sk: ezssh_release::load_secret_key(&k.secret, None).unwrap(),
        pubkey: ezssh_release::public_key_line(&k.public),
    }
}

fn sign(k: &Keys, data: &[u8], name: &str, version: &str) -> String {
    ezssh_release::sign(&k.sk, Cursor::new(data), name, version, 1_700_000_000).unwrap()
}

fn install_of(kind: InstallKind) -> Install {
    Install {
        kind,
        target: PathBuf::from("/x/ezssh"),
        exe: PathBuf::from("/x/ezssh"),
        args: vec![],
    }
}

fn cfg(kind: InstallKind, os: &'static str, current: &str, pubkey: &str) -> Config {
    Config {
        repo: "owner/repo".into(),
        pubkey: pubkey.into(),
        current: Version::parse(current).unwrap(),
        install: install_of(kind),
        manifest_override: None,
        os,
        arch: "x86_64",
    }
}

fn manifest(version: &str, entries: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut platforms = serde_json::Map::new();
    for (k, url, sig) in entries {
        platforms.insert(
            (*k).into(),
            serde_json::json!({ "url": url, "signature": sig }),
        );
    }
    serde_json::to_vec(&serde_json::json!({
        "version": version, "notes": "Fixes.", "pub_date": "2026-11-02T10:00:00Z", "platforms": platforms,
    }))
    .unwrap()
}

#[test]
fn targets_per_kind() {
    assert_eq!(
        cfg(InstallKind::Nsis, "windows", "1.0.0", "").targets(),
        ["windows-x86_64-nsis", "windows-x86_64"]
    );
    assert_eq!(
        cfg(InstallKind::Msi, "windows", "1.0.0", "").targets(),
        ["windows-x86_64-msi"]
    );
    assert_eq!(
        cfg(InstallKind::AppImage, "linux", "1.0.0", "").targets(),
        ["linux-x86_64-appimage", "linux-x86_64"]
    );
    assert_eq!(
        cfg(InstallKind::Deb, "linux", "1.0.0", "").targets(),
        ["linux-x86_64-deb"]
    );
    assert_eq!(
        cfg(InstallKind::App, "darwin", "1.0.0", "").targets(),
        ["darwin-x86_64-app", "darwin-x86_64"]
    );
    assert!(
        cfg(InstallKind::Portable, "linux", "1.0.0", "")
            .targets()
            .is_empty()
    );
    assert!(
        cfg(InstallKind::Unpackaged, "linux", "1.0.0", "")
            .targets()
            .is_empty()
    );
}

#[test]
fn evaluate_picks_the_installer_for_this_copy() {
    let m = manifest(
        "v1.2.0",
        &[
            ("linux-x86_64", "u-appimage", "s1"),
            ("linux-x86_64-appimage", "u-appimage", "s1"),
            ("linux-x86_64-deb", "u-deb", "s2"),
        ],
    );
    let deb = evaluate(&cfg(InstallKind::Deb, "linux", "1.1.9", "k"), &m, false)
        .unwrap()
        .unwrap();
    assert_eq!(deb.version, "1.2.0");
    assert_eq!(deb.url.as_deref(), Some("u-deb"));
    assert_eq!(deb.target.as_deref(), Some("linux-x86_64-deb"));
    assert_eq!(deb.notes.as_deref(), Some("Fixes."));
    assert_eq!(
        deb.release_page,
        "https://github.com/owner/repo/releases/tag/v1.2.0"
    );
    // Same or older: nothing.
    assert!(
        evaluate(&cfg(InstallKind::Deb, "linux", "1.2.0", "k"), &m, false)
            .unwrap()
            .is_none()
    );
    assert!(
        evaluate(&cfg(InstallKind::Deb, "linux", "1.3.0", "k"), &m, false)
            .unwrap()
            .is_none()
    );
    // A pre-release of the running version is older than the release.
    assert!(
        evaluate(
            &cfg(InstallKind::Deb, "linux", "1.2.0-beta.1", "k"),
            &m,
            true
        )
        .unwrap()
        .is_some()
    );
    // Portable copies hear about it but get no installer.
    let p = evaluate(
        &cfg(InstallKind::Portable, "linux", "1.0.0", "k"),
        &m,
        false,
    )
    .unwrap()
    .unwrap();
    assert!(!p.installable());
    // A deb copy never falls back to the bare key (an AppImage).
    let only_bare = manifest("1.2.0", &[("linux-x86_64", "u-appimage", "s1")]);
    let d = evaluate(
        &cfg(InstallKind::Deb, "linux", "1.0.0", "k"),
        &only_bare,
        false,
    )
    .unwrap()
    .unwrap();
    assert!(d.url.is_none());
    let a = evaluate(
        &cfg(InstallKind::AppImage, "linux", "1.0.0", "k"),
        &only_bare,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(a.target.as_deref(), Some("linux-x86_64"));
    // Garbage is an error, not "no update".
    assert!(evaluate(&cfg(InstallKind::Deb, "linux", "1.0.0", "k"), b"{}", false).is_err());
    assert!(
        evaluate(
            &cfg(InstallKind::Deb, "linux", "1.0.0", "k"),
            br#"{"version":"x"}"#,
            false
        )
        .is_err()
    );
}

#[test]
fn signed_version_is_required_and_must_match() {
    check_signed_version("timestamp:1\tfile:f\tversion:1.2.0", "1.2.0").unwrap();
    check_signed_version("timestamp:1\tfile:f\tversion:v1.2.0", "1.2.0").unwrap();
    assert!(check_signed_version("timestamp:1\tfile:f\tversion:1.1.0", "1.2.0").is_err());
    assert!(check_signed_version("timestamp:1\tfile:f", "1.2.0").is_err());
}

#[test]
fn download_names_are_safe() {
    assert_eq!(
        file_name_from_url("https://h/d/v1/ezssh_1.2.0_amd64.deb?x=1"),
        "ezssh_1.2.0_amd64.deb"
    );
    assert_eq!(
        file_name_from_url("https://h/d/..%2F..%2Fevil"),
        "_2F.._2Fevil"
    );
    assert_eq!(file_name_from_url("https://h/"), "ezssh-update");
}

#[test]
fn detect_portable_and_unpackaged() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("ezssh");
    std::fs::write(&exe, b"").unwrap();
    let args = vec![OsString::from("--db"), OsString::from("/tmp/x")];
    let i = detect_from(&exe, None, args.clone());
    assert_eq!(i.kind, InstallKind::Unpackaged);
    assert_eq!(i.args, args);
    std::fs::write(dir.path().join(PORTABLE_MARKER), b"portable").unwrap();
    assert_eq!(detect_from(&exe, None, vec![]).kind, InstallKind::Portable);
}

#[cfg(target_os = "linux")]
#[test]
fn detect_appimage() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("ezssh_1.0.0_x86_64.AppImage");
    std::fs::write(&img, b"").unwrap();
    let i = detect_from(
        Path::new("/tmp/.mount_ezsshXYZ/usr/bin/ezssh"),
        Some(img.clone()),
        vec![],
    );
    assert_eq!(i.kind, InstallKind::AppImage);
    assert_eq!(i.target, img);
    assert_eq!(i.exe, img);
}

#[tokio::test]
async fn check_and_download_verify_end_to_end() {
    let k = keys();
    let installer = vec![7u8; 300_000];
    let name = "ezssh_1.2.0_amd64.deb";
    let sig = sign(&k, &installer, name, "1.2.0");
    let wrong_version = sign(&k, &installer, name, "1.1.0");
    let other_key = sign(&keys(), &installer, name, "1.2.0");
    let mut tampered = installer.clone();
    tampered[1000] ^= 1;

    let mut files = HashMap::new();
    files.insert(format!("/dl/{name}"), installer.clone());
    files.insert("/bad/ezssh_1.2.0_amd64.deb".to_string(), tampered);
    let base = serve(files.clone()).await;
    let m = manifest(
        "1.2.0",
        &[("linux-x86_64-deb", &format!("{base}/dl/{name}"), &sig)],
    );
    files.insert("/latest.json".into(), m);
    let base = serve(files).await;

    let mut c = cfg(InstallKind::Deb, "linux", "1.1.0", &k.pubkey);
    c.manifest_override = Some(format!("{base}/latest.json"));
    let update = check(&c, false).await.unwrap().expect("an update");
    assert!(update.installable());

    let dir = tempfile::tempdir().unwrap();
    let mut seen = Vec::new();
    let path = download(&c, &update, dir.path(), |d, t| seen.push((d, t)))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), installer);
    assert_eq!(path.file_name().unwrap(), name);
    assert_eq!(seen.first(), Some(&(0, Some(300_000))));
    assert_eq!(seen.last(), Some(&(300_000, Some(300_000))));

    // Tampered bytes: refused after the download, file removed.
    let mut bad = update.clone();
    bad.url = Some(format!("{base}/bad/ezssh_1.2.0_amd64.deb"));
    let d2 = tempfile::tempdir().unwrap();
    let e = download(&c, &bad, d2.path(), |_, _| {})
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("does not verify"), "{e}");
    assert!(!d2.path().join(name).exists());

    // Signed for another version, or by another key: refused before downloading.
    let mut v = update.clone();
    v.signature = Some(wrong_version);
    let e = download(&c, &v, d2.path(), |_, _| {})
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("signed for 1.1.0"), "{e}");
    let mut o = update.clone();
    o.signature = Some(other_key);
    let e = download(&c, &o, d2.path(), |_, _| {})
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("another key"), "{e}");

    // A body shorter than announced fails.
    let mut s = update.clone();
    s.url = Some(format!("{base}/short/dl/{name}"));
    assert!(download(&c, &s, d2.path(), |_, _| {}).await.is_err());

    // No manifest published: a clear error; no key compiled in: refused.
    let mut none = c.clone();
    none.manifest_override = Some(format!("{base}/missing.json"));
    let e = check(&none, false).await.unwrap_err().to_string();
    assert!(e.contains("no release with an update manifest"), "{e}");
    let mut nokey = c.clone();
    nokey.pubkey.clear();
    assert!(
        check(&nokey, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("no update key")
    );
}

#[test]
fn release_tool_manifest_reads_back() {
    // The manifest builder of the release job and the reader agree.
    let assets = [
        ezssh_release::Signed {
            file_name: "ezssh_2.0.0_x64-setup.exe".into(),
            signature: "A".into(),
        },
        ezssh_release::Signed {
            file_name: "ezssh_2.0.0_x64_en-US.msi".into(),
            signature: "B".into(),
        },
        ezssh_release::Signed {
            file_name: "ezssh_2.0.0_x86_64.AppImage".into(),
            signature: "C".into(),
        },
        ezssh_release::Signed {
            file_name: "ezssh_2.0.0_amd64.deb".into(),
            signature: "D".into(),
        },
        ezssh_release::Signed {
            file_name: "ezssh_2.0.0_aarch64.app.tar.gz".into(),
            signature: "E".into(),
        },
    ];
    let m = ezssh_release::manifest(
        "2.0.0",
        "",
        "2026-11-02T00:00:00Z",
        "https://h/v2.0.0",
        &assets,
    )
    .unwrap();
    let bytes = serde_json::to_vec(&m).unwrap();
    let pick = |kind, os| {
        evaluate(&cfg(kind, os, "1.0.0", "k"), &bytes, false)
            .unwrap()
            .unwrap()
            .signature
    };
    assert_eq!(pick(InstallKind::Nsis, "windows").as_deref(), Some("A"));
    assert_eq!(pick(InstallKind::Msi, "windows").as_deref(), Some("B"));
    assert_eq!(pick(InstallKind::AppImage, "linux").as_deref(), Some("C"));
    assert_eq!(pick(InstallKind::Deb, "linux").as_deref(), Some("D"));
    let mut mac = cfg(InstallKind::App, "darwin", "1.0.0", "k");
    mac.arch = "aarch64";
    assert_eq!(
        evaluate(&mac, &bytes, false)
            .unwrap()
            .unwrap()
            .signature
            .as_deref(),
        Some("E")
    );
}

#[cfg(unix)]
#[test]
fn appimage_is_replaced_atomically_with_its_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("ezssh.AppImage");
    std::fs::write(&target, b"old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o750)).unwrap();
    let dl = tempfile::tempdir().unwrap();
    let new = dl.path().join("ezssh_2.0.0_x86_64.AppImage");
    std::fs::write(&new, b"new").unwrap();
    let mut inst = install_of(InstallKind::AppImage);
    inst.target = target.clone();
    inst.args = vec![OsString::from("--db"), OsString::from("x")];
    let after = install::install(&inst, &new).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert!(!new.exists(), "download removed");
    assert_eq!(
        after,
        AfterInstall::Relaunch {
            program: target,
            args: inst.args.clone()
        }
    );
    // No stray temp file left next to it.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn app_bundle_swap_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("ezSsh.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    std::fs::write(app.join("Contents/MacOS/ezssh"), b"old").unwrap();

    // Build ezSsh.app.tar.gz with a new executable.
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("ezSsh.app/Contents/MacOS")).unwrap();
    std::fs::write(src.path().join("ezSsh.app/Contents/MacOS/ezssh"), b"new").unwrap();
    let tgz = src.path().join("ezssh_2.0.0_aarch64.app.tar.gz");
    {
        let f = std::fs::File::create(&tgz).unwrap();
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            f,
            flate2::Compression::fast(),
        ));
        b.append_dir_all("ezSsh.app", src.path().join("ezSsh.app"))
            .unwrap();
        b.into_inner().unwrap().finish().unwrap();
    }
    install::swap_app_bundle(&app, &tgz).unwrap();
    assert_eq!(
        std::fs::read(app.join("Contents/MacOS/ezssh")).unwrap(),
        b"new"
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        [OsString::from("ezSsh.app")],
        "staging and backup removed"
    );

    // An archive without an .app leaves the installed bundle alone.
    let empty = src.path().join("empty.tar.gz");
    {
        let f = std::fs::File::create(&empty).unwrap();
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            f,
            flate2::Compression::fast(),
        ));
        b.append_dir_all("notes", src.path().join("ezSsh.app/Contents"))
            .unwrap();
        b.into_inner().unwrap().finish().unwrap();
    }
    assert!(install::swap_app_bundle(&app, &empty).is_err());
    assert_eq!(
        std::fs::read(app.join("Contents/MacOS/ezssh")).unwrap(),
        b"new"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn portable_copies_are_not_installed_over() {
    let e = install::install(&install_of(InstallKind::Portable), Path::new("/nope")).unwrap_err();
    assert!(e.to_string().contains("cannot update itself"));
}

#[test]
fn windows_helper_script() {
    let s = install::windows_script(
        InstallKind::Nsis,
        Path::new(r"C:\Users\O'Brien\AppData\Local\Temp\ezssh-update-1\ezssh_2.0.0_x64-setup.exe"),
        4242,
        Path::new(r"C:\Users\O'Brien\AppData\Local\ezSsh\ezssh.exe"),
        &[
            OsString::from("--db"),
            OsString::from(r"C:\My Data\ezterm.sqlite"),
        ],
        Path::new(r"C:\Temp\ezssh-update.log"),
    );
    assert!(s.contains("Wait-Process -Id 4242"), "{s}");
    assert!(s.contains(r"-FilePath 'C:\Users\O''Brien\AppData\Local\Temp\ezssh-update-1\ezssh_2.0.0_x64-setup.exe' -ArgumentList '/P' -Wait -PassThru"), "{s}");
    assert!(s.contains(r"Remove-Item -LiteralPath 'C:\Users\O''Brien\AppData\Local\Temp\ezssh-update-1' -Recurse"), "{s}");
    assert!(s.ends_with("Start-Process -FilePath 'C:\\Users\\O''Brien\\AppData\\Local\\ezSsh\\ezssh.exe' -ArgumentList '--db','\"C:\\My Data\\ezterm.sqlite\"'\n"), "{s}");

    let m = install::windows_script(
        InstallKind::Msi,
        Path::new(r"C:\T\d\ezssh_2.0.0_x64_en-US.msi"),
        1,
        Path::new(r"C:\Program Files\ezSsh\ezssh.exe"),
        &[],
        Path::new(r"C:\T\l.log"),
    );
    assert!(m.contains(r#"msiexec.exe" -ArgumentList '/i','C:\T\d\ezssh_2.0.0_x64_en-US.msi','/passive','/norestart' -Wait -PassThru"#), "{m}");
    assert!(
        m.ends_with("Start-Process -FilePath 'C:\\Program Files\\ezSsh\\ezssh.exe'\n"),
        "{m}"
    );
    // A file outside a download folder: only the file goes.
    assert!(
        m.contains(r"Remove-Item -LiteralPath 'C:\T\d\ezssh_2.0.0_x64_en-US.msi' -Force"),
        "{m}"
    );
}
