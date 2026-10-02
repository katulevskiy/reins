# Harnesses

Reins connects to AI harnesses in two ways:

- **MCP tools.** The agent gets tools for the services connected on your phone (`gmail_search`, `github_pr_create`,
  `calendar_create_event`, ...) plus Reins's own tools (`rewarden_get_result`, `rewarden_request_access`,
  `rewarden_list_accounts`, `rewarden_upload`). Every call goes to your phone.
- **Hooks.** Before the harness runs a shell command or touches a file, it asks `rewarden hook <harness>`. Commands and
  files that match the guard rules go to your phone. Everything else passes through untouched.

Git needs neither: once `rewarden resume` (or `rewarden git setup`) is done, every git command on the computer goes
through the desktop app, whichever harness runs it.

## Prerequisites

```sh
curl -fsSL https://reins2fa.com/install.sh | sh
rewarden login     # the hosted server; on your own: rewarden login https://reins.example.com
```

On Windows, in PowerShell: `irm https://reins2fa.com/install.ps1 | iex`, then the same `rewarden login`.

Without `rewarden login`, the MCP server has no server to reach, and the hook asks in a desktop notification instead
of on your phone.

## Commands

```sh
rewarden harness add <harness>       # register the MCP server and the hook
rewarden harness add --all           # the same for every harness found on this computer
rewarden harness remove <harness>    # take out exactly what add put in
rewarden harness list [<harness>]    # what is set up, for one harness or all
```

`<harness>` is `claude-code`, `codex`, `gemini` or `cursor` (`claude` and `gemini-cli` also work). A harness counts as
found when its settings directory is in your home directory (`~/.claude` or `~/.claude.json`, `~/.codex`, `~/.gemini`,
`~/.cursor`), its program is on the `PATH` or in a usual install directory (`~/.local/bin`, `/opt/homebrew/bin`,
`/usr/local/bin`, npm's), or, for Cursor, the app is installed. The install script and the Reins app add Reins to
exactly these.

`add` edits the harness's settings files as text. It inserts one entry per file and leaves the rest of the file as
it was. It records what it changed in `~/.local/state/rewarden/harnesses.json`. `remove` uses that record to restore
each file byte for byte, as long as nobody edited around the entry in the meantime. If an entry named `rewarden`
already exists with a different value, `add` stops and says so. It does not overwrite.

The MCP server entry runs `rewarden mcp --via "<Harness name>"`, a stdio MCP server that forwards to
`<server>/mcp` with the desktop app's session. The name is shown on your phone ("Laptop · Claude Code"). The hook
entry runs `rewarden hook <harness>` with a timeout 30 s longer than the guard's own.

### Windows

`~` is your profile folder (`%USERPROFILE%`, for example `C:\Users\me`): the harnesses keep their settings there on
Windows too, so the files below are the same (`C:\Users\me\.claude.json`, `C:\Users\me\.codex\config.toml`, ...). The
record of what `add` changed is in `%LOCALAPPDATA%\rewarden\harnesses.json`, and the guard settings in
`%APPDATA%\rewarden\config.toml`.

The MCP server entry names `rewarden.exe` by its full path. Harnesses run hook commands through a shell, and which one
differs (Claude Code uses Git Bash, or PowerShell without it; others use PowerShell or cmd), so the hook command is
written in the one form all of them run: the path with forward slashes and no quotes,
`C:/Users/me/AppData/Local/Programs/Reins/rewarden.exe hook claude-code`. When the path needs quotes (a space in your
user name), the hook runs plain `rewarden` if that folder is on your `PATH` (the install script puts it there), else
the quoted path, which Git Bash and cmd run but PowerShell does not. Run `rewarden harness add` again after moving
`rewarden.exe`.

## Claude Code

```sh
rewarden harness add claude-code
```

| What | Where |
|---|---|
| MCP server `rewarden` (stdio) | `~/.claude.json`, `mcpServers` |
| `PreToolUse` hook, matcher `Bash\|Edit\|Write\|MultiEdit\|NotebookEdit\|Read` | `~/.claude/settings.json`, `hooks.PreToolUse` |

Restart Claude Code. `/mcp` and `/hooks` show the new entries.

The hook answers with Claude Code's `permissionDecision`: `allow` when you approved on the phone, `deny` when you
refused, `ask` (Claude Code's own prompt) when the guard is set to `on_no_answer = "ask"` and nobody answered.
Commands that match no rule get no answer from the hook, so Claude Code's normal permission rules apply.

Alternative without the desktop app: Claude Code can also use the server's MCP endpoint directly as a remote HTTP
server. It signs in through the browser with the same two-digit code. That gives you the tools but no hook.

## Codex

```sh
rewarden harness add codex
```

| What | Where |
|---|---|
| MCP server table `[mcp_servers.rewarden]` | `~/.codex/config.toml` |
| `PreToolUse` hook, matcher `^(Bash\|apply_patch)$` | `~/.codex/hooks.json`, `hooks.PreToolUse` |

Restart Codex, then **trust the hook once with `/hooks`**. Codex runs only hooks you have reviewed. For `apply_patch`
the guard checks every file the patch adds, updates or deletes. Codex hooks cannot answer "ask". When the guard would
ask and nobody answered, the hook stays silent and Codex's own approval policy decides.

## Gemini CLI

```sh
rewarden harness add gemini
```

| What | Where |
|---|---|
| MCP server `rewarden` | `~/.gemini/settings.json`, `mcpServers` |
| `BeforeTool` hook, matcher `^(run_shell_command\|write_file\|replace\|read_file)$` | `~/.gemini/settings.json`, `hooks.BeforeTool` |

Restart Gemini CLI. `/mcp` and `/hooks` show the entries. Gemini's hook answers are `allow` or `deny`. "No answer"
with `on_no_answer = "ask"` leaves the decision to Gemini CLI.

## Cursor

```sh
rewarden harness add cursor
```

| What | Where |
|---|---|
| MCP server `rewarden` (stdio) | `~/.cursor/mcp.json`, `mcpServers` |
| hooks `beforeShellExecution`, `beforeReadFile`, `preToolUse` (matcher `Write\|Delete`) | `~/.cursor/hooks.json` (`"version": 1` is added if missing) |

Restart Cursor. Settings → MCP and Hooks show the entries. Cursor's file hooks know only `allow` and `deny`, so
"ask" becomes `deny` there. As with the other harnesses, commands and file changes that match no rule get no answer,
so your own Cursor approval settings still apply to them (Cursor logs the empty answer as a hook that did not decide
and goes on). Reads that match no rule get an explicit `allow`, since Cursor never asks before reading a file.

## Cloud AIs

Claude.ai and ChatGPT connect to `https://<server>/mcp` as a custom connector (OAuth, with the two-digit code on your
phone). They get the tools but not hooks, since they do not run commands on your computer. Other MCP clients that
support remote servers with OAuth 2.1 (dynamic client registration or client ID metadata documents) can connect the
same way. One limit: requests sent from a web page are accepted only from the Claude and ChatGPT origins.

## The guard rules (`[guard]`)

The hook sends a command or file to your phone when it matches a rule in `~/.config/rewarden/config.toml`:

```toml
[guard]
defaults = true                 # use the built-in rules below as well as yours
commands = ["make deploy", "text:delete from"]
files = ["secrets/*"]
allow_commands = []             # exceptions, checked first
allow_files = ["*.pub"]
timeout_secs = 120              # how long the hook waits for the phone
on_no_answer = "deny"           # or "ask": leave it to the harness's own prompt
```

**Command patterns** are shell words. The first word names the program (`git`, `/usr/bin/git` and `sudo git` all
count). The other words must appear among the arguments in that order, not necessarily next to each other: `git push
--force` matches `git -C app push origin main --force`. `*` matches any run of characters within a word. `-x` also
matches a group of short options containing it (`rm -r` matches `rm -rf`). A pattern starting with `text:` matches
those words anywhere in the command, ignoring case and punctuation (`text:drop table`).

Windows commands are matched too. Program names match ignoring case and an `.exe`, `.cmd` or `.bat` ending
(`C:\Program Files\Git\cmd\git.exe push --force` is `git push --force`). A cmd switch in a pattern (`/s`) matches
ignoring case, also among switches written together (`rd /S/Q`). A PowerShell parameter in a pattern, written with a
capital (`-Recurse`), matches ignoring case and any abbreviation PowerShell accepts (`-r`, `-rec`, `-Recurse:$true`).
Backslashes in paths are kept (`type C:\Users\me\app\.env` matches `.env`), and the commands inside `cmd /c`,
`powershell -Command`, `pwsh -c`, `powershell -EncodedCommand` and `Invoke-Expression` are checked as well.

**File patterns** without a `/` match the file name (`*.pem`, `.env`). With a `/` they match the end of the path
(`.ssh/*`, `.aws/credentials`). A command's arguments are checked against the file patterns too, so `cat .env`
matches.

**Built-in command rules:** force pushes and ref deletions (`git push -f`, `--force*`, `+refspec`, `-d`, `--delete`,
`--mirror`, `--prune`, `:ref`), `git reset --hard`, `git clean -f`, `git branch -D`, `git checkout -f`,
`git filter-branch`, `git filter-repo`, `rm -r`, `terraform`/`tofu` `apply` and `destroy`, `kubectl apply`/`delete`,
`helm uninstall`/`delete`, `DROP TABLE`/`DATABASE`/`SCHEMA`, `TRUNCATE TABLE`, package publishing (`npm`, `pnpm`,
`yarn`, `cargo`, `poetry`, `uv`, `twine`, `gem`, `docker push`), `gh repo delete`, `gh release delete`, `mkfs*`,
`dd of=*`. On Windows: `Remove-Item -Recurse` and its aliases (`ri`, `del`, `erase`, `rd`, `rmdir`; `rm -Recurse` is
`rm -r`), cmd's `rd /s`, `rmdir /s`, `del /s`, `erase /s`, `format <drive>:`, `Format-Volume`, `Clear-Disk`.

**Built-in file rules:** `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.jks`, `*.keystore`, `*.tfstate`,
SSH private keys (`id_rsa`, `id_ed25519`, ...), `.ssh/*`, cloud and registry credentials (`.aws/credentials`,
`.aws/config`, `.config/gcloud/*` (on Windows `AppData/Roaming/gcloud/*`), `.azure/*`, `.kube/config`,
`.docker/config.json`, `.netrc` (on Windows also `_netrc`), `.git-credentials`, `.npmrc`, `.pypirc`, `.vault-token`,
`credentials.json`). Exceptions: `*.pub`, `known_hosts`, `.env.example`,
`.env.sample`, `.env.template`, `.env.dist`.

The phone shows the harness that asked, the command (or files), the working directory and the rule that matched. A standing answer can
cover a topic such as `command:make deploy`.

## What hooks do not do

Hooks match text patterns. A determined agent can get around them, for example by writing a script and running it.
They are guard rails against mistakes. The protection against a hostile agent is that it holds no credentials: git
tokens, API keys and passwords stay on the phone and are released per action. See
[security-model.md](security-model.md).
