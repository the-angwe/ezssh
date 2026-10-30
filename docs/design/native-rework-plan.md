# ezSsh native rework: dropping the webview

Status: phase 1 (spike) built 2026-10-30, results in §9; go decision
confirmed by hand (tmux, Claude Code). Phase 2 (core extraction) built
2026-10-30, §10. Phase 3 (terminal widget parity) built 2026-10-30, §11. Proposal written
2026-10-29 against ezTerm v1.14.0 (`feat/176-terminal-palettes` on top of
v1.13.1).

ezSsh is a permanent fork of ezTerm (github.com/ZerosAndOnesLLC/ezTerm),
decided 2026-10-30. The rework happens on the fork; nothing goes upstream.
The Tauri build keeps its ezTerm product name, identifier and data paths
until the cut-over (phase 8), so existing installs keep opening the same
database.

## 1. Goal and constraints

Replace Tauri v2 + Next.js/React + xterm.js with a single native Rust
process, keeping every user-visible feature listed in section 6. The Rust
side that already does the real work (russh, russh-sftp, vault, sqlx,
sync, backup, import, forwards, VcXsrv, WSL fs) is kept and promoted to a
plain library crate. Only the command layer, the window shell and the
frontend are rewritten.

Hard constraints taken from the current app:

- Windows first, must run on VMs and over RDP (Windows admins' daily
  reality), so a software rendering fallback is mandatory.
- GPLv3 app, so copyleft or dual licences are acceptable dependencies.
- The update channel (`latest.json` + minisign signatures, see
  `docs/UPDATER_SIGNING.md`) must keep working so v1.x installs can update
  into the native build.
- SQLite schema, vault format, backup archive and sync bundle formats are
  unchanged; the native build opens the same `ezterm.sqlite`.

Upstream note: settled. ezSsh is a permanent fork, so no upstream RFC is
needed (phase 0 is closed).

## 2. Library choice

### 2.1 Terminal emulation: `alacritty_terminal` 0.26

| Candidate | Verdict |
|---|---|
| **alacritty_terminal 0.26.0** (Apr 2026, crates.io) | **Chosen.** Grid + scrollback, selection (simple / semantic / lines), regex search, damage tracking, OSC 8 hyperlinks, OSC 52 via `Event::ClipboardStore`, title, bell, kitty keyboard flags in `TermMode`, bracketed paste, mouse modes, vi mode, truecolor. Pure Rust, no PTY coupling (we feed bytes from the SSH channel). Battle-tested as the embedded model in Zed, COSMIC Terminal, iced_term and egui_term. |
| wezterm-term / termwiz | Not on crates.io as `wezterm-term` (git dependency on the wezterm monorepo). Heavier (image protocols, its own cell model). The plan used `termwiz` 0.23 for `KeyCode::encode`; the spike found it implements only the xterm and CSI-u encodings (its `Kitty` variant is ignored) and pulls a large tree, so `ezssh-term` has its own encoder (§9.3). |
| libghostty-vt (ghostty-sys, libghostty-rs, ghosttea-vt) | Alpha C API, nothing tagged, needs the Zig toolchain in CI, several competing Rust wrappers. Revisit in a year. |
| vte 0.15 alone | Only the parser. We would be re-implementing the grid; no reason to when alacritty_terminal re-exports vte already. |

What alacritty_terminal does **not** provide and we write ourselves:
key-to-bytes encoding (our own, xterm + kitty), mouse-report encoding (SGR 1006, ~80
lines), the glyph renderer, URL detection for the web-links behaviour
(regex hints over the grid, as Alacritty does), the find overlay UI and the
copy / paste policy.

### 2.2 GUI: `egui` 0.37 + `eframe`

| Candidate | Verdict |
|---|---|
| **egui / eframe 0.37.0** (Oct 2026) | **Chosen.** See reasons below. |
| iced 0.14 | Best typography (cosmic-text) and a true software renderer (tiny-skia), and cosmic-term proves the terminal path. But no release since Dec 2025, no built-in context menus, no drag-and-drop helpers, no floating windows for the cascade mode, no accessibility tree, and the Elm architecture makes ~25 dialogs verbose. Too much hand-rolling for a parity port. |
| GPUI + gpui-component 0.8 | The richest widget kit (dock layout, virtual table, code editor, AccessKit) and Zed's own terminal as a reference. But crates.io `gpui` is stale (0.2.2, Oct 2025); gpui-component pins a Zed snapshot (`gpui-pre =0.3.10`), Windows needs a DirectX 11 GPU with only VM-emulated adapters as a fallback, and build times / binary size are Zed-scale. Dependency risk is the wrong trade for a small team. |
| Slint 1.18 | GPL option fits our licence and Windows support is excellent, but the `.slint` DSL has no path to a custom-painted terminal grid other than image buffers or path elements. No. |
| Dioxus native (Blitz), Xilem/Masonry, Floem | Blitz is still an HTML/CSS engine, which is what we are leaving. Xilem and Floem are too young (floem 0.2.0, Nov 2024). |

Why egui wins for this app specifically:

- **The typography blocker is gone.** 0.34 moved to skrifa + vello_cpu
  (hinting, variable fonts), 0.35 to harfrust shaping, 0.37 added colour
  emoji, `Context::add_font_provider` and the `egui_system_font_provider`
  crate (fontique) for system-font fallback. CJK, Nerd Font glyphs, box
  drawing and emoji in a terminal are covered; the font picker gets real
  system font enumeration.
- **The UI is dense tool UI, which is egui's home turf.** Context menus,
  `Modal`, drag-and-drop (`dnd_drag_source` / `dnd_drop_zone`), resizable
  tables (`egui_extras::TableBuilder`), collapsing trees, tooltips, popups
  and floating windows (`egui::Window` or `Area` for the cascade MDI) are
  built in. Every view mode in `mdi-area.tsx` maps to a few lines of
  layout code.
- **Immediate mode matches the current architecture.** The React tree is
  already "render from a zustand store"; egui is "render from a struct"
  every frame. Components port one-to-one as functions.
- **AccessKit built in**, with the 0.37 role / popup / TextEdit work.
  Neither iced nor GPUI-on-Linux offers this.
- **Rendering fallback on Windows.** eframe's wgpu backend lists the DX12
  WARP adapter ("Microsoft Basic Render Driver"); we select it when no
  hardware adapter is present so RDP / VM sessions work. glow (OpenGL)
  stays as the second backend behind a flag.
- **Multi-viewport** exists if we later want detachable tab windows.

Known egui costs, accepted:

- Repaint-on-demand only; terminal output calls `request_repaint`, cursor
  blink uses `request_repaint_after`. Idle CPU is near zero.
- Widgets look like egui unless styled. We port the design tokens from
  `globals.css` into an `egui::Style` (section 4.3) and draw tabs, rows
  and the status bar ourselves; that is where the MobaXterm look lives
  anyway.
- No OS drag *source* (drag-out to Explorer). Same limitation as today.

Supporting crates (all current on crates.io as of 2026-10-29):
`egui_extras` 0.37 (tables, SVG icons), `egui_system_font_provider` 0.37,
`egui_ltreeview` 0.10 (sidebar tree with DnD; fall back to hand-rolled if
its DnD rules do not match ours), `egui-notify` 0.23 (toasts),
`egui-phosphor` 0.14 or Lucide SVGs via egui_extras (icons), `egui_inbox`
0.14 (async results into the UI), `rfd` 0.17 (native file dialogs),
`arboard` 3.7 (clipboard reads; egui handles writes), `open` 5.4 (links),
`rodio` 0.23 (bell), `minisign-verify` 0.3
+ `self-replace` 1.5 or `cargo-packager-updater` 0.2 (updater),
`cargo-packager` 0.11 (NSIS / MSI / AppImage / deb / app bundles, the
Tauri bundler extracted for non-Tauri apps).

## 3. Target architecture

```
ezSsh/
├── Cargo.toml                 workspace
├── crates/
│   ├── ezssh-core/           everything now in src-tauri/src except commands/
│   │   ├── api/               the former commands, as plain async fns on AppState
│   │   ├── ssh/ sftp/ local/ wslfs/ vault/ db/ sync/ backup/ import/ xserver/
│   │   ├── updater/           latest.json fetch + minisign verify + install (new)
│   │   └── events.rs          enum Event { Output{cid,bytes}, Close{cid,..}, Transfer{..}, ForwardStatus{..}, UpdaterProgress{..} }
│   ├── ezssh-term/            terminal widget: alacritty_terminal model + egui painter + input encoding
│   └── ezssh-app/             eframe binary: AppState mirror, views/ (one module per current component), style.rs
├── migrations/                unchanged
└── site/                      unchanged
```

Threading model:

- eframe owns the main thread. A tokio multi-thread runtime runs on a
  background thread and owns every connection, exactly as today.
- `core::Handle` exposes `spawn(fut) -> Pending<T>`; results land in an
  `egui_inbox` and are drained at the top of `App::update`. Replaces
  `invoke()`.
- `core::events` is an `mpsc` of the enum above; replaces `emit`,
  `listen` and the Tauri `Channel`. Every send calls `ctx.request_repaint()`.
- Terminal bytes do not go through the event queue. Each connection holds
  `Arc<FairMutex<Term<Listener>>>` plus a `vte::ansi::Processor`; the SSH
  reader task calls `processor.advance(&mut term, bytes)` and requests a
  repaint. The UI thread only locks the term to paint, resize, select or
  search. This is Zed's arrangement and removes the IPC channel, the arm /
  detach buffering in `ssh.ts` and the stall watchdog (`terminal_output_sent`)
  entirely; they existed to work around webview IPC.
- `Listener` (alacritty `EventListener`) forwards: `Wakeup` → repaint;
  `Title` → tab title; `Bell` → bell policy; `PtyWrite` → SSH write (answers
  to DA / CPR / colour queries); `ClipboardStore` → clipboard, gated by
  `terminal.osc52Clipboard`; `ClipboardLoad` → ignored (reads stay refused,
  same policy as today); `ColorRequest` / `TextAreaSizeRequest` → answered
  from palette and cell metrics.

## 4. Component-by-component mapping

### 4.1 Terminal widget (`ezssh-term`)

| Today (xterm.js) | Native |
|---|---|
| `Terminal` options: cursorBlink, cursorStyle, fontFamily, fontSize, scrollback, theme | `TermConfig` into `alacritty_terminal::term::Config` (scrollback) + widget style (font, size, cursor, palette) |
| fit addon + ResizeObserver + rAF | `ui.available_rect()` → cols/rows from cell metrics each frame; `term.resize` + `ssh_resize` when changed |
| search addon + find overlay | `term::search::RegexSearch`, `search_next`; case toggle prefixes `(?i)`, regex toggle escapes; counter via a full scan capped at N matches |
| web-links addon | URL regex over visible lines (Alacritty's hint regex); underline on hover; `open::that` on click. OSC 8 hyperlinks from `cell.hyperlink()` |
| OSC 52 handler (`osc52.ts`) | alacritty parses OSC 52; our `Listener` enforces the policy. Port the 11 vitest cases to Rust tests on the listener (selector set, size cap, NUL strip, refused reads) |
| copy on select, right-click paste / menu, Shift+Insert, Ctrl+Shift+C/V/F | egui input handling in the widget; same bindings; `terminal.rightClick` setting honoured |
| paste via `term.paste()` (CRLF normalisation + bracketed paste) | own `paste()`: normalise `\r\n`/`\n` → `\r`, wrap in `ESC[200~ … ESC[201~` when `TermMode::BRACKETED_PASTE`; multi-line confirm modal when `warnMultilinePaste` |
| Ctrl+wheel zoom 8–48 | same, 40-unit accumulator, re-derive metrics and resize |
| bell none / visual / sound | visual: 120 ms inset ring painted over the widget; sound: rodio 880 Hz, 80 ms |
| Connecting / failed / disconnected overlay + Reconnect | painted overlay over the widget, same state machine as `terminal.tsx` |
| HostKeyDialog, XServerMissingDialog, AuthFixOverlay routing on error codes | unchanged routing, modals from `ezssh-app` |
| selection, Select All, Clear scrollback | `Selection::new(SelectionType::{Simple,Semantic,Lines})` on click / double / triple; `selection_to_string`; `term.grid_mut().clear_history()` |
| key encoding (xterm.js internal) | `ezssh_term::keys::encode` (xterm legacy + kitty progressive enhancement) with modes from `term.mode()`; `ezssh_term::mouse` for X10 / UTF-8 / SGR reports |
| rendering | per frame: `renderable_content()`; merge same-bg runs into rects, same-fg runs into one `painter.text` galley each; cursor by style; selection and search-match highlights from the palette; 4 px padding ring in palette bg |
| 20 palettes (`palettes.ts`) | `palettes.rs` with the same ids and `light` flag, tests ported |

Gate for this crate: vim, tmux, htop, Claude Code (kitty keyboard
protocol), `cat` of an 8 MB file, CJK + emoji + Nerd Font glyphs, box
drawing, mouse selection inside tmux, OSC 52 from neovim over SSH.

### 4.2 App shell (`ezssh-app`)

| Today | Native |
|---|---|
| `page.tsx` vault gate | `enum Screen { Splash, Unlock, Main }` in `App::update` |
| splash window (2 s, separate webview) | first frames of the main window show the logo with the 220 ms fade while the DB opens; no second window. The 2 s minimum is dropped unless upstream wants it kept (trivial to keep) |
| `main-shell.tsx` grid, sidebar 180–520 persisted, Ctrl+B, Ctrl+Shift+W, idle auto-lock, quit confirm | `SidePanel::left` with `resizable(true)` and range, `CentralPanel`, `TopBottomPanel` status bar; global shortcuts via `ctx.input`; idle clock from `ctx.input().time`; `on_close_event` → `ViewportCommand::CancelClose` + confirm modal |
| `tabs-shell.tsx` strip (reorder, middle-click, overflow scroll, dots, pane toggles) | hand-drawn tab strip in a horizontal `ScrollArea`; reorder with `dnd_drag_source` |
| `mdi-area.tsx` six view modes | one function: tabs → show active; tile-h/v/grid/auto → rect split (fixed grids need no tiling crate); cascade → `egui::Area` per tab with our own 24 px chrome, 8 resize handles, z order, minimize strip, maximize. Hidden tabs are not painted but their `Term` stays alive, so connections survive mode switches |
| `resize-handle.tsx` keyboard + double-click | egui panel resize for sidebar; custom `Sense::drag` strip for the SFTP pane with the same keyboard behaviour |
| `status-bar.tsx` | `TopBottomPanel::bottom`, 24 px |
| toasts | egui-notify with the four kinds and TTLs 4 / 6 / 8 / 4 s |
| theme dark / light persisted in `theme` setting | `Style` tokens (section 4.3), `ctx.set_style`; window decorations follow the app theme (egui 0.36) |
| `settings.ts` SPEC + store | `settings.rs`: same keys, kinds, defaults, bounds, parse / normalise / serialise, write-through with rollback; tests ported |
| `tabs-store.ts` | `TabsState` struct, same fields and actions; tests ported |
| localStorage layout keys | `layout.json` next to the DB (view mode, tile grid, minimized tabs, pane widths, columns, last update check). Still not roamed by backups |

### 4.3 Design tokens

`globals.css` → `style.rs`: bg, surface, surface-2, border, border-strong,
fg, muted, accent, success, warning, danger, selection for both themes;
radii 2–4 px (0 for tabs / rows); 24 px rows, 32 px tab bar, 24 px status
bar; Segoe UI Variable → system UI font via the font provider, Cascadia
Mono bundled for the terminal default. `dialog-in` / `overlay-in` become
`ctx.animate_value_with_time` (140 / 120 ms), honouring reduced motion.

### 4.4 Sidebar and dialogs

| Component | Native notes |
|---|---|
| `sessions-sidebar.tsx` | tree with folders first, badges, kind icons, rail colour; DnD zones (50/50 for sessions, 25/50/25 for folders, whole row into-folder, background = root); folder / session / root context menus with the same items; header buttons. Use `egui_ltreeview` if its drop-zone rules can be configured as above, else hand-roll on `CollapsingHeader` + `dnd_*` |
| `session-dialog.tsx` (4 tabs, 1138 lines) | `Modal` with a tab bar; same fields; shell dropdown from `list_local_shells`; six colour swatches; env editor; forwards list in edit mode; Esc / Ctrl+Enter |
| `settings-dialog.tsx` | `Modal`, five sections, write-through; palette combo with live swatch |
| `credentials-dialog.tsx`, `credential-picker.tsx`, `credential-events.ts` | same grouping; the picker refreshes from a shared `Rc<RefCell<Vec<Credential>>>` instead of a DOM event |
| `forwards-pane.tsx`, `forward-dialog.tsx` | 280 px right pane; status colours; context menu; validation warnings |
| `sftp-pane.tsx` family | `TableBuilder` with resizable Name / Size and fixed Mode columns, horizontal scroll; multi-select (click / Ctrl / Shift); breadcrumb; context menu; `rfd` save / folder dialogs; OS drop via `raw.dropped_files` (directories arrive as paths, so recursive upload is a plain `walkdir` with `mkdir`, no `webkitGetAsEntry`); streaming via `upload_begin/chunk/finish` 1 MB chunks; two progress strips |
| `sync-dialog.tsx` | kind radio, S3 fields, 3 s status poll (now a `request_repaint_after`), Push / Pull → temp → Restore |
| `backup-dialog.tsx`, `restore-dialog.tsx` | same three-step and checkbox tree flows |
| `import-mobaxterm-dialog.tsx` | same |
| `unlock-screen.tsx`, `recovery-*`, `change-password`, `reset-vault` | same, including Esc gating; strength bar from `password_strength.rs`; "reload after password change" becomes a state reset |
| `update-dialog.tsx` | same phases driven by `Event::UpdaterProgress`; relaunch = spawn `current_exe()` then exit |
| `host-key`, `xserver-missing`, `auth-fix`, `find-overlay`, `font-picker`, `tile-grid`, `move-to-folder`, `confirm`, `prompt`, `context-menu` | small modals / popups; font picker lists OS presets plus system fonts from the font provider |

### 4.5 Core changes (`ezssh-core`)

- `commands/*.rs` → `api/*.rs`: drop `#[tauri::command]`, `AppHandle`,
  `Window`, `Channel`; take `&AppState` and an `EventSink`. Error type and
  codes unchanged so the UI routing survives.
- `ssh_connect` / `local_connect` return a `ConnectionHandle { term,
  writer }` instead of pushing to a channel.
- `updater/`: fetch `latest.json` (same URL, same platform keys), verify
  the minisign signature over the artifact with the existing public key,
  install: Windows runs the NSIS / MSI installer passive, Linux AppImage
  replaces itself (`self-replace`), deb prompts, macOS swaps the `.app`.
  `cargo-packager-updater` implements exactly this flow and reads the same
  manifest shape; use it if its signature format matches ours in the spike,
  otherwise ~300 lines of our own.
- `splash`, `platform`, `terminal_output_sent`: removed.
- Everything else (vault, db, sync, backup, import, forwards, xserver,
  wslfs, local shells) compiles unchanged apart from import paths.

## 5. Phases

Each phase ends on `main`-mergeable state; the Tauri app keeps working
until phase 7 because phase 1 makes the core a library both shells use.

| # | Phase | Output | Rough size |
|---|---|---|---|
| 0 | ~~RFC upstream~~ | Closed 2026-10-30: permanent fork, renamed ezSsh | — |
| 1 | Spike | `ezssh-term` prototype over a local PTY and one SSH session in a bare eframe window (`ezssh-spike`). Pass the gate in 4.1 plus: WARP fallback on a Windows VM over RDP, IME input, 60 fps on `cat` of 8 MB, binary size. Go / no-go. **Built 2026-10-30, §9** | 1 week |
| 2 | Core extraction | `crates/ezssh-core` without Tauri types; `src-tauri` becomes a thin adapter; all Rust tests green; no behaviour change. **Built 2026-10-30, §10** | 1 week |
| 3 | Terminal widget | full 4.1 parity, Rust tests for osc52 policy, palettes, paste normalisation, key encoding table. **Built 2026-10-30, §11** | 1–2 weeks |
| 4 | Shell | unlock, main shell, sidebar + DnD + menus, tab strip, six view modes incl. cascade, status bar, toasts, settings, theme, layout persistence | 2–3 weeks |
| 5 | Dialogs | session, credentials, forwards, host key, auth fix, xserver, import, backup, restore, sync, recovery, change password, reset, update, find, font picker | 2 weeks |
| 6 | SFTP pane | table, multi-select, drops, streaming uploads, downloads, progress | 1 week |
| 7 | Packaging and updater | cargo-packager configs (NSIS + MSI with VcXsrv resources and the uninstall hook, AppImage, deb, .app), portable tar.xz, `latest.json` publish job rewritten, minisign signing kept, CI matrix, Linux runtime deps drop from webkit2gtk to libxkbcommon / wayland / X11 libs | 1 week |
| 8 | Cut-over | delete `ui/`, `src-tauri/`, Tauri deps; README, CLAUDE.md, CONTRIBUTING, site docs; release v2.0.0 through the v1 update channel | days |

Total: roughly 10–12 working weeks for one person, less with the current
pace of AI-assisted porting since most of it is mechanical translation of
components whose behaviour is already specified by the TS source and
tests.

## 6. Parity checklist

Every item below must be demonstrably present before cut-over. Source of
truth: README, release notes v0.12 – v1.13.1, and the component inventory
of 2026-10-29.

Connections: SSH password / key / agent; TOFU host keys and mismatch hard
fail; WSL tabs with distro + user + start dir; local shells (cmd /
powershell / pwsh / `/etc/shells`, `-i`, `-NoLogo`); X11 forwarding with
bundled or installed VcXsrv, install-from-app; L / R / D forwards saved or
ad-hoc with live status, conflict detection, restart on edit, auto-start;
MobaXterm import with duplicate strategy and key files into the vault;
auto-seeded WSL and Local Shells folders.

Terminal: 16 / 256 / truecolor; copy / paste / select all / clear /
find (case, regex, counter, Enter / Shift+Enter); Shift+Insert,
Ctrl+Shift+C/V/F; Ctrl+wheel zoom 8–48 with PTY resize; OSC 52 writes
only; per-session font, size, scrollback, cursor style, env, keepalive,
timeout, initial command, compression; 20 palettes applied live; bell
none / visual / sound; copy on select; right-click menu or paste;
multi-line paste warning; connecting / failed / disconnected overlays with
exit status and signal names; auth-fix overlay; link hover and open.

Workspace: sidebar tree with folders, DnD, badges, colours, rail,
collapse (Ctrl+B), resizable 180–520; tab strip with dots, colours,
middle-click close, reorder, overflow scroll, pane toggles; six view modes
(Ctrl+Shift+W) with cascade minimize / maximize / restore; SFTP pane with
breadcrumb, resizable columns, multi-select, context menu, OS drop
including directories, streaming upload, multi-download, progress, WSL
browsing; forwards pane; status bar with user@host, cwd, X pill, version,
theme, settings, update check, lock; toasts; dark / light.

Settings: every key in `settings.ts` with its bounds; idle auto-lock;
confirm close / quit; auto-open SFTP; tab label mode; update channel,
cadence and launch check.

Security and data: Argon2id + ChaCha20-Poly1305 vault, zeroise; three
credential kinds with rename / rotate / delete-with-usage; change master
password with snapshot; recovery code generate / unlock; forgot-password
reset; known hosts in SQLite; redacted logging; encrypted backup with
selective restore; local-folder and S3 sync with ETag conflicts; signed
auto-update with pre-release channel.

## 7. Risks and open questions

- **Typography is the one real technical risk** and phase 1 exists to
  retire it. If egui 0.37's font provider does not cover Nerd Fonts and
  CJK fallback inside a monospace grid to our satisfaction, the fallback
  plan is iced 0.14 + a cosmic-term-style renderer, accepting the extra UI
  work. Decide at the end of phase 1, not later.
- **Accessibility regressions**: the webview gave screen readers a DOM.
  egui's AccessKit covers widgets; the terminal grid needs an explicit
  AccessKit node with the visible text (Zed does the same).
- **Drag-out to Explorer** stays unsupported (no winit drag source).
- **Splash window**: dropping the separate always-on-top splash changes
  first-launch feel; cheap to keep as a second viewport if upstream wants.
- **Binary size**: eframe + wgpu + fonts lands around 15–25 MB per
  platform versus the current webview-backed binary; acceptable for a
  portable tarball, worth measuring in phase 1.
- **Linux packaging**: AppImage / deb no longer need webkit2gtk, which was
  the main runtime dependency complaint; new deps are libxkbcommon,
  libwayland, libX11 / xcb, fontconfig.
- **Upstream appetite**: resolved by forking permanently (ezSsh).

## 8. Sources consulted (2026-10-29)

crates.io metadata for every crate named above; egui release notes 0.35 –
0.37; iced releases page (0.14.0 latest); gpui-component README and crate
dependencies; Zed Windows docs (DirectX 11 requirement); alacritty_terminal
0.26 docs (`Event`, `Term`); termwiz `KeyCode::encode`; COSMIC Terminal
architecture notes; egui_term 0.1.0 (egui 0.31, Apr 2025) and iced_term
0.8.0 (Mar 2026) dependency pins.

## 9. Phase 1 results (2026-10-30)

### 9.1 What was built

Branch `native/phase-1-spike`, two new workspace crates next to the
untouched Tauri app (`src-tauri` still builds; its 98 tests pass):

- `crates/ezssh-term` (≈2 600 lines incl. tests): `terminal.rs` (shared
  `Term` + VT parser, `Feeder` for byte sources, `Pty` trait for input,
  alacritty `EventListener` with the OSC 52 policy and query answers),
  `view.rs` (egui widget: painter, keyboard, mouse, selection, IME, bell,
  exit overlay), `keys.rs` (xterm + kitty key encoder), `mouse.rs` (X10 /
  UTF-8 / SGR reports), `boxdraw.rs` (built-in box drawing, blocks,
  Powerline), `palette.rs` (ezterm-dark), `fonts.rs` (named system font,
  regular + bold, via fontique). 31 unit tests.
- `crates/ezssh-spike` (≈1 200 lines): bare eframe window with tabs, local
  PTY backend (portable-pty), one-session russh backend (agent → key files →
  password / keyboard-interactive, `known_hosts` check), wgpu adapter
  selector with software fallback, `--bench`, `--run`, `--screenshot`.
  See `crates/ezssh-spike/README.md`.

### 9.2 Measurements

Linux, GNOME on Wayland, Intel Arc iGPU (Meteor Lake), 120 Hz panel, 5/3
scale, 1100×700 pt window (151×39 cells at 12 pt, 129×34 at 14 pt),
JetBrains Mono. Frame numbers are from runs with the window visible; eframe
paints nothing while a Wayland window is hidden (§9.3).

| Test | wgpu Vulkan (iGPU) | wgpu llvmpipe (software, WARP stand-in) | glow (OpenGL) |
|---|---|---|---|
| `--bench` 8 MiB generated coloured text, pass 1 (cold) | 172 ms, 70 fps, worst paint 68 ms | 177 ms, 39 fps | 159 ms, 69 fps |
| `--bench` passes 2–3 (warm) | 85 ms (≈94 MiB/s), 117–128 fps, paint p50/p95 0.8/1.3 ms | 101–106 ms, 66–70 fps, paint ≈1.0–1.3 ms | 87–90 ms, 114–122 fps, paint 1.5/1.7 ms |
| `cat` of an 8 MiB file through the local PTY, first run | < 1 s, 98 fps, worst frame 74 ms | < 1 s, 85 fps, worst 81 ms | — |
| same, second run (fonts warm) | 120 fps, worst frame 10.6 ms | 107 fps, worst 23 ms | — |
| Idle CPU, shell prompt, 5 s | 0 ticks (after the IME fix below; 9 % of a core before) | | |

- The cold-pass outliers (≈70 ms, one frame) are the first CJK glyph: the
  system font fallback lookup and loading the Noto CJK collection. One-off
  per script per run.
- Parser alone (window hidden, so no painting): 60–85 MiB/s while a release
  build ran in parallel. The PTY, not the terminal, limits `cat`.
- Startup: system font enumeration 16 ms, named font load 15–25 ms.
- Binary size (Linux x86-64, both wgpu and glow backends, russh, tokio):
  39.8 MB release, 30.0 MB stripped, **22.9 MB** with fat LTO, one
  codegen unit, `panic = "abort"` and strip. Inside the 15–25 MB guess in
  §7. Phase 7 picks the release profile; `panic = "abort"` would make a
  panicking reader thread take the whole app down, so it may stay off at a
  small size cost. Dropping glow or unused wgpu backends is the next lever.

### 9.3 Findings and deviations from the plan

1. **termwiz dropped.** `KeyCode::encode` has no kitty encoding. `keys.rs`
   implements xterm legacy encoding and kitty progressive enhancement
   (disambiguate, event types, alternate keys, all-keys-as-escapes,
   associated text); Shift+Enter → `CSI 13;2u` is a test case. 12 tests.
2. **egui-winit 0.37 IME feedback loop on Wayland.** egui-winit re-sends the
   IME cursor area on every frame that had input; GNOME / IBus answers each
   with an empty pre-edit event; that event causes the next frame. Result: a
   constant 120 fps repaint while idle. `view::ImeLoopFilter`, called from
   `App::raw_input_hook`, drops empty pre-edits outside a composition. Worth
   an upstream egui issue.
3. **eframe 0.37 runs only `App::logic` while a window is hidden or
   occluded** (`App::ui` is skipped). Terminal parsing is unaffected (it
   runs on reader threads and catches up on the next visible frame), but
   anything time-based in the app shell (idle auto-lock, update checks,
   toasts expiring, the spike's automation) must live in `logic()`.
4. **Clipboard shortcuts.** eframe turns Ctrl+C/V/X into copy / paste
   events by default, which would swallow ^C. `NativeOptions::
   clipboard_shortcuts = false`; the widget handles Ctrl+Shift+C/V,
   Shift+Insert, Ctrl+Insert and right-click paste itself and reads the
   clipboard with arboard. On GNOME Wayland arboard falls back to the X11
   clipboard (no data-control protocol); it works through XWayland.
5. **Box drawing needs pixel-exact geometry.** egui feathers every filled
   rect; at fractional scale the overlapping half-transparent edges showed
   as ticks along lines, and per-cell stroke widths flipped between 1 and
   2 px where the nominal width sat on .5 px. Box drawing and cell
   backgrounds now go into un-feathered meshes in whole device pixels, with
   one stroke width per frame. Tested.
6. **Ligatures are on.** egui 0.37 shapes runs with harfrust, so JetBrains
   Mono draws `->`, `!=`, `===` as ligatures. Grid alignment holds (the
   shaped run keeps the advance). Phase 3 needs a way to turn `calt` off
   (egui font tweak / feature settings, to be checked) or defaults to a
   ligature-free face (Cascadia Mono, JetBrains Mono NL).
7. **No Nerd Font on this machine**, so Nerd Font icons render as tofu; the
   four Powerline separators are drawn by `boxdraw.rs`. Decision for phase 3:
   bundle "Symbols Nerd Font Mono" (MIT / OFL) as a fallback family.
8. **No italic face yet**: italic text renders upright. Phase 3 loads italic
   and bold-italic.
9. **Emoji**: colour emoji render (egui 0.37 colour fonts); one emoji
   (U+1F600) came out monochrome from an earlier fallback font. Flags render
   as letter pairs and skin-tone modifiers as separate cells, as in most
   terminals.
10. **Colour queries** (OSC 4 / 10 / 11 with `?`) are answered from the
   palette by the listener, which cannot lock the `Term`, so colours a
   program changed with OSC 4 are not reflected. Phase 3 moves the answer.
11. **AltGr on Windows (risk, untested).** egui-winit drops text events
   while Ctrl is held; Windows reports AltGr as Ctrl+Alt, so characters
   like `@` on a German layout may not arrive. Must be checked on Windows
   (gate item 8 in the spike README); the fix would be in our raw input
   hook or a winit-level key handler.
12. **Wayland window decorations** come from sctk-adwaita (client-side);
   fine for the spike, revisit with the app shell's theme.

### 9.4 Gate status

| Gate item (§4.1 and phase 1) | Status | Evidence |
|---|---|---|
| vim | rendering verified (vim.tiny: alternate screen, file view); editing needs a person | screenshot |
| tmux | not installed here; GNU screen renders correctly | screenshot; manual |
| htop | not installed here; `top` renders correctly (inverse header, bold rows) | screenshot; manual |
| Claude Code (kitty keyboard) | protocol push / pop / query answered (`CSI ? 1 u` after `CSI > 1 u`); encoder unit-tested | probe screenshot; manual |
| `cat` of 8 MB at 60 fps | **pass**: 98–120 fps (GPU), 85–107 fps (software) | §9.2 |
| CJK + emoji + Nerd Font glyphs | CJK **pass** (wide cells aligned); emoji mostly; Nerd icons need a bundled font (item 7) | screenshot |
| box drawing | **pass**: single, heavy, double, rounded, blocks, shades, quadrants, Powerline | screenshots at 12 and 14 pt |
| mouse selection inside tmux | needs a person | manual |
| OSC 52 from neovim over SSH | policy unit-tested (write, NUL strip, size cap, read refused, setting off) | manual for live |
| WARP fallback on a Windows VM over RDP | adapter selector built and logged; software path verified with llvmpipe | needs Windows VM |
| IME | events and cursor area wired; idle loop fixed | needs IBus / fcitx / Windows IME |
| binary size | **pass**: 22.9 MB with LTO + strip (30 MB plain stripped) | measured |
| live SSH session | connect error path verified; no sshd on this machine | needs a host |

### 9.5 Recommendation

**Go**, provisionally, for egui 0.37 + alacritty_terminal 0.26. The
typography risk named in §7 did not materialise on Linux: CJK fallback,
colour emoji and wide-cell alignment work through egui's own font stack,
and throughput is bound by the PTY and vsync, not by the widget. The
remaining unknowns are Windows-specific (WARP over RDP, AltGr, the Windows
IME) and should be run before phase 3 starts in earnest; phase 2 (core
extraction) does not depend on them and can start now.

## 10. Phase 2 results (2026-10-30)

Branch `native/phase-2-core` (from `native/phase-1-spike`).

### 10.1 Layout

- `crates/ezssh-core` holds everything that was in `src-tauri/src` except
  the window shell: `db`, `vault`, `ssh` (client, forwards, known hosts),
  `sftp`, `wslfs`, `local`, `sync`, `backup`, `import`, `xserver`,
  `error`, `log_redacted`, `state`. Files were moved with `git mv`; only
  the four that carried Tauri types changed (`ssh/client.rs`,
  `local/spawn.rs`, `sftp/transfer.rs`, `wslfs/transfer.rs`).
- `ezssh_core::api::*` are the former commands as plain async functions,
  `fn(state: &AppState, <same params>) -> Result<same type>`. Same names
  and parameters as the `invoke()` surface, so the native shell can port
  each frontend call one-to-one.
- `src-tauri` is the adapter: 85 generated one-line `#[tauri::command]`
  wrappers, plus the Tauri-only commands that stay there until the
  cut-over: `ui_ready` (splash window), `terminal_output_sent` (webview
  stall watchdog) and `updater_check` / `updater_download_install`
  (tauri-plugin-updater; the pending `Update` is its own managed state
  now). A script check compared all 89 commands against the pre-move
  source: identical frontend parameters, return types and registration.

### 10.2 The seam

- `events::OutputSink` (one per connection, `send(&[u8]) -> bool`)
  replaces the Tauri IPC `Channel`. Tauri wraps the channel
  (`src-tauri/src/output.rs`); the native shell will feed `ezssh-term`'s
  `Term` directly from it.
- `events::EventSink` + `enum Event { Closed { connection_id, reason },
  WriteFailed, ForwardStatus, Transfer }` replaces `AppHandle::emit`.
  `AppState` holds the sink (`state.events`), so no API function takes an
  app handle. `CloseReason` is `Exit(u32) | Signal(name) | Ended`.
- `src-tauri/src/events.rs` maps events back onto the historic names and
  payloads (`ssh:close:{id}` with the exit code / `"killed by SIG…"` /
  null, `ssh:error:{id}`, `forwards:status:{id}`,
  `sftp:transfer:{tid}`); unit tests pin each payload, including the
  hand-built auto-start failure shape. The app handle is attached in
  `setup`, because `AppState` is still built before the Tauri app so the
  runtime arrangement does not change.
- `events::channel(wake)` is the sink the native app will use: a std
  queue drained once per frame, `wake` = `ctx.request_repaint()`.
- `AppState::open(path, events)` opens the DB, runs migrations and sets
  the vault state (what `main` used to do inline);
  `state::default_db_path()` keeps the ezTerm data path.

### 10.3 Verification

- `cargo test -p ezterm -p ezssh-core`: 98 moved tests + 2 event-queue
  tests + 4 wire-format tests + 1 new end-to-end test
  (`crates/ezssh-core/tests/api_local.rs`: fresh DB → vault init →
  local-shell session → `local_connect` → write → output and
  `Closed { Ended }` arrive → reopen sees a locked vault). Unix only;
  there is still no SSH server here for an SSH round trip.
- `cargo clippy --workspace --all-targets` is clean. The backend carried 24
  clippy warnings before; `clippy --fix` took the mechanical ones and the
  rest were fixed by hand (a `TcpipDispatch` type alias, a `Driver` struct
  for the SSH driver task's arguments). CI now runs check / clippy / test
  on `-p ezterm -p ezssh-core`, clippy with `-D warnings`.
- The Tauri binary starts, opens a fresh database through the new path
  and runs without errors (12 s smoke run with a temporary data dir).
  The webview flows (connect, SFTP, forwards) were not clicked through.

### 10.4 Notes for the next phases

- SSH write failures are now logged at warn as well as emitted.
- Phase 4 wires `ezssh-core` into a new `ezssh-app` crate: one
  `AppState` on a background tokio runtime, `api::*` calls through a
  pending-result inbox, `events::channel` drained in `App::logic`.

## 11. Phase 3 results (2026-10-30)

Same branch as phase 2 (`native/phase-2-core`); phase 3 touches only
`crates/ezssh-term` and `crates/ezssh-spike`, so the two are separable by
path.

### 11.1 What the widget does now (§4.1 rows)

| §4.1 row | Native, as built |
|---|---|
| Terminal options | `TermOptions { scrollback, osc52_write, cursor: CursorKind, cursor_blink }` (model); `TermStyle { palette, copy_on_select, right_click, warn_multiline_paste, bell, padding, builtin_box_drawing, ligatures }` (app-wide); per tab `set_font_size` / `set_faces`. Live: `set_palette`, `set_osc52_write`, `set_cursor` |
| fit + resize | unchanged from phase 1 (cols / rows from cell metrics each frame) |
| search + find overlay | `search.rs`: plain or regex, case toggle (inline `(?i)` / `(?-i)` over alacritty's smart case), the most recent 1 000 matches counted (`n/1000+` past that), current match = selection (Ctrl+Shift+C copies it), others tinted; Enter / Shift+Enter / Esc; recounted on every query change and step |
| web links | `links.rs`: xterm.js web-links regex (http/https only, same trailing-punctuation rule), OSC 8 hyperlinks; underline + hand cursor on hover; a click without drag returns `TermAction::OpenUrl`. Inactive while a program has mouse reporting on (Shift overrides) |
| OSC 52 | `osc52.rs` port of `osc52.ts` with its 11 tests, through `hook.rs` (below): lenient base64 (unpadded, whitespace), 4 MiB cap checked before decoding, NUL stripped, reads refused, live on/off |
| copy / paste / shortcuts | Ctrl+Shift+C/V/F, Shift+Insert, Ctrl+Insert; right-click menu (Copy, Paste, Select All, Find…, Font…, Clear scrollback) or paste per `terminal.rightClick`, Ctrl+right-click always the menu |
| paste | `Terminal::paste` (CR normalisation, bracketed, end marker stripped); multi-line pastes return `TermAction::ConfirmPaste { text, lines }` while `warn_multiline_paste` is on (the app asks, then calls `paste`) |
| Ctrl+wheel zoom | per tab, 40 px per step, one wheel notch = 100 px (WebView2's value, so a notch is 2–3 pt as before), 8–48 |
| bell | `BellMode::Visual` = 120 ms inset ring in the palette's yellow; `Sound` returns `TermAction::Bell` (the app plays it; no audio dependency in the widget) |
| overlays | `TerminalView::overlay(Overlay::{Connecting, Failed, Closed})` paints the dimmed terminal and the card; Reconnect returns `TermAction::Reconnect`; the terminal takes no input under an overlay. The app owns the state machine (phase 4) |
| selection, Select All, Clear | `select_all()`; `clear_scrollback()` = xterm.js `clear()` (history dropped, cursor line becomes the first line) |
| key encoding | unchanged encoder; a 42-row table test against xterm.js's legacy encodings passes as is |
| rendering | italic and bold-italic faces; cursor shape from the session plus DECSCUSR, 600 ms blink that stops after 5 min idle, hollow when unfocused |
| palettes | all 17 from `palettes.ts`, same ids, labels say ezSsh; a test parses `palettes.ts` and compares every colour (drop it with `ui/` in phase 8) |

### 11.2 Findings

1. **alacritty's OSC 52 decoder is strict** (padded base64 only), so
   unpadded payloads from Rust / Go programs would be lost. `hook.rs` wraps
   `Term` in a `Handler` that overrides `clipboard_store` and
   `dynamic_color_sequence` and forwards the other 69 methods. The
   forwarding block is generated from vte 0.15's trait (every method
   defaults to a no-op, so a hand-written list could silently drop one);
   regenerate on a vte upgrade.
2. **Colour queries (§9.3 item 10) fixed** by the same hook: answered with
   the `Term`'s live colours (OSC 4/10/11 changes included), in order with
   other replies (a colour query followed by DA1 gets the colour first;
   tested).
3. **No OpenType feature control in egui 0.37** (harfrust is called without
   features), so ligatures cannot be switched off in the shaper. Instead,
   with `ligatures = false` (default) a run of adjacent ASCII punctuation is
   painted cell by cell. Cost on a 201×61 screen of dense code: paint p50
   0.66 ms vs 0.39 ms with ligatures on.
4. **Fonts follow the webview's stack** instead of bundling Cascadia Mono
   (§4.3): the session value is parsed as a CSS-like list, then `Cascadia
   Mono, Menlo, DejaVu Sans Mono, Consolas` (SF Mono first on macOS), then
   fontconfig's generic monospace. Only "Symbols Nerd Font Mono" (nerd-fonts
   v3.4.0, MIT, 2.5 MB) is bundled, as the last face of every terminal
   family. Faces that exist only by synthesis count as missing (egui cannot
   embolden or slant).
5. **egui 0.37 ships no fonts by default**; tests enable `default_fonts`
   as a dev-dependency feature. Its bundled UI font lacks arrows and ⚠, so
   the find bar says Prev / Next and the overlay uses colour, not icons.
6. **Popups are egui `Area`s**: their first frame is an invisible sizing
   pass. With the spike window hidden on Wayland, a forced screenshot frame
   is that sizing pass and no further frame comes, so popups were verified
   headless instead (below).
7. A press and its release usually land in different frames; link hover
   is kept while a press that started on a link is held, else a click never
   opened it (found by the headless test).

### 11.3 Verification

- `cargo test -p ezssh-term`: 69 unit tests (osc52 11, palettes 7, search
  4, links 4, terminal 15, keys 12, view 6, fonts 3, boxdraw 5, mouse 2) and 9
  headless UI tests (`tests/ui.rs`, egui_kittest: context menu items and
  actions, Clear scrollback, right-click paste mode, find bar counter /
  Enter / Shift+Enter / case / Esc, overlays and Reconnect, input blocked
  under an overlay, multi-line paste confirmation, typing, Ctrl+wheel,
  link click). `EZSSH_UI_SHOTS=<dir>` renders the states to PNG with wgpu;
  the menu, find bar and overlay cards were checked that way.
- Spike screenshots (window hidden): italic / bold-italic, ligatures split,
  Nerd Font icons and Powerline glyphs, CJK + emoji fallback, box drawing,
  Campbell palette, find highlights. `cargo clippy --workspace --all-targets`
  clean.
- Not verified here: everything in the spike README's manual checklist,
  plus item 10 (the phase 3 flows by hand), Windows, AltGr, IME.

### 11.4 Left for later phases

- Font picker contents (system font list via fontique) are phase 5; the
  widget only returns `OpenFontPicker`. The spike has a minimal size /
  family window.
- The bell sound (rodio or a platform beep) belongs to the app (phase 4).
- AccessKit node with the visible text for screen readers (§7) is not
  done.
- Find match points go stale while output scrolls; they are recounted on
  the next keystroke in the find bar.

