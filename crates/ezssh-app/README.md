# ezssh-app

The ezSsh window: an eframe (egui 0.37) shell around `ezssh-core`
(backend) and `ezssh-term` (terminal widget). Design and history:
`docs/design/native-rework-plan.md` (§12 to §16).

```sh
cargo run -p ezssh-app --release                 # the real database
cargo run -p ezssh-app --release -- --db /tmp/x/ezterm.sqlite   # a throwaway one
cargo run -p ezssh-app -- --renderer warp        # software rendering (VMs, RDP)
RUST_LOG=ezssh_app=debug,ezssh_core=debug cargo run -p ezssh-app
cargo packager -p ezssh-app -f deb,appimage      # installers into target/packages (packaging/README.md)
```

The binary is `ezssh`. Without `--db` it opens `ezterm.sqlite` in the data
folder ezTerm used, so an existing ezTerm install's sessions, credentials
and settings carry over. Running ezTerm and ezSsh on the same database at
once is untested; quit ezTerm first. Window layout (view mode, tile grid,
sidebar width, SFTP pane width and columns, last update check) is kept in
`layout.json` next to the database.

## Layout

| File | Holds |
|---|---|
| `app.rs` | start-up, the vault gate, settings hydrate, WSL / local-shell autodetect, idle lock, quit confirm, Ctrl+B / Ctrl+K / Ctrl+Shift+W |
| `core.rs` | a tokio runtime owns the `AppState`; `Core::call(future, continuation)` runs the continuation on the UI thread |
| `conn.rs` | the connect state machine per tab: output straight into the terminal, per-attempt generations, reconnect, error routing |
| `settings.rs` | app preferences: spec, parse, normalise, write-through with rollback |
| `tabs.rs` | open tabs, view mode, cascade geometry |
| `layout.rs` | `layout.json` |
| `style.rs`, `icons.rs`, `ui_font.rs` | colour / size / type tokens, Lucide icons (`tools/gen_icons.mjs`), system UI font |
| `widgets.rs` | the `Button` family, icon buttons, switches, segmented controls, chips, the nav rail, row fills, the dialog chrome, context menus, resize handles, status dots, empty states (`docs/design/ui-facelift.md`) |
| `toasts.rs`, `bell.rs` | toasts (four kinds); the 880 Hz bell |
| `form.rs`, `files.rs`, `creds.rs`, `font_presets.rs` | dialog form controls; native file dialogs (rfd); the shared credentials list; the font list (installed monospace families, OS presets as fallback) |
| `views/` | unlock (+ recovery unlock, reset vault), shell, sidebar (tree, DnD, menus), tab strip, MDI (six view modes, cascade chrome, minimized strip), status bar, settings, small dialogs (confirm, prompt, move to folder, tile grid, host key) |
| `views/` (dialogs) | `session_dialog`, `cred_picker`, `auth_fix`, `credentials`, `forwards` (pane, dialog, session Forwards tab), `vault_dialogs` (change password, recovery code), `appearance`, `xserver`, `backup`, `restore`, `sync`, `import` |
| `views/sftp.rs` | the file pane (SFTP or the WSL share), OS drops of files and folders, progress strips, row and list menus |
| `views/update.rs` | launch check (cadence in `layout.json`), status-bar button with a dot, download progress, install, restart; the updater itself is `ezssh_core::updater` |
| `packaging/` | icons, NSIS strings, WiX fragment, `portable.txt`; the cargo-packager config is `[package.metadata.packager]` in `Cargo.toml` |

## Tests

`cargo test -p ezssh-app`: unit tests plus `src/ui_tests/` (`mod.rs` and one
file per dialog group), which drives
the real `App` on a temp database with egui_kittest (create the vault,
open a real local shell, lock / unlock with the tab kept, view modes,
cascade drag / minimise, folder create + drag and drop, settings
write-through, theme, sidebar resize persistence, host-key dialog,
multi-line paste, idle lock; every phase 5 dialog, checking the database
after saves; the file pane against the in-process SSH / SFTP server of
`crates/ezssh-testsrv`; the update dialog against a signed release served
by `ezssh_testsrv::http`, replacing and restarting a stand-in AppImage). Unix only (local PTYs). Native file dialogs cannot run
headless; the tests hand the dialogs a path directly.
`EZSSH_UI_SHOTS=<dir> cargo test -p ezssh-app ui_tests` renders each state
to PNG.

## Manual checklist (things the headless tests cannot show)

1. Start against a copy of a real ezTerm database: the tree, colours and
   badges match what ezTerm showed.
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
7. Lock (sidebar footer), leave it for a while, unlock: tabs still connected.
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
17. Packages (`packaging/README.md`): install the deb (`sudo apt install
    ./target/packages/ezssh_*_amd64.deb`), start ezSsh from the desktop
    menu; run the AppImage. On Windows the NSIS setup and the MSI from a
    release build, uninstall with the VcXsrv checkbox; macOS `ezSsh.app`
    (right-click → Open, it is not notarised).
18. Updater end to end with a test key: `packaging/README.md` § "Trying
    the updater" (a local `latest.json` via `EZSSH_UPDATE_MANIFEST`, one
    run per installed kind: AppImage, deb, NSIS, MSI, `.app`; portable
    shows "Open releases page").
