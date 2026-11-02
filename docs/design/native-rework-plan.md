# ezSsh native rework: dropping the webview

Status: phase 1 (spike) built 2026-10-30, results in §9; go decision
confirmed by hand (tmux, Claude Code). Phase 2 (core extraction) built
2026-10-30, §10. Phase 3 (terminal widget parity) built 2026-10-30, §11.
Phase 4 (app shell, `crates/ezssh-app`) built 2026-11-01, §12. Phase 5
(dialogs; passwords moved onto sessions) built 2026-11-01, §13. Phase 6
(file pane) built 2026-11-02, §14. Phase 7 (packaging, updater, update
dialog) built 2026-11-02, §15. Phase 8 (cut-over: webview app, site and
spike removed, version 2.0.0-beta0) done 2026-11-02, §16. Proposal written
2026-10-29 against ezTerm v1.14.0 (`feat/176-terminal-palettes` on top of
v1.13.1).

ezSsh is a permanent fork of ezTerm (github.com/ZerosAndOnesLLC/ezTerm),
decided 2026-10-30. The rework happens on the fork; nothing goes upstream.
The native build keeps ezTerm's data paths, so it opens an existing ezTerm
database. Paths into `ui/`, `src-tauri/` and `site/` below refer to the
tree before phase 8 (`git show 66d1be8:<path>`).

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
| 4 | Shell | unlock, main shell, sidebar + DnD + menus, tab strip, six view modes incl. cascade, status bar, toasts, settings, theme, layout persistence. **Built 2026-11-01, §12** | 2–3 weeks |
| 5 | Dialogs | session, credentials, forwards, host key, auth fix, xserver, import, backup, restore, sync, recovery, change password, reset, find, font picker; passwords bound to sessions. **Built 2026-11-01, §13** (update dialog moved to phase 7) | 2 weeks |
| 6 | SFTP pane | table, multi-select, drops, streaming uploads, downloads, progress. **Built 2026-11-02, §14** | 1 week |
| 7 | Packaging and updater | update dialog (from phase 5), cargo-packager configs (NSIS + MSI with VcXsrv resources and the uninstall hook, AppImage, deb, .app), portable tar.xz, `latest.json` publish job rewritten, minisign signing kept, CI matrix, Linux runtime deps drop from webkit2gtk to libxkbcommon / wayland / X11 libs **Built 2026-11-02, §15** | 1 week |
| 8 | Cut-over | delete `ui/`, `src-tauri/`, Tauri deps; README, CLAUDE.md, CONTRIBUTING, site docs; release v2.0.0 through the v1 update channel. **Done 2026-11-02, §16**: site and spike removed too; version 2.0.0-beta0; v1 installs cannot be updated (upstream key) | days |

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
| copy / paste / shortcuts | Ctrl+Shift+C/V/F, Shift+Insert, Ctrl+Insert; right-click menu (Copy, Paste, Select All, Find…, Appearance…, Clear scrollback) or paste per `terminal.rightClick`, Ctrl+right-click always the menu |
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
  widget only returns `OpenFontPicker` (renamed `OpenAppearance` on 2026-11-02, see §13). The spike has a minimal size /
  family window.
- The bell sound (rodio or a platform beep) belongs to the app (phase 4).
- AccessKit node with the visible text for screen readers (§7) is not
  done.
- Find match points go stale while output scrolls; they are recounted on
  the next keystroke in the find bar.


## 12. Phase 4 results (2026-11-01)

Branch `native/phase-4-shell` (from `native/phase-2-core` after the user
committed phases 2 and 3). New crate `crates/ezssh-app`, binary `ezssh`;
its README has the file map, the run commands and a manual checklist.
Also touched: `ezssh-term` (`TerminalView::input`, `palette::Rgb`
re-export), `ezssh-core` (`HostKeyUntrusted` carries the fingerprint),
CI and the build scripts.

### 12.1 What the shell does (§4.2 rows)

| §4.2 row | As built |
|---|---|
| vault gate | `Screen::{Unlock, Main}`; the database opens and the vault state and theme are read before the first frame (≈0.25 s to the unlock screen, debug build), so there is no splash. Create (with the strength meter) and unlock work; the recovery / forgot links are shown disabled (phase 5). |
| main shell | sidebar 180–520 px with its own resize strip (drag, double-click reset, ←/→ / Home / End when focused, keeps 320 px + the active tab's panes free), Ctrl+B and the 24 px rail; idle auto-lock (`security.idleLockMinutes`, checked in `App::logic`); quit confirmation through `ViewportCommand::CancelClose`; WSL and local-shell autodetect with the same toasts on every unlock. |
| tab strip | natural width 180–260 px, proportional shrink to 180 then scroll (wheel, chevrons, drag auto-scroll, active tab kept in view); drag reorder with the insertion line and ±24 px release band; middle-click close; colour dot, status dot (pulsing while connecting, alert icon on error), file / forwards pane toggles, tooltip with summary, status and error. |
| six view modes | one function (`views/mdi.rs`): tabs, tile H / V with 1 px `border-strong` gaps, tile grid (rows × cols dialog, 160 px minimum rows, extra rows and scrolling past that), auto (squarest grid), cascade (24 px chrome, drag by title, eight resize handles, double-click maximise, minimise strip, press raises the topmost frame). Hidden tabs are not painted; their terminals keep running. |
| status bar | dot + summary in the UI mono font, SFTP cwd, X server pill (polled every 5 s), version, theme / settings / lock; updates button disabled (phase 5 / 7). |
| toasts | own implementation (four kinds, 4 / 6 / 8 / 4 s, bottom-right, dismiss), expiry in `App::logic`. |
| theme | `style.rs` tokens from `globals.css`, applied to egui's visuals; views read the tokens directly so rows, badges and tabs match their Tailwind originals. The window decorations follow through `ctx.set_theme`. |
| settings | `settings.rs` is `settings.ts` (21 keys, same bounds, parse / normalise, write-through with rollback); the dialog has the five sections, number fields commit on Enter / blur, palette combo with the live swatch. Palette and OSC 52 apply to open tabs at once. |
| tabs store | `tabs.rs` with the same actions and rules (open collapses the sidebar, close picks the last tab, `request_close` confirms live tabs, reorder contract, staircase cascade, maximise / restore). |
| layout persistence | `layout.json` next to the database: view mode, tile grid, sidebar width, SFTP pane width and columns. Minimized tabs are not kept (tabs do not survive a restart). |
| sidebar (§4.4) | tree with folders first, count badges, kind tiles in the session colour, selection rail, trash on hover; drag and drop with the webview's slot rules (sessions 50/50, folder drags 25/50/25, a session over a folder is always "into", whitespace = root, self-drops refused) and its exact move / reorder calls (`plan_drop`, unit-tested); auto-scroll; Escape cancels. Folder, session and root context menus with every item; New folder, Rename, Delete, Duplicate, Move to folder…, Sort A→Z / Z→A and Settings… work, the phase 5 items are disabled with a note. |
| connections (§3) | `conn.rs`: the core feeds the terminal directly; one generation number per connect attempt replaces arm / detach; input and resizes queue per attempt and are sent in order once the connection id is known (query answers during the handshake are kept); the terminal outlives connections, so Reconnect keeps the scrollback as before. Connect waits for the first layout (≤300 ms) so the remote starts at the right size. Host-key prompt (untrusted / mismatch) pulled forward from phase 5. Auth and X server errors show the Failed card until phase 5 adds their dialogs. |

### 12.2 Findings and deviations

1. **The untrusted-host prompt never showed a fingerprint** in the webview
   either: `AppError::HostKeyUntrusted` carried none and the UI read an
   absent `actual`. The variant is now `HostKeyUntrusted { actual }`,
   serialized as `actual` like the mismatch case, which also fixes the
   Tauri dialog. Test in `error.rs`.
2. **Lock keeps the tabs.** The webview unmounted the workspace on lock,
   which dropped every connection and reconnected after unlock, although
   the Settings hint says "Open connections stay up". Native keeps tabs
   and connections behind the unlock screen.
3. Local shell summaries read `@/bin/sh:0` (every kind was formatted as
   SSH); now SSH `user@host[:port]`, WSL `user@distro`, local the
   program. `appearance.tabLabel = user@host` leaves local shells on
   their name, as its hint says.
4. **egui's keyboard focus is not modal-aware**: a focused terminal kept
   reading keys typed into a dialog. `TerminalView::input(false)` (new)
   blocks input the way an overlay does; the app passes it while any
   dialog is open.
5. egui 0.37 renames met here: `Id::new` → `Id::unique` (global ids) /
   `IdSalt`, `Panel::show_inside` → `show`, `Ui::id` → `scope_id`,
   `TextEdit::horizontal_align` → `align`. `Panel`s are resizable and draw
   a separator by default; both are off where the app paints its own
   border and resize strip (the default handle swallowed the strip's
   drag; the test caught it). An `Area` offers the whole screen as
   available width, so menus size themselves from their widest label.
6. Icons: the 45 Lucide icons the shell uses, written as SVG from the
   webview's own `lucide-react` 1.57.0 by `tools/gen_icons.mjs` (white
   stroke, tinted per use, rendered by egui_extras / resvg). `Trash2` is
   an alias of `trash` in that version. egui-phosphor still targets egui
   0.36; egui-lucide was too young to depend on.
7. UI font: the system UI face through fontique (Segoe UI Variable on
   Windows, fontconfig's system-ui / sans-serif elsewhere; Noto Sans
   here) plus its semibold face when it has a real one. `font-mono` text
   uses the terminal's default font.
8. Bell sound without an audio library: Windows `Beep(880, 80)`; a
   generated 880 Hz WAV through pw-play / paplay / aplay (Linux) or afplay
   (macOS); rate-limited to one per 100 ms.
9. Ctrl+B / Ctrl+Shift+W act only while no terminal has focus. That is
   what the webview did in practice (xterm.js swallowed both) and keeps
   ^B for tmux.
10. Accessibility: every hand-painted control (rows, tabs, toggles, icon
    buttons, menu items, the resize strip) reports a role and label to
    AccessKit. The terminal grid itself still has no text node (§11.4).
11. Headless testing with egui_kittest: its 0.25 s default step makes two
    clicks too slow for a double-click (tests use 60 Hz steps); its
    `MissingGlyphPolicy::Panic` caught "→" missing from the UI font,
    covered in the real window by eframe's system font fallback, which
    the tests now install too.

### 12.3 Verification

- `cargo test -p ezssh-app`: 45 unit tests (settings incl. the vitest
  cases, layout, tabs store and cascade geometry, drop planning, tab
  widths and drop targets, connection plumbing, toasts, password
  strength, icons, bell) and 6 end-to-end tests in `src/ui_tests.rs` on
  the real `App` with a temp database: create the vault → seeded Local
  Shells → open a local shell and run a command through the PTY → lock
  (connection kept, wrong password refused) → unlock; tile H / V / grid /
  auto / cascade, minimise and restore, title-bar drag, Ctrl+Shift+W;
  New folder, drag a session into it, root menu items, close
  confirmation; settings write-through to SQLite and the theme toggle;
  sidebar resize saved to `layout.json`; host-key dialog Cancel,
  Reconnect, multi-line paste confirmation, idle lock.
  `EZSSH_UI_SHOTS=<dir>` renders 19 screens; all were looked at.
- Whole workspace: `cargo test -p ezterm -p ezssh-core -p ezssh-term
  -p ezssh-app` green, `cargo clippy --workspace --all-targets` clean.
  CI and `build.sh` / `build.cmd` now check, lint and test `ezssh-term`
  and `ezssh-app` too (with the xkb / xcb headers eframe needs).
- `ezssh --db <temp>` opens a real window (wgpu, Intel Arc) to the
  unlock screen without errors.
- Release binary 43 MB stripped, no LTO (the spike was 30 MB; the rest is
  the backend: sqlx, russh, rust-s3, reqwest, plus resvg for the icons).
  LTO and size work belong to phase 7 (the spike went 30 → 22.9 MB with
  fat LTO).
- Not verified here: SSH sessions (no sshd), the window close prompt
  (needs a real close request), Windows. The README checklist covers
  them.

### 12.4 Left for later phases

- Phase 5: session editor, credentials, import, backup / restore, sync,
  password / recovery / reset dialogs, update dialog, auth-fix overlay,
  X server install, font picker, forwards pane. Their entry points exist
  and are disabled.
- Phase 6: the SFTP pane (placeholder of the right width, resizable
  width setting already in `layout.json`).
- Dialog entry animations (`dialog-in`) are not ported.

## 13. Phase 5 results (2026-11-01)

Same branch (`native/phase-4-shell`, after the user committed phase 4).
Every dialog of §4.4 except the update dialog is ported into
`crates/ezssh-app/src/views/`, and session passwords moved from the
shared credentials list onto their sessions (§13.2), as the user asked.

### 13.1 What was built

| §4.4 row | As built |
|---|---|
| `session-dialog.tsx` | `session_dialog.rs`: 760 px modal, side tabs General / Terminal / Advanced / Forwards (SSH), kind and auth radio cards, local-shell dropdown from `list_local_shells` + custom path, font dropdown of the installed monospace families + custom family (`font_presets.rs`; `lib/fonts.ts` fixed list only as fallback), colour swatches, env editor, the TSX validation order and copy; Esc closes, Ctrl+Enter saves unless a dialog is stacked on top. Opened from the toolbar, the empty state, folder menus (New Session) and session menus (Edit). |
| `credential-picker.tsx` | `cred_picker.rs`: private keys and key passphrases only (no password picker any more), inline "Add new", "Manage credentials" stacks the manager; reads the shared `app.creds` cache (`creds.rs`), which replaces `credential-events.ts`: whoever changes a credential reloads it, every picker sees the new list next frame and clamps a vanished selection. |
| `credentials-dialog.tsx` | `credentials.rs`: groups SSH keys / Key passphrases with add, rename, rotate, delete with the list of sessions using it; a note that session passwords live on their session; leftover unowned password rows (only the Tauri picker makes them) show delete-only. |
| `auth-fix-overlay.tsx` | `auth_fix.rs`: painted over the failed tab's terminal (`TermSlot::auth_fix`, set by `conn.rs` on an SSH auth failure); username, method, password or key; saves (env kept), updates the tab's session copy, reconnects. "Close tab" closes the tab, as in the TSX. |
| `forwards-pane.tsx`, `forward-dialog.tsx` | `forwards.rs`: the 280 px pane with saved and tab-only forwards, live status from `Event::ForwardStatus` (routed by `conn.rs`), start / stop / edit / delete, context menu, the four dialog modes, validation and warnings; also the session dialog's Forwards tab (`forwards::session_tab`). |
| `change-password`, `recovery-*`, `reset-vault` | `vault_dialogs.rs` + `unlock.rs`: change password (strength meter, snapshot path on success; the webview's page reload becomes "close all tabs and lock", since the core already locked the vault and closed every connection, and unlocking again reloads everything), recovery code generate / replace / copy, recovery unlock and reset-vault (type `DELETE`) on the unlock screen. |
| `font-picker-popover.tsx` | Replaced on 2026-11-02 by `appearance.rs`: the terminal / tab-header menu's Appearance… opens a dialog with the session dialog's appearance fields (tab colour, font family from `ezssh_term::fonts::monospace_families` + Custom…, size 8–48, scrollback, cursor style). Colour, font and cursor preview on the tab (scrollback waits, shrinking drops lines); Cancel / Escape restore; "Apply to tab" keeps them for this tab only (`Tab::font_family` / `Tab::color` + the terminal); "Save to session" writes the row and `App::apply_session_terminal` updates every open tab of the session, colour included. |
| `xserver-missing-dialog.tsx` | `xserver.rs`: opened by `conn.rs` on `XServerMissing`; install bundled VcXsrv (Windows) then retry, or continue without X11; no install button off Windows. |
| `backup-dialog.tsx`, `restore-dialog.tsx` | `backup.rs`, `restore.rs`: reauth → passphrase → native save dialog; restore picks a file (or starts from a sync pull), passphrase, checkbox tree with All / None, "password included" badge on password sessions, key-not-selected warning for key sessions, settings toggle, renamed list. |
| `sync-dialog.tsx` | `sync.rs`: local / S3 cards, status polled every 3 s while open (`request_repaint_after`), enable / update & push, push now, disable, pull → temp file → restore dialog. |
| `import-mobaxterm-dialog.tsx` | `import.rs`: picks the file, preview stats, new folders, duplicate strategy, session list, commit, toasts as in the TSX. |
| `update-dialog.tsx` | **Moved to phase 7.** It drives the updater, which is still Tauri-only (`tauri-plugin-updater`); the dialog lands with the native updater. The status-bar button stays disabled with `UPDATER_HINT`. |

Shared pieces: `form.rs` (Section / Field / IconSlot, `.input`, secret
field with eye toggle, textarea, select, checkbox, radio cards, side tabs,
`<kbd>`, `labelled` for AccessKit names), `files.rs` (native open / save /
folder dialogs through rfd 0.17 on the core runtime; 0.17 rather than the
Tauri plugin's 0.16 because 0.16's `gtk3` and `xdg-portal` features cannot
be unified in one workspace build, and the native app should not depend
on GTK), 27 more Lucide icons.

### 13.2 Passwords belong to their session

Before: a password was a row in the shared `credentials` table, picked
from a list in the session dialog and possibly shared by several
sessions. Now:

- `credentials.session_id` (new, `REFERENCES sessions ON DELETE CASCADE`,
  unique where set) marks a session's own password. `credential_list` /
  `credential_list_detailed` return shared rows only (private keys, key
  passphrases); update / delete of an owned row is `NotFound`.
- `SessionInput.password: Option<Secret>` (`secret.rs`: zeroized,
  `Debug` prints `***`, deserialize only). For a `password` session a
  non-empty value is encrypted and stored or replaced in the same
  transaction as the session row (`db::sessions::{create_with,
  update_with, PasswordOp, bind_password}`); empty or absent keeps the
  stored one. Switching to key / agent auth deletes it; duplicate copies
  it; delete removes it (explicitly, not only through the cascade, as
  `vault::reset` does). Key and passphrase slots refuse owned rows.
- Migration `20261101120000_session_owned_passwords`: every session that
  used a shared password gets its own copy (same ciphertext; the lowest
  session id keeps the original row), stale references from non-password
  sessions are cleared, and password rows no session uses are dropped
  (the app could neither show nor attach them).
- Backups and sync bundles carry `session_id` on credential entries
  (absent in old files, `serde(default)`); the preview lists only shared
  credentials and marks password sessions `has_password`; restore gives
  each restored password session its own copy, also from old files where
  one row served several sessions. The bundle version is unchanged.
- The Tauri app keeps working without touching `ui/`. Its `credential_list`
  command now returns the shared rows plus every session password
  (`credential_list_with_session_passwords`, labelled with the session's
  name), because the webview picker drops a selection it cannot find.
  Saving a session that names an unowned password row (just created by
  the picker) claims it, replacing the session's old row; naming another
  session's password copies it into the session's own row.
  MobaXterm re-import that flips a password session to key auth drops
  the old password.
- Key passphrases stay shared on purpose: a passphrase belongs to its
  key, and one key usually serves many sessions.

### 13.3 Findings

1. egui 0.37's `TextEdit::desired_width` includes the margins; the first
   `form.rs` subtracted them again (every field 16–44 px short).
2. Inside a `Modal`, a `ScrollArea` never grows past last frame's
   height; `backup.rs`'s shell lays out plainly while the body fits and
   scrolls at a fixed height once it does not.
3. A disabled `widgets::primary` button senses hover only and was
   reported to AccessKit as enabled; it now reports the real state, also
   during a modal's first (sizing) frame.
4. Toasts paint above modals and can cover a footer button for a few
   seconds (tests clear them before clicking there).
5. Several errors the TSX swallowed (`.catch(() => {})`) are now shown
   and logged: shell detection, env load, forward list / delete.
6. The core reports a wrong backup passphrase or recovery code as
   `BadPassword` ("incorrect master password"); the dialogs translate it.

### 13.4 Verification

- `cargo test -p ezssh-core`: 103 unit tests (one new: the migration on
  data shaped like an older build's) + `tests/session_passwords.rs`
  (lifecycle, the webview's create / edit / rotate / pick-another flows,
  backup round trip, old bundle with a shared password).
- `cargo test -p ezssh-app`: 88 tests, 22 of them end to end in
  `src/ui_tests/` (now a directory: `mod.rs` = the phase 4 tests and
  helpers, plus `sessions.rs`, `credentials.rs`, `vault.rs`, `data.rs`).
  They drive every dialog on a temp database, check the database after
  saves (owned password rows, restored passwords bound to the restored
  session), and render 88 screens with `EZSSH_UI_SHOTS`.
- `cargo clippy --workspace --all-targets` clean; `ezterm` tests green.
- An independent review of the merged change found, and this phase
  fixed: the webview could not save an edit of a password session (its
  picker lost the own row, and claiming a new row hit the unique index);
  an auth-fix overlay blocked the keyboard of every tab (now only its
  own); a second tab missing the X server replaced the first one's
  dialog (now queued, and an install reconnects all of them); lock kept
  auth-fix overlays (typed secrets) alive; restore / import were not
  modal while the native file dialog was open and late results could
  land in a newer dialog (instance serials, a waiting modal); a sync pull
  finishing after a lock popped a restore dialog; dead tabs after a
  master-password change or reset are closed in the success handler;
  auth-fix pickers of two tabs shared ids; secret fields now drop egui's
  undo snapshots every frame (`form::forget_history`; freed, not
  zeroized, since egui edits a plain `String`).
- Not verified here: native file dialogs (the portal; tests inject the
  path), SSH logins (no sshd), VcXsrv install (Windows), S3. README
  checklist.

### 13.5 Left for later

- Update dialog: phase 7.

Fixed after the phase (2026-11-01):

- One dialog card, `widgets::Dialog` (header none / title strip / full
  header with ✕, inline or strip footer, a body that scrolls once it no
  longer fits; `Dialog::data` for backup / restore / sync / import),
  replaces `widgets::dialog`'s body, `vault_dialogs::shell` and
  `backup::shell`. The generic card lost egui's default gap above its
  footer strip (the TSX has none).
- One bordered button, `widgets::Ghost` (icon, `compact`, `quiet`,
  colour, disabled, AccessKit label, tooltip), replaces
  `vault_dialogs::ghost_button`, `backup::outline` / `cancel_button`,
  `forwards::ghost_icon` and `cred_picker::ghost_button`;
  `vault_dialogs::primary_button` was `widgets::primary_with_icon`;
  `widgets::button_row` and `widgets::IconBtn` moved in too.
- `form::option_cards` lays out a row of radio cards at the height of
  the tallest.
- "Last sync" shows local time (chrono, already in the workspace).
- The font lists show the installed monospace families
  (`ezssh_term::fonts::monospace_families`) instead of a fixed list.

## 14. Phase 6 results (2026-11-02)

Branch `native/phase-6-sftp` (from `native/phase-4-shell` after the
user's commit 9c72495). The file pane of SSH and WSL tabs is ported into
`crates/ezssh-app/src/views/sftp.rs`; the placeholder in `mdi.rs` is gone.

### 14.1 What was built (§4.4 `sftp-pane.tsx` row)

| Piece | As built |
|---|---|
| Opening | First draw on a connected tab: `sftp_open`, `realpath(".")`, list home in one core call. A reconnect (new connection id) reopens on the new connection in the folder it showed, home if that is gone (the webview kept its `opened` flag and listed on the dead connection). Not connected / connecting / failed (with Retry) states. |
| Breadcrumb | Root button + one link per segment, mono 11 px, last highlighted, horizontal scroll kept at its end. |
| Table | Sticky NAME / SIZE / MODE header; Name grows to fill, Size and Name resizable (drag, double-click resets, keyboard steps, `layout.json`), Mode fixed 72 px; sideways scroll when the columns need more than the pane; only visible rows are laid out (large folders). Icons Folder / FileSymlink / File, `formatSize` and `rwxrwxrwx` as in the TSX. |
| Selection | Click replaces (inside a selection it keeps it), Ctrl toggles, Shift extends from the anchor; cleared when another folder is listed; right-click on an unselected row selects it first (the webview did not, so its menu could act on a row that looked unselected). |
| Menus | Row: Download… / Download N files… (selected files only), Rename…, Delete. New: right-click on the list background → Upload files…, Upload folder…, New directory…, Refresh (Upload folder replaces drops where the desktop has none, see 14.2). |
| Uploads | Header button (native multi-file dialog), drops, menu. All go through the path-based core transfers; there is no 16 GB guardrail and no 1 MB chunk pipeline (`sftp_upload_begin/chunk/finish` stay for the Tauri build). A dropped folder is walked off the UI thread (sorted, parents first, folder symlinks skipped, unreadable entries counted as failed), its folders are created (an existing one is fine, re-uploading a tree over itself works) and its files queued in one batch. |
| Downloads | One file: native save dialog. Several: folder dialog; each remote name is turned into one safe local path component (`local_name`: a remote `..\..\x` cannot leave the folder on Windows, reserved names and characters are replaced). |
| Progress | One strip for uploads and downloads (`Event::Transfer`, now routed in `conn.rs`): `…` until the first event, percent + bar, `✓ done` 1.5 s, `✗ failed` 4 s with a toast (the webview's `TransferStatus` hid failed transfers silently). Folder jobs: `upload tree/ 3/12`, then `done (12 files)` / `done (2 failed)` with ×; their files show only while they send. At most six transfer rows, then "N more waiting". Progress that arrives before its ticket is registered is kept and applied. |
| Refresh | After mkdir / rename / delete, after uploads into the shown folder settle (once, not per file), when the pane is shown again, on the button. A failed listing keeps the folder it had and shows the error strip (×), instead of the old entries under the new breadcrumb. |
| Prompts | `PromptState` gained `set_error`: mkdir / rename stay open with the server's error; names with `/`, `.` or `..` are refused in place (the webview joined `a/b` into a nested path). |
| Pane width | 4 px handle on the right edge, bounds `SFTP_PANE_MIN..min(SFTP_PANE_MAX, room left for a 320 px terminal and the forwards pane)`. |

Core changes (`ezssh-core`):

- `api::sftp::sftp_upload_many` / `sftp_download_many(items)`: one ticket
  per file, run one after another in one task. One task per file would
  open every local file at once while they queue for the session lock (a
  dropped tree of thousands of files would hit the descriptor limit).
- `sftp::transfer::{upload, download}` and the WSL versions sent no final
  event when the local file could not be opened / created, so a UI waited
  forever; `emit_failed` now ends every transfer. Detached transfer
  failures are logged (`let _ =` before).
- `SftpEntry: Clone`.

### 14.2 Findings

1. winit 0.30.13 has no file drag and drop on Wayland at all (only
   Windows, macOS, X11), and on every platform reports no pointer
   position during an OS drag (`HoveredFile` / `DroppedFile` only). So a
   drop goes to the visible pane's folder (panes are docked in tabs mode
   only, so there is at most one), per-folder drop targets of the webview
   are not possible, and a drop with no visible pane toasts where to drop.
   On GNOME Wayland, `WAYLAND_DISPLAY=` runs the app under XWayland with
   drops working; the list menu covers uploads either way.
2. Drag-out to the OS file manager stays unsupported (no winit drag
   source), as §7 expected; Download… is the path.
3. Transfers of one connection serialise on the SFTP session lock, and a
   listing waits behind a running transfer. That is the core's existing
   behaviour; the pane shows its spinner meanwhile. A second SFTP channel
   for listings would lift it.
4. egui 0.37 takes held modifiers only from `Event::ModifiersChanged`;
   tests that Ctrl / Shift-click must send it (the modifiers on
   `PointerButton` are not used for `InputState::modifiers`).
5. egui's multi-pass frames hand `dropped_files` to the first pass only,
   so a drop cannot upload twice.
6. russh calls `server::Handler::data` for subsystem channels too, so the
   test server's echo shell answers only its shell channels (the SFTP
   client got its own INIT back otherwise).

### 14.3 Test server

`crates/ezssh-testsrv`: russh 0.64 server with password auth for one
user, an echo shell and the `sftp` subsystem over a local folder
(`LocalFs`: open / read / write / stat / lstat / fstat / setstat /
opendir / readdir / mkdir / rmdir / remove / rename / realpath; `..` stops
at the root), optional per-op delay. In process for tests, or
`cargo run -p ezssh-testsrv -- --root DIR` by hand. It is the first SSH
server the tests have (the workstation has no sshd), so SSH connect,
password auth and host-key TOFU are now exercised end to end too.

### 14.4 Verification

- `ezssh-core/tests/sftp_api.rs`: connect with a password, open, realpath,
  list order, mkdir, upload / download of 300 kB with progress, batches,
  transfers that cannot start or fail on the server end with an error
  event, rename, rmdir refused on a non-empty folder, remove, the browser
  goes with the connection.
- `ezssh-app/src/views/sftp.rs` unit tests: sizes, modes, remote paths,
  name validation, `local_name` stays one path component, selection rules,
  the folder walk (order, folder symlink skipped).
- `ezssh-app/src/ui_tests/sftp.rs`: the real app against the test server:
  host-key prompt, pane from the tab strip, click / Ctrl / Shift
  selection, the multi-download menu entry, downloads, double-click and
  breadcrumb navigation, drop overlay, drops of a file and a folder tree
  (remote contents checked), a 40-file folder (rows collapse into the
  job), 12 plain files ("more waiting"), a 3 MB upload with its bar,
  mkdir with an invalid then a valid name, rename and delete from the row
  menu, delete of a non-empty folder fails with a toast, an unreadable
  folder keeps the listing and shows the error, reconnect reopens in the
  same folder, hide / show lists again. 14 screens with
  `EZSSH_UI_SHOTS`.
- `cargo test -p ezterm -p ezssh-core -p ezssh-term -p ezssh-app -p ezssh-testsrv`
  green, `cargo clippy --workspace --all-targets` clean. CI and the build
  scripts include `ezssh-testsrv`.
- Not verified here: OS drops from a real file manager (Wayland here), the
  native save / folder / multi-file dialogs (portal), WSL browsing
  (Windows), a real OpenSSH server. README checklist item 16.

### 14.5 Left for later

- Per-folder drop targets and drag-out, both blocked on winit (14.2).
- chmod (the core has `sftp_chmod`; the webview had no dialog either);
  recursive folder delete.
- A second SFTP channel so listings do not wait behind transfers.

### 14.6 Added after the phase (2026-11-02)

The user asked for three of the 14.5 items.

- **Folder download.** Folders get Download… (folder dialog; the folder
  is recreated inside the chosen one), and a multi-selection with folders
  offers "Download N items…". Core `api::sftp::sftp_walk(root)` flattens
  a remote folder depth first (folders parents first, files with relative
  paths, empty folders kept, symlinked folders not followed but listed as
  files, unlistable sub-folders reported and skipped, an unlistable root
  is an error, at most 200 000 entries). Every path component goes
  through `local_name`; local folders are created first, and files whose
  folder could not be created count as failed. A folder is a job in the
  strip (`download proj/ 2/3`), like a dropped folder.
- **Multi-selection delete.** With more than one row selected the row
  menu offers "Delete N files" / "Delete N items", one confirmation
  ("Folders must be empty on the remote server."), then each entry is
  removed in turn; failures do not stop the rest and end in one toast
  ("Deleted 1 of 2", first error). A single folder still needs to be
  empty, as in the webview.
- **Cancel.** Core: `TransferRegistry` (`state.transfers`) holds a cancel
  flag per path-based transfer from ticket to end, so a queued file can
  be cancelled before its turn; the transfer loops (SFTP and WSL) check
  it before every 32 KiB chunk and after waiting for the session lock.
  `api::sftp::sftp_transfer_cancel(id)` sets it (false once ended). A
  cancelled upload removes its partial remote file, a cancelled download
  its partial local file. `TransferProgress.cancelled` marks the final
  event (no error, no failure toast); it is left out of the JSON when
  false, so the webview's payload is unchanged. `sftp_upload` /
  `sftp_download` now run through the batch path. App: a × on every
  running or queued transfer row ("cancelling…" until the final event),
  on a running folder job (cancels what is left of it: sent files stay),
  and on the "N more waiting" line (cancels those). A cancelled job ends
  as `cancelled (2/5)`.

Tests: `ezssh-core/tests/sftp_api.rs::cancel_and_walk` (walk order and
errors, cancel of a running upload and download with partial files gone,
cancel of a queued batch member while the next one still runs);
`ezssh-app/src/ui_tests/sftp.rs::folder_download_multi_delete_and_cancel`
(folder download with nested and empty folders, delete of 3 selected
items, a partial failure, cancel of a download, an upload, a folder job
and the waiting queue; run four times for timing). 4 more screens.


## 15. Phase 7 results (2026-11-02)

Branch `native/phase-6-sftp` (phase 6 committed as "rework phase 6" and
follow-ups). The native app updates itself, is packaged by cargo-packager
and has its own release workflow; the update dialog of phase 5 is in.

### 15.1 Updater (`ezssh-core::updater`, §4.5)

`cargo-packager-updater` 0.2 was read and not used: it has no deb
support, requires a `format` field in every manifest entry (not in the
v1 `latest.json`), ignores the signed version, and panics when the
Windows installer cannot be started. Own code instead (~700 lines with
tests), same manifest and signature format as `tauri-plugin-updater`:

| Step | As built |
|---|---|
| Check | stable: `github.com/<repo>/releases/latest/download/latest.json`; pre-release: the newest published release of either kind that has a `latest.json` asset (the Tauri code took the newest non-draft release even when it had none). 404 → "no release with an update manifest is published yet". Versions compared as semver. Static (`platforms`) and dynamic (`url` + `signature`) manifests. `EZSSH_UPDATE_MANIFEST=URL` reads another manifest (manual tests); the signature check still applies. |
| Which installer | `updater::detect()` at start-up: `portable.txt` next to the binary → portable; Linux `$APPIMAGE` → AppImage, `/usr/…` + `/var/lib/dpkg/info/ezssh.list` → deb; macOS `*.app/Contents/MacOS` → app; Windows `uninstall.exe` next to the exe → NSIS, a `target` folder → unpackaged, else MSI. Keys tried: `{os}-{arch}-{kind}`, then the bare `{os}-{arch}` only for the platform's default kind (a deb never falls back to the AppImage behind the bare key, the failure behind upstream issue #36). Portable and unpackaged copies are told about the release and get "Open releases page". |
| Download | streamed to a fresh `ezssh-update-<pid>-<ms>` temp folder while minisign's stream verifier hashes it; 60 s idle timeout, 1 GiB cap, short bodies fail; a file that fails is deleted. Before any byte is fetched: the signature must be by the compiled-in key and its trusted comment must name the announced version (`version:` field; the Tauri plugin made that optional). `Event::UpdateProgress` at most every 100 ms. |
| Install | NSIS / MSI: a hidden PowerShell helper (`-EncodedCommand`) waits for the app's pid to exit, runs `setup.exe /P` or `msiexec /i … /passive /norestart`, logs the exit code to `%TEMP%\ezssh-update.log`, deletes the download folder (only a folder named `ezssh-update-*`) and starts the app with its arguments; the app exits. AppImage: copy next to the old file, same mode (+x), fsync, rename over it (atomic; the Tauri plugin wrote in place). deb: `pkexec dpkg -i`, with the `sudo dpkg -i` line in the error when the prompt is cancelled. `.app`: unpack next to the bundle, swap by renames, put the old one back on failure. |
| Restart | "Installed v… Restarting" for 800 ms, then the new program is spawned with the same arguments and the window closes without the quit prompt. |
| Key | `crates/ezssh-core/updater.pub`, compiled in, empty for now: such a build says it has no update key. The fork has no signing key yet and upstream's (in `tauri.conf.json`) is not ours; `docs/UPDATER_SIGNING.md` has the one-time setup. Repo defaults to `the-angwe/ezssh`, `EZSSH_UPDATE_REPO` at build time overrides. |

### 15.2 Release tool (`crates/ezssh-release`)

`keygen` (encrypted minisign key, prints the `updater.pub` line), `sign`
(one base64 `.sig` per file, trusted comment
`timestamp:…\tfile:…\tversion:…`), `verify` (against a public key file
and version; the release job uses it to catch a secret that does not
match `updater.pub`), `manifest` (`latest.json` from cargo-packager's file
names, refuses two files for one key). Depends on minisign + serde_json
only, so the release job builds it in seconds. Core and app tests sign
their fixtures with it, so writer and reader are tested against each
other.

### 15.3 Packaging

`[package.metadata.packager]` in `crates/ezssh-app/Cargo.toml` (product
ezSsh, identifier `xyz.angwe.ezssh`, data paths unchanged) and
`crates/ezssh-app/packaging/` (icons copied from `src-tauri/icons`, NSIS
strings, WiX fragment, `portable.txt`, README). `before-packaging-command`
builds profile `dist` (fat LTO, one codegen unit, stripped). `panic`
stays `unwind` (§9.2 left that open): with `abort` a panicking reader
thread would take every tab down.

- NSIS: per-user install (`%LOCALAPPDATA%\ezSsh`, no UAC). cargo-packager's
  NSIS template has no uninstall hook; its "delete app data" checkbox is
  pointed at the per-user VcXsrv folder and relabelled, so uninstall
  offers to remove VcXsrv and says that sessions and the vault stay (the
  Tauri hook removed it unasked).
- MSI: the Tauri WiX fragment, renamed, linked through
  `custom-action-refs`.
- VcXsrv: staged at `target/vcxsrv` on Windows, shipped as a resource
  next to `ezssh.exe` (`vcxsrv/`, which `xserver::detect_install_path`
  already checks); a missing folder adds nothing on other platforms.
- deb: dependencies listed by hand (the binary links libc only; winit,
  xkbcommon, fontconfig and the GPU API are loaded at run time). No
  webkit2gtk.
- The window got an icon (`ViewportBuilder::with_icon`); the Windows exe
  has no embedded icon resource yet (shortcuts use the installer's icon).

### 15.4 Workflows

`release.yml` is now the native build: tag must equal the `Cargo.toml`
version (a mismatch would make installed copies offer the same update
forever; the Tauri workflow rewrote the version instead), cargo-packager
per platform (Windows nsis+wix, Linux x64/arm64 deb+appimage with
`APPIMAGE_EXTRACT_AND_RUN`, macOS app → `.app.tar.gz`), portable `.tar.xz`
with the marker, then one release job that signs, verifies against
`updater.pub`, writes `latest.json`, checks its URLs, SHA256SUMS, and a
draft release (pre-release for `-` versions). Without the secret it
publishes without `latest.json`. The Tauri workflow is
`release-tauri.yml`: manual runs, artifacts only. CI, `build.sh` and
`build.cmd` include `ezssh-release`.

### 15.5 Verification

- `ezssh-release`: 7 unit tests (cargo-packager names → manifest keys, key
  clashes, sign / verify round trip incl. base64 keys, signatures read by
  `minisign-verify` the way the app reads them, encrypted keys, RFC 3339).
- `ezssh-core/src/updater/tests.rs` (12): keys per install kind; manifest
  evaluation (older / equal / pre-release ordering, deb never takes the
  bare key, portable gets no installer, garbage is an error); signed
  version required; safe download names; detection (portable, AppImage);
  end to end over `ezssh_testsrv::http`: check + download with progress,
  tampered bytes refused and deleted, wrong version and foreign key
  refused before downloading, short body, no manifest, no key; the
  release tool's manifest read back for all five kinds; AppImage swap
  keeps the mode and leaves no temp file; `.app` swap and a bad archive
  that leaves the bundle alone; the Windows helper script (quoting of
  `'` and spaces, MSI arguments, only `ezssh-update-*` folders deleted).
- `ezssh-app/src/ui_tests/update.rs` (2): unlock → settings → launch check
  opens the dialog → Install & restart replaces a stand-in AppImage and
  starts it with the same arguments; status-bar check says up to date,
  then a tampered installer ends in the error and nothing is installed.
  3 screens with `EZSSH_UI_SHOTS`. Unit tests for the launch cadence and
  byte formatting; `layout.json` keeps the last check.
- The Windows install path (`#[cfg(windows)]`) type-checks for
  `x86_64-pc-windows-gnu` in a scratch crate; it has not run on Windows.
- Local packaging (Linux x86_64): `cargo packager -p ezssh-app -f
  deb,appimage` → `ezssh_1.14.0_amd64.deb` 18.3 MB (package `ezssh`,
  dependencies as above, desktop entry, hicolor icons 32–512) and
  `ezssh_1.14.0_x86_64.AppImage` 17.7 MB, which starts, opens a window
  (Vulkan, Intel Arc) and detects itself as an AppImage. `target/dist/ezssh`
  is 41.3 MB (fat LTO + strip; 43 MB stripped without LTO, §12.3). The
  `dist` build takes about 6 minutes here.
- Workspace clippy clean; all Rust tests green.

### 15.6 Findings

1. cargo-packager names a deb after the kebab-cased product name
   (`ez-ssh`) unless `deb.package-name` is set; the updater looks for
   `ezssh.list`, so it is set. Its `@2x` icon file lands in a
   non-standard `256x256@2` hicolor folder, so the 256 px icon is a plain
   `256x256.png`. Config keys follow its serde aliases
   (`installer-mode`, `custom-language-file`), not the docs' names.
2. A label inside a row does not wrap in egui; `widgets::error_strip` now
   wraps, so long errors no longer widen a dialog past its header.
3. The binary barely shrinks with LTO (41.3 vs 43 MB); the size is the
   backend (sqlx, russh, rust-s3, reqwest), resvg and both renderers.

### 15.7 Left for the user / later

- **Generate the update key** and set the two secrets
  (`docs/UPDATER_SIGNING.md`); until then releases carry no `latest.json`
  and builds report "no update key".
- Run a tagged release once: Windows NSIS / MSI, arm64 Linux and macOS
  were never built here (WiX, NSIS and the `.app` are configured, not
  tried), and the updater has not installed over a real NSIS / MSI / deb /
  `.app` install (app README items 17–18).
- Embedded exe icon on Windows (needs a resource compiler step);
  macOS signing and notarisation; `panic = "abort"` and dropping a
  renderer for size.
- Phase 8: delete `ui/`, `src-tauri/` and `release-tauri.yml` (the
  packages already use their own copy of the icons).

## 16. Phase 8 results (2026-11-02)

Cut-over. The repository is the native app only; version `2.0.0-beta0`.

### 16.1 Removed

- `ui/` (Next.js + xterm.js, including the HTML mockups in `ui/design`),
  `src-tauri/` (adapter, splash, Tauri updater, installer hooks, icons),
  `site/` (Astro site and its mockups), `crates/ezssh-spike` (phase 1 test
  window; its results stay in §9), the root `package.json`, `ezterm.png`,
  `.github/workflows/release-tauri.yml` and `site.yml`, the CI `ui` and
  `site` jobs, `docs/superpowers/` (webview-era specs and plans) and the
  webview design notes (`docs/design/critique.md`, `design-system.md`,
  `implementation-notes.md`; tokens now live in `style.rs`). The lockfile
  went from 946 to 738 packages (Tauri, wry, webkit2gtk, gtk).
- Core API only the Tauri adapter declared (the webview's components
  never called them either): `forward_stop_all`, `wsl_list_distros`,
  `known_host_list` / `known_host_remove`, `sftp_chmod`,
  `credential_list_with_session_passwords`; and the webview file pane's
  upload paths `sftp_upload_bytes` (256 MB in-memory cap) and
  `sftp_upload_begin` / `_chunk` / `_finish` / `_abort` with
  `sftp/upload_stream.rs` and `AppState.upload_streams`. Below them:
  `upload_bytes` in `sftp::transfer` and `wslfs::transfer`,
  `wslfs::ops::chmod`, `db::known_hosts::remove`,
  `db::credentials::list_with_session_passwords`, the per-connection
  `output_sent` counters of the stall watchdog, the empty `ssh_smoke` test
  and its `integration` feature, the `async-channel` and `bytes`
  dependencies, and the Tauri `resources/vcxsrv` probe path.
- `db::sessions::bind_password` no longer claims an unowned password row
  or copies another session's row when a password session names one in
  `credential_id`: only the Tauri picker sent that. Password sessions
  ignore `credential_id`; unowned rows from ezTerm are still listed (and
  deletable) in the credentials manager, legacy backups still give each
  session its own copy, and duplicate still copies the row.
- The `palette::matches_the_webview_palettes` test (it read
  `ui/lib/palettes.ts`).

### 16.2 Kept for ezTerm data

Data paths (`ZerosAndOnes/ezTerm/ezterm.sqlite`, per-user VcXsrv under
`zerosandones/ezterm`), the backup magic, the sync object name, the vault
verifier, migration history, settings keys and values, palette ids and
the `layout.json` view-mode ids. Comments that described ports of
webview files now describe the code; where a value matters because ezTerm
wrote it, they say so.

### 16.3 Changed

- User-visible names: backup / restore dialogs and file names
  (`ezssh-backup-<date>.json`), sync placeholders, the forward conflict
  message ("another ezSsh tab"), the restore preview's version label.
- `tools/gen_icons.mjs` takes the lucide-react folder as an argument or
  `LUCIDE_REACT_DIR` (there is no `ui/node_modules` any more).
- `build.sh` / `build.cmd`: run, build, release (`dist`), package, test,
  clean. CI: one `rust` job on `--workspace` plus `cargo audit`.
- README, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT, CLAUDE.md, RELEASING,
  UPDATER_SIGNING, issue and PR templates point at the fork and describe
  the native app. Release notes `docs/release-notes/v2.0.0-beta0.md`.

### 16.4 Findings

1. **"Release v2.0.0 through the v1 update channel" (§5) is not
   possible.** ezTerm v1 checks upstream's
   `ZerosAndOnesLLC/ezTerm/releases/latest/download/latest.json` and
   verifies with upstream's minisign key, neither of which the fork
   controls. ezTerm users install ezSsh once by hand; it opens their
   database.
2. **The switch is one way.** Upstream ezTerm runs the sqlx migrator
   with default settings, which refuses a database that has an applied
   migration it does not know (`VersionMissing`). ezSsh applies
   `20261101120000_session_owned_passwords` on first start, so ezTerm can
   no longer open that database afterwards. The release notes and README
   say to back up first or try ezSsh with `--db` on a copy.
3. `2.0.0-beta0` is a semver pre-release: the release job marks it as a
   GitHub pre-release, the stable channel (`releases/latest`) never sees
   it, the pre-release channel does. Pre-release identifiers compare as
   text, so `beta10` < `beta2`. In a deb, `-beta0` is the Debian revision
   and sorts above a bare `2.0.0`; the updater installs with `dpkg -i`,
   which allows that downgrade, so beta → final still updates.

### 16.5 Verification

`cargo check --workspace --all-targets` with `-D warnings`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace`
(core 115 unit + integration, app 104, term 73 + 9, release 7, testsrv)
all green on Linux. No release was tagged and nothing was built on
Windows or macOS (§15.7 still applies).

### 16.6 Left for the user

- Generate the update key and set the two secrets (§15.7), commit, tag
  `v2.0.0-beta0`, check the draft, publish.
- The ezTerm-era release notes (`docs/release-notes/v0.*`, `v1.*`) are
  kept as history.
