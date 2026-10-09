# Reins desktop app

The app for people who never open a terminal: a window that pairs this computer with the phone (a QR code), adds
Reins to every AI harness it finds, starts the background service and sends git through it, and afterwards a shield
in the menu bar (macOS) or tray (Windows, Linux) with Pause for 1 hour, Resume, Open Reins and Quit. It is built with
[GPUI](https://gpui.rs) through Zeron's fork ([zui](https://github.com/katulevskiy/zui)), in the phone apps' look: Geist,
cool neutrals, one violet accent.

It does its work through the `reins_desktop` library in its own process and through the running daemon's control
API; it never runs `reins` for that. The `reins` program ships next to it (inside `Reins.app`, in the install
directory on Windows, inside the AppImage): it is what the harnesses and the background service run, and the status
window's "Install command line tool" links it to `~/.local/bin`.

| Module | What |
| --- | --- |
| `pairing` | the device flow (`reins_desktop::server::device`, feature `device-flow`), the browser sign-in, `--demo` |
| `backend` | harness detection and setup, the service (or the daemon inside the app), pause and resume, updates |
| `model`, `ui` | the screens: onboarding, first-time setup, status |
| `tray` | `tray-icon` on macOS and Windows, `ksni` (StatusNotifierItem) on Linux |
| `autostart` | open at login: a launch agent, the `Run` key (`Reins app`), an XDG autostart entry |
| `upgrade` | "Restart to update": the new installer in place of the running copy |

## Updates

At start and every 6 hours a release build reads `<releases>/app.json`, a list of installers signed with the same
release key as `reins update`'s `latest.json` (every release publishes it; see CONTRIBUTING.md, "The update feed"). When it lists a newer build
for this computer (`macos-universal`, `windows-x86_64`, `linux-x86_64`), the app downloads that installer into
`updates/` in its state directory, checks its size and SHA-256 against the signed list, keeps it (a later check does
not download it again), and only then shows "Reins X is ready" with **Restart to update**:

- macOS: the disk image is mounted read-only, `Reins.app` copied next to the running one with `ditto` and swapped in
  by renaming (the old one is removed at the next start); Reins opens again. When the app's folder cannot be written,
  the disk image opens instead, to drag Reins to Applications.
- Windows: once the app has quit, `msiexec /i … /passive /norestart` installs the MSI (per user, no administrator),
  then Reins starts again.
- Linux: only an AppImage in a folder Reins may write to: the new one is written next to it, renamed over it and
  started.

The first start of the new app restarts the background service (and, for an AppImage, copies the new `reins` to
`~/.local/bin` first), so it runs the new `reins`. A failed download shows nothing and is tried again at the next
check. Where the feed has no `app.json`, or no installer for this computer, and for any other copy (the tarball, a
build from source), the app says "Reins X is available" with a link to the download page, as before.

## Run it

```sh
cargo build -p reins-desktop -p reins-desktop-app     # reins next to reins-app, as in the bundle
target/debug/reins-app --demo                                # a pretend pairing; nothing is sent
```

`REINS_HOME=/tmp/somewhere` makes the app treat that directory as the home directory (harness settings, git's global
config, login items, its own settings and state), to try it without touching the real ones. `REINS_DAEMON=in-app` runs
the daemon inside the app instead of installing the background service. `REINS_SERVER` pairs with another server.

## Installers

`scripts/package/` builds them; the release workflow runs the same scripts.

| Script | Makes | Where it runs |
| --- | --- | --- |
| `macos.sh` | `Reins.app` in `Reins-<version>-macOS.dmg` (drag to Applications) | a Mac |
| `windows.ps1` | `Reins-<version>-Windows-x64.msi` (WiX v4, per user) | Windows |
| `linux.sh` | `Reins-<version>-Linux-<arch>.AppImage` and a tarball | Linux (a container works) |
| `icons.sh` | the icons and the disk image background, from `docs/assets/logo.svg` | anywhere with resvg and ImageMagick |

Each also writes the file without the version (`Reins-macOS.dmg`, ...), the name the website's download links use.
Signing: `REINS_DEVELOPER_ID` and `REINS_NOTARY_PROFILE` (or `REINS_NOTARY_KEY*`) on macOS, `REINS_WINDOWS_CERT` on
Windows; without them the scripts say so and build unsigned (ad hoc on macOS) installers.
