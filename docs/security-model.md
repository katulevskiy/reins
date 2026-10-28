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
| Virtual card provider key (Privacy.com) | phone, encrypted store | on the phone that connected it only (not in the synced account state); sent to the provider only, to make, read and close cards. |
| Payment settings, purchase ledger, the mandate signing key | phone, encrypted store | travel with the account state like the grants (the server holds them encrypted with the account key). The ledger keeps a virtual card's id and last four digits, never a card number. |

Vault secrets an AI asks to see (passwords, one-time codes, notes, card numbers, SSH private keys) are asked for
every time. They can never be covered by a standing permission, and their values are not written to the activity log.
Secrets released to the desktop app (`reins run`, the API proxy) and SSH signatures can be covered by a standing
permission for one item, or for one key on one server, if you choose to give one. Autopilot never releases them on its
own in Manual or Auto; a Bypass you turned on does ([below](#bypass)).

Purchases ([Payments](payments.md)) are approved by you each time, on the purchase screen, or by a spend limit you
set for one AI, paying with a virtual card capped by its provider (a few an hour at most, each one notified); never by
a standing permission or Autopilot's own judgement. A Bypass you turned on approves them as their screen would
([below](#bypass)), within your budgets. What pays leaves the phone only with an approved purchase, for that cart: a
virtual card made for it, capped at its total plus a small tolerance (the safest), a card from the vault (always asked
for, never by a spend limit), or nothing at all (the store's saved payment method, or you pay on the phone). What a
purchase counts against budgets and limits never goes down because of what the agent reports. For an AI connected
directly, the server relays card details in memory like any answer and never writes them down; while a desktop app
is paired, a card from the vault goes to no other connection, since which connection asked is the server's to say.
For the desktop app's connection the phone hands card details over only sealed to the app's pinned key, and signs the
whole answer with the account's payment key and the request's nonce. The app's bridge passes on only approvals signed
that way by the key you typed in once on that computer from the phone's screen (`reins payments-trust`), so a server
that strips, changes or makes up an answer gets nothing to pay with and cannot fake an approval. Every
approval carries a mandate signed on the phone, so the cart you approved can be proved later. [What leaves the phone,
and when](payments.md#what-leaves-the-phone-and-when).

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

**Phone roles** (accounts whose primary created a roster; the
design: `reins-designs/phone-roles/phone-roles-and-live-sync.md`). The roster is a hash-chained list of entries
signed by the account's owner key; the server appends one only when the signature verifies against the current owner
key and its epoch is the next one (a compare-and-swap: a rollback, a replay or a fork is refused with
`epoch_conflict`). What each reason may change is fixed (`reins_proto::roster::check_successor`): only a recovery
replaces the owner key, and it removes the old primary. Signing and owner keys must be canonical Ed25519 points not of
small order. The approval role is the roster's primary: every approval-only call checks the caller against the member
row the latest entry made primary (with its enrolled device key), and the role moves in the same transaction as the
entry that moves it, a `handover` (appended by the primary) or a `recovery` (the master password hash, counted with
the takeover proofs); `PUT /device` no longer moves it by a proof. Every member is bound to the device key it enrolled
with, and a new viewer only through an enrollment join that went through the commit-then-reveal exchange
(`reins_proto::enroll`; one join lists the phone in one entry, the primary cannot approve it before the reveal, and the
challenge expires within 5 minutes): its six digits cover both phones' keys and two fresh nonces, each side fixed
before it sees the other's, so a server that swaps keys has one chance in a million per attempt instead of an offline
search. Each entry signs the hash of the recovery wrap in force; only a recovery or a `rewrap` (a changed master
password) brings a new one, and the server hands the wrap out only with the master password hash (counted). With a
roster:

- Answers to requests (A4), pairings (A6) and joins must carry the primary's Ed25519 signature over the account, the
  kind, the item's id, the epoch and the answer's hash (`SignedAnswer`); an unsigned or misdirected answer is refused.
- Requests, joins and handover requests are marked seen only when the approval device looks: another device of the
  account cannot take them off the primary's list.
- Ops are signed by their author; the server checks the author's role in the roster (never what the op claims), the
  class (`primary` only from the primary, `proposal` from any member, `restrictive` from any member and late from the
  previous primary), the signature, a strictly rising counter per author and the size limits. Phones check it again,
  open the body (sealed with the envelope as associated data) and apply a restrictive op only if it carries one of the
  typed restrictions (revoke a grant, Lockdown, stop Bypass), which can only tighten.
- Viewers read the waiting list, connections and the log; they approve, deny, pair and remove nothing. A stolen viewer
  can propose (inert until the primary keeps it) and pull the brake (Lockdown). It also holds the account secret (a
  viewer keeps it, as decided for the design), so whoever has it unlocked can enter the recovery code from it and
  take the primary role with a recovery: the old primary is told by push, and the remedy is the same as for a stolen
  primary (recover back from another phone, then reset the vault and change the sign-in methods). The primary removes
  a stolen viewer; a removed viewer's open sync poll ends at once.
- Op bodies are capped per class (64 KiB for the primary, 16 KiB for a proposal, 1 KiB for a restriction). A phone
  other than the primary (a viewer, or a replaced primary) may hold only its own share of the account's log
  (`REINS_MEMBER_LOG_*`) and never the tenth kept for the primary, so it cannot fill the log and stop the primary's
  ops; checkpoints are never counted. Alert pushes to the primary for viewers' ops come at most once per half
  minute.
- Limits are per phone (op uploads, proposals, handover requests, log share), per account (roster changes, ops and
  bytes the log holds, at most 3 open join requests) or per IP address (join requests, `REINS_JOIN_RATELIMIT_*`), never
  one for the whole server. Join requests expire after 10 minutes and are dropped on the next request and by the
  scheduled purge. A handover request that replaces a phone's waiting one is not pushed again.

**Signing a device out** (Settings → Devices, `DELETE /reins/api/devices/{id}`). Only the approval device may do it,
not to itself, and with the master password hash of the recovery code or master password typed then (counted with
the takeover proofs; the app never sends one it keeps, though the server cannot tell a typed code from one read off a
phone that keeps it). The server first records the sign-out, so that its device id cannot sign in (password sign-in,
SSO before the code is exchanged, token refresh, and every call); then ends the device's WorkOS sessions at WorkOS
(if WorkOS fails, nothing more happens and the phone is told to try again); then, in one transaction, deletes its
Vaultwarden sign-in, its SSO session mappings and any approval role naming it; it drops its push registration and its
"add another phone" requests and logs who signed out what. Sign-ins must use ids the phone API can name (36
characters at most), so every device signed in can be signed out. An open notifications connection of the device
(sync notices, no secrets) stays open until it reconnects.

The limit: a phone keeps the account secret (the recovery code), its integration tokens and the vault key in its
encrypted store, and the app shows the recovery code to whoever has the phone and its screen lock. With it, a
reinstall (a new device id) and a sign-in to the account, they can take the approval role again, and sign the other
phones out the same way. Until the recovery code can be changed, the remedy for a phone in the wrong hands is
resetting the vault, and securing the sign-in methods reachable from it (email, Google, passkeys). Vaultwarden's own
device list (`GET /api/devices`) shows any signed-in device the same list.

- Approving needs the phone's screen lock or biometrics. Denying is one tap. A routine request can also be approved
  from its notification, which works only once the phone is unlocked; that approves exactly what its screen would approve untouched (never an item that looks like a
  code or a password) and creates no permission. Requests that are asked every time, and everything on Autopilot's
  [hard floor](#autopilot), have no such button, and the phone's core refuses to approve them that way (only a
  [Bypass](#bypass) you turned on approves them).
- "Approve and allow for 1 hour" (8 hours after repeated identical approvals) creates an ordinary standing permission
  for the same connection, kind of request and target, shown under Grants.
- The starting rule (chosen during setup, "ask every time" until then and by default): "headlines for its first hour"
  gives a connection you just approved, for one hour, one headlines permission per connected integration that has
  them (Gmail, and integrations with lists; never the vault, payments, the desktop app or MCP servers). They are
  ordinary grants: listed, logged, revocable. A headlines permission is its own kind of scope, which no read, write or
  other check consults: the phone's core uses it only for a Gmail search, whose results it then strips to sender,
  recipients, subject and date (the snippet, taken from the body, is dropped), and for a list in another integration,
  whose items it strips to id, name, sender and date. Reading a message or anything else is never covered by it, and
  what looks like a code or a password still waits. The retired "reads for a day" rule becomes "ask every time" when
  the phone next reads it (also from a restored backup); the 24-hour grants it gave stay until they end.
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

**The other way: `reins vault add`.** A secret sent from the computer goes to the phone in a box from the app's pinned
key to the phone's own key (X25519, XSalsa20-Poly1305), with the request's nonce, the time it was made, the item's
name, the field, the kind of a new item and whether an existing item may be changed. The phone opens it only if the
app's pinned key made it, only for the item, field and kind the request names, only within 15 minutes, and only once
(it remembers the nonces it saw), so the server can neither read the value, nor put another one in its place, nor send
an old request again. The approval is bound to the item as it was when the phone asked: if the vault changed before
you approved, nothing is saved. An existing item changes only when the computer asked for it (`--replace`), and keeps
the earlier value; the terminal and the phone show the same four check digits of the value, so that a value an AI on
the computer sent in your place does not pass for yours.

The phone's key (one per installation, never copied to another phone) reaches the computer once, sealed to the app,
after you approve it on the phone; you then type the twelve digits the phone shows, and the computer keeps the key.
The key never travels in the clear, so a server that answered with its own key could not know which digits to aim
for; the computer never shows the digits it received, and refuses its own eight. What the phone sends back afterwards
(that it saved the value, the item names for `reins vault list`) is boxed from the phone's key, so only that phone can
have written it. A phone that cannot open a box (a new phone) makes the computer say so; it asks for the new key only
when you run `reins vault add --new-phone`.

What the server learns: the item's name, the field and the kind (they travel beside the box so the phone can show the
request), the value's length (from the box's size), and the phone's refusals, which are not boxed: one may say that
an item with that name exists and what kind it is. The boxes use the two long-term keys, without forward secrecy:
whoever gets either one, the computer's key file (readable by any program running as you) or the phone's key, can
open every box recorded between them, the phone's key answer included. There is no way to make new keys yet other
than pairing the computer again or installing the app again. On the phone the value passes through ordinary memory
while it is saved. An SSH key's private half is never given out by the phone, to an AI or on its own screens (the
phone signs instead), though a Bitwarden app signed in to the same account (not possible for an account without a
master password) can show it. Text the phone sends back through the server is printed by `reins` labelled as such and
without control characters, since the server could have written it.

**Git pushes.** The desktop app reads the pack git is about to send and works out what it does: which branches or
tags it touches, whether each update is a fast-forward (asking the host's API when needed), the commits, the files,
and the line counts. That summary is what the phone shows. The digest is computed over the repository, the ref
updates, the SHA-256 of the pack and the push options. The phone's credential is valid only for that digest. When
part of the analysis cannot be completed, the screen shows a note instead of a guess. A push whose history could not
be checked is treated as a force push.

**Reins Secrets** ([secrets.md](secrets.md)). A project's environment is released to the desktop app sealed to its
pinned key, like any secret, for one command: the program (by its absolute path) and its arguments as `reins run`
spawns them, without a shell, the project's folder on the computer, and who runs it (you, or which agent). The phone
decides what the release is bound to and says so inside the sealed answer: exactly that command, or an argument prefix
you picked on the phone (`npm run *`; never for shells, interpreters, launchers or the program alone). A standing
permission binds the same things, and every one of them is compared. Production environments, and those whose
`reins.toml` says `confirm = "always"`, are asked every time unless you change that on the phone; the repository can
make an environment stricter, never looser; those are released for one run only, and the phone caps how long any
release is kept (8 hours, never past the permission that covers it). A program outside the usual system folders is shown
by its whole path, with a warning row when it sits in a temporary folder, in the project's folder or off the login
`PATH`. Values sent from the computer (`reins secrets set`, import) travel in a box
to the phone's own key, as `reins vault add`'s do, with one set of four check digits over the whole batch. A git host's
token found on the computer goes the same way to the phone's integration. **Revoke on all computers** ends the
permissions on the phone and leaves, for each paired computer, a note sealed to its key on the server; its daemon picks
it up, drops what it holds, and only then acknowledges it, and the server keeps handing a note to any daemon that has
not acknowledged it; a note for everything outranks the others, notes dropped to stay within bounds (or forgotten in a
server restart) make the computer drop everything it held from before, and a note that does not open counts as one
for everything. The phone ends the permissions in its own store first and keeps a note it could not deliver to send
again, naming the computers not told yet. The server cannot read the note, and one it made up could only make a
computer ask again.

## What the server sees

The server is a relay, and it is in a position to read what it relays.

**Stored in its database:** your Vaultwarden account (email, master password hash, encrypted vault, devices), the
phone's push token, the AI clients that registered, your AI connections (the name each gave itself, the label it
paired with, host, creation and last-use times), and hashed refresh tokens. The names you give connections on the
phone are not among them: they live in the phone's encrypted account state. A renamed computer's name is kept for its
desktop app in a box from the phone's key to the app's key, padded to one length; the server cannot read or forge it,
and the app ignores it until it has pinned the phone's key.

**In memory only:** waiting requests and their results (at most 10 minutes), pairings and sign-in sessions (minutes),
sealed "drop your secrets" notes for paired computers (a day at most),
which integrations your phone has (ids only, no account names), and the names and tool lists of MCP servers you added
on the phone.

**For phone roles:** the roster (public keys, device ids and names, epochs, signatures), the recovery wrap and a
handover's bundle (both encrypted, opaque to the server), signed ops and checkpoints (sealed with the view key; the
server sees their author, class, size, time and whether they alert), acknowledgements, and the primary's signed
heartbeat.

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

## Deferred decisions

A request that waits for review longer than its Instant window becomes a ticket, and the server keeps it until its
deadline (up to 7 days).

**Stored in clear:**
- the metadata: ticket id, state, deadlines, connection, and the request digest;
- the reviewer's note on a denial, and their "Why?" question, because the agent has no key and must read them;
- a write's minimal result (ids, a status, a link).

**Never stored in clear:**
- the request body: it is sealed on the server to the approval device's box key, and the clear copy is dropped;
- an agent's answer to a "Why?", sealed the same way;
- a read's result: it passes through memory to the agent waiting for it, and is never stored.

The request did reach the server over TLS, as every request does.

**Why a compromised server can't fake an outcome:**
- Every transition the phone makes is signed with its device key. The phone acts only on what its own record proves:
  - it runs a collected read only on its own valid approval of that exact request;
  - it releases only the items the owner picked, once;
  - it signs an expiry only when its own clock and record agree.
- Webhooks carry only those signed events, so a server can delay or drop them but not forge `ticket.approved`.
- Each event names the state it moves from and is accepted once. An approval signed at one point of a ticket's
  history can't be replayed at another.
- Agents and the desktop app pin the phone's signing key. The phone shows it as twelve digits (a connection's More),
  so a key a server swapped in shows as different digits.
- On the desktop, a ticket counts as approved only after the app opens the phone's sealed answer and checks its
  nonce, the digest of the raw command and its expiry. A one-time pass is refused for any call whose shown text isn't
  exactly the raw call, or which contains invisible or bidi characters.

## Circles (shared control)

Circles let another Reins account share control of this one (Decisions v2, 04): a **delegate** (either decides), a
**co-signer** (both decide), a **guardian** (must approve; cooperative in Personal mode, 08 R7) or a **watcher** (is
told). Both accounts need phone roles: each side is known by its roster's **owner key**.

**What the server learns** (08 R8 accepts it): who is in an arrangement with whom, its kind and state, packet deadlines
and states, and when an answer was given. Sealed-sender routing is a later option.

**Never stored in clear:**
- the arrangement (its scope, windows, the two keys): each side's copy is sealed to that side's roster credential key;
- a review packet: the card the helper decides from (never the raw request), signed by the owner's approval phone and
  sealed to the helper's approval phone;
- an answer: signed by the helper's approval phone and sealed to the owner's;
- an invitation's link secret (only its SHA-256) and its proposed terms (sealed to the helper's approval phone).

**Why a compromised server can't add, widen or answer for anyone:**
- **Pinning:** an invitation is a commit-then-reveal over both accounts' owner keys and fresh nonces, and both phones
  show the same six digits. A server that swaps a key must do so before it sees the other side's nonce, so its digits
  are random; the owner's phone reveals for the first answer only.
- **Membership:** an arrangement counts only with both owner-key signatures (the helper's, then the owner's over it and
  the helper's signature), and a phone accepts a copy only when its own side names this account's current owner key.
  A planted link, a wider scope or a swapped helper doesn't verify.
- **Answers:** each one is bound to the arrangement, the packet, the request digest, the packet's nonce and its own
  nonce, and names the signer's roster entry. The owner's phone verifies the helper's roster from the entry the code
  pinned (each step signed by the owner key; never an older head than one it saw), requires the signer to be its
  current primary (a viewer or a replaced phone never counts), and counts each packet once. Paused or ended (here, on
  the owner's phone) means no answer counts, whatever time it claims.
- **Changes** (pause, resume, end) are signed by the changer's owner key, name the arrangement and carry a sequence
  that only moves forward; an unsigned end from the server only suspends.
- **A new owner key** (a recovery, a reset) suspends the arrangement until the two compare a new code; a handover
  verifies through the chain.
- **Obligations are hard:** a co-signer's or guardian's slot is filled only by that person's verified answer; Auto,
  Bypass and Overnight fill no slot of a Circle request; one person never fills two slots (a delegate who is also a
  co-signer doesn't fill the owner's), and nobody shares control with themself. Either deny wins.
- **Nothing runs on the server's word:** the owner's approval phone settles the votes and runs the request through
  its own approval path, after its usual revalidation; an offline phone does so when it next syncs.
- **Unknown widens review, never authority:** a request whose effects can't be read is inside every co-signer's,
  guardian's and watcher's scope, and inside no delegate's.

**Known limits:** the peer's latest roster entries come from the server; withholding a newer entry can only delay a
handover's effect (the old primary's answers still count until the owner's phone sees it). Guardianship in Personal
mode is cooperative: a ward can reset the account (the guardian is told) or use credentials directly; enforceable
guardianship is the Family tenant (P7).

### Guardianship (cooperative, P7)

Personal guardianship is **cooperative and tamper-evident** (08 R7), and called so everywhere. The ward's phone holds
the credentials and evaluates the policy, and the server is blind, so leaving can't be prevented: it is made
**visible**. Enforceable guardianship is only the Family tenant, where the family holds the credentials.

**The terms** (signed into the arrangement at setup, so the server never sees them): the notice the **ward** chose
before the code (0, 3 or 7 days: nothing longer, no "permanent"), a minor mark, and whether the weekly digest is on.
The guardian's phone sees the notice before it accepts.

**What the server learns** beyond other Circles: each signed change of a guardianship and when it arrived
(`reins_circle_changes`: its body names no scope or terms), when a ward's end takes effect (`effective_at`), and the
notices it writes to guardians (`reins_guardian_notices`: a reset, a deletion, a roster change, a change of the link).

**What the guardian sees:** the request card within scope, as a summary (no bodies, no detail), and an opt-in weekly
digest of automatic decisions in scope: the day and the matrix line, never a body, a recipient or a title. No inbox,
chat history or vault. **The ward sees** the scope, every decision the guardian made, and each note.

**Minors and age signals:** only the guardian proposes a minor mark, and it counts only once the ward's phone signs its
acceptance after the owner check (biometric, asked by the core), with Apple's Declared Age Range or Google Play Age
Signals where available (an adult's range refuses it), else recorded as accepted by guardian and ward. The mark is in
a signed change the server stores in clear, so it says only whether a platform backed it: never the age range, never
which platform. The server does learn that a ward accepted a minor mark. A minor's end waits for the guardian's signed
consent, which the core also gates on the owner check.

**Whose clock:** a ward's end lapses on the ward's phone only when the server ends the link (its sweep, on the server's
clock), so a ward's clock set forward ends nothing early; the guardian's phone uses its own clock. Only proof
suspends a ward's guardianship (the guardian's owner key changed, through its roster chain), and then the ward can end
it at once. A link merely missing on the server, or ended there without a signature, is never a reason: the ward's end
request is then kept on its phone and lapses after the full notice, on its clock (the server can't attest it any more;
a cooperative limit). The server answers an error, never an empty list, when it can't read a phone's links. Ending
steps (an end, a ward's end request, a guardian's consent) are never refused by the per-link change cap.

**Notices** are returned until the guardian's phone acknowledges them (`POST /reins/api/guardian/notices/seen`), so one
lost in transit comes again. A guardian's phone counts a guardianship's changes strictly in order: if the server
withholds one, the later ones wait ("Waiting for earlier changes"). Only what needs the guardian notifies (an ask to
leave, an end with notice, an early end or pause, a reset, a deletion, a recovery); the rest is listed quietly.

**Abuse pass** (each with a test or a stated limit):
- **Coercing an adult:** a notice is 0, 3 or 7 days (`circle::tests::notice_days_are_0_3_or_7`); a guardian can't mark
  an adult a minor alone (`guardian::tests::a_guardian_cannot_mark_an_adult_minor_alone`,
  `guardianship::a_minors_end_waits_for_consent`); the guardian can't turn the digest on
  (`guardianship::no_notice_ends_at_once_and_only_the_ward_sets_the_digest`).
- **A ward ending early:** the ward's phone signs an `EndRequested` that takes exactly its notice and keeps the review
  until then (`guardian::tests::obligations_stay_during_the_notice`, `guardianship::guardian_end_to_end`); a ward
  can't pause one (`circle::tests::wards_cannot_pause_guardians`, refused by the server too). The guardian's phone
  checks every change against its own copy of the terms **and the time the server logged it**, so a plain end, a short
  notice, a backdated request or a pause shows as "Ended early" or "Paused by Sam"
  (`guardian::tests::an_early_end_is_flagged_to_the_guardian`,
  `circle::tests::a_backdated_end_is_refused_and_never_shortens_the_notice`). The server refuses a request dated
  more than 5 minutes from its own clock. A change without its arrival time is refused (fail closed), and a
  guardianship step is refused on any other kind of link (`circle::tests::a_helper_still_only_ends_outside_a_guardianship`).
- **The server forging or hiding notices:** it can drop or delay them. It can't forge a signed change (the guardian's
  phone verifies the ward's owner key) or a roster change (checked from the anchor the code pinned); what nobody can
  sign (a reset, a deletion, a link that vanished) shows as "Reported by the server"
  (`guardian_notices::tests::*`, `guardianship::guardian_end_to_end`).
- **Escaping by reset or deletion:** each guardian is told before the rows go
  (`guardian_notices::tests::reset_notifies_every_guardian`, `a_deleted_account_notifies_before_erase`).
- **Content beyond scope:** guardians get summary cards only (P6's redaction), and the digest carries matrix lines.
- **R7's limit:** an uncooperative ward can start a new account, or use its credentials directly; the guardian learns
  only of the reset or deletion. Use the Family tenant where that matters.
- **Recovery drops oversight:** a recovery changes the ward's owner key, so the guardianship suspends (P6) until the two
  compare a new safety code. The guardian is told ("Account recovered", checked against the chain), but reviews nothing
  until a new invitation.

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

**Autopilot on the desktop.** The desktop app shows who decided a request (Autopilot, Overnight, a bypass,
Lockdown), Autopilot's confidence and reasons, and, while a request waits, which way Autopilot leans. Only who decided is
written to the activity log (`activity.jsonl`). The leaning, the confidence and the reasons are kept in the app's
memory, asked of the server without waiting. An AI that tails the log therefore does not learn how close its asks
come to being let through. This is not a wall against a same-user agent, which can read the app's session (above) and
ask the server for the same as the app. The server shows these only to the connections the phone paired as desktop
apps, never to an AI's own connection.

A change of Autopilot's mode asked from the desktop app is applied at once only when it makes Reins stricter. Anything
looser (Auto, leaving Lockdown, "This was wrong" towards approving) needs two things:
- the app's own proof of the call, boxed from its key to the phone key it pinned, so the server can neither make one up
  nor send one twice;
- the owner's approval on the phone, held, with the phone's owner check.

A bypass and Overnight start on the phone only.

**Who runs a command** (Reins Secrets). The daemon tells a person from an AI agent by the process that asks it
(`reins run` connects to a socket only this user can open; the daemon reads the caller's process id from it) and that
process's ancestors: a known agent harness among them makes it that agent, and what the caller says can only make it an
agent. A program of the same user that cuts itself off from its ancestry (a double fork, a service of the user) and
says nothing passes for a person; that is the same-user limit above, and running the daemon as its own user closes it.
The approval names the exact command either way. On Windows the daemon does not take over `reins run` yet, and the
runner is what the caller's environment says.

**Local mode** (not logged in): the GitHub token comes from `gh auth token`, an environment variable or a file the
agent's user can usually read. Local mode protects against an agent's mistakes. Phone mode protects against a hostile
agent.

## Hooks

Harness hooks compare commands and file paths with patterns ([harnesses.md](harnesses.md#the-guard-rules-guard)).
They are a heuristic first line: they catch the obvious (force pushes, `rm -r`, `terraform apply`, reading `.env` or
private keys) and, on purpose, match every word of a command, so false positives are expected (a test edit that
mentions `.env`, a commit message). Those are handled on the phone: the question shows the rule and the word that
matched, a `.env` question is an ordinary yes or no that Autopilot can learn, and Bypass answers it.
Determined code can still evade the guard: a name built at run time, a command hidden in a script or an alias, a
tool the hook does not see. The other layers are what hold then: credentials stay on the phone and are released per
action, grants are narrow and time-boxed, Autopilot judges each request, and an OS sandbox (a separate user, a
container) keeps the agent from the files at all. Smarter, context-aware judgment of commands is coming from
Autopilot's risk model. When the phone does not answer, the hook denies by default (`on_no_answer = "deny"`).

### Safety nets

When a hook's request is allowed, the desktop app can make it recoverable first (Decisions v2 P3,
[01 §5](design/decisions-v2/01-decision-engine.md), [08 R2](design/decisions-v2/08-review-resolutions.md)).

What it does:

- **Deletes:** `rm` becomes `reins net run … -- rm`, which moves what it deletes into the Reins trash
  (`~/.local/state/reins/nets`, kept 7 days).
- **Destructive git:** `branch -D`, `reset --hard`, `worktree remove --force`, `clean -f` and force pushes keep a
  backup ref (30 days) or a trashed copy first.
- **File tools:** they get a copy of the file they change.

A rewrite replaces one command word, and only after the syntax tree shows nothing else changed. Commands that could
give that word another meaning are never rewritten:

- a function or alias named like it;
- `eval` or `source`;
- a command word that isn't literal.

Only the commands up to the first one off the allowlist are netted. What follows them runs after them, can't change
them, and keeps its own level. Among them, a variable is allowed only when it is a one-letter name the script itself
set to plain text before, and a `for` loop only when it goes over plain words.

A net that can't be made deletes nothing and leaves the request at its own level.

`reins net run` is bound to the hook's plan:

- the plan names its store;
- it runs only the planned programs, in the planned folders, within 12 hours;
- it deletes for real only the scratch roots the hook recorded.

Nothing before a netted command may set `HOME`, `PATH`, `XDG_*`, `GIT_*` or `REINS_*`: only one-letter names, which no
shell, git or Reins reads.

Activity says Netted only once something was really kept. The trash keeps at most a tenth of its disk, oldest first.

A force push is pinned to the remote tip that was kept. Backup refs live under `refs/reins/backup/`, so a
`git push --mirror` would publish them; Reins never nets a mirror push.

Nets are a guardrail, not a boundary. An agent running as the same user can delete without a hook, or edit the
trash. The phone shows the net it is told about read-only, and never asks less because of it. The net travels as its
own argument, which only the hook sets. `reins ask`'s words are the agent's own, so they can't claim one.

## Autopilot

Autopilot runs only on the phone. Nothing it uses or learns leaves the phone. It is limited three ways.

- **Hard floor.** In Manual and Auto, some requests always wait for you: new connections, standing permission
  requests, account listings, secret releases, SSH signatures, everything that is asked every time (above), destructive
  MCP tools, pushes that rewrite or delete history (or whose history could not be checked), hook questions in one of
  the guard's destructive groups, uploaded files that may hold something to run, and anything from a connection
  paired less than 10 minutes ago. Lockdown denies, and never approves. Bypass is the exception, by design (below).
  The phone puts a hook question in a group itself, reading everything it carries with its own copy of the guard's
  built-in rules (`reins_policy::guard`), whatever its layout; the most severe match wins, and the words of a
  destructive command count where a shell runs them (not in the source a heredoc feeds Python). The hook runs where
  the AI does, so its label is only shown, and can make a question stricter, never looser. Questions about private
  keys, cloud credentials and saved tokens are asked every time too, and so is any the phone cannot read with
  certainty. Certain is a strict, small part of shell only (`reins_policy::strict`): plain and quoted words, `;`,
  `&&`, `||`, `|`, simple redirections, `NAME=value` before a command, a heredoc into a program that does not run
  its input or into a shell, a few wrappers with their exact options (`sudo`, `env`, `nice`, `nohup`, `time`,
  `timeout`, `command`) and `sh -c` two levels deep. Anything else (`$`, a substitution, parentheses, braces, a
  program that runs code it is given, an interpreter, a pipe into a shell, a Windows path, a character outside ASCII,
  a detail cut short) is uncertain: shown as "Couldn't read this", never remembered, never one tap, never
  Autopilot's, but an ordinary yes or no on screen, and Bypass approves it. A question is never less guarded than
  before this reading, except a `.env` read, read with certainty (a test compares a corpus with the previous rule). A `.env` question is judged like any request. A remembered answer to a desktop question covers
  exactly what was asked, on that computer (bound to the desktop app's key): that file by its full path, that command
  in that folder, or that question; never the topic the computer chose. Autopilot learns a question in the `.env`
  kind only when everything its command does is reading `.env` files.
- **Prompt-injection rule.** The model scores each request twice: once on the facts the phone verified (service,
  action, target, history), and once with the AI-written text added (reason, email body, commit messages, arguments).
  The approve score it uses is the lower of the two, and the deny score the higher. AI-written text can therefore make
  a request look riskier, never safer.
- **Approve once.** Autopilot never creates standing permissions of its own accord. Its automatic decisions never
  train it, so it cannot reinforce its own mistakes.

### Bypass

Bypass is the switch for "approve everything, I'm not watching". While one runs, every request is approved at once,
of every kind and every item in it: the hard floor above included (secrets, SSH signatures, purchases with what their
screen would pick and within your budgets, destructive tools, force pushes and deletions, flagged uploads, account
listings, standing permission requests), from a connection however new. A global bypass applies to every AI that is
not locked down, whatever mode it has of its own, and turning one on also approves what already waits in its scope.
It needs no model, so it answers while the app is closed (a push, the notification extension). That includes the
desktop app's questions: a harness hook's question (a secret file, or a force push) is answered yes within a poll of
the hook, which then lets the command run, so an agent left working is never held by a guard question.

Its guards:

- **Turning it on is the owner's act.** It is held to confirm on the phone, then needs the phone's biometrics or
  screen lock, on every path that starts one (the Autopilot page, an AI's page, the quick switcher). Siri, Shortcuts,
  Focus filters and notification actions cannot start one. Stopping it and Lockdown need neither.
- **Account takeover stays with you.** A new connection (pairing an AI or a computer) and another phone joining the
  account never reach Bypass; they always wait for you.
- **Nothing outlives it.** A standing permission an AI asks for during a bypass is marked as that bypass's (the
  global one, or that AI's own). It counts only while that bypass is in force: every evaluation checks this in the
  same database transaction, so it stops counting the moment the bypass ends (stopped, run out, locked down, or
  replaced by another mode), whether or not it has been taken back yet. It is not stored at all if the bypass ended
  before the approval finished; the request then waits for you. Taking it back is cleanup: done in the transaction
  that ends the bypass, and again by a sweep whenever Autopilot is read; each says "Ended with the bypass" in Activity,
  once. A timed bypass also caps it at its planned end. An AI's own bypass takes back only that AI's.
- **It is visible.** An ongoing notification (Android) or Live Activity (iOS), the red mode pill and the Autopilot page
  show it while it runs, with Stop; one without an end shows how long it has run.

Details: [autopilot.md](autopilot.md).

## Organizations (Managed work profiles)

Reins for organizations (Decisions v2 O1, `REINS_ORGS_ENABLED`) adds a second kind of account: a **work account**,
an organization's member, which the phone and the desktop keep as a separate profile next to the Personal account.

- **Separate accounts, separate profiles.** WorkOS keeps one identity per email, so a work account (`dana@acme.com`)
  and a Personal account (`dana@gmail.com`) never share one. Each profile runs its own core on its own data
  directory with its own data key (`reins_core::profiles`), and pairs its own agents. The org never receives
  anything from the Personal profile: no endpoint, console page or event names an account that isn't its member.
- **Who an org's member is** comes from verified sources only: a domain counts after its DNS TXT record; the
  tenant's WorkOS organization and directory are bound only when WorkOS says that organization holds one of the
  tenant's verified domains (and the directory is that organization's), each to one tenant; a sign-in counts as the
  org's when WorkOS names the tenant's organization on it and the method is the tenant's (SSO; Google; GitHub with an
  active membership of the tenant's GitHub organization). Email domains alone grant nothing. An existing account
  that isn't the org's member is never captured, by sign-in or by the directory.
- **Directory events** are read from the WorkOS API with the server's key; a webhook only wakes the sync. An event
  applies only to the tenant bound to both its directory and its WorkOS organization, and creates or links members
  only on the tenant's verified domains. A periodic listing fills in what events missed, but never reads a broken
  listing as an empty company: a page without its data is an error, an empty listing removes nobody, and more than a
  few members absent at once wait for an admin to confirm exactly those (a two-person change). Moving a tenant to
  another directory unlinks the old one's members instead of removing them. Only an account that was provisioned
  as a placeholder (an invitation, no keys) can be taken over by a first sign-in.
- **Work accounts sign in only through the org**: password and API-key sign-in are refused (after a correct
  credential, so a guess can't tell work accounts apart), and the passkey rule is waived only for a sign-in the gate
  verified through the org's identity.
- **Roles and two people.** Admin actions (roles, sign-in, plan, domains, removing members) need a second admin's
  approval whenever another admin could approve; the proposer can never approve their own change, and approvals
  re-check everything as of then. Removing someone who holds a privileged role takes what making them one takes. Org
  owner and Security admin go to people, never groups. An org always keeps an Org owner; when the directory removes
  the last one, two Security admins together may make a new one. All of it is recorded in an append-only,
  hash-linked event log.
- **Removal** (directory suspend or delete, or an admin's two-person removal) signs the account out everywhere,
  removes its AI and desktop connections and refresh tokens, revokes its enrolled devices (a revoked device can't
  enroll itself again), in one transaction, typically in milliseconds after the webhook. Each member's state is the
  org's own (`reins_org_members`); one org per account; the directory restores only what it suspended.
- **On the phone, each profile is its own**: a notification, its actions and a push name the profile they belong to
  (pushes carry `p`, a tag of the account); the app never acts on another profile's with the running profile's core,
  and only opens the app on that profile. A work profile's core gets no access to the phone's own data (contacts,
  calendar, texts), and on iOS keeps its own Google sign-ins.
- **Device enrollment** records a P-256 key the phone proves it holds, the platform's attestations and posture.
  The server verifies Android key attestation and App Attest against pinned roots (O2); an org's policy can require a
  verified attestation, a screen lock, no root or jailbreak, and minimum OS and app versions. A device that fails
  may not decide, and its reviews go to others.
- **The admin console** has no two-step sign-in yet; accounts with two-factor sign-in are refused there. Approving a
  pending admin change takes the approver's enrolled phone (a device signature), so a console session alone approves
  nothing.

### Zones from facts kept by id (Decisions v2 P2)

The context graph decides a request's zone from facts only, looked up exactly. Two kinds of party are kept by the
service's stable id, never by what an agent writes or by a name someone else can take:

- **Telegram:** a chat is its numeric id. Before the matrix reads a send, the phone resolves the chat it names (a
  handle, a title, an id) through Telegram exactly as the send would, and evaluates the chat that would receive it.
  The owner's own chat (Saved Messages) is Mine by the account's own id. A chat Telegram can't find is Outside and
  isn't a question; a handle or a name is only a label, and a title's label always carries its kind ("Mom
  (group)") and never starts with `@`, so it can't pass for a handle. An automatic approval sends to the chat id it
  was evaluated for, never to whatever the name resolves to by then. A fact kept by an `@handle` before ids is never
  moved: when Telegram resolves that very handle (its username lookup, not a title), a question card for the chat's
  id preselects the handle's zone and the owner confirms it; email and phone facts are never carried over.
- **GitHub:** an owner the phone knows is kept by the account's numeric id (`#<id>`); the login is only its label.
  A request names logins, so the phone asks GitHub which account each login is now (cached for an hour) and matches
  by id: a login the owner gave up and someone else took is Outside. Without an answer (offline) it is Outside too.
  The connected account is read as its token's own user. Facts kept by login before this move onto the id the
  first time GitHub answers for them. The computer's own `[auto]` facts and an org's policy facts are still written
  by login.

### Decisions in Managed mode (O2)

A work account's agents never reach a phone: their calls go to the org's **decision service**, which decides with the
org's policy, and an **executor** runs what was decided with the org's own credentials.

- **Keys, per tenant** (sealed by a KMS seam, `org::kms`; with the built-in `LocalKms` a master key file outside the
  database):
  - the **issuer key** signs the org's policy snapshots and its roster (members, roles, groups, enrolled device keys,
    and whether each device may decide);
  - the **decision service key** countersigns decisions and signs audit checkpoints and export heads;
  - a **data key** seals request bodies, results and receipts at rest, bound to their request.
- **Policy** is drafted in the console and activated by a two-person change, which needs the proposer's and a second
  Security admin's device signatures (two different device keys); only then does the issuer key sign it. Every
  decision names the snapshot hash it was made against.
- **Evaluation** composes the org's rules (deny, review with named reviewers, permit) with the member's own policy,
  which can only add asks and denies (a member's permits are dropped, and their matrix only makes cells stricter).
  The always-ask floor and L3 are hard reviews; the org's cell review always applies when its matrix asks. With no
  active policy, nothing is decided.
- **Reviews** are answered on reviewers' enrolled phones: a P-256 signature over (org, request, digest, decision,
  snapshot, time), from a device that is enrolled to this org, not revoked and passes the posture rule, by a person
  the obligation names. The requester can't approve an org review (`exclude_requester`); one person counts once. The
  phone recomputes the digest and builds what it shows from the request itself, so what it signs is what it shows.
- **The executor** is a separate program and trust domain (`crates/reins-executor`), registered to one tenant with
  its own Ed25519 key by a device-signed two-person change. It pins the org's issuer and decision keys out of band
  (`reins-executor pin`) and, before acting on any job, checks the countersignature, the issuer-signed policy and
  roster (at most 15 minutes old), that the request it would run has exactly the countersigned digest, every
  reviewer's device signature, and its own re-evaluation of the request with the same engine; a newer, stricter
  policy makes it stale. It refuses anything else and reports why in a signed receipt. It never runs a request twice.
- **Leases.** A work daemon's repository credential is asked for like any request (judged by the repository's zone
  and the org's rules), minted by the executor for exactly that repository id, sealed to a key only the daemon holds,
  and lives 15 minutes. Removing a member revokes their leases at once, and the executor revokes the credential at
  GitHub within its next pass. Only the executor that minted a lease holds its token, so only it is told to revoke
  it, and only it can say it did.
- **Lease lifetime against GitHub's.** GitHub's installation tokens live one hour, whatever is asked, so a token can
  outlive its 15-minute lease. What holds it to the lease:
  - the executor mints only while the countersigned lease lasts, and revokes each token at its lease's end itself,
    first thing every pass and without the server;
  - the server's sweep ends every lease at its end (its sealed copy is deleted), and a token minted after that is
    never handed out and goes to the revocation queue;
  - deprovisioning revokes at once, as above.

  **The residual window:** an executor that is offline (or was removed) from a lease's end until the token's own
  expiry leaves that token valid at GitHub for up to an hour after it was minted, up to 45 minutes past its lease.
  Nobody else holds the token, so nobody else can revoke it. Once that hour is over the server records
  `lease.token_lapsed` and drops it from the queue.
- **Flooding.** A connection may have 50 open org requests (under review, approved or running); beyond that the
  agent is told to wait (03 §8), and nothing is recorded.
- **Audit.** Every decision, review, execution, lease, policy and executor change and admin action is an event in the
  org's hash chain (O1's event table, kept as is), checkpointed and signed every 10 minutes. An export ends with a
  signed head; `reins-audit verify` checks every hash, link and signature and that the head covers the last event,
  and, given a checkpoint kept from before (an earlier export or a witness), that the chain wasn't rewritten or rolled
  back since.
- **Retention.** Request bodies are kept 0, 30 (default) or 365 days, set by a device-signed two-person change.

### Audit packaging (O5)

- **Timestamps.** With an RFC 3161 authority configured, every checkpoint (its signed envelope, byte for byte) is
  timestamped; the server checks only that the answer is for its own imprint and nonce, and keeps the authority's
  response for `openssl ts -verify`. A checkpoint with a timestamp is an anchor a compromised server can't move: a
  rewritten chain, even re-signed with the decision service key, fails `reins-audit verify --anchor` against it.
- **Pruning** deletes events older than the plan keeps (Team 30 days, Business 1 year, Enterprise 7 years) only below
  the newest stored checkpoint signed before the cut-off, which stays as the chain's anchor; a chain that doesn't
  verify as stored is never pruned (the evidence of an edit stays).
- **Streaming.** A destination's configuration is sealed under the tenant data key and never shown back; turning it on
  is a two-person change with device signatures that names the configuration's digest. A change lapses pending
  enables, and a destination that is on can't be changed until two people turn it off, so one admin can neither
  redirect nor silence an approved stream. Deliveries go through the outbound
  guard (HTTPS, public addresses, no redirects) and are not audit events themselves; a destination starting or
  stopping to fail is.
- **Evidence reports** are built only from an export that verified with the org's decision service key, offline by
  `reins-audit report` or in the console, and are deterministic.
- **Bring your own KMS.** The master key that seals every tenant key can be wrapped by the operator's AWS KMS, GCP
  Cloud KMS or Azure Key Vault key, unwrapped once at launch; if the unwrap fails the server doesn't start (no
  fallback to a local key). **Limit:** the unwrapped master and the signing keys are in the server's memory while it
  runs, so revoking the KMS key stops the next launch, not the running one; HSM-held signing keys (06 §3) are later.

### Approval routing (O3)

Org reviews name people by **relationship** as well as by group or role: the requester's manager, the repository's
code owners, a resource's owner, whoever is on call, a group's backup. The decision service resolves them at submit
from verified sources only (never from the agent): the directory's `manager_email` (or a two-person admin change on
the Team tier, which has no directory; on tiers with a directory only the directory's word counts), the default
branch's `CODEOWNERS` through the org's read-only GitHub App (logins count only when SAML or the member's own
authorization proved them), the on-call provider's API, and the policy's signed resource tags. The requester is never
a reviewer through any of them, and a member without a device that may decide (or a confirmed web passkey) is
skipped, so their reviews route to others.

- **Escalation adds, never removes.** An obligation nobody can satisfy (no manager on record, owners unlinked, the
  on-call API down) escalates at once along its chain (by default the group's backup, the manager, then Security
  admins); an unanswered one escalates when its window passes. After the last step it expires as not approved: it
  never auto-approves and never falls to the requester. Every resolution is audited with its source and a hash of the
  provider's answer (`review.resolved`).
- **What executors trust.** The countersigned decision carries the resolution. An executor checks groups, roles,
  named members, the manager and group backups against the issuer-signed roster (which now lists managers and
  backups), and drops anyone they don't admit. **Code owners, on-call and resource owners are dynamic: an executor
  takes them on the decision service's countersignature.** A compromised decision service could therefore name an
  accomplice for those obligations (the audit records the evidence it claimed); groups, roles and managers it can't
  forge.
- **Signed web approvals (L0–L2 only).** Slack and Teams carry one line (requester, agent, effect kind, resource,
  level, who must decide: never a body) and a one-time link: 32 random bytes stored as SHA-256, for one member and one
  request, valid until the review's deadline or an hour. The page needs that member's own console SSO session, and
  the answer is a WebAuthn assertion over SHA-256 of the same review message a phone signs (org, request, digest,
  decision, snapshot, time), at the server's origin, user-verified. The passkey must be one the member registered in
  the console and confirmed on their enrolled phone (the phone signs the key's hash and registration time; a refusal
  is an unsigned revoke); only then is it in the issuer-signed roster, where executors verify the assertion again and
  refuse it for anything they judge L3. L3 always needs the app: no link is issued, an old link opens "Open the Reins
  app", and the server refuses it too.
- **Device binding is as the browser reports.** Registration uses attestation `none`, so "device-bound" (backup
  eligibility clear) and user verification are the authenticator's own word: a synced passkey that lies about it
  isn't caught. This is accepted for L0–L2 web approvals; the console says so.
- **Channels never decide.** Inbound Slack events need a valid `X-Slack-Signature` within five minutes; Teams
  activities a Bot Framework token (issuer, the org's bot as audience, a key endorsed for the channel, the activity's
  `serviceUrl` in the token, Microsoft's connector hosts only). Both are acknowledged and change nothing else. A Teams
  bot is one org's, claimed by proving its secret to Microsoft; reinstalling a channel with another app or secret
  turns it off until two admins turn it on again, and a pending proposal to turn it on is bound to the install it was
  made for.
- Changes to who approves (managers, backups and windows, on-call sources, channels, the GitHub reader) are
  two-person, device-signed admin changes. Their secrets (API keys, bot tokens, the reader's key) are sealed under the
  tenant's data key as posted and never shown again.

What this doesn't do yet:
- With `LocalKms`, the issuer key, the decision service and the database share a host: an attacker holding all
  three can sign a policy. The split 08 R5 relies on (executing a non-compliant action needs the issuer key or an
  executor) holds fully with an HSM or customer KMS that enforces the two-signature rule itself (O5).
- Checkpoints aren't published to an external witness yet (O5), so detecting a rewrite needs a checkpoint someone
  kept.
- **Attestation:** a chain's certificates must be within their dates when the attestation is checked (a pinned root
  is trusted by its key, whatever its certificate's dates). Android chains are checked against Google's attestation
  status list, fetched at launch and every 6 hours; when it can't be fetched an attestation is verified without it
  (its record says `unchecked`), and the next list re-checks every verified device, so one revoked since stops
  counting as attested. Apple publishes no revocation for App Attest, so none is checked.
- **Play Integrity is recorded only, never verified:** this server doesn't call Google's verdict API (that needs the
  org's own Google Cloud service account). A Play Integrity token never makes a device attested, whatever it says,
  and its record says `recorded_only`. A policy that requires attestation needs Android key attestation or App
  Attest.
- **A work account's desktop git calls** (`/reins/desktop/calls`: pushes and fetches through the desktop proxy) still
  go to the work profile's phone, not to the org's decision service. They are decided on that phone with its own
  credentials, as a Personal account's would be, and never reach the org's audit log. The org path for git is the
  lease API above; the desktop daemon's side of it (fetching an org lease for an org remote) isn't built yet. Until
  then, an org that needs every git credential to come from the org keeps work repositories off phone-held GitHub
  accounts (05 §5, 08 R6).


### Policy as code, simulation and staged rollout (O4)

- **The policy repo is input, not authority.** An org may keep its policy in a GitHub repo (`acme/reins-policy`,
  bound by a two-person change). The server reads it with the org's *GitHub reader*, a second App whose private key
  and webhook secret are sealed under the org's data key; webhooks are verified (`X-Hub-Signature-256`, constant
  time), must come from the org's installation and its bound repo, and are acted on once. A PR gets a check run (its
  compile errors, or its simulation line); a merge saves and simulates a draft and only **proposes** the activation,
  as the system. A first Security admin confirms it on their phone (becoming its proposer), and a second approves it
  on theirs: merging the PR, re-posting the webhook or being the only admin activates nothing. Repos are read within
  200 policy files and 1 MiB.
- **Simulation is evidence.** Every decision keeps its body-free input (effects, zones, levels: decision metadata,
  never the request body), and a draft is replayed over the last 30 days with the very evaluator the live service and
  the executors run, each case at its own time. Replaying the *active* policy must reproduce what was recorded; a
  difference is *drift*. Once a policy is active, `ActivatePolicy` names a report made for exactly that draft's hash,
  at most a day old, with no drift: a draft changed after it was simulated is another hash and needs its own. Reports
  name request ids only; opening one is an audited read by an Auditor or a Security admin.
- **Two people, always.** Activation takes two holders of the capability (their device signatures), even with
  two-person approval turned off and even with a single Security admin (then it is refused). A Family's guardians are
  the exception: every guardian holding the capability decides, so a single guardian activates alone.
- **Signed staged rollout.** With a policy active, activation makes the draft a *candidate*: the issuer signs it,
  resolves the pilot group, and pre-signs every rollout step (pilot, 25%, everyone, then the rollback) as `Rollout`
  envelopes. Each stage carries `not_before`, activation plus the dwell of the stages before it. The decision service
  publishes the next stage only after the dwell with enough decisions in both cohorts and the guards green (ask rate
  and median time to decision), and the rollback when a guard trips; a rollback is terminal and the candidate goes
  back to draft. A member's cohort is a hash of (rollout, member), percentages nest and the pilot stays in, so nobody
  flips back and forth. Every decision records the snapshot and rollout step it was made under.
- **What a compromised decision service can and can't do.** Executors hold every signed step they've seen and refuse
  a decision whose snapshot, or a job whose current policy, isn't the member's effective version under them: the
  service can't put a member outside the cohort on the candidate, nor keep anyone on the candidate after a rollback.
  A step whose `not_before` is still ahead counts for nobody, by the executor's own clock. What it *can* do is publish
  a pre-signed stage as soon as its time comes even if a guard should have tripped, or withhold the rollback: either
  only moves a rollout that two Security admins approved, no faster than its approved schedule. A decision made under
  the previous stage still runs for two minutes after an executor first sees the next one; older, it is stale.
  Executors keep the steps they've seen on disk (`state_dir/rollouts.log`), so a restart doesn't forget them.

What this doesn't do yet:
- An executor that never received a rollout's steps (the decision service withheld them from the start) can't tell
  a candidate from the active policy: it falls back to O2's check, any issuer-signed snapshot and the policy named
  current. Publishing the steps to a witness (O5) closes this.
- One Connection admin can replace the GitHub reader's key and webhook secret. That lets them post check runs and
  propose drafts, never activate one.
- A rollout advanced by hand through its stages with too few decisions reaches "everyone" without a stable baseline:
  there only the time-to-decision guard applies.

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
- Phone roles, server stage: the legacy "add another phone" join (accounts without a roster) still compares six
  digits over the new phone's key alone, which a server able to try about a million key pairs could match
  (enrollment as a viewer uses the commit-then-reveal exchange instead); a recovery is not emailed (the hosted server
  sends no email), only pushed to the old primary; with the account secret on every viewer, an unlocked stolen viewer
  can recover to primary (above).
- Phone roles, phone stage:
  - **Removal is not revocation (Q1 = B).** A removed viewer, or a replaced primary, keeps the account secret it
    holds. It reads nothing written after it left (the view key is rotated), signs nothing the others accept, and
    answers nothing: a phone that does not decide answers no request, pairing, upload or purchase, whatever its own
    Autopilot modes or grants say. But it can still open the vault and, with the master key, the recovery wrap, and so
    recover to primary. Only rotating the account secret (a vault reset now; recovery-code rotation is a follow-up)
    revokes it for good.
  - **Tokens stay with the primary.** In the op log a token (and the payment mandate key) is sealed to the
    credential key on top of the view key, so a viewer syncs "GitHub · octocat" and never the token. A phone that sees
    it is not the primary (a viewer, a removed phone, a primary replaced by a handover or a recovery) deletes every
    integration token and the mandate key it holds, at once; until it runs once after the change, they are as safe as
    that phone's lock.
  - **What a viewer adds is a proposal.** It is inert until the primary keeps it. The primary shows Keep / Discard
    from the payload it opened (sealed to the credential key), not from the viewer's title, and keeps exactly what it
    showed: new rows only (nothing replaced), an integration's account with its token or an MCP server with its own,
    the columns a proposal may set (what a server says of its tools is listed again by the primary), never a key of
    Reins, of payments or of this installation, a grant, a mode, a paired computer or a setting.
  - **A viewer cannot stop the log.** An op sealed under a key id no roster entry brought is refused, not waited for;
    an op signed before a phone's anchor is skipped once the phone has a state.
  - **A phone that recovers verifies before it signs.** It reads the log with its recovery proof (it is no member
    yet), applies it under the roster before the recovery (the outgoing primary's checkpoint, then the signed ops),
    and names as the old primary's last op only what it verified, never past a gap (the old primary's ops after a
    gap are dropped, with a warning). Nothing the old primary's key can produce blocks a recovery, since a thief may
    hold it: what does not check out is for the user to confirm. With anything of the old primary in the log, its
    last heartbeat must verify (its key, an epoch of its tenure), reach its checkpoint and its newest op, sign the
    same history at its head, and count no more of its own ops than were verified; a missing, stale or forged one,
    a server that serves less than that heartbeat signed, or a server that claims the old primary wrote more than it
    serves, asks the user ("Recover without the lost phone's last status?") and signs nothing until confirmed. A
    new account (nothing of the old primary in the log) needs no heartbeat. A checkpoint the phone cannot verify
    asks to start afresh. Whatever a recovering phone takes from a checkpoint it did not apply op by op (which a
    thief holding the old primary could have put just before) is untrusted, marked before it loads and tightened
    row by row as it is materialized, until every row of it was seen: every grant revoked, every mode Manual
    (Lockdown stays) and no Bypass, Autopilot's learning and approved targets forgotten and its profiles reset,
    paired computers to pair again, MCP servers' tools listed again and their address to be checked, every "always
    ask" asking, the starting policy the strictest, payments with no limit, tolerance, default card or address,
    phone pay or merchant account, this phone's payment ledger kept, every connection's Autopilot age reset, secrets
    seen only added to, and no token taken from it. What the new primary sets afterwards is never tightened again.
  - **A replaced primary cannot stop the others.** Only the current primary's heartbeat at the current epoch can show a
    fork (and stop applying for good); one from a replaced primary or an older epoch that disagrees is a warning ("Your
    devices may disagree"), never kept.
  - A phone with no roster of its own that recovers cannot tell a current roster from an older, consistent one a
    server kept (with that time's recovery wrap). The recovery shows the members it would recover into, the epoch and
    the owner key's fingerprint, and signs nothing until the user confirms; a phone that verified the roster before
    refuses anything older.
- Hooks are pattern-based (above).
- Autopilot's model was trained and evaluated on synthetic data ([model card](../tools/laya/MODEL_CARD.md)).

To report a vulnerability, see [SECURITY.md](../SECURITY.md).
