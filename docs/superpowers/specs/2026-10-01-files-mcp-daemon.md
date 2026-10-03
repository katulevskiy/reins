# Large files, universal MCP, and the desktop app's next parts

Date: 2026-10-01. Status: approved direction (user), being implemented in parallel.

## Principles (from the user)

- **The phone stays the executor and keeps the credentials.** Nothing moves off the phone by default.
- **Large files go through the server, for one operation only, under the phone's control.** The phone tells the AI
  where to upload (or gives it a download link), tells the server what to expect, and decides; the server holds the
  bytes only until that operation is done.
- **Universal MCP runs through the phone**, except large results, which the phone detects and routes through the
  server.
- The desktop app (`reins`) grows into the local hub: `ask`, harness hooks, `mcp`, `run`, an API proxy, an SSH
  agent, and git for more hosts.
- Harness "auto-login" is NOT done by copying Claude/Codex/Grok sign-ins (refresh tokens rotate; copying breaks one
  machine and breaches terms). Instead: `reins harness add <name>` wires Reins into the harness (MCP + hooks),
  and `reins run --profile <name> -- <harness>` launches it with API keys released from the vault by the phone.

## Contract (committed in `reins-proto`; do not change it, report if it must change)

- `blob.rs`: slots, states, previews, `BlobSend` (+ `SendBody::JsonBase64`), `BlobFetch`, `BlobDownload`,
  `UploadInstructions`, limits (`INLINE_LIMIT` 256 KiB, `MAX_BLOB_BYTES` 1 GiB, TTLs).
- `remote_mcp.rs`: `McpServerReport`/`McpToolReport` (in `ServicesReport.mcp`), `McpCall`, `ProxyCall(Result)`,
  `exposed_name`, `server_id_ok`, `HEAVY_RESULT_BYTES`.
- `gmail::ToolCall::{Mcp, RequestUpload}`; `relay::ToolResult::Mcp { result }`; `device::Pending.blobs`;
  `pairing::PushKind::Blob`.
- Tools: `github_api_read` (GET, Read), `github_api_write` (POST/PUT/PATCH/DELETE, Write, `body` JSON); optional
  `blob` on `github_release_asset_upload` (content now optional), `github_file_put`, `vault_attachment_add` (content now
  optional), `vault_send_create`.
- Desktop-only tools: `desktop_ask` (service `desktop`), `vault_secret_release` (class `secrets`), `vault_ssh_keys`,
  `vault_ssh_sign` (class `ssh`), and `{gitlab,codeberg,bitbucket}_git_{fetch,push,tag_push}` (services `gitlab`,
  `codeberg`, `bitbucket`, classes `code`/`releases`). `PushSummary::tool_for(service)`, `fetch_tool_for(service)`.
- Sealed payloads (crypto_box sealed box to the pinned desktop key, as for git): `AskAnswer`, `SecretGrant`,
  `SshSignature` (in `desktop.rs`), plus the existing `CredentialGrant` for the new git hosts.

## S — Server (`src/api/reins/*`)

1. **Blob store.** Files under `$DATA_FOLDER/reins-blobs/` (0700, random file names), metadata in memory (a
   `BlobHub` in `Hub`, TTL map; a restart loses them, fine). Per user at most 20 blobs and 2 GiB; global 8 GiB. Expired
   files deleted by the existing purge job (and on startup: wipe the directory). Capabilities: separate random 32-byte
   base64url secrets for upload and download, looked up by their SHA-256 (no timing leaks).
   - Phone API (approval-device auth like the rest of `/reins/api`), owner checked on every call:
     `POST /reins/api/blobs` (`BlobSlotRequest` → `BlobSlot`), `GET /reins/api/blobs/<id>` (`BlobInfo`),
     `POST /reins/api/blobs/<id>/decision` (`BlobDecision`; only for `Upload` purpose), `GET
     /reins/api/blobs/<id>/content` (stream, `Range` optional), `PUT /reins/api/blobs/output?connection_id=…&name=…&ttl_secs=…`
     (phone-made result → `BlobDownload`), `POST /reins/api/blobs/<id>/send` (`BlobSend` → `BlobSendResult`),
     `POST /reins/api/blobs/fetch` (`BlobFetch` → `BlobDownload`), `DELETE /reins/api/blobs/<id>`.
   - Public: `PUT` (and raw `POST`) `/reins/blob/<secret>` uploads (stream to disk, enforce `max_bytes`, SHA-256
     while streaming, sniff type from magic bytes: PNG/JPEG/GIF/WebP/PDF/ZIP/gzip/ELF/Mach-O/MP4/UTF-8 text, build the
     `BlobPreview`), single use, answers 201 `{"blob": id, "size", "sha256", "next": <plain words>}`; for an `Upload`
     blob, push FCM `PushKind::Blob` and include it in `Pending.blobs` (delivered once). `GET /reins/blob/<secret>`
     downloads (only `Output` blobs and approved `Upload` blobs; `Content-Disposition: attachment` with a sanitized
     name; until `expires_at`, at most 20 downloads).
   - `send`/`fetch`: https only (http to loopback allowed only when a test-only config says so); resolve the host and
     refuse private, loopback, link-local and metadata addresses (SSRF); redirects: follow up to 3, dropping the
     phone's headers when the host changes; timeouts; never log headers or bodies. `SendBody::JsonBase64` builds the
     JSON with the base64 of the blob streamed in.
   - nginx (lead does it): `client_max_body_size 1g` and `proxy_request_buffering off` for `/reins/blob/`.
2. **Universal MCP tools.** Keep each user's reported `McpServerReport`s (validated; invalid ones dropped) from `PUT
   /reins/api/services`. `tools/list` adds them as `exposed_name(server, tool)`, title/description prefixed with the
   server name, `inputSchema` as reported, annotations from `read_only`/`destructive`; `tools/call` of such a name →
   relay `ToolCall::Mcp { server, tool: <original name>, arguments }`. Results `ToolResult::Mcp { result }` are passed
   through as the MCP `CallToolResult` (`content`, `structuredContent`, `isError`) instead of the JSON-text rendering.
3. **`reins_upload` tool** (always listed): `{name, size, content_type?, reason}` → relay `ToolCall::RequestUpload`.
   Description: for passing a file to another tool as a link; the phone answers with an upload link; the user approves
   when it arrives; then the download link works.
4. **`POST /reins/api/mcp/call`** (`ProxyCall` → `ProxyCallResult`): one JSON-RPC POST to the endpoint (SSRF rules
   above; `Accept: application/json, text/event-stream`; SSE parsed until the response with the request's id), large
   content items (base64 image/audio data, embedded resources, long text) stored as `Output` blobs and replaced by
   `{"type":"resource_link","uri":<download_url>,"name":…,"mimeType":…}`.
5. **Who asked:** an optional request header `X-Reins-Via` on `/mcp` (≤ 40 chars, sanitized) is appended to the
   connection label of relayed requests ("Laptop · Claude Code").
6. Tests for all of it (integration tests in `tests/reins_server`).

## P1 — Phone core: files and `github_api` (`crates/reins-core`)

- API client for every blob endpoint. `PendingKind::Blob` items from `Pending.blobs` and pushes; `blob_view(id)`,
  `answer_blob(id, approve)` (UniFFI), audit entries.
- `RequestUpload`: open an `Upload` slot (max = size + 10 %, ≤ `MAX_BLOB_BYTES`) and answer at once with
  `{status: "upload_ready", blob, upload_url, download_url, expires_at, next}`; `next` says: upload with `curl -T`,
  the user will be asked when it arrives, then the download link works.
- File-taking tools (`github_release_asset_upload`, `github_file_put`, `vault_attachment_add`, `vault_send_create`):
  neither content nor `blob` (or the content would exceed what fits) → open a `ToolInput` slot and answer
  `UploadInstructions` (not an error). With `blob`: it must belong to the connection, be uploaded, unexpired, of
  purpose `ToolInput` for this tool; the write preview shows the file (name, size, type, sha256, text head or image) and
  `ApprovalView.blob`; perform: GitHub targets via `BlobSend` (asset upload: raw to the `upload_url`; contents API:
  `JsonBase64` into `content`), vault ones read the bytes (`GET …/content`, ≤ 100 MiB) and encrypt on the phone as
  today; delete the blob after.
- Large downloads: results over `INLINE_LIMIT` (release assets, raw files, logs, artifacts when not already a URL) →
  `BlobFetch` with the request's auth headers, answer `{download_url, name, size, sha256, content_type, expires_at}`;
  vault attachments (decrypted on the phone) over the limit → `PUT …/blobs/output` and the same answer.
- `github_api_read/write`: path checks (starts with `/`, no `..`, `//`, scheme, host, control chars), query map,
  resource from the path (`/repos/o/r…` → `o/r`, branch paths → `o/r@b`, `/orgs/x…` → `x`, `/user`, `/gists`,
  `/notifications` … → `account`, else `github`), parents as elsewhere; write class from the path (issues, pulls, code,
  releases, actions, settings, account) via a new `Preview.class: Option<String>` honoured by the flow (it overrides
  the spec's class); once-only for repository deletion/transfer/visibility/archive/rename, collaborators, invitations,
  teams, webhooks, deploy keys, secrets, branch protection/rulesets, and any org-level DELETE. Preview lines: method,
  path, query, pretty body (≤ 2,000 chars). Reads return GitHub's JSON (cut with a note when large, binary → link).
- UniFFI: `BlobView { id, connection_label, name, size, content_type, sha256, purpose (text), preview_text:
  Option<String>, preview_image: Option<Vec<u8>>, created_at, expires_at }`, `PendingKind::Blob`,
  `ApprovalView.blob: Option<BlobView>`.

## P2 — Phone core: universal MCP (`crates/reins-core`)

- Store table `mcp_servers` (id, name, url, auth kind, client registration, tools JSON, heavy tools, added_at); tokens
  in the secrets store.
- MCP client (Streamable HTTP): `initialize` (2025-06-18, fall back to 2025-03-26), `notifications/initialized`,
  `tools/list` with cursors, `tools/call`; JSON or SSE answers; `Mcp-Session-Id`; 401 → refresh, else "needs sign-in".
- OAuth (MCP authorization spec): RFC 9728 resource metadata (from `WWW-Authenticate` or well-known), RFC 8414 / OIDC
  discovery, RFC 7591 registration with redirect `dev.reins.android://mcp-oauth`, PKCE S256, `resource`; refresh.
  Static bearer token as an alternative.
- UniFFI: `mcp_servers() -> Vec<McpServerView>`, `mcp_add(url, name: Option<String>) -> McpAddStep`
  (`Added { server }` or `NeedsSignIn { server_id, authorize_url }`), `mcp_finish_sign_in(server_id, redirect_url)`,
  `mcp_add_with_token(url, token, name)`, `mcp_refresh(id)`, `mcp_remove(id)`, `mcp_set_heavy(id, tool, heavy)`;
  `McpServerView { id, name, url, status ("ok"|"needs_sign_in"|"error"), error: Option<String>, tools:
  Vec<McpToolView { name, title, description, read_only, destructive, heavy }> }`.
- The services report carries the servers and tools (re-sent when they change).
- `ToolCall::Mcp`: grants with service `mcp:<id>`, resource = tool name, access read when `read_only`, else write;
  `destructive` → once-only; unknown server/tool → error. Approval: `ApprovalView.mcp: Option<McpCallView { server_name,
  server_url, tool, title, description, arguments_json (pretty, ≤ 8,000 chars), read_only, destructive }>`. Perform:
  `tools/call` and answer `ToolResult::Mcp`; a result over `HEAVY_RESULT_BYTES` marks the tool heavy, and its large
  items are uploaded via `PUT …/blobs/output` and replaced by `resource_link`s; heavy tools go through `POST
  /reins/api/mcp/call` from then on.

## P3 — Phone core: desktop tools and more git hosts (`crates/reins-core`)

- `desktop_ask` (connector `desktop`): preview question/detail/topic; resource = topic or `ask`; perform → sealed
  `AskAnswer { approved: true }`; denial is the normal denied outcome.
- `vault_secret_release`: resolve `item/field` (item id or exact name; fields `password`, `username`, `totp` (current
  code), `notes`, a custom field name, `uri`); preview shows command, purpose, item names and field names only, lease;
  resource = the item (one) or `secrets` (several); perform → sealed `SecretGrant` (expires now + lease).
- `vault_ssh_keys`: the vault's SSH key items (public key line, SHA256 fingerprint, name), plain answer.
  `vault_ssh_sign`: preview "Sign in to <host or host key> with <key name>"; resource `<fp>@<host>` (parent `<fp>`);
  perform: sign on the phone (ed25519; RSA with rsa-sha2-256/512 by flags; ECDSA P-256/P-384) → sealed `SshSignature`.
- Connectors `gitlab` (gitlab.com), `codeberg` (codeberg.org, Gitea API), `bitbucket` (bitbucket.org): token
  sign-in (checked against the host's API), git fetch/push/tag push sharing the GitHub git code (move the shared part
  into a module); credential user names: GitLab `oauth2`, Codeberg the account name, Bitbucket per its token kind.
- Desktop-only key check as for GitHub git (already in the flow).
- UniFFI: `ApprovalView.ask: Option<AskView { question, detail, topic }>`, `.secrets: Option<SecretReleaseView {
  command, purpose, items: Vec<String>, lease_secs }>`, `.ssh: Option<SshSignView { key_name, key_fingerprint, host,
  host_key }>`; git host pushes reuse `ApprovalView.git`.

## D1 — Desktop: `ask`, hooks, harnesses, `mcp` (`crates/reins-desktop`)

- `reins ask "<question>" [--detail TEXT|-] [--topic T] [--timeout S]`: phone mode via `desktop_ask` (verify the
  sealed nonce); local mode: a terminal prompt when interactive, else the desktop prompt. Exit 0 yes, 1 no, 2 no
  answer/unavailable.
- `reins hook <harness>`: reads the harness's pre-tool hook JSON from stdin; when the command or file matches the
  guard rules (config `[guard]`, sensible defaults: force pushes, `reset --hard`, `rm -rf`, `terraform apply|destroy`,
  `kubectl apply|delete`, `DROP`, `publish`, `.env`/keys files…), asks the phone and answers in the harness's format
  (Claude Code `PreToolUse` `permissionDecision`; Cursor `beforeShellExecution`; others where the harness has
  pre-command hooks — check each harness's current docs); unmatched → allow silently.
- `reins harness add|remove|list <claude-code|codex|gemini|cursor>`: registers `reins mcp` in the harness's MCP
  config and the hook where supported; `remove` undoes exactly; `list` shows what is set.
- `reins mcp [--via NAME]`: stdio MCP server bridging to `<server>/mcp` with the app's token (refresh on 401),
  sending `X-Reins-Via`.

## D2 — Desktop: `run`, API proxy, SSH agent (`crates/reins-desktop`)

- `reins run [--env NAME=vault:Item/field]... [--profile P] -- <cmd>`: `vault_secret_release`, open the sealed
  `SecretGrant`, run the command with those variables, exit with its code, zeroize. Profiles in config.
- API proxy: config `[[api]] name, base, header ("Authorization: Bearer {secret}"), secret = "vault:Item/field",
  lease_secs`; `http://127.0.0.1:7457/api/<name>/<path>` → `<base>/<path>` with the header; secret leased via
  `vault_secret_release` and kept until expiry; streaming; same Host/Origin checks; never log secrets.
- SSH agent on a unix socket (`$XDG_RUNTIME_DIR/reins/ssh-agent.sock`): identities from `vault_ssh_keys`, signing
  via `vault_ssh_sign`, `session-bind@openssh.com` for the host key, host name from `~/.ssh/known_hosts` when
  unhashed; waiting notice to the ssh client via `SO_PEERCRED` + `notice`; `reins ssh setup|unsetup` (an
  `IdentityAgent` block in `~/.ssh/config`, reversible).

## D3 — Desktop: git for GitLab, Codeberg, Bitbucket (`crates/reins-desktop`)

- Config `[[git.hosts]]` (github.com, gitlab.com, codeberg.org, bitbucket.org built in, each with service id,
  git/API bases); proxy routes `/<host>/<repo path>(.git)/…` with nested paths (GitLab groups); tool names via
  `fetch_tool_for(service)` / `summary.tool_for(service)`; analysis with `GitHubRemote` for GitHub, `NoRemote` (pack
  only) elsewhere; `pause`/`resume`/`git setup` cover every enabled host.

## A — Android (after P1–P3 merge)

MCP servers screens (list, add by URL with optional name or token, sign-in via Custom Tabs with the
`dev.reins.android://mcp-oauth` redirect, detail with tools and heavy toggles, remove); approval rendering for MCP
calls, attached files (preview), uploads (pending blob sheet), asks, secrets, SSH; integration cards for GitLab,
Codeberg, Bitbucket with token pages.

## Rules for every worker

Test-first; workspace lints (pedantic clippy as errors, `unsafe_code = "forbid"`); keep shared files' edits additive
and minimal (new code in new files); never log tokens, headers, file contents or secret values; commit on your branch,
do not push or merge; report files, deviations, test counts, branch and commit.
