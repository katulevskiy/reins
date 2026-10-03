# GitHub and vault tools: conventions every area follows

Read this fully before writing code. It is the contract between the parallel workers; deviations must be reported.

## 1. How a call travels (what already exists — do not change it)

* An AI calls an MCP tool (`github_issue_create`). The server validates the arguments against the **tool registry**
  (`crates/reins-proto/src/connector.rs` + `connector/github/*.rs`, `connector/vault/*.rs`) and relays a
  `ConnectorCall {service, op, args}` to the phone.
* On the phone (`crates/reins-core/src/connector/flow.rs`, do not edit) a **read** (`Effect::List|Read|Search`) calls
  the connector's `fetch()` → `Vec<Item>`; the user ticks items (or a standing grant covers them) and the ticked
  items are returned to the AI as JSON (`items_json`). A **write** (`Effect::Write`) calls `preview()` → `Preview` (what
  will happen, shown to the user), then, after approval (or a covering standing grant), `perform()` → `Value` (the answer
  the AI gets). `perform()` is called only after approval; `preview()` and `fetch()` must change nothing.
* Connectors: `connector/github/mod.rs` (`GitHub`) and `connector/vault/mod.rs` (`Vault`) dispatch every call to the
  **areas** in order (`repos`, `code`, `issues`, `actions` for GitHub; `items`, `sends` for the vault). An area function
  returns `None` when the op is not its own. **You only edit your own area's files** (listed in §6). If a shared helper
  is missing, put a private copy in your area and mention it in your report; never edit `mod.rs`, `flow.rs`, `Cargo.toml`
  or another area's file.

## 2. Naming

* Tool name `github_<noun>_<verb>` (e.g. `github_issue_create`, `github_release_asset_upload`), op = the same without
  the `github_` prefix (`issue_create`). Vault: `vault_<noun>_<verb>`, op without prefix. The six legacy GitHub tools
  (`github_list_repos`, `github_list_issues`, `github_get_issue`, `github_search`, `github_comment`,
  `github_create_issue`) and the two legacy vault tools (`vault_search`, `vault_get`) keep their names and behaviour
  (you may add optional params); existing tests must keep passing.
* `title` is a short human phrase ("Create a GitHub release"); `description` tells the AI what it does, the
  arguments' meaning, limits and that the user approves on their phone. Every param has a description.
* Keep tool descriptions truthful about approval: reads show what was found and the user ticks; writes show a preview.

## 3. Registry model (proto)

In `reins-proto` `connector/<service>/<area>.rs` expose `pub(super) fn tools() -> Vec<ToolSpec>` using the helpers
from `crate::connector`: `tool(...)`, `str_p` (short trimmed text), `text_p` (free text kept exactly: bodies, file
contents), `int_p`, `bool_p`, `list_p`, `choice_p`, `map_p` (string→string), `json_p` (arbitrary JSON you validate yourself,
bounded), `LIMIT`. Builder methods on a `ToolSpec`: `.in_class("code")` (the kind of change; **required for every
Write**, see §4) and `.once()` (asked every time; see §4). `resource_param`: the name of the argument that identifies
the thing (`Some("repo")`); it is only used for labelling.

* Every Write tool has a class (`.in_class`). Reads have none.
* `Effect::List` = names only (repos, workflows, labels, folders), `Read` = content, `Search` = looks for content,
  `Write` = changes anything or sends anything out (including *creating* things and *running* things).
* Limits: keep strings bounded (`str_p` max 100–300; bodies `text_p` ≤ 65_000 unless a file; file/asset content
  arrives as base64 in a `text_p` with max ≤ 3_000_000 characters, decoded ≤ 2 MB — say so in the description).
  Booleans are optional and omitted means "leave unchanged" for updates (never default a destructive flag to true).
* Update tools change only the fields given; require at least one field (validate in `preview()`).
* Tool names are unique across the registry (a test enforces it).

## 4. Permissions model (the point of this work)

A **standing permission** (grant) is bound to one AI connection, one account, a time window, a use count, and:
* an **access** — `list`, `read` or `write` (never mixed);
* **resources** — what it covers, matched hierarchically (`reins_policy::resource_covers`): `A` covers `A`, `A@x`, `A/x`;
  a name that is a prefix but not a whole part covers nothing;
* for writes, **classes** — the kinds of change it allows (`ToolSpec.class`).

**Resources** (the string an `Item.resource` / `Preview.resource` carries, plus `parents` so the user can widen it):
* GitHub: `owner/repo` for anything about a repository as a whole (issues, releases, settings, workflow runs);
  `owner/repo@branch` when the operation is about **one branch** (reading/writing files, commits, pushing, merging into a
  branch, creating/deleting a branch, reading contents at a ref that names a branch). `parents` = `[("owner/repo", "Any branch of owner/repo"), ("owner", "Every repository of owner")]`
  (use `github::parents(repo, branch)`). A ref that is a tag or a commit SHA is not a branch: use `owner/repo`.
  Account-level things (notifications, stars, gists, the user's own profile, org listings): resource `account`
  with label "Your GitHub account" (no parents).
* Vault: `{folder}/{type}/{item}` where `folder` is the folder id or `none`, `type` is `login|note|card|identity|ssh_key`,
  `item` is the item id; creating in a folder uses `{folder}/{type}`; parents = `[("{folder}/{type}", "<Type plural> in <folder name>"), ("{folder}", "<folder name>")]`
  (folder name `No folder` for `none`). Folder operations: resource `{folder}`. Sends: `sends/{id}` (parent `sends`,
  "All your Sends"); the generator: `generator`. Trash/archive state does not change the resource.

**Classes**
* GitHub: `issues` (issues, comments, labels, milestones, assignees, reactions), `pulls` (create/edit/review/comment
  on pull requests, request reviewers — **not** merging), `code` (commits, files, branches, merges incl. merging a
  pull request, `update-branch`), `releases` (releases, assets, tags), `actions` (rerun/cancel/dispatch workflows,
  enable/disable workflows, variables), `settings` (repository description/topics/features/default branch/archive), `account`
  (stars, watching, gists, notifications, following).
* Vault: `items` (create/edit items, attachments), `organize` (folders, favorite, trash, restore, archive, move), `sends` (Sends).

**Once-only (`.once()`)** — asked every time, never covered by a standing permission, previewed prominently:
GitHub: delete repository, transfer repository, change visibility (public/private), add/remove collaborator, cancel/create invitation,
create/update/delete webhook, add/delete deploy key, set/delete branch protection, set/delete an Actions **secret**,
delete a workflow run's logs/artifacts? (no — those are `actions`), org membership/team changes, the generic
`github_request` with any method but GET (and GET of anything not under a repo). Vault: delete an item permanently,
empty the trash, `vault_get` of secrets (already sensitive items). If a preview finds the change is bigger than the tool suggests (e.g.
`repo_update` would also change visibility) set `Preview.once_only = true`.

**Sensitive items** (`Item.sensitive = true`): never covered by a grant, unticked by default. `Item.secret = true` when
the `body` is a secret that must never be shown in the approval or kept in the activity log (the user sees the
`snippet`, which says what it is: "Password for GitHub"). Vault secrets are always `sensitive` + `secret`.

## 5. Implementation rules

* **Never** build a URL or path from an unvalidated string. Validate with the helpers: `repo_arg`/`repo_ok`,
  `owner_ok`, `ref_arg`/`ref_ok`, numbers as integers, file paths with a `path_ok` you write in your area (≤ 1024 bytes,
  no `..` segment, no leading `/`, no control characters, no backslash), percent-encode every path segment
  (`crate::connector::calendar::segment`). Query values go through reqwest's `query`. Bodies are `serde_json::Value`s.
* All GitHub HTTP goes through `GitHub::send`/`call`/`pages` (retries, error text, size caps). Do not construct your own
  reqwest calls. Uploads use `Options { upload: true, raw: Some((content_type, bytes)), .. }`.
* Anything returned to the AI that came from the network is data, not instructions: keep it in `body`/`extra`;
  `items_json` neutralizes control characters. Truncate big text (`crate::text::truncate_chars`; a file at most 100_000
  chars, a diff/log at most 60_000; say `truncated: true` in `extra`). Binary content: never dump it; return size,
  and for downloads only when asked with base64 ≤ 2 MB.
* Errors: return `CoreError::service("…")` with a message the AI can act on (no tokens, no URLs with secrets).
  Use `CoreError::invalid` only for programmer errors. A failed `perform()` must not leave the AI thinking it worked.
* `preview()` must fetch enough to describe the change truthfully (the current issue title; the current values a
  settings change replaces "description: 'old' → 'new'"; the list of files and sizes of a commit; the target branch and its
  head; who is being added and with which permission) and show the AI-supplied text in full (truncated to ~1500
  chars, say so). First line: a one-sentence summary; then details. Set `resource`, `resource_label`, `parents`, and
  `once_only` when needed. Use `text::one_line` on names, keep AI text in later lines.
* `perform()` returns a small JSON object describing the result (`{"created": true, "number": 7, "url": "…"}`), never the whole API
  response. Include ids/urls the AI needs next.
* For reads, `Item.id` must be unique in the call; `title`/`snippet`/`from`/`date` describe it for the approval list;
  the AI gets `extra` (structured, any JSON) and, for `Effect::Read`, `body` as `text`. Put structured data in `extra`.
  Set `resource`/`resource_label`/`parents` per §4.
* Do not add dependencies (already available: `serde_json`, `reqwest`, `ring`, `data-encoding`, `url`, `zeroize`, `aes`+`cbc`, `crypto_box`
  with `seal`, `chrono`-free date helpers in `crate::text` (`parse_when`, `iso_utc`)). If you truly need one, stop and report.
* Comments: like the surrounding code — short doc comments on public items and non-obvious decisions, no narration.
* Lints are strict (`-D warnings`, pedantic clippy). `cargo fmt --all`, then
  `cargo clippy -p reins-proto -p reins-core --all-targets` must be clean, `cargo test -p reins-proto -p reins-core` green.

## 6. Areas

### GitHub

**`repos`** — files: `crates/reins-proto/src/connector/github/repos.rs`, `crates/reins-core/src/connector/github/repos.rs`, tests `crates/reins-core/tests/github_repos.rs`.
Repositories, branches, and repository administration:
* repos: list (existing `list_repos`, add optional `owner`/`org` filter and `visibility`), get (details: description, topics, default branch, visibility, size, languages,
  counts, license, parent), create (user or org; name, description, private, auto_init, gitignore, license; class `settings`), update settings
  (description, homepage, topics, default branch, has_issues/projects/wiki, allow merge/squash/rebase, delete branch on merge, archived; class `settings`; **visibility is a separate once-only tool**),
  fork (class `settings`, to org optionally), list forks, delete (once), transfer (once), set visibility (once), languages, contributors, topics get/set, README get (rendered text),
  list org repos, list user's orgs' repos.
* branches: list, get (head sha, protected), create from sha/branch (class `code`, resource `owner/repo@newbranch`), delete (class `code`), rename (class `code`), merge one branch into another (`merge` API; class `code`, resource is the **base** branch; preview shows the head commits), get/set/delete branch protection (get = Read; set/delete once-only), list rulesets (read).
* access & integrations (all once-only writes, reads normal): collaborators list/add(permission)/remove, invitations list/cancel, repo teams list/add/remove, webhooks list/get/create/update/delete/ping, deploy keys list/add/delete.
* Traffic/stats: clones/views (Read), commit activity (Read) — optional.

**`code`** — files: `crates/reins-proto/src/connector/github/code.rs`, `crates/reins-core/src/connector/github/code.rs`, tests `crates/reins-core/tests/github_code.rs`.
Files, commits, tags, releases:
* contents: get file (text; base64 for binary ≤ 2 MB; `ref` optional; resource `owner/repo@ref` when the ref is a branch) , list directory, tree (recursive, truncated), raw blob, download archive link (return the URL only), search code in a repo is in `issues` area (search) — not yours.
* commits: list (path/author/since/until/branch filters), get (message, stats, files with patch snippets ≤ 60k chars), compare two refs (ahead/behind, files, commits), get commit statuses/check runs summary for a ref (Read; belongs to you).
* writing code (class `code`, resource `owner/repo@branch`, parents `parents(repo, Some(branch))`): `file_put` (create or update one file; needs `sha` for updates — fetch it yourself in preview/perform when not given), `file_delete`, `commit_files` (multi-file commit using the Git Data API: blobs → tree → commit → update ref; each file `{path, content | content_base64 | delete:true, mode?}` in a `json_p`; optionally create the branch from `base_branch` if it does not exist; `force` is **not** offered), `revert`? (skip). Preview lists every path, action, byte size, and the first ~300 chars of each text change; says which branch head it advances and whether the branch is the default branch ("pushes to the default branch main").
  Author/committer optional (`author_name`, `author_email`); otherwise GitHub uses the token's user.
* tags: list, create lightweight or annotated (class `releases`, resource `owner/repo`), delete tag (class `releases`), get tag.
* releases: list, get, latest, get by tag, create (tag_name, target_commitish, name, body, draft, prerelease, generate_release_notes, make_latest; class `releases`), update (class `releases`), delete (class `releases`), generate release notes (Read-like: it calls POST generate-notes but changes nothing → Effect::Read), list assets, upload asset (`content_base64`, `name`, `label`, `content_type`; class `releases`; uses the upload host), update asset name/label, delete asset, download asset (Read; text or base64 ≤ 2 MB else metadata only).

**`issues`** — files: `crates/reins-proto/src/connector/github/issues.rs`, `crates/reins-core/src/connector/github/issues.rs`, tests `crates/reins-core/tests/github_issues.rs`. (Its file already holds the six legacy tools.)
Issues, pull requests, reviews, search:
* issues: list (assignee/label/milestone/creator/sort filters; legacy `list_issues` gets these as optional params), get (legacy `get_issue`), create (labels, assignees, milestone; legacy `create_issue` gets these optional params), update (title, body, state open/closed with reason, labels replace, assignees replace, milestone; class `issues`), lock/unlock, comments list/edit/delete (legacy `comment` creates), reactions add/list, timeline/events (Read), labels: list, create, update, delete, add to issue, remove from issue; milestones: list, create, update, delete; assignees: list assignable, add, remove; transfer issue (once-only).
* pull requests: list, get (with mergeability, checks summary), create (head, base, title, body, draft, maintainer_can_modify; class `pulls`; resource `owner/repo` — the *base* branch matters only for merging), update (title/body/base/state; class `pulls`), files changed (list with patch ≤ 60k), diff (text), commits, review list, review create (approve/request_changes/comment + inline comments; class `pulls`), review dismiss, review comments list/create/edit/delete, request/remove reviewers, mark ready for review (REST `POST /pulls/{n}/ready_for_review` does not exist — use GraphQL `markPullRequestReadyForReview` via `POST /graphql`; convert to draft the same), update branch (class `code`), **merge** (`merge_method` merge|squash|rebase, commit title/message, sha guard; class `code`, resource `owner/repo@<base branch>` with `parents`, preview shows title, head→base, checks state, mergeable state; if the base is the repo's default branch say so), close/reopen via update. Auto-merge enable/disable via GraphQL is optional.
* search: `github_search` (legacy: issues & PRs), plus `search_code`, `search_repos`, `search_commits`, `search_users`, `search_topics`, `search_labels` (Read/Search; results are items; code-search results carry path/repo/snippet, resource `owner/repo`).
* discussions (GraphQL read): list, get — optional.

**`actions`** — files: `crates/reins-proto/src/connector/github/actions.rs`, `crates/reins-core/src/connector/github/actions.rs`, tests `crates/reins-core/tests/github_actions.rs`.
Workflows, runs, variables, secrets, security, and the user's own account:
* workflows: list, get, dispatch (`workflow_dispatch` with `ref` and `inputs` map; class `actions`), enable, disable (class `actions`), usage/timing (Read), **workflow file editing is done through the `code` area's `file_put`** (mention that in the descriptions).
* runs: list (filters: workflow, branch, event, status, actor), get, rerun (whole run, with `enable_debug_logging`), rerun failed jobs, rerun a single job, cancel, force-cancel (once), delete run (class `actions`), approve a run from a fork PR, list jobs of a run, get job, **job logs** (text ≤ 60k chars, tail-biased; follows the redirect), run logs download (return the tail of each job's log, no zip), list artifacts, delete artifact, artifact download URL, list run attempts, pending deployments review (approve/reject; once-only).
* variables (repo and environment): list, get, create, update, delete (class `actions`); secrets: list names (Read), create/update (sealed-box encryption with the repo public key using `crypto_box::PublicKey::seal`; **once-only**; never echo the value), delete (once).
* environments: list, get, create/update (once), delete (once); caches list/delete (class `actions`).
* checks & statuses live in `code`; **security alerts**: Dependabot alerts list/get/update (dismiss), code-scanning alerts list/get/update, secret-scanning alerts list/get (**sensitive + secret**: they contain secrets)/update, security advisories list, dependency graph SBOM (Read, truncated).
* account (class `account`, resource `account`): authenticated user, get user by name, orgs list, org get, org repos/members/teams (Read), starred list, star/unstar, watch/unwatch (subscription), follow/unfollow, notifications list/mark read (thread/all), gists list/get/create/update/delete/star, user emails? (no), SSH/GPG keys (list names only — Read), rate limit (Read).
* `github_request` (generic escape hatch, the last resort): `method`, `path` (must start with `/repos/`, `/user`, `/users/`, `/orgs/`, `/search/`, `/gists`, `/notifications`, `/rate_limit`, `/graphql` is not allowed; no `..`, no `?` — use `query` map), `query` map, `body` json. GET → Read (resource `owner/repo` parsed from the path, else `account`); any other method → a Write with class `settings`, **once-only**, preview shows method+path+body. Refuse paths under `/authorizations`, `/applications`, `/user/keys`, `/user/gpg_keys`, `/user/emails`, `/user/tokens`, `/app`. The tool is registered as **two tools**: `github_request_read` (Effect::Read) and `github_request_write` (Effect::Write, once).

### Vault

**`items`** — files: `crates/reins-proto/src/connector/vault/items.rs`, `crates/reins-core/src/connector/vault/items.rs` (+ you may edit `vault/totp.rs`), tests `crates/reins-core/tests/vault_items.rs` (the existing `tests/vault.rs` must keep passing; you may extend its mock helpers by copying them into your test file).
Everything about items, folders and their states:
* Types (Bitwarden cipher `type`): 1 login, 2 secure note, 3 card, 4 identity, 5 SSH key. Handle per-item keys (`cipher.key`) when reading and writing (new items: write with the user key directly is fine; **editing an item that has its own key must keep using that key**). Skip organization items (`organizationId != null`) in listings, say so in the count note.
* `vault_search` (legacy; keep behaviour; add optional `type`, `folder`, `state` = active|trash|archived|all (default active), `favorite`; results Items with the §4 resource path, title = name, from = username/holder/identity name, snippet = first host or type label; never any secret), `vault_folders_list` (List: folder id, name, item counts by type), `vault_item_view` (Read: non-secret overview of one item: type, name, folder, favorite, state, dates, URIs, username, notes length only, card brand + last 4 + expiry month/year, identity name/email/company/city/country (no ID numbers), SSH public key + fingerprint, custom field **names** and types, attachment names/sizes, password-history count; **sensitive false**), `vault_get` (legacy; extend `field` with: `notes`, `uri`, `card_number`, `card_code`, `card_holder`, `card_expiry`, `identity` (all identity fields as one text block), `identity_ssn`, `identity_passport`, `identity_license`, `ssh_private_key`, `ssh_public_key`, `ssh_fingerprint`, `custom` (with new optional param `custom_field` naming the field), `password_history`; Items are `sensitive` + `secret` except `uri`, `ssh_public_key`, `ssh_fingerprint`, `card_holder`, `card_expiry` which are ordinary reads with the item's resource), `vault_attachment_get` (Read; sensitive + secret; base64 ≤ 2 MB, else metadata + "too large").
* Writes: `vault_item_create` (class `items`; `type` + `name` + `notes` + `favorite` + `folder` + type-specific fields — use one `json_p` named `fields` documented in the description per type: login {username, password, totp, uris[]}, note {}, card {holder, brand, number, exp_month, exp_year, code}, identity {title, first_name, … full Bitwarden identity set}, ssh_key {private_key, public_key, fingerprint}; plus `custom_fields` json list of {name, value, type: text|hidden|boolean}; every string encrypted with the user key; the preview shows every non-secret field and, for secrets, only "password set (N characters)" — never the secret text), `vault_item_update` (class `items`; same `fields` semantics, only given fields change; preview shows old → new for non-secrets and "password changes" for secrets), `vault_attachment_add` (class `items`; `content_base64`, `file_name` ≤ 2 MB; the file key encryption per Bitwarden attachments v2), `vault_attachment_delete` (class `items`), `vault_item_favorite` (organize), `vault_item_move` (organize; to folder or none), `vault_item_trash` (organize; soft delete), `vault_item_restore` (organize), `vault_item_archive` / `vault_item_unarchive` (organize; `PUT /api/ciphers/{id}/archive` and `/unarchive`), `vault_item_delete` (**once-only**; permanent delete of one trashed-or-not item), `vault_trash_empty` (**once-only**), `vault_folder_create`, `vault_folder_rename`, `vault_folder_delete` (organize; items stay, unfiled), `vault_item_clone` (class `items`; new item from an existing one, optionally a new name; secrets are copied on the phone and never shown).
* Bulk: `vault_item_trash`/`restore`/`favorite`/`move` accept one `item`; do not add bulk tools.
* Never put a secret into `Preview.lines`, `Item.snippet`, `Item.title`, error messages, or logs. Secrets appear only in `Item.body` (with `secret: true`).

**`sends`** — files: `crates/reins-proto/src/connector/vault/sends.rs`, `crates/reins-core/src/connector/vault/sends.rs`, tests `crates/reins-core/tests/vault_sends.rs`.
Sends and the generator:
* Sends: `vault_send_list` (List: name, type, access count / max, expiration, deletion date, disabled, has-password, hide-email; no links, no content), `vault_send_get` (Read; sensitive + secret: text content or file metadata and the **link** `https://<server>/#/send/<accessId>/<urlsafe-base64 key>`), `vault_send_create` (class `sends`; `type` text|file; `name`, `notes`, `text` (+ `hidden`) or `file_name` + `content_base64` (≤ 2 MB), `password`, `max_access_count`, `expires_in_hours`/`expiration_date`, `delete_in_days` (1–31) / `deletion_date`, `disabled`, `hide_email`; encrypt per the Bitwarden Send format: a random 16-byte send key, `HKDF-SHA256(key, salt = "bitwarden-send", info = "send", 64 bytes)` → enc/mac keys; name/notes/text/fileName encrypted with the derived key; the `key` field = the send key encrypted with the user key; password = `base64(PBKDF2-HMAC-SHA256(password, salt = send key, 100_000 iterations, 32))`; files: encrypted with the derived key as an EncArrayBuffer (`0x02 ‖ iv ‖ mac ‖ ciphertext`), upload with the v2 flow — `POST /api/sends/file/v2` then `POST /api/sends/{id}/file/{fileId}` multipart whose `data` part filename equals the encrypted fileName EncString; the API returns the accessId; return the link once — and the preview never prints the password or the text beyond ~300 chars), `vault_send_update` (class `sends`; name, notes, text, max count, dates, disabled, hide-email, new password; the type and file cannot change), `vault_send_remove_password` (class `sends`), `vault_send_delete` (class `sends`). Vaultwarden limits: deletion date < 31 days ahead.
* Generator (Effect::Read, resource `generator`, label "Generator", `sensitive: false`, `secret: true` so the value is not kept in the log; no network): `vault_generate_password` (length 5–128, `uppercase`/`lowercase`/`numbers`/`symbols` booleans, `min_uppercase` etc., `avoid_ambiguous`, `exclude` characters; uses `ring::rand` with rejection sampling — no modulo bias — and guarantees the minimums), `vault_generate_passphrase` (words 3–20, separator, capitalize, include_number; the EFF large wordlist (7776 words) embedded with `include_str!` in a new file `crates/reins-core/src/connector/vault/eff_large_wordlist.txt`, fetched from https://www.eff.org/files/2016/07/18/eff_large_wordlist.txt and verified: 7776 lines, `dice<TAB>word`), `vault_generate_username` (`type`: random_word|plus_addressed_email|catch_all_email; word from the same list, capitalize, include_number; email/domain arguments), and `vault_generate_check_password` (Read: strength estimate — length, character classes, entropy bits, common-password check against a tiny embedded list — **without ever sending the password anywhere**; no HIBP call).
