# Contributing to ezSsh

Thanks for considering a contribution. ezSsh is an open-source SSH client
written in Rust (egui + alacritty_terminal); PRs and issues are welcome.

## Before you start

- Browse [open issues](https://github.com/the-angwe/ezssh/issues) — if what
  you want to work on isn't there, open one first so we can agree on shape and
  scope before code lands.
- For non-trivial features, write a short design note under `docs/design/`
  first (`native-rework-plan.md` shows the level of detail: what, why, the
  alternatives, how it is verified).
- Security-sensitive changes: see [SECURITY.md](SECURITY.md). Don't open
  public issues for vulnerabilities.

## Dev quickstart

See [Development](README.md#development) in the README. The short version:

```bash
cargo run -p ezssh-app -- --db /tmp/ezssh-dev/ezterm.sqlite
```

## Before opening a PR

- `cargo check --workspace --all-targets` must pass clean (no warnings;
  remove unused code, don't `#[allow(...)]` past warnings).
- `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- `cargo test --workspace` passes. UI changes come with a test in
  `crates/ezssh-app/src/ui_tests/` (or `crates/ezssh-term/tests/ui.rs` for the
  terminal widget).
- Bump `Cargo.toml` version per the convention: major = breaking, minor =
  features, patch = fixes. Docs-only commits don't bump.
- Migrations are added with `sqlx migrate add` so the timestamp is correct.
  The database, vault, backup and sync formats are shared with ezTerm
  installs; changes must keep existing data opening.

## Commit messages

Conventional Commits, for example:

- `feat(ssh-forwards): EADDRINUSE friendly msg, async-error toasts`
- `fix(sftp): keep the selection after a rename`
- `docs(release-notes): v2.0.0`
- `sec(ssh-forwards): bind_addr parse-check at save`

No `Co-Authored-By` or "Made with X" attribution in commit messages.

## UI changes

ezSsh's look-and-feel target is **MobaXterm** — left Sessions sidebar,
tabbed terminal area, optional SFTP side-pane, dark by default with a light
toggle. Colours, sizes and the type scale live in
`crates/ezssh-app/src/style.rs`; shared controls in `widgets.rs` and
`form.rs`. Match existing styles; don't introduce new visual languages
without an explicit design update.

## Questions

Open a [Discussion](https://github.com/the-angwe/ezssh/discussions) for usage
Q&A or feature ideas, an [Issue](https://github.com/the-angwe/ezssh/issues)
for confirmed bugs.
