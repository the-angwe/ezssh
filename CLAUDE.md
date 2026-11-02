# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ezSsh is a free, open-source Windows SSH client that should **look and feel like MobaXterm**. It is a **permanent fork** of ezTerm (`github.com/ZerosAndOnesLLC/ezTerm`); nothing goes upstream. The fork lives at `github.com/the-angwe/ezssh` (git remote `fork`; `origin` is upstream, no push rights), app identifier `xyz.angwe.ezssh`. Feature set: connection/session manager, encrypted credential vault ("locker"), SSH, SFTP, WSL and local shells, port forwards, X11 forwarding, an xterm-compatible terminal.

Since 2.0 it is one native Rust process (egui/eframe 0.37 + alacritty_terminal 0.26); the Tauri + Next.js + xterm.js app is gone (removed in phase 8, 2026-11-02). Design, phases and results: `docs/design/native-rework-plan.md` (§9–§16).

## Compatibility with ezTerm data (do not break)

- The database is ezTerm's: `ezterm.sqlite` under `ProjectDirs("com", "ZerosAndOnes", "ezTerm")`; the per-user VcXsrv under `ProjectDirs("com", "zerosandones", "ezterm")`. Do not rename these.
- Backup magic `ezterm-backup`, sync object `ezterm-sync.json`, the vault verifier plaintext and the migration history are formats shared with ezTerm installs and old backups. Change them only with a migration path.
- New migrations make the database unopenable for ezTerm v1 (sqlx refuses unknown applied migrations). That is accepted; say so in release notes when it matters.
- **Session passwords belong to their session** (`credentials.session_id`, migration `20261101120000`): `SessionInput.password: Option<Secret>` sets or keeps it; the shared credentials list holds only private keys and key passphrases. Backups / sync carry the owner. Password sessions ignore `credential_id`; unowned password rows left by ezTerm are only listed (delete-only group in the credentials manager). §13.2 of the plan.

## Layout

```
crates/
  ezssh-core/     backend: DB, vault, SSH/SFTP, local PTY, WSL fs, forwards, sync, backup, import, X server, updater
  ezssh-term/     terminal widget: alacritty_terminal model, egui painter, key / mouse encoding, palettes, fonts
  ezssh-app/      the `ezssh` binary (see its README): window, sidebar, tabs, dialogs, file pane, packaging/
  ezssh-testsrv/  in-process SSH + SFTP + HTTP server for tests (and `cargo run -p ezssh-testsrv` by hand)
  ezssh-release/  update key, installer signatures, latest.json (release job)
migrations/       sqlx migrations
```

## Architecture Notes

- **Core API**: `ezssh_core::api::*` are plain async fns on `&AppState`; every one touching vault data calls `require_unlocked` first. Output and async events go through `events::OutputSink` / `events::EventSink`. New backend code goes in `ezssh-core`, never in the app.
- **Threading**: eframe owns the main thread; a tokio runtime owns every connection. `ezssh-app/src/core.rs`: `Core::call(what, |st| fut, |app, r| ..)` runs the continuation on the UI thread (drained in `App::logic`). `conn.rs` wires terminals to connections (per-attempt generations, reconnect).
- **Terminal data path**: the SSH / PTY reader task feeds bytes straight into the tab's `Term` and requests a repaint; the UI thread locks it only to paint, resize, select or search. The widget returns `TermAction`s (Reconnect, ConfirmPaste, OpenUrl, Bell, OpenAppearance) for the app to handle. `ezssh-term/src/hook.rs` forwards vte's `Handler` to alacritty's `Term` and is generated; regenerate it on a vte upgrade.
- **SFTP pane follows the session** (MobaXterm-style): it opens an SFTP subsystem on the *same* connection — no second auth. WSL tabs browse `\\wsl.localhost` through `wslfs`. Uploads and downloads are path-based (`api::sftp::sftp_upload_many` / `sftp_download_many`, one file at a time, cancellable). winit 0.30 has no Wayland file drops.
- **Vault**: master password unlocks the app. Argon2id derives the vault key; credentials are stored as `(nonce, ciphertext)` rows. Plaintext secrets exist only in memory during use and are zeroized on drop. Never log, serialize, or hand plaintext credentials to the UI except as ephemeral auth material.
- **Known hosts**: TOFU on first connect; a mismatch fails the connect and shows both fingerprints. Stored in SQLite, not `~/.ssh/known_hosts`.
- **Updater**: `ezssh_core::updater` checks `latest.json`, verifies the minisign signature while downloading (signed version required), installs per kind detected at run time (NSIS / MSI via a PowerShell helper, AppImage swap, deb via pkexec, `.app` swap; portable copies only link to the release). Public key `crates/ezssh-core/updater.pub` (empty until the user generates one, `docs/UPDATER_SIGNING.md`).
- **Packaging**: cargo-packager config in `[package.metadata.packager]` of `crates/ezssh-app/Cargo.toml` + `crates/ezssh-app/packaging/`; profile `dist`. `.github/workflows/release.yml` builds, signs and drafts releases (`docs/RELEASING.md`).
- Time-based app logic goes in `eframe::App::logic`, not `ui`: eframe skips `ui` while the window is hidden.

## UX Requirements

- Layout mirrors MobaXterm: left **Sessions** sidebar (folders + saved sessions), main tabbed terminal area, optional **SFTP side-pane** docked on the left of the active tab.
- **Dark mode by default**, with a light theme toggle. Theme persists per user.
- **Terminal context menu** (right-click): Copy, Paste, Select All, Clear, Find.
- **Shift+Insert** pastes from clipboard into the terminal (MobaXterm / xterm convention).
- Selection-to-copy on mouse release is optional (setting, off by default) to avoid surprising new users.
- Design tokens live in `crates/ezssh-app/src/style.rs`; shared controls in `widgets.rs` / `form.rs`.

## Commands

- `cargo check --workspace --all-targets` — must pass clean before every commit
- `cargo test --workspace` — all tests. `ezssh-term/tests/ui.rs` and `ezssh-app/src/ui_tests/` drive the widget and the whole app headless with egui_kittest; `EZSSH_UI_SHOTS=<dir>` renders PNGs. App UI tests are Unix only (local PTYs).
- `cargo clippy --workspace --all-targets -- -D warnings` — must stay clean
- `cargo run -p ezssh-app [-- --db PATH --renderer wgpu|warp|glow]` — run the app
- `cargo packager -p ezssh-app -f deb,appimage` (Linux) / `-f nsis,wix` (Windows) — installers into `target/packages`
- `sqlx migrate add <name>` — create a migration (use `date` for correct timestamp naming)
- `./build.sh` / `build.cmd` wrap the above.

## Conventions

- Bump the workspace `Cargo.toml` version before each commit (major=breaking, minor=features, patch=fixes); the release tag must equal it.
- Conventional Commits; no `Co-Authored-By` / "Made with" lines in commit messages (CONTRIBUTING.md).
- Fix warnings properly — remove unused code, don't `#[allow(...)]` past them.
- Async everywhere (tokio); no blocking calls on the UI thread or in async tasks (use `spawn_blocking`).
- Read struct definitions before writing code that uses them.
- Don't run `cargo fmt --all`; keep the existing formatting.
