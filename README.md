# ezSsh

A free, open-source SSH client with a MobaXterm-style session manager,
encrypted credential vault, terminal, SFTP side-pane, WSL and local-shell
tabs, port forwards and X11 forwarding. One native Rust program: an
[egui](https://github.com/emilk/egui) window around an
[alacritty_terminal](https://crates.io/crates/alacritty_terminal) terminal,
no webview.

ezSsh is a permanent fork of [ezTerm](https://github.com/ZerosAndOnesLLC/ezTerm)
(Tauri + Next.js + xterm.js), rebuilt natively from version 2.0; the rework is
documented in [docs/design/native-rework-plan.md](docs/design/native-rework-plan.md).
It opens the database of an existing ezTerm install: sessions, folders,
credentials, known hosts and settings carry over, and ezTerm backups and sync
bundles restore.

**Status:** 2.0.0-beta0, the first native build. Windows-first; Linux and
macOS builds come from the same release pipeline, where WSL, local Windows
shells and the bundled X server do not apply.

**Licence:** GNU General Public License, version 3 — see [LICENSE](LICENSE).

## Features

### Connections
- **SSH** over [russh](https://crates.io/crates/russh) with password, private
  key, or SSH-agent auth. Host-key TOFU on first connect; hard-fail on mismatch.
- **WSL** tabs — `wsl.exe -d <distro> [-u <user>]` under a ConPTY. `code .`,
  `explorer.exe`, and the rest of WSL interop Just Works.
- **Local shells** — `cmd`, `powershell`, `pwsh`, or any absolute path, with an
  optional starting directory.
- **X11 forwarding** against bundled [VcXsrv](https://sourceforge.net/projects/vcxsrv/) —
  tick "Forward X11" on an SSH session and remote GUI apps (`xeyes`, `gedit`,
  JetBrains tools, …) pop as native Windows windows. The Windows release
  tarball ships VcXsrv in a `vcxsrv/` subfolder so no separate install is
  needed.
- **Port forwarding** — Local (`-L`), Remote (`-R`), and Dynamic / SOCKS5 (`-D`)
  tunnels. Save them to a session to auto-start on connect, or add ad-hoc tunnels
  to a live tab. A side pane shows each tunnel's live state (green running / red
  stopped, amber when the port is already in use by another tab or window) with
  start / stop / edit / delete from buttons or a right-click menu.
- **MobaXterm import** — point at a `.mxtsessions` export or `MobaXterm.ini` and
  import SSH + WSL rows with their folder structure. Private-key files are read
  off disk and stored as encrypted vault credentials, auto-attached to the
  matching sessions.

### Terminal
- [alacritty_terminal](https://crates.io/crates/alacritty_terminal) with **full 16-colour + 256-colour + 24-bit truecolor** support
  (remote sees `TERM=xterm-256color`).
- Copy / Paste (`Ctrl+Shift+C` / `Ctrl+Shift+V` / `Shift+Insert`), Select All,
  Clear Scrollback, Find (`Ctrl+Shift+F`) with case-sensitive / regex toggles.
- **Ctrl + mouse wheel** zoom (8–48 pt, SSH channel resized on every step).
- **OSC 52 clipboard writes** — programs on the remote end (Claude Code,
  neovim, tmux) can copy to your local clipboard through SSH. Reads are
  refused so a remote host can never pull clipboard contents.
- Clickable links (URLs and OSC 8 hyperlinks), bracketed paste, mouse
  reporting for tmux / vim / htop, kitty keyboard protocol.
- Per-session font size, scrollback depth, cursor style, env vars, keepalive,
  connect timeout, initial command, compression.

### Settings

- **Settings panel** (sidebar menu or the gear in the sidebar footer). Every control
  saves on change. Terminal colour palette (ezSsh dark / light, Campbell,
  Solarized, Dracula, Gruvbox, Nord, One Dark, Monokai, Tango, and the
  GNOME / GitHub / Google light + dark pairs from Ptyxis — applied live to
  open tabs), defaults for new sessions (scrollback, font, cursor,
  keepalive, connect timeout), copy-on-select, right-click paste, a
  warning before multi-line pastes, the OSC 52 clipboard switch, bell
  behaviour, confirm-before-close / quit, auto-open the file pane, update
  channel and cadence, and an idle auto-lock timer for the vault.

### Workspace
- **Sessions sidebar** — folders + drag-and-drop (sessions into folders, folders
  into folders, drop on the tree background to root). Folder session-count
  badges, coloured icons per folder, green-glow rail for the currently connected
  row. Resizable with a grab-strip on the right edge (180 – 520 px, persisted).
- **Tab bar** with coloured status dots (pulse on connecting, green on
  connected, red on error), middle-click to close, per-tab SFTP toggle.
  Tabs auto-size down to a readable floor, then the strip scrolls (mouse
  wheel or edge chevrons) and keeps the active tab in view.
- **SFTP side-pane** docked on the active SSH or WSL tab — resizable with a
  grab-strip on its right edge (200 – 640 px, persisted), Name and Size
  columns resizable from the header (drag, double-click to reset, arrow
  keys when focused; the list scrolls sideways once the columns outgrow
  the pane) — breadcrumb navigation, multi-select, right-click context menu
  (Download, Rename, Delete, New folder), drop files and folders from the file
  manager to upload them, download files and whole folders, live progress
  with cancel. WSL
  tabs browse the distro's filesystem through the Plan 9 bridge
  (`\\wsl.localhost\<distro>\…`) and shell out to the distro's coreutils
  for POSIX-aware listing — same UX as MobaXterm's WSL pane,
  no extra packages required.
- **Inline auth-fix overlay** — when a connect fails with bad auth / missing
  credential, a dedicated overlay lets the user fix username, auth method, or
  vault credential without leaving the tab. No "close tab and start over" ritual.
- **Six view modes** (the Layout menu in the tab strip, `Ctrl+Shift+W`
  cycles): tabs, tile horizontally / vertically / grid / auto, and cascade
  with minimise / maximise / restore. `Alt+1`…`Alt+9` pick a tab (`Alt+9`
  the last; `⌘1`…`⌘9` on macOS), also while a terminal has the keyboard.
- **Sessions sidebar footer** — update check, lock, theme and settings;
  `Ctrl+K` focuses the session filter.
- **Status bar** — active-session `user@host`, SFTP cwd, X-server and
  update chips, version.
- **Toasts** for create / rename / delete / import events.
- Dark by default with a light theme toggle; theme persists per user.

### Security
- **Vault** — Argon2id KDF → ChaCha20-Poly1305 AEAD for every stored secret.
  Master password unlocks the app; secrets live encrypted at rest in SQLite and
  are zeroised in memory after use.
- **Credentials** — a session's password belongs to that session; private keys
  and key passphrases are shared credentials that several sessions can use.
- **Signed updates** — the in-app updater installs only installers signed with
  the project's update key (minisign), on a stable or pre-release channel.
- **Known hosts** — stored in SQLite, not `~/.ssh/known_hosts`. Mismatch is a
  hard fail unless the user explicitly trusts the new fingerprint.
- **Redacted logging** — host / user / fingerprint are the only identifiers
  that appear in traces; secrets never touch logs.

## Install

Installers and portable archives are on the
[Releases](https://github.com/the-angwe/ezssh/releases) page:

| Platform | Installer | Portable |
|---|---|---|
| Windows x86_64 | `ezssh_<version>_x64-setup.exe` (per user) or `ezssh_<version>_x64_en-US.msi` | `ezSsh_<version>_windows-x64-portable.tar.xz` |
| Linux x86_64 | `ezssh_<version>_x86_64.AppImage`, `ezssh_<version>_amd64.deb` | `ezSsh_<version>_linux-amd64-portable.tar.xz` |
| Linux aarch64 | `ezssh_<version>_aarch64.AppImage`, `ezssh_<version>_arm64.deb` | `ezSsh_<version>_linux-aarch64-portable.tar.xz` |
| macOS aarch64 | `ezssh_<version>_aarch64.app.tar.gz` (unsigned) | `ezSsh_<version>_macos-aarch64-portable.tar.xz` |

Installed copies update themselves from the release's signed `latest.json`;
portable copies link to the release instead.

ezSsh installs next to ezTerm (its own identifier, `xyz.angwe.ezssh`) and uses
ezTerm's data folder, so quit ezTerm before starting ezSsh on the same
database. The first start upgrades that database, after which ezTerm no
longer opens it; back it up first if you may go back. ezTerm v1 installs do
not update into ezSsh: install ezSsh once by hand.

**Runtime dependencies:**
- **Windows** — the installers and the portable archive bundle
  [VcXsrv](https://sourceforge.net/projects/vcxsrv/) next to `ezssh.exe`, so
  X11 forwarding works out of the box; a system install at
  `%ProgramFiles%\VcXsrv\` is used when the bundled copy is missing. Software
  rendering (WARP) is used automatically on machines without a usable GPU
  (VMs, RDP).
- **Linux** — xkbcommon, fontconfig, the X11 or Wayland client libraries, and
  Vulkan or EGL (the deb lists them). No webkit2gtk.

## Development

```bash
cargo run -p ezssh-app                          # the real database
cargo run -p ezssh-app -- --db /tmp/x/ezterm.sqlite   # a throwaway one
cargo run -p ezssh-app -- --help                # renderer and other options
```

Linux needs the xkbcommon / xcb headers to build
(`libxkbcommon-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`).
`./build.sh` (or `build.cmd` on Windows) wraps the usual commands.

### Tests + linters

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The app and terminal tests drive the real UI headless with
[egui_kittest](https://crates.io/crates/egui_kittest); `EZSSH_UI_SHOTS=<dir>`
also renders their screens to PNG. File-pane and updater tests run against
`crates/ezssh-testsrv`, an in-process SSH / SFTP / HTTP server.

### Local release build

```bash
cargo build -p ezssh-app --profile dist        # target/dist/ezssh (fat LTO)
cargo packager -p ezssh-app -f deb,appimage    # installers, see crates/ezssh-app/packaging/
```

## Architecture

```
ezssh/
├── crates/
│   ├── ezssh-core/     backend: SSH / SFTP, vault, DB, local PTY, WSL fs, forwards,
│   │                   sync, backup, import, X server, updater; `api::*` on `&AppState`
│   ├── ezssh-term/     terminal widget: alacritty_terminal model, egui painter,
│   │                   key / mouse encoding, palettes, fonts
│   ├── ezssh-app/      the `ezssh` binary: window, sidebar, tabs, dialogs, file pane
│   ├── ezssh-testsrv/  SSH + SFTP + HTTP test server (tests and manual checks)
│   └── ezssh-release/  update key, installer signatures, latest.json
├── migrations/         sqlx migrations (SQLite)
└── .github/            CI and release workflows
```

- **One process** — eframe owns the main thread; a tokio runtime on a
  background thread owns every connection. The UI calls `ezssh_core::api::*`
  and gets the result back on the UI thread; async events (transfers,
  forward status, update progress) arrive through `events::EventSink`.
- **Terminal data path** — the SSH or PTY reader task feeds output straight
  into the tab's terminal model and asks for a repaint; the UI thread only
  locks the model to paint, resize, select or search. Keystrokes are encoded
  by `ezssh-term` and written to the connection.
- **Session kinds** — a single `sessions` table with a `session_kind` column
  (`ssh` / `wsl` / `local`) re-purposes `host` + `username` per kind. The
  connect flow dispatches to either the russh client (SSH) or the
  `portable-pty` backend (WSL / local).
- **X11 forwarding** — russh's `server_channel_open_x11` handler pipes each
  incoming X11 channel bidirectionally into a loopback TCP connection to
  VcXsrv. VcXsrv lifecycle is ref-counted per display.

## Releases

See [docs/RELEASING.md](docs/RELEASING.md). tl;dr: bump the version in
`Cargo.toml`, add `docs/release-notes/v<version>.md`, push a `v<version>` tag;
GitHub Actions builds the installers and portable archives, signs them, writes
`latest.json` and opens a draft release for you to review.

## Licence

ezSsh is licensed under the **GNU General Public License, version 3** (GPLv3
only — not "or later"). See [LICENSE](LICENSE) for the full text.

Third-party components retain their own licences:
- `egui`, `eframe`, `alacritty_terminal` — MIT / Apache 2.0
- `russh`, `russh-sftp` — Apache 2.0
- `sqlx`, `tokio` — MIT / Apache 2.0
- `portable-pty` — MIT
- Lucide icons — ISC
- Symbols Nerd Font — MIT
- VcXsrv (bundled on Windows) — GPL v2

## Contributing

Issues and PRs welcome at <https://github.com/the-angwe/ezssh>; see
[CONTRIBUTING.md](CONTRIBUTING.md). Please run `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` before opening a PR.
