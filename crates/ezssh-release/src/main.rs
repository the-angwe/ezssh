//! `ezssh-release`: update signing key, artifact signatures and
//! `latest.json` for the release workflow. See the crate docs (lib.rs) for
//! the formats and `docs/UPDATER_SIGNING.md` for the one-time key setup.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use ezssh_release as rel;

const HELP: &str = "\
ezssh-release — sign ezSsh installers and write latest.json

USAGE:
    ezssh-release keygen --out DIR [--no-password]
        Write ezssh-update.key (secret) and ezssh-update.pub into DIR and
        print the line for crates/ezssh-core/updater.pub. The password is
        read from EZSSH_SIGNING_KEY_PASSWORD (required unless --no-password).

    ezssh-release sign --version V FILE...
        Write FILE.sig (one base64 line) for each FILE. The secret key comes
        from EZSSH_SIGNING_KEY (the key file's text, raw or base64), its
        password from EZSSH_SIGNING_KEY_PASSWORD.

    ezssh-release verify --pubkey FILE --version V FILE...
        Check FILE.sig of each FILE against a public key file (raw or the
        one-line form) and that each was signed for version V.

    ezssh-release manifest --dir DIR --version V --base-url URL
                           [--notes FILE] [--pub-date RFC3339] [--out FILE]
        Build latest.json from the installers in DIR that have a .sig.
        Writes DIR/latest.json unless --out is given.
";

fn main() {
    if let Err(e) = run() {
        eprintln!("ezssh-release: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    match cmd.as_str() {
        "keygen" => keygen(&rest),
        "sign" => sign(&rest),
        "verify" => verify(&rest),
        "manifest" => manifest(&rest),
        "" | "-h" | "--help" | "help" => {
            print!("{HELP}");
            Ok(())
        }
        other => bail!("unknown command {other:?}; see --help"),
    }
}

/// `--flag value` pairs and the remaining positional arguments.
struct Parsed {
    opts: Vec<(String, String)>,
    flags: Vec<String>,
    files: Vec<PathBuf>,
}

fn parse(args: &[String], with_value: &[&str], bool_flags: &[&str]) -> Result<Parsed> {
    let mut p = Parsed {
        opts: Vec::new(),
        flags: Vec::new(),
        files: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if with_value.contains(&a.as_str()) {
            let v = it.next().ok_or_else(|| anyhow!("{a} needs a value"))?;
            p.opts.push((a.clone(), v.clone()));
        } else if bool_flags.contains(&a.as_str()) {
            p.flags.push(a.clone());
        } else if a.starts_with("--") {
            bail!("unknown option {a}");
        } else {
            p.files.push(PathBuf::from(a));
        }
    }
    Ok(p)
}

impl Parsed {
    fn get(&self, k: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }
    fn need(&self, k: &str) -> Result<&str> {
        self.get(k).ok_or_else(|| anyhow!("{k} is required"))
    }
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{} has no UTF-8 file name", path.display()))
}

fn sig_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".sig");
    PathBuf::from(s)
}

fn keygen(args: &[String]) -> Result<()> {
    let p = parse(args, &["--out"], &["--no-password"])?;
    let out = PathBuf::from(p.need("--out")?);
    let password = if p.flags.iter().any(|f| f == "--no-password") {
        None
    } else {
        Some(env_opt("EZSSH_SIGNING_KEY_PASSWORD").ok_or_else(|| {
            anyhow!("set EZSSH_SIGNING_KEY_PASSWORD (or pass --no-password for an unencrypted key)")
        })?)
    };
    let keys = rel::keygen(password)?;
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let sk = out.join("ezssh-update.key");
    let pk = out.join("ezssh-update.pub");
    if sk.exists() {
        bail!("{} exists; not overwriting a signing key", sk.display());
    }
    write_private(&sk, &keys.secret)?;
    std::fs::write(&pk, &keys.public).with_context(|| format!("writing {}", pk.display()))?;
    eprintln!("wrote {} and {}", sk.display(), pk.display());
    eprintln!("put this line into crates/ezssh-core/updater.pub:");
    println!("{}", rel::public_key_line(&keys.public));
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sign(args: &[String]) -> Result<()> {
    let p = parse(args, &["--version"], &[])?;
    let version = rel::normalise_version(p.need("--version")?)?;
    if p.files.is_empty() {
        bail!("no files to sign");
    }
    let key =
        env_opt("EZSSH_SIGNING_KEY").ok_or_else(|| anyhow!("EZSSH_SIGNING_KEY is not set"))?;
    let sk = rel::load_secret_key(&key, env_opt("EZSSH_SIGNING_KEY_PASSWORD"))?;
    let ts = now();
    for f in &p.files {
        let name = file_name(f)?;
        let data = std::fs::File::open(f).with_context(|| format!("opening {}", f.display()))?;
        let sig = rel::sign(&sk, std::io::BufReader::new(data), &name, &version, ts)?;
        let out = sig_path(f);
        std::fs::write(&out, &sig).with_context(|| format!("writing {}", out.display()))?;
        eprintln!("signed {name}");
    }
    Ok(())
}

fn verify(args: &[String]) -> Result<()> {
    let p = parse(args, &["--pubkey", "--version"], &[])?;
    let version = rel::normalise_version(p.need("--version")?)?;
    let pk_path = p.need("--pubkey")?;
    let pk_text = std::fs::read_to_string(pk_path).with_context(|| format!("reading {pk_path}"))?;
    if pk_text.trim().is_empty() {
        bail!("{pk_path} is empty: no update key configured");
    }
    let pk = rel::load_public_key(&pk_text)?;
    if p.files.is_empty() {
        bail!("no files to verify");
    }
    for f in &p.files {
        let sp = sig_path(f);
        let sig =
            std::fs::read_to_string(&sp).with_context(|| format!("reading {}", sp.display()))?;
        let data = std::fs::File::open(f).with_context(|| format!("opening {}", f.display()))?;
        let comment = rel::verify(&pk, std::io::BufReader::new(data), &sig)
            .with_context(|| format!("{}", f.display()))?;
        match rel::signed_version(&comment) {
            Some(v) if v == version => eprintln!("ok {}", f.display()),
            other => bail!(
                "{} was signed for version {other:?}, not {version}",
                f.display()
            ),
        }
    }
    Ok(())
}

fn manifest(args: &[String]) -> Result<()> {
    let p = parse(
        args,
        &[
            "--dir",
            "--version",
            "--base-url",
            "--notes",
            "--pub-date",
            "--out",
        ],
        &[],
    )?;
    let dir = PathBuf::from(p.need("--dir")?);
    let version = rel::normalise_version(p.need("--version")?)?;
    let base = p.need("--base-url")?;
    let notes = match p.get("--notes") {
        Some(f) if Path::new(f).exists() => {
            let text = std::fs::read_to_string(f).with_context(|| format!("reading {f}"))?;
            // The webview capped notes at 4000 bytes; keep that, on a char boundary.
            let mut end = text.len().min(4000);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text[..end].to_string()
        }
        Some(f) => {
            eprintln!("no notes file {f}; notes left empty");
            String::new()
        }
        None => String::new(),
    };
    let pub_date = p
        .get("--pub-date")
        .map(str::to_string)
        .unwrap_or_else(|| rel::rfc3339(now()));

    let mut assets = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .filter_map(|e| e.ok().and_then(|e| e.file_name().into_string().ok()))
        .collect();
    names.sort();
    for name in names {
        if rel::targets_for(&name).is_empty() {
            continue;
        }
        let sp = dir.join(format!("{name}.sig"));
        match std::fs::read_to_string(&sp) {
            Ok(sig) => assets.push(rel::Signed {
                file_name: name,
                signature: sig.trim().to_string(),
            }),
            Err(e) => eprintln!("skip {name}: no signature ({e})"),
        }
    }
    let m = rel::manifest(&version, &notes, &pub_date, base, &assets)?;
    let out = p
        .get("--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("latest.json"));
    let text = serde_json::to_string_pretty(&m).context("encoding latest.json")?;
    std::fs::write(&out, format!("{text}\n"))
        .with_context(|| format!("writing {}", out.display()))?;
    eprintln!(
        "wrote {} ({} platform keys)",
        out.display(),
        m["platforms"].as_object().map_or(0, |o| o.len())
    );
    Ok(())
}
