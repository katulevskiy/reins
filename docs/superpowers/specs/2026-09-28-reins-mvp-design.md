# Reins MVP — Design Spec

Date: 2026-09-28 · Status: draft for review · Base: Vaultwarden 1.37.3 (AGPL-3.0)

## 1. Goal

Reins turns a Vaultwarden account into a **permission platform for AI agents**. A user
connects services (Gmail in the MVP) on their phone, connects AI clients (Claude, ChatGPT)
to Reins over MCP, and every action an AI takes is checked against **fine-grained,
phone-held grants**. Anything not covered by a grant is shown on the phone for approval,
Duo-style.

**MVP success criteria**

1. A user adds Reins as a custom connector in Claude.ai and in ChatGPT, authorizing it
   by approving a matching code on their phone.
2. The AI sees `gmail_search`, `gmail_read` and `gmail_send` tools.
3. A call not covered by a grant produces a phone notification. The approval screen shows exactly
   what will be released or sent; approve (biometric) or deny.
4. Approvals can be one-time, time-boxed, or use-limited, and can cover: specific message
   IDs, structured filters (sender/recipient/subject/body regex, labels, date range), or
   send-to recipient rules.
5. Covered calls execute without user interaction while the phone is reachable.
6. Phone offline → the AI gets a clear, actionable error.
7. The server never holds Gmail credentials and cannot access Gmail even if fully compromised.

**Out of scope for MVP:** iOS, multiple approval devices per account, connectors other than
Gmail, self-hosters using a prebuilt APK (push needs a matching Firebase project), UnifiedPush,
Google OAuth verification/CASA, in-app account sign-up (use the bundled web vault),
MCP Tasks extension, SSE progress streaming, formal protocol proof.

## 2. Architecture

```
 Claude / ChatGPT ──MCP (HTTPS, OAuth 2.1)──►  Reins server (Vaultwarden fork)
                                               │  • MCP endpoint + OAuth AS
                                               │  • in-memory relay (no Gmail creds)
                                               │  • FCM sender
                                FCM wake (id only)│  ▲ HTTPS fetch/respond (Bitwarden login token)
                                               ▼  │
                                         Reins Android app
                                         Kotlin shell + Compose UI
                                         └─ reins-core (Rust, UniFFI)
                                              • policy engine  • Gmail connector
                                              • local store    • Vaultwarden API client
                                               │
                                               ▼  Gmail API (token from Play Services, on-device)
```

**Trust model.** The phone is both the approver and the **only executor**. Gmail access
tokens are obtained on-device from Google Play Services (`AuthorizationClient`), bound to
the app's package + signing certificate; no Gmail refresh token exists on the phone or the
server. Grants live only on the phone. The server is a relay: it authenticates AI clients,
forwards requests, and returns results.

**Accepted MVP risks (documented, not mitigated):**
- The server sees tool arguments and released results in transit (unavoidable: the AI
  receives them over the same channel).
- A compromised server can fabricate requests labelled as any connected AI client. It can
  then (a) obtain data covered by existing grants for that client, or (b) show fake
  approval prompts. It can never exceed grants or reach Gmail without the phone.
- Relay state is in memory; one server instance per deployment.

**Why a new approval device is harmless.** An attacker who knows the master password can
log a second phone in and register it as the approval device (the old device gets notified).
That phone has no Google authorization for the victim's Gmail and no grants; it can only
approve new AI connections, which then have nothing to execute against.

## 3. Repository layout

The repo *is* the Vaultwarden fork (upstream remote `upstream`, base tag `1.37.3`), so
upstream merges stay trivial. Reins code is isolated in new directories; upstream files
are touched only at registration points (route mounts, `mod` lines, config group, schema,
migrations, workspace members).

```
Cargo.toml                 workspace: + crates/*
src/api/reins/          server feature (new)
  mod.rs  mcp.rs  oauth.rs  relay.rs  device_api.rs  fcm.rs  pages/ (HTML templates)
src/db/models/reins_*.rs
migrations/{sqlite,mysql,postgresql}/2026-09-28-000000_reins/
crates/
  reins-proto/          serde types shared by server and phone (no IO)
  reins-policy/         grant model + matcher (pure, no IO, heavily tested)
  reins-core/           phone logic, UniFFI exports (tokio, reqwest+rustls, rusqlite)
  reins-e2e/            test harness: real server + core with fake Gmail/Keystore/push
android/                   Gradle project (Kotlin, Compose)
docs/superpowers/          specs and plans
```

## 4. Server (Vaultwarden fork)

### 4.1 Persistent tables (all three DB backends)

| Table | Purpose | Key columns |
|---|---|---|
| `reins_devices` | the user's approval device (one per user) | `user_uuid` PK, `device_uuid`, `fcm_token`, `updated_at` |
| `reins_clients` | DCR-registered MCP clients | `client_id` PK, `client_name`, `redirect_uris` (JSON), `created_at` |
| `reins_connections` | an AI client authorized by a user | `uuid` PK, `user_uuid`, `client_id`, `label`, `created_at`, `last_used_at` |
| `reins_refresh_tokens` | MCP refresh tokens (SHA-256 hashed) | `token_hash` PK, `connection_uuid`, `expires_at` |

Deleting a connection cascades to its refresh tokens; access tokens die because every MCP
request checks that the connection still exists.

In-memory only (moka caches with TTL): OAuth authorization sessions and codes (5 min),
relay requests and results (10 min). **Email content and tool arguments are never written
to disk.**

### 4.2 MCP endpoint — `POST {domain}/mcp`

Hand-written JSON-RPC over Rocket (rmcp is axum-based; we reuse none of its HTTP layer).
Responses are always `application/json` (no SSE, no session IDs; `GET`/`DELETE` → 405).

**Dual-era protocol support:**
- *Modern (2026-07-28)*, used by ChatGPT: `server/discover`, `tools/list`, `tools/call`.
  Reads `params._meta["io.modelcontextprotocol/protocolVersion"]`; validates the
  `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name` headers against the body (mismatch →
  400, `-32020`); every result carries `resultType: "complete"`; `tools/list` carries
  `ttlMs` and `cacheScope: "private"`; `serverInfo` in result `_meta`.
- *Legacy (2025-11-25)*, used by Claude: `initialize`, `notifications/initialized` (202),
  `ping`, `tools/list`, `tools/call`.
- Unknown version → 400 with `-32022` and `data.supported`. Unknown method → 404, `-32601`.
  All errors are HTTP 400/404 with JSON-RPC bodies (never 422: breaks ChatGPT).
- `Origin` present and not an allowed AI origin → 403.

**Auth:** `Authorization: Bearer <access token>`. Missing/invalid → 401 with
`WWW-Authenticate: Bearer resource_metadata="{domain}/.well-known/oauth-protected-resource/mcp"`.

**Tools** (deterministic order; JSON Schema inputs):

| Tool | Input | Output |
|---|---|---|
| `gmail_search` | `query: string` (Gmail syntax), `max_results?: 1..50` (default 10) | list of `{id, thread_id, from, to, subject, date, snippet}` |
| `gmail_read` | `message_ids: string[]` (1..20) | list of `{id, thread_id, from, to, cc, subject, date, body_text}` |
| `gmail_send` | `to: string[]`, `cc?: string[]`, `subject: string`, `body: string`, `reply_to_message_id?: string` | `{id, thread_id}` |
| `reins_get_result` | `request_id: string` | the delayed result of an earlier call |

Results are `structuredContent` plus the same JSON in a `text` block. Policy outcomes
(denied, offline, pending) are `isError: true` results with human-readable text, so the
model can relay them to the user.

### 4.3 Relay semantics and timing

`tools/call` → create `RelayRequest {id, user, connection_id, connection_label, call, created_at}`
in memory → send FCM high-priority data message `{request_id}` → await a `oneshot` for up
to **45 s** (ChatGPT's hard tool timeout is 60 s):

| Situation | AI receives |
|---|---|
| Phone responds with result within 45 s | the result |
| Phone denies | `Denied by the user on their Reins device.` |
| Phone did not fetch the request within 10 s | `Reins: your approval device is offline. Ask the user to open the Reins app; the request is waiting there. Then call reins_get_result with request_id=<id>.` |
| Fetched but no decision within 45 s | `Waiting for the user to approve on their phone. Call reins_get_result with request_id=<id> after they confirm.` |

Pending requests stay retrievable by the phone for 10 minutes (so opening the app later
shows them). A late result is stored in memory until fetched via `reins_get_result` or
expired. `reins_get_result` itself waits up to 45 s on the same oneshot/notify, and only
the connection that created a request can read its result.

### 4.4 Phone-facing API — `/reins/api/*`

Authenticated with Vaultwarden's existing `Headers` guard (normal Bitwarden login token)
**and** the caller must be the user's registered approval device.

| Method, path | Purpose |
|---|---|
| `PUT /reins/api/device` | register this device as approval device (`fcm_token?`); notifies the previous device |
| `GET /reins/api/requests/pending?wait=25` | pending relay requests; long-polls up to 25 s when empty (foreground channel, works without FCM) |
| `GET /reins/api/requests/{id}` | fetch one request (marks it delivered) |
| `POST /reins/api/requests/{id}/response` | `{outcome: result \| denied \| error, payload}` |
| `GET /reins/api/pairings/{id}` / `POST …/response` | AI-connection pairing (§4.5) |
| `GET /reins/api/connections`, `DELETE …/{id}` | list/revoke AI connections |

### 4.5 OAuth 2.1 authorization server for AI clients

Mounted at root (outside the domain base path where required):

- `/.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp`
  → `{resource: "{domain}/mcp", authorization_servers: [issuer], scopes_supported: ["mcp"]}`.
- `/.well-known/oauth-authorization-server` → issuer `{domain}`; endpoints
  `/reins/oauth/{authorize,token,register}`; `code_challenge_methods_supported: ["S256"]`;
  `token_endpoint_auth_methods_supported: ["none"]`; `grant_types_supported:
  ["authorization_code","refresh_token"]`; `client_id_metadata_document_supported: true`;
  `authorization_response_iss_parameter_supported: true`.
- **Client identification:** CIMD (client_id is an HTTPS URL; fetched through Vaultwarden's
  SSRF-guarded `http_client`, cached 1 h) or DCR (`/register`, public clients only).
  Redirect URIs must match the client's registered list exactly, except loopback URIs,
  which match with any port.
- **Authorize page** (server-rendered HTML, no JS crypto, no password):
  1. User enters their account email.
  2. Server creates a pairing `{id, user, client, redirect_uri, code_challenge, resource,
     state, two-digit match code}` and pushes it to the approval device. The page shows the
     match code and long-polls. Unknown emails get the identical page (no enumeration),
     and requests are rate-limited per IP.
  3. Phone shows: *"Connect **ChatGPT** (chatgpt.com) to Reins? Code **47**"*. The user
     picks the matching code from three options, can rename the label, and approves with biometrics.
  4. Page redirects to `redirect_uri?code=…&state=…&iss={issuer}`.
- **Token endpoint** (`application/x-www-form-urlencoded`): verifies PKCE S256, `resource`
  equals the canonical MCP URL, code single-use (60 s). Issues an RS256 JWT access token
  (issuer `{domain}|mcp`, `aud` = MCP URL, `sub` = user, `cid` = connection, 1 h) and an
  opaque rotating refresh token (30 days). Dead refresh token → `invalid_grant`.
- Allowed redirect URIs in practice: `https://claude.ai/api/mcp/auth_callback`,
  `https://chatgpt.com/connector_platform_oauth_redirect`,
  `https://chatgpt.com/connector/oauth/{id}`, loopback (Claude Code).

### 4.6 Push

`fcm.rs` calls FCM HTTP v1 (`messages:send`) with a Google service-account JWT (RS256, via
the existing `jsonwebtoken` crate), token cached for half its lifetime. Data-only message,
`priority: high`, payload `{t: "req"|"pair", id}`. No request content ever transits Google.

New config group `reins`: `reins_enabled`, `reins_fcm_service_account` (path),
`reins_relay_wait_secs` (45), `reins_offline_secs` (10).

## 5. Rust crates

### 5.1 `reins-proto`
Serde types used on both sides: `ToolCall` (`GmailSearch{query,max_results}`,
`GmailRead{message_ids}`, `GmailSend{to,cc,subject,body,reply_to_message_id}`),
`RelayRequest`, `RelayResponse`, `Pairing`, result DTOs (`MessageSummary`, `MessageFull`,
`SentMessage`). Versioned with a `v: 1` field.

### 5.2 `reins-policy` (pure)

```text
Grant {
  id, connection_id,                       // which AI client
  scope: Scope,                            // Read covers both gmail_search results and gmail_read
  created_at, expires_at: Option<ts>, max_uses: Option<n>, uses, revoked
}                                          // no expiry and no max_uses = until revoked
Scope::Read(ReadScope { message_ids: Option<Set>, from: Vec<AddrRule>, to: Vec<AddrRule>,
                        subject: Option<Regex>, body: Option<Regex>, labels: Vec<String>,
                        after: Option<Date>, before: Option<Date> })
Scope::Send(SendScope { recipients: Vec<AddrRule>, subject: Option<Regex>, body: Option<Regex> })
AddrRule = Exact(addr) | Domain(domain) | Regex(re)
```

- **One-time approvals do not create grants.** Approving a pending request executes exactly
  that request (the selected messages, or the shown email) and is recorded in the audit
  log. Grants exist only for standing permission ("allow for 1 h", "for 5 uses", "until
  revoked").
- All constraint fields must match (AND); list fields match if any rule matches (OR).
  An empty `ReadScope` is rejected at construction and, if one is ever loaded from
  storage, matches nothing (fail closed).
- Patterns are case-insensitive. `subject`/`body` patterns search; address patterns must
  match the whole address. `Domain(d)` matches exactly `d`, not subdomains.
- Matching is performed **locally against actual message data** fetched from Gmail. The
  AI's Gmail query is only a prefilter and is never composed with grant filters (so query
  injection like `x) OR (y` cannot widen scope).
- `Send` requires *every* recipient in `to`+`cc` to be covered.
- Regexes use the `regex` crate (linear time, no ReDoS), size-limited, compiled once and cached.
- `evaluate_read(grants, connection, messages, now) -> ReadDecision { allowed: [(msg, grant)], needs_approval: [msg] }`
  and `evaluate_send(grants, connection, email, now) -> Allowed(grant) | NeedsApproval`.
  When several grants match, unlimited grants are preferred over use-limited ones. One
  executed tool call consumes one use of each grant it relied on, applied only after
  successful execution.
- Property tests (proptest): an allowed item always satisfies some unexpired, unrevoked
  grant for the same connection and action; revocation and expiry are always honored.

### 5.3 `reins-core` (phone)

- **Runtime:** one process-wide multi-thread tokio runtime (`OnceLock`); UniFFI exports are
  `async fn` → Kotlin `suspend fun`. Nothing runs on the Android main thread. Gmail
  metadata/body fetches run concurrently (`buffer_unordered(8)`, respecting quota).
- **Vaultwarden client:** prelogin → KDF (PBKDF2-SHA256 or Argon2id, per account) → master
  password hash → `/identity/connect/token` (device type Android, persistent device id);
  TOTP 2FA supported; other 2FA methods return a clear "not supported yet" error.
  Token refresh handled internally.
- **Local store:** SQLite (`rusqlite`, bundled) in app-private storage: grants, audit log,
  cached connections, session. Sensitive values (refresh token, audit details) are
  AES-256-GCM encrypted with a random data key that is wrapped by an Android Keystore key.
- **Foreign traits implemented in Kotlin** (`#[uniffi::export(foreign)]`):
  `KeyWrapper { wrap, unwrap }` (Keystore AES-GCM), `GoogleTokenProvider { access_token() }`
  (AuthorizationClient; returns `NeedsUserInteraction` if consent is required),
  `Notifier { request_pending(summary) }`.
- **Request handling** (`handle_request(id)`): fetch request → gather facts (for reads:
  run the Gmail search/fetch locally; data stays on the phone) → `evaluate` → if fully
  allowed: execute and respond; otherwise notify and park as pending for the UI.
- **Approval** (`approve(request_id, ApprovalChoice)`): the UI supplies the selected
  message IDs (reads) or confirms the exact email (send) plus a lifetime and optional
  broadened scope; core creates the grant, executes, responds, and appends to the audit log.
- **Gmail connector:** REST v1 (`messages.list` with `q`, `messages.get` format
  metadata/full, `messages.send` raw base64url RFC 2822, threading headers for replies).
  Plain-text body extraction from MIME (text/plain preferred, HTML stripped otherwise).
  Scopes: `gmail.readonly`, `gmail.send`.

## 6. Android app (`android/`)

Package `dev.reins.android`; minSdk 31, targetSdk 36; Kotlin + Jetpack Compose only.
Dependencies limited to: Compose (BOM), `activity-compose`, `lifecycle-viewmodel-compose`,
`androidx.biometric`, `work-runtime-ktx`, `firebase-messaging`, `play-services-auth`
(AuthorizationClient), `kotlinx-coroutines`, JNA (`@aar`, for UniFFI). No DI framework,
no Room/Retrofit/OkHttp (Rust owns storage and networking).

Build: Gradle `Exec` task runs `cargo ndk -t arm64-v8a -t x86_64 -o src/main/jniLibs build
--release -p reins-core` and `uniffi-bindgen generate --language kotlin`; NDK r28+
(16 KB pages).

**Screens**
1. *Sign in*: server URL, email, master password, TOTP if required.
2. *Home*: pending requests (top), Gmail status, connected AIs, quick links.
3. *Approval*: who is asking (connection label), what (query / recipients / full email
   preview), for reads a checklist of found messages (from, subject, date, snippet);
   lifetime selector (Once · 1 h · 24 h · 7 days · until revoked · N uses); optional
   "also allow similar" builder (sender/domain, subject regex, label). Approve requires
   `BiometricPrompt` (strong biometric or device credential). Deny is one tap.
4. *Connect AI*: pairing approval with three-way code match and label edit.
5. *Grants*: list per connection, revoke.
6. *Activity*: audit log (what was released/sent, to whom, when, under which grant).
7. *Settings*: connect/disconnect Gmail, sign out, re-register device.

`FLAG_SECURE` on approval, grants and activity screens.

**Background:** `FirebaseMessagingService.onMessageReceived` checks priority and enqueues an
expedited `CoroutineWorker` that calls `core.handleRequest(id)`. When a request needs
approval, the app posts a notification opening the Approval screen. While the app is in
the foreground it also long-polls `/requests/pending`.

## 7. Error handling

- Every core API returns a typed `ReinsError` (network, auth, gmail, policy, storage,
  needs_user_interaction) mapped to Kotlin exceptions; UI shows actionable messages.
- Gmail 401 → refresh token via Play Services once, then `NeedsUserInteraction`.
- Gmail 429/5xx → bounded exponential backoff within the relay budget.
- The phone always responds to a fetched request (result, denied, or `error{message}`), so the
  AI never waits the full timeout for a known failure.
- Server: MCP errors are JSON-RPC shaped; OAuth errors RFC 6749 shaped; phone API errors use
  Vaultwarden's existing error type.

## 8. Testing

- `reins-policy`: unit + property tests (the security core).
- `reins-core`: Gmail connector against `wiremock`; MIME parsing fixtures; store
  encryption round-trips; KDF test vectors against Bitwarden's documented values.
- Server: unit tests for PKCE, redirect matching, CIMD/DCR, JWT audience, MCP dual-era
  handling (fixtures of real Claude `initialize` and ChatGPT `server/discover` requests),
  relay timing (offline / pending / late result).
- `reins-e2e`: starts the real server (SQLite, temp dir) and a headless "phone" built on
  `reins-core` with fake Gmail, fake Keystore and an in-process push hook; drives a full
  OAuth + MCP session: authorize → pair → search → approve → read → send → revoke.
- Android: build + install on emulator; instrumented smoke test of sign-in and approval
  screens against a local server.
- CI rules follow Vaultwarden's: `cargo clippy` with warnings denied, `cargo fmt`.

## 9. External prerequisites (manual, by the project owner)

1. Google Cloud project: OAuth consent screen (External, **In production, unverified**, to
   avoid 7-day refresh expiry; 100-user lifetime cap until verification + CASA), Gmail
   API enabled, Android OAuth client (package + signing SHA-1).
2. Firebase project: `google-services.json` for the app, service-account JSON for the server.
3. A public HTTPS domain with an IPv4 `A` record (Claude connectors are IPv4-only); for
   development, a tunnel (e.g. cloudflared).

Until 1–2 exist, development uses the e2e harness and the foreground long-poll channel.

## 10. Open risks to validate early

- Silent `AuthorizationClient.authorize()` from a background worker (spike first; fallback:
  notification asking the user to open the app).
- Claude's handling of the 45 s wait (Claude allows 240 s; fine) and ChatGPT's 60 s cap.
- Vaultwarden workspace lints (`warnings = deny`, pedantic clippy) applied to new crates.
