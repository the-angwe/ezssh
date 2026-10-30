# ezssh-spike

Phase 1 of the native rework (`docs/design/native-rework-plan.md` §5, results
in §9): the `ezssh-term` widget in a bare eframe window, over a local PTY and
one SSH session. It is a test bench, not the product. No vault, no saved
sessions, no settings.

## Build and run

```sh
cargo build --release -p ezssh-spike
./target/release/ezssh-spike                          # one local shell tab
./target/release/ezssh-spike --ssh user@host          # agent, then ~/.ssh/id_*
./target/release/ezssh-spike --ssh user@host:2222 --password --trust-new-host
./target/release/ezssh-spike --renderer warp          # force the software adapter
./target/release/ezssh-spike --bench                  # 8 MiB throughput / frame-rate bench
./target/release/ezssh-spike --help
```

Linux needs no webkit2gtk; the window uses Wayland or X11 directly.

Host keys: checked against `~/.ssh/known_hosts` (read only) and a spike file
in the user data dir (`~/.local/share/ezssh-spike/known_hosts`,
`%APPDATA%\ezSsh\ezssh-spike\data\known_hosts`). An unknown key is refused and
its fingerprint printed; `--trust-new-host` stores it in the spike file. A
changed key is always refused.

Logs: `RUST_LOG=ezssh_term=debug,ezssh_spike=debug`. The status bar shows
frame rate, worst frame, last paint time, throughput, grid size, font and
renderer.

## Keys

| Keys | Action |
|---|---|
| Ctrl+Shift+C, Ctrl+Insert | Copy selection |
| Ctrl+Shift+V, Shift+Insert, right-click | Paste (bracketed when the app asks) |
| Shift+PageUp / PageDown / Home / End | Scroll history (not in full-screen apps) |
| Ctrl+wheel | Zoom 8–48 pt |
| Shift+drag | Select even when the app has mouse reporting on |
| Double / triple click | Word / line selection |

Everything else, including Ctrl+C, goes to the shell.

## Manual gate checklist

These need a person at the keyboard (or a host / OS this machine does not
have). Tick them in §9 of the plan.

1. **vim / neovim**: edit, visual mode, `:split`, mouse (`:set mouse=a`),
   colourschemes with truecolor.
2. **tmux**: panes, status line, `set -g mouse on`, drag-select inside a pane,
   Shift+drag to select across panes.
3. **htop**: meters, F-keys, mouse clicks on the header.
4. **Claude Code**: Shift+Enter inserts a newline (kitty keyboard
   protocol), Ctrl+C / Esc behave, the TUI redraws cleanly while streaming.
5. **OSC 52 from neovim over SSH**: `"+y` in remote neovim with
   `clipboard=unnamedplus` and an OSC 52 provider, paste locally.
6. **IME**: Japanese / Chinese input with IBus or fcitx (Linux) and the
   Windows IME; candidate window opens at the cursor.
7. **Windows VM over RDP**: `--renderer wgpu` picks "Microsoft Basic Render
   Driver" (WARP) when there is no GPU; also try `--renderer warp` and
   `--renderer glow`. Run `--bench` and note the numbers.
8. **Windows keyboard layouts**: AltGr characters on a German or Swiss
   layout (`@`, `{`, `\`), dead keys.
9. **Live SSH**: `--ssh user@host`, `cat` a large file, resize the window,
   run 1–3 there.
