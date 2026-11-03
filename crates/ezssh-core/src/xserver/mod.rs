//! VcXsrv (GPLv2, free) lifecycle manager.
//!
//! ezSsh has no X server of its own; it delegates to VcXsrv on Windows
//! and manages its lifecycle per-display with reference counting. The
//! first SSH session that opts into X11 forwarding brings VcXsrv up on
//! `:0`; subsequent sessions share the same display; the server exits
//! when the last forwarding session disconnects.
//!
//! v1 runs VcXsrv with `-ac` (access control off) and accepts any cookie
//! on the forwarded channel. xauth MIT-MAGIC-COOKIE-1 negotiation is a
//! follow-up — `-ac` is fine for loopback since VcXsrv only listens on
//! localhost TCP 6000+display.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command};

use tokio::sync::Mutex;

use crate::error::{AppError, Result};

/// Default X11 display number. VcXsrv listens on TCP 127.0.0.1:6000.
pub const DEFAULT_DISPLAY: u8 = 0;

/// Where to look for vcxsrv.exe **as a system install**. Checked after
/// the bundled copy next to the ezSsh binary (see `detect_install_path`).
const VCXSRV_CANDIDATES: &[&str] = &[
    r"C:\Program Files\VcXsrv\vcxsrv.exe",
    r"C:\Program Files (x86)\VcXsrv\vcxsrv.exe",
];

/// Path under the ezSsh binary's directory where a bundled VcXsrv lives:
/// the portable tar.xz has `vcxsrv/` next to `ezssh.exe`, and the NSIS /
/// MSI installers place their `vcxsrv` resource there too
/// (`[package.metadata.packager] resources` in `ezssh-app/Cargo.toml`).
const BUNDLED_VCXSRV_REL: &str = "vcxsrv/vcxsrv.exe";

pub struct XServerManager {
    displays: Mutex<HashMap<u8, DisplayEntry>>,
}

struct DisplayEntry {
    child: Child,
    refs: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct XServerStatus {
    pub installed: bool,
    pub install_path: Option<String>,
    pub running_displays: Vec<u8>,
}

impl Default for XServerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl XServerManager {
    pub fn new() -> Self {
        Self {
            displays: Mutex::new(HashMap::new()),
        }
    }

    /// Return the VcXsrv install path if we can find it. Priority:
    /// 1. Bundled next to the ezSsh binary (`vcxsrv/`, portable tar.xz and
    ///    installers). Lets "install ezSsh" Just Work without a separate
    ///    VcXsrv install.
    /// 2. Per-user install at `<data_dir>/vcxsrv/vcxsrv.exe` (the ezTerm
    ///    data folder, see [`user_install_dir`]) — populated by the in-app
    ///    "Install VcXsrv" flow, by ezSsh or by an earlier ezTerm.
    /// 3. `C:\Program Files\VcXsrv\` and the x86 variant — user's own
    ///    system install (useful for dev builds / `cargo run`).
    pub fn detect_install_path() -> Option<PathBuf> {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent() {
                let bundled = dir.join(BUNDLED_VCXSRV_REL);
                if bundled.exists() {
                    return Some(bundled);
                }
            }
        if let Some(user) = user_install_path()
            && user.exists() {
                return Some(user);
            }
        for candidate in VCXSRV_CANDIDATES {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    pub async fn status(&self) -> XServerStatus {
        let path = Self::detect_install_path();
        let displays = self.displays.lock().await;
        let mut running: Vec<u8> = displays.keys().copied().collect();
        running.sort_unstable();
        XServerStatus {
            installed: path.is_some(),
            install_path: path.map(|p| p.to_string_lossy().into_owned()),
            running_displays: running,
        }
    }

    /// Ref-count + start. Fails if VcXsrv isn't installed.
    pub async fn acquire(&self, display: u8) -> Result<()> {
        let mut displays = self.displays.lock().await;
        if let Some(entry) = displays.get_mut(&display) {
            entry.refs += 1;
            return Ok(());
        }
        let path = Self::detect_install_path().ok_or(AppError::XServerMissing)?;
        let child = Command::new(&path)
            .args([
                // Display number goes first, unprefixed with "--" etc.
                &format!(":{display}"),
                // Access control off — any forwarded X11 client can connect.
                // We're only listening on loopback so this is safe-ish.
                "-ac",
                // Each X window becomes a native Windows window.
                "-multiwindow",
                // Bridge clipboard both ways.
                "-clipboard",
                // Run quietly — no splash / reminder.
                "-silent-dup-error",
            ])
            .spawn()
            .map_err(|e| AppError::Ssh(format!("vcxsrv spawn: {e}")))?;
        displays.insert(display, DisplayEntry { child, refs: 1 });
        Ok(())
    }

    /// Decrement refcount for `display`. When it hits zero the child is
    /// killed. No-op if the display wasn't tracked.
    pub async fn release(&self, display: u8) {
        let mut displays = self.displays.lock().await;
        let should_stop = match displays.get_mut(&display) {
            Some(entry) => {
                entry.refs = entry.refs.saturating_sub(1);
                entry.refs == 0
            }
            None => false,
        };
        if should_stop
            && let Some(mut entry) = displays.remove(&display) {
                // kill + wait block (TerminateProcess may take a moment to
                // land); keep them off the runtime workers and off the
                // `displays` lock.
                let display_number = display;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = entry.child.kill() {
                        tracing::warn!(display_number, "vcxsrv kill failed: {e}");
                    }
                    if let Err(e) = entry.child.wait() {
                        tracing::warn!(display_number, "vcxsrv wait failed: {e}");
                    }
                });
            }
    }
}

/// Per-user VcXsrv install directory, in ezTerm's data folder so a copy an
/// ezTerm install provisioned is found (and removed by the uninstallers'
/// `%APPDATA%\zerosandones\ezterm\data\vcxsrv`). Used by the in-app installer
/// (see `install_vcxsrv`) and picked up by `detect_install_path` when
/// neither the bundled copy nor a system install is present.
pub fn user_install_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "zerosandones", "ezterm")
        .map(|d| d.data_dir().join("vcxsrv"))
}

/// Full path to the expected `vcxsrv.exe` under [`user_install_dir`].
pub fn user_install_path() -> Option<PathBuf> {
    user_install_dir().map(|d| d.join("vcxsrv.exe"))
}

/// Source for the VcXsrv installer. SourceForge's download URL issues
/// an HTTP redirect to a rotating mirror; `reqwest` follows up to 10
/// hops by default. Pinned to 1.20.14.0 (current stable as of 2026-Q2)
/// so the install surface is deterministic. The installer is an NSIS
/// EXE — we drive it with `/S /D=<target>` below.
#[cfg(target_os = "windows")]
const VCXSRV_INSTALLER_URL: &str =
    "https://sourceforge.net/projects/vcxsrv/files/vcxsrv/1.20.14.0/\
     vcxsrv-64.1.20.14.0.installer.exe/download";

/// Download VcXsrv and silent-install it to the per-user data dir, then
/// return the absolute path to the installed `vcxsrv.exe`. Fails if
/// the download can't be fetched, the installer exits non-zero, or the
/// expected binary isn't present after install.
///
/// Runs entirely without admin: NSIS accepts `/D=<path>` to write to
/// a user-writable location. `/D=` **must be the last argument** and
/// the path must be passed un-quoted on the Windows command line — we
/// use `CommandExt::raw_arg` on Windows to preserve that.
#[cfg(target_os = "windows")]
pub async fn install_vcxsrv() -> Result<PathBuf> {
    let target_dir = user_install_dir()
        .ok_or_else(|| AppError::Validation("cannot resolve user data dir".into()))?;
    if let Some(parent) = target_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Download the installer to a temp file. SourceForge redirects to
    // a mirror; reqwest follows up to 10 hops. A 30s total timeout
    // guards against a mirror that hangs halfway.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| AppError::Validation(format!("reqwest build: {e}")))?;
    let resp = client
        .get(VCXSRV_INSTALLER_URL)
        .send()
        .await
        .map_err(|e| AppError::Validation(format!("vcxsrv download: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Validation(format!("vcxsrv download: {e}")))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Validation(format!("vcxsrv download: {e}")))?;

    let installer_path = std::env::temp_dir().join("ezterm-vcxsrv-installer.exe");
    tokio::fs::write(&installer_path, &bytes).await?;

    // Spawn the installer on a blocking thread so we can use
    // `std::os::windows::process::CommandExt::raw_arg` — NSIS parses
    // its own command line and breaks on quoted `/D=...`.
    let installer = installer_path.clone();
    let target = target_dir.clone();
    let status = tokio::task::spawn_blocking(move || {
        use std::os::windows::process::CommandExt as _;
        let target_str = target.to_string_lossy().to_string();
        let mut cmd = std::process::Command::new(&installer);
        cmd.raw_arg("/S");
        cmd.raw_arg(format!("/D={target_str}"));
        cmd.status()
    })
    .await
    .map_err(|e| AppError::Validation(format!("vcxsrv install join: {e}")))??;

    // Installer done — clean up temp regardless of outcome so we don't
    // leave a 3 MB blob behind on retry loops.
    let _ = std::fs::remove_file(&installer_path);

    if !status.success() {
        return Err(AppError::Validation(format!(
            "VcXsrv installer exited with code {:?}",
            status.code()
        )));
    }
    let vcxsrv_exe = target_dir.join("vcxsrv.exe");
    if !vcxsrv_exe.exists() {
        return Err(AppError::Validation(
            "VcXsrv installer finished but vcxsrv.exe was not placed at the expected path".into(),
        ));
    }
    Ok(vcxsrv_exe)
}

/// Non-Windows stub: VcXsrv only exists on Windows, so this is a
/// compile-time no-op that returns a clear error if somehow invoked.
#[cfg(not(target_os = "windows"))]
pub async fn install_vcxsrv() -> Result<PathBuf> {
    Err(AppError::Validation(
        "VcXsrv install is only supported on Windows".into(),
    ))
}

/// Generate an MIT-MAGIC-COOKIE-1 value (32 hex chars = 128 bits).
/// VcXsrv with `-ac` ignores the cookie; we still send a real one so the
/// SSH wire protocol is well-formed and so future cookie-enforcing
/// upgrades don't require a client-side change.
pub fn generate_cookie() -> String {
    use rand::{rngs::SysRng, TryRng};
    let mut buf = [0u8; 16];
    // Fall back to a zero cookie if the OS RNG fails. VcXsrv launched
    // with `-ac` ignores the cookie value, so a zero value is still a
    // well-formed SSH-X11 request on this client.
    let _ = SysRng.try_fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
