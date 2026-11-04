//! Installing a verified download and starting the new version.
//!
//! Blocking code: run it on a blocking thread.
//!
//! | Kind | Install | Restart |
//! |---|---|---|
//! | NSIS / MSI | a hidden PowerShell helper waits for this process to exit, runs the installer passively (`setup.exe /P`, `msiexec /i … /passive /norestart`), deletes the download and starts the app | the helper |
//! | AppImage | the new file is copied next to the old one and renamed over it (atomic; the running image keeps its inode) | spawn the AppImage |
//! | deb | `pkexec dpkg -i` (graphical password prompt) | spawn `/usr/bin/ezssh` |
//! | macOS app | the `.app.tar.gz` is unpacked next to the bundle, the bundles are swapped by renames, the old one removed | spawn the bundle's executable |
//!
//! Each step logs; nothing is swallowed. A failed swap puts the old file or
//! bundle back.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Install, InstallKind, err};
use crate::error::Result;

/// What the app does once [`install`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterInstall {
    /// Start this program, then exit.
    Relaunch {
        program: PathBuf,
        args: Vec<OsString>,
    },
    /// A helper takes over (Windows): just exit, promptly.
    Exit,
}

/// Install the verified `file` over `install`.
pub fn install(install: &Install, file: &Path) -> Result<AfterInstall> {
    tracing::info!(kind = ?install.kind, file = %file.display(), target = %install.target.display(), "updater: installing");
    match install.kind {
        InstallKind::Nsis | InstallKind::Msi => windows_install(install, file),
        InstallKind::AppImage => {
            replace_file(&install.target, file)?;
            remove_download(file);
            Ok(AfterInstall::Relaunch {
                program: install.target.clone(),
                args: install.args.clone(),
            })
        }
        InstallKind::Deb => {
            deb_install(file)?;
            remove_download(file);
            Ok(AfterInstall::Relaunch {
                program: install.exe.clone(),
                args: install.args.clone(),
            })
        }
        InstallKind::App => {
            swap_app_bundle(&install.target, file)?;
            remove_download(file);
            Ok(AfterInstall::Relaunch {
                program: install.exe.clone(),
                args: install.args.clone(),
            })
        }
        InstallKind::Portable | InstallKind::Unpackaged => Err(err(format!(
            "a {} cannot update itself; download the new version from the releases page",
            install.kind.label()
        ))),
    }
}

/// Start the new version (for [`AfterInstall::Relaunch`]).
pub fn relaunch(after: &AfterInstall) -> Result<()> {
    let AfterInstall::Relaunch { program, args } = after else {
        return Ok(());
    };
    tracing::info!(program = %program.display(), ?args, "updater: starting the new version");
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| err(format!("starting {}: {e}", program.display())))
}

/// Remove the download and its folder once installed.
fn remove_download(file: &Path) {
    if let Err(e) = std::fs::remove_file(file) {
        tracing::warn!(file = %file.display(), "updater: removing the download: {e}");
    }
    if let Some(dir) = file.parent()
        && let Err(e) = std::fs::remove_dir(dir)
    {
        tracing::debug!(dir = %dir.display(), "updater: download folder kept: {e}");
    }
}

// ------------------------------------------------------------ AppImage

/// Put `new` in place of `target` atomically: copy it next to `target`
/// (same file system), give it `target`'s permissions, rename it over.
#[cfg(unix)]
pub fn replace_file(target: &Path, new: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = target
        .parent()
        .ok_or_else(|| err(format!("{} has no folder", target.display())))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let mode = std::fs::metadata(target)
        .map(|m| m.permissions().mode())
        .map_err(|e| err(format!("reading {}: {e}", target.display())))?;
    let step = || -> std::io::Result<()> {
        std::fs::copy(new, &tmp)?;
        // Executable for whoever could read it.
        let exec = mode | ((mode & 0o444) >> 2);
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(exec))?;
        std::fs::File::open(&tmp)?.sync_all()?;
        std::fs::rename(&tmp, target)
    };
    if let Err(e) = step() {
        if let Err(rm) = std::fs::remove_file(&tmp)
            && rm.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(tmp = %tmp.display(), "updater: removing a partial copy: {rm}");
        }
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(" (no write access to {})", dir.display())
        } else {
            String::new()
        };
        return Err(err(format!("replacing {}: {e}{hint}", target.display())));
    }
    tracing::info!(target = %target.display(), "updater: file replaced");
    Ok(())
}

#[cfg(not(unix))]
pub fn replace_file(target: &Path, _new: &Path) -> Result<()> {
    Err(err(format!(
        "replacing {} is only done on Linux",
        target.display()
    )))
}

// ------------------------------------------------------------ deb

#[cfg(target_os = "linux")]
fn deb_install(file: &Path) -> Result<()> {
    let manual = format!(
        "install it from a terminal with: sudo dpkg -i '{}'",
        file.display()
    );
    let status = std::process::Command::new("pkexec")
        .arg("dpkg")
        .arg("-i")
        .arg(file)
        .status()
        .map_err(|e| err(format!("pkexec: {e}; {manual}")))?;
    match status.code() {
        Some(0) => {
            tracing::info!("updater: dpkg -i done");
            Ok(())
        }
        // pkexec: 126 = the password prompt was dismissed, 127 = not authorised.
        Some(126 | 127) => Err(err(format!("the password prompt was cancelled; {manual}"))),
        other => Err(err(format!("dpkg -i failed (exit {other:?}); {manual}"))),
    }
}

#[cfg(not(target_os = "linux"))]
fn deb_install(_file: &Path) -> Result<()> {
    Err(err("Debian packages install on Linux only"))
}

// ------------------------------------------------------------ macOS app

/// Unpack `tar_gz` (one `*.app` folder at its root) next to `app` and swap
/// the two bundles. The old bundle is removed after a successful swap and
/// put back after a failed one.
#[cfg(unix)]
pub fn swap_app_bundle(app: &Path, tar_gz: &Path) -> Result<()> {
    let parent = app
        .parent()
        .ok_or_else(|| err(format!("{} has no folder", app.display())))?;
    let name = app
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let pid = std::process::id();
    let staging = parent.join(format!(".{name}.update-{pid}"));
    let backup = parent.join(format!(".{name}.old-{pid}"));

    let cleanup = |p: &Path| {
        if p.exists()
            && let Err(e) = std::fs::remove_dir_all(p)
        {
            tracing::warn!(path = %p.display(), "updater: cleanup: {e}");
        }
    };
    cleanup(&staging);
    let unpack = || -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&staging)?;
        let f = std::fs::File::open(tar_gz)?;
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
        ar.set_preserve_permissions(true);
        // `unpack` refuses entries that would leave the folder.
        ar.unpack(&staging)?;
        let mut apps = std::fs::read_dir(&staging)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.extension().is_some_and(|e| e == "app"));
        let found = apps
            .next()
            .ok_or_else(|| std::io::Error::other("the archive holds no .app"))?;
        if apps.next().is_some() {
            return Err(std::io::Error::other(
                "the archive holds more than one .app",
            ));
        }
        Ok(found)
    };
    let new_app = match unpack() {
        Ok(p) => p,
        Err(e) => {
            cleanup(&staging);
            return Err(err(format!("unpacking {}: {e}", tar_gz.display())));
        }
    };
    if let Err(e) = std::fs::rename(app, &backup) {
        cleanup(&staging);
        return Err(err(format!("moving {} aside: {e}", app.display())));
    }
    if let Err(e) = std::fs::rename(&new_app, app) {
        let restored = std::fs::rename(&backup, app);
        cleanup(&staging);
        return Err(err(format!(
            "putting the new {name} in place: {e}{}",
            match restored {
                Ok(()) => String::new(),
                Err(r) => format!(
                    "; restoring the old one failed too ({r}), it is at {}",
                    backup.display()
                ),
            }
        )));
    }
    cleanup(&backup);
    cleanup(&staging);
    tracing::info!(app = %app.display(), "updater: bundle swapped");
    Ok(())
}

#[cfg(not(unix))]
pub fn swap_app_bundle(app: &Path, _tar_gz: &Path) -> Result<()> {
    Err(err(format!(
        "swapping {} is only done on macOS",
        app.display()
    )))
}

// ------------------------------------------------------------ Windows

/// A PowerShell single-quoted string literal.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// One element of a Start-Process `-ArgumentList` (Windows PowerShell 5.1
/// joins the list with spaces without quoting, so quote what has spaces).
fn ps_arg(s: &str) -> String {
    if s.is_empty() || s.contains([' ', '\t', '"']) {
        ps_quote(&format!("\"{}\"", s.replace('"', "\\\"")))
    } else {
        ps_quote(s)
    }
}

/// The helper script for a Windows install: wait for `pid` to exit, run
/// the installer passively, log its exit code to `log`, delete the
/// download folder, start `exe` with `args`.
pub fn windows_script(
    kind: InstallKind,
    installer: &Path,
    pid: u32,
    exe: &Path,
    args: &[OsString],
    log: &Path,
) -> String {
    let inst = installer.to_string_lossy();
    let mut s = String::new();
    s.push_str("$ErrorActionPreference = 'Continue'\n");
    s.push_str(&format!("$log = {}\n", ps_quote(&log.to_string_lossy())));
    s.push_str(
        "Add-Content -LiteralPath $log -Value \"$(Get-Date -Format o) update helper started\"\n",
    );
    s.push_str(&format!(
        "Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue\n"
    ));
    match kind {
        InstallKind::Msi => s.push_str(&format!(
            "$p = Start-Process -FilePath \"$env:SystemRoot\\System32\\msiexec.exe\" -ArgumentList '/i',{},'/passive','/norestart' -Wait -PassThru\n",
            ps_arg(&inst)
        )),
        _ => s.push_str(&format!("$p = Start-Process -FilePath {} -ArgumentList '/P' -Wait -PassThru\n", ps_quote(&inst))),
    }
    s.push_str("Add-Content -LiteralPath $log -Value \"$(Get-Date -Format o) installer exit $($p.ExitCode)\"\n");
    // Only ever the folder `download_dir` made.
    let dir = inst.rsplit_once(['\\', '/']).map(|(d, _)| d);
    let made_by_us = dir
        .and_then(|d| d.rsplit(['\\', '/']).next())
        .is_some_and(|n| n.starts_with("ezssh-update-"));
    match dir {
        Some(d) if made_by_us => s.push_str(&format!(
            "Remove-Item -LiteralPath {} -Recurse -Force -ErrorAction SilentlyContinue\n",
            ps_quote(d)
        )),
        _ => s.push_str(&format!(
            "Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue\n",
            ps_quote(&inst)
        )),
    }
    let list: Vec<String> = args.iter().map(|a| ps_arg(&a.to_string_lossy())).collect();
    s.push_str(&format!(
        "Start-Process -FilePath {}",
        ps_quote(&exe.to_string_lossy())
    ));
    if !list.is_empty() {
        s.push_str(&format!(" -ArgumentList {}", list.join(",")));
    }
    s.push('\n');
    s
}

#[cfg(windows)]
fn windows_install(install: &Install, file: &Path) -> Result<AfterInstall> {
    use base64::Engine as _;
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let log = std::env::temp_dir().join("ezssh-update.log");
    let script = windows_script(
        install.kind,
        file,
        std::process::id(),
        &install.exe,
        &install.args,
        &log,
    );
    tracing::debug!(%script, "updater: helper script");
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let ps = std::env::var("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .unwrap_or_else(|_| PathBuf::from("powershell.exe"));
    std::process::Command::new(&ps)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
        ])
        .arg(encoded)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| {
            err(format!(
                "starting the update helper ({}): {e}",
                ps.display()
            ))
        })?;
    tracing::info!(log = %log.display(), "updater: helper started; exiting for the installer");
    Ok(AfterInstall::Exit)
}

#[cfg(not(windows))]
fn windows_install(install: &Install, _file: &Path) -> Result<AfterInstall> {
    Err(err(format!(
        "a {} installs on Windows only",
        install.kind.label()
    )))
}
