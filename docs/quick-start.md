# Quick start

This guide takes you from nothing to an agent whose actions wait for your phone. The apps use the hosted server,
`https://app.reins2fa.com`, unless you tell them otherwise. If you run your own, see [self-hosting](self-hosting.md) for
pointing them at it.

You need:

- an Android phone (Android 12 or later);
- for the desktop app, a Linux computer (x86_64 or aarch64), a Mac (Apple silicon or Intel) or a Windows 10 or 11
  computer (x86_64; Arm when the release has a build for it).

## 1. Install the phone app and create an account

1. On the phone, open <https://reins2fa.com/app> and install the APK (Android asks you to allow installs from your
   browser once). The app updates itself from the same place.
2. Tap **Continue** and sign in with Google, Apple, GitHub or a code sent to your email. There is no password: the
   first time, the phone makes your account and the keys of its vault by itself and keeps them. **Settings →
   Account → Recovery code** shows the code that opens the vault if you lose the phone; write it down. **Use another
   server** is for your own server, which may also offer a master password instead.
3. Signing in makes this phone your **approval device**. Only one phone per account is the approval device. On a
   second phone, **Continue** finds the account's keys on the first one: tap **Ask my other phone**, check that both
   show the same six digits, and approve on the first phone (or enter the recovery code). The new phone then takes
   over the role, and the old one is told it was replaced. **Use this phone for approvals** in the settings moves it
   back.
4. Allow notifications. Requests arrive as notifications, even when the app is closed.

The app supports authenticator-app (TOTP) two-factor login. Other second factors are not supported by the app yet.

## 2. Connect services on the phone

Go to **Activity → Integrations**. Each service is connected on the phone and its credentials stay there:

| Service | How it connects |
|---|---|
| Gmail, Google Calendar, Google Contacts | Android's account picker, then Google's consent screen. Several Google accounts can be added. |
| GitHub | A personal access token you create on GitHub. The app opens the page with the settings filled in and picks the token up from the clipboard. |
| GitLab, Codeberg, Bitbucket | A token, for git through the desktop app. |
| Telegram | Your own account: phone number, code, and 2-step password if you have one. |
| Phone calendar, contacts, SMS | Android permissions, asked when you connect each one. |
| Password vault | Nothing to enter for an account made with **Continue**. Otherwise your master password, entered once. The phone keeps the vault key sealed, not the password. |
| Other MCP servers | Add the server's URL. The app signs in with OAuth or a token you give it. |

## 3a. Connect a cloud AI (Claude.ai, ChatGPT)

- **Claude.ai**: Settings → Connectors → Add custom connector, URL `https://app.reins2fa.com/mcp`.
- **ChatGPT**: Settings → Connectors (developer mode), same URL.

A browser page asks for your account email and shows a two-digit code. Your phone shows three codes. Tap the one that
matches, give the connection a name, and confirm. From then on that AI has Reins's tools for the services you
connected, and each call waits for your phone unless a standing permission covers it.

## 3b. Install the desktop app

```sh
curl -fsSL https://reins2fa.com/install.sh | sh
```

It works on Linux and macOS. The script downloads the build for your computer, checks its SHA-256, and installs
`rewarden` to `~/.local/bin` (set `REWARDEN_INSTALL_DIR` to change that). Later updates: `rewarden update`. It installs
only releases signed with the release key built into the program.

On a Mac, `~/.local/bin` is usually not on your `PATH`; the script prints the line to add to `~/.zshrc`. macOS support
is alpha: built and tested on GitHub's macOS runners, not yet field-tested end to end on a real Mac. Two things differ
from Linux: git does not print "waiting for approval on your phone" while a push or clone waits (it just waits), and
the daemon's memory is not shielded from debuggers running as you. See the
[desktop app README](../crates/rewarden-desktop/README.md#macos-alpha).

Or download it from the [latest GitHub release](https://github.com/katulevskiy/reins/releases/latest): the
`reins-desktop-<version>-<target>.tar.gz` archive for your computer (`x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, `aarch64-apple-darwin` for Apple silicon, `x86_64-apple-darwin` for Intel Macs), checked
against `SHA256SUMS`. Unpack it and put `rewarden` on your `PATH`, for example in `~/.local/bin`. On a Mac, an archive
downloaded with a browser is quarantined and Gatekeeper refuses the unsigned program; clear the flag with
`xattr -d com.apple.quarantine rewarden` (`curl` downloads, like the install script's, are not quarantined).

On Windows, in PowerShell (no administrator needed):

```powershell
irm https://reins2fa.com/install.ps1 | iex
```

The script downloads the latest GitHub release's `reins-desktop-<version>-x86_64-pc-windows-msvc.zip` (or
`aarch64-pc-windows-msvc` on Arm), checks it against the release's `SHA256SUMS`, installs `rewarden.exe` to
`%LOCALAPPDATA%\Programs\Reins` (set `REWARDEN_INSTALL_DIR` to change that) and adds that folder to your user `PATH`.
Open a new terminal afterwards. `$env:REWARDEN_VERSION = "v0.2.0"` before it installs that release instead of the
latest. Later updates: `rewarden update` (from the same signed release feed as on Linux and macOS) or the script
again. You can also
unpack the zip yourself and put `rewarden.exe` anywhere on your `PATH`. Windows support is alpha: built and tested on
GitHub's Windows runners, not yet field-tested end to end on a real PC; see the
[desktop app README](../crates/rewarden-desktop/README.md#windows-alpha) for what differs.

Pair it with your phone:

```sh
rewarden login
```

It pairs with the hosted server; for your own, add its address (`rewarden login https://reins.example.com`). The
terminal shows a QR code. Scan it with the phone's camera, or in the app (**Settings → Connect a computer**; without a
camera, type the code printed under it). The phone shows three numbers: tap the one the terminal shows. The terminal
also prints the app's key, for example `4821 9930`, and the phone shows the same digits in a **Desktop app key** card.
**Approve only if the digits match.** On approval the phone pins this key to the connection. From then on only this
computer can open the credentials the phone sends it. `rewarden login --browser` signs in through a browser page
instead (enter your email there; `--no-browser` prints the link instead of opening it).

Start the background service and send git through it:

```sh
rewarden resume     # installs the background service if needed (systemd user unit, launchd agent on a Mac, or
                    # on Windows an entry in your Run key), then routes github.com remotes through it
rewarden status     # version, who decides, server, key, pending local approvals, git routing
```

On Windows the background service is a windowless copy of `rewarden.exe` in `%LOCALAPPDATA%\rewarden`, started at
once and at every logon from `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (Task Manager lists it under
Startup apps as `rewarden-daemon.exe`). Its log is `%LOCALAPPDATA%\rewarden\daemon.log`. Unlike systemd it is not
restarted when it stops; `rewarden status` says so and `rewarden resume` starts it again.

`rewarden resume` adds `url.<proxy>.insteadOf` rules to your global git config. `https://github.com/...`,
`git@github.com:...` and `ssh://git@github.com/...` remotes then go to the daemon at `http://127.0.0.1:7457/github.com/`.
`rewarden pause` removes exactly those rules, and git talks to GitHub directly again. To route only one repository:
`rewarden git setup --repo DIR` (undo with `rewarden git unsetup --repo DIR`).

Try it: clone a private repository or push a branch. Your phone shows the repository, the branch, the commits and the
changed files. Approve, then run the git command again if git stopped waiting. The daemon keeps git waiting up to
`approval_timeout_secs` (120 s by default).

## 4. Connect a local agent

```sh
rewarden harness add claude-code      # or: codex, gemini, cursor
```

This registers `rewarden mcp` as an MCP server and `rewarden hook` as the harness's pre-tool hook. Restart the
harness. For Codex, also trust the new hook once with `/hooks`. See [harnesses.md](harnesses.md) for what is
written where and how to undo it.

Now ask the agent to do something with a connected service ("list my open pull requests"), or to run a command the
guard watches (`git push --force`). The request appears on your phone.

## More from the desktop app

### Ask a question from a script

```sh
rewarden ask "Deploy build 142 to production?" --detail "make deploy ENV=prod" --topic "command:make deploy"
echo $?   # 0 yes, 1 no, 2 no answer
```

When logged in, the question goes to your phone. Otherwise it is asked in the terminal or as a desktop notification.
`--detail -` reads the detail from stdin. `--topic` names what a standing answer on the phone may cover.

### Run a program with secrets from the vault

```sh
rewarden run --env OPENAI_API_KEY=vault:OpenAI/password --purpose "eval run" -- python eval.py
```

The phone shows the command, the purpose and the item and field names (never the values). When you approve, the
values are released sealed to this computer and set as environment variables for that one command, then wiped from
`rewarden`'s memory. Fields: `password`, `username`, `totp` (the current code), `notes`, `uri`, or a custom field's
name. The item is named by its id or exact name.

Named sets go in `~/.config/rewarden/config.toml` (on Windows `%APPDATA%\rewarden\config.toml`):

```toml
[run.profiles.openai]
env = { OPENAI_API_KEY = "vault:OpenAI/password" }
purpose = "OpenAI scripts"
```

```sh
rewarden run --profile openai -- python script.py
```

`rewarden run` exits with the command's own exit code. It exits with 125 when the secrets were not released. On
Windows a bare name also finds `.cmd` and `.bat` programs (`rewarden run ... -- npm test` runs `npm.cmd`).

### Call an API without handing out its key

```toml
[[api]]
name = "openai"
base = "https://api.openai.com"
secret = "vault:OpenAI/password"
# header = "Authorization: Bearer {secret}"   (the default)
# lease_secs = 3600                           (60 to 86400)
```

The agent calls `http://127.0.0.1:7457/api/openai/v1/models` with no key. The daemon asks the phone for the secret once
per lease, adds the header, and forwards the request.

### SSH with keys that stay on the phone

```sh
rewarden ssh setup     # adds an IdentityAgent block to ~/.ssh/config (rewarden ssh unsetup removes it)
rewarden ssh status
```

The daemon runs an SSH agent whose identities are the SSH key items in your vault. Each signature is made on the
phone after you approve it. The phone shows the key and the server you are connecting to.

On Windows the agent is a named pipe (`\\.\pipe\rewarden-ssh-agent-...`, `rewarden ssh status` shows it), and
`rewarden ssh setup` points Windows' own OpenSSH (`C:\Windows\System32\OpenSSH\ssh.exe`) at it through
`%USERPROFILE%\.ssh\config`. Git for Windows brings its own ssh, which cannot use the pipe; for git over SSH set
`git config --global core.sshCommand C:/Windows/System32/OpenSSH/ssh.exe` (git through `rewarden resume` uses HTTPS
and needs no agent). Tools that read `SSH_AUTH_SOCK` instead can be given the pipe's name.

### Without a phone: local mode

When the desktop app is not logged in, git requests are decided by a local policy. The default policy allows reads,
and asks about pushes and risky pushes. The questions appear as desktop notifications (a dialog on macOS, a Yes/No
message box on Windows), or you answer them with
`rewarden pending`, `rewarden approve <id>`, `rewarden deny <id>`. The GitHub token comes from `gh auth token` by
default (`[github] token = "env:NAME"` or `"file:PATH"` to change that). Local mode protects against an agent's
mistakes, not against a hostile agent (see [security-model.md](security-model.md)).

## Undo everything

```sh
rewarden harness remove claude-code   # each harness you added
rewarden ssh unsetup
rewarden pause                        # git talks to the hosts directly again
rewarden service uninstall
rewarden logout
rm ~/.local/bin/rewarden
```

On Windows the last step is deleting `%LOCALAPPDATA%\Programs\Reins` and taking it out of your user `PATH` (Settings →
System → About → Advanced system settings → Environment Variables).

Settings live in `~/.config/rewarden/` and state (the app's key, the session) in `~/.local/state/rewarden/` (on
Windows `%APPDATA%\rewarden\` and `%LOCALAPPDATA%\rewarden\`). Delete both to forget everything. On the phone, remove the connection under Settings to revoke the computer's access.
