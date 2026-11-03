//! A small SSH server for tests and manual checks of the file pane.
//!
//! - password auth against one user,
//! - a shell channel that echoes what it gets (`exit` closes it),
//! - the `sftp` subsystem over a local folder: remote `/` is `root`,
//! - and, separately, [`http::serve`] for updater tests.
//!
//! Not a security boundary: paths are resolved lexically (`..` stops at
//! the root) but a symlink inside `root` that points out of it is
//! followed. Bind it to loopback only.

pub mod http;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh::keys::ssh_key::{Algorithm, HashAlg};
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use russh_sftp::server::StatusReply;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// What the server accepts and serves.
#[derive(Clone, Debug)]
pub struct Config {
    /// Folder shown as remote `/`.
    pub root: PathBuf,
    /// Remote home directory (`realpath(".")`), absolute, inside `root`.
    pub home: String,
    pub user: String,
    pub password: String,
    /// Sleep before every SFTP read and write, to watch progress bars.
    pub op_delay: Duration,
}

impl Config {
    pub fn new(root: impl Into<PathBuf>, user: &str, password: &str) -> Self {
        Self {
            root: root.into(),
            home: "/".into(),
            user: user.into(),
            password: password.into(),
            op_delay: Duration::ZERO,
        }
    }
}

/// A running server. Stops when dropped.
pub struct TestServer {
    pub addr: SocketAddr,
    /// `SHA256:…` fingerprint of the host key, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    task: JoinHandle<()>,
}

impl TestServer {
    /// Listen on `bind` (port 0 picks a free one) inside the current tokio
    /// runtime.
    pub async fn start(cfg: Config, bind: SocketAddr) -> anyhow::Result<Self> {
        let root = std::fs::canonicalize(&cfg.root)
            .map_err(|e| anyhow::anyhow!("root {}: {e}", cfg.root.display()))?;
        if !root.is_dir() {
            anyhow::bail!("root {} is not a directory", root.display());
        }
        let key = russh::keys::PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .map_err(|e| anyhow::anyhow!("host key: {e}"))?;
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let config = Arc::new(russh::server::Config {
            auth_rejection_time: Duration::from_millis(100),
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![key],
            ..Default::default()
        });
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared { cfg: Config { root, ..cfg } });
        tracing::info!(%addr, root = %shared.cfg.root.display(), user = %shared.cfg.user, "testsrv: listening");
        let task = tokio::spawn(async move {
            loop {
                let (stream, peer) = match listener.accept().await {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("testsrv: accept failed: {e}");
                        continue;
                    }
                };
                tracing::debug!(%peer, "testsrv: connection");
                let handler = SshHandler { shared: shared.clone(), channels: HashMap::new(), shells: Default::default() };
                let config = config.clone();
                tokio::spawn(async move {
                    match russh::server::run_stream(config, stream, handler).await {
                        Ok(running) => {
                            if let Err(e) = running.await {
                                tracing::debug!(%peer, "testsrv: session ended: {e}");
                            }
                        }
                        Err(e) => tracing::warn!(%peer, "testsrv: handshake failed: {e}"),
                    }
                });
            }
        });
        Ok(Self { addr, fingerprint, task })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Shared {
    cfg: Config,
}

struct SshHandler {
    shared: Arc<Shared>,
    /// Session channels until a subsystem request takes one.
    channels: HashMap<ChannelId, Channel<Msg>>,
    /// Channels that asked for a shell: the only ones that echo
    /// (russh hands `data` of subsystem channels to the handler too).
    shells: std::collections::HashSet<ChannelId>,
}

impl russh::server::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let ok = user == self.shared.cfg.user && password == self.shared.cfg.password;
        tracing::debug!(user, ok, "testsrv: password auth");
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn env_request(&mut self, channel: ChannelId, _k: &str, _v: &str, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        self.shells.insert(channel);
        // Nobody reads a shell channel's `Channel<Msg>` (the echo runs in
        // `data`); russh still queues every packet on it and its session
        // loop blocks once 100 wait unread. Drain it.
        if let Some(mut ch) = self.channels.remove(&channel) {
            tokio::spawn(async move { while ch.wait().await.is_some() {} });
        }
        session.data(channel, &b"ezssh-testsrv echo shell (type exit to leave)\r\n$ "[..])?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        // Echo, turning Enter into a new prompt.
        if !self.shells.contains(&channel) {
            return Ok(());
        }
        let mut out = Vec::with_capacity(data.len() + 4);
        for &b in data {
            if b == b'\r' {
                out.extend_from_slice(b"\r\n$ ");
            } else {
                out.push(b);
            }
        }
        session.data(channel, out)?;
        if data.windows(4).any(|w| w == b"exit") {
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(&mut self, channel_id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        tracing::debug!(name, "testsrv: subsystem request");
        match (name, self.channels.remove(&channel_id)) {
            ("sftp", Some(channel)) => {
                session.channel_success(channel_id)?;
                let fs = LocalFs { shared: self.shared.clone(), handles: HashMap::new(), next: 1, version: None };
                russh_sftp::server::run(channel.into_stream(), fs).await;
            }
            (_, ch) => {
                if let Some(ch) = ch {
                    self.channels.insert(channel_id, ch);
                }
                session.channel_failure(channel_id)?;
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ SFTP

enum Open {
    File(tokio::fs::File),
    /// Entries left to send; `None` once sent (next readdir is EOF).
    Dir(Option<Vec<File>>),
}

struct LocalFs {
    shared: Arc<Shared>,
    handles: HashMap<String, Open>,
    next: u64,
    version: Option<u32>,
}

fn fail(e: io::Error) -> StatusReply {
    let code = match e.kind() {
        io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    };
    tracing::debug!(?code, "testsrv: sftp error: {e}");
    code.with_message(e.to_string())
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() }
}

/// Collapse `.` / `..` lexically; the result is absolute and `..` stops at `/`.
pub fn normalise(base: &str, path: &str) -> String {
    let joined = if path.starts_with('/') { path.to_string() } else { format!("{base}/{path}") };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

fn attrs_of(meta: &std::fs::Metadata) -> FileAttributes {
    let mut a = FileAttributes::from(meta);
    if meta.file_type().is_symlink() {
        a.set_regular(false);
        a.set_symlink(true);
    }
    a
}

impl LocalFs {
    fn local(&self, remote: &str) -> PathBuf {
        let clean = normalise(&self.shared.cfg.home, remote);
        let mut p = self.shared.cfg.root.clone();
        for c in Path::new(clean.trim_start_matches('/')).components() {
            if let Component::Normal(s) = c {
                p.push(s);
            }
        }
        p
    }

    fn handle(&mut self, open: Open) -> String {
        let h = format!("h{}", self.next);
        self.next += 1;
        self.handles.insert(h.clone(), open);
        h
    }

    async fn pause(&self) {
        if !self.shared.cfg.op_delay.is_zero() {
            tokio::time::sleep(self.shared.cfg.op_delay).await;
        }
    }
}

impl russh_sftp::server::Handler for LocalFs {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported.into()
    }

    async fn init(&mut self, version: u32, _ext: HashMap<String, String>) -> Result<Version, Self::Error> {
        if self.version.replace(version).is_some() {
            return Err(StatusCode::ConnectionLost.into());
        }
        Ok(Version::new())
    }

    async fn open(&mut self, id: u32, filename: String, pflags: OpenFlags, _attrs: FileAttributes) -> Result<Handle, Self::Error> {
        let path = self.local(&filename);
        tracing::debug!(%filename, ?pflags, "testsrv: open");
        let opts: std::fs::OpenOptions = pflags.into();
        let file = tokio::fs::OpenOptions::from(opts).open(&path).await.map_err(fail)?;
        Ok(Handle { id, handle: self.handle(Open::File(file)) })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        match self.handles.remove(&handle) {
            Some(Open::File(mut f)) => f.flush().await.map_err(fail)?,
            Some(Open::Dir(_)) => {}
            None => return Err(StatusCode::Failure.with_message("unknown handle")),
        }
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        self.pause().await;
        let Some(Open::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure.with_message("not a file handle"));
        };
        f.seek(io::SeekFrom::Start(offset)).await.map_err(fail)?;
        let mut buf = vec![0u8; len.min(256 * 1024) as usize];
        let n = f.read(&mut buf).await.map_err(fail)?;
        if n == 0 {
            return Err(StatusCode::Eof.into());
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, Self::Error> {
        self.pause().await;
        let Some(Open::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure.with_message("not a file handle"));
        };
        f.seek(io::SeekFrom::Start(offset)).await.map_err(fail)?;
        f.write_all(&data).await.map_err(fail)?;
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = tokio::fs::symlink_metadata(self.local(&path)).await.map_err(fail)?;
        Ok(Attrs { id, attrs: attrs_of(&meta) })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = tokio::fs::metadata(self.local(&path)).await.map_err(fail)?;
        Ok(Attrs { id, attrs: attrs_of(&meta) })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let Some(Open::File(f)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure.with_message("not a file handle"));
        };
        let meta = f.metadata().await.map_err(fail)?;
        Ok(Attrs { id, attrs: attrs_of(&meta) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        #[cfg(unix)]
        if let Some(mode) = attrs.permissions {
            use std::os::unix::fs::PermissionsExt;
            let perm = std::fs::Permissions::from_mode(mode & 0o7777);
            tokio::fs::set_permissions(self.local(&path), perm).await.map_err(fail)?;
        }
        #[cfg(not(unix))]
        let _ = (path, attrs);
        Ok(ok(id))
    }

    async fn fsetstat(&mut self, id: u32, _handle: String, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let dir = self.local(&path);
        tracing::debug!(%path, "testsrv: opendir");
        let mut rd = tokio::fs::read_dir(&dir).await.map_err(fail)?;
        let mut files = vec![File::new(".", FileAttributes::dummy()), File::new("..", FileAttributes::dummy())];
        while let Some(e) = rd.next_entry().await.map_err(fail)? {
            let meta = e.metadata().await.map_err(fail)?;
            files.push(File::new(e.file_name().to_string_lossy().into_owned(), attrs_of(&meta)));
        }
        Ok(Handle { id, handle: self.handle(Open::Dir(Some(files))) })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&handle) {
            Some(Open::Dir(pending)) => match pending.take() {
                Some(files) => Ok(Name { id, files }),
                None => Err(StatusCode::Eof.into()),
            },
            _ => Err(StatusCode::Failure.with_message("not a directory handle")),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        tokio::fs::remove_file(self.local(&filename)).await.map_err(fail)?;
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        tokio::fs::create_dir(self.local(&path)).await.map_err(fail)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        tokio::fs::remove_dir(self.local(&path)).await.map_err(fail)?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let clean = normalise(&self.shared.cfg.home, &path);
        tracing::debug!(%path, %clean, "testsrv: realpath");
        Ok(Name { id, files: vec![File::dummy(clean)] })
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> Result<Status, Self::Error> {
        let to = self.local(&newpath);
        // SFTP v3 rename refuses to overwrite (OpenSSH does too).
        if tokio::fs::symlink_metadata(&to).await.is_ok() {
            return Err(StatusCode::Failure.with_message("target exists"));
        }
        tokio::fs::rename(self.local(&oldpath), to).await.map_err(fail)?;
        Ok(ok(id))
    }
}

#[cfg(test)]
mod tests {
    use super::normalise;

    #[test]
    fn normalise_stays_inside_the_root() {
        assert_eq!(normalise("/", "."), "/");
        assert_eq!(normalise("/home/u", "."), "/home/u");
        assert_eq!(normalise("/home/u", "a/../b"), "/home/u/b");
        assert_eq!(normalise("/", "../../etc"), "/etc");
        assert_eq!(normalise("/x", "/a//b/./c/"), "/a/b/c");
    }
}
