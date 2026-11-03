//! In-app updater: find a newer release, download its installer, check the
//! signature, install it, start the new version.
//!
//! It reads the `latest.json` the release workflow publishes, in the shape
//! `tauri-plugin-updater` used (`{version, notes, pub_date, platforms}`,
//! one `{url, signature}` per `{os}-{arch}-{installer}` key plus a bare
//! `{os}-{arch}` default). Signatures are minisign, base64-wrapped, made
//! by `ezssh-release sign`; the public key is `updater.pub` at the crate
//! root, compiled in. Unlike tauri-plugin-updater, the signed version is
//! required: a signature whose trusted comment does not name the
//! announced version is refused, so an unsigned manifest cannot offer an
//! older installer under a new version number.
//!
//! Which installer applies is decided at runtime from where the binary
//! sits ([`detect`]): the NSIS installer leaves `uninstall.exe` next to
//! it, an AppImage runs with `$APPIMAGE` set, the deb owns
//! `/usr/bin/ezssh`, a macOS bundle lives in `*.app/Contents/MacOS`, and
//! the portable archives carry a `portable.txt` marker. Portable and
//! unpackaged builds are told about updates but not updated in place.

pub mod install;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
pub use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, Result};

/// The GitHub repository whose releases carry `latest.json`. A build can
/// point elsewhere with `EZSSH_UPDATE_REPO=owner/name` at compile time.
pub const DEFAULT_REPO: &str = "the-angwe/ezssh";

/// The update public key (one line, see `ezssh-release keygen`). Empty
/// until a key has been generated; such a build cannot check for updates.
const PUBKEY: &str = include_str!("../../updater.pub");

/// File next to the binary that marks a portable archive.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// Largest installer the updater downloads.
const MAX_DOWNLOAD: u64 = 1 << 30;
/// A download that receives nothing for this long fails.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

fn err(msg: impl Into<String>) -> AppError {
    AppError::Updater(msg.into())
}

// ------------------------------------------------------------ install kind

/// How this copy of the app was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    Nsis,
    Msi,
    AppImage,
    Deb,
    /// macOS `.app` bundle.
    App,
    /// Unpacked from a portable archive.
    Portable,
    /// A `cargo run` / `cargo build` binary or anything not recognised.
    Unpackaged,
}

impl InstallKind {
    /// The installer token in manifest keys (`{os}-{arch}-{installer}`).
    pub fn installer(self) -> Option<&'static str> {
        match self {
            Self::Nsis => Some("nsis"),
            Self::Msi => Some("msi"),
            Self::AppImage => Some("appimage"),
            Self::Deb => Some("deb"),
            Self::App => Some("app"),
            Self::Portable | Self::Unpackaged => None,
        }
    }

    /// What the dialog calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Nsis => "Windows installer",
            Self::Msi => "MSI package",
            Self::AppImage => "AppImage",
            Self::Deb => "Debian package",
            Self::App => "macOS app",
            Self::Portable => "portable archive",
            Self::Unpackaged => "development build",
        }
    }

    /// Whether the updater can replace this copy in place.
    pub fn self_updates(self) -> bool {
        self.installer().is_some()
    }
}

/// This running copy: its kind, what an install replaces and how to start
/// it again. Detect it once at start-up: after a deb upgrade the running
/// executable's path reads as deleted.
#[derive(Clone, Debug)]
pub struct Install {
    pub kind: InstallKind,
    /// The file or bundle an install replaces: the AppImage file, the
    /// `.app` directory; the executable otherwise.
    pub target: PathBuf,
    /// The executable to start after the update (the AppImage file for
    /// AppImages).
    pub exe: PathBuf,
    /// Arguments to start it with (the current ones, minus argv[0]).
    pub args: Vec<OsString>,
}

/// Detect how the running binary was installed.
pub fn detect() -> Install {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("updater: current_exe failed: {e}");
            PathBuf::from("ezssh")
        }
    };
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let args = std::env::args_os().skip(1).collect();
    let install = detect_from(&exe, appimage, args);
    tracing::info!(kind = ?install.kind, target = %install.target.display(), "updater: install detected");
    install
}

/// [`detect`] on given inputs (testable).
pub fn detect_from(exe: &Path, appimage: Option<PathBuf>, args: Vec<OsString>) -> Install {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let simple = |kind| Install { kind, target: exe.to_path_buf(), exe: exe.to_path_buf(), args: args.clone() };
    if dir.join(PORTABLE_MARKER).is_file() {
        return simple(InstallKind::Portable);
    }
    if cfg!(target_os = "linux") {
        if let Some(img) = appimage.filter(|p| p.is_file()) {
            return Install { kind: InstallKind::AppImage, target: img.clone(), exe: img, args };
        }
        if exe.starts_with("/usr") && Path::new("/var/lib/dpkg/info/ezssh.list").is_file() {
            return simple(InstallKind::Deb);
        }
        return simple(InstallKind::Unpackaged);
    }
    if cfg!(target_os = "macos") {
        // .../ezSsh.app/Contents/MacOS/ezssh
        let bundle = dir
            .parent()
            .and_then(Path::parent)
            .filter(|b| dir.ends_with("Contents/MacOS") && b.extension().is_some_and(|e| e == "app"));
        return match bundle {
            Some(b) => Install { kind: InstallKind::App, target: b.to_path_buf(), exe: exe.to_path_buf(), args },
            None => simple(InstallKind::Unpackaged),
        };
    }
    if cfg!(windows) {
        if dir.join("uninstall.exe").is_file() {
            return simple(InstallKind::Nsis);
        }
        // Cargo output directories are not installs.
        let in_target = dir.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target"));
        return simple(if in_target { InstallKind::Unpackaged } else { InstallKind::Msi });
    }
    simple(InstallKind::Unpackaged)
}

// ------------------------------------------------------------ config

/// Everything a check needs.
#[derive(Clone, Debug)]
pub struct Config {
    /// `owner/name` on GitHub.
    pub repo: String,
    /// One-line public key; empty means updates are not configured.
    pub pubkey: String,
    pub current: Version,
    pub install: Install,
    /// Read this manifest instead of the GitHub ones (manual tests; the
    /// signature is still checked against the built-in key).
    pub manifest_override: Option<String>,
    /// `{os}-{arch}` of this build.
    pub os: &'static str,
    pub arch: &'static str,
}

impl Config {
    /// The configuration compiled into this build, for `install`.
    pub fn new(install: Install, manifest_override: Option<String>) -> Self {
        let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver");
        Self {
            repo: option_env!("EZSSH_UPDATE_REPO").unwrap_or(DEFAULT_REPO).to_string(),
            pubkey: PUBKEY.trim().to_string(),
            current,
            install,
            manifest_override,
            os: updater_os(),
            arch: updater_arch(),
        }
    }

    /// The keys to look up in a manifest, best first. Portable and
    /// unpackaged builds look for no key: the bare `{os}-{arch}` entry is
    /// another installer kind they cannot run.
    pub fn targets(&self) -> Vec<String> {
        match self.install.kind.installer() {
            Some(i) => {
                let mut v = vec![format!("{}-{}-{i}", self.os, self.arch)];
                // The bare key is the platform's default installer; only
                // that kind may fall back to it.
                let default = matches!(
                    (self.os, self.install.kind),
                    ("windows", InstallKind::Nsis) | ("linux", InstallKind::AppImage) | ("darwin", InstallKind::App)
                );
                if default {
                    v.push(format!("{}-{}", self.os, self.arch));
                }
                v
            }
            None => Vec::new(),
        }
    }

    pub fn release_page(&self, version: &str) -> String {
        format!("https://github.com/{}/releases/tag/v{version}", self.repo)
    }
}

fn updater_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

fn updater_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "i686"
    } else if cfg!(target_arch = "arm") {
        "armv7"
    } else {
        "unknown"
    }
}

// ------------------------------------------------------------ check

/// A release newer than the running one.
#[derive(Clone, Debug, Serialize)]
pub struct Update {
    pub current_version: String,
    pub version: String,
    /// RFC 3339, as published.
    pub date: Option<String>,
    pub notes: Option<String>,
    pub pre_release: bool,
    /// The installer for this copy, when the manifest has one and this
    /// copy can update itself.
    pub url: Option<String>,
    pub signature: Option<String>,
    /// The manifest key that matched.
    pub target: Option<String>,
    /// Where to download it by hand.
    pub release_page: String,
}

impl Update {
    pub fn installable(&self) -> bool {
        self.url.is_some()
    }
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(alias = "name")]
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    platforms: Option<std::collections::HashMap<String, Platform>>,
    // The dynamic (single platform) shape.
    url: Option<String>,
    signature: Option<String>,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
    signature: String,
}

fn client(cfg: &Config) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("ezSsh-updater/{}", cfg.current))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| err(format!("http client: {e}")))
}

/// Look for a newer release on the stable or the pre-release channel.
pub async fn check(cfg: &Config, pre_release: bool) -> Result<Option<Update>> {
    if cfg.pubkey.is_empty() {
        return Err(err("this build has no update key, so it cannot verify updates; download new versions from the releases page"));
    }
    let http = client(cfg)?;
    let url = match &cfg.manifest_override {
        Some(u) => u.clone(),
        None if pre_release => prerelease_manifest_url(&http, &cfg.repo).await?,
        None => format!("https://github.com/{}/releases/latest/download/latest.json", cfg.repo),
    };
    tracing::info!(%url, pre_release, "updater: checking");
    let resp = http.get(&url).timeout(Duration::from_secs(30)).send().await.map_err(|e| err(format!("{url}: {e}")))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(err(format!("no release with an update manifest is published yet ({url})")));
    }
    let resp = resp.error_for_status().map_err(|e| err(format!("{url}: {e}")))?;
    let body = resp.bytes().await.map_err(|e| err(format!("{url}: {e}")))?;
    evaluate(cfg, &body, pre_release)
}

/// Decide what a manifest offers this copy.
pub fn evaluate(cfg: &Config, manifest: &[u8], pre_release: bool) -> Result<Option<Update>> {
    let m: Manifest = serde_json::from_slice(manifest).map_err(|e| err(format!("bad latest.json: {e}")))?;
    let version = Version::parse(m.version.trim().trim_start_matches('v'))
        .map_err(|e| err(format!("bad version {:?} in latest.json: {e}", m.version)))?;
    tracing::debug!(%version, current = %cfg.current, "updater: manifest read");
    if version <= cfg.current {
        return Ok(None);
    }
    let mut found = None;
    let targets = cfg.targets();
    match (&m.platforms, &m.url, &m.signature) {
        (Some(p), _, _) => {
            for t in &targets {
                if let Some(e) = p.get(t) {
                    found = Some((t.clone(), e.url.clone(), e.signature.clone()));
                    break;
                }
            }
            if found.is_none() && cfg.install.kind.self_updates() {
                tracing::warn!(?targets, "updater: the release has no installer for this copy");
            }
        }
        (None, Some(u), Some(s)) if !targets.is_empty() => found = Some((String::new(), u.clone(), s.clone())),
        (None, _, _) => {}
    }
    let v = version.to_string();
    let (target, url, signature) = match found {
        Some((t, u, s)) => (Some(t).filter(|t| !t.is_empty()), Some(u), Some(s)),
        None => (None, None, None),
    };
    Ok(Some(Update {
        current_version: cfg.current.to_string(),
        release_page: cfg.release_page(&v),
        version: v,
        date: m.pub_date,
        notes: m.notes.filter(|n| !n.trim().is_empty()),
        pre_release,
        url,
        signature,
        target,
    }))
}

/// The pre-release channel: the newest published (non-draft) release of
/// either kind that carries a `latest.json`.
async fn prerelease_manifest_url(http: &reqwest::Client, repo: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    let api = format!("https://api.github.com/repos/{repo}/releases?per_page=20");
    let body = http
        .get(&api)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| err(format!("github releases: {e}")))?
        .bytes()
        .await
        .map_err(|e| err(format!("github releases: {e}")))?;
    let releases: Vec<Release> = serde_json::from_slice(&body).map_err(|e| err(format!("github releases: {e}")))?;
    releases
        .into_iter()
        .filter(|r| !r.draft)
        .find_map(|r| {
            let a = r.assets.into_iter().find(|a| a.name == "latest.json")?;
            tracing::debug!(tag = %r.tag_name, "updater: pre-release channel picks");
            Some(a.browser_download_url)
        })
        .ok_or_else(|| err("no published release carries an update manifest yet"))
}

// ------------------------------------------------------------ download

/// Download `update`'s installer into `dir`, checking its signature while
/// it arrives. `progress(downloaded, total)` is called per chunk. Returns
/// the verified file; a file that fails is deleted.
pub async fn download(
    cfg: &Config,
    update: &Update,
    dir: &Path,
    mut progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<PathBuf> {
    let (Some(url), Some(signature)) = (&update.url, &update.signature) else {
        return Err(err("this release has no installer for this copy"));
    };
    let pk = public_key(&cfg.pubkey)?;
    let sig = decode_signature(signature)?;
    // Refuse before downloading: a signature for another version, by
    // another key, or in the legacy (not prehashed) mode.
    check_signed_version(sig.trusted_comment(), &update.version)?;
    let mut verifier = pk.verify_stream(&sig).map_err(|e| match e {
        minisign_verify::Error::UnexpectedKeyId => err("the installer was signed with another key than this build trusts"),
        e => err(format!("signature: {e}")),
    })?;

    let name = file_name_from_url(url);
    tokio::fs::create_dir_all(dir).await.map_err(|e| err(format!("creating {}: {e}", dir.display())))?;
    let path = dir.join(&name);
    let http = client(cfg)?;
    tracing::info!(%url, path = %path.display(), "updater: downloading");
    let mut resp = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| err(format!("download: {e}")))?;
    let total = resp.content_length();
    if total.is_some_and(|t| t > MAX_DOWNLOAD) {
        return Err(err(format!("installer too large ({} bytes)", total.unwrap_or(0))));
    }
    let mut file = tokio::fs::File::create(&path).await.map_err(|e| err(format!("creating {}: {e}", path.display())))?;
    let result = async {
        let mut done = 0u64;
        progress(0, total);
        loop {
            let chunk = tokio::time::timeout(IDLE_TIMEOUT, resp.chunk())
                .await
                .map_err(|_| err("download stalled"))?
                .map_err(|e| err(format!("download: {e}")))?;
            let Some(chunk) = chunk else { break };
            done += chunk.len() as u64;
            if done > MAX_DOWNLOAD {
                return Err(err("installer too large"));
            }
            verifier.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| err(format!("writing {}: {e}", path.display())))?;
            progress(done, total);
        }
        if let Some(t) = total
            && t != done
        {
            return Err(err(format!("download ended after {done} of {t} bytes")));
        }
        // Without a Content-Length the loop above never reports a
        // "finished" pair; the UI's throttle keys on `total == done`.
        if total.is_none() {
            progress(done, Some(done));
        }
        file.flush().await.map_err(|e| err(format!("writing {}: {e}", path.display())))?;
        file.sync_all().await.map_err(|e| err(format!("writing {}: {e}", path.display())))?;
        verifier.finalize().map_err(|e| {
            tracing::warn!(%url, "updater: signature check failed: {e}");
            err("the installer's signature does not verify; not installing")
        })?;
        tracing::info!(bytes = done, "updater: download verified");
        Ok(())
    }
    .await;
    drop(file);
    if let Err(e) = result {
        if let Err(rm) = tokio::fs::remove_file(&path).await {
            tracing::warn!(path = %path.display(), "updater: removing a failed download: {rm}");
        }
        return Err(e);
    }
    Ok(path)
}

fn public_key(line: &str) -> Result<minisign_verify::PublicKey> {
    let text = B64.decode(line.trim()).map_err(|e| err(format!("update key: {e}")))?;
    let text = String::from_utf8(text).map_err(|_| err("update key is not text"))?;
    minisign_verify::PublicKey::decode(&text).map_err(|e| err(format!("update key: {e}")))
}

fn decode_signature(line: &str) -> Result<minisign_verify::Signature> {
    let text = B64.decode(line.trim()).map_err(|e| err(format!("signature: {e}")))?;
    let text = String::from_utf8(text).map_err(|_| err("signature is not text"))?;
    minisign_verify::Signature::decode(&text).map_err(|e| err(format!("signature: {e}")))
}

/// The trusted comment must name the announced version. Read before the
/// download for an early refusal; it is only trusted once `finalize` has
/// verified the global signature, which covers the comment.
fn check_signed_version(trusted_comment: &str, announced: &str) -> Result<()> {
    let signed = trusted_comment
        .split('\t')
        .find_map(|f| f.strip_prefix("version:"))
        .ok_or_else(|| err("the signature does not name a version; not installing"))?;
    let same = match (Version::parse(signed.trim_start_matches('v')), Version::parse(announced)) {
        (Ok(a), Ok(b)) => a == b,
        _ => signed == announced,
    };
    if same {
        Ok(())
    } else {
        Err(err(format!("the installer was signed for {signed}, the release says {announced}; not installing")))
    }
}

/// A safe local file name from the URL's last segment.
fn file_name_from_url(url: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    let clean: String = last
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let clean = clean.trim_start_matches('.').to_string();
    if clean.is_empty() { "ezssh-update".into() } else { clean }
}

/// A fresh folder for one download under the system temp directory.
pub fn download_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("ezssh-update-{}-{stamp}", std::process::id()))
}

#[cfg(test)]
mod tests;
