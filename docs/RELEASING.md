# Releasing

The release pipeline lives in [`.github/workflows/release.yml`](../.github/workflows/release.yml).
A `v*` tag builds every platform, signs the installers, writes the updater
manifest and opens a draft GitHub Release that you review and publish by
hand.

## What gets built

| Platform | Runner | Installers (cargo-packager) | Portable |
|---|---|---|---|
| Windows x86_64 | `windows-latest` | `ezssh_<v>_x64-setup.exe` (NSIS, per user), `ezssh_<v>_x64_en-US.msi` | `ezSsh_<v>_windows-x64-portable.tar.xz` |
| Linux x86_64 | `ubuntu-latest` | `ezssh_<v>_x86_64.AppImage`, `ezssh_<v>_amd64.deb` | `ezSsh_<v>_linux-amd64-portable.tar.xz` |
| Linux aarch64 | `ubuntu-24.04-arm` | `ezssh_<v>_aarch64.AppImage`, `ezssh_<v>_arm64.deb` | `ezSsh_<v>_linux-aarch64-portable.tar.xz` |
| macOS aarch64 | `macos-latest` | `ezssh_<v>_aarch64.app.tar.gz` (unsigned, not notarised) | `ezSsh_<v>_macos-aarch64-portable.tar.xz` |

The Windows installers and portable archive carry VcXsrv (installed from
Chocolatey on the runner, staged at `target/vcxsrv`). Each portable
archive holds the binary, README, LICENSE and a `portable.txt` marker that
tells the updater to link to the release instead of replacing that copy.
Packaging config: `[package.metadata.packager]` in
`crates/ezssh-app/Cargo.toml`, details in
[`crates/ezssh-app/packaging/README.md`](../crates/ezssh-app/packaging/README.md).

The release job then signs every installer with `ezssh-release`
(secrets `EZSSH_SIGNING_KEY` / `EZSSH_SIGNING_KEY_PASSWORD`), verifies the
signatures against `crates/ezssh-core/updater.pub`, writes `latest.json`
(notes from `docs/release-notes/<tag>.md`), checks that every URL in it is
an asset of the release, and writes `SHA256SUMS`. Without the secret the
release is still created, but with no `latest.json`, so no installed copy
is offered it. One-time key setup: [UPDATER_SIGNING.md](UPDATER_SIGNING.md).

## Cut a release

1. **Bump the version** in the workspace `Cargo.toml`
   (`[workspace.package] version`) and run `cargo check` so `Cargo.lock`
   follows. The tag must equal this version; the workflow fails otherwise
   (the binary reports it to the updater, so a mismatch would offer the
   same update forever).
2. **Write the notes** in `docs/release-notes/v<version>.md`. They become
   the GitHub release body and the `notes` of `latest.json`, which the
   update dialog shows.
3. Commit, push, then **tag**: `git tag v2.0.0 && git push fork v2.0.0`.
4. Watch the `release` workflow in the Actions tab. The four build jobs run
   in parallel; `publish draft release` attaches everything to a draft.
5. Open the draft on GitHub, check the assets, and **Publish**. Installed
   copies see the release from then on (drafts are invisible to them).

A version with a `-` (`2.0.0-beta0`) is published as a GitHub
pre-release, which only copies set to the pre-release update channel are
offered; the stable channel reads `releases/latest`, which skips
pre-releases. Pre-release identifiers compare as text (`beta10` sorts
before `beta2`), so keep them to one digit or use `beta.N`.

`workflow_dispatch` runs the build jobs without a tag to smoke-test the
pipeline: artifacts land on the workflow run, no release is created.

The `prune` job keeps the newest 20 releases (drafts and pre-releases
included) and deletes older ones with their tags.

## Platform caveats

- **Windows** — WSL, local shells and the bundled X server work. The
  executable has no embedded icon yet.
- **Linux** — WSL autodetect returns nothing and `cmd.exe` / `pwsh.exe`
  local shells won't spawn; SSH, SFTP, forwards and `/etc/shells` local
  shells work. The deb's dependencies are listed in the packager config.
- **macOS** — same as Linux for the Windows-specific features. The `.app`
  is unsigned, so Gatekeeper asks on first launch (System Settings →
  Privacy & Security). Signing and notarisation need secrets.

## Cutting a release by hand

`gh` is the same locally and in CI. With the workflow artifacts downloaded
into `artifacts/` and signed as in the release job:

```bash
gh release create v2.0.0 --draft --notes-file docs/release-notes/v2.0.0.md artifacts/*
```
