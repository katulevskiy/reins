# Reins desktop app

The app for people who never open a terminal: a window that pairs this computer with the phone (a QR code), adds
Reins to every AI harness it finds, starts the background service and sends git through it, and afterwards a status
window and a shield in the menu bar (macOS) or tray (Windows, Linux) with a Pause submenu, Resume, Open Reins and
Quit. It is built with
[GPUI](https://gpui.rs) through Zeron's fork ([zui](https://github.com/katulevskiy/zui)), in the phone apps' look: Geist,
cool neutrals, one violet accent.

It does its work through the `reins_desktop` library in its own process and through the running daemon's control
API; it never runs `reins` for that. The `reins` program ships next to it (inside `Reins.app`, in the install
directory on Windows, inside the AppImage): it is what the harnesses and the background service run, and the status
window's "Install command line tool" links it to `~/.local/bin`.

| Module | What |
| --- | --- |
| `pairing` | the device flow (`reins_desktop::server::device`, feature `device-flow`), the browser sign-in, `--demo` |
| `backend` | harness detection and setup, the service (or the daemon inside the app), pause and resume, the daemon's status and overview, the activity log, settings changes (restarting the service when the daemon needs it), updates |
| `model` | the app's state: screen and section, pairing, setup, pausing, the actions the window and tray take |
| `pause` | the pause lengths and how a pause reads ("Paused for 3 h 12 min more", "Paused until you resume") |
| `format` | times as the window says them: "2 min ago", "3 h 12 min", "14:32" |
| `demo` | `--demo` only: made-up activity, connections, keys and AI tools where this computer has none |
| `ui/mod.rs` | the window: one column before setup, sidebar and sections afterwards |
| `ui/parts.rs` | building blocks: cards, rows, buttons, toggles, chips, segmented controls, outcome badges |
| `ui/field.rs` | one-line text fields typed by hand (the server, the Rules lists) |
| `ui/onboarding.rs`, `ui/setup.rs` | pairing with the phone; the first-time setup |
| `ui/sidebar.rs` | the mark and state, the sections, pausing |
| `ui/overview.rs`, `ui/activity.rs`, `ui/connections.rs`, `ui/keys.rs`, `ui/rules.rs`, `ui/settings.rs` | the sections |
| `tray` | `tray-icon` on macOS and Windows, `ksni` (StatusNotifierItem) on Linux |
| `autostart` | open at login: a launch agent, the `Run` key (`Reins app`), an XDG autostart entry |
| `upgrade` | "Restart to update": the new installer in place of the running copy |

## The status window

After pairing and setup the window (resizable, 1000 × 680 at first) has a sidebar with the state (On, Paused, Needs
you), the sections and the pause control, and these sections. Everything is read again every 3 seconds: the daemon's
status and overview (`control::Client::overview`), and this computer's activity log (`reins_desktop::journal`, the
newest 500).

- **Overview**: the state in a sentence and what fixes it, what waits for the phone now (and for how long), today's
  approved, denied, timed out and failed requests, pausing, the latest requests, the update banner.
- **Activity**: every request on this computer (git, SSH, API keys, `reins run` secrets, hook commands and files,
  `reins ask`, MCP tool calls), filtered by outcome and kind; a row opens to its detail, the reason and its times.
- **Connections**: the AI tools Reins is in (toggles), the git hosts sent through Reins (a toggle each, with the
  repositories reached), the APIs called with keys from the phone, SSH sign-ins, and MCP tool calls by AI tool.
- **Keys & secrets**: the `[[api]]` entries (vault reference, how long a key is held, whether one is held now), the
  `reins run` profiles (variable names and vault references; values are never on this computer), the SSH agent (on or
  off, its socket, the keys the phone listed), and how to use them with the daemon's address.
- **Rules** ("What asks your phone"): the built-in groups of hook rules, each asking the phone or going through; your
  own commands and files to ask about, and to never ask about; what happens when nobody answers; how long hooks and
  approvals wait. They are checked on this computer; approving always happens on the phone.
- **Settings**: notifications ("Check your phone" here, with or without a sound), open at login, the command line tool,
  the background service, the account and pairing, the version, Quit.

Settings are written to `config.toml` with `reins_desktop::settings`, keeping its comments. A change the daemon reads
only when it starts (a git host, the SSH agent, the approval wait) restarts the background service (or the daemon
inside the app), and a git host change routes git again unless paused.

### Pausing

Pause (sidebar, Overview or the tray's Pause submenu) offers 15 minutes, 1 hour, 4 hours, 24 hours and Until I resume.
Pausing stops sending git through Reins (it talks to the hosts directly); hooks and MCP tools still ask the phone. A
timed pause ends by itself (`paused_until` in `app.json`); "Until I resume" (`paused_manual`) lasts until Resume.

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
REINS_HOME=$(mktemp -d) REINS_DEMO_SCREEN=status REINS_DEMO_SECTION=activity target/debug/reins-app --demo
```

`--demo` pretends to pair and, where this computer has none yet, shows made-up activity, connections, API keys, a run
profile and AI tools; nothing is sent anywhere, pausing changes nothing on the computer and no service is restarted.
`REINS_DEMO_SCREEN=status` opens the status window straight away, and `REINS_DEMO_SECTION` (`overview`, `activity`,
`connections`, `keys`, `rules`, `settings`) picks its section. The demo changes settings only under `REINS_HOME`.
`REINS_APPEARANCE=light` or `dark` overrides the system's appearance (for screenshots of both).

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
