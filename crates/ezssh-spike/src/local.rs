//! Local shell over a PTY (portable-pty: openpty on Unix, ConPTY on Windows).

use std::borrow::Cow;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, mpsc};

use anyhow::{Context as _, Result};
use eframe::egui;
use ezssh_term::{Palette, Pty, TermOptions, Terminal, WindowSize};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

struct LocalPty {
    tx: mpsc::Sender<Vec<u8>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
}

impl Pty for LocalPty {
    fn write(&self, bytes: Cow<'static, [u8]>) {
        if let Err(e) = self.tx.send(bytes.into_owned()) {
            tracing::warn!("local.pty: write after writer thread ended: {e}");
        }
    }

    fn resize(&self, size: WindowSize) {
        let pty_size = PtySize {
            rows: size.num_lines,
            cols: size.num_cols,
            pixel_width: size.num_cols.saturating_mul(size.cell_width),
            pixel_height: size.num_lines.saturating_mul(size.cell_height),
        };
        match self.master.lock() {
            Ok(m) => {
                if let Err(e) = m.resize(pty_size) {
                    tracing::error!("local.pty: resize to {}x{} failed: {e}", size.num_cols, size.num_lines);
                }
            }
            Err(e) => tracing::error!("local.pty: master poisoned: {e}"),
        }
    }
}

/// Start `shell` (or the platform default) and return its terminal.
pub fn spawn(ctx: &egui::Context, shell: Option<&str>, options: TermOptions, palette: Palette) -> Result<Terminal> {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .context("openpty")?;
    let mut cmd = match shell {
        Some(s) => CommandBuilder::new(s),
        None => CommandBuilder::new_default_prog(),
    };
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "ezSsh");
    if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()) {
        cmd.cwd(home);
    }
    let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("spawn {}", shell.unwrap_or("default shell")))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().context("pty reader")?;
    let mut writer = pair.master.take_writer().context("pty writer")?;
    tracing::info!(shell = shell.unwrap_or("<default>"), pid = ?child.process_id(), "local.spawn");

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name("local-pty-writer".into())
        .spawn(move || {
            for bytes in rx {
                if let Err(e) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                    tracing::warn!("local.pty: write failed, writer ends: {e}");
                    break;
                }
            }
        })
        .context("writer thread")?;

    let pty = Arc::new(LocalPty { tx, master: Mutex::new(pair.master) });
    let (term, feeder) = Terminal::new(ctx, pty, options, palette);

    let reader_feeder = feeder.clone();
    std::thread::Builder::new()
        .name("local-pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        tracing::debug!("local.pty: reader EOF");
                        break;
                    }
                    Ok(n) => reader_feeder.feed(&buf[..n]),
                    Err(e) => {
                        // EIO is how Linux reports the slave side closing.
                        tracing::debug!("local.pty: reader ended: {e}");
                        break;
                    }
                }
            }
        })
        .context("reader thread")?;

    std::thread::Builder::new()
        .name("local-pty-wait".into())
        .spawn(move || match child.wait() {
            Ok(status) => feeder.exited(format!("process exited with code {}", status.exit_code())),
            Err(e) => {
                tracing::error!("local.pty: wait failed: {e}");
                feeder.exited(format!("wait failed: {e}"));
            }
        })
        .context("wait thread")?;

    Ok(term)
}
