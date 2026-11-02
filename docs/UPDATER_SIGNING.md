# Updater signing: one-time setup

ezSsh's in-app updater (`crates/ezssh-core/src/updater/`) installs only
installers signed with the update key whose public half is compiled into
the app (`crates/ezssh-core/updater.pub`). Until that file holds a key,
"Check for updates" says the build has no update key, and the release
workflow publishes releases without `latest.json`, so no installed copy is
offered them.

The formats are the ones ezTerm's `tauri-plugin-updater` used: minisign keys and
signatures, each base64-wrapped into one line, `latest.json` with
`{version, notes, pub_date, platforms}`. The signature's trusted comment
names the release version, and the app refuses an installer whose
signature names another version than the manifest announces.

## 1. Generate the key

```sh
export EZSSH_SIGNING_KEY_PASSWORD='a long passphrase'
cargo run -p ezssh-release -- keygen --out ~/.ezssh-keys
```

This writes `~/.ezssh-keys/ezssh-update.key` (secret, mode 600, encrypted
with the passphrase) and `ezssh-update.pub`, and prints one base64 line.
Keep the secret key and the passphrase somewhere safe (a password
manager): losing them means installed copies can no longer be updated
(see "Rotating the key").

## 2. Commit the public key

Put the printed line into `crates/ezssh-core/updater.pub` (the file holds
only that line) and commit it. Every build from then on trusts that key.

## 3. Add the secret key to GitHub

Repository → Settings → Secrets and variables → Actions → New repository
secret:

| Name | Value |
|---|---|
| `EZSSH_SIGNING_KEY` | the whole contents of `ezssh-update.key` (or its base64) |
| `EZSSH_SIGNING_KEY_PASSWORD` | the passphrase |

Only the `release` job of `.github/workflows/release.yml` reads them; the
build jobs never see the key.

## 4. Release

Bump `version` in the workspace `Cargo.toml`, commit, tag `vX.Y.Z` (or
`vX.Y.Z-beta.N` for a pre-release) and push the tag. The workflow:

1. checks that the tag equals the `Cargo.toml` version (the app reports
   that version to the updater),
2. builds the NSIS + MSI, AppImage + deb (x86_64 and arm64), the macOS
   `.app.tar.gz` and a portable `.tar.xz` per platform,
3. signs every installer (`ezssh-release sign`), checks the signatures
   against the committed `updater.pub` (`ezssh-release verify`; a key
   mismatch fails the release here, not on users' machines),
4. writes `latest.json` (`ezssh-release manifest`) with release notes from
   `docs/release-notes/vX.Y.Z.md` when that file exists,
5. creates a draft release (marked pre-release for `-` versions).

Publish the draft. The stable channel reads
`https://github.com/<repo>/releases/latest/download/latest.json`, which
GitHub resolves to the newest published non-pre-release; the pre-release
channel takes the newest published release of either kind that carries a
`latest.json`.

## Signing by hand

```sh
export EZSSH_SIGNING_KEY="$(cat ~/.ezssh-keys/ezssh-update.key)"
export EZSSH_SIGNING_KEY_PASSWORD='…'
ezssh-release sign --version 1.15.0 target/packages/*.deb target/packages/*.AppImage
ezssh-release verify --pubkey crates/ezssh-core/updater.pub --version 1.15.0 target/packages/*.deb
ezssh-release manifest --dir target/packages --version 1.15.0 --base-url https://example/dl
```

## Rotating the key

Generate a new pair, replace `updater.pub` and both secrets, release. Any
copy built with the old public key refuses updates signed with the new
one; those users have to download the new version by hand once. If the
old key is not compromised, sign one last release with the old key that
already contains the new `updater.pub`, so installed copies move over on
their own.

## ezTerm installs

ezTerm (the Tauri build this fork started from) updates through
`tauri-plugin-updater` from upstream's releases with upstream's key, which
this fork does not have. ezTerm installs therefore never see ezSsh
releases; users install ezSsh once by hand, and it opens their existing
database.
