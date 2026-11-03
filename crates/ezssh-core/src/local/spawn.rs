//! PTY spawn path for local shells and WSL — see the module docs in
//! `local/mod.rs` for the data-flow overview.

use std::io::{Read, Write};
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;

use crate::error::{AppError, Result};
use crate::events::{CloseReason, Event, Output};
use crate::state::AppState;
use super::{Connection, LocalInput, WslMeta};

pub struct SpawnRequest {
    /// "wsl" or "local".
    pub kind:     String,
    /// For wsl: distro name (empty = default distro). For local: shell
    /// short-name ('cmd' / 'pwsh' / 'powershell') or absolute path.
    pub program:  String,
    /// For wsl: optional user (empty = distro default). For local: optional
    /// starting directory (empty = process cwd).
    pub extra:    String,
    /// For wsl: optional Linux starting dir passed as `wsl.exe --cd
    /// <value>`; empty/None falls back to `~` so users land in their
    /// Linux home. Ignored for local (cmd/pwsh) rows — those continue
    /// to use `extra` as the Windows cwd.
    pub starting_dir: Option<String>,
    pub cols:     u16,
    pub rows:     u16,
}

pub struct SpawnOutcome {
    pub connection_id: u64,
}

pub async fn spawn(
    state: &AppState,
    req: SpawnRequest,
    output: Output,
) -> Result<SpawnOutcome> {
    let cmd = build_command(&req.kind, &req.program, &req.extra, req.starting_dir.as_deref())?;
    let id = state.local.alloc_id();
    // For WSL tabs, capture distro + user so the file browser can resolve
    // `\\wsl.localhost\<distro>\` paths and run `wsl.exe -d <distro> -u
    // <user>` for POSIX-aware ops (ls/chmod/realpath). Empty `program`
    // means "default distro" — leave the stored name empty and let
    // wslfs::open() resolve it lazily on first use.
    let wsl_meta = if req.kind == "wsl" {
        let user = req.extra.trim();
        Some(WslMeta {
            distro: req.program.trim().to_string(),
            user:   if user.is_empty() { None } else { Some(user.to_string()) },
        })
    } else {
        None
    };

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: req.rows,
            cols: req.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::Ssh(format!("pty open: {e}")))?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::Ssh(format!("spawn: {e}")))?;
    // Releasing the slave lets the PTY signal EOF on the reader when the
    // child exits. The child handle stays with the writer thread: on
    // close it is killed before the writer drops (on Unix portable-pty's
    // writer sends "\n" + VEOF on drop, which would *run* a half-typed
    // command line and leave a shell with a foreground program alive) and
    // then waited on, so no zombie remains per closed tab.
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::Ssh(format!("reader: {e}")))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::Ssh(format!("writer: {e}")))?;
    let master = pair.master;

    let (tx, mut rx) = mpsc::unbounded_channel::<LocalInput>();
    // The reader thread asks the writer to reap once the child is gone.
    let reaper_tx = tx.clone();

    // Reader: bytes → the UI's output sink; EOF or read error →
    // `Event::Closed` with no exit status.
    let reader_events = state.events.clone();
    let reader_id = id;
    thread::spawn(move || {
        let mut buf = vec![0u8; 8192];
        let mut consumer_gone = false;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if !output.send(&buf[..n]) && !consumer_gone {
                        consumer_gone = true;
                        tracing::debug!(connection_id = reader_id, "local: output consumer gone, dropping output until close");
                    }
                }
                Err(e) => {
                    tracing::debug!(connection_id = reader_id, "local pty read ended: {e}");
                    break;
                }
            }
        }
        reader_events.emit(Event::Closed { connection_id: reader_id, reason: CloseReason::Ended });
        // Let the writer thread reap the child; it is a no-op if the UI's
        // disconnect already arrived.
        let _ = reaper_tx.send(LocalInput::Close);
    });

    // Writer / resize: blocking_recv on tokio mpsc from a std thread.
    thread::spawn(move || {
        let master = master;
        let mut child = child;
        while let Some(input) = rx.blocking_recv() {
            match input {
                LocalInput::Bytes(bytes) => {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                LocalInput::Resize { cols, rows } => {
                    let _ = master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
                LocalInput::Close => break,
            }
        }
        // Close (user, or the reader saw EOF), or every sender is gone.
        // Kill first: a shell with a foreground program would otherwise
        // survive the writer's VEOF and the reader would never see EOF.
        match child.try_wait() {
            Ok(Some(status)) => tracing::debug!(connection_id = id, %status, "local: child already exited"),
            Ok(None) => {
                if let Err(e) = child.kill() {
                    tracing::warn!(connection_id = id, "local: kill failed: {e}");
                }
                match child.wait() {
                    Ok(status) => tracing::debug!(connection_id = id, %status, "local: child reaped"),
                    Err(e) => tracing::warn!(connection_id = id, "local: wait failed: {e}"),
                }
            }
            Err(e) => tracing::warn!(connection_id = id, "local: try_wait failed: {e}"),
        }
        // Drop order: writer → master → PTY handles.
        drop(writer);
        drop(master);
    });

    state
        .local
        .insert(Connection {
            id,
            stdin: tx,
            wsl_meta,
        })
        .await;
    Ok(SpawnOutcome { connection_id: id })
}

fn build_command(
    kind: &str,
    program: &str,
    extra: &str,
    starting_dir: Option<&str>,
) -> Result<CommandBuilder> {
    match kind {
        "wsl" => {
            let mut c = CommandBuilder::new("wsl.exe");
            if !program.trim().is_empty() {
                c.arg("-d");
                c.arg(program.trim());
            }
            if !extra.trim().is_empty() {
                c.arg("-u");
                c.arg(extra.trim());
            }
            // Start in the user's configured Linux directory, or `~` if
            // unset. Without `--cd`, WSL inherits the Windows cwd and
            // translates it to `/mnt/c/...`, which is almost never what
            // the user wants. `~` is recognised by `wsl.exe --cd` and
            // expanded inside the distro.
            let cd = starting_dir
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("~");
            c.arg("--cd");
            c.arg(cd);
            Ok(c)
        }
        "local" => build_local_command(program, extra),
        _ => Err(AppError::Validation(format!("invalid session kind: {kind}"))),
    }
}

#[cfg(windows)]
fn build_local_command(program: &str, extra: &str) -> Result<CommandBuilder> {
    let exe = match program.trim() {
        "" | "cmd" => "cmd.exe",
        "pwsh" => "pwsh.exe",
        "powershell" => "powershell.exe",
        other => other,
    };
    let mut c = CommandBuilder::new(exe);
    if exe.ends_with("pwsh.exe") || exe.ends_with("powershell.exe") {
        c.arg("-NoLogo");
    }
    let cwd = extra.trim();
    if !cwd.is_empty() {
        c.cwd(cwd);
    }
    Ok(c)
}

/// Unix `local` shell launcher. `program` is an absolute shell binary
/// path (e.g. `/bin/bash`); blank falls back to `$SHELL` then `/bin/sh`.
/// `extra` is an optional cwd, mirroring Windows local semantics.
///
/// Invoked as an interactive shell (`-i`), matching gnome-terminal /
/// Konsole / iTerm defaults: `~/.bashrc` (and `~/.zshrc` etc.) load
/// reliably, so users' PATH and exports from the rc file are picked up.
/// A bare `-l` login shell only reads `~/.profile` / `~/.bash_profile`
/// and skips the rc file unless the user wired it up themselves, which
/// surprised users with "env vars don't load" reports (#104).
#[cfg(unix)]
fn build_local_command(program: &str, extra: &str) -> Result<CommandBuilder> {
    let shell = match program.trim() {
        "" => std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        other => other.to_string(),
    };
    let mut c = CommandBuilder::new(&shell);
    c.arg("-i");
    let cwd = extra.trim();
    if !cwd.is_empty() {
        c.cwd(cwd);
    }
    Ok(c)
}

#[cfg(not(any(unix, windows)))]
fn build_local_command(_program: &str, _extra: &str) -> Result<CommandBuilder> {
    Err(AppError::Validation(
        "local shells are not supported on this platform".into(),
    ))
}

/// Convenience for the API layer. Takes a `i64` session id and resolves the
/// DB row to assemble a `SpawnRequest`.
pub async fn spawn_from_session(
    state: &AppState,
    session_id: i64,
    cols: u16,
    rows: u16,
    output: Output,
) -> Result<SpawnOutcome> {
    let session = crate::db::sessions::get(&state.db, session_id).await?;
    spawn(
        state,
        SpawnRequest {
            kind: session.session_kind.clone(),
            program: session.host.clone(),
            extra: session.username.clone(),
            starting_dir: session.starting_dir.clone(),
            cols,
            rows,
        },
        output,
    )
    .await
}
