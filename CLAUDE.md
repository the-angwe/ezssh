# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ezSsh is a free, open-source Windows SSH client that should **look and feel like MobaXterm**. It is a **permanent fork** of ezTerm (`github.com/ZerosAndOnesLLC/ezTerm`); nothing goes upstream. Target feature set: connection/session manager, encrypted credential vault ("locker"), SSH, SFTP, and an xterm-compatible terminal.

### Native rework (in progress)

The app is being rebuilt as one native Rust process (egui/eframe 0.37 + alacritty_terminal 0.26), replacing Tauri + Next.js + xterm.js. Plan, phases and results: `docs/design/native-rework-plan.md`.

- Phase 1 (spike) is built: `crates/ezssh-term` (terminal widget) and `crates/ezssh-spike` (test window; see its README). Results and open gate items in §9 of the plan.
- Phase 2 (core extraction) is built: `crates/ezssh-core` holds the whole backend (DB, vault, SSH/SFTP, local PTY, forwards, sync, backup, import, X server). `ezssh_core::api::*` are the former commands as plain async fns on `&AppState`; output and async events go through `events::OutputSink` / `events::EventSink`. `src-tauri` is a thin adapter (generated `#[tauri::command]` wrappers, Tauri event mapping in `src-tauri/src/events.rs`, updater and splash). New backend code goes in `ezssh-core`, never in `src-tauri`. Details in §10 of the plan.
- The Tauri app (`src-tauri/`, `ui/`) stays the shipping build until the cut-over (phase 8). Do not touch `ui/` before then. It keeps the ezTerm product name, identifier and data paths so existing installs open the same database.
- Phase 3 (terminal widget parity) is built in `crates/ezssh-term`: find, links, context menu, overlays, OSC 52 policy, palettes, fonts. The widget returns `TermAction`s (Reconnect, ConfirmPaste, OpenUrl, Bell, OpenFontPicker) for the app to handle. `hook.rs` forwards vte's `Handler` to alacritty's `Term` and is generated; regenerate it on a vte upgrade. Details in §11 of the plan.
- Phase 4 (app shell) is built in `crates/ezssh-app` (binary `ezssh`, see its README): unlock, sidebar with drag and drop, tab strip, six view modes, status bar, toasts, settings, theme, `layout.json`. The phase 5 dialogs (session editor, credentials, import, backup, sync, …) are disabled entry points; create sessions in the Tauri app meanwhile. Details in §12 of the plan. `core.rs` runs `api::*` on a tokio runtime and hands results back as closures on `&mut App`; `conn.rs` wires terminals to connections.
- `cargo test -p ezterm -p ezssh-core -p ezssh-term -p ezssh-app` runs the Rust tests (`ezssh-term/tests/ui.rs` and `ezssh-app/src/ui_tests.rs` drive the widget and the whole app headless with egui_kittest; `EZSSH_UI_SHOTS=<dir>` renders PNGs); `cargo clippy --workspace --all-targets` must stay clean.
- Time-based app logic goes in `eframe::App::logic`, not `ui`: eframe skips `ui` while the window is hidden.

## UX Requirements (non-negotiable for v0.1)

- Layout mirrors MobaXterm: left **Sessions** sidebar (folders + saved sessions), main tabbed terminal area, optional **SFTP side-pane** docked on the left of the active tab.
- **Dark mode by default**, with a light theme toggle. Theme persists per user.
- **Terminal context menu** (right-click): Copy, Paste, Select All, Clear, Find.
- **Shift+Insert** pastes from clipboard into the terminal (MobaXterm / xterm convention).
- Selection-to-copy on mouse release is optional (setting, off by default) to avoid surprising new users.

## Stack

- **Shell**: [Tauri v2](https://tauri.app/) — Rust backend, webview frontend
- **Frontend**: Next.js (static export into Tauri assets) + [xterm.js](https://xtermjs.org/) with `fit`, `web-links`, and `search` addons
- **SSH / SFTP**: [`russh`](https://crates.io/crates/russh) + [`russh-sftp`](https://crates.io/crates/russh-sftp) (pure-Rust, tokio-based)
- **Persistence**: SQLite via [`sqlx`](https://crates.io/crates/sqlx) — sessions, folders, known_hosts, encrypted credential blobs
- **Crypto**: `argon2` (KDF from master password) + `chacha20poly1305` (AEAD for vault blobs); secrets wrapped in `zeroize`

## Planned Layout

```
ezTerm/
├── src-tauri/     # Rust: Tauri commands, SSH/SFTP, vault, DB
├── ui/            # Next.js app (static export)
├── migrations/    # sqlx migrations
└── Cargo.toml
```

## Architecture Notes

- **Command boundary**: All SSH/SFTP/vault operations live in Rust; the webview calls them via Tauri commands. The frontend is a dumb renderer plus UI state — no protocol logic in TypeScript.
- **Terminal data path**: Rust opens the SSH shell channel; output bytes flow to the frontend over a Tauri IPC channel passed into the connect command (raw bytes, buffered on the frontend until the keystroke handler is wired), close / error arrive as `ssh:close` / `ssh:error` events, and keystrokes flow back through a Tauri command. xterm.js only renders and captures input.
- **SFTP pane follows the session** (MobaXterm-style): when a tab connects, an optional side pane opens an SFTP subsystem on the *same* connection — no second auth.
- **Vault**: master password unlocks the app. Argon2id derives the vault key; credentials are stored as `(nonce, ciphertext)` rows. Plaintext secrets exist only in memory during use and are zeroized on drop. Never log, serialize, or return plaintext credentials to the frontend except as ephemeral auth material.
- **Known hosts**: TOFU on first connect; mismatch is a hard failure. Stored in SQLite, not `~/.ssh/known_hosts`.

## Commands

No code has been scaffolded yet. Once `src-tauri/` and `ui/` exist, the expected workflow is:

- `cargo check` — must pass before every commit (see global rules)
- `cargo test -p ezterm -p ezssh-core -p ezssh-term` — Rust unit/integration tests
- `cd ui && npm run lint` — frontend lint (fix properly, no ignores)
- `cd ui && npm test` — frontend unit tests (vitest)
- `cargo tauri dev` — run the app in dev mode
- `cargo tauri build` — produce a Windows installer
- `sqlx migrate add <name>` — create migration (use `date` for correct timestamp naming)
- `sqlx migrate run` — apply migrations (DATABASE_URL from `.env`)

Update this section once the actual scripts land.

## Conventions

- Bump `Cargo.toml` version before each commit (major=breaking, minor=features, patch=fixes).
- Fix warnings properly — remove unused code, don't `#[allow(...)]` past them.
- Async everywhere (tokio); no blocking calls on the Tauri command thread.
- Read struct definitions before writing code that uses them.

## Out of Scope for v0.1

Port forwarding, jump hosts / ProxyJump, X11 server, RDP / Telnet / Serial, macros, session recording, split panes, and theming beyond a default dark/light pair. Don't add these without an explicit design update.
