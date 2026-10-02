# Reins desktop app

The app for people who never open a terminal: a window that pairs this computer with the phone (a QR code), adds
Reins to every AI harness it finds, starts the background service and sends git through it, and afterwards a shield
in the menu bar (macOS) or tray (Windows, Linux) with Pause for 1 hour, Resume, Open Reins and Quit. It is built with
[GPUI](https://gpui.rs) through Zeron's fork ([zui](https://github.com/katulevskiy/zui)), in the phone apps' look: Geist,
cool neutrals, one violet accent.

It does its work through the `rewarden_desktop` library in its own process and through the running daemon's control
API; it never runs `rewarden` for that. The `rewarden` program ships next to it (inside `Reins.app`, in the install
directory on Windows, inside the AppImage): it is what the harnesses and the background service run, and the status
window's "Install command line tool" links it to `~/.local/bin`.

| Module | What |
| --- | --- |
| `pairing` | the device flow (`rewarden_desktop::server::device`, feature `device-flow`), the browser sign-in, `--demo` |
| `backend` | harness detection and setup, the service (or the daemon inside the app), pause and resume, updates |
| `model`, `ui` | the screens: onboarding, first-time setup, status |
| `tray` | `tray-icon` on macOS and Windows, `ksni` (StatusNotifierItem) on Linux |
| `autostart` | open at login: a launch agent, the `Run` key (`Reins app`), an XDG autostart entry |

## Run it

```sh
cargo build -p rewarden-desktop -p rewarden-desktop-app     # rewarden next to reins-app, as in the bundle
target/debug/reins-app --demo                                # a pretend pairing; nothing is sent
```

`REINS_HOME=/tmp/somewhere` makes the app treat that directory as the home directory (harness settings, git's global
config, login items, its own settings and state), to try it without touching the real ones. `REINS_DAEMON=in-app` runs
the daemon inside the app instead of installing the background service. `REWARDEN_SERVER` pairs with another server.

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
