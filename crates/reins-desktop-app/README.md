# Reins desktop app

The app for people who never open a terminal: a welcome flow that pairs this computer with the phone (a QR code),
connects the AI tools it finds, starts the background service and sends git through it, and afterwards a status
window (with health checks, a test to the phone and work sessions) and a shield in the menu bar (macOS) or tray
(Windows, Linux) with a Pause submenu, Resume, End work session, Open Reins and Quit. It is built with
[GPUI](https://gpui.rs) through Zeron's fork ([zui](https://github.com/katulevskiy/zui)), in the phone apps' look: Geist,
cool neutrals, one violet accent.

It does its work through the `reins_desktop` library in its own process and through the running daemon's control
API; it never runs `reins` for that. The `reins` program ships next to it (inside `Reins.app`, in the install
directory on Windows, inside the AppImage): it is what the harnesses and the background service run, and the status
window's "Install command line tool" links it to `~/.local/bin`.

| Module | What |
| --- | --- |
| `pairing` | the device flow (`reins_desktop::server::device`, feature `device-flow`), the browser sign-in, `--demo` |
| `backend` | harness detection and setup, the service (or the daemon inside the app), pause and resume, the daemon's status and overview, the activity log, the running work session, starting and ending one, settings changes (restarting the service when the daemon needs it), updates |
| `model` | the app's state: screen and section, pairing, setup, the health checks and the test, pausing, the window's place, the actions the window and tray take; `model/work_actions.rs`: the work session card's actions (the form, asking the phone, ending) |
| `work` | work sessions as the window shows them: the start form and the request it makes (default branches refused), the repositories git reached lately, time left, the tray's first line |
| `welcome` | the welcome flow's steps (and where a start resumes), the AI tools step's rows |
| `health` | how the health checks add up ("All good", "2 things to look at", "Needs fixing"), the fixes the window does itself, how a test ended |
| `shortcuts` | the keyboard shortcuts and how they are written on this computer |
| `pause` | the pause lengths and how a pause reads ("Paused for 3 h 12 min more", "Paused until you resume") |
| `format` | times as the window says them: "2 min ago", "3 h 12 min", "14:32" |
| `demo` | `--demo` only: made-up activity, connections, keys and AI tools where this computer has none |
| `ui/mod.rs` | the window: one column before setup, sidebar and sections afterwards |
| `ui/parts.rs` | building blocks: cards, rows, buttons, toggles, chips, segmented controls, outcome badges |
| `ui/field.rs` | one-line text fields typed by hand (the server, the Rules lists, the work session's reason and branches) |
| `ui/onboarding.rs`, `ui/setup.rs` | the welcome flow: pairing with the phone; the AI tools, turning on, done |
| `ui/health.rs` | the health card, "Send a test to my phone", Settings' "Run checks" |
| `ui/work.rs` | Overview's work session card: the form, waiting for the phone, the session running |
| `ui/sidebar.rs` | the mark and state, the sections, pausing |
| `ui/overview.rs`, `ui/activity.rs`, `ui/connections.rs`, `ui/keys.rs`, `ui/rules.rs`, `ui/settings.rs` | the sections |
| `tray` | `tray-icon` on macOS and Windows, `ksni` (StatusNotifierItem) on Linux |
| `autostart` | open at login: a launch agent, the `Run` key (`Reins app`), an XDG autostart entry |
| `upgrade` | "Restart to update": the new installer in place of the running copy |

## The welcome flow

The first start walks through four steps, shown at the top as **1 Pair · 2 AI tools · 3 Turn on · Done**:

1. **Pair**: the QR code (the device flow) and its code beside the three things to do: open Reins on the phone, scan
   the code, tap the number the window shows. This computer's key is there to compare with the one the phone shows.
   "Sign in with browser instead" and "Use another server" are the quieter ways.
2. **Connect your AI tools**: every harness Reins knows (Claude Code, Codex, Gemini CLI, Cursor), installed or not. A
   **Connect** button adds Reins to an installed one right away (`harness::add`) and turns into "Connected" with
   **Undo** (`harness::remove`), with the note to restart that tool; **Connect all** connects the rest. Errors show
   on the row. A copy of Reins in a disk image or other temporary place cannot connect (it would not be there after a
   restart): the step says to install Reins first.
3. **Turn on Reins**: starts the background service, sends git through it and opens Reins at login (the last two can
   be left out), each step ticked as it goes. When something fails: **Try again** or **Continue anyway**.
4. **Reins is on**: what was set up, **Send a test to my phone**, and **Open Reins** for the status window.

`app.json` keeps the step reached (`setup_step`): quitting half-way opens the same step next time. Pairing again
starts over at step 2.

## The status window

After pairing and setup the window (resizable, 1000 × 680 at first) has a sidebar with the state (On, Paused, Needs
you; "Work session · 1 h 12 min" under it while one runs, which goes to Overview), the sections and the pause control,
and these sections. Everything is read again every 3 seconds: the daemon's
status and overview (`control::Client::overview`), and this computer's activity log (`reins_desktop::journal`, the
newest 500).

- **Overview**: the state in a sentence and what fixes it, what waits for the phone now (and for how long), the work
  session card, the health card and the test to the phone, today's approved, denied, timed out and failed requests, pausing, the latest
  requests, the update banner.
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
  the background service, the account and pairing, the health checks ("Run checks"), the keyboard shortcuts, the
  version, Quit.

Empty sections say what they are for and how to start: an empty Activity offers the test to the phone, Keys & secrets
shows the `config.toml` entries to add (with "Open config.toml"), Connections points to where the APIs and the SSH
agent are set up.

Settings are written to `config.toml` with `reins_desktop::settings`, keeping its comments. A change the daemon reads
only when it starts (a git host, the SSH agent, the approval wait) restarts the background service (or the daemon
inside the app), and a git host change routes git again unless paused.

### Health and the test

The health card runs `reins_desktop::doctor` (what `reins doctor` checks: paired, the session, the server, the clock,
the phone seen lately, the background service and its version, git routed, each installed AI tool connected,
notifications) on the tokio runtime: when the status window opens, then once a minute while it is open, and on
**Check again** (or Ctrl/Cmd+R). It never runs in the 3-second refresh. It shows the overall state (All good, N things
to look at, Needs fixing) and every check that is not OK with what was found. Where the window can fix it, a button
does: **Start** or **Restart** the service, **Resume** git, **Connect** an AI tool, **Pair again**; the checks run again
afterwards. Otherwise it says what to do.

**Send a test to my phone** (Overview, the welcome flow's last step, an empty Activity) sends `reins test`'s harmless
question (`ask::send_test`, logged in the activity log like any other) and waits up to 90 seconds with a countdown:
"Approved on your phone. Reins works end to end.", "Denied on your phone — that's how a denial stops an agent.", "No
answer within 90 s — run the checks above", or why it could not ask.

### Work sessions

Before focused work the user approves on the phone, once, a time-boxed bundle (`reins_desktop::work_session`, as
`reins allow` does): reading chosen integrations and git pushes to named branches. The phone turns it into ordinary
grants that end together; force pushes, deleting, the vault and purchases still ask every time.

Overview's **Work session** card says so in a line, with **Start a work session**. Its form:

- **How long**: 30 min, 1 hour, 2 hours (the default), 4 hours, 8 hours.
- **What it's for**: one line; empty is "Focused work".
- **Read**: Mail (`gmail`), Calendar (`gcalendar`), GitHub (`github`), off at first. Integrations not connected on the
  phone are left out (the session then lists them as skipped).
- **Push with git**: the repositories this computer reached with git lately (the daemon's connections and the
  activity log's git requests, newest first, on the git hosts `config.toml` knows, which name the service), each with
  a branch field. Only rows with a branch are included; a default branch (`main`, `master`, ...) is refused on the row
  ("pushes to main still ask; name a feature branch"). Without any, the form says that `reins allow 2h` in a
  repository adds its branch.

**Ask my phone** (enabled once something is chosen and nothing is refused; Enter in a field does the same) runs
`work_session::start` on the tokio runtime and shows "Waiting for your phone… approve the work session there" with the
approval wait's countdown (`approval_timeout_secs`). A refusal, a timeout or a missing pairing shows on the card with
**Try again** and **Change**. Once approved, the card shows the session: what it is for, the time left ("1 h 12 min
left", moving on with the refresh), until when on this computer's clock, what it allows and what was left out, and
**End now**, which asks first ("End the session? Its permissions end on your phone now." **End** / **Keep**) and
then runs `work_session::end`. The running session is read with every refresh (`work_session::current`,
`work-session.json`); when it runs out the card offers a new one.

### Keyboard shortcuts

Cmd (macOS) or Ctrl (Windows, Linux) with **1**…**6** goes to a section, **R** checks again and reads everything again,
**W** closes the window (Reins keeps running in the tray), **Q** quits; **Esc** closes an open activity row. The
sidebar shows a section's shortcut under the pointer, and Settings lists them all.

### The tray icon

The shield shows the state: a check while Reins is on, pause bars while paused, three dots and an amber dot while a
request waits on the phone (the menu's first line and the tooltip then say "Waiting on your phone: …"), and an
exclamation mark while it needs the user (not paired, the service not running). While a work session runs and nothing
needs the user or waits on the phone, the first line and the tooltip say "Work session: 1 h 12 min left"; **End work
session** (enabled while one runs) ends it at once. macOS uses the template images (`tray-*.png`), the others the
coloured ones (`tray-color-*.png`).

### The window

The window opens where it was and as big as it was (`window` in `app.json`), at least its smallest size and at most the
display; it opens centred when that display is gone. Wayland does not let apps place windows: there only the size
comes back.

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
REINS_HOME=$(mktemp -d) REINS_DEMO_SCREEN=done REINS_DEMO_TEST=1 target/debug/reins-app --demo
REINS_HOME=$(mktemp -d) REINS_DEMO_SCREEN=status REINS_DEMO_SESSION=1 target/debug/reins-app --demo
```

`--demo` pretends to pair and, where this computer has none yet, shows made-up activity, connections, API keys, a run
profile and AI tools; nothing is sent anywhere, pausing changes nothing on the computer and no service is restarted.
In the welcome flow it pretends three AI tools are installed; connecting, undoing and turning on change nothing on the
computer. The health card shows sample checks with one AI tool to connect (its Connect button fixes it), and the test
is answered "Approve" after 4 seconds.

`REINS_DEMO_SCREEN` opens a screen straight away: `pair`, `tools` (`tools-connected`: with one just connected, Undo
showing), `turn-on` (`turning-on`: turning on as it opens), `done` or `status`; `REINS_DEMO_SECTION` (`overview`,
`activity`, `connections`, `keys`, `rules`, `settings`) picks the status window's section. `REINS_DEMO_TEST=1` sends
the test as the app opens, `REINS_DEMO_EMPTY=1` leaves the samples out (the empty states). `REINS_DEMO_SESSION=1`
starts with a work session running ("Fix the login bug", 1 h 12 min left), `form` with the form open and filled in,
`waiting` with it sent; in the demo the pretend phone approves a session after 3 seconds (a minute with `waiting`) and
**End now** ends it, without sending or writing anything. The demo changes settings
only under `REINS_HOME`. `REINS_APPEARANCE=light` or `dark` overrides the system's appearance (for screenshots of
both).

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
