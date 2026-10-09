# Security model

Reins assumes the AI agent may be wrong, confused, or steered by text it read (prompt injection). It does not
assume the agent is honest. The design rests on one rule: **the agent never holds credentials.** It can ask for
actions. The phone decides, and the phone (or, for git, a short-lived credential the phone seals to your computer)
carries them out.

## Parties

| Party | Trusted with | Not trusted with |
|---|---|---|
| **You, through your phone** | every decision; every credential | |
| **AI agent** (Claude Code, Codex, Cursor, Gemini CLI, Claude.ai, ChatGPT) | the results of actions you allowed | credentials; deciding anything |
| **Reins server** | relaying requests and answers; your account | your service credentials; deciding anything |
| **Desktop app** (`reins`, on your computer) | short-lived, single-purpose credentials sealed to its key | long-lived tokens; deciding anything in phone mode |
| **Google Firebase** | waking your phone (a request id only) | request contents |

## What the agent can and cannot do

The agent **can**:

- call the tools of the services you connected. Each call waits for your phone unless a standing permission you gave
  covers it;
- ask for a standing permission (`reins_request_access`). The phone shows it highlighted, and you can grant less
  than was asked for, or nothing;
- run git through the desktop app. Reads and pushes need your approval or a standing permission for that repository;
- run commands on your computer as far as its harness allows. Hooks send the risky ones to your phone.

The agent **cannot**:

- read a Gmail, GitHub, Telegram, vault or MCP server token. They are on the phone;
- read the git token. The desktop app holds it in memory, adds it to the upstream request itself, and never writes it
  to disk or logs;
- push something other than what you approved. A push approval is bound to a digest of the exact bytes git sent;
- reuse an approval. Sealed answers carry a fresh random nonce from the desktop app and an expiry;
- see which accounts you connected, unless you allow it. `reins_list_accounts` lists integrations only. Addresses
  need your approval, and errors never reveal them;
- talk Autopilot into approving. AI-written text can only lower its approval score ([below](#autopilot)).

## Where secrets live

| Secret | Where | Protection |
|---|---|---|
| Gmail, Google Calendar and Contacts access | phone, Google Play services | per-app tokens bound to the app's package and signing key. No refresh token exists in Reins. |
| GitHub, GitLab, Codeberg, Bitbucket tokens; Telegram session; MCP server tokens; vault key | phone, encrypted store | AES-256-GCM, with a data key wrapped by the Android Keystore. |
| Vault items | server (encrypted), phone (decrypts on request) | Bitwarden's end-to-end encryption. The phone unlocks the vault key once with your master password and keeps it sealed. It does not keep the password. |
| Grants, activity log, Autopilot memory | phone, encrypted store | as above. Never sent to the server. |
| Desktop app key (X25519) | `~/.local/state/reins/identity.key`, 0600 | readable by your OS user. |
| Desktop app session (OAuth tokens for the server) | `~/.local/state/reins/session.json`, 0600 | readable by your OS user. |
| Released git credentials, API keys, `reins run` secrets | desktop app memory | until the lease ends (git fetch 1 h, push 10 min, API proxy as configured). `reins run` wipes the values once the command starts. |
| SSH private keys | phone (vault) | signatures are made on the phone. The key never leaves it. |

Vault secrets an AI asks to see (passwords, one-time codes, notes, card numbers, SSH private keys) are asked for
every time. They can never be covered by a standing permission, and their values are not written to the activity log.
Secrets released to the desktop app (`reins run`, the API proxy) and SSH signatures can be covered by a standing
permission for one item, or for one key on one server, if you choose to give one. Autopilot never releases them on its
own.

## Accounts without a master password

On the hosted server, people sign in through WorkOS AuthKit (Google, Apple, GitHub, Microsoft or an email code) and
never set a password ([self-hosting](self-hosting.md#sign-in-without-passwords-workos-authkit)). The vault still
has Bitwarden's end-to-end encryption; only what protects its key changes:

- **The account secret.** After the first sign-in the phone makes the account's keys itself: a random user key and an
  RSA key pair, wrapped exactly as a Bitwarden client wraps them for a master password, with a random 256-bit secret
  in the password's place (PBKDF2-SHA256 over a fixed salt, so that an email change at WorkOS does not lock the
  vault). The server stores the wrapped keys and a hash of that "password", as for any account. The secret stays in
  the phone's encrypted store (its data key wrapped by the Android Keystore or the iOS keychain); it is never shown
  unless you ask for the recovery code, and never typed.
- **The recovery code** is the secret in base32, in thirteen groups of four. Settings > Account shows it after
  biometrics. Whoever has it and can sign in to the account can open the vault; without it and without a phone that
  keeps the secret, the vault cannot be opened by anyone, including the server.
- **Resetting the vault** is the way out when both are lost (the Unlock screen's "Lost both? Reset the vault"). The
  phone signs in again (the server makes WorkOS ask for the sign-in method again: `prompt=login`, `max_age=0`), checks
  it is the same account, and calls `POST /reins/api/account/reset`. The server allows that only within 10 minutes of
  a phone app's WorkOS sign-in by the same device, with an access token from that sign-in (not one the device held
  before), once per sign-in. It deletes the vault (items, folders, Sends), the keys, the
  encrypted account state, emergency access and organization memberships, the approval device, every other device's
  sign-in, and the AI and desktop connections; the phone then makes new keys and a new recovery code as for a new
  account. Nothing of the old vault is readable afterwards, by anyone.
- **Another phone** gets the secret from the approval device ("Add another phone"): the new phone makes an X25519 key
  and asks through the server; both phones show a six-digit code derived from that key; you compare them and approve
  with biometrics; the approval device seals the secret to the key (a sealed box naming the request), and the server
  relays it without being able to open it. A server that swapped the key would make the codes differ.

**What changes in trust.** WorkOS (and whoever controls the Google, Apple, GitHub or email account you sign in with)
can now sign in to your Reins account. That gets them the server-side account, not the vault: opening it takes the
account secret. Nor does it get them the approval role: once the account has an approval device, another device takes
it only with the recovery code or your phone's yes ([which device approves](#which-device-approves)). It does let
them reset the vault: that destroys its contents and signs your phones and AIs out (you would notice), but reveals
nothing of it, and the AIs you connected do not send their requests to the phone that reset. The server
refuses WorkOS impersonation sessions, follows WorkOS when it revokes a session or deletes a user, and takes an email
change only once WorkOS has verified the new address.

**Deleting the account.** Settings → Delete account in either app deletes the account in-app
(`POST /reins/api/account/delete`), so that someone who controls only your identity cannot delete it either. The
request carries the account's email, typed by you; an access token issued in the last five minutes (the phone
refreshes its token first, so a copied bearer token is not enough for long); and the approval device's device key, or
from any other device the same proof as for taking the approval role. An account with no approval device yet needs
only the email. Wrong emails and proofs count against the account like wrong proofs. The last owner of an
organization is refused before anything is deleted. On a WorkOS server the WorkOS user is deleted first (which ends its
sessions, so the identity cannot sign in to a new, empty account by itself); if WorkOS cannot delete it, nothing is
deleted. Then the server deletes the vault, devices and their push tokens, the approval device, AI and desktop
connections with their tokens, SSO sessions and identity, the encrypted account state, waiting requests and files, and
logs the deletion; the phone signs out and deletes its encrypted copy of the account and its cached keys.

## Which device approves

One device per account approves: it gets the AIs' requests and new connections. The server decides which
(`PUT /reins/api/device`):

- **The account's first approval device** needs nothing.
- **The approval device registering again** (a new push token, a restart, a new sign-in on the same phone) needs
  nothing. The same device means the same Vaultwarden device *and* the same device key: 32 random bytes the phone
  makes once, keeps in its encrypted store and sends with every phone-API call (`Reins-Device-Key`); the server keeps
  their SHA-256. A Vaultwarden device id is no secret (the account's device list shows it, and a sign-in may name any
  id), so a sign-in that claims the approval device's id without its key is another device, for registering and for
  every call only the approval device may make.
- **Any other device** takes the role only with a proof:
  - the master password hash of the account secret (the phone has the secret after the recovery code or another
    phone's approval), or, for accounts made with one, of the master password; the server checks it like a password
    sign-in. Wrong hashes count against the account: after 5 within 15 minutes (`REINS_DEVICE_PROOF_*`) every
    attempt waits;
  - or the approval device's yes to that device's "add another phone" request: the server keeps it for 5 minutes,
    for one takeover, and only for the device key that asked.

Without a proof the server answers `403 proof_required`, and the apps say "This account already has a phone for
approvals. Approve this phone from it, or enter your recovery code.", with exactly those two ways on (a reset of the
vault also frees the role, by deleting everything the role protected). The phone that
loses the role is told by push. The phone's core attaches the proof itself when it has one, so a phone that just got
the secret, or signed in with the master password, moves the role without asking again.

- Approving needs the phone's screen lock or biometrics. Denying is one tap. A routine request can also be approved
  from its notification, which works only once the phone is unlocked; that approves exactly what its screen would approve untouched (never an item that looks like a
  code or a password) and creates no permission. Requests that are asked every time, and everything on Autopilot's
  [hard floor](#autopilot), have no such button, and the phone's core refuses to approve them that way.
- "Approve and allow for 1 hour" (8 hours after repeated identical approvals) creates an ordinary standing permission
  for the same connection, kind of request and target, shown under Grants.
- The starting rule (chosen during setup, "ask every time" until then): "reads for a day" gives a connection you just
  approved standing read permissions for 24 hours, one per connected integration (never the vault or the desktop app)
  and per MCP server's read-only tools. They are ordinary grants: listed, logged, revocable. A read permission
  includes listing the same things, never writing.
- No standing permission releases an email or message that looks like a login code or a password; it always waits
  for the user's tick.
- One-time approvals execute exactly what was shown and create no permission. Standing permissions are limited to one
  connection and can be narrowed by target (sender, recipient, repository, branch, kind of change), time and number of
  uses.
- Some requests are **asked every time** and never covered by a standing permission: vault secrets an AI asks to
  see, force pushes and other history rewrites, deleting branches or repositories, transfers, visibility, collaborators, invitations, teams,
  webhooks, deploy keys, repository secrets, branch protection and rulesets, organization-level deletions, and MCP
  tools their server marks destructive.

## Pairing

**AI connections.** Connecting an AI opens a browser page on the server. You type your account email, and the page
shows a two-digit code. Your phone shows three codes, and you tap the matching one, name the connection and confirm
with biometrics. The page looks the same for unknown emails, so it does not reveal which accounts exist. The AI gets
an OAuth access token valid for 1 hour, and a refresh token valid for 30 days that rotates and is stored hashed.
Removing the connection on the phone revokes both.

**The desktop app** pairs the same way, and adds its X25519 public key to the request. The terminal, the browser page
and the phone all show the key's fingerprint (eight digits, such as `4821 9930`). You approve only if they match. The
phone then **pins** the key to that connection. Requests for desktop-only tools (git, `ask`, secret release, SSH) are
answered only for a connection whose pinned key matches the key in the request. Other AI clients never see those
tools.

## Sealed answers

Whatever the phone sends to the desktop app (a git credential, a yes to `reins ask`, released secrets, an SSH
signature) is a sealed box: X25519 with XSalsa20-Poly1305, encrypted to the pinned key. The server relays it but
cannot open it. The desktop app accepts an answer only if:

- it opens with the app's private key;
- it echoes the nonce of this very request;
- it names the same repository and access (read or write), and for a push, the same push digest;
- it has not expired.

So a server that is compromised cannot read the credential, cannot substitute its own key (the key was pinned when
you compared fingerprints), and cannot replay an older answer.

**The other way: `reins vault add`.** A secret typed on the computer goes to the phone in a box from the app's pinned
key to the phone's own key (X25519, XSalsa20-Poly1305), with the request's nonce, the item's name and the field. The
phone opens it only if the app's pinned key made it, and only for the item and field the request names, so the server
can neither read the value nor put another one in its place. The phone's key (one per installation, never copied to
another phone) reaches the computer once, sealed to the app, after you approve it on the phone; you then type the
eight digits the phone showed, and the computer keeps the key. Because the key never travels in the clear, a server
that answered with its own key could not know which digits to aim for.

**Git pushes.** The desktop app reads the pack git is about to send and works out what it does: which branches or
tags it touches, whether each update is a fast-forward (asking the host's API when needed), the commits, the files,
and the line counts. That summary is what the phone shows. The digest is computed over the repository, the ref
updates, the SHA-256 of the pack and the push options. The phone's credential is valid only for that digest. When
part of the analysis cannot be completed, the screen shows a note instead of a guess. A push whose history could not
be checked is treated as a force push.

## What the server sees

The server is a relay, and it is in a position to read what it relays.

**Stored in its database:** your Vaultwarden account (email, master password hash, encrypted vault, devices), the
phone's push token, the AI clients that registered, your AI connections (name, label, host, creation and last-use
times), and hashed refresh tokens.

**In memory only:** waiting requests and their results (at most 10 minutes), pairings and sign-in sessions (minutes),
which integrations your phone has (ids only, no account names), and the names and tool lists of MCP servers you added
on the phone.

**On disk for one operation:** large files (at most an hour, deleted when the operation is done).

**Sees in transit:** tool arguments (the email the AI wants to send) and results (the emails it was allowed to
read). This is unavoidable: the AI receives the results over the same connection. They are not logged. The server also
sees sealed desktop answers, which it cannot open.

**Passes credentials through, in two cases.** When a file is too large for a tool call (a release asset upload, a big
download), and when an MCP tool you added returns very large results, the phone asks the server to make that one HTTPS
request, with the authorization header it needs. The server uses the header for that request only, does not store or
log it, and refuses private, loopback and cloud metadata addresses. Every other credential stays on the phone.

**A compromised server could:** read arguments and results in transit; forge requests that look like they come from
one of your connections, which the phone still has to approve unless a standing permission covers them; show you a
misleading pairing page; withhold or delay requests; capture a header passed through for a large-file operation.

**It could not:** reach Gmail or any connected service on its own; open sealed desktop answers; get around a pinned
desktop key; approve anything; read your vault; read your grants, activity log or Autopilot memory.

Push notifications through Firebase carry only a request id. The phone then fetches the request from the server.

## The desktop app

- It listens on loopback only (`127.0.0.1:7457` by default). Requests must carry a loopback `Host` header, which
  blocks DNS rebinding. Requests with an `Origin` header and `OPTIONS` requests are refused, so web pages cannot reach
  it.
- Its control API (`reins pending`, `approve`, `deny`, `status`) needs a random token stored in a 0600 file.
- On Linux the daemon marks itself non-dumpable, so other processes of the same user cannot attach to it or read its
  memory through `/proc`. macOS and Windows have no such protection here.
- On Windows the state files (`%LOCALAPPDATA%\reins\`) get an access list for your user alone instead of mode 0600,
  the SSH agent's named pipe can be written only by your user (and the administrators), and the background service is
  a copy of the program in that same folder, so another user cannot replace what runs at your logon.
- Release updates (`reins update`) install only builds signed with the release key built into the binary, and
  never an older build. The desktop app's own updates come from a second list signed with the same key
  (`app.json`); it keeps a downloaded installer only when its size and SHA-256 match that list. The install script
  checks the published SHA-256.

**Same-user limits.** An agent running as your OS user can read the files in `~/.local/state/reins/`, including
the app's key and session. With them it could act as the desktop app: call the server, receive sealed answers, and
open them. It still cannot get anything you do not approve on the phone. It does benefit from standing permissions
you gave the desktop connection, and it could approve local-mode prompts through the control API. For full
separation, run the daemon as its own OS user and keep the agent from reading that user's files.

**Local mode** (not logged in): the GitHub token comes from `gh auth token`, an environment variable or a file the
agent's user can usually read. Local mode protects against an agent's mistakes. Phone mode protects against a hostile
agent.

## Hooks

Harness hooks compare commands and file paths with patterns ([harnesses.md](harnesses.md#the-guard-rules-guard)).
They catch the obvious: force pushes, `rm -r`, `terraform apply`, reading `.env` or private keys. They cannot catch
a command hidden in a script, an alias, or a tool the hook does not see. Treat them as guard rails against mistakes,
not as a sandbox. When the phone does not answer, the hook denies by default (`on_no_answer = "deny"`).

## Autopilot

Autopilot runs only on the phone. Nothing it uses or learns leaves the phone. It is limited three ways.

- **Hard floor.** Some requests always wait for you, in every mode: new connections, standing permission requests,
  account listings, secret releases, SSH signatures, everything that is asked every time (above), destructive MCP
  tools, pushes that rewrite or delete history (or whose history could not be checked), hook questions about a
  destructive command or a secret file, uploaded files that may hold something to run, and anything from a connection
  paired less than 10 minutes ago. Lockdown denies, and never
  approves.
- **Prompt-injection rule.** The model scores each request twice: once on the facts the phone verified (service,
  action, target, history), and once with the AI-written text added (reason, email body, commit messages, arguments).
  The approve score it uses is the lower of the two, and the deny score the higher. AI-written text can therefore make
  a request look riskier, never safer.
- **Approve once.** Autopilot never creates standing permissions. Its automatic decisions never train it, so it
  cannot reinforce its own mistakes.

Details: [autopilot.md](autopilot.md).

## Known gaps

- The server sees tool arguments and results in transit (above). End-to-end encryption between the AI and the phone
  is not possible while the AI needs the plaintext.
- One approval device per account, and the first one needs no proof ([which device approves](#which-device-approves)):
  an account that never had one (signed up, no phone set up yet) goes to whichever of its signed-in devices registers
  first. A phone that moves the role with the recovery code or the master password has none of the first phone's
  service credentials or grants.
- An approval device registered before device keys (servers from before October 2026) is known by its device id
  until it registers again; its first registration with a device key keeps that key.
- On an account made with a master password, the approval device has no account secret to send, so "add another
  phone" cannot approve there; the new phone proves itself with the master password.
- Hooks are pattern-based (above).
- Autopilot's model was trained and evaluated on synthetic data ([model card](../tools/laya/MODEL_CARD.md)).

To report a vulnerability, see [SECURITY.md](../SECURITY.md).
