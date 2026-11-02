# Packaging ezSsh

The installers are built by [cargo-packager](https://github.com/crabnebula-dev/cargo-packager)
0.11 from `[package.metadata.packager]` in `../Cargo.toml`; this folder
holds what that config points at. The release workflow
(`.github/workflows/release.yml`) runs the same commands per platform and
then signs everything with `ezssh-release` (see `docs/UPDATER_SIGNING.md`).

```sh
cargo install cargo-packager --version 0.11.8 --locked
cargo packager -p ezssh-app -f deb,appimage     # Linux
cargo packager -p ezssh-app -f nsis,wix         # Windows (WiX and NSIS are downloaded on first use)
cargo packager -p ezssh-app -f app              # macOS
```

Each run first builds `target/dist/ezssh` (profile `dist`: fat LTO, one
codegen unit, stripped) and writes into `target/packages`:

| Format | File | Installs to | Updater kind |
|---|---|---|---|
| NSIS | `ezssh_<v>_x64-setup.exe` | `%LOCALAPPDATA%\ezSsh` (per user, no admin) | `nsis` (found by `uninstall.exe` next to the exe) |
| WiX | `ezssh_<v>_x64_en-US.msi` | Program Files | `msi` |
| AppImage | `ezssh_<v>_x86_64.AppImage` | wherever the user keeps it | `appimage` (`$APPIMAGE`) |
| deb | `ezssh_<v>_amd64.deb` | `/usr/bin/ezssh` + desktop entry | `deb` (`/var/lib/dpkg/info/ezssh.list`) |
| app | `ezSsh.app` (the release job tars it to `ezssh_<v>_aarch64.app.tar.gz`) | `/Applications` | `app` |

The portable archives (`ezSsh_<v>_<platform>-portable.tar.xz`) are made by
the workflow, not by cargo-packager: the binary, README, LICENSE, VcXsrv on
Windows, and `portable.txt`, which tells the updater not to replace that
copy.

## What is in here

| File | Used by |
|---|---|
| `icons/` | installer, desktop entry, `.app` and window icons (the ezTerm icon set) |
| `nsis/English.nsh` | cargo-packager's English strings with the uninstall checkbox reworded: it removes the per-user VcXsrv (`nsis.appdata-paths`), never the database or the vault |
| `wix/remove-user-vcxsrv.wxs` | MSI uninstall removes the per-user VcXsrv |
| `portable.txt` | marker in the portable archives |

Data paths are those ezTerm used (`ezterm.sqlite` under
`ZerosAndOnes/ezTerm`, the per-user VcXsrv under `zerosandones/ezterm`),
so an installed ezSsh opens an existing ezTerm database.
The install identity is new (`xyz.angwe.ezssh`, product ezSsh), so it
installs next to ezTerm, not over it.

## Linux runtime dependencies

The binary links only libc; everything else is loaded at run time, so the
deb lists it by hand: xkbcommon (+ x11), fontconfig, the X11 client libs
winit opens (X11, xcb, Xcursor, Xi, Xrandr), libwayland-client, and Vulkan
or EGL for wgpu. The AppImage takes them from the host. No webkit2gtk.

## Trying the updater

The signature check cannot be skipped, so a test needs a throwaway key in
`crates/ezssh-core/updater.pub` (do not commit it):

```sh
cargo run -p ezssh-release -- keygen --out /tmp/ezssh-k --no-password > crates/ezssh-core/updater.pub
cargo packager -p ezssh-app -f appimage          # version A (installed copy)
cp target/packages/ezssh_*_x86_64.AppImage /tmp/ezssh-old.AppImage
# bump `version` in Cargo.toml to B > A, then:
cargo packager -p ezssh-app -f appimage
mkdir -p /tmp/ezssh-rel && cp target/packages/ezssh_B_x86_64.AppImage /tmp/ezssh-rel/
EZSSH_SIGNING_KEY="$(cat /tmp/ezssh-k/ezssh-update.key)" \
  cargo run -p ezssh-release -- sign --version B /tmp/ezssh-rel/*.AppImage
cargo run -p ezssh-release -- manifest --dir /tmp/ezssh-rel --version B --base-url http://127.0.0.1:8000
python3 -m http.server -d /tmp/ezssh-rel 8000 &
EZSSH_UPDATE_MANIFEST=http://127.0.0.1:8000/latest.json /tmp/ezssh-old.AppImage
```

Status bar → ✦ (or the launch check) offers B; "Install & restart"
replaces `/tmp/ezssh-old.AppImage` and starts it again. Same for a deb
(`sudo apt install ./…deb` for A, a password prompt for B), NSIS / MSI on
Windows (a helper waits for the app to exit, runs the installer passively,
starts the app; it logs to `%TEMP%\ezssh-update.log`) and the `.app`.
Revert `updater.pub` and the version afterwards.
