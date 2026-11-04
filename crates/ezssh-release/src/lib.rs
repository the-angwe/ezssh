//! The release side of the in-app updater: the update signing key, one
//! signature per installer, and the `latest.json` manifest.
//!
//! The formats are the ones `tauri-plugin-updater` used, so the manifest
//! shape and signature files stay what the v1 releases published:
//!
//! - A public key is the whole minisign `.pub` file (comment line + key
//!   line), base64-encoded into one line. That line is what
//!   `crates/ezssh-core/updater.pub` holds.
//! - A signature is the whole minisign `.sig` file, base64-encoded into one
//!   line, so it drops straight into the manifest's `signature` field.
//! - The trusted comment is `timestamp:<unix>\tfile:<name>\tversion:<v>`.
//!   The trusted comment is covered by the signature, so the app checks
//!   the signed version against the one the manifest announces (an
//!   unsigned manifest cannot pair a new version number with an old
//!   installer).
//! - `latest.json` is `{version, notes, pub_date, platforms}` with one
//!   `{url, signature}` per `{os}-{arch}-{installer}` key plus a bare
//!   `{os}-{arch}` key for each platform's default installer.

use std::collections::BTreeMap;
use std::io::Read;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use minisign::{KeyPair, PublicKey, PublicKeyBox, SecretKey, SecretKeyBox, SignatureBox};

/// The key types, for callers holding keys (tests of the app side).
pub use minisign;

/// A freshly generated key pair as file contents.
pub struct KeyFiles {
    /// The minisign secret key file (keep it secret; it goes into the
    /// `EZSSH_SIGNING_KEY` repository secret).
    pub secret: String,
    /// The minisign public key file.
    pub public: String,
}

/// Generate a signing key pair. `password` encrypts the secret key; `None`
/// leaves it unencrypted (only sensible for tests).
pub fn keygen(password: Option<String>) -> Result<KeyFiles> {
    let kp = match password {
        Some(pw) if pw.is_empty() => {
            bail!("an empty password would prompt; pass None for an unencrypted key")
        }
        Some(pw) => KeyPair::generate_encrypted_keypair(Some(pw))
            .context("generating an encrypted key pair")?,
        None => KeyPair::generate_unencrypted_keypair().context("generating a key pair")?,
    };
    let secret = kp
        .sk
        .to_box(Some("ezSsh update signing key"))
        .context("encoding the secret key")?
        .into_string();
    let public = kp
        .pk
        .to_box()
        .context("encoding the public key")?
        .into_string();
    Ok(KeyFiles { secret, public })
}

/// The one-line form of a public key file, as the app embeds it.
pub fn public_key_line(public_file: &str) -> String {
    B64.encode(public_file.as_bytes())
}

/// Accepts a raw minisign file or its one-line base64 form (the way CI
/// secrets usually hold it) and returns the raw file text.
fn unwrap_b64(text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.starts_with("untrusted comment:") {
        return Ok(format!("{trimmed}\n"));
    }
    let bytes = B64
        .decode(trimmed)
        .context("neither a minisign file nor base64 of one")?;
    String::from_utf8(bytes).context("the base64 does not hold text")
}

/// Load a secret key from its file text (raw or base64). An encrypted key
/// needs its password; a missing one is an error, never a prompt.
pub fn load_secret_key(text: &str, password: Option<String>) -> Result<SecretKey> {
    let raw = unwrap_b64(text)?;
    let boxed = SecretKeyBox::from_string(&raw).context("parsing the secret key")?;
    match password.filter(|p| !p.is_empty()) {
        Some(pw) => boxed
            .into_secret_key(Some(pw))
            .context("decrypting the secret key (wrong password?)"),
        None => boxed
            .into_unencrypted_secret_key()
            .context("the secret key is encrypted: set its password"),
    }
}

/// Load a public key from its file text or its one-line form.
pub fn load_public_key(text: &str) -> Result<PublicKey> {
    let raw = unwrap_b64(text)?;
    PublicKeyBox::from_string(&raw)
        .and_then(|b| b.into_public_key())
        .context("parsing the public key")
}

/// The trusted comment written into every signature.
pub fn trusted_comment(timestamp: u64, file_name: &str, version: &str) -> String {
    format!("timestamp:{timestamp}\tfile:{file_name}\tversion:{version}")
}

/// Sign `data` (an installer named `file_name` of release `version`) and
/// return the one-line base64 signature.
pub fn sign(
    sk: &SecretKey,
    data: impl Read,
    file_name: &str,
    version: &str,
    timestamp: u64,
) -> Result<String> {
    if file_name.contains(['\t', '\n', '\r']) {
        bail!("file name {file_name:?} cannot go into a trusted comment");
    }
    let pk = PublicKey::from_secret_key(sk).context("deriving the public key")?;
    let comment = trusted_comment(timestamp, file_name, version);
    let sig = minisign::sign(
        Some(&pk),
        sk,
        data,
        Some(&comment),
        Some("signature from ezSsh update key"),
    )
    .with_context(|| format!("signing {file_name}"))?;
    Ok(B64.encode(sig.into_string().as_bytes()))
}

/// Check a one-line signature over `data` against a public key; returns
/// the trusted comment.
pub fn verify(pk: &PublicKey, data: impl Read + std::io::Seek, signature: &str) -> Result<String> {
    let raw = unwrap_b64(signature)?;
    let sig = SignatureBox::from_string(&raw).context("parsing the signature")?;
    minisign::verify(pk, &sig, data, true, false, false).context("signature does not verify")?;
    sig.trusted_comment().context("reading the trusted comment")
}

// ------------------------------------------------------------- manifest

/// Which manifest keys an artifact serves, from its file name. Portable
/// archives, checksums and signatures serve none.
///
/// The names are cargo-packager's (`ezssh_<v>_<arch>-setup.exe`,
/// `ezssh_<v>_<arch>_<lang>.msi`, `ezssh_<v>_<arch>.AppImage`,
/// `ezssh_<v>_<arch>.deb`) plus the macOS updater tarball the release job
/// makes (`ezssh_<v>_<arch>.app.tar.gz`).
pub fn targets_for(file_name: &str) -> Vec<String> {
    let (stem, os, installer, default) = if let Some(s) = file_name.strip_suffix("-setup.exe") {
        (s.to_string(), "windows", "nsis", true)
    } else if let Some(s) = file_name.strip_suffix(".msi") {
        // `_<arch>_<lang>`: drop the language.
        match s.rsplit_once('_') {
            Some((head, _lang)) => (head.to_string(), "windows", "msi", false),
            None => return Vec::new(),
        }
    } else if let Some(s) = file_name.strip_suffix(".AppImage") {
        (s.to_string(), "linux", "appimage", true)
    } else if let Some(s) = file_name.strip_suffix(".deb") {
        (s.to_string(), "linux", "deb", false)
    } else if let Some(s) = file_name.strip_suffix(".app.tar.gz") {
        (s.to_string(), "darwin", "app", true)
    } else {
        return Vec::new();
    };
    // The arch token is the last `_` field, but `x86_64` has an `_` itself.
    const ARCHES: [(&str, &str); 9] = [
        ("x86_64", "x86_64"),
        ("amd64", "x86_64"),
        ("x64", "x86_64"),
        ("i686", "i686"),
        ("i386", "i686"),
        ("aarch64", "aarch64"),
        ("arm64", "aarch64"),
        ("armv7", "armv7"),
        ("armhf", "armv7"),
    ];
    let Some(arch) = ARCHES
        .iter()
        .find(|(token, _)| {
            stem.strip_suffix(token)
                .is_some_and(|head| head.ends_with('_'))
        })
        .map(|(_, arch)| *arch)
    else {
        return Vec::new();
    };
    let mut keys = vec![format!("{os}-{arch}-{installer}")];
    if default {
        keys.push(format!("{os}-{arch}"));
    }
    keys
}

/// One signed artifact for the manifest.
pub struct Signed {
    pub file_name: String,
    pub signature: String,
}

/// Build `latest.json`. Fails when two artifacts claim the same key (two
/// AppImages for one arch would make the choice arbitrary).
pub fn manifest(
    version: &str,
    notes: &str,
    pub_date: &str,
    base_url: &str,
    assets: &[Signed],
) -> Result<serde_json::Value> {
    let mut platforms: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut owner: BTreeMap<String, &str> = BTreeMap::new();
    let base = base_url.trim_end_matches('/');
    for a in assets {
        for key in targets_for(&a.file_name) {
            if let Some(prev) = owner.insert(key.clone(), &a.file_name) {
                bail!("{key} is claimed by both {prev} and {}", a.file_name);
            }
            platforms.insert(
                key,
                serde_json::json!({
                    "url": format!("{base}/{}", a.file_name),
                    "signature": a.signature,
                }),
            );
        }
    }
    if platforms.is_empty() {
        bail!("no installer among the signed files");
    }
    Ok(serde_json::json!({
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": platforms,
    }))
}

/// RFC 3339 UTC time (seconds precision) for a unix timestamp.
pub fn rfc3339(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    )
}

/// The version a trusted comment was signed for.
pub fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment
        .split('\t')
        .find_map(|f| f.strip_prefix("version:"))
}

/// Check that a release version is plain `MAJOR.MINOR.PATCH[-pre]` and
/// return it without a leading `v`.
pub fn normalise_version(v: &str) -> Result<String> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(anyhow!("{v:?} is not a MAJOR.MINOR.PATCH version"));
    }
    Ok(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn targets_from_cargo_packager_names() {
        assert_eq!(
            targets_for("ezssh_1.15.0_x64-setup.exe"),
            ["windows-x86_64-nsis", "windows-x86_64"]
        );
        assert_eq!(
            targets_for("ezssh_1.15.0_x64_en-US.msi"),
            ["windows-x86_64-msi"]
        );
        assert_eq!(
            targets_for("ezssh_1.15.0_x86_64.AppImage"),
            ["linux-x86_64-appimage", "linux-x86_64"]
        );
        assert_eq!(
            targets_for("ezssh_1.15.0_aarch64.AppImage"),
            ["linux-aarch64-appimage", "linux-aarch64"]
        );
        assert_eq!(targets_for("ezssh_1.15.0_amd64.deb"), ["linux-x86_64-deb"]);
        assert_eq!(targets_for("ezssh_1.15.0_arm64.deb"), ["linux-aarch64-deb"]);
        assert_eq!(
            targets_for("ezssh_1.15.0_aarch64.app.tar.gz"),
            ["darwin-aarch64-app", "darwin-aarch64"]
        );
        assert!(targets_for("ezSsh_1.15.0_linux-amd64-portable.tar.xz").is_empty());
        assert!(targets_for("ezssh_1.15.0_x64-setup.exe.sig").is_empty());
        assert!(targets_for("SHA256SUMS").is_empty());
        assert!(targets_for("ezssh_1.15.0_sparc.deb").is_empty());
    }

    #[test]
    fn manifest_has_default_and_installer_keys_and_refuses_clashes() {
        let assets = [
            Signed {
                file_name: "ezssh_1.15.0_x64-setup.exe".into(),
                signature: "S1".into(),
            },
            Signed {
                file_name: "ezssh_1.15.0_x64_en-US.msi".into(),
                signature: "S2".into(),
            },
            Signed {
                file_name: "ezssh_1.15.0_amd64.deb".into(),
                signature: "S3".into(),
            },
        ];
        let m = manifest(
            "1.15.0",
            "notes",
            "2026-11-02T10:00:00Z",
            "https://x/dl/",
            &assets,
        )
        .unwrap();
        assert_eq!(m["version"], "1.15.0");
        assert_eq!(
            m["platforms"]["windows-x86_64"]["url"],
            "https://x/dl/ezssh_1.15.0_x64-setup.exe"
        );
        assert_eq!(m["platforms"]["windows-x86_64-msi"]["signature"], "S2");
        assert_eq!(m["platforms"]["linux-x86_64-deb"]["signature"], "S3");
        assert!(m["platforms"].get("linux-x86_64").is_none());

        let clash = [
            Signed {
                file_name: "a_1_x86_64.AppImage".into(),
                signature: "A".into(),
            },
            Signed {
                file_name: "b_1_amd64.AppImage".into(),
                signature: "B".into(),
            },
        ];
        assert!(manifest("1", "", "", "u", &clash).is_err());
        assert!(manifest("1", "", "", "u", &[]).is_err());
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let keys = keygen(None).unwrap();
        let sk = load_secret_key(&keys.secret, None).unwrap();
        // The base64 forms load too.
        let sk2 = load_secret_key(&B64.encode(keys.secret.as_bytes()), None).unwrap();
        let pk = load_public_key(&public_key_line(&keys.public)).unwrap();
        let data = b"installer bytes".to_vec();
        let sig = sign(
            &sk,
            Cursor::new(&data),
            "ezssh_1.2.3_amd64.deb",
            "1.2.3",
            1_700_000_000,
        )
        .unwrap();
        assert!(!sig.contains('\n'));
        let comment = verify(&pk, Cursor::new(&data), &sig).unwrap();
        assert_eq!(
            comment,
            "timestamp:1700000000\tfile:ezssh_1.2.3_amd64.deb\tversion:1.2.3"
        );
        assert_eq!(signed_version(&comment), Some("1.2.3"));
        // Tampered data or another key fail.
        assert!(verify(&pk, Cursor::new(b"other".to_vec()), &sig).is_err());
        let other = load_public_key(&keygen(None).unwrap().public).unwrap();
        assert!(verify(&other, Cursor::new(&data), &sig).is_err());
        let sig2 = sign(&sk2, Cursor::new(&data), "f", "1.2.3", 1).unwrap();
        verify(&pk, Cursor::new(&data), &sig2).unwrap();
    }

    #[test]
    fn signatures_verify_with_minisign_verify_like_the_app() {
        use minisign_verify::{PublicKey as VPk, Signature as VSig};
        let keys = keygen(None).unwrap();
        let sk = load_secret_key(&keys.secret, None).unwrap();
        let sig = sign(&sk, Cursor::new(b"abc".to_vec()), "f", "9.9.9", 5).unwrap();
        let pk_text =
            String::from_utf8(B64.decode(public_key_line(&keys.public)).unwrap()).unwrap();
        let sig_text = String::from_utf8(B64.decode(sig).unwrap()).unwrap();
        let pk = VPk::decode(&pk_text).unwrap();
        let s = VSig::decode(&sig_text).unwrap();
        pk.verify(b"abc", &s, false).unwrap();
        assert_eq!(s.trusted_comment(), "timestamp:5\tfile:f\tversion:9.9.9");
    }

    #[test]
    fn encrypted_keys_need_their_password() {
        let keys = keygen(Some("pw".into())).unwrap();
        assert!(load_secret_key(&keys.secret, None).is_err());
        assert!(load_secret_key(&keys.secret, Some("wrong".into())).is_err());
        load_secret_key(&keys.secret, Some("pw".into())).unwrap();
        assert!(keygen(Some(String::new())).is_err());
    }

    #[test]
    fn rfc3339_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_793_181_296), "2026-10-28T09:54:56Z");
    }

    #[test]
    fn versions() {
        assert_eq!(normalise_version("v1.15.0").unwrap(), "1.15.0");
        assert_eq!(normalise_version("2.0.0-beta.1").unwrap(), "2.0.0-beta.1");
        assert!(normalise_version("1.15").is_err());
        assert!(normalise_version("ezssh-1.0.0").is_err());
    }
}
