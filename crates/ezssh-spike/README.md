# ezssh-spike

Phase 1 of the native rework (`docs/design/native-rework-plan.md` §5, results
in §9), kept as the hands-on bench for the `ezssh-term` widget (phase 3, §11):
the widget in a bare eframe window, over a local PTY and one SSH session. It
is a test bench, not the product. No vault, no saved sessions; the
`terminal.*` settings are command-line flags (`--help`).

## Build and run

```sh
cargo build --release -p ezssh-spike
./target/release/ezssh-spike                          # one local shell tab
./target/release/ezssh-spike --ssh user@host          # agent, then ~/.ssh/id_*
./target/release/ezssh-spike --ssh user@host:2222 --password --trust-new-host
./target/release/ezssh-spike --renderer warp          # force the software adapter
./target/release/ezssh-spike --bench                  # 8 MiB throughput / frame-rate bench
./target/release/ezssh-spike --palette solarized-dark --cursor bar --bell visual --right-click paste
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
| Ctrl+Shift+V, Shift+Insert | Paste (bracketed when the app asks; multi-line pastes ask first) |
| Right-click | Context menu: Copy, Paste, Select All, Find…, Font…, Clear scrollback (`--right-click paste` pastes instead; Ctrl+right-click still opens the menu) |
| Ctrl+Shift+F | Find bar: Enter / Shift+Enter next / previous, `Aa` case, `.*` regex, Esc closes |
| Click on a link | Opens `http(s)` URLs and OSC 8 links in the browser |
| Shift+PageUp / PageDown / Home / End | Scroll history (not in full-screen apps) |
| Ctrl+wheel | Zoom this tab, 8–48 pt |
| Shift+drag | Select even when the app has mouse reporting on |
| Double / triple click | Word / line selection |

Everything else, including Ctrl+C, goes to the shell. The top bar switches
the colour palette live. A tab whose shell exited shows the Disconnected card
with Reconnect.

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
10. **Phase 3 widget by hand**: right-click menu items, find bar, link hover
    and click, Ctrl+wheel zoom step size, multi-line paste prompt, bell modes,
    cursor blink, italic text, Nerd Font icons (`ls` with eza / lsd icons).

`cargo test -p ezssh-term --test ui` runs the same widget flows headless;
`EZSSH_UI_SHOTS=<dir>` also renders them to PNG.
