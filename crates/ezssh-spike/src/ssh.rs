//! One SSH shell session (russh), feeding the terminal from the channel
//! reader task. Spike-only: host keys are checked against
//! `~/.ssh/known_hosts` plus a spike-local file, credentials come from the
//! agent, key files or a password typed at startup. The real connect path
//! (vault, TOFU in SQLite) arrives with the core extraction in phase 2.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use eframe::egui;
use ezssh_term::{Feeder, Palette, Pty, TermOptions, Terminal, WindowSize};
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

#[derive(Clone, Debug)]
pub struct Target {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub key: Option<PathBuf>,
    pub trust_new_host: bool,
}

impl Target {
    /// Parse `user@host[:port]`.
    pub fn parse(s: &str) -> Result<Target> {
        let (user, rest) = s.split_once('@').ok_or_else(|| anyhow!("expected user@host[:port], got {s:?}"))?;
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().with_context(|| format!("bad port {p:?}"))?),
            _ => (rest, 22),
        };
        if user.is_empty() || host.is_empty() {
            bail!("expected user@host[:port], got {s:?}");
        }
        Ok(Target { user: user.into(), host: host.into(), port, key: None, trust_new_host: false })
    }

    pub fn label(&self) -> String {
        if self.port == 22 { format!("{}@{}", self.user, self.host) } else { format!("{}@{}:{}", self.user, self.host, self.port) }
    }
}

enum Input {
    Data(Vec<u8>),
    Resize(WindowSize),
}

struct SshPty {
    tx: mpsc::UnboundedSender<Input>,
    size: Mutex<WindowSize>,
}

impl Pty for SshPty {
    fn write(&self, bytes: Cow<'static, [u8]>) {
        if self.tx.send(Input::Data(bytes.into_owned())).is_err() {
            tracing::debug!("ssh.pty: write after session ended");
        }
    }

    fn resize(&self, size: WindowSize) {
        match self.size.lock() {
            Ok(mut s) => *s = size,
            Err(e) => tracing::error!("ssh.pty: size poisoned: {e}"),
        }
        if self.tx.send(Input::Resize(size)).is_err() {
            tracing::debug!("ssh.pty: resize after session ended");
        }
    }
}

/// Outcome of the host-key check, read after the handshake.
#[derive(Debug, Clone)]
enum HostKey {
    Unchecked,
    Known,
    Learned(String),
    Unknown(String),
    Changed { fingerprint: String, file: PathBuf, line: usize },
    Error(String),
}

struct Handler {
    host: String,
    port: u16,
    trust_new_host: bool,
    spike_known_hosts: PathBuf,
    result: Arc<Mutex<HostKey>>,
}

impl Handler {
    fn check(&self, key: &russh::keys::PublicKey) -> HostKey {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        let home_file = directories::UserDirs::new().map(|d| d.home_dir().join(".ssh").join("known_hosts"));
        for file in home_file.iter().chain(std::iter::once(&self.spike_known_hosts)) {
            match russh::keys::check_known_hosts_path(&self.host, self.port, key, file) {
                Ok(true) => {
                    tracing::info!(file = %file.display(), %fp, "ssh.hostkey: known");
                    return HostKey::Known;
                }
                Ok(false) => {}
                Err(russh::keys::Error::KeyChanged { line }) => {
                    return HostKey::Changed { fingerprint: fp, file: file.clone(), line };
                }
                Err(e) => return HostKey::Error(format!("reading {}: {e}", file.display())),
            }
        }
        if !self.trust_new_host {
            return HostKey::Unknown(fp);
        }
        if let Some(dir) = self.spike_known_hosts.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            return HostKey::Error(format!("creating {}: {e}", dir.display()));
        }
        match russh::keys::known_hosts::learn_known_hosts_path(&self.host, self.port, key, &self.spike_known_hosts) {
            Ok(()) => {
                tracing::info!(file = %self.spike_known_hosts.display(), %fp, "ssh.hostkey: learned");
                HostKey::Learned(fp)
            }
            Err(e) => HostKey::Error(format!("writing {}: {e}", self.spike_known_hosts.display())),
        }
    }
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let outcome = self.check(&server_key.public_key());
        let accept = matches!(outcome, HostKey::Known | HostKey::Learned(_));
        match self.result.lock() {
            Ok(mut r) => *r = outcome,
            Err(e) => tracing::error!("ssh.hostkey: result slot poisoned: {e}"),
        }
        Ok(accept)
    }
}

/// Open a terminal and start connecting in the background.
pub fn spawn(
    rt: &tokio::runtime::Handle,
    ctx: &egui::Context,
    target: Target,
    password: Option<Zeroizing<String>>,
    options: TermOptions,
    palette: Palette,
) -> Terminal {
    let (tx, rx) = mpsc::unbounded_channel();
    let pty = Arc::new(SshPty {
        tx,
        size: Mutex::new(WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 }),
    });
    let (term, feeder) = Terminal::new(ctx, pty.clone(), options, palette);
    feeder.feed(format!("Connecting to {}…\r\n", target.label()).as_bytes());
    rt.spawn(async move {
        let label = target.label();
        match run(&target, password, pty, rx, &feeder).await {
            Ok(reason) => feeder.exited(reason),
            Err(e) => {
                tracing::error!(target = %label, "ssh.session: {e:#}");
                feeder.feed(format!("\r\n\x1b[31mssh: {e:#}\x1b[0m\r\n").as_bytes());
                feeder.exited("connection failed");
            }
        }
    });
    term
}

fn spike_known_hosts() -> PathBuf {
    directories::ProjectDirs::from("ch", "ezSsh", "ezssh-spike")
        .map(|d| d.data_dir().join("known_hosts"))
        .unwrap_or_else(|| PathBuf::from("ezssh-spike-known_hosts"))
}

async fn run(
    target: &Target,
    password: Option<Zeroizing<String>>,
    pty: Arc<SshPty>,
    mut rx: mpsc::UnboundedReceiver<Input>,
    feeder: &Feeder,
) -> Result<String> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let result = Arc::new(Mutex::new(HostKey::Unchecked));
    let handler = Handler {
        host: target.host.clone(),
        port: target.port,
        trust_new_host: target.trust_new_host,
        spike_known_hosts: spike_known_hosts(),
        result: result.clone(),
    };
    tracing::info!(target = %target.label(), "ssh.connect.begin");
    let connected = tokio::time::timeout(
        Duration::from_secs(20),
        client::connect(config, (target.host.clone(), target.port), handler),
    )
    .await
    .map_err(|_| anyhow!("connect timeout after 20 s"))?;
    let host_key = result.lock().map(|r| r.clone()).unwrap_or(HostKey::Error("result slot poisoned".into()));
    let mut handle = match (connected, host_key) {
        (Ok(h), HostKey::Known) => h,
        (Ok(h), HostKey::Learned(fp)) => {
            feeder.feed(format!("Host key {fp} added to {}\r\n", spike_known_hosts().display()).as_bytes());
            h
        }
        (_, HostKey::Unknown(fp)) => bail!(
            "unknown host key {fp} for {}:{}; check it and rerun with --trust-new-host",
            target.host,
            target.port
        ),
        (_, HostKey::Changed { fingerprint, file, line }) => bail!(
            "HOST KEY CHANGED for {}:{} (now {fingerprint}); mismatching entry {}:{line}",
            target.host,
            target.port,
            file.display()
        ),
        (_, HostKey::Error(e)) => bail!("host key check failed: {e}"),
        (Err(e), _) => return Err(anyhow!(e).context("connect")),
        (Ok(_), HostKey::Unchecked) => bail!("server sent no host key"),
    };

    let method = authenticate(&mut handle, target, password).await?;
    tracing::info!(target = %target.label(), method, "ssh.auth.ok");

    let channel = handle.channel_open_session().await.context("open session channel")?;
    let size = pty.size.lock().map(|s| *s).map_err(|e| anyhow!("size poisoned: {e}"))?;
    channel
        .request_pty(
            false,
            "xterm-256color",
            size.num_cols as u32,
            size.num_lines as u32,
            (size.num_cols as u32) * size.cell_width as u32,
            (size.num_lines as u32) * size.cell_height as u32,
            &[],
        )
        .await
        .context("request pty")?;
    channel.request_shell(true).await.context("request shell")?;
    let (mut reader, writer) = channel.split();

    let reason = loop {
        tokio::select! {
            msg = reader.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => feeder.feed(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    tracing::info!(exit_status, "ssh.channel.exit");
                    break format!("exit status {exit_status}");
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => break format!("killed by SIG{signal_name:?}"),
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break "connection closed".to_string(),
                Some(other) => tracing::trace!(?other, "ssh.channel: ignored message"),
            },
            input = rx.recv() => match input {
                Some(Input::Data(bytes)) => {
                    if let Err(e) = writer.data(&bytes[..]).await {
                        break format!("write failed: {e}");
                    }
                }
                Some(Input::Resize(s)) => {
                    if let Err(e) = writer
                        .window_change(s.num_cols as u32, s.num_lines as u32, s.num_cols as u32 * s.cell_width as u32, s.num_lines as u32 * s.cell_height as u32)
                        .await
                    {
                        tracing::warn!("ssh.channel: window_change failed: {e}");
                    }
                }
                None => break "terminal closed".to_string(),
            },
        }
    };
    if let Err(e) = handle.disconnect(Disconnect::ByApplication, "", "en").await {
        tracing::debug!("ssh.disconnect: {e}");
    }
    Ok(reason)
}

#[cfg(unix)]
async fn agent() -> Result<russh::keys::agent::client::AgentClient<tokio::net::UnixStream>, russh::keys::Error> {
    russh::keys::agent::client::AgentClient::connect_env().await
}

#[cfg(windows)]
async fn agent()
-> Result<russh::keys::agent::client::AgentClient<tokio::net::windows::named_pipe::NamedPipeClient>, russh::keys::Error> {
    russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await
}

/// Try agent, key files, then password / keyboard-interactive. Returns the method that worked.
async fn authenticate(handle: &mut Handle<Handler>, target: &Target, password: Option<Zeroizing<String>>) -> Result<&'static str> {
    let user = target.user.clone();
    let rsa_hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
    let mut tried = Vec::new();

    match agent().await {
        Ok(mut agent) => match agent.request_identities().await {
            Ok(ids) => {
                for id in ids {
                    let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = id else { continue };
                    match handle.authenticate_publickey_with(user.clone(), key, rsa_hash, &mut agent).await {
                        Ok(r) if r.success() => return Ok("agent"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("ssh.auth: agent key failed: {e}"),
                    }
                }
                tried.push("agent");
            }
            Err(e) => tracing::info!("ssh.auth: agent identities unavailable: {e}"),
        },
        Err(e) => tracing::info!("ssh.auth: no agent: {e}"),
    }

    let key_files: Vec<PathBuf> = match &target.key {
        Some(k) => vec![k.clone()],
        None => directories::UserDirs::new()
            .map(|d| ["id_ed25519", "id_ecdsa", "id_rsa"].iter().map(|n| d.home_dir().join(".ssh").join(n)).collect())
            .unwrap_or_default(),
    };
    for path in key_files.iter().filter(|p| p.exists()) {
        let key = match russh::keys::load_secret_key(path, password.as_deref().map(|s| s.as_str())) {
            Ok(k) => k,
            Err(e) => {
                tracing::info!(key = %path.display(), "ssh.auth: key not usable: {e}");
                continue;
            }
        };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
        match handle.authenticate_publickey(user.clone(), key).await {
            Ok(r) if r.success() => return Ok("key file"),
            Ok(_) => tried.push("key file"),
            Err(e) => tracing::warn!(key = %path.display(), "ssh.auth: key failed: {e}"),
        }
    }

    if let Some(pw) = password {
        match handle.authenticate_password(user.clone(), pw.as_str()).await {
            Ok(r) if r.success() => return Ok("password"),
            Ok(_) => tried.push("password"),
            Err(e) => tracing::warn!("ssh.auth: password failed: {e}"),
        }
        // Servers with PasswordAuthentication off often still take the
        // password through keyboard-interactive.
        let mut resp = handle.authenticate_keyboard_interactive_start(user.clone(), None).await?;
        for _ in 0..3 {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok("keyboard-interactive"),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let answers = prompts.iter().map(|_| pw.as_str().to_string()).collect();
                    resp = handle.authenticate_keyboard_interactive_respond(answers).await?;
                }
            }
        }
        tried.push("keyboard-interactive");
    }
    bail!(
        "authentication failed (tried: {}); pass --password to be asked for one",
        if tried.is_empty() { "nothing".to_string() } else { tried.join(", ") }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        let t = Target::parse("root@example.com").unwrap();
        assert_eq!((t.user.as_str(), t.host.as_str(), t.port), ("root", "example.com", 22));
        let t = Target::parse("me@10.0.0.1:2222").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("10.0.0.1", 2222));
        assert!(Target::parse("example.com").is_err());
        assert!(Target::parse("a@b:notaport").is_err());
    }
}
