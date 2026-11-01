# ezssh-app

The native ezSsh window: an eframe (egui 0.37) shell around
`ezssh-core` (backend) and `ezssh-term` (terminal widget). Rework phase 4
of `docs/design/native-rework-plan.md` (results in §12).

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
| `views/` | unlock, shell, sidebar (tree, DnD, menus), tab strip, MDI (six view modes, cascade chrome, minimized strip), status bar, settings, small dialogs (confirm, prompt, move to folder, tile grid, host key) |

## Not in this build yet

Phase 5 (dialogs): session editor (New / Edit session), credentials,
import, backup / restore, sync, change password, recovery code, reset
vault, update dialog, auth-fix overlay, X server install, font picker.
Their menu items and buttons are present but disabled, with a hover note.
Phase 6: the SFTP pane (the toggle opens a placeholder of the right
width). Phase 5: the forwards pane (same). Phase 7: updater.

Until then, create and edit sessions in the ezTerm app; this build reads
the same database.

## Tests

`cargo test -p ezssh-app`: unit tests plus `src/ui_tests.rs`, which drives
the real `App` on a temp database with egui_kittest (create the vault,
open a real local shell, lock / unlock with the tab kept, view modes,
cascade drag / minimise, folder create + drag and drop, settings
write-through, theme, sidebar resize persistence, host-key dialog,
multi-line paste, idle lock). Unix only (local PTYs).
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
