# Rewarden desktop app: the git proxy

Date: 2026-09-30. Status: approved direction, being implemented.

## Why

AI agents do git work on the user's computer: clone, fetch, commit, push. Routing that through the phone makes no
sense (packs are megabytes to gigabytes), and handing the agent a GitHub token gives it everything the token can do,
forever. The desktop app is a small daemon that sits between git and GitHub:

- git talks plain HTTP to the daemon on loopback (`url.<proxy>.insteadOf` rules), the daemon talks HTTPS to GitHub;
- the agent never sees a token: the daemon adds the credential to the upstream request and keeps it in memory only;
- every read and every push is allowed first, by the **phone** (through the Rewarden server, the normal grant engine:
  per repository, per branch, for a time, read vs write, force pushes always asked) or, without a phone, by a **local
  policy** with desktop prompts;
- the phone is shown exactly what is pushed (branch, fast-forward or force, commits, files, +/− lines), worked out by
  the daemon from the bytes git sends, and the approval is bound to those bytes (digest);
- pack data flows computer ↔ GitHub directly; the phone only sees metadata and seals a credential to the daemon.

The daemon is the start of "bring your connections to any harness": later parts (a local MCP bridge, more git hosts)
plug into the same daemon. Out of scope now: SSH transport (an SSH agent cannot see what it authorizes; HTTPS can),
GitHub App token minting (designed for: the phone could mint a token narrowed to the repo; today it seals its stored
token), confidential computing.

## Pieces and owners

| Piece | Where | Owner |
|---|---|---|
| Contract: tool specs, wire types, fingerprint, digest | `crates/rewarden-proto/src/desktop.rs`, `connector/github/git.rs`, `pairing.rs` | done (lead) |
| Daemon skeleton: config, identity, HTTP client, pkt-lines, interfaces | `crates/rewarden-desktop/src/{lib,config,identity,http}.rs`, `git/pktline.rs`, `auth/mod.rs` | done (lead) |
| D1 push analysis | `crates/rewarden-desktop/src/git/{object,pack,remote,analyze}.rs` (+ new files under `git/`) | worker |
| D2 daemon runtime | `src/{main,daemon,control,setup,service,harden}.rs`, `src/proxy/*`, `src/auth/{local,policy,prompt}.rs`, `tests/` | worker |
| D3 Rewarden link | `src/server/*`, `src/auth/rewarden.rs` | worker |
| S server | `src/api/rewarden/*` (Vaultwarden fork) | worker |
| P phone core | `crates/rewarden-core` | worker |
| A Android | `android/` | worker |

Workers only edit their files. Shared files (`Cargo.toml`s, `lib.rs`, `git/mod.rs`) only to add a dependency or a
module line, and say so in the report. Lints are the workspace's: pedantic clippy, `unsafe_code = "forbid"`,
`warnings = "deny"`. Every piece is test-first.

## Contract (implemented, in `rewarden-proto`)

### Tools (desktop only)

Registered in the connector registry like every GitHub tool, but `ToolSpec.desktop_only = true`:

| tool | op | effect | class | params |
|---|---|---|---|---|
| `github_git_fetch` | `git_fetch` | Read | – | `repo`, `client_key`, `nonce` |
| `github_git_push` | `git_push` | Write | `code` | `repo`, `client_key`, `nonce`, `digest`, `summary` (JSON) |
| `github_git_tag_push` | `git_tag_push` | Write | `releases` | same as push |

Desktop-only tools are never in MCP `tools/list` and `parse_invocation` treats them as unknown (done, tested). They
are only accepted on the server's desktop API (below), and the phone only answers them for a connection whose pinned
key equals `client_key`.

`PushSummary::tool()` picks push vs tag push (tags only → tag push). `PushSummary::resource(repo)` is `repo@branch` for
one branch, else `repo`. `PushSummary::once_only()`: a force push (`fast_forward != Some(true)` on an update), a
deletion, a moved tag, several branches, or a ref that is neither branch nor tag → asked every time, never remembered.

### Keys, pairing, sealing

- The daemon has an X25519 key (`identity.key`, 0600). Public key: base64url, no padding, 32 bytes.
- `rewarden login` starts the server's OAuth authorize with an extra parameter `rewarden_client_key=<public key>`.
  The server carries it into `PairingRequest.client_key`. The phone shows `key_fingerprint(client_key)` ("4821 9930")
  next to the pairing; the terminal shows the same; the user compares. On approval the phone pins the key to the new
  connection id (the pairing response returns it).
- The phone answers a desktop tool with a `CredentialGrant` JSON, sealed (crypto_box sealed box, X25519 +
  XSalsa20-Poly1305) to the pinned key, base64url no padding:
  - fetch: one item, `items[0].sealed` (+ `items[0].expires_at`), grant `access = "read"`, `expires_at = now +
    FETCH_LEASE_SECS` (1 h), `digest = None`;
  - push: data `{"sealed": …}`, grant `access = "write"`, `expires_at = now + PUSH_LEASE_SECS` (10 min),
    `digest = Some(<the digest approved>)`.
  - `nonce` echoes the request's nonce; `repo` echoes the repo; `username = "x-access-token"`, `token` = the GitHub
    token of the account.
- The daemon refuses a grant whose nonce, repo, access or digest differ, or that has expired. So the server can neither
  read the token, nor substitute its own key (pinned at pairing, compared by the user), nor replay an old answer.
- `push_digest(repo, commands, sha256(pack), push_options)`: see `desktop.rs`.

## D1: push analysis (`git/`)

Input: the repo, the commands (`old new ref`), push options, the pack file (exactly the pack bytes, from `PACK`), and
a `Remote` for what GitHub already has. Output: `PushSummary` that passes `validate()`. Never fails: whatever cannot
be worked out goes to `notes` (≤ 10, ≤ 200 chars each) and is left out; unknown ancestry is `fast_forward: None`.

- `object.rs`: object ids (20-byte SHA-1, hex), hashing (`"<kind> <len>\0"`), commit parsing (tree, parents, author
  name/email/time, subject = first line, ≤ 200 chars), tree parsing (mode, name, id), tag objects (type of target).
- `pack.rs`: pack v2/v3 from a file: header, entries (type, size varint, zlib streams: find their compressed length by
  inflating), `OFS_DELTA` and `REF_DELTA`, delta application (copy/insert), thin packs (a `REF_DELTA` base outside the
  pack comes from `Remote::object`), hash check of every resolved object. Bounded memory: keep an index of entries,
  inflate on demand, cache resolved bases (LRU or size-bounded), refuse objects > 64 MiB for analysis (note it).
- `remote.rs`: `GitHubRemote` on the REST API with the credential:
  - raw blob: `GET /repos/{repo}/git/blobs/{sha}` with `Accept: application/vnd.github.raw`;
  - tree: `GET /repos/{repo}/git/trees/{sha}` (JSON) re-serialised to the raw tree format (mode without a leading zero
    for directories: `40000`), accepted only if its SHA-1 matches;
  - commit metadata: `GET /repos/{repo}/git/commits/{sha}` (tree, parents) — a raw commit cannot be rebuilt exactly, so
    commits are never delta bases from the remote (note it);
  - ancestry: `GET /repos/{repo}/compare/{ancestor}...{descendant}` → `status` `ahead` or `identical` ⇒ true, `behind` or
    `diverged` ⇒ false, 404 ⇒ unknown.
  The trait in `remote.rs` may be reshaped freely; keep `GitHubRemote::new(http, api_base, repo, credential)`.
  Also `NoRemote` (knows nothing). Requests: 15 s timeout each, at most ~200 per push (budget), `User-Agent`,
  `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`.
- `analyze.rs`, per command:
  - change: create / update / delete from zero ids;
  - commits: walk parents from `new` through commits in the pack (stop at commits not in the pack: those are on the
    server); `commit_count` = commits found (walk cap 10 000 → note "at least N"), `commits` = newest first, ≤ 50;
  - `fast_forward` (updates only): the walk reaches `old` ⇒ true; else ask `Remote::is_ancestor(old, b)` for each
    boundary commit `b` (parents outside the pack) — any true ⇒ true, all false ⇒ false, otherwise `None`. An update
    with no new objects (moving a branch to a commit GitHub has) ⇒ `is_ancestor(old, new)`;
  - files: diff the root trees of `old` and `new` (for a create: of the first-parent boundary commit and `new`; when
    there is none, 0 and a note), descending only into subtrees whose ids differ; trees from the pack, else the remote;
    status added / modified / deleted / type-changed (no rename detection); listed ≤ 300, counted ≤ 5 000;
  - lines: for text blobs ≤ 1 MiB on both sides, count added/removed lines with a line diff (any Myers / patience
    implementation; adding the `similar` crate is fine), total budget 32 MiB per push; binary = NUL byte in the first
    8 KiB; when not counted, `additions/deletions = None` on that file and the ref totals are `None` unless all counted;
  - tags: an annotated tag object in the pack → its target; a lightweight tag → the commit.
- Tests (in the module files and `tests/analyze.rs`): repos built with the real `git` CLI in temp dirs; packs made the
  way push makes them (`git pack-objects --stdout --thin --revs` fed `new` and `^old`); a test `Remote` backed by a
  bare repo (`git cat-file`, `git merge-base --is-ancestor`). Cases: fast-forward, force push (rebase), new branch,
  delete, tag create/move, OFS and thin REF deltas (big files edited slightly), binary file, deep paths, more than 50
  commits, an object missing everywhere (note, not a failure), corrupted pack (note), `GitHubRemote` against a local
  mock HTTP server (re-serialised tree hash check, compare statuses).

## D2: daemon runtime

CLI (`clap`), binary `rewarden`:

```
rewarden daemon                  run in the foreground (what the service runs)
rewarden status                  daemon running? mode, who decides, listen address, logged-in server, fingerprint
rewarden git setup [--repo DIR]  route github.com remotes through the proxy (global git config, or one repository)
rewarden git unsetup [--repo DIR]
rewarden pending                 local approvals waiting
rewarden approve <id> | deny <id>
rewarden login <server-url> [--no-browser]     (D3's server::oauth::login)
rewarden logout
rewarden service install | uninstall           systemd user unit / launchd agent
```

Daemon (`daemon.rs`): loads `Config` and `Paths`, `Identity::load_or_create`, picks the authorizer (`Mode::Auto`: the
phone when `server::oauth::logged_in_server` is some, else local; `Mode::Rewarden` / `Mode::Local` force one), writes a
random control token to `control.token` (0600), hardens the process (`harden.rs`: Linux
`rustix::process::set_dumpable_behavior(NotDumpable)`; elsewhere nothing), serves one hyper HTTP/1.1 listener on
`config.listen` (loopback only).

Every request: the `Host` header must be `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>` (DNS rebinding);
`OPTIONS` and anything with an `Origin` header is refused (browsers). Paths:

- `/_rewarden/...` control API (`control.rs`), all need `X-Rewarden-Token: <control token>`:
  `GET status`, `GET pending`, `POST pending/<id>/approve`, `POST pending/<id>/deny`. JSON.
- `/<host>/<owner>/<repo>[.git]/info/refs?service=git-upload-pack|git-receive-pack`,
  `/<host>/<owner>/<repo>[.git]/git-upload-pack`, `/…/git-receive-pack`, `/…/info/lfs/<rest>`: the proxy. `<host>`
  must equal `config.github.host`; owner and repo `[A-Za-z0-9._-]+`, no `..`.

Proxy (`proxy/`), upstream `config.github.git_base` + `/<owner>/<repo>.git/...` via `crate::http::client(None)`:

- upload-pack (`info/refs?service=git-upload-pack` and `POST git-upload-pack`): first without credentials (public
  repositories need no approval); on 401/403/404 ask `Authorizer::read` and retry with the credential; remember per repo
  that it needs one (skip the anonymous try next time). Forward `Git-Protocol`, `Content-Type`, `Content-Encoding`,
  `Accept`, `Accept-Encoding`, `User-Agent`; stream the request and the response bodies both ways (packs are big).
- `info/refs?service=git-receive-pack`: `Authorizer::read` (a push needs the ref list first), then forward.
- `POST git-receive-pack`: read the whole body (decode `Content-Encoding: gzip` for parsing, forward the original
  bytes), spill to a temp file above 8 MiB, refuse above 2 GiB. `parse_receive_head`; a body without commands (git's
  probe) is forwarded as is with the read credential. Else: pack = body from `pack_offset` (written to its own temp
  file for D1), `sha256(pack)`, `push_digest`, `analyze_push(repo, commands, options, pack, &GitHubRemote::new(..,
  read credential))`, `Authorizer::push(repo, &summary, &digest)`. Approved → forward the original body with the push
  credential and stream GitHub's answer back. Refused → answer git ourselves (`proxy/report.rs`), status 200,
  `Content-Type: application/x-git-receive-pack-result`: `unpack ok`, `ng <ref> <reason>` for every command (on band 1
  when the client asked `side-band-64k`/`side-band`, with the human message also on band 2 so git prints
  `remote: …`), flush. Tested with the real git client: it must print `! [remote rejected] main -> main (…)`.
- Other refusals (read denied, waiting, unavailable): HTTP 403 `text/plain` with the message (git prints it as
  `remote: …`). Waiting messages say to approve on the phone and run git again.
- Redirects from GitHub (renamed repositories): follow up to 3 same-host redirects internally; never pass a
  `Location` to git (it would bypass the proxy).
- LFS (`/info/lfs/objects/batch`, `/info/lfs/locks…`): forward with the read credential.
- Logging: method, repo, decision, sizes. Never a token, never an `Authorization` header.

Local authorizer (`auth/local.rs`, `policy.rs`, `prompt.rs`):

- Token source `config.github.token`: `gh` (runs `gh auth token`, cached 10 min), `env:NAME`, `file:PATH` (first line).
  Credential `username = "x-access-token"`, `expires_at = now + 600`.
- Policy: the first `[[policy.rules]]` entry whose `repo` pattern (and `branch` pattern, if set, matched against the
  pushed branch; a push of several refs must match for every ref) matches decides `read` / `push` / `risky`, falling
  back to the top-level defaults (`read = allow`, `push = ask`, `risky = ask`). A push is risky when
  `PushSummary::once_only()` (or any update `is_risky()`); risky uses the `risky` rule, never `push`. Patterns: `*` within
  a part, `**` across parts.
- Ask: add to the pending list (id, what, summary lines), show a desktop prompt — Linux `notify-send` with actions
  (`--action=approve=Approve --action=deny=Deny --wait`), macOS `osascript` `display dialog` with Approve/Deny buttons,
  else nothing — and wait for the prompt, `rewarden approve/deny`, or `approval_timeout_secs` (→ `Refusal::Waiting`;
  a retry of the same push digest within 10 minutes finds the earlier answer). Prompts never block the listener.
- Honest limits (in the README/docs): in local mode another process of the same user could approve through the control
  API or read the token source; the local mode protects against an agent's mistakes, the phone mode against a hostile
  agent (plus run the daemon as its own OS user for full separation).

`setup.rs`: `git config [--global | -C DIR --local] --add url.<proxy_base>.insteadOf <x>` for `https://github.com/`,
`git@github.com:`, `ssh://git@github.com/`; idempotent; `unsetup` removes exactly those. `service.rs`: systemd user unit
`~/.config/systemd/user/rewarden.service` (`ExecStart=<current exe> daemon`, `Restart=on-failure`) + `systemctl --user
enable --now`; macOS `~/Library/LaunchAgents/dev.rewarden.daemon.plist` + `launchctl load -w`.

Tests (`crates/rewarden-desktop/tests/`): an upstream made of `git http-backend` (CGI, `GIT_PROJECT_ROOT`,
`GIT_HTTP_EXPORT_ALL`, `http.receivepack=true`) behind a small hyper server in the test, plus a fake GitHub REST API
answering from the same bare repository; a fake `Authorizer` (scripted answers) and the real local one. Real `git
clone`, `fetch`, `push` through the daemon: public clone without asking, private (upstream requires a basic auth token)
asks and passes the token, push approved goes through, push denied prints `remote rejected`, probe/large chunked push,
gzip bodies, Host/Origin checks, control API token, setup/unsetup on a temp `HOME`.

## D3: Rewarden link (`server/`, `auth/rewarden.rs`)

- `server::oauth::login(paths, identity, server_url, open_browser)`:
  1. `GET <server>/.well-known/oauth-authorization-server` (endpoints);
  2. dynamic registration `POST registration_endpoint` `{"client_name": "Rewarden desktop app on <hostname>",
     "redirect_uris": ["http://127.0.0.1/callback"], "grant_types": ["authorization_code","refresh_token"],
     "response_types": ["code"], "token_endpoint_auth_method": "none"}` (loopback ports may differ, RFC 8252);
  3. listen on `127.0.0.1:0`, PKCE S256, random `state`, authorize URL with `response_type=code`, `client_id`,
     `redirect_uri=http://127.0.0.1:<port>/callback`, `code_challenge`, `code_challenge_method=S256`, `state`,
     `resource=<server>/mcp`, `rewarden_client_key=<identity public key>`;
  4. print the URL and "Your phone will show the key 4821 9930. Approve only if it matches."; open the browser
     (`xdg-open` / `open` / `start`) unless `--no-browser`;
  5. accept the callback (check `state`, show a small "You can close this tab" page), exchange the code
     (`grant_type=authorization_code`, `code`, `redirect_uri`, `client_id`, `code_verifier`, `resource`);
  6. save `session.json` (0600): server, client id, access token + expiry, refresh token. Refresh when expired
     (`grant_type=refresh_token`, keep the new refresh token if one is returned); a refresh failure means logged out.
- `server::client`: `POST <server>/rewarden/desktop/calls` and `GET <server>/rewarden/desktop/calls/<id>` (contract in S).
- `RewardenAuthorizer` (`auth/rewarden.rs`):
  - `read(repo)`: cached live read credential for the repo → use it. Else call `github_git_fetch {repo, client_key,
    nonce}` (+ `account` from config) and poll until answered or `approval_timeout_secs`; open `items[0].sealed`, check
    nonce, repo, `access == "read"`, not expired; cache until `expires_at`.
  - `push(repo, summary, digest)`: `summary.tool()` with `{repo, client_key, nonce, digest, summary}`; poll; open
    `sealed`, check nonce, repo, `access == "write"`, `digest`, not expired. Never cached for other pushes.
  - Outcomes: denied → `Refusal::Denied(reason or "Denied on your phone.")`; error → `Unavailable(message)`;
    offline/pending past the timeout → `Waiting("Waiting for approval on your phone. Approve it, then run git push
    again.")`. Retries: a `(tool, repo, digest)` (or `(fetch, repo)`) asked less than 10 minutes ago reuses the earlier
    request id (poll it) instead of asking again.
- Tests: a mock server (hyper, in the test) implementing discovery, registration, token, and the desktop calls API,
  whose "phone" seals grants with `identity::seal_to`; wrong nonce / digest / key / expired grants are refused; login
  via a scripted browser (GET the authorize URL, follow to the loopback callback).

## S: server (Vaultwarden fork)

- `authorize_get`: optional `rewarden_client_key`; must pass `desktop::decode_key`; stored in `PairingClient`
  (`client_key: Option<String>`) and sent in `PairingRequest.client_key` (decoys too). The email and wait pages show
  "Desktop app key 4821 9930" when present.
- Desktop API, bearer = the same OAuth access token as MCP (same validation as `mcp_routes::authenticate`):
  - `POST /rewarden/desktop/calls` body `{"tool": "<desktop tool>", "arguments": {...}, "account": null|"..."}`,
    ≤ 512 KiB. Only `desktop_only` specs (else 400 `{"error":"unknown_tool"}`); arguments validated with
    `ToolSpec::parse` (400 `{"error":"invalid_arguments","message":…}`). Submits to the relay like an MCP tool call
    (same FCM push), waits like MCP (`HUB.relay.wait`), and answers 200
    `{"request_id": "...", "status": "answered", "outcome": <RelayOutcome JSON>}` or `{"request_id","status":"pending"}`
    or `{"request_id","status":"offline"}`.
  - `GET /rewarden/desktop/calls/<request_id>`: same answer shape after waiting again; 404 `{"error":"not_found"}`
    for unknown, expired, or another connection's request.
  - 401 with `WWW-Authenticate` (as MCP) for a missing or bad token; rate limited like MCP.
- Tests: unit tests for parsing; integration tests in `tests/rewarden_server` (desktop call relayed to a fake phone and
  answered; MCP cannot list or call desktop tools; client key reaches the pairing request; bad keys refused).
- `docs/deployment.md`: a short section on the desktop app.

## P: phone core (`rewarden-core`)

- Store: table `desktop_keys(connection_id TEXT PRIMARY KEY, public_key TEXT NOT NULL, pinned_at INTEGER NOT NULL)`
  (migration). `answer_pairing`: when approved and the server returned a connection id and the pairing had a valid
  `client_key`, pin it. Removing a connection (and logging out) drops its key.
- `PairingView.key_fingerprint: Option<String>` (from `client_key`); the pending item title for a desktop client stays
  generic. Pairings with an invalid `client_key` show no fingerprint and pin nothing.
- Flow (`connector/flow.rs`): for a `desktop_only` spec, before anything else, the connection must have a pinned key
  equal to `client_key`, else answer an error ("This must come from the Rewarden desktop app paired with this
  phone."). Account choice for git: the one named, or the only one, or (several) the first whose token can see the
  repository (`GET /repos/{repo}`).
- GitHub connector, new area `connector/github/git.rs`:
  - `git_fetch` (Read): `GET /repos/{repo}` must succeed (else the usual errors); one `Item`: id = resource = `repo`,
    label = repo (+ "private" when it is), title "Clone and fetch <repo>", snippet "Git on your computer can read this
    repository for 1 hour.", parents `[(owner, "Every repository of <owner>")]`, not sensitive, `extra = {"sealed",
    "expires_at"}` with the read grant sealed to `client_key`. Read grants on the repo or owner cover it.
  - `git_push` / `git_tag_push` (Write): `summary` parsed as `PushSummary` and validated, `summary.tool()` must equal
    the call's tool, refs must be branches for push (tags allowed alongside) / tags only for tag push. Preview:
    `resource = summary.resource(repo)`, label "<repo>, branch <b>" or "<repo>", parents (`repo@b` → `[(repo, "Any
    branch of <repo>"), (owner, "Every repository of <owner>")]`; `repo` → `[(owner, …)]`), `once_only =
    summary.once_only()`, lines: one per ref ("Push 3 commits to main", "Create branch x with 2 commits", "Delete branch
    x", "Force push to main: rewrites history", "Tag v1.2 → 1a2b3c4"), then up to 5 commit subjects and the "+A −D in N
    files" line. Perform: seal the write grant with the digest; data `{"sealed": …}`.
  - Sealing with `crypto_box::PublicKey::seal` (the crate is already a dependency), base64url no padding.
- `ApprovalView.git: Option<GitPushView>` (UniFFI record) built from the parked call's summary:
  `GitPushView { repo, pack_bytes: u64, notes: Vec<String>, refs: Vec<GitRefView> }`,
  `GitRefView { name, kind: String ("branch"|"tag"|"other"), short_name, change: String ("create"|"update"|"delete"),
  force: bool (update and fast_forward != Some(true)), force_unknown: bool (fast_forward None on an update),
  commit_count: u32, commits: Vec<GitCommitView>, files_changed: u32, files: Vec<GitFileView>, additions: Option<u64>,
  deletions: Option<u64> }`, `GitCommitView { short_sha, subject, author }`, `GitFileView { path, status: String
  ("added"|"modified"|"deleted"|"type_changed"), additions: Option<u32>, deletions: Option<u32>, binary: bool }`.
  Fetch requests have `git = None` (the item list shows the repo).
- `op_title` gets "Clone and fetch with git", "Push with git", "Push tags with git" (from the spec titles).
- Tests: pinning (approve pins, deny does not, bad key ignored, removal drops), refused when key missing or different,
  fetch covered by a read grant / parked and released, push parked with lines and git view, force push once-only, the
  sealed answers open with the matching secret key and carry nonce/digest/expiry, tag push validation, account choice.

## A: Android

- Pairing screen: when `keyFingerprint != null`, a card "Desktop app key" with the digits large and "Check that your
  computer shows the same numbers. If they differ, deny." 
- Approval sheet: when `approvalView.git != null`, a git section instead of the plain lines: repo; per ref the branch
  or tag name, a chip (New branch / Update / Delete / Tag), a red "Rewrites history (force push)" or amber "Could not
  check history" warning, commit list (short sha in mono, subject, author; "and N more"), "+A −D in N files" and the
  file list (status letter colored, path, +/−; "and N more"), notes, pack size. Once-only behaviour already exists.
- Activity and pending titles come from `opTitle`.
- Tests (Robolectric/Compose): the pairing fingerprint card, a push with commits and files, a force push warning, a
  tag push; `FakeCore`/`TestData` updated for the new fields; screenshots updated.

## Lead: integration

After the workers merge: e2e scenario in `crates/rewarden-e2e` (real server + real phone core + daemon + `git
http-backend` upstream + fake GitHub API): login with the phone approving the pairing, clone a private repo (phone
approves read), push a branch (phone approves, grant remembered for the branch), force push asked every time, denied
push rejected in git. Then deploy the server, build the APK, publish the `rewarden` binary next to the APK, push.
