# Reins Secrets: design

Reins Secrets turns the vault on your phone into the place a developer's secrets live: `.env` values, API keys, SSH keys, git
and registry tokens, cloud credentials. Programs and AI agents use them on any of your computers, and every use is approved
the way everything else in Reins is (your phone, your grants, Autopilot). The goal is the reach of 1Password Developer and
Infisical with none of their setup: one scan moves what is lying around a computer into the vault, and from then on
`reins run -- npm run dev` just works, on this computer and on the next one.

This document is the plan the implementation follows. It builds on what exists (see `security-model.md`), names the
gaps it closes, and fixes the formats and interfaces the workstreams share.

## 1. Principles

1. **The phone is the root of trust.** Values are decrypted only on the phone, and leave it only sealed to one computer's
   key, for one approved use, for a bounded lease. The server relays sealed boxes and never sees a value (tool calls that
   carry plaintext today are replaced, section 7).
2. **Agents work with names, never values.** An agent can see which secrets exist and ask to run a command with them; it
   never reads a value, in a tool result, in a file, or in the command's output (section 6).
3. **One step to adopt, zero steps to use.** Import is a scan and one approval. Using a project's secrets is `reins run`
   in its folder; a new computer is "pair, then tick what this computer uses".
4. **Approval is bound to what runs.** A grant for secrets names the project, the environment, the computer, who runs it
   (a person or which agent) and the command pattern, not just the item (closes gap: grants not bound to the command).
5. **Fail closed, revocable, visible.** Leases are short, held only in the daemon's memory, listed and revocable from the
   phone and the desktop app; a lost computer is cut off by unpairing it.
6. **Plain files stay plain.** What a repo commits (`reins.toml`, `.env.reins`) holds references only, so it is safe to
   commit and readable by any tool.

## 2. Model

### 2.1 Projects and environments

A **project** is a named set of **environments** (`development`, `staging`, `production`, or any name), each a set of
**variables** (`OPENAI_API_KEY = sk-…`). It maps onto the existing Bitwarden vault without schema changes:

| Concept | Vault representation |
|---|---|
| All Reins Secrets | folder `Reins` |
| Project `acme-web` | folder `Reins/acme-web` |
| Environment `development` | one **Secure Note item** named `acme-web/development` in that folder, type marker in a hidden custom field `reins.kind = env/1` |
| Variable | one **hidden custom field** on that item: name = variable name, value = value |
| Non-secret variable (optional) | a **text** custom field (kept so an env can be complete; never needs approval to read its name) |
| Machine credential (gh token, npm token, AWS profile) | an env item in project `Reins/machine/<computer>` or `Reins/shared` (section 5) |
| SSH key | existing **SshKey item** in folder `Reins/ssh` |

Why one item per environment: one approval releases a whole environment (a dev server needs all of them), the item's
revision gives the environment one version, and vault history keeps earlier values (the phone already moves a replaced
value into history). The vault stays fully Bitwarden-compatible: a Bitwarden client sees a note with hidden fields.

### 2.2 References

The reference syntax stays `vault:<item>/<field>` (`crates/reins-desktop/src/secrets.rs`, split at the last `/`), so
`vault:acme-web/development/OPENAI_API_KEY` names item `acme-web/development`, field `OPENAI_API_KEY`. New shorthands:

- `vault:acme-web/development` (no field, item is an env) = every variable of the environment.
- Inside a project (a `reins.toml` above the working directory), `env:development/OPENAI_API_KEY` and `env:development`.

Items are resolved on the phone by id, then exact name (existing resolver). Renaming an environment updates nothing in
repos because repos reference the project and env names, which the phone keeps unique per folder.

### 2.3 Files in a repo

`reins.toml` at the project root, committed:

```toml
[project]
name = "acme-web"                 # the vault folder Reins/acme-web
default_env = "development"

[env.development]                 # optional per-env settings
lease = "8h"                      # how long a release may be held by the daemon (cap: the phone's policy)

[env.production]
lease = "15m"
confirm = "always"                # never covered by a grant: always asks the phone
```

`.env.reins`, also committed, optional, for tools that read dotenv files: the variable names, each pointing at its
reference (written by import, kept in sync by `reins secrets pull`):

```dotenv
OPENAI_API_KEY=vault:acme-web/development/OPENAI_API_KEY
STRIPE_SECRET_KEY=vault:acme-web/development/STRIPE_SECRET_KEY
LOG_LEVEL=debug                   # non-secret values may stay literal
```

`reins run` reads `.env.reins` if present (literal values pass through, references are resolved), else `reins.toml`
(the whole environment). Neither file ever holds a secret value; the guard keeps treating `.env*` as protected except
`.env.reins` and `.env.example`.

## 3. Using secrets

### 3.1 `reins run`

```
reins run [-e ENV] [--project NAME] [--env NAME=REF ...] [--purpose TEXT] [--no-redact] -- COMMAND...
```

1. Finds the project (`reins.toml` upwards from the cwd, or `--project`) and the environment (`-e`, else
   `default_env`), plus explicit `--env` pairs and existing `[run.profiles]` (kept).
2. Asks the **daemon** (control API, new route `POST /secrets/lease`) for a lease covering exactly those references for
   this command. The daemon returns a cached lease if one matches (same computer principal, project, env, command
   pattern, still valid), else asks the phone with one `vault_env_release` (3.2) and caches the answer.
3. Spawns the command with the variables in its environment. By default it **redacts**: the child's stdout/stderr are
   piped through a filter that replaces any released value (and its base64/URL-encoded forms) with
   `[vault:acme-web/development/OPENAI_API_KEY]`. Redaction is forced on when the caller is an agent (6.2); a person can
   pass `--no-redact` for interactive tools that need a raw TTY.
4. Journals the run (references, command, who), never values.

### 3.2 Release protocol

New desktop-only tool **`vault_env_release`** (replaces repeated `vault_secret_release` calls; the old tool stays for
single references):

```json
{ "project": "acme-web", "env": "development", "fields": ["OPENAI_API_KEY", "..."],
  "program": "/usr/bin/npm", "args": ["run", "dev"], "root": "/home/dana/dev/acme-web",
  "runner": "person" | "agent:claude-code", "lease_secs": 28800, "purpose": "…", "nonce": "…", "client_key": "…" }
```

`fields` left out means every variable of the environment. The command is the argument list `reins run` spawns (it
never uses a shell), with the program resolved to its absolute path; `root` is the project's folder (canonical path).

The phone answers with the existing `SecretGrant` (sealed to the pinned computer key, bound to the nonce), extended with
`project`, `env`, the item revision, and the binding the phone decided: the `runner` and `root` it approved and the
`command` (a `CommandScope`: exactly the approved argument list, or the prefix the user chose on the phone; a pattern
the computer merely proposed is never used). The variables granted are the names in `secrets`. Limits: up to 200 fields
per release, 64 KB of values. The approval view (`SecretReleaseView`) shows: who (person or agent and which), which
computer, the command, the project and env, the variable names (never values), the lease, and the Remember choices of
the approval dock.

The daemon reuses a cached lease only when all of these match: runner, computer (its own connection), project root,
environment and its revision, the variables asked for are among those granted, and the program with its arguments is
covered by the granted command. Anything else asks again.

### 3.3 Grants bound to what runs

A standing grant for secrets gets a scope with these dimensions (stored in the existing grant scope, `GrantScopeChoice`
gains a `secrets` part):

| Dimension | Values |
|---|---|
| project, env | exact |
| computer | the connection (as today) |
| runner | `person`, or `agent:<harness>` (6.1) |
| project root | the folder on the computer (canonical path) |
| command | the program (absolute path) and its arguments, compared as whole arguments: exactly, or a prefix followed by any number of whole arguments (`npm run *` covers `npm run test --watch`, never `npm runx`), chosen in the dock's Custom sheet; default = the exact command |
| fields | all of the env, or the listed ones |

A prefix never ends inside an argument and never has a wildcard in the middle. It must name at least one subcommand
after the program (`npm run *`, not `npm *`), and is refused (only the exact command can be remembered) for programs
that run whatever they are given: shells and interpreters (`sh`, `bash`, `zsh`, `fish`, `cmd`, `pwsh`, `powershell`,
`python`, `node`, `deno`, `bun`, `ruby`, `perl`, `php`, `osascript`, …), launchers and wrappers (`npx`, `bunx`, `uvx`,
`env`, `xargs`, `sudo`, `nohup`, `time`, `timeout`, `exec`, `eval`, `watch`, `nix-shell`, `make`, `ssh`, …), and
subcommands that do the same (`npm exec`, `pnpm dlx`, `yarn dlx`, `cargo run`, `uv run`, `docker run`, `docker exec`,
`go run`, `git -c`, …), and for a prefix that includes `--`. The list is in `reins_proto::desktop` (`prefix_refusal`).
A program is compared by both its names, as typed (`npm`, often a link) and as resolved (`…/npm/bin/npm-cli.js`), and
refused when either is listed; a resolved name loses a script's extension and a `-cli` ending (`npm-cli.js`,
`npx-cli.js`, `yarn.js`, `pnpm.cjs` and `nodemon.js` are `npm`, `npx`, `yarn`, `pnpm` and `nodemon`). Another program of
the same name elsewhere on PATH is another program.

`confirm = "always"` in `reins.toml` and any env named `production`/`prod` make the env `no_standing` by default (the
user can change it on the phone, never from the repo alone). Bypass still approves everything (by the user's decision),
and its grants end with it (existing).

### 3.4 Leases and revocation

- The daemon holds released values in memory only (`Zeroizing`), per lease key `(principal, project, env, fields,
  command pattern)`, until `min(phone expiry, reins.toml lease, phone policy)`.
- `reins secrets leases` lists them; `reins secrets revoke [--all]` drops them; the desktop app's Keys & secrets page shows
  and drops them; the phone's grant/activity pages get **Revoke on all computers**, which the daemon picks up through
  the existing poll (a new `lease_revocations` message, sealed to the computer).
- A child process that already received values keeps them (the OS gives no way back); leases stop new releases.

### 3.5 Other ways in

- **API proxy** (`[[api]]`, exists): unchanged, but may reference env variables (`secret = "env:production/OPENAI_API_KEY"`).
- **SSH agent** (exists): keys imported to `Reins/ssh`; nothing else changes.
- **git** (exists): host tokens stay in the phone's integration store (they are integrations, not vault items); import
  moves a found `gh`/`.git-credentials` token there (5.3).
- **`reins secrets get REF`** for scripts that need one value on stdout: person-only (refused when the caller is an agent),
  always asked, never granted.

## 4. Managing secrets

CLI (`crates/reins-desktop`, `reins secrets …`), mirrored in the desktop app's Keys & secrets page and the phone's vault
screens:

| Command | Does |
|---|---|
| `scan [PATH...] [--json]` | finds secrets on this computer (section 5), prints names, places, risks; no values |
| `import [--from-scan FILE] [--select ...]` | moves the selection into the vault (one batch approval), rewrites sources |
| `init [NAME]` | creates `reins.toml` (and the vault folder on first push) |
| `ls [PROJECT[/ENV]]` | projects, envs, variable names, versions; values never |
| `set ENV/NAME` | sets a value, read without echo or from stdin, via the batch store (one approval, check digits) |
| `unset ENV/NAME`, `rename`, `cp ENV ENV2` | edits through the same store path |
| `push [-e ENV] [FILE]` | uploads a dotenv file's values to an env (then offers to delete the plaintext file) |
| `pull [-e ENV]` | writes `.env.reins` (references only) for tools that need a file |
| `diff ENV ENV2` | names present in one and not the other |
| `leases`, `revoke` | 3.4 |
| `doctor` | checks the project: references resolve, no plaintext `.env` left, guard rules on |

Writes use a new **batch** store tool `vault_secrets_store` (extends `vault_secret_store`: many variables of one env in
one sealed box, one approval, one set of check digits over the batch, history kept for replaced values).

## 5. Scan and import (workstream A)

### 5.1 Scanner

A pure library module in `reins-desktop` (`secrets_scan`), used by the CLI and the desktop app. Local only, started only
by a person (the desktop window or a terminal command; refused when the hook context says an agent is calling).

Sources (each a parser that yields findings):

| Source | Where | Finding |
|---|---|---|
| dotenv | `.env`, `.env.*` (not `.env.example`, `.env.reins`) under project roots (`~/dev`, `~/code`, `~/src`, `~/projects`, `~/work`, git repos found under `~`, depth-limited) | one finding per file, with its variables |
| SSH | `~/.ssh/*` private keys (OpenSSH/PEM headers) | one per key; passphrase or not |
| GitHub CLI | `~/.config/gh/hosts.yml` (and `gh auth token` when the token is in the keychain) | token + scopes (via the API only if the user asks) |
| git credentials | `~/.git-credentials`, `credential.helper store` files | per host |
| npm / yarn / pnpm | `~/.npmrc`, project `.npmrc` (`_authToken`) | per registry |
| PyPI | `~/.pypirc` | per repository |
| crates.io | `~/.cargo/credentials.toml` | token |
| Docker | `~/.docker/config.json` `auths` (not credential helpers) | per registry |
| AWS | `~/.aws/credentials` | per profile |
| GCP | application default credentials JSON | one |
| Kubernetes | `~/.kube/config` tokens / client keys (not exec plugins) | per user entry |
| shell | `~/.zshrc`, `~/.bashrc`, `~/.profile`, `~/.config/fish/config.fish`: `export NAME=value` where NAME looks like a secret or the value matches a detector | per variable |
| netrc | `~/.netrc` | per machine |

Detectors classify values (OpenAI, Anthropic, Stripe live/test, GitHub PAT/fine-grained/app, Slack, AWS access key,
Google API key, JWT, private key PEM, Postgres/MySQL/Mongo/Redis URLs with passwords, generic high-entropy) with a
confidence, so the review can pre-tick real secrets and leave `LOG_LEVEL=debug` alone. Gitleaks-style rules, compiled in.

Each finding carries: source kind, path, line(s), variable names, detector kind, **masked preview** (first 4 + last 4
characters at most, never more than a quarter of the value), risk flags (`readable_by_agents` when the guard does not
already protect the path or an agent harness is installed, `plain_text`, `in_git_history` from `git log --all -- PATH`
and `git ls-files`, `no_passphrase`, `world_readable`), and a stable id. Values are read only to classify and mask, kept
in `Zeroizing` buffers, and dropped after the scan; import re-reads the file.

Walk rules: never follow symlinks out of the home folder, skip other users' files, skip `node_modules`, `.git/objects`,
`target`, `vendor`, `.venv`, caches, files over 1 MB, binary files; stop at a time and count budget and say so.

Output: `ScanReport { findings, skipped, took }`, JSON-serializable (`reins secrets scan --json`), stable for the UI.

### 5.2 Review (desktop app and CLI)

As designed in `reins-designs/desktop-secrets.html`: grouped findings, risk chips, pre-ticked by confidence, per-variable
ticks for dotenv files, a right panel with the count and **Then: Use them through Reins** (default) or **Keep local
copies too**. CLI: `reins secrets import --select` opens an interactive picker, or takes `--all-suggested`.

### 5.3 Import

1. Group the selection: dotenv files by project (project name = the repo folder name, env = from the file name:
   `.env` → `development`, `.env.production` → `production`, `.env.local` → `development`), machine credentials into
   `Reins/machine/<computer name>` envs (`aws-default`, `npm`, `pypi`, `docker`, …), SSH keys into `Reins/ssh`.
2. One `vault_secrets_store` per target env plus SSH key stores, sent as **one approval batch** on the phone (a new
   grouped view: "Move 20 secrets from Dana's MacBook into your vault", per-env counts, one set of check digits).
3. On success, per source, if **Use them through Reins**:
   - dotenv → write `reins.toml` (if missing) and `.env.reins`, move the original to the OS trash (`trash` crate);
     if the file is in git history, warn to rotate (the review already says so);
   - SSH key → after the Reins agent has signed once with the imported key (verified), move the private key file to the
     trash, keep `.pub`; run `reins ssh setup` if not already;
   - gh / `.git-credentials` → hand the token to the phone's GitHub integration (new sealed tool
     `integration_token_store`, approval shows the host and scopes), turn on git through Reins for that host, remove the
     token from the file / `gh auth logout` (keychain);
   - npm/pypi/cargo/docker/aws/kube/netrc → keep the variables in the machine env and write a run profile
     (`[run.profiles.aws-default]`, …) or, for tools that read a config file, a small wrapper script in
     `~/.local/share/reins/bin` added to PATH (`aws`, `npm publish`), then remove the plaintext token;
   - shell exports → comment the line out with a note pointing at `reins run`/profiles.
   With **Keep local copies too**, step 3 is skipped.
4. Everything step 3 changes is recorded in an **import receipt** (`state/imports/<time>.json`: what moved where, what
   was trashed) so `reins secrets import --undo` can restore files from the trash.

### 5.4 Another computer

After pairing, the desktop app (and `reins secrets setup`) lists what the vault offers: projects (written into repos as
they are cloned: `reins secrets pull` in a repo with `reins.toml`), SSH keys (agent on), git hosts (through Reins),
machine envs (profiles and wrappers). The user ticks what this computer uses; nothing is copied to disk.

## 6. Agents

### 6.1 Agents are their own principal

Today agents (`reins mcp --via <harness>`) share the computer's connection and grants with the person's CLI. Reins
Secrets separates them: every desktop call carries `runner` (`person` or `agent:<harness>`), set by the daemon from how it
was called (an MCP bridge or a hook process is an agent; an interactive terminal not under a harness is a person;
`reins run` started from an agent's shell inherits `REINS_RUNNER=agent:<harness>`, which the harness hooks export and the
guard forbids unsetting). Grants and leases are keyed by runner, so "allow npm run dev for me" does not cover Claude Code.

### 6.2 What an agent can do

| Need | How | Sees values? |
|---|---|---|
| know what exists | MCP `secrets_list` (projects, envs, variable names, which are set) | no |
| run with secrets | its shell runs `reins run -- …` (redaction forced on), or MCP `secrets_run` (runs in the project's cwd, returns redacted output, exit code) | no |
| call an API | the API proxy (`[[api]]`) | no |
| add or change a secret | MCP `secrets_request_set` → asks the person to type it on the computer or the phone (the value never passes through the agent) | no |
| read a value | not offered. `vault_get` stays for connections that are not agents and is refused for `agent:*` runners by default (a setting on the phone can re-allow it per agent) | — |

The guard adds `reins secrets get`, `reins run --no-redact`, `env`/`printenv`/`cat /proc/*/environ` of a reins child, and
the daemon's state files (`session.json`, `control.token`, `phone-key.json`) to what asks the phone when an agent runs them.

### 6.3 Making it easy for agents

- `reins harness add` writes a short instruction block for each harness (Claude Code `CLAUDE.md` / skill, Codex
  `AGENTS.md`, Gemini, Cursor rules): "Secrets for this project are in Reins. Use `secrets_list` to see names, run commands
  that need them with `reins run -- <cmd>`, never ask the user to paste a key."
- `reins.toml` makes a repo self-describing: an agent that clones it knows how to run it.

## 7. Removing plaintext from the server

`vault_get`, `vault_item_create` and `vault_item_update` carry values in tool arguments/results through the server. Phase 2
moves desktop and agent use to sealed tools only (`vault_env_release`, `vault_secrets_store`); `vault_get` for non-desktop
connections (claude.ai, ChatGPT) remains, clearly labelled on the phone as "the value goes to <AI> through the server".

## 8. CI and servers (phase 3, outline)

Machines that cannot hold a person's approval loop:

- **Machine identities**: a new connection kind `machine` (a CI provider or a server), paired from the phone with a
  scoped policy: which project, which env, which command pattern, max lease, allowed hours.
- **GitHub Actions**: a `reins/secrets-action` exchanges the job's OIDC token with the Reins server; the server checks the
  issuer, repo, ref and environment claims against the machine policy and relays a release request to the phone (or, if
  the phone pre-approved, uses a **pre-sealed bundle**: the phone seals the env's current version to the machine's key
  when the policy is created or the env changes; the server stores only that box). The runner gets the values for the job
  only. The phone shows every CI release in Activity.
- **Servers**: the same with a long-lived machine key and short leases, `reins run` on the server.
- Rotation hooks (later): `reins secrets rotate ENV/NAME` with provider plugins (Stripe, OpenAI, GitHub) that mint a new
  key, store it, and revoke the old one.

## 9. Security notes

- New guard rules and runner separation close "agent runs `reins run -e X=… -- echo $X`": redaction forced for agents, and
  the grant names the runner and command.
- The scanner is read-only, user-started, local; findings hold masked previews only.
- Import never deletes: files go to the trash and the receipt can undo.
- Batch store keeps the check-digit comparison (one set over the batch). In Bypass, stores still need the person to be
  present at the computer to type or import; Bypass approves the phone side as today.
- Machine identities never get `vault_get`, never get SSH signing unless their policy names a key and host.

## 10. Workstreams

| | Scope | Depends on |
|---|---|---|
| **A** Scan and import | `secrets_scan` library, `reins secrets scan/import`, desktop review screens (5.1–5.3 with the import calling B's tools), import receipt and undo | B's store tool for step 2 (A builds scan + review first, then wires import) |
| **B** Core and protocol | reins.toml/.env.reins, `reins run` changes and redaction, daemon lease cache + control routes, `vault_env_release`, `vault_secrets_store`, `integration_token_store`, grant scope for secrets, phone approval views (iOS + Android), `reins secrets` management commands, revocation | — |
| **C** Agents | runner principal end to end, MCP `secrets_list` / `secrets_run` / `secrets_request_set`, guard additions, harness instruction blocks, `vault_get` refusal for agents | B |
| **D** Another computer | `reins secrets setup` and the desktop screen (5.4) | A, B |
| **E** CI and servers | section 8 | B, C |
