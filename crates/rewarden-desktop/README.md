# rewarden (desktop app)

A small daemon that sits between git and GitHub on your computer, so AI agents can clone and push without ever holding
a GitHub token, and only after your phone allowed that exact read or push.

git talks plain HTTP to the daemon on `127.0.0.1:7457`; the daemon talks HTTPS to GitHub and adds the credential. Your
phone sees what a push does (branch, fast-forward or force push, commits, files, +/− lines), worked out from the bytes
git sends, and the approval is bound to exactly those bytes. Pack data goes from your computer to GitHub directly.

## Install

```sh
curl -fsSL https://reins2fa.com/install.sh | sh
```

Linux x86_64 and arm64 (static binaries, any distro) and macOS (Apple silicon and Intel). It installs to
`~/.local/bin` (`REWARDEN_INSTALL_DIR` to change). `rewarden update` installs the latest release later; it only accepts
releases signed with the Reins release key built into the app, never an older one, and restarts the background
service.

Every [GitHub release](https://github.com/katulevskiy/reins/releases) also has the app for Linux (static, x86_64 and
aarch64) and macOS (Apple silicon and Intel), as `reins-desktop-<version>-<target>.tar.gz`, and for Windows
(`x86_64-pc-windows-msvc`, and `aarch64-pc-windows-msvc` when it built) as `reins-desktop-<version>-<target>.zip` with
`rewarden.exe`, all checked against `SHA256SUMS`.

On Windows, in PowerShell:

```powershell
irm https://reins2fa.com/install.ps1 | iex
```

### macOS (alpha)

The install script and `rewarden update` serve macOS builds too. macOS support is alpha: the builds are made and
tested on GitHub's macOS runners, but the whole flow (login, service, approvals) has not been field-tested end to end
on a real Mac yet. It works as on Linux, with the background service as a launchd agent
(`~/Library/LaunchAgents/dev.rewarden.daemon.plist`, log in `~/Library/Logs/rewarden.log`) and approvals on the
desktop as a dialog. `~/.local/bin` is usually not on a Mac's `PATH`; the script says how to add it (in `~/.zshrc`).

The program is not signed with an Apple Developer ID. A download made with `curl` (as the install script does) carries
no quarantine flag, so Gatekeeper lets it run. If you downloaded the archive with a browser instead, macOS refuses to
open it until you clear the flag: `xattr -d com.apple.quarantine rewarden`.

Two Linux-only protections are absent on macOS:

- git does not print "waiting for approval on your phone" while it waits (the daemon finds the git process through
  `/proc`, which macOS does not have); the push or clone simply waits until you answer;
- the daemon's memory is not shielded from debuggers of the same user (Linux's non-dumpable flag).

### Windows (alpha)

Windows 10 and 11, x86_64; Arm64 when the release has a build for it. `scripts/install.ps1` (above)
installs from the latest GitHub release: it checks the zip against `SHA256SUMS`, puts `rewarden.exe` in
`%LOCALAPPDATA%\Programs\Reins` (`REWARDEN_INSTALL_DIR` to change) and adds that folder to your user `PATH`; no
administrator is needed. `rewarden update` takes signed releases from the same feed as on Linux and macOS once it
serves Windows builds (`scripts/release-desktop.sh --from`).

Windows support is alpha. CI builds it and runs the crate's tests on a GitHub Windows runner, including the real
background service (installed, started, stopped and removed), the install script, the named-pipe SSH agent with
Windows' OpenSSH, and `rewarden run` with PowerShell commands. It has not been field-tested end to end on a real PC
(login with a phone, the approval dialogs, harnesses on Windows). What it does there:

- **Files**: settings in `%APPDATA%\rewarden\config.toml`, state (key, session, control token) in
  `%LOCALAPPDATA%\rewarden\`. Both folders, and every secret file, get an access list that leaves them to you alone
  (`icacls`), instead of Unix's 0700/0600.
- **Background service**: `rewarden resume` (or `rewarden service install`) makes `rewarden-daemon.exe`, a copy of
  `rewarden.exe` that Windows starts without a console window, in `%LOCALAPPDATA%\rewarden\`, adds it to
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (value `Reins`; Task Manager's Startup apps shows it) and starts
  it. It logs to `%LOCALAPPDATA%\rewarden\daemon.log`. `rewarden status` shows whether it is installed. The `Run` key
  needs no administrator and runs the daemon in your desktop session, where the approval dialogs appear (a Windows
  service would need both an administrator and a session of its own). Unlike systemd it does not restart the daemon
  when it stops; `rewarden resume` starts it again. `rewarden update` and the install script replace the running
  program by renaming it aside (Windows does not let a running program be overwritten) and restart the service.
- **Approvals on the desktop** (local mode, `rewarden ask` without a terminal): a Yes/No message box (PowerShell and
  Windows Forms, part of Windows), with No as the default; it closes when the request times out.
- **SSH agent**: a named pipe, `\\.\pipe\rewarden-ssh-agent-<id>`, that only you (and the administrators) can write
  to, refusing remote clients. `rewarden ssh setup` points Windows' own OpenSSH at it in `%USERPROFILE%\.ssh\config`
  (as `//./pipe/...`). Git for Windows' bundled ssh cannot use named pipes: for git over SSH set
  `git config --global core.sshCommand C:/Windows/System32/OpenSSH/ssh.exe`.
- **Harnesses**: the same files under `%USERPROFILE%`; the hook command is the program's path with forward slashes,
  which Git Bash, cmd and PowerShell all run ([harnesses.md](../../docs/harnesses.md#windows)). The guard knows
  PowerShell's and cmd's recursive deletes and reads backslash paths.
- **`rewarden run`** finds `.cmd` and `.bat` programs (`npm`), and keeps waiting through Ctrl-C (which reaches the
  command too) so its exit code comes back.

Missing on Windows, as on macOS: git does not print "waiting for approval on your phone" while it waits (the daemon
finds the git process through Linux's `/proc`), and the daemon's memory is not shielded from programs running as you
(that needs a process access list, set with Win32 calls this crate does not make: it has no unsafe code). The program
is not signed with a code signing certificate: a zip downloaded with a browser carries Windows' mark of the web, and
SmartScreen may warn before the first start (More info → Run anyway, or **Unblock** in the file's properties).

## Set up

```sh
rewarden login     # scan the QR code with the phone; the phone shows a key: it must match the terminal
rewarden resume    # start the background service, send github.com git through it
rewarden status
```

`rewarden login` pairs with the hosted server, `https://app.reins2fa.com`; for a self-hosted one, give its address
(`rewarden login https://reins.example.com`). It shows a QR code (OAuth device authorization, RFC 8628): the phone
scans it, the user taps the number shown in the terminal and compares the key. `rewarden login --browser` uses the
browser sign-in instead (also what a server without the device flow gets). The desktop window uses the same flow
through `server::device::DevicePairing` (`start`, then draw `qr_url`, for example with `qr_modules`, then `wait`).

`rewarden pause` switches git back to talking to GitHub directly (it removes exactly the git config lines `resume`
added; the service keeps running and does nothing); `rewarden resume` switches back. For finer control: `rewarden
service install|uninstall` and `rewarden git setup|unsetup [--repo DIR]` (one repository only).

Nothing else changes for git or the agent: `git clone https://github.com/me/app`, `git push` work as before (SSH
remotes `git@github.com:` are rewritten to HTTPS too). `rewarden git unsetup` undoes the rules.

On the phone you decide per repository, per branch, for a time; read and write separately. Force pushes, deleted
branches and moved tags are asked every time. If you take longer than `approval_timeout_secs` (120 s) git stops with
"waiting for approval on your phone"; approve, then run git again: the same push is not asked twice.

## Secrets, API keys and SSH keys from your phone

These need `rewarden login` (they live in the vault on your phone; local mode has none).

**One command with secrets.** `rewarden run` asks your phone for the secrets, runs the command with them as
environment variables, and exits with its code (125: not released, 126/127: the command could not run). The values
are wiped from `rewarden` once the command started and never printed.

```sh
rewarden run --env OPENAI_API_KEY=vault:OpenAI/password -- python agent.py
rewarden run --profile deploy -- ./deploy.sh
```

A reference is `vault:<item name or id>/<field>` (`password`, `username`, `totp`, `notes`, `uri`, or a custom field).
Profiles in `config.toml`:

```toml
[run.profiles.deploy]
purpose = "deploy the site"          # shown on the phone
env = { AWS_ACCESS_KEY_ID = "vault:AWS/username", AWS_SECRET_ACCESS_KEY = "vault:AWS/password" }
```

**APIs without keys in the agent.** The daemon forwards `http://127.0.0.1:7457/api/<name>/<path>` to `<base>/<path>`
and adds the header with a key your phone released; it keeps the key in memory until the lease ends (or the API
answers 401), never logs it, and streams bodies both ways. Point the agent's base URL at the proxy
(`OPENAI_BASE_URL=http://127.0.0.1:7457/api/openai`).

```toml
[[api]]
name = "openai"
base = "https://api.openai.com/v1"
header = "Authorization: Bearer {secret}"   # the default
secret = "vault:OpenAI/password"
lease_secs = 3600
```

**SSH keys that stay on the phone.** The daemon runs an SSH agent on `$XDG_RUNTIME_DIR/rewarden/ssh-agent.sock`
(0600; on Windows a named pipe, see [Windows](#windows-alpha)). It lists your vault's SSH keys, and your phone signs each login after showing the server (named from
`~/.ssh/known_hosts` when the entry is not hashed). `rewarden ssh setup` adds a marked `Host *` / `IdentityAgent` block
at the end of `~/.ssh/config` (more specific settings earlier in the file still win); `rewarden ssh unsetup` removes
exactly that block; `rewarden ssh status` shows both. `[ssh] enabled = false` turns the agent off; `socket` and
`known_hosts` override the paths.

While your phone decides, the waiting command (git, ssh, the agent's HTTP client, `rewarden run`) gets a
"waiting for approval in your Reins app" line on its stderr.

## Without a phone

Without `rewarden login`, a local policy decides and asks on the desktop (a notification with Approve/Deny on Linux, a
dialog on macOS, a Yes/No message box on Windows, or `rewarden pending` / `rewarden approve <id>`). The GitHub token comes from `gh auth token`. Settings
in `~/.config/rewarden/config.toml` (on Windows `%APPDATA%\rewarden\config.toml`):

```toml
mode = "auto"              # auto: the phone when logged in, else local | local | rewarden
approval_timeout_secs = 120

[github]
token = "gh"               # gh | env:GITHUB_TOKEN | file:/path/to/token

[policy]
read = "allow"             # allow | ask | deny
push = "ask"
risky = "ask"              # force pushes, deletions, moved tags

[[policy.rules]]           # the first matching rule decides; * within a part, ** across parts
repo = "me/*"
branch = "feature/**"
push = "allow"
```

## AI harnesses: MCP, hooks and `ask`

```sh
rewarden harness add claude-code   # or codex, gemini, cursor; `remove` undoes exactly that, `list` shows it
```

`add` registers `rewarden mcp --via <harness>` as an MCP server in the harness's user settings (`~/.claude.json`,
`~/.codex/config.toml`, `~/.gemini/settings.json`, `~/.cursor/mcp.json`) and `rewarden hook <harness>` as its
pre-command hook (`~/.claude/settings.json` `PreToolUse`, `~/.codex/hooks.json` `PreToolUse` (trust it once with
`/hooks`), `~/.gemini/settings.json` `BeforeTool`, `~/.cursor/hooks.json` `beforeShellExecution`, `beforeReadFile`,
`preToolUse`). Each change is one inserted entry; the rest of the file stays as it was, and `remove` gives back files
that existed byte for byte and deletes the ones `add` created.

- `rewarden mcp` passes MCP messages to your Reins server with this app's session (renewed as needed); your phone
  shows which harness asks.
- `rewarden hook` asks your phone before risky commands (force pushes, `reset --hard`, `rm -r`, `terraform
  apply|destroy`, `kubectl apply|delete`, `DROP TABLE`, publishing…) and before reading or changing secret files
  (`.env`, keys, `~/.ssh`, cloud credentials). Everything else goes through without a question.
- `rewarden ask "Deploy to production?" [--detail TEXT|-] [--topic T] [--timeout S]` asks anything: exit 0 yes, 1 no, 2
  no answer. Without a phone it asks in the terminal, or with a desktop notification.

```toml
[guard]
defaults = true                 # the built-in rules, plus:
commands = ["make deploy", "text:shutdown"]   # shell words, `*` in a word, `-x` matches `-xyz`; text: anywhere
files = ["*.sqlite"]            # a name, or the end of a path (`.aws/credentials`)
allow_commands = ["git push --force* origin feature/*"]
allow_files = ["fixtures/*.key"]
timeout_secs = 120
on_no_answer = "deny"           # deny | ask (leave it to the harness's own prompt, where it has one)
```

## What it protects against

- The agent never sees a token: it is added inside the daemon and kept in memory only (phone mode).
- In phone mode the token is sealed by the phone to this app's key (pinned at login, after you compared the digits),
  so the Reins server relaying it cannot read it, and an answer cannot be replayed for another push.
- Local mode protects against an agent's mistakes, not a hostile one: a process running as you could read the token
  source or answer the local prompt. For a hostile agent use the phone, and for full separation run the daemon as its
  own OS user.
- The daemon only listens on loopback and refuses browsers (Host and Origin checks).

## Publishing a release (maintainers)

`scripts/release-desktop.sh` builds static binaries from the current commit, signs the manifest with the release key
(`~/.config/rewarden-release/release-signing.pk8`, never in the repository) and uploads it; installs and updates use it
at once. `scripts/release-desktop.sh --macos-from vX.Y.Z` (or `latest`) publishes a GitHub release instead: the Linux
binaries built here from its tag, the macOS and Windows ones taken from its archives (checked against its
`SHA256SUMS`; Windows on Arm only when the release has it), all with the GitHub release's version and build id, in one
signed manifest (`--from` is the same flag). It also uploads `install.ps1` next to `install.sh`; the site has to serve
it as `/install.ps1` the way it serves `/install.sh`. The script's header explains how versions compare. Losing the key means existing installs can no longer update themselves (they would need the install script
again with a new key built in).
