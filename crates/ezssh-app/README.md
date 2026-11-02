# ezssh-app

The native ezSsh window: an eframe (egui 0.37) shell around
`ezssh-core` (backend) and `ezssh-term` (terminal widget). Rework phases 4
(shell), 5 (dialogs) and 6 (file pane) of `docs/design/native-rework-plan.md`
(results in §12, §13 and §14).

```sh
cargo run -p ezssh-app --release                 # the real database (shared with the Tauri build)
cargo run -p ezssh-app --release -- --db /tmp/x/ezterm.sqlite   # a throwaway one
cargo run -p ezssh-app -- --renderer warp        # software rendering (VMs, RDP)
RUST_LOG=ezssh_app=debug,ezssh_core=debug cargo run -p ezssh-app
```

The binary is `ezssh`. Without `--db` it opens the same `ezterm.sqlite`
as the installed ezTerm app. Running both on the same database at once is
untested; quit ezTerm first. Window layout (view mode, tile grid, sidebar width, SFTP
pane width and columns) is kept in `layout.json` next to the database.

## Layout

| File | Ports |
|---|---|
| `app.rs` | `page.tsx` vault gate, `main-shell.tsx` (settings hydrate, WSL / local-shell autodetect, idle lock, quit confirm, Ctrl+B / Ctrl+Shift+W) |
| `core.rs` | `invoke()`: a tokio runtime owns the `AppState`; `Core::call(future, continuation)` runs the continuation on the UI thread |
| `conn.rs` | `terminal.tsx` connect state machine, `ssh.ts` (output straight into the terminal, per-attempt generations instead of arm / detach, no stall watchdog) |
| `settings.rs` | `lib/settings.ts` (spec, parse, normalise, write-through with rollback) |
| `tabs.rs` | `lib/tabs-store.ts`, cascade geometry of `use-mdi-drag.ts` / `use-mdi-resize.ts` |
| `layout.rs` | the `ezterm.*` localStorage keys |
| `style.rs`, `icons.rs`, `ui_font.rs` | `globals.css` tokens, Lucide icons (`tools/gen_icons.mjs`), system UI font |
| `widgets.rs` | `.btn-*`, `.icon-btn`, dialog shell, `context-menu.tsx`, `resize-handle.tsx`, `status-dot.tsx`, `empty-state.tsx` |
| `toasts.rs`, `bell.rs` | `lib/toast.ts` + `toast-region.tsx`; the 880 Hz bell |
| `form.rs`, `files.rs`, `creds.rs`, `font_presets.rs` | Section / Field / `.input` controls of the dialogs; native file dialogs (rfd); the shared credentials list (`credential-events.ts`); the font list (installed monospace families, `lib/fonts.ts` as fallback) |
| `views/` | unlock (+ recovery unlock, reset vault), shell, sidebar (tree, DnD, menus), tab strip, MDI (six view modes, cascade chrome, minimized strip), status bar, settings, small dialogs (confirm, prompt, move to folder, tile grid, host key) |
| `views/sftp.rs` (phase 6) | `sftp-pane.tsx`, `sftp-breadcrumb.tsx`, `sftp-file-row.tsx`, `transfer-status.tsx`: the file pane (SFTP or the WSL share), OS drops of files and folders, progress strips, row and list menus |
| `views/` (phase 5) | `session_dialog`, `cred_picker`, `auth_fix`, `credentials`, `forwards` (pane, dialog, session Forwards tab), `vault_dialogs` (change password, recovery code), `appearance` (replaced `font_picker`), `xserver`, `backup`, `restore`, `sync`, `import` |

## Not in this build yet

Phase 7: the updater and its dialog (the status-bar button is
disabled).

## Tests

`cargo test -p ezssh-app`: unit tests plus `src/ui_tests/` (`mod.rs` and one
file per dialog group), which drives
the real `App` on a temp database with egui_kittest (create the vault,
open a real local shell, lock / unlock with the tab kept, view modes,
cascade drag / minimise, folder create + drag and drop, settings
write-through, theme, sidebar resize persistence, host-key dialog,
multi-line paste, idle lock; every phase 5 dialog, checking the database
after saves; the file pane against the in-process SSH / SFTP server of
`crates/ezssh-testsrv`). Unix only (local PTYs). Native file dialogs cannot run
headless; the tests hand the dialogs a path directly.
`EZSSH_UI_SHOTS=<dir> cargo test -p ezssh-app ui_tests` renders each state
to PNG.

## Manual checklist (things the headless tests cannot show)

1. Start against a copy of a real database: the tree, colours and badges
   match the ezTerm app.
2. Double-click an SSH session: host-key prompt on first connect, then a
   working shell; vim, htop, tmux, Claude Code.
3. Window close (title-bar ✕, Alt+F4) with a connected tab asks first;
   Quit closes.
4. Tab strip with 10+ tabs: shrinks to 180 px, then the chevrons and the
   mouse wheel scroll it; drag reorder with the indicator line;
   middle-click closes.
5. Cascade: move and resize frames from all eight handles; double-click
   the title bar maximises; minimise / restore from the strip.
6. Sidebar: drag sessions between folders (whole folder row = into),
   folders 25 / 50 / 25, whitespace = root; auto-scroll near the edges;
   Escape cancels.
7. Lock (status bar), leave it for a while, unlock: tabs still connected.
8. Settings → Security → idle lock 1 minute, wait: the vault locks.
9. Light theme: window decorations follow (Wayland / Windows).
10. Bell = sound: `printf '\a'` beeps (pw-play / paplay / aplay on Linux).
11. New session (SSH, password): connect works; Edit shows "Saved — leave
    blank to keep"; a wrong password opens the auth-fix overlay over the
    tab, fixing it reconnects.
12. A database from the ezTerm app with a password shared by two sessions:
    after the first start both still connect, and the Credentials manager
    lists only keys and passphrases.
13. Backup → Restore → Sync (local folder) → Import: the native open / save
    / folder dialogs appear (desktop portal on Linux, system dialogs on
    Windows) and the chosen path is used.
14. Forwards pane on a connected SSH tab: add a local forward, it shows
    running; stop; edit restarts it.
15. Windows: X11 session without VcXsrv → install from the dialog.
16. File pane against a real server (and WSL on Windows): browse, drag
    files and a folder from Explorer / the file manager onto the pane,
    download one file (save dialog), several and a folder (folder dialog),
    rename, delete several at once, cancel a running transfer. On Linux, drops need X11 or XWayland
    (`WAYLAND_DISPLAY= cargo run -p ezssh-app`): winit 0.30 has no
    Wayland drag and drop; the list's right-click menu has Upload files /
    Upload folder instead. Without a server at hand:
    `cargo run -p ezssh-testsrv -- --root /some/dir --home / --delay-ms 2`
    serves a folder on 127.0.0.1:2222 as `ezssh` / `ezssh`.
