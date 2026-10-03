# Reins Plan 2: Server (Vaultwarden fork) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the Vaultwarden 1.37.3 fork into the Reins server: an OAuth 2.1 authorization server for AI clients with phone pairing, a dual-era MCP endpoint that relays tool calls to the user's approval device, the phone-facing API (contracts §A), an FCM wake-up sender, and four new tables on all three database backends.

**Architecture:** All Reins server code lives in `src/api/reins/` (plus `src/auth/reins.rs` for the MCP JWT and `src/db/models/reins_*.rs`); upstream files are touched only at registration points. Protocol logic (JSON-RPC dispatch, OAuth validation, relay timing, HTML rendering, FCM message building) is written as pure functions and plain structs with `#[cfg(test)]` unit tests; Rocket handlers are thin adapters. HTTP behaviour is verified by an integration test crate target (`tests/reins_server/`) that spawns the real `vaultwarden` binary with a temporary SQLite database and drives it over HTTP.

**Tech Stack:** Rust 1.98.1 (edition 2024), Rocket 0.5.1, tokio 1.53.1 (`sync`, `time`; dev: `test-util`, `macros`), diesel 2.3.13 (sqlite/mysql/postgresql), jsonwebtoken 11.0.0 (RS256), ring 0.17.14 / data-encoding 2.11.1, reqwest 0.13.5 via `crate::http_client`, moka 0.12.16 (CIMD cache), url 2.5.8, chrono 0.4.45, `reins-proto` (Plan 1).

**Spec:** `docs/superpowers/specs/2026-09-28-reins-mvp-design.md` (§2, §4, §7, §8). **Binding contracts:** `docs/superpowers/specs/2026-09-29-reins-contracts.md` sections A and C (this plan implements them exactly) and "Proto invariants consumers must honor". Background notes: `docs/superpowers/notes/vaultwarden-extension-points.md`, `docs/superpowers/notes/platform-research-2026-09.md`. Style reference: `docs/superpowers/plans/2026-09-29-reins-1-proto-policy.md`.

## Global Constraints

- Repo root `<repo>`, branch `reins-mvp`. Before any cargo command: `export PATH="$HOME/.cargo/bin:$PATH"`.
- Test command: `cargo test --features sqlite <filter>`. Lint gate (all must pass before each commit): `cargo fmt --check`, `cargo clippy --features sqlite --all-targets -- -D warnings`, and `cargo clippy --features sqlite,mysql,postgresql -- -D warnings` (native `libpq` and `libmysqlclient`/`mysql_config` are installed on this machine; verified with `cargo check --features sqlite,mysql,postgresql`).
- Workspace lints apply to the `vaultwarden` crate: `warnings = deny`, `unsafe_code = forbid`, `non_ascii_idents = forbid`, clippy `pedantic`; deny `str_to_string` (use `to_owned()`), `redundant_clone`, `clone_on_ref_ptr`, `unused_qualifications`, `single_use_lifetimes`, `trivial_casts`, `variant_size_differences`. rustfmt: `max_width = 120`, `use_small_heuristics = "Off"`.
- Upstream files are modified only at registration points: `Cargo.toml` (`reins-proto` in `[dependencies]`, new `[dev-dependencies]`), `src/main.rs` (mounts, job), `src/api/mod.rs` (`mod` + re-exports), `src/auth.rs` (`mod` line), `src/config.rs` (config group + one validation call), `src/db/schema.rs`, `src/db/models/mod.rs`, `src/util.rs` (`LOGGED_ROUTES`), `.env.template`. Never change existing behaviour of upstream routes.
- Email content and tool arguments are never written to disk (spec §4.1): relay requests, results, pairings, OAuth sessions and codes live only in memory.
- Relay timing (spec §4.3, contracts C): wait **45 s** (`REINS_RELAY_WAIT_SECS`), offline threshold **10 s** (`REINS_OFFLINE_SECS`), pending requests/pairings/late results retained **600 s**.
- AI-visible texts, verbatim (spec §4.3): `Denied by the user on their Reins device.` · `Reins: your approval device is offline. Ask the user to open the Reins app; the request is waiting there. Then call reins_get_result with request_id=<id>.` · `Waiting for the user to approve on their phone. Call reins_get_result with request_id=<id> after they confirm.`
- MCP versions: modern `2026-07-28`; legacy `2025-11-25` (also accept `2025-06-18`, `2025-03-26`). Errors: header mismatch `-32020` (HTTP 400), unsupported version `-32022` with `data.supported` (HTTP 400), unknown method `-32601` (HTTP 404). Never answer 422 on `/mcp` or `/reins/oauth/token`.
- OAuth: PKCE `S256` only; authorization codes single-use, **60 s**; authorization sessions **300 s**; access token RS256 JWT, `iss = "{DOMAIN origin}|mcp"`, `aud` = canonical MCP URL, lifetime **3600 s**; refresh tokens opaque, stored as SHA-256 hex, rotating, **30 days**; dead refresh token → `invalid_grant`; CIMD documents cached **1 h**.
- Phone API errors: HTTP status + `{"error": "<code>", "message": "<text>"}`; codes `not_approval_device` (403), `not_found` (404), `already_answered` (409), `wrong_code` (409), `bad_version` (400), `bad_request` (400), `unauthorized` (401, from the catcher), `internal_error` (500). `wait` for A2 is clamped to `0..=25`.
- FCM: HTTP v1 `https://fcm.googleapis.com/v1/projects/{project_id}/messages:send`, data-only, `android.priority = "HIGH"`, data `{t, id}`; service-account token cached for half its lifetime. Dev key path: `~/.config/reins/fcm-service-account.json`.

## Review Focus

- **A phone that answers a pairing with the wrong code, twice, or for someone else's pairing** must never create a connection: wrong code → 409 `wrong_code` and the browser gets `access_denied`; second answer → 409 `already_answered`; other user → 404. Pinned in Task 5 (hub) and Task 11 (HTTP flow).
- **Authorization-code replay and PKCE/redirect/resource substitution** at `/reins/oauth/token` (reused code, wrong `code_verifier`, different `redirect_uri`, `resource` of another server, code older than 60 s) must all yield `invalid_grant`/`invalid_target` and never a token. Pinned in Task 9 (pure) and Task 11 (HTTP).
- **A revoked connection keeps using its still-valid access token or its refresh token**: after A8 the next `/mcp` call must be 401 and the refresh token `invalid_grant`. Pinned in Task 14.
- **Hostile display strings from AI clients** (`client_name` with bidi overrides, control characters, HTML such as `<script>`, 10 KB names) must be stripped/escaped before reaching the phone or the HTML page. Pinned in Task 5 (sanitize) and Task 10 (HTML escaping).
- **The phone never fetches a request (offline) or fetches it but never decides**: the AI must get the exact offline text after the offline threshold (not after the full wait), the exact pending text after the wait, and `reins_get_result` must deliver a late result exactly once per connection and never to another connection. Pinned in Task 4 (paused-time tests) and Task 14 (HTTP).

---

## Decisions

Made autonomously while planning (the human was unavailable); each is binding for the executor.

1. **Timestamps in the new tables are `BIGINT` unix seconds**, not `DATETIME/TIMESTAMP`, because every contract DTO uses `i64` seconds; no chrono conversion code is needed.
2. **Foreign keys:** `reins_devices.user_uuid` and `reins_connections.user_uuid` reference `users(uuid) ON DELETE CASCADE`; `reins_refresh_tokens.connection_uuid` references `reins_connections(uuid) ON DELETE CASCADE`. No FK to `devices` (its PK is composite and device rows are rewritten by upstream code). Vaultwarden never enables `PRAGMA foreign_keys` on SQLite, so `ReinsConnection::delete` deletes the refresh tokens explicitly.
3. **Connections store `client_name` and `client_host` denormalized** (needed by `ConnectionInfo`), because CIMD clients are never persisted: only DCR clients go into `reins_clients`; CIMD documents live in a 1 h in-memory moka cache.
4. **In-memory state uses a small `TtlMap` on `tokio::time::Instant` instead of moka** for relay requests, pairings, OAuth sessions and codes. Reason: the timing rules (10 s / 45 s / 60 s / 600 s) must be unit-tested with `tokio::time::pause()`, and moka's clock cannot be paused. moka is still used for the CIMD cache (no timing tests needed). The spec's "moka caches with TTL" intent (bounded, expiring, in memory) is preserved: every map has a TTL and a capacity cap.
5. **Waiting uses `tokio::sync::watch`** (per relay request / pairing: state channel; global: a "new item" counter for A2 long-polls) instead of `oneshot`/`Notify`: a watch receiver never misses an update made between "check state" and "start waiting", and several waiters (the MCP call and a later `reins_get_result`) can observe the same result.
6. **A2 (`GET /pending`) returns only items not yet delivered.** Returning every unanswered item would make the long-poll return instantly whenever the user leaves a request undecided (busy loop). The phone keeps delivered items locally (Plan 3 `pending()`); A3/A5 re-fetch any unanswered item by id. (Reported as a contract clarification.)
7. **Late results stay readable** by `reins_get_result` (same connection only) until the 600 s TTL expires, i.e. repeated calls return the same result; the phone's answer is accepted once (`already_answered` afterwards).
8. **`reins_get_result` applies the same two rules as the first wait**, measured from the moment it is called: offline text if the request is still undelivered after the offline threshold, pending text after the wait.
9. **Structured results use RFC 3339 dates in both `structuredContent` and the text block**, so the text block is exactly `serde_json::to_string(&structuredContent)`. `structuredContent` is always an object: `{"messages": [...]}` for search/read, `{"id", "thread_id"}` for send. No `outputSchema` is advertised (clients would validate against it; not worth the risk in the MVP).
10. **Phone-reported errors** (`RelayOutcome::Error`) reach the AI as `isError` text `The Reins device could not complete the request: <message>`; a `Denied` reason is not shown (the spec text is fixed).
11. **Every `/mcp` method requires a valid bearer token**, including `initialize`, `server/discover`, `ping` and notifications, because Claude only starts OAuth on a 401. Origin check (403) runs before auth.
12. **Allowed `Origin` values for `/mcp`:** the server's own `DOMAIN` origin, `https://claude.ai`, `https://claude.com`, `https://chatgpt.com`, `https://chat.openai.com`. Requests without `Origin` (server-to-server, as both vendors do) are allowed. No CORS preflight support for browser-hosted MCP clients (out of MVP scope; the upstream `Cors` fairing already answers `OPTIONS` with 200).
13. **Header validation is lenient about absence, strict about mismatch:** `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name` are compared with the body only when present (values in `=?base64?…?=` form are decoded first). A request is "modern" when its `params._meta["io.modelcontextprotocol/protocolVersion"]` is present, when the `MCP-Protocol-Version` header is `2026-07-28`, or when the method is `server/discover`; otherwise it is legacy. `ping` is answered in both eras.
14. **JSON-RPC errors on `/mcp` use HTTP 400 except unknown method (404) and auth (401)**; malformed JSON → `-32700`, batch arrays and non-2.0 envelopes → `-32600`, unknown tool name → `-32602`. Invalid tool *arguments* (unknown property, wrong type, failed `ToolCall::normalized()`) are an `isError: true` tool result, as contracts C require.
15. **Request bodies are read as raw `Data`** (never Rocket `Json<T>`/`Form<T>` guards, which answer 422 on parse failure) on `/mcp`, `/reins/oauth/*` and `/reins/api/*`.
16. **OAuth issuer = `CONFIG.domain()`** (the full DOMAIN, path included, no trailing slash); JWT `iss` = `"{domain_origin}|mcp"` like every other Vaultwarden issuer; MCP URL = `"{domain}/mcp"` canonicalized (lower-case scheme/host, no trailing slash). `/.well-known/*` is mounted at the server root (outside the DOMAIN path) and also answers the RFC 8414/9728 path-inserted variants (`/.well-known/oauth-authorization-server{domain_path}`, `/.well-known/oauth-protected-resource{domain_path}/mcp`).
17. **`resource` is optional** on authorize and token (some clients omit it); when present it must equal the canonical MCP URL, else `invalid_target`.
18. **Authorize page flow (zero JavaScript):** `GET /reins/oauth/authorize` validates the request and renders an email form; `POST /reins/oauth/authorize` (per-IP `check_limit_login`) starts the pairing and answers `303` to `GET /reins/oauth/authorize/wait?session=…`, which renders the code with `<meta http-equiv="refresh" content="2">` until the pairing resolves and then answers `303` to the client's `redirect_uri`. The off-site redirect therefore always follows a GET navigation, never a form POST, so upstream `AppHeaders`' CSP `form-action 'self'` cannot block it. Inline `<style>` is allowed by the upstream CSP (`style-src 'self' 'unsafe-inline'`). Pages are rendered by Rust functions with HTML escaping (`pages.rs`), not handlebars templates, so no upstream template registration changes.
19. **Unknown email, disabled user, or user without an approval device** get the identical wait page with a random code backed by a decoy pairing that nobody can answer; it expires like a real one.
20. **`client_host`** shown on the phone is the host of the validated `redirect_uri` (server-verified), e.g. `chatgpt.com`, `claude.ai`, `localhost`.
21. **Pairing choices:** three distinct values in `10..=99`, one equal to the browser code, in random order. **`client_name`/label sanitizing:** remove Unicode control characters, bidi controls (U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069) and zero-width characters (U+200B–U+200D, U+2060, U+FEFF), collapse whitespace, truncate to 64 characters; empty → `"Unknown client"` (name) / the sanitized client name (label).
22. **`PairingResponse` validation:** `approved: true` requires `chosen_code`, which must be one of the three choices (else 400 `bad_request`, pairing untouched); a valid choice that is not the browser code → 409 `wrong_code` and the pairing fails; `approved: false` → denied (`connection_id: null`).
23. **Rate limits (per client IP, upstream limiters):** email submission on the authorize page uses `crate::ratelimit::check_limit_login` (default burst 10 per 60 s); `POST /reins/oauth/register` uses `crate::ratelimit::check_limit_unauthenticated` (default burst 50 per 60 s), as upstream does for other unauthenticated endpoints. The page answers 429 with an HTML error page; `/register` answers 429 with an RFC 6749-style JSON error.
24. **DCR accepts only public clients** (`token_endpoint_auth_method` absent or `"none"`), requires `redirect_uris` (1..=10, each `https://…` or loopback `http://localhost|127.0.0.1|[::1]`), `grant_types` ⊆ {`authorization_code`,`refresh_token`}, `response_types` ⊆ {`code`}; `application_type` is accepted but not required (MCP says clients MUST send it, but rejecting a client that forgets it gains nothing).
25. **CIMD:** a `client_id` starting with `https://` is a metadata document URL. It is fetched with `crate::http_client::make_http_request` (SSRF guard, 10 s timeout), must answer 200 JSON ≤ 64 KiB from the same URL (redirects rejected by comparing the final URL), and its `client_id` field must equal the URL. Fetch failures are not cached.
26. **Redirect-URI matching:** exact string match against the registered list, except loopback redirect URIs (`http` + host `localhost`, `127.0.0.1` or `[::1]`), which match when scheme, host, path and query are equal and only the port differs (RFC 8252 §7.3).
27. **HTTP-level tests run against the real binary.** `rocket::local::asynchronous::Client` tests were evaluated and rejected: the `DbConn` guard needs a managed `DbPool`, which only `DbPool::from_config()` builds (reads the global `CONFIG`), and `CONFIG` is a process-global `LazyLock` that reads the environment once, cannot be reconfigured per test (`std::env::set_var` is `unsafe` in edition 2024 and the workspace forbids `unsafe_code`), exits the process (`exit(12)`) when validation fails, and is also read by the `AppHeaders` fairing. Instead `tests/reins_server/` (a Cargo integration-test target) spawns `env!("CARGO_BIN_EXE_vaultwarden")` per test with its own temp `DATA_FOLDER`, free port, `DOMAIN=http://127.0.0.1:<port>`, `REINS_ENABLED=true` and short relay timings, and talks HTTP with reqwest. A "phone" in these tests is a Vaultwarden login with `deviceType=0` (register via `/identity/accounts/register` with an arbitrary `masterPasswordHash` string, log in with the same string). Plan 3's `reins-e2e` harness remains the full end-to-end test with `reins-core`.
28. **Purge job:** a scheduled job (`REINS_PURGE_SCHEDULE`, default hourly `"0 25 * * * *"`) deletes expired refresh tokens and sweeps expired in-memory entries. In-memory maps also purge lazily on every insert.
29. **Disabled mode:** when `REINS_ENABLED=false` (default) the route vectors are empty, nothing is mounted, no job is scheduled; migrations still run (tables exist but stay empty).
30. **Config validation** (only when enabled): `DOMAIN` must be set explicitly; `DOMAIN` must be `https://` unless its host is loopback; `1 <= REINS_OFFLINE_SECS <= REINS_RELAY_WAIT_SECS <= 55`; `REINS_FCM_SERVICE_ACCOUNT` empty or an existing file that parses as a Google service-account JSON.
31. **FCM failures never fail a relay**: the push is spawned after the request is stored and errors are logged (`warn!`), because the phone can still pick the request up by long-polling. An FCM `404 UNREGISTERED`/`NOT_FOUND` clears the stored `fcm_token`.
32. **Approval-device registration (A1) sanitizes `fcm_token`**: trimmed, empty → `None`, longer than 4096 bytes or containing control characters → 400 `bad_request`.

## File Structure

```
Cargo.toml                                        modify: [dev-dependencies] tokio (macros, test-util)
.env.template                                     modify: REINS_* documentation block
migrations/sqlite/2026-09-28-000000_reins/{up,down}.sql      create
migrations/mysql/2026-09-28-000000_reins/{up,down}.sql       create
migrations/postgresql/2026-09-28-000000_reins/{up,down}.sql  create
crates/reins-proto/src/lib.rs                  modify: pub mod device
crates/reins-proto/src/device.rs               create: phone API DTOs + ApiError (contracts §A)
src/main.rs                                       modify: mount reins routes/catchers, purge job
src/config.rs                                     modify: `reins` config group + validate call
src/util.rs                                       modify: LOGGED_ROUTES += /reins, /mcp, /.well-known
src/auth.rs                                       modify: `#[path = "auth/reins.rs"] pub mod reins;`
src/auth/reins.rs                              create: MCP access-token claims, encode/decode with audience
src/db/schema.rs                                  modify: 4 tables + joinable + allow_tables
src/db/models/mod.rs                              modify: register 3 model files
src/db/models/reins_device.rs                  create: ReinsDevice
src/db/models/reins_client.rs                  create: ReinsClient (DCR)
src/db/models/reins_connection.rs              create: ReinsConnection, ReinsRefreshToken
src/api/mod.rs                                    modify: `pub mod reins;` + re-exports
src/api/reins/mod.rs                           create: wiring, settings, routes(), well_known_routes(), catchers(), purge job
src/api/reins/ttl.rs                           create: TtlMap (tokio-time TTL map with capacity)
src/api/reins/relay.rs                         create: RelayHub — requests, results, delivery, long-poll
src/api/reins/pairing.rs                       create: pairing invariants (choices, sanitize, validate) + PairingHub
src/api/reins/fcm.rs                           create: FCM HTTP v1 sender, service-account JWT, token cache
src/api/reins/device_api.rs                    create: A1–A8 handlers, ApiErr responder, 401 catcher
src/api/reins/oauth.rs                         create: pure OAuth: metadata, PKCE, redirect matching, client metadata, token requests
src/api/reins/pages.rs                         create: server-rendered HTML (escape, email form, wait, error)
src/api/reins/oauth_routes.rs                  create: well-known, register, CIMD, authorize/wait, token handlers
src/api/reins/mcp.rs                           create: pure JSON-RPC/MCP dual-era dispatch
src/api/reins/tools.rs                         create: tool schemas, argument mapping, result rendering
src/api/reins/mcp_routes.rs                    create: POST/GET/DELETE /mcp handler, bearer auth, relay wiring
tests/reins_server/main.rs                     create: integration test target (spawns the real binary)
tests/reins_server/harness.rs                  create: server process, accounts, phone + OAuth helpers
tests/reins_server/phone_api.rs                create: A1–A8 HTTP tests
tests/reins_server/oauth.rs                    create: metadata, DCR, authorize, token HTTP tests
tests/reins_server/mcp.rs                      create: MCP HTTP tests incl. relay timing
```

---
### Task 1: `reins-proto::device` — phone API DTOs

**Files:**
- Create: `crates/reins-proto/src/device.rs`
- Modify: `crates/reins-proto/src/lib.rs` (add `pub mod device;` after `pub mod gmail;`)

**Interfaces:**
- Consumes: `reins_proto::ids::ConnectionId`, `reins_proto::relay::RelayRequest`, `reins_proto::pairing::PairingRequest` (Plan 1).
- Produces (module `reins_proto::device`, all `Clone + Debug + PartialEq + Eq + Serialize + Deserialize`):
  - `pub const MAX_PENDING_WAIT_SECS: u32 = 25;`
  - `pub mod codes { NOT_APPROVAL_DEVICE, NOT_FOUND, ALREADY_ANSWERED, WRONG_CODE, BAD_VERSION, BAD_REQUEST, UNAUTHORIZED, INTERNAL: &str }`
  - `DeviceRegistration { fcm_token: Option<String> }` (A1 body; field defaults to `None`)
  - `DeviceRegistered { replaced_previous: bool }` (A1)
  - `Pending { requests: Vec<RelayRequest>, pairings: Vec<PairingRequest> }` + `fn is_empty(&self) -> bool` (A2)
  - `PairingResult { connection_id: Option<ConnectionId> }` (A6; `None` serializes as `null`)
  - `ConnectionInfo { id: ConnectionId, label: String, client_name: String, client_host: String, created_at: i64, last_used_at: Option<i64> }`, `Connections { connections: Vec<ConnectionInfo> }` (A7)
  - `ApiError { error: String, message: String }` + `fn new(error: &str, message: impl Into<String>) -> ApiError`

- [ ] **Step 1: Write the failing tests**

Create `crates/reins-proto/src/device.rs` with only the test module (the types come in Step 3):

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pairing::PairingRequest;

    #[test]
    fn registration_token_is_optional() {
        let r: DeviceRegistration = serde_json::from_value(json!({})).unwrap();
        assert_eq!(r.fcm_token, None);
        let r: DeviceRegistration = serde_json::from_value(json!({"fcm_token": "tok"})).unwrap();
        assert_eq!(r.fcm_token.as_deref(), Some("tok"));
        assert_eq!(
            serde_json::to_value(DeviceRegistered {
                replaced_previous: true
            })
            .unwrap(),
            json!({"replaced_previous": true})
        );
    }

    #[test]
    fn pending_wire_format() {
        let empty = Pending::default();
        assert!(empty.is_empty());
        assert_eq!(serde_json::to_value(&empty).unwrap(), json!({"requests": [], "pairings": []}));
        let p = Pending {
            requests: vec![],
            pairings: vec![PairingRequest {
                v: 1,
                id: "p1".into(),
                client_name: "ChatGPT".into(),
                client_host: "chatgpt.com".into(),
                choices: [12, 47, 83],
                created_at: 5,
            }],
        };
        assert!(!p.is_empty());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["pairings"][0]["choices"], json!([12, 47, 83]));
        assert_eq!(serde_json::from_value::<Pending>(v).unwrap(), p);
    }

    #[test]
    fn pairing_result_denied_is_null() {
        let denied = PairingResult {
            connection_id: None,
        };
        assert_eq!(serde_json::to_value(&denied).unwrap(), json!({"connection_id": null}));
        let ok = PairingResult {
            connection_id: Some("c1".into()),
        };
        assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"connection_id": "c1"}));
    }

    #[test]
    fn connections_wire_format() {
        let c = Connections {
            connections: vec![ConnectionInfo {
                id: "c1".into(),
                label: "Work ChatGPT".into(),
                client_name: "ChatGPT".into(),
                client_host: "chatgpt.com".into(),
                created_at: 10,
                last_used_at: None,
            }],
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(
            v,
            json!({"connections": [{"id": "c1", "label": "Work ChatGPT", "client_name": "ChatGPT",
                "client_host": "chatgpt.com", "created_at": 10, "last_used_at": null}]})
        );
        assert_eq!(serde_json::from_value::<Connections>(v).unwrap(), c);
    }

    #[test]
    fn api_error_wire_format() {
        let e = ApiError::new(codes::NOT_APPROVAL_DEVICE, "This device is not the approval device");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"error": "not_approval_device", "message": "This device is not the approval device"})
        );
        assert_eq!(codes::WRONG_CODE, "wrong_code");
        assert_eq!(MAX_PENDING_WAIT_SECS, 25);
    }
}
```

Add `pub mod device;` to `crates/reins-proto/src/lib.rs` so the module list reads:

```rust
pub mod device;
pub mod gmail;
pub mod ids;
pub mod pairing;
pub mod relay;
mod validate;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p reins-proto device`
Expected: FAIL to compile — `cannot find type DeviceRegistration in this scope` (and the other types).

- [ ] **Step 3: Implement the DTOs**

Insert above the test module in `crates/reins-proto/src/device.rs`:

```rust
//! Phone API bodies (`{domain}/reins/api/*`, contracts §A), shared by the
//! server and the phone core.

use serde::{Deserialize, Serialize};

use crate::ids::ConnectionId;
use crate::pairing::PairingRequest;
use crate::relay::RelayRequest;

/// Largest `wait` (seconds) honoured by `GET /pending`; larger values are clamped.
pub const MAX_PENDING_WAIT_SECS: u32 = 25;

/// Values of [`ApiError::error`].
pub mod codes {
    /// 403: the caller is not the user's registered approval device.
    pub const NOT_APPROVAL_DEVICE: &str = "not_approval_device";
    /// 404: unknown, expired, or belongs to another user.
    pub const NOT_FOUND: &str = "not_found";
    /// 409: the request or pairing was already answered.
    pub const ALREADY_ANSWERED: &str = "already_answered";
    /// 409: the chosen pairing code is not the one shown in the browser; the pairing is cancelled.
    pub const WRONG_CODE: &str = "wrong_code";
    /// 400: message `v` is not the supported protocol version.
    pub const BAD_VERSION: &str = "bad_version";
    /// 400: malformed body or invalid field.
    pub const BAD_REQUEST: &str = "bad_request";
    /// 401: missing or invalid Vaultwarden access token.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// 500: server-side failure (e.g. database); retry later.
    pub const INTERNAL: &str = "internal_error";
}

/// A1 `PUT /device` body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistration {
    #[serde(default)]
    pub fcm_token: Option<String>,
}

/// A1 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistered {
    /// True when another device was the approval device before this call.
    pub replaced_previous: bool,
}

/// A2 `GET /pending` response. Items returned here are marked delivered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    #[serde(default)]
    pub requests: Vec<RelayRequest>,
    #[serde(default)]
    pub pairings: Vec<PairingRequest>,
}

impl Pending {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.pairings.is_empty()
    }
}

/// A6 response; `connection_id` is `None` when the user denied the pairing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResult {
    pub connection_id: Option<ConnectionId>,
}

/// One authorized AI client, as listed by A7.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: ConnectionId,
    pub label: String,
    /// Client-declared name (sanitized by the server, still untrusted).
    pub client_name: String,
    /// Host of the redirect URI used when the connection was authorized.
    pub client_host: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds of the last MCP call, if any.
    pub last_used_at: Option<i64>,
}

/// A7 response.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connections {
    pub connections: Vec<ConnectionInfo>,
}

/// Error body of every phone API error response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// One of [`codes`].
    pub error: String,
    pub message: String,
}

impl ApiError {
    #[must_use]
    pub fn new(error: &str, message: impl Into<String>) -> Self {
        Self {
            error: error.to_owned(),
            message: message.into(),
        }
    }
}
```

- [ ] **Step 4: Run the tests and lints**

Run: `cargo test -p reins-proto && cargo clippy -p reins-proto --all-targets -- -D warnings && cargo fmt -p reins-proto --check`
Expected: all tests pass (5 new in `device::tests`), no clippy findings, fmt clean.

- [ ] **Step 5: Commit**

```bash
git add crates/reins-proto/src/device.rs crates/reins-proto/src/lib.rs
git commit -m "feat(proto): phone API device DTOs and ApiError"
```

---

### Task 2: `reins` config group, module skeleton and wiring

**Files:**
- Modify: `Cargo.toml` (new `[dev-dependencies]` table just before the `# Strip debuginfo from the release builds` comment / `[profile.release]`)
- Modify: `src/config.rs` (new group after the `push { … },` group ending at line 538; one validation call after the `if cfg.push_enabled { … }` block ending at line 1063)
- Modify: `src/api/mod.rs` (line 6: add `pub mod reins;` after `mod push;`)
- Modify: `src/main.rs:582-598` (mounts inside `launch_rocket`)
- Modify: `src/util.rs:300` (`LOGGED_ROUTES`)
- Modify: `.env.template` (new block between the push section, line 130, and `### Schedule jobs ###`)
- Create: `src/api/reins/mod.rs`

**Interfaces:**
- Consumes: `crate::CONFIG` getters generated by this task: `reins_enabled() -> bool`, `reins_fcm_service_account() -> String`, `reins_relay_wait_secs() -> u64`, `reins_offline_secs() -> u64`, `reins_purge_schedule() -> String`.
- Produces (`crate::api::reins`):
  - constants `ITEM_TTL: Duration` (600 s), `SESSION_TTL` (300 s), `CODE_TTL` (60 s), `CIMD_TTL` (3600 s), `ACCESS_TOKEN_SECS: i64 = 3600`, `REFRESH_TOKEN_SECS: i64 = 2_592_000`, `MAX_RELAY_WAIT_SECS: u64 = 55`
  - `struct Timing { relay_wait: Duration, offline: Duration }` + `Timing::from_config() -> Timing`
  - `fn enabled() -> bool`
  - `fn routes() -> Vec<Route>` (mounted at `{domain_path}/`; later tasks append their routes), `fn well_known_routes() -> Vec<Route>` (mounted at `/`), `fn catchers() -> Vec<Catcher>` (registered at `{domain_path}/reins/api`) — all empty when disabled
  - `fn validate_settings(domain: &str, domain_set: bool, wait_secs: u64, offline_secs: u64, fcm_path: &str) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Create `src/api/reins/mod.rs` with the module doc, the temporary lint allowance and the tests:

```rust
//! Reins server: MCP endpoint, OAuth 2.1 authorization server for AI clients,
//! phone API and in-memory relay (spec §4, contracts §A and §C).
// Modules are wired into routes incrementally by Plan 2; Task 14 removes this allowance.
#![allow(dead_code, reason = "Reins modules are wired incrementally; removed in Plan 2 Task 14")]

#[cfg(test)]
mod tests {
    use super::*;

    const OK_DOMAIN: &str = "https://reins.example.com";

    #[test]
    fn accepts_defaults() {
        assert_eq!(validate_settings(OK_DOMAIN, true, 45, 10, ""), Ok(()));
        assert_eq!(validate_settings("http://127.0.0.1:8000", true, 45, 10, ""), Ok(()));
        assert_eq!(validate_settings("http://localhost:8000/vw", true, 4, 2, ""), Ok(()));
        assert_eq!(validate_settings("http://[::1]:8000", true, 55, 55, ""), Ok(()));
    }

    #[test]
    fn requires_explicit_https_domain() {
        assert!(validate_settings(OK_DOMAIN, false, 45, 10, "").unwrap_err().contains("DOMAIN"));
        assert!(validate_settings("http://reins.example.com", true, 45, 10, "").unwrap_err().contains("https"));
        assert!(validate_settings("not a url", true, 45, 10, "").is_err());
    }

    #[test]
    fn timing_bounds() {
        assert!(validate_settings(OK_DOMAIN, true, 0, 0, "").unwrap_err().contains("REINS_RELAY_WAIT_SECS"));
        assert!(validate_settings(OK_DOMAIN, true, 56, 10, "").unwrap_err().contains("REINS_RELAY_WAIT_SECS"));
        assert!(validate_settings(OK_DOMAIN, true, 45, 0, "").unwrap_err().contains("REINS_OFFLINE_SECS"));
        assert!(validate_settings(OK_DOMAIN, true, 10, 11, "").unwrap_err().contains("REINS_OFFLINE_SECS"));
    }

    #[test]
    fn fcm_path_must_exist_when_set() {
        let missing = std::env::temp_dir().join("reins-missing-service-account.json");
        let err = validate_settings(OK_DOMAIN, true, 45, 10, missing.to_str().unwrap()).unwrap_err();
        assert!(err.contains("REINS_FCM_SERVICE_ACCOUNT"), "{err}");
    }

    #[test]
    fn ttl_constants_match_spec() {
        assert_eq!(ITEM_TTL.as_secs(), 600);
        assert_eq!(SESSION_TTL.as_secs(), 300);
        assert_eq!(CODE_TTL.as_secs(), 60);
        assert_eq!(CIMD_TTL.as_secs(), 3600);
        assert_eq!(ACCESS_TOKEN_SECS, 3600);
        assert_eq!(REFRESH_TOKEN_SECS, 30 * 24 * 3600);
    }
}
```

Register the module in `src/api/mod.rs` (lines 1-7 become):

```rust
mod admin;
pub mod core;
mod icons;
mod identity;
mod notifications;
mod push;
pub mod reins;
mod web;
```

Add the test-only tokio features to `Cargo.toml`, directly above the line `# Strip debuginfo from the release builds`:

```toml
[dev-dependencies]
# Paused-clock relay timing tests and #[tokio::test]
tokio = { version = "1.53.1", features = ["macros", "test-util"] }

```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --features sqlite api::reins`
Expected: FAIL to compile — `cannot find function validate_settings`, `cannot find value ITEM_TTL`.

- [ ] **Step 3: Add the config group**

In `src/config.rs`, directly after the `push { … },` group (after line 538 `    },`) insert:

```rust
    /// Reins AI permission relay
    reins {
        /// Enable Reins |> Mounts the MCP endpoint ({DOMAIN}/mcp), the OAuth server for AI clients and the phone API
        reins_enabled:               bool,   false,  def,    false;
        /// FCM service account |> Path to the Firebase service-account JSON used to wake the approval device. Empty disables push.
        reins_fcm_service_account:   String, false,  def,    String::new();
        /// Relay wait (seconds) |> How long an MCP tool call waits for the phone. Must be below ChatGPT's 60 s tool timeout (max 55).
        reins_relay_wait_secs:       u64,    false,  def,    45;
        /// Offline threshold (seconds) |> A request not fetched by the phone within this time is reported as "device offline".
        reins_offline_secs:          u64,    false,  def,    10;
        /// Purge schedule |> Cron schedule of the job deleting expired Reins refresh tokens and in-memory entries. Blank disables it.
        reins_purge_schedule:        String, false,  def,    "0 25 * * * *".to_owned();
    },
```

In `validate_config`, directly after the closing `}` of the `if cfg.push_enabled { … }` block (line 1063, before `let invalid_flags = …`) insert:

```rust
    if cfg.reins_enabled
        && let Err(e) = crate::api::reins::validate_settings(
            &cfg.domain,
            cfg.domain_set,
            cfg.reins_relay_wait_secs,
            cfg.reins_offline_secs,
            &cfg.reins_fcm_service_account,
        )
    {
        err!(e)
    }
```

- [ ] **Step 4: Implement the module skeleton**

In `src/api/reins/mod.rs`, between the `#![allow…]` line and the test module, insert:

```rust

use std::{path::Path, time::Duration};

use rocket::{Catcher, Route};

use crate::CONFIG;

/// Lifetime of relay requests, pairings and late results (contracts §A).
pub const ITEM_TTL: Duration = Duration::from_secs(600);
/// Lifetime of an authorize-page session (spec §4.1).
pub const SESSION_TTL: Duration = Duration::from_secs(300);
/// Lifetime of an authorization code (spec §4.5).
pub const CODE_TTL: Duration = Duration::from_secs(60);
/// How long a fetched CIMD document is trusted (spec §4.5).
pub const CIMD_TTL: Duration = Duration::from_secs(3600);
/// MCP access token lifetime (seconds).
pub const ACCESS_TOKEN_SECS: i64 = 3600;
/// MCP refresh token lifetime (seconds): 30 days.
pub const REFRESH_TOKEN_SECS: i64 = 30 * 24 * 3600;
/// Upper bound for `REINS_RELAY_WAIT_SECS` (ChatGPT's hard tool timeout is 60 s).
pub const MAX_RELAY_WAIT_SECS: u64 = 55;

/// Relay timing (spec §4.3). Tests construct it directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Maximum time a tool call waits for the phone's answer.
    pub relay_wait: Duration,
    /// A request not delivered to the phone within this time is reported as offline.
    pub offline: Duration,
}

impl Timing {
    pub fn from_config() -> Self {
        Self {
            relay_wait: Duration::from_secs(CONFIG.reins_relay_wait_secs()),
            offline: Duration::from_secs(CONFIG.reins_offline_secs()),
        }
    }
}

pub fn enabled() -> bool {
    CONFIG.reins_enabled()
}

/// Routes mounted at `{domain_path}/` (they carry their full paths: `/mcp`, `/reins/...`).
pub fn routes() -> Vec<Route> {
    if !enabled() {
        return Vec::new();
    }
    Vec::new()
}

/// OAuth/RFC 9728 discovery documents, mounted at the server root `/`.
pub fn well_known_routes() -> Vec<Route> {
    if !enabled() {
        return Vec::new();
    }
    Vec::new()
}

/// Catchers registered at `{domain_path}/reins/api`.
pub fn catchers() -> Vec<Catcher> {
    if !enabled() {
        return Vec::new();
    }
    Vec::new()
}

/// Cross-field checks for the `reins` config group; only called when Reins is enabled.
pub fn validate_settings(
    domain: &str,
    domain_set: bool,
    wait_secs: u64,
    offline_secs: u64,
    fcm_path: &str,
) -> Result<(), String> {
    if !domain_set {
        return Err("`REINS_ENABLED` requires `DOMAIN` to be set to the public URL of this server".to_owned());
    }
    let url = url::Url::parse(domain).map_err(|e| format!("`DOMAIN` is not a valid URL: {e}"))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("`REINS_ENABLED` requires an https:// `DOMAIN` (http is only allowed for localhost, 127.0.0.1 and [::1])"
            .to_owned());
    }
    if !(1..=MAX_RELAY_WAIT_SECS).contains(&wait_secs) {
        return Err(format!(
            "`REINS_RELAY_WAIT_SECS` must be between 1 and {MAX_RELAY_WAIT_SECS} (ChatGPT aborts tool calls after 60 s)"
        ));
    }
    if offline_secs == 0 || offline_secs > wait_secs {
        return Err("`REINS_OFFLINE_SECS` must be at least 1 and at most `REINS_RELAY_WAIT_SECS`".to_owned());
    }
    if !fcm_path.is_empty() && !Path::new(fcm_path).is_file() {
        return Err(format!("`REINS_FCM_SERVICE_ACCOUNT` file `{fcm_path}` does not exist"));
    }
    Ok(())
}
```

(`routes()`/`well_known_routes()`/`catchers()` return `Vec::new()` in both branches for now; Tasks 7, 10, 11 and 14 replace the second `Vec::new()` with their `routes![…]`/`catchers![…]`.)

- [ ] **Step 5: Mount the (still empty) route sets and log them**

In `src/main.rs` `launch_rocket`, after `.mount([basepath, "/notifications"].concat(), api::notifications_routes())` (line 589) insert:

```rust
        .mount([basepath, "/"].concat(), api::reins::routes())
        .mount("/", api::reins::well_known_routes())
```

and after `.register([basepath, "/admin"].concat(), api::admin_catchers())` (line 592) insert:

```rust
        .register([basepath, "/reins/api"].concat(), api::reins::catchers())
```

In `src/util.rs` replace line 300:

```rust
const LOGGED_ROUTES: [&str; 7] = ["/api", "/admin", "/identity", "/icons", "/attachments", "/events", "/notifications"];
```

with:

```rust
const LOGGED_ROUTES: [&str; 10] = [
    "/api",
    "/admin",
    "/identity",
    "/icons",
    "/attachments",
    "/events",
    "/notifications",
    "/reins",
    "/mcp",
    "/.well-known",
];
```

- [ ] **Step 6: Document the options**

In `.env.template`, after the line `# PUSH_IDENTITY_URI=https://identity.bitwarden.eu` (line 130) and its following blank line, insert:

```ini
################
### Reins ###
################

## Enables the Reins AI permission relay: the MCP endpoint ({DOMAIN}/mcp), the OAuth
## authorization server for AI clients (/reins/oauth/*, /.well-known/oauth-*) and the
## phone API (/reins/api/*). Requires DOMAIN (https://, or http:// on localhost/127.0.0.1).
# REINS_ENABLED=false

## Firebase service-account JSON used to wake the approval device through FCM HTTP v1.
## Leave empty to disable push; the Reins app then only receives requests while it is open.
# REINS_FCM_SERVICE_ACCOUNT=/path/to/fcm-service-account.json

## Seconds an MCP tool call waits for the phone before telling the AI to call
## reins_get_result later. Must be 1..=55 (ChatGPT aborts tool calls after 60 s).
# REINS_RELAY_WAIT_SECS=45

## Seconds after which a request the phone has not fetched is reported to the AI as "device offline".
## Must be 1..=REINS_RELAY_WAIT_SECS.
# REINS_OFFLINE_SECS=10

## Cron schedule of the job that deletes expired Reins refresh tokens and in-memory entries.
# REINS_PURGE_SCHEDULE="0 25 * * * *"

```

- [ ] **Step 7: Run the tests, lints and a startup check**

Run: `cargo test --features sqlite api::reins`
Expected: 5 passed.

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings && cargo clippy --features sqlite,mysql,postgresql -- -D warnings`
Expected: clean.

Run (startup rejects a bad config):

```bash
cargo build --features sqlite
D=$(mktemp -d) && (cd "$D" && DATA_FOLDER="$D" WEB_VAULT_ENABLED=false DOMAIN=http://127.0.0.1:18080 ROCKET_PORT=18080 \
  REINS_ENABLED=true REINS_RELAY_WAIT_SECS=90 <repo>/target/debug/vaultwarden; echo "exit=$?")
```

Expected: output contains ``REINS_RELAY_WAIT_SECS` must be between 1 and 55`` and `exit=12`.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/config.rs src/api/mod.rs src/api/reins/mod.rs src/main.rs src/util.rs .env.template
git commit -m "feat(reins): config group, module skeleton and route mounts"
```

---

### Task 3: Database tables, schema and models (sqlite, mysql, postgresql)

**Files:**
- Create: `migrations/sqlite/2026-09-28-000000_reins/up.sql`, `.../down.sql`
- Create: `migrations/mysql/2026-09-28-000000_reins/up.sql`, `.../down.sql`
- Create: `migrations/postgresql/2026-09-28-000000_reins/up.sql`, `.../down.sql`
- Modify: `src/db/schema.rs` (append four `table!` blocks after the `archives` table, before the first `joinable!`, line ~352)
- Create: `src/db/models/reins_device.rs`, `src/db/models/reins_client.rs`, `src/db/models/reins_connection.rs`
- Modify: `src/db/models/mod.rs` (register the three files)
- Modify: `Cargo.toml` (`[dependencies]`: add `reins-proto` after `macros = { path = "./macros" }`)

**Interfaces:**
- Consumes: `crate::db::{DbConn, DbConnInner}`, `crate::db::models::{UserId, DeviceId}`, `crate::api::EmptyResult`, `crate::error::MapResult`, `crate::util::get_uuid`, `reins_proto::device::ConnectionInfo`, `reins_proto::ids::ConnectionId`.
- Produces (`crate::db::models::*`):
  - `ReinsDevice { user_uuid: UserId, device_uuid: DeviceId, fcm_token: Option<String>, updated_at: i64 }`
    - `async fn find_by_user(user_uuid: &UserId, conn: &DbConn) -> Option<ReinsDevice>`
    - `async fn replace(&self, conn: &DbConn) -> Result<Option<ReinsDevice>, crate::Error>` (returns the previous row)
    - `async fn clear_fcm_token(user_uuid: &UserId, fcm_token: &str, conn: &DbConn) -> EmptyResult` (only if the stored token still equals `fcm_token`)
  - `ReinsClient { client_id: String, client_name: String, redirect_uris: String /* JSON array */, created_at: i64 }`
    - `fn new(client_name: String, redirect_uris: &[String], now: i64) -> ReinsClient` (fresh uuid `client_id`)
    - `fn redirect_uri_list(&self) -> Vec<String>`
    - `async fn save(&self, conn: &DbConn) -> EmptyResult`, `async fn find(client_id: &str, conn: &DbConn) -> Option<ReinsClient>`
  - `ReinsConnection { uuid: String, user_uuid: UserId, client_id: String, client_name: String, client_host: String, label: String, created_at: i64, last_used_at: Option<i64> }`
    - `fn new(user_uuid: UserId, client_id: String, client_name: String, client_host: String, label: String, now: i64) -> ReinsConnection`
    - `fn to_info(&self) -> reins_proto::device::ConnectionInfo`
    - `async fn save(&self, conn) -> EmptyResult`, `async fn find_by_uuid_and_user(uuid: &str, user_uuid: &UserId, conn) -> Option<Self>`, `async fn find_by_user(user_uuid: &UserId, conn) -> Vec<Self>` (oldest first), `async fn touch(uuid: &str, now: i64, conn) -> EmptyResult` (sets `last_used_at` at most once per 60 s), `async fn delete(&self, conn) -> EmptyResult` (also deletes its refresh tokens)
  - `ReinsRefreshToken { token_hash: String, connection_uuid: String, expires_at: i64 }`
    - `async fn save(&self, conn) -> EmptyResult`, `async fn take(token_hash: &str, now: i64, conn) -> Option<Self>` (single use: deletes the row; `None` if unknown or expired), `async fn delete_expired(now: i64, conn) -> EmptyResult`

- [ ] **Step 1: Write the migrations**

`migrations/sqlite/2026-09-28-000000_reins/up.sql`:

```sql
CREATE TABLE reins_devices (
    user_uuid   CHAR(36) NOT NULL PRIMARY KEY REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid CHAR(36) NOT NULL,
    fcm_token   TEXT,
    updated_at  BIGINT   NOT NULL
);

CREATE TABLE reins_clients (
    client_id     VARCHAR(64) NOT NULL PRIMARY KEY,
    client_name   TEXT        NOT NULL,
    redirect_uris TEXT        NOT NULL,
    created_at    BIGINT      NOT NULL
);

CREATE TABLE reins_connections (
    uuid         CHAR(36) NOT NULL PRIMARY KEY,
    user_uuid    CHAR(36) NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    client_id    TEXT     NOT NULL,
    client_name  TEXT     NOT NULL,
    client_host  TEXT     NOT NULL,
    label        TEXT     NOT NULL,
    created_at   BIGINT   NOT NULL,
    last_used_at BIGINT
);

CREATE INDEX idx_reins_connections_user ON reins_connections (user_uuid);

CREATE TABLE reins_refresh_tokens (
    token_hash      CHAR(64) NOT NULL PRIMARY KEY,
    connection_uuid CHAR(36) NOT NULL REFERENCES reins_connections (uuid) ON DELETE CASCADE,
    expires_at      BIGINT   NOT NULL
);

CREATE INDEX idx_reins_refresh_tokens_connection ON reins_refresh_tokens (connection_uuid);
```

`migrations/postgresql/2026-09-28-000000_reins/up.sql`: identical text to the SQLite file above (the same DDL is valid PostgreSQL).

`migrations/mysql/2026-09-28-000000_reins/up.sql`:

```sql
CREATE TABLE reins_devices (
    user_uuid   CHAR(36) NOT NULL PRIMARY KEY,
    device_uuid CHAR(36) NOT NULL,
    fcm_token   TEXT,
    updated_at  BIGINT   NOT NULL,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_clients (
    client_id     VARCHAR(64) NOT NULL PRIMARY KEY,
    client_name   TEXT        NOT NULL,
    redirect_uris TEXT        NOT NULL,
    created_at    BIGINT      NOT NULL
);

CREATE TABLE reins_connections (
    uuid         CHAR(36) NOT NULL PRIMARY KEY,
    user_uuid    CHAR(36) NOT NULL,
    client_id    TEXT     NOT NULL,
    client_name  TEXT     NOT NULL,
    client_host  TEXT     NOT NULL,
    label        TEXT     NOT NULL,
    created_at   BIGINT   NOT NULL,
    last_used_at BIGINT,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_connections_user ON reins_connections (user_uuid);

CREATE TABLE reins_refresh_tokens (
    token_hash      CHAR(64) NOT NULL PRIMARY KEY,
    connection_uuid CHAR(36) NOT NULL,
    expires_at      BIGINT   NOT NULL,
    FOREIGN KEY (connection_uuid) REFERENCES reins_connections (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_refresh_tokens_connection ON reins_refresh_tokens (connection_uuid);
```

`down.sql` for all three backends (identical):

```sql
DROP TABLE IF EXISTS reins_refresh_tokens;
DROP TABLE IF EXISTS reins_connections;
DROP TABLE IF EXISTS reins_clients;
DROP TABLE IF EXISTS reins_devices;
```

- [ ] **Step 2: Add the schema**

In `src/db/schema.rs`, after the `archives` `table! { … }` block and before `joinable!(archives -> users (user_uuid));`, insert (no joins are used, so no `joinable!`/`allow_tables_to_appear_in_same_query!` entries are needed):

```rust
table! {
    reins_devices (user_uuid) {
        user_uuid -> Text,
        device_uuid -> Text,
        fcm_token -> Nullable<Text>,
        updated_at -> BigInt,
    }
}

table! {
    reins_clients (client_id) {
        client_id -> Text,
        client_name -> Text,
        redirect_uris -> Text,
        created_at -> BigInt,
    }
}

table! {
    reins_connections (uuid) {
        uuid -> Text,
        user_uuid -> Text,
        client_id -> Text,
        client_name -> Text,
        client_host -> Text,
        label -> Text,
        created_at -> BigInt,
        last_used_at -> Nullable<BigInt>,
    }
}

table! {
    reins_refresh_tokens (token_hash) {
        token_hash -> Text,
        connection_uuid -> Text,
        expires_at -> BigInt,
    }
}
```

Add the proto crate to the root `Cargo.toml` `[dependencies]`, directly after `macros = { path = "./macros" }`:

```toml
reins-proto = { path = "crates/reins-proto" }
```

- [ ] **Step 3: Write the failing model tests**

Each model file starts with its test module; the tests use an in-memory SQLite database migrated with the real embedded migrations (`crate::db::sqlite_migrations::MIGRATIONS` is private to `crate::db`, and these files are descendants of it, so they may use it). Query logic lives in synchronous `fn q_*(c: &mut DbConnInner, …)` functions that the async `DbConn` wrappers call, so tests call the `q_*` functions directly.

`src/db/models/reins_device.rs` (test part):

```rust
#[cfg(all(test, sqlite))]
pub(super) fn test_db() -> DbConnInner {
    use diesel::Connection;
    use diesel_migrations::MigrationHarness;
    let mut c = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    c.run_pending_migrations(crate::db::sqlite_migrations::MIGRATIONS).unwrap();
    DbConnInner::Sqlite(c)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;

    fn device(user: &str, dev: &str, token: Option<&str>) -> ReinsDevice {
        ReinsDevice {
            user_uuid: UserId::from(user.to_owned()),
            device_uuid: DeviceId::from(dev.to_owned()),
            fcm_token: token.map(str::to_owned),
            updated_at: 1,
        }
    }

    #[test]
    fn replace_returns_previous_device() {
        let mut c = test_db();
        assert!(q_find_by_user(&mut c, &UserId::from("u1".to_owned())).is_none());
        assert!(q_replace(&mut c, &device("u1", "d1", Some("t1"))).unwrap().is_none());
        let prev = q_replace(&mut c, &device("u1", "d2", None)).unwrap().unwrap();
        assert_eq!(prev.device_uuid, DeviceId::from("d1".to_owned()));
        assert_eq!(prev.fcm_token.as_deref(), Some("t1"));
        let now = q_find_by_user(&mut c, &UserId::from("u1".to_owned())).unwrap();
        assert_eq!(now.device_uuid, DeviceId::from("d2".to_owned()));
        assert_eq!(now.fcm_token, None);
    }

    #[test]
    fn devices_are_per_user_and_token_can_be_cleared() {
        let mut c = test_db();
        q_replace(&mut c, &device("u1", "d1", Some("t1"))).unwrap();
        q_replace(&mut c, &device("u2", "d9", Some("t9"))).unwrap();
        q_clear_fcm_token(&mut c, &UserId::from("u1".to_owned()), "stale").unwrap();
        assert_eq!(q_find_by_user(&mut c, &UserId::from("u1".to_owned())).unwrap().fcm_token.as_deref(), Some("t1"));
        q_clear_fcm_token(&mut c, &UserId::from("u1".to_owned()), "t1").unwrap();
        assert_eq!(q_find_by_user(&mut c, &UserId::from("u1".to_owned())).unwrap().fcm_token, None);
        assert_eq!(q_find_by_user(&mut c, &UserId::from("u2".to_owned())).unwrap().fcm_token.as_deref(), Some("t9"));
    }
}
```

`src/db/models/reins_client.rs` (test part):

```rust
#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    #[test]
    fn client_round_trips_redirect_uris() {
        let mut c = test_db();
        let uris = vec!["https://claude.ai/api/mcp/auth_callback".to_owned(), "http://localhost/callback".to_owned()];
        let client = ReinsClient::new("Claude".to_owned(), &uris, 7);
        assert_eq!(client.client_id.len(), 36);
        q_save(&mut c, &client).unwrap();
        let found = q_find(&mut c, &client.client_id).unwrap();
        assert_eq!(found.client_name, "Claude");
        assert_eq!(found.redirect_uri_list(), uris);
        assert!(q_find(&mut c, "nope").is_none());
    }

    #[test]
    fn corrupt_redirect_uris_yield_empty_list() {
        let client = ReinsClient {
            client_id: "x".to_owned(),
            client_name: "x".to_owned(),
            redirect_uris: "not json".to_owned(),
            created_at: 0,
        };
        assert!(client.redirect_uri_list().is_empty());
    }
}
```

`src/db/models/reins_connection.rs` (test part):

```rust
#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    fn conn_for(user: &str, now: i64) -> ReinsConnection {
        ReinsConnection::new(
            UserId::from(user.to_owned()),
            "https://chatgpt.com/oauth/client.json".to_owned(),
            "ChatGPT".to_owned(),
            "chatgpt.com".to_owned(),
            "My ChatGPT".to_owned(),
            now,
        )
    }

    fn token(hash: &str, conn: &ReinsConnection, expires_at: i64) -> ReinsRefreshToken {
        ReinsRefreshToken {
            token_hash: hash.to_owned(),
            connection_uuid: conn.uuid.clone(),
            expires_at,
        }
    }

    #[test]
    fn connections_are_scoped_to_their_user() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        let b = conn_for("u1", 20);
        q_save(&mut c, &a).unwrap();
        q_save(&mut c, &b).unwrap();
        q_save(&mut c, &conn_for("u2", 5)).unwrap();
        let mine: Vec<String> = q_find_by_user(&mut c, &UserId::from("u1".to_owned())).into_iter().map(|x| x.uuid).collect();
        assert_eq!(mine, vec![a.uuid.clone(), b.uuid.clone()]);
        assert!(q_find_by_uuid_and_user(&mut c, &a.uuid, &UserId::from("u1".to_owned())).is_some());
        assert!(q_find_by_uuid_and_user(&mut c, &a.uuid, &UserId::from("u2".to_owned())).is_none());
    }

    #[test]
    fn to_info_maps_fields() {
        let a = conn_for("u1", 10);
        let info = a.to_info();
        assert_eq!(info.id.0, a.uuid);
        assert_eq!(info.label, "My ChatGPT");
        assert_eq!(info.client_host, "chatgpt.com");
        assert_eq!((info.created_at, info.last_used_at), (10, None));
    }

    #[test]
    fn touch_is_throttled_to_once_a_minute() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        let user = UserId::from("u1".to_owned());
        q_touch(&mut c, &a.uuid, 100).unwrap();
        assert_eq!(q_find_by_uuid_and_user(&mut c, &a.uuid, &user).unwrap().last_used_at, Some(100));
        q_touch(&mut c, &a.uuid, 159).unwrap();
        assert_eq!(q_find_by_uuid_and_user(&mut c, &a.uuid, &user).unwrap().last_used_at, Some(100));
        q_touch(&mut c, &a.uuid, 161).unwrap();
        assert_eq!(q_find_by_uuid_and_user(&mut c, &a.uuid, &user).unwrap().last_used_at, Some(161));
    }

    #[test]
    fn refresh_tokens_are_single_use_and_expire() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        q_save_token(&mut c, &token("h1", &a, 1000)).unwrap();
        q_save_token(&mut c, &token("h2", &a, 50)).unwrap();
        assert_eq!(q_take_token(&mut c, "h1", 999).unwrap().connection_uuid, a.uuid);
        assert!(q_take_token(&mut c, "h1", 999).is_none(), "second use must fail");
        assert!(q_take_token(&mut c, "h2", 50).is_none(), "expired at exactly expires_at");
        assert!(q_take_token(&mut c, "h2", 10).is_none(), "an expired take still consumed the row");
        assert!(q_take_token(&mut c, "unknown", 0).is_none());
    }

    #[test]
    fn deleting_a_connection_deletes_its_refresh_tokens() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        let b = conn_for("u1", 11);
        q_save(&mut c, &a).unwrap();
        q_save(&mut c, &b).unwrap();
        q_save_token(&mut c, &token("ha", &a, 1000)).unwrap();
        q_save_token(&mut c, &token("hb", &b, 1000)).unwrap();
        q_delete(&mut c, &a.uuid).unwrap();
        assert!(q_find_by_uuid_and_user(&mut c, &a.uuid, &UserId::from("u1".to_owned())).is_none());
        assert!(q_take_token(&mut c, "ha", 0).is_none());
        assert!(q_take_token(&mut c, "hb", 0).is_some());
    }

    #[test]
    fn delete_expired_keeps_live_tokens() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        q_save_token(&mut c, &token("old", &a, 100)).unwrap();
        q_save_token(&mut c, &token("new", &a, 300)).unwrap();
        q_delete_expired_tokens(&mut c, 200).unwrap();
        assert!(q_take_token(&mut c, "old", 0).is_none());
        assert!(q_take_token(&mut c, "new", 0).is_some());
    }
}
```

Register the files in `src/db/models/mod.rs`: after `mod org_policy;`/`mod organization;` keep alphabetical order and add

```rust
#[allow(dead_code, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
mod reins_client;
#[allow(dead_code, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
mod reins_connection;
#[allow(dead_code, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
mod reins_device;
```

between `mod organization;` and `mod send;`, and after `pub use self::org_policy::{…};`/`pub use self::organization::{…};` add

```rust
#[allow(unused_imports, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
pub use self::reins_client::ReinsClient;
#[allow(unused_imports, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
pub use self::reins_connection::{ReinsConnection, ReinsRefreshToken};
#[allow(unused_imports, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
pub use self::reins_device::ReinsDevice;
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test --features sqlite db::models::reins`
Expected: FAIL to compile — `cannot find type ReinsDevice`, `cannot find function q_replace`, etc.

- [ ] **Step 5: Implement the models**

Top of `src/db/models/reins_device.rs` (above the test code):

```rust
use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{DbConn, DbConnInner, schema::reins_devices},
    error::MapResult,
};

use super::{DeviceId, UserId};

/// The user's approval device: exactly one per user (spec §4.1, contracts A1).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_devices)]
#[diesel(primary_key(user_uuid))]
pub struct ReinsDevice {
    pub user_uuid: UserId,
    pub device_uuid: DeviceId,
    pub fcm_token: Option<String>,
    /// Unix seconds.
    pub updated_at: i64,
}

impl ReinsDevice {
    pub async fn find_by_user(user_uuid: &UserId, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_by_user(c, user_uuid)).await
    }

    /// Makes `self` the user's approval device and returns the device it replaced.
    pub async fn replace(&self, conn: &DbConn) -> Result<Option<Self>, crate::Error> {
        conn.run(move |c| q_replace(c, self)).await.map_res("Error saving Reins device")
    }

    /// Forgets `fcm_token` if it is still the stored token (FCM reported it unregistered).
    pub async fn clear_fcm_token(user_uuid: &UserId, fcm_token: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_clear_fcm_token(c, user_uuid, fcm_token)).await.map_res("Error clearing Reins FCM token")
    }
}

fn q_find_by_user(c: &mut DbConnInner, user_uuid: &UserId) -> Option<ReinsDevice> {
    reins_devices::table.filter(reins_devices::user_uuid.eq(user_uuid)).first::<ReinsDevice>(c).ok()
}

fn q_replace(c: &mut DbConnInner, row: &ReinsDevice) -> QueryResult<Option<ReinsDevice>> {
    c.transaction(|c| {
        let previous = reins_devices::table
            .filter(reins_devices::user_uuid.eq(&row.user_uuid))
            .first::<ReinsDevice>(c)
            .optional()?;
        diesel::delete(reins_devices::table.filter(reins_devices::user_uuid.eq(&row.user_uuid))).execute(c)?;
        diesel::insert_into(reins_devices::table).values(row).execute(c)?;
        Ok(previous)
    })
}

fn q_clear_fcm_token(c: &mut DbConnInner, user_uuid: &UserId, fcm_token: &str) -> QueryResult<()> {
    diesel::update(
        reins_devices::table
            .filter(reins_devices::user_uuid.eq(user_uuid))
            .filter(reins_devices::fcm_token.eq(fcm_token)),
    )
    .set(reins_devices::fcm_token.eq(None::<String>))
    .execute(c)
    .map(|_| ())
}
```

Top of `src/db/models/reins_client.rs`:

```rust
use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{DbConn, DbConnInner, schema::reins_clients},
    error::MapResult,
    util::get_uuid,
};

/// An MCP client registered through Dynamic Client Registration (public client).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_clients)]
#[diesel(primary_key(client_id))]
pub struct ReinsClient {
    pub client_id: String,
    pub client_name: String,
    /// JSON array of exact redirect URIs.
    pub redirect_uris: String,
    /// Unix seconds.
    pub created_at: i64,
}

impl ReinsClient {
    pub fn new(client_name: String, redirect_uris: &[String], now: i64) -> Self {
        Self {
            client_id: get_uuid(),
            client_name,
            redirect_uris: serde_json::to_string(redirect_uris).expect("a list of strings always serializes"),
            created_at: now,
        }
    }

    /// Registered redirect URIs; a corrupt column yields an empty list (matches nothing).
    pub fn redirect_uri_list(&self) -> Vec<String> {
        serde_json::from_str(&self.redirect_uris).unwrap_or_default()
    }

    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save(c, self)).await.map_res("Error saving Reins client")
    }

    pub async fn find(client_id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find(c, client_id)).await
    }
}

fn q_save(c: &mut DbConnInner, row: &ReinsClient) -> QueryResult<()> {
    diesel::insert_into(reins_clients::table).values(row).execute(c).map(|_| ())
}

fn q_find(c: &mut DbConnInner, client_id: &str) -> Option<ReinsClient> {
    reins_clients::table.filter(reins_clients::client_id.eq(client_id)).first::<ReinsClient>(c).ok()
}
```

Top of `src/db/models/reins_connection.rs`:

```rust
use diesel::prelude::*;
use reins_proto::{device::ConnectionInfo, ids::ConnectionId};

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{reins_connections, reins_refresh_tokens},
    },
    error::MapResult,
    util::get_uuid,
};

use super::UserId;

/// `last_used_at` is written at most once per this many seconds.
const TOUCH_INTERVAL_SECS: i64 = 60;

/// An AI client authorized by a user (one per completed OAuth pairing).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_connections)]
#[diesel(primary_key(uuid))]
pub struct ReinsConnection {
    pub uuid: String,
    pub user_uuid: UserId,
    pub client_id: String,
    pub client_name: String,
    pub client_host: String,
    pub label: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// An MCP refresh token, stored only as its SHA-256 hex digest.
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_refresh_tokens)]
#[diesel(primary_key(token_hash))]
pub struct ReinsRefreshToken {
    pub token_hash: String,
    pub connection_uuid: String,
    /// Unix seconds; the token is dead at `now >= expires_at`.
    pub expires_at: i64,
}

impl ReinsConnection {
    pub fn new(
        user_uuid: UserId,
        client_id: String,
        client_name: String,
        client_host: String,
        label: String,
        now: i64,
    ) -> Self {
        Self {
            uuid: get_uuid(),
            user_uuid,
            client_id,
            client_name,
            client_host,
            label,
            created_at: now,
            last_used_at: None,
        }
    }

    pub fn to_info(&self) -> ConnectionInfo {
        ConnectionInfo {
            id: ConnectionId(self.uuid.clone()),
            label: self.label.clone(),
            client_name: self.client_name.clone(),
            client_host: self.client_host.clone(),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        }
    }

    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save(c, self)).await.map_res("Error saving Reins connection")
    }

    pub async fn find_by_uuid_and_user(uuid: &str, user_uuid: &UserId, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_by_uuid_and_user(c, uuid, user_uuid)).await
    }

    pub async fn find_by_user(user_uuid: &UserId, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_find_by_user(c, user_uuid)).await
    }

    pub async fn touch(uuid: &str, now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_touch(c, uuid, now)).await.map_res("Error updating Reins connection")
    }

    /// Deletes the connection and its refresh tokens (SQLite does not enforce the FK cascade).
    pub async fn delete(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete(c, &self.uuid)).await.map_res("Error deleting Reins connection")
    }
}

impl ReinsRefreshToken {
    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save_token(c, self)).await.map_res("Error saving Reins refresh token")
    }

    /// Consumes the token: deletes it and returns it only if it existed and was still valid.
    pub async fn take(token_hash: &str, now: i64, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_take_token(c, token_hash, now)).await
    }

    pub async fn delete_expired(now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete_expired_tokens(c, now)).await.map_res("Error purging Reins refresh tokens")
    }
}

fn q_save(c: &mut DbConnInner, row: &ReinsConnection) -> QueryResult<()> {
    diesel::insert_into(reins_connections::table).values(row).execute(c).map(|_| ())
}

fn q_find_by_uuid_and_user(c: &mut DbConnInner, uuid: &str, user_uuid: &UserId) -> Option<ReinsConnection> {
    reins_connections::table
        .filter(reins_connections::uuid.eq(uuid))
        .filter(reins_connections::user_uuid.eq(user_uuid))
        .first::<ReinsConnection>(c)
        .ok()
}

fn q_find_by_user(c: &mut DbConnInner, user_uuid: &UserId) -> Vec<ReinsConnection> {
    reins_connections::table
        .filter(reins_connections::user_uuid.eq(user_uuid))
        .order((reins_connections::created_at.asc(), reins_connections::uuid.asc()))
        .load::<ReinsConnection>(c)
        .unwrap_or_default()
}

fn q_touch(c: &mut DbConnInner, uuid: &str, now: i64) -> QueryResult<()> {
    diesel::update(
        reins_connections::table.filter(reins_connections::uuid.eq(uuid)).filter(
            reins_connections::last_used_at
                .is_null()
                .or(reins_connections::last_used_at.lt(now - TOUCH_INTERVAL_SECS)),
        ),
    )
    .set(reins_connections::last_used_at.eq(Some(now)))
    .execute(c)
    .map(|_| ())
}

fn q_delete(c: &mut DbConnInner, uuid: &str) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(reins_refresh_tokens::table.filter(reins_refresh_tokens::connection_uuid.eq(uuid)))
            .execute(c)?;
        diesel::delete(reins_connections::table.filter(reins_connections::uuid.eq(uuid))).execute(c)?;
        Ok(())
    })
}

fn q_save_token(c: &mut DbConnInner, row: &ReinsRefreshToken) -> QueryResult<()> {
    diesel::insert_into(reins_refresh_tokens::table).values(row).execute(c).map(|_| ())
}

fn q_take_token(c: &mut DbConnInner, token_hash: &str, now: i64) -> Option<ReinsRefreshToken> {
    let row = reins_refresh_tokens::table
        .filter(reins_refresh_tokens::token_hash.eq(token_hash))
        .first::<ReinsRefreshToken>(c)
        .ok()?;
    // Only the caller whose DELETE removed the row may use it (concurrent refreshes race here).
    let deleted = diesel::delete(reins_refresh_tokens::table.filter(reins_refresh_tokens::token_hash.eq(token_hash)))
        .execute(c)
        .ok()?;
    (deleted == 1 && now < row.expires_at).then_some(row)
}

fn q_delete_expired_tokens(c: &mut DbConnInner, now: i64) -> QueryResult<()> {
    diesel::delete(reins_refresh_tokens::table.filter(reins_refresh_tokens::expires_at.le(now)))
        .execute(c)
        .map(|_| ())
}
```

If the compiler rejects `.eq(user_uuid)` on a `&UserId` inside a `move` closure because of the borrowed parameter's lifetime, keep the pattern used upstream (e.g. `Archive::find_by_user`): the closure captures the reference and `DbConn::run` does not require `'static` captures.

- [ ] **Step 6: Run the tests and lints**

Run: `cargo test --features sqlite db::models::reins`
Expected: 10 passed.

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings && cargo clippy --features sqlite,mysql,postgresql -- -D warnings`
Expected: clean (the mysql/postgresql build proves the models compile for every backend).

Run (migration applies on a real server start, SQLite):

```bash
cargo build --features sqlite
D=$(mktemp -d) && (cd "$D" && DATA_FOLDER="$D" WEB_VAULT_ENABLED=false ROCKET_PORT=18081 \
  timeout 8 <repo>/target/debug/vaultwarden >/dev/null 2>&1; \
  sqlite3 "$D/db.sqlite3" ".tables" | tr -s ' ' '\n' | grep reins_)
```

Expected: `reins_clients`, `reins_connections`, `reins_devices`, `reins_refresh_tokens` listed.

- [ ] **Step 7: Commit**

```bash
git add migrations/sqlite/2026-09-28-000000_reins migrations/mysql/2026-09-28-000000_reins \
  migrations/postgresql/2026-09-28-000000_reins src/db/schema.rs src/db/models/mod.rs \
  src/db/models/reins_device.rs src/db/models/reins_client.rs src/db/models/reins_connection.rs \
  Cargo.toml Cargo.lock
git commit -m "feat(reins): database tables and models for devices, clients, connections, refresh tokens"
```

---

### Task 4: `TtlMap` and the relay hub (requests, delivery, results, timing)

**Files:**
- Create: `src/api/reins/ttl.rs`
- Create: `src/api/reins/relay.rs`
- Modify: `src/api/reins/mod.rs` (add `pub mod relay;` and `pub mod ttl;` below the `#![allow…]` line)

**Interfaces:**
- Consumes: `super::{Timing, ITEM_TTL}` (Task 2), `crate::util::get_uuid() -> String`, `reins_proto::{PROTOCOL_VERSION, gmail::ToolCall, ids::{ConnectionId, RequestId}, relay::{RelayOutcome, RelayRequest}}`.
- Produces:
  - `ttl::TtlMap<K: Eq + Hash, V>`: `new(ttl: Duration, capacity: usize)`, `insert(&mut self, K, V) -> Result<(), ttl::Full>`, `get(&self, &K) -> Option<&V>`, `get_mut(&mut self, &K) -> Option<&mut V>`, `remove(&mut self, &K) -> Option<V>`, `values(&self) -> impl Iterator<Item = &V>`, `purge(&mut self)`, `len(&self) -> usize`, `is_empty(&self) -> bool`; `ttl::Full` (unit struct)
  - `relay::ItemSignal`: `new() -> ItemSignal`, `notify(&self)`, `subscribe(&self) -> tokio::sync::watch::Receiver<u64>`
  - `relay::RelayHub`: `new(timing: Timing, signal: Arc<ItemSignal>) -> RelayHub`, `with_capacity(timing, signal, capacity: usize)`,
    `submit(&self, user: &str, connection_id: &ConnectionId, connection_label: &str, call: ToolCall, now_unix: i64) -> Result<RelayRequest, Full>`,
    `take_undelivered(&self, user: &str) -> Vec<RelayRequest>` (marks delivered, oldest first),
    `fetch(&self, user: &str, id: &RequestId) -> Option<RelayRequest>` (marks delivered),
    `answer(&self, user: &str, id: &RequestId, outcome: RelayOutcome) -> Result<(), AnswerError>`,
    `async wait(&self, id: &RequestId, connection_id: &ConnectionId) -> WaitResult`, `purge(&self)`
  - `relay::WaitResult { Answered(RelayOutcome), Offline, Pending, NotFound }`, `relay::AnswerError { NotFound, AlreadyAnswered }`, `relay::MAX_REQUESTS: usize = 10_000`

- [ ] **Step 1: Write the failing tests**

`src/api/reins/ttl.rs` — tests first:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn entries_expire_exactly_at_ttl() {
        let mut m = TtlMap::new(Duration::from_secs(60), 10);
        m.insert("a", 1).unwrap();
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(m.get(&"a"), Some(&1));
        assert_eq!(m.len(), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(m.get(&"a"), None);
        assert!(m.is_empty());
        assert_eq!(m.remove(&"a"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn capacity_counts_only_live_entries() {
        let mut m = TtlMap::new(Duration::from_secs(10), 2);
        m.insert("a", 1).unwrap();
        m.insert("b", 2).unwrap();
        assert_eq!(m.insert("c", 3), Err(Full));
        m.insert("a", 10).unwrap();
        assert_eq!(m.get(&"a"), Some(&10));
        tokio::time::advance(Duration::from_secs(10)).await;
        m.insert("c", 3).unwrap();
        assert_eq!(m.values().copied().collect::<Vec<_>>(), vec![3]);
    }

    #[tokio::test(start_paused = true)]
    async fn remove_returns_a_live_value_once() {
        let mut m = TtlMap::new(Duration::from_secs(10), 2);
        m.insert("a", 1).unwrap();
        *m.get_mut(&"a").unwrap() += 1;
        assert_eq!(m.remove(&"a"), Some(2));
        assert_eq!(m.remove(&"a"), None);
        m.insert("b", 1).unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(m.get_mut(&"b").is_none());
    }
}
```

`src/api/reins/relay.rs` — tests first:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reins_proto::relay::ToolResult;
    use tokio::time::{Instant, sleep};

    use super::*;

    const USER: &str = "user-1";

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn hub() -> RelayHub {
        RelayHub::new(
            Timing {
                relay_wait: secs(45),
                offline: secs(10),
            },
            Arc::new(ItemSignal::new()),
        )
    }

    fn conn() -> ConnectionId {
        "conn-1".into()
    }

    fn search() -> ToolCall {
        ToolCall::GmailSearch {
            query: "from:bank".to_owned(),
            max_results: 10,
        }
    }

    fn found() -> RelayOutcome {
        RelayOutcome::Result {
            result: ToolResult::Search {
                messages: vec![],
            },
        }
    }

    #[tokio::test(start_paused = true)]
    async fn undelivered_request_is_offline_after_the_threshold_not_the_full_wait() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        let start = Instant::now();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        assert_eq!(start.elapsed(), secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn delivered_but_undecided_is_pending_after_the_wait() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        let start = Instant::now();
        let (res, ()) = tokio::join!(h.wait(&req.id, &conn()), async {
            sleep(secs(3)).await;
            assert!(h.fetch(USER, &req.id).is_some());
        });
        assert_eq!(res, WaitResult::Pending);
        assert_eq!(start.elapsed(), secs(45));
    }

    #[tokio::test(start_paused = true)]
    async fn answer_within_the_wait_is_returned_immediately() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        let start = Instant::now();
        let (res, ()) = tokio::join!(h.wait(&req.id, &conn()), async {
            sleep(secs(2)).await;
            assert_eq!(h.take_undelivered(USER).len(), 1);
            sleep(secs(5)).await;
            h.answer(USER, &req.id, found()).unwrap();
        });
        assert_eq!(res, WaitResult::Answered(found()));
        assert_eq!(start.elapsed(), secs(7));
    }

    #[tokio::test(start_paused = true)]
    async fn late_result_is_readable_by_the_same_connection_only() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        let (first, ()) = tokio::join!(h.wait(&req.id, &conn()), async {
            sleep(secs(1)).await;
            h.fetch(USER, &req.id).unwrap();
        });
        assert_eq!(first, WaitResult::Pending);
        sleep(secs(5)).await;
        h.answer(USER, &req.id, found()).unwrap();
        let start = Instant::now();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Answered(found()));
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Answered(found()), "idempotent until expiry");
        assert_eq!(h.wait(&req.id, &"conn-2".into()).await, WaitResult::NotFound);
    }

    #[tokio::test(start_paused = true)]
    async fn get_result_on_a_still_undelivered_request_is_offline_again() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        let start = Instant::now();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        assert_eq!(start.elapsed(), secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn take_undelivered_is_per_user_ordered_and_one_shot() {
        let h = hub();
        let a = h.submit(USER, &conn(), "ChatGPT", search(), 5).unwrap();
        let b = h.submit(USER, &conn(), "ChatGPT", search(), 1).unwrap();
        h.submit("user-2", &conn(), "ChatGPT", search(), 0).unwrap();
        let got: Vec<RequestId> = h.take_undelivered(USER).into_iter().map(|r| r.id).collect();
        assert_eq!(got, vec![b.id, a.id]);
        assert!(h.take_undelivered(USER).is_empty());
        assert_eq!(h.take_undelivered("user-2").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn answers_are_checked_for_owner_and_accepted_once() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        assert!(h.fetch("user-2", &req.id).is_none());
        assert_eq!(h.answer("user-2", &req.id, found()), Err(AnswerError::NotFound));
        assert_eq!(h.answer(USER, &"nope".into(), found()), Err(AnswerError::NotFound));
        h.answer(USER, &req.id, found()).unwrap();
        let denied = RelayOutcome::Denied {
            reason: None,
        };
        assert_eq!(h.answer(USER, &req.id, denied), Err(AnswerError::AlreadyAnswered));
    }

    #[tokio::test(start_paused = true)]
    async fn requests_expire_after_ten_minutes() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        tokio::time::advance(secs(600)).await;
        assert!(h.fetch(USER, &req.id).is_none());
        assert!(h.take_undelivered(USER).is_empty());
        assert_eq!(h.answer(USER, &req.id, found()), Err(AnswerError::NotFound));
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::NotFound);
    }

    #[tokio::test(start_paused = true)]
    async fn submit_wakes_long_polls_and_respects_capacity() {
        let signal = Arc::new(ItemSignal::new());
        let h = RelayHub::with_capacity(
            Timing {
                relay_wait: secs(45),
                offline: secs(10),
            },
            Arc::clone(&signal),
            1,
        );
        let mut rx = signal.subscribe();
        assert!(!rx.has_changed().unwrap());
        let req = h.submit(USER, &conn(), "ChatGPT", search(), 0).unwrap();
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        assert_eq!(req.v, reins_proto::PROTOCOL_VERSION);
        assert_eq!(req.connection_label, "ChatGPT");
        assert!(h.submit(USER, &conn(), "ChatGPT", search(), 0).is_err());
        assert!(!rx.has_changed().unwrap(), "a rejected submit does not wake anyone");
    }
}
```

Register the modules in `src/api/reins/mod.rs`, directly below the `#![allow(…)]` line:

```rust

pub mod relay;
pub mod ttl;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --features sqlite api::reins::`
Expected: FAIL to compile — `cannot find type TtlMap`, `cannot find type RelayHub`.

- [ ] **Step 3: Implement `TtlMap`**

Top of `src/api/reins/ttl.rs`:

```rust
//! Expiring map on tokio's clock, so timing is testable with a paused clock (plan Decision 4).

use std::{collections::HashMap, hash::Hash, time::Duration};

use tokio::time::Instant;

/// Returned by [`TtlMap::insert`] when the map already holds `capacity` live entries.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// Entries expire `ttl` after insertion. Expired entries are invisible at once and
/// physically dropped on the next `insert` or `purge`.
pub struct TtlMap<K, V> {
    ttl: Duration,
    capacity: usize,
    entries: HashMap<K, (Instant, V)>,
}

impl<K: Eq + Hash, V> TtlMap<K, V> {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: HashMap::new(),
        }
    }

    /// Inserts or replaces `key`; the entry lives until `now + ttl`.
    pub fn insert(&mut self, key: K, value: V) -> Result<(), Full> {
        self.purge();
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            return Err(Full);
        }
        self.entries.insert(key, (Instant::now() + self.ttl, value));
        Ok(())
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        let now = Instant::now();
        self.entries.get(key).filter(|(expires, _)| now < *expires).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let now = Instant::now();
        self.entries.get_mut(key).filter(|(expires, _)| now < *expires).map(|(_, v)| v)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (expires, value) = self.entries.remove(key)?;
        (Instant::now() < expires).then_some(value)
    }

    /// Live values, in unspecified order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        let now = Instant::now();
        self.entries.values().filter(move |(expires, _)| now < *expires).map(|(_, v)| v)
    }

    pub fn purge(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, (expires, _)| now < *expires);
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.values().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
```

- [ ] **Step 4: Implement the relay hub**

Top of `src/api/reins/relay.rs`:

```rust
//! In-memory relay between MCP tool calls and the approval device (spec §4.3).
//!
//! Nothing here touches disk: tool arguments and results live only in these maps.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use reins_proto::{
    PROTOCOL_VERSION,
    gmail::ToolCall,
    ids::{ConnectionId, RequestId},
    relay::{RelayOutcome, RelayRequest},
};
use tokio::{sync::watch, time::Instant};

use super::{
    ITEM_TTL, Timing,
    ttl::{Full, TtlMap},
};
use crate::util::get_uuid;

/// Upper bound on relay requests held in memory at once.
pub const MAX_REQUESTS: usize = 10_000;

/// Bumped whenever a request or pairing is queued; phone long-polls (A2) wait on it.
pub struct ItemSignal {
    tx: watch::Sender<u64>,
}

impl ItemSignal {
    pub fn new() -> Self {
        Self {
            tx: watch::Sender::new(0),
        }
    }

    pub fn notify(&self) {
        self.tx.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.tx.subscribe()
    }
}

impl Default for ItemSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// What a waiting tool call can observe about its request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RequestState {
    /// The phone fetched the request (A2 or A3).
    delivered: bool,
    /// The phone's answer (A4), kept until the entry expires.
    outcome: Option<RelayOutcome>,
}

struct RequestEntry {
    user: String,
    request: RelayRequest,
    state: watch::Sender<RequestState>,
}

/// Outcome of waiting on a relay request (spec §4.3 table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitResult {
    /// The phone answered (result, denied or error).
    Answered(RelayOutcome),
    /// The phone did not fetch the request within the offline threshold.
    Offline,
    /// Fetched, but no decision within the relay wait.
    Pending,
    /// Unknown, expired, or created by another connection.
    NotFound,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    /// Unknown, expired, or belongs to another user.
    NotFound,
    AlreadyAnswered,
}

pub struct RelayHub {
    timing: Timing,
    signal: Arc<ItemSignal>,
    requests: Mutex<TtlMap<RequestId, RequestEntry>>,
}

fn mark_delivered(state: &mut RequestState) -> bool {
    if state.delivered {
        false
    } else {
        state.delivered = true;
        true
    }
}

impl RelayHub {
    pub fn new(timing: Timing, signal: Arc<ItemSignal>) -> Self {
        Self::with_capacity(timing, signal, MAX_REQUESTS)
    }

    pub fn with_capacity(timing: Timing, signal: Arc<ItemSignal>, capacity: usize) -> Self {
        Self {
            timing,
            signal,
            requests: Mutex::new(TtlMap::new(ITEM_TTL, capacity)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TtlMap<RequestId, RequestEntry>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues a normalized tool call for `user`'s approval device and wakes long-polls.
    pub fn submit(
        &self,
        user: &str,
        connection_id: &ConnectionId,
        connection_label: &str,
        call: ToolCall,
        now_unix: i64,
    ) -> Result<RelayRequest, Full> {
        let request = RelayRequest {
            v: PROTOCOL_VERSION,
            id: RequestId(get_uuid()),
            connection_id: connection_id.clone(),
            connection_label: connection_label.to_owned(),
            created_at: now_unix,
            call,
        };
        let entry = RequestEntry {
            user: user.to_owned(),
            request: request.clone(),
            state: watch::Sender::new(RequestState::default()),
        };
        self.lock().insert(request.id.clone(), entry)?;
        self.signal.notify();
        Ok(request)
    }

    /// Requests of `user` never handed to the phone before, oldest first; marks them delivered.
    pub fn take_undelivered(&self, user: &str) -> Vec<RelayRequest> {
        let map = self.lock();
        let mut out = Vec::new();
        for entry in map.values() {
            if entry.user == user && entry.state.send_if_modified(mark_delivered) {
                out.push(entry.request.clone());
            }
        }
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// One request of `user` (answered or not); marks it delivered.
    pub fn fetch(&self, user: &str, id: &RequestId) -> Option<RelayRequest> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user)?;
        entry.state.send_if_modified(mark_delivered);
        Some(entry.request.clone())
    }

    /// Stores the phone's answer; only the first answer counts.
    pub fn answer(&self, user: &str, id: &RequestId, outcome: RelayOutcome) -> Result<(), AnswerError> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user).ok_or(AnswerError::NotFound)?;
        let stored = entry.state.send_if_modified(move |s| {
            if s.outcome.is_some() {
                return false;
            }
            s.delivered = true;
            s.outcome = Some(outcome);
            true
        });
        if stored {
            Ok(())
        } else {
            Err(AnswerError::AlreadyAnswered)
        }
    }

    /// Waits for the phone per spec §4.3, measured from now: `Offline` if the request is still
    /// undelivered after `timing.offline`, `Pending` after `timing.relay_wait`, else the answer.
    /// Only the connection that created the request may wait on it.
    pub async fn wait(&self, id: &RequestId, connection_id: &ConnectionId) -> WaitResult {
        let start = Instant::now();
        let receiver = {
            let map = self.lock();
            map.get(id).filter(|e| e.request.connection_id == *connection_id).map(|e| e.state.subscribe())
        };
        let Some(mut rx) = receiver else {
            return WaitResult::NotFound;
        };
        let offline_at = start + self.timing.offline;
        let pending_at = start + self.timing.relay_wait;
        loop {
            let (delivered, outcome) = {
                let state = rx.borrow_and_update();
                (state.delivered, state.outcome.clone())
            };
            if let Some(outcome) = outcome {
                return WaitResult::Answered(outcome);
            }
            let now = Instant::now();
            if !delivered && now >= offline_at {
                return WaitResult::Offline;
            }
            if now >= pending_at {
                return WaitResult::Pending;
            }
            let deadline = if delivered {
                pending_at
            } else {
                offline_at.min(pending_at)
            };
            if let Ok(Err(_)) = tokio::time::timeout_at(deadline, rx.changed()).await {
                // The entry was dropped (expired and purged) while we waited.
                return WaitResult::NotFound;
            }
        }
    }

    pub fn purge(&self) {
        self.lock().purge();
    }
}
```

- [ ] **Step 5: Run the tests and lints**

Run: `cargo test --features sqlite api::reins::`
Expected: all `ttl::tests` (3) and `relay::tests` (9) pass, plus Task 2's tests.

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings`
Expected: clean. (If clippy reports `equatable_if_let` on `if let Ok(Err(_)) = …`, rewrite it as `if matches!(tokio::time::timeout_at(deadline, rx.changed()).await, Ok(Err(_)))`.)

- [ ] **Step 6: Commit**

```bash
git add src/api/reins/mod.rs src/api/reins/ttl.rs src/api/reins/relay.rs
git commit -m "feat(reins): in-memory relay hub with offline/pending timing"
```

---

### Task 5: Pairing invariants, pairing hub and the combined `Hub` (A2 long-poll core)

**Files:**
- Create: `src/api/reins/pairing.rs`
- Modify: `src/api/reins/mod.rs` (add `pub mod pairing;`, the `Hub` struct, `HUB` static and a `hub_tests` module)

**Interfaces:**
- Consumes: `relay::{ItemSignal, RelayHub}`, `ttl::{Full, TtlMap}` (Task 4); `ITEM_TTL`, `Timing` (Task 2); `crate::util::get_uuid`; `rand::RngExt` (as in `src/crypto.rs`); `reins_proto::{PROTOCOL_VERSION, ids::{ConnectionId, PairingId}, pairing::{PairingRequest, PairingResponse}, device::Pending}`.
- Produces (`crate::api::reins::pairing`):
  - `const CODE_MIN: u8 = 10`, `CODE_MAX: u8 = 99`, `MAX_NAME_CHARS: usize = 64`, `MAX_PAIRINGS: usize = 1_000`, `UNKNOWN_CLIENT: &str = "Unknown client"`
  - `fn generate_choices() -> (u8, [u8; 3])` — (browser code, choices)
  - `fn sanitize_display(raw: &str, max_chars: usize) -> String`, `fn sanitize_client_name(raw: &str) -> String`
  - `struct PairingClient { client_id: String, client_name: String, client_host: String }`
  - `enum PairingStatus { Waiting, Approved { connection_id: ConnectionId }, Rejected }`
  - `enum PairingAnswer { Approved { client: PairingClient, label: String }, Denied, WrongCode }`, `enum PairingAnswerError { NotFound, AlreadyAnswered, Invalid(String) }`
  - `struct PairingHub`: `new(signal: Arc<ItemSignal>)`, `with_capacity(signal, capacity)`, `start(&self, user: &str, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full>`, `start_decoy(&self, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full>`, `take_undelivered(&self, user: &str) -> Vec<PairingRequest>`, `fetch(&self, user: &str, id: &PairingId) -> Option<PairingRequest>`, `answer(&self, user: &str, id: &PairingId, response: &PairingResponse) -> Result<PairingAnswer, PairingAnswerError>`, `complete(&self, id: &PairingId, connection_id: ConnectionId)`, `fail(&self, id: &PairingId)`, `status(&self, id: &PairingId) -> Option<PairingStatus>`, `purge(&self)`
- Produces (`crate::api::reins`): `struct Hub { signal: Arc<ItemSignal>, relay: RelayHub, pairings: PairingHub }`, `Hub::new(timing: Timing) -> Hub`, `async Hub::pending(&self, user: &str, wait: Duration) -> Pending`, `Hub::purge(&self)`, `static HUB: LazyLock<Hub>` (built from `Timing::from_config()`).

- [ ] **Step 1: Write the failing tests**

`src/api/reins/pairing.rs` — tests first:

```rust
#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use super::*;

    fn hub() -> PairingHub {
        PairingHub::new(Arc::new(ItemSignal::new()))
    }

    fn client() -> PairingClient {
        PairingClient {
            client_id: "https://chatgpt.com/oauth/client.json".to_owned(),
            client_name: "Chat\u{202E}GPT".to_owned(),
            client_host: "chatgpt.com".to_owned(),
        }
    }

    fn approve(code: u8, label: Option<&str>) -> PairingResponse {
        PairingResponse {
            v: 1,
            approved: true,
            chosen_code: Some(code),
            label: label.map(str::to_owned),
        }
    }

    fn other_choice(req: &PairingRequest, code: u8) -> u8 {
        *req.choices.iter().find(|c| **c != code).unwrap()
    }

    #[test]
    fn choices_are_three_distinct_two_digit_codes_containing_the_code() {
        let mut positions = HashSet::new();
        let mut codes = HashSet::new();
        for _ in 0..2000 {
            let (code, choices) = generate_choices();
            assert!(choices.iter().all(|c| (CODE_MIN..=CODE_MAX).contains(c)), "{choices:?}");
            assert!(choices[0] != choices[1] && choices[1] != choices[2] && choices[0] != choices[2], "{choices:?}");
            positions.insert(choices.iter().position(|c| *c == code).expect("code offered"));
            codes.insert(code);
        }
        assert_eq!(positions.len(), 3, "the code must not always sit in the same slot");
        assert!(codes.len() > 50, "codes must vary");
    }

    #[test]
    fn sanitize_strips_invisible_and_bidi_characters() {
        assert_eq!(sanitize_client_name("Chat\u{202E}GPT"), "ChatGPT");
        assert_eq!(sanitize_client_name("Cl\u{200B}au\u{FEFF}de\u{2066}"), "Claude");
        assert_eq!(sanitize_client_name("  Evil \n\t  Bot\u{7}  "), "Evil Bot");
        assert_eq!(sanitize_client_name("\u{061C}\u{200E}\u{200F}"), UNKNOWN_CLIENT);
        assert_eq!(sanitize_client_name(""), UNKNOWN_CLIENT);
        assert_eq!(sanitize_client_name(&"x".repeat(10_000)).chars().count(), MAX_NAME_CHARS);
        // HTML is kept here and escaped where it is rendered (pages.rs).
        assert_eq!(sanitize_client_name("<b>Bot</b>"), "<b>Bot</b>");
    }

    #[tokio::test(start_paused = true)]
    async fn start_sanitizes_and_offers_the_code() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 7).unwrap();
        assert_eq!(req.client_name, "ChatGPT");
        assert_eq!(req.client_host, "chatgpt.com");
        assert!(req.choices.contains(&code));
        assert_eq!((req.v, req.created_at), (1, 7));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn delivery_is_per_user_and_one_shot() {
        let h = hub();
        let (req, _) = h.start("u1", client(), 0).unwrap();
        assert_eq!(h.take_undelivered("u2").len(), 0);
        assert_eq!(h.take_undelivered("u1").len(), 1);
        assert_eq!(h.take_undelivered("u1").len(), 0);
        assert!(h.fetch("u2", &req.id).is_none());
        assert_eq!(h.fetch("u1", &req.id).unwrap().id, req.id);
    }

    #[tokio::test(start_paused = true)]
    async fn correct_code_approves_exactly_once() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        let answer = h.answer("u1", &req.id, &approve(code, Some(" Work\u{200B} GPT "))).unwrap();
        let PairingAnswer::Approved {
            client,
            label,
        } = answer
        else {
            panic!("expected approval, got {answer:?}")
        };
        assert_eq!(label, "Work GPT");
        assert_eq!(client.client_name, "ChatGPT");
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting), "still storing the connection");
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::AlreadyAnswered));
        h.complete(&req.id, "conn-1".into());
        assert_eq!(
            h.status(&req.id),
            Some(PairingStatus::Approved {
                connection_id: "conn-1".into()
            })
        );
        assert!(h.fetch("u1", &req.id).is_none(), "answered pairings are no longer offered");
    }

    #[tokio::test(start_paused = true)]
    async fn label_defaults_to_the_client_name() {
        let h = hub();
        for label in [None, Some("   \u{200B} ")] {
            let (req, code) = h.start("u1", client(), 0).unwrap();
            match h.answer("u1", &req.id, &approve(code, label)).unwrap() {
                PairingAnswer::Approved {
                    label,
                    ..
                } => assert_eq!(label, "ChatGPT"),
                other => panic!("{other:?}"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn wrong_code_cancels_the_pairing() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        let wrong = other_choice(&req, code);
        assert_eq!(h.answer("u1", &req.id, &approve(wrong, None)), Ok(PairingAnswer::WrongCode));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Rejected));
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::AlreadyAnswered));
    }

    #[tokio::test(start_paused = true)]
    async fn malformed_answers_leave_the_pairing_open() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        let mut no_code = approve(code, None);
        no_code.chosen_code = None;
        assert!(matches!(h.answer("u1", &req.id, &no_code), Err(PairingAnswerError::Invalid(_))));
        let not_offered = (CODE_MIN..=CODE_MAX).find(|c| !req.choices.contains(c)).unwrap();
        assert!(matches!(h.answer("u1", &req.id, &approve(not_offered, None)), Err(PairingAnswerError::Invalid(_))));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting));
        assert!(matches!(h.answer("u1", &req.id, &approve(code, None)), Ok(PairingAnswer::Approved { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn deny_rejects_and_other_users_cannot_answer() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        assert_eq!(h.answer("u2", &req.id, &approve(code, None)), Err(PairingAnswerError::NotFound));
        let deny = PairingResponse {
            v: 1,
            approved: false,
            chosen_code: None,
            label: None,
        };
        assert_eq!(h.answer("u1", &req.id, &deny), Ok(PairingAnswer::Denied));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Rejected));
    }

    #[tokio::test(start_paused = true)]
    async fn fail_rejects_an_approved_pairing() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        h.answer("u1", &req.id, &approve(code, None)).unwrap();
        h.fail(&req.id);
        assert_eq!(h.status(&req.id), Some(PairingStatus::Rejected));
    }

    #[tokio::test(start_paused = true)]
    async fn decoys_look_real_but_nobody_can_see_or_answer_them() {
        let h = hub();
        let (req, code) = h.start_decoy(client(), 0).unwrap();
        assert!(req.choices.contains(&code));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting));
        assert!(h.take_undelivered("").is_empty());
        assert!(h.take_undelivered("u1").is_empty());
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::NotFound));
    }

    #[tokio::test(start_paused = true)]
    async fn pairings_expire_after_ten_minutes() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        tokio::time::advance(Duration::from_secs(600)).await;
        assert_eq!(h.status(&req.id), None);
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::NotFound));
    }
}
```

Append to `src/api/reins/mod.rs` (after the existing `mod tests`):

```rust
#[cfg(test)]
mod hub_tests {
    use reins_proto::gmail::ToolCall;
    use tokio::time::{Instant, sleep};

    use super::*;

    fn test_hub() -> Hub {
        Hub::new(Timing {
            relay_wait: Duration::from_secs(45),
            offline: Duration::from_secs(10),
        })
    }

    fn client() -> pairing::PairingClient {
        pairing::PairingClient {
            client_id: "cid".to_owned(),
            client_name: "Claude".to_owned(),
            client_host: "claude.ai".to_owned(),
        }
    }

    fn read_call() -> ToolCall {
        ToolCall::GmailRead {
            message_ids: vec!["m1".to_owned()],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pending_returns_queued_items_immediately_and_only_once() {
        let hub = test_hub();
        hub.pairings.start("u1", client(), 0).unwrap();
        hub.relay.submit("u1", &"c1".into(), "Claude", read_call(), 0).unwrap();
        let start = Instant::now();
        let p = hub.pending("u1", Duration::from_secs(25)).await;
        assert_eq!((p.requests.len(), p.pairings.len()), (1, 1));
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert!(hub.pending("u1", Duration::ZERO).await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn pending_long_polls_until_an_item_for_this_user_arrives() {
        let hub = test_hub();
        let start = Instant::now();
        let (p, ()) = tokio::join!(hub.pending("u1", Duration::from_secs(25)), async {
            sleep(Duration::from_secs(3)).await;
            hub.pairings.start("u2", client(), 0).unwrap();
            sleep(Duration::from_secs(2)).await;
            hub.relay.submit("u1", &"c1".into(), "Claude", read_call(), 0).unwrap();
        });
        assert_eq!(p.requests.len(), 1);
        assert!(p.pairings.is_empty());
        assert_eq!(start.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn pending_gives_up_after_the_wait() {
        let hub = test_hub();
        let start = Instant::now();
        assert!(hub.pending("u1", Duration::from_secs(25)).await.is_empty());
        assert_eq!(start.elapsed(), Duration::from_secs(25));
    }
}
```

Register the module: in `src/api/reins/mod.rs` the module list becomes

```rust
pub mod pairing;
pub mod relay;
pub mod ttl;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --features sqlite api::reins::`
Expected: FAIL to compile — `cannot find type PairingHub`, `cannot find struct Hub`.

- [ ] **Step 3: Implement the pairing module**

Top of `src/api/reins/pairing.rs`:

```rust
//! AI-connection pairing (spec §4.5 step 2-3, contracts A5/A6): the browser shows a
//! two-digit code, the phone picks it among three choices.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rand::RngExt;
use reins_proto::{
    PROTOCOL_VERSION,
    ids::{ConnectionId, PairingId},
    pairing::{PairingRequest, PairingResponse},
};

use super::{
    ITEM_TTL,
    relay::ItemSignal,
    ttl::{Full, TtlMap},
};
use crate::util::get_uuid;

pub const CODE_MIN: u8 = 10;
pub const CODE_MAX: u8 = 99;
/// Longest client name or connection label kept (characters).
pub const MAX_NAME_CHARS: usize = 64;
/// Upper bound on pairings (including decoys) held in memory.
pub const MAX_PAIRINGS: usize = 1_000;
pub const UNKNOWN_CLIENT: &str = "Unknown client";

/// Returns the browser code and three distinct codes in `CODE_MIN..=CODE_MAX`, in random
/// order, exactly one of which is the browser code.
pub fn generate_choices() -> (u8, [u8; 3]) {
    let mut rng = rand::rng();
    let mut picked: Vec<u8> = Vec::with_capacity(3);
    while picked.len() < 3 {
        let candidate = rng.random_range(CODE_MIN..=CODE_MAX);
        if !picked.contains(&candidate) {
            picked.push(candidate);
        }
    }
    let code = picked[rng.random_range(0..3)];
    (code, [picked[0], picked[1], picked[2]])
}

/// Control, bidi-override, isolate and zero-width characters: never shown to the user.
fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Makes untrusted display text safe to show: whitespace collapsed to single spaces,
/// invisible characters removed, at most `max_chars` characters. HTML is escaped by the renderer.
pub fn sanitize_display(raw: &str, max_chars: usize) -> String {
    let visible: String =
        raw.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !is_invisible(*c)).collect();
    visible.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max_chars).collect()
}

/// A client-declared name as shown on the phone and stored with the connection.
pub fn sanitize_client_name(raw: &str) -> String {
    let name = sanitize_display(raw, MAX_NAME_CHARS);
    if name.is_empty() {
        UNKNOWN_CLIENT.to_owned()
    } else {
        name
    }
}

/// Server-verified facts about the AI client, fixed when the pairing starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingClient {
    pub client_id: String,
    pub client_name: String,
    /// Host of the validated redirect URI.
    pub client_host: String,
}

/// What the browser's wait page sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingStatus {
    Waiting,
    Approved {
        connection_id: ConnectionId,
    },
    /// Denied, wrong code, or the connection could not be stored.
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingAnswer {
    /// Create the connection, then call [`PairingHub::complete`] (or [`PairingHub::fail`]).
    Approved {
        client: PairingClient,
        label: String,
    },
    Denied,
    WrongCode,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingAnswerError {
    /// Unknown, expired, a decoy, or another user's pairing.
    NotFound,
    AlreadyAnswered,
    /// Malformed answer (400); the pairing stays open.
    Invalid(String),
}

enum Stage {
    Open,
    /// Approved with the right code; the connection is being stored.
    Answering,
    Done(PairingStatus),
}

struct PairingEntry {
    /// `None` for decoys (unknown email): no phone ever sees them.
    user: Option<String>,
    client: PairingClient,
    code: u8,
    request: PairingRequest,
    delivered: bool,
    stage: Stage,
}

enum Verdict {
    Approve {
        label: Option<String>,
    },
    Deny,
    WrongCode,
}

/// Decision 22: approval needs a chosen code among the choices; a valid but wrong choice cancels.
fn judge_response(response: &PairingResponse, code: u8, choices: &[u8; 3]) -> Result<Verdict, String> {
    if !response.approved {
        return Ok(Verdict::Deny);
    }
    let Some(chosen) = response.chosen_code else {
        return Err("chosen_code is required when approved".to_owned());
    };
    if !choices.contains(&chosen) {
        return Err(format!("chosen_code {chosen} is not one of the offered choices"));
    }
    if chosen != code {
        return Ok(Verdict::WrongCode);
    }
    let label = response.label.as_deref().map(|l| sanitize_display(l, MAX_NAME_CHARS)).filter(|l| !l.is_empty());
    Ok(Verdict::Approve {
        label,
    })
}

pub struct PairingHub {
    signal: Arc<ItemSignal>,
    pairings: Mutex<TtlMap<PairingId, PairingEntry>>,
}

impl PairingHub {
    pub fn new(signal: Arc<ItemSignal>) -> Self {
        Self::with_capacity(signal, MAX_PAIRINGS)
    }

    pub fn with_capacity(signal: Arc<ItemSignal>, capacity: usize) -> Self {
        Self {
            signal,
            pairings: Mutex::new(TtlMap::new(ITEM_TTL, capacity)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TtlMap<PairingId, PairingEntry>> {
        self.pairings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn insert(
        &self,
        user: Option<String>,
        client: PairingClient,
        now_unix: i64,
    ) -> Result<(PairingRequest, u8), Full> {
        let (code, choices) = generate_choices();
        let client = PairingClient {
            client_name: sanitize_client_name(&client.client_name),
            ..client
        };
        let request = PairingRequest {
            v: PROTOCOL_VERSION,
            id: PairingId(get_uuid()),
            client_name: client.client_name.clone(),
            client_host: client.client_host.clone(),
            choices,
            created_at: now_unix,
        };
        let entry = PairingEntry {
            user,
            client,
            code,
            request: request.clone(),
            delivered: false,
            stage: Stage::Open,
        };
        self.lock().insert(request.id.clone(), entry)?;
        Ok((request, code))
    }

    /// Starts a pairing for `user`'s approval device; returns it and the browser code.
    pub fn start(&self, user: &str, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full> {
        let started = self.insert(Some(user.to_owned()), client, now_unix)?;
        self.signal.notify();
        Ok(started)
    }

    /// Indistinguishable from `start` for the browser, but bound to no user (Decision 19).
    pub fn start_decoy(&self, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full> {
        self.insert(None, client, now_unix)
    }

    /// Open pairings of `user` never handed out before, oldest first; marks them delivered.
    pub fn take_undelivered(&self, user: &str) -> Vec<PairingRequest> {
        let mut map = self.lock();
        let ids: Vec<PairingId> = map
            .values()
            .filter(|e| e.user.as_deref() == Some(user) && !e.delivered && matches!(e.stage, Stage::Open))
            .map(|e| e.request.id.clone())
            .collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(entry) = map.get_mut(&id) {
                entry.delivered = true;
                out.push(entry.request.clone());
            }
        }
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// One open pairing of `user`; marks it delivered.
    pub fn fetch(&self, user: &str, id: &PairingId) -> Option<PairingRequest> {
        let mut map = self.lock();
        let entry = map.get_mut(id).filter(|e| e.user.as_deref() == Some(user) && matches!(e.stage, Stage::Open))?;
        entry.delivered = true;
        Some(entry.request.clone())
    }

    /// Applies the phone's answer (Decision 22). Only the first valid answer counts.
    pub fn answer(
        &self,
        user: &str,
        id: &PairingId,
        response: &PairingResponse,
    ) -> Result<PairingAnswer, PairingAnswerError> {
        let mut map = self.lock();
        let entry =
            map.get_mut(id).filter(|e| e.user.as_deref() == Some(user)).ok_or(PairingAnswerError::NotFound)?;
        if !matches!(entry.stage, Stage::Open) {
            return Err(PairingAnswerError::AlreadyAnswered);
        }
        let verdict =
            judge_response(response, entry.code, &entry.request.choices).map_err(PairingAnswerError::Invalid)?;
        entry.delivered = true;
        Ok(match verdict {
            Verdict::Deny => {
                entry.stage = Stage::Done(PairingStatus::Rejected);
                PairingAnswer::Denied
            }
            Verdict::WrongCode => {
                entry.stage = Stage::Done(PairingStatus::Rejected);
                PairingAnswer::WrongCode
            }
            Verdict::Approve {
                label,
            } => {
                entry.stage = Stage::Answering;
                PairingAnswer::Approved {
                    label: label.unwrap_or_else(|| entry.client.client_name.clone()),
                    client: entry.client.clone(),
                }
            }
        })
    }

    /// Records the connection created for an approved pairing.
    pub fn complete(&self, id: &PairingId, connection_id: ConnectionId) {
        if let Some(entry) = self.lock().get_mut(id)
            && matches!(entry.stage, Stage::Answering)
        {
            entry.stage = Stage::Done(PairingStatus::Approved {
                connection_id,
            });
        }
    }

    /// The approved pairing could not be completed (e.g. database error).
    pub fn fail(&self, id: &PairingId) {
        if let Some(entry) = self.lock().get_mut(id) {
            entry.stage = Stage::Done(PairingStatus::Rejected);
        }
    }

    /// Status for the browser; `None` once expired or unknown.
    pub fn status(&self, id: &PairingId) -> Option<PairingStatus> {
        self.lock().get(id).map(|e| match &e.stage {
            Stage::Open | Stage::Answering => PairingStatus::Waiting,
            Stage::Done(status) => status.clone(),
        })
    }

    pub fn purge(&self) {
        self.lock().purge();
    }
}
```

- [ ] **Step 4: Implement `Hub`**

In `src/api/reins/mod.rs`, extend the `use` block and add the hub after `Timing`'s `impl`:

```rust
use std::{
    path::Path,
    sync::{Arc, LazyLock},
    time::Duration,
};

use reins_proto::device::Pending;
use rocket::{Catcher, Route};

use self::{
    pairing::PairingHub,
    relay::{ItemSignal, RelayHub},
};
use crate::CONFIG;
```

```rust
/// All in-memory relay state (requests, results, pairings) of this server instance.
pub struct Hub {
    pub signal: Arc<ItemSignal>,
    pub relay: RelayHub,
    pub pairings: PairingHub,
}

impl Hub {
    pub fn new(timing: Timing) -> Self {
        let signal = Arc::new(ItemSignal::new());
        Self {
            relay: RelayHub::new(timing, Arc::clone(&signal)),
            pairings: PairingHub::new(Arc::clone(&signal)),
            signal,
        }
    }

    /// A2: undelivered requests and pairings of `user`; if there are none, waits up to `wait`
    /// and returns as soon as one appears. Returned items are marked delivered.
    pub async fn pending(&self, user: &str, wait: Duration) -> Pending {
        let deadline = tokio::time::Instant::now() + wait;
        let mut rx = self.signal.subscribe();
        loop {
            let pending = Pending {
                requests: self.relay.take_undelivered(user),
                pairings: self.pairings.take_undelivered(user),
            };
            if !pending.is_empty() || tokio::time::Instant::now() >= deadline {
                return pending;
            }
            if let Ok(Err(_)) = tokio::time::timeout_at(deadline, rx.changed()).await {
                tokio::time::sleep_until(deadline).await;
            }
        }
    }

    pub fn purge(&self) {
        self.relay.purge();
        self.pairings.purge();
    }
}

/// The process-wide hub (spec §2: one server instance per deployment).
pub static HUB: LazyLock<Hub> = LazyLock::new(|| Hub::new(Timing::from_config()));
```

- [ ] **Step 5: Run the tests and lints**

Run: `cargo test --features sqlite api::reins::`
Expected: all pass (pairing: 12, hub_tests: 3, plus Tasks 2 and 4).

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/api/reins/mod.rs src/api/reins/pairing.rs
git commit -m "feat(reins): pairing codes, sanitizing, pairing hub and A2 long-poll core"
```

---

### Task 6: FCM HTTP v1 sender

**Files:**
- Create: `src/api/reins/fcm.rs`
- Modify: `src/api/reins/mod.rs` (add `pub mod fcm;`; make `validate_settings` parse the service-account file)

**Interfaces:**
- Consumes: `crate::http_client::make_http_request(reqwest::Method, &str) -> Result<reqwest::RequestBuilder, crate::Error>`, `crate::db::{DbPool, models::{ReinsDevice, UserId}}`, `ReinsDevice::clear_fcm_token(&UserId, &str, &DbConn)` (Task 3), `reins_proto::pairing::{PushKind, PushMessage}`, `CONFIG.reins_fcm_service_account()`.
- Produces (`crate::api::reins::fcm`):
  - `struct ServiceAccount { project_id, client_email, private_key, token_uri: String }`, `ServiceAccount::from_json(&str) -> Result<ServiceAccount, String>`, `ServiceAccount::from_file(&str) -> Result<ServiceAccount, String>`
  - `const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging"`
  - `fn sign_assertion(account: &ServiceAccount, now: i64) -> Result<String, String>` (RS256 JWT-bearer assertion, 1 h)
  - `fn message_body(fcm_token: &str, push: &PushMessage) -> serde_json::Value`
  - `enum SendOutcome { Sent, Unregistered }`, `fn classify_response(status: u16, body: &str) -> Result<SendOutcome, String>`
  - `struct FcmSender` with `new(ServiceAccount)` and `async send(&self, fcm_token: &str, push: &PushMessage) -> Result<SendOutcome, String>` (access token cached for half its lifetime)
  - `fn spawn_push(pool: DbPool, user_uuid: UserId, fcm_token: Option<String>, push: PushMessage)` — fire-and-forget; debug-log no-op when push is unconfigured or the device has no token; clears a token FCM reports as unregistered.

- [ ] **Step 1: Write the failing tests**

`src/api/reins/fcm.rs` — tests first:

```rust
#[cfg(test)]
mod tests {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation};
    use openssl::{pkey::PKey, rsa::Rsa};
    use reins_proto::pairing::PushKind;
    use serde_json::Value;

    use super::*;

    /// A throwaway service account with a fresh PKCS#8 key, as Google issues them.
    fn account() -> (ServiceAccount, String) {
        let pkey = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let private_pem = String::from_utf8(pkey.private_key_to_pem_pkcs8().unwrap()).unwrap();
        let public_pem = String::from_utf8(pkey.public_key_to_pem().unwrap()).unwrap();
        let json = json!({
            "type": "service_account",
            "project_id": "reins-test",
            "private_key_id": "abc",
            "private_key": private_pem,
            "client_email": "fcm@reins-test.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token"
        });
        (ServiceAccount::from_json(&json.to_string()).unwrap(), public_pem)
    }

    #[test]
    fn parses_and_validates_service_accounts() {
        let (sa, _) = account();
        assert_eq!(sa.project_id, "reins-test");
        assert_eq!(sa.token_uri, "https://oauth2.googleapis.com/token");
        assert!(ServiceAccount::from_json("{}").is_err());
        assert!(ServiceAccount::from_json("not json").is_err());
        let no_uri = json!({"project_id": "p", "client_email": "e@x", "private_key": "k"});
        assert_eq!(ServiceAccount::from_json(&no_uri.to_string()).unwrap().token_uri, DEFAULT_TOKEN_URI);
        let http_uri = json!({"project_id": "p", "client_email": "e@x", "private_key": "k", "token_uri": "http://evil"});
        assert!(ServiceAccount::from_json(&http_uri.to_string()).is_err());
        let empty_project = json!({"project_id": "", "client_email": "e@x", "private_key": "k"});
        assert!(ServiceAccount::from_json(&empty_project.to_string()).is_err());
        assert!(ServiceAccount::from_file("/nonexistent/reins-sa.json").unwrap_err().contains("/nonexistent"));
    }

    #[test]
    fn assertion_is_a_google_jwt_bearer_token() {
        let (sa, public_pem) = account();
        let jwt = sign_assertion(&sa, 1_700_000_000).unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&["https://oauth2.googleapis.com/token"]);
        validation.validate_exp = false;
        let data =
            jsonwebtoken::decode::<Value>(&jwt, &DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap(), &validation)
                .unwrap();
        assert_eq!(data.claims["iss"], "fcm@reins-test.iam.gserviceaccount.com");
        assert_eq!(data.claims["scope"], FCM_SCOPE);
        assert_eq!(data.claims["iat"], 1_700_000_000);
        assert_eq!(data.claims["exp"], 1_700_003_600);
        let bad = ServiceAccount {
            private_key: "garbage".to_owned(),
            ..sa
        };
        assert!(sign_assertion(&bad, 0).is_err());
    }

    #[test]
    fn message_is_data_only_high_priority_and_carries_only_the_id() {
        let push = PushMessage {
            t: PushKind::Pair,
            id: "p1".to_owned(),
        };
        let body = message_body("device-token", &push);
        assert_eq!(
            body,
            json!({"message": {
                "token": "device-token",
                "data": {"t": "pair", "id": "p1"},
                "android": {"priority": "HIGH", "ttl": "600s"}
            }})
        );
        assert!(body["message"].get("notification").is_none());
    }

    #[test]
    fn classifies_fcm_responses() {
        assert!(matches!(classify_response(200, "{}"), Ok(SendOutcome::Sent)));
        assert!(matches!(classify_response(404, "{}"), Ok(SendOutcome::Unregistered)));
        let unregistered = r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","details":[{"errorCode":"UNREGISTERED"}]}}"#;
        assert!(matches!(classify_response(400, unregistered), Ok(SendOutcome::Unregistered)));
        let err = classify_response(503, "overloaded").unwrap_err();
        assert!(err.contains("503") && err.contains("overloaded"), "{err}");
        assert!(classify_response(500, &"x".repeat(10_000)).unwrap_err().len() < 400);
    }

    #[test]
    fn token_refresh_happens_at_half_lifetime() {
        let now = std::time::Instant::now();
        assert_eq!(refresh_at(now, 3600), now + std::time::Duration::from_secs(1800));
        assert_eq!(refresh_at(now, -5), now);
    }
}
```

In `src/api/reins/mod.rs` add `pub mod fcm;` to the module list (alphabetical: `fcm`, `pairing`, `relay`, `ttl`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --features sqlite api::reins::fcm`
Expected: FAIL to compile — `cannot find type ServiceAccount`.

- [ ] **Step 3: Implement the sender**

Top of `src/api/reins/fcm.rs`:

```rust
//! FCM HTTP v1 sender (spec §4.6). Pushes carry only `{t, id}`; no request content
//! ever transits Google.

use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reins_proto::pairing::PushMessage;
use serde_json::Value;

use crate::{
    CONFIG,
    db::{
        DbPool,
        models::{ReinsDevice, UserId},
    },
    http_client::make_http_request,
};

pub const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
pub const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const ASSERTION_SECS: i64 = 3600;
/// FCM drops an undelivered wake-up after this long (matches the relay item TTL).
const MESSAGE_TTL: &str = "600s";
const ERROR_EXCERPT_CHARS: usize = 200;

/// The fields of a Google service-account key file that the sender needs.
#[derive(Clone, Debug, Deserialize)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    DEFAULT_TOKEN_URI.to_owned()
}

impl ServiceAccount {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let account: Self = serde_json::from_str(json).map_err(|e| format!("invalid service-account JSON: {e}"))?;
        if account.project_id.is_empty() || account.client_email.is_empty() || account.private_key.is_empty() {
            return Err("service-account JSON lacks project_id, client_email or private_key".to_owned());
        }
        if !account.token_uri.starts_with("https://") {
            return Err("service-account token_uri must be https".to_owned());
        }
        Ok(account)
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
        Self::from_json(&json)
    }
}

#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// Signs the RFC 7523 JWT-bearer assertion exchanged at `token_uri` for an access token.
pub fn sign_assertion(account: &ServiceAccount, now: i64) -> Result<String, String> {
    // Google ships PKCS#8 keys; OpenSSL accepts both PKCS#8 and PKCS#1 and re-encodes as PKCS#1,
    // the form `EncodingKey::from_rsa_pem` is known to accept (as in `auth::initialize_keys`).
    let rsa = openssl::rsa::Rsa::private_key_from_pem(account.private_key.as_bytes())
        .map_err(|e| format!("invalid service-account private key: {e}"))?;
    let pkcs1 = rsa.private_key_to_pem().map_err(|e| format!("cannot encode private key: {e}"))?;
    let key = EncodingKey::from_rsa_pem(&pkcs1).map_err(|e| format!("invalid service-account private key: {e}"))?;
    let claims = AssertionClaims {
        iss: &account.client_email,
        scope: FCM_SCOPE,
        aud: &account.token_uri,
        iat: now,
        exp: now + ASSERTION_SECS,
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(|e| format!("cannot sign assertion: {e}"))
}

/// FCM v1 `messages:send` body: data-only, high priority (spec §4.6).
pub fn message_body(fcm_token: &str, push: &PushMessage) -> Value {
    json!({
        "message": {
            "token": fcm_token,
            "data": push,
            "android": {"priority": "HIGH", "ttl": MESSAGE_TTL}
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// The device token is no longer valid (app uninstalled or token rotated).
    Unregistered,
}

pub fn classify_response(status: u16, body: &str) -> Result<SendOutcome, String> {
    if (200..300).contains(&status) {
        return Ok(SendOutcome::Sent);
    }
    if status == 404 || body.contains("UNREGISTERED") {
        return Ok(SendOutcome::Unregistered);
    }
    let excerpt: String = body.chars().take(ERROR_EXCERPT_CHARS).collect();
    Err(format!("FCM answered {status}: {excerpt}"))
}

/// When a token valid for `expires_in` seconds should be replaced (spec: half its lifetime).
fn refresh_at(now: Instant, expires_in: i64) -> Instant {
    now + Duration::from_secs(u64::try_from(expires_in / 2).unwrap_or(0))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

struct CachedToken {
    token: String,
    refresh_at: Instant,
}

pub struct FcmSender {
    account: ServiceAccount,
    cached: tokio::sync::Mutex<Option<CachedToken>>,
}

impl FcmSender {
    pub fn new(account: ServiceAccount) -> Self {
        Self {
            account,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    async fn access_token(&self) -> Result<String, String> {
        let mut cached = self.cached.lock().await;
        if let Some(c) = cached.as_ref()
            && Instant::now() < c.refresh_at
        {
            return Ok(c.token.clone());
        }
        let assertion = sign_assertion(&self.account, chrono::Utc::now().timestamp())?;
        let response = make_http_request(reqwest::Method::POST, &self.account.token_uri)
            .map_err(|e| e.to_string())?
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())])
            .send()
            .await
            .map_err(|e| format!("token request failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("token endpoint answered {status}: {}", body.chars().take(ERROR_EXCERPT_CHARS).collect::<String>()));
        }
        let token: TokenResponse = response.json().await.map_err(|e| format!("invalid token response: {e}"))?;
        *cached = Some(CachedToken {
            token: token.access_token.clone(),
            refresh_at: refresh_at(Instant::now(), token.expires_in),
        });
        Ok(token.access_token)
    }

    pub async fn send(&self, fcm_token: &str, push: &PushMessage) -> Result<SendOutcome, String> {
        let access_token = self.access_token().await?;
        let url = format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", self.account.project_id);
        let response = make_http_request(reqwest::Method::POST, &url)
            .map_err(|e| e.to_string())?
            .bearer_auth(access_token)
            .json(&message_body(fcm_token, push))
            .send()
            .await
            .map_err(|e| format!("FCM request failed: {e}"))?;
        let status = response.status().as_u16();
        if status == 401 {
            // Revoked or rotated key: fetch a new access token next time.
            *self.cached.lock().await = None;
        }
        let body = response.text().await.unwrap_or_default();
        classify_response(status, &body)
    }
}

static SENDER: LazyLock<Option<FcmSender>> = LazyLock::new(|| {
    let path = CONFIG.reins_fcm_service_account();
    if path.is_empty() {
        return None;
    }
    match ServiceAccount::from_file(&path) {
        Ok(account) => Some(FcmSender::new(account)),
        Err(e) => {
            error!("Reins push disabled: {e}");
            None
        }
    }
});

/// Wakes the approval device without blocking the caller (Decision 31).
pub fn spawn_push(pool: DbPool, user_uuid: UserId, fcm_token: Option<String>, push: PushMessage) {
    let Some(sender) = SENDER.as_ref() else {
        debug!("Reins push not configured; the phone must poll for {:?} {}", push.t, push.id);
        return;
    };
    let Some(fcm_token) = fcm_token else {
        debug!("Reins approval device of {user_uuid} has no FCM token; it must poll");
        return;
    };
    tokio::spawn(async move {
        match sender.send(&fcm_token, &push).await {
            Ok(SendOutcome::Sent) => debug!("Reins push {:?} {} sent", push.t, push.id),
            Ok(SendOutcome::Unregistered) => {
                warn!("FCM token of the Reins device of {user_uuid} is unregistered; clearing it");
                if let Ok(conn) = pool.get().await
                    && let Err(e) = ReinsDevice::clear_fcm_token(&user_uuid, &fcm_token, &conn).await
                {
                    warn!("Could not clear the Reins FCM token: {e:?}");
                }
            }
            Err(e) => warn!("Reins push failed: {e}"),
        }
    });
}
```

- [ ] **Step 4: Validate the key file at startup**

In `src/api/reins/mod.rs` `validate_settings`, replace the last check

```rust
    if !fcm_path.is_empty() && !Path::new(fcm_path).is_file() {
        return Err(format!("`REINS_FCM_SERVICE_ACCOUNT` file `{fcm_path}` does not exist"));
    }
```

with

```rust
    if !fcm_path.is_empty() {
        fcm::ServiceAccount::from_file(fcm_path).map_err(|e| format!("`REINS_FCM_SERVICE_ACCOUNT`: {e}"))?;
    }
```

and drop `path::Path` from the `use std::{…}` list at the top of `mod.rs` (now unused).

- [ ] **Step 5: Run the tests, lints and a startup check with the dev key**

Run: `cargo test --features sqlite api::reins::`
Expected: all pass (5 new `fcm::tests`; Task 2's `fcm_path_must_exist_when_set` still passes because the error names the variable).

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings`
Expected: clean.

Run (the real dev key parses; the server starts):

```bash
cargo build --features sqlite
D=$(mktemp -d) && (cd "$D" && DATA_FOLDER="$D" WEB_VAULT_ENABLED=false DOMAIN=http://127.0.0.1:18082 ROCKET_PORT=18082 \
  REINS_ENABLED=true REINS_FCM_SERVICE_ACCOUNT=~/.config/reins/fcm-service-account.json \
  timeout 8 <repo>/target/debug/vaultwarden 2>&1 | grep -E "Rocket has launched|REINS")
```

Expected: `Rocket has launched from http://…:18082` and no `REINS_FCM_SERVICE_ACCOUNT` error.

- [ ] **Step 6: Commit**

```bash
git add src/api/reins/mod.rs src/api/reins/fcm.rs
git commit -m "feat(reins): FCM HTTP v1 sender with cached service-account token"
```

---

### Task 7: Phone API A1–A8 and the HTTP integration-test harness

**Files:**
- Create: `src/api/reins/device_api.rs`
- Modify: `src/api/reins/mod.rs` (`pub mod device_api;`, `now_unix()`, `routes()`/`catchers()` bodies)
- Create: `tests/reins_server/main.rs`, `tests/reins_server/harness.rs`, `tests/reins_server/phone_api.rs`

**Interfaces:**
- Consumes: `crate::auth::Headers { user: User, device: Device, .. }` (`user.uuid: UserId`, `device.uuid: DeviceId`), `crate::db::{DbConn, DbPool}`, models from Task 3, `HUB` (Task 5), `fcm::spawn_push` (Task 6), `reins_proto::{check_version, device::*, ids::*, pairing::*, relay::*}`.
- Produces:
  - `crate::api::reins::now_unix() -> i64`
  - `device_api::routes() -> Vec<Route>` (A1–A8 at `/reins/api/...`), `device_api::catchers() -> Vec<Catcher>` (JSON 401/404)
  - `device_api::PhoneResult<T> = Result<T, Custom<Json<ApiError>>>`, `device_api::api_err(Status, &str, impl Into<String>) -> Custom<Json<ApiError>>`
  - pure helpers `clamp_wait(Option<&str>) -> u32`, `normalize_fcm_token(Option<String>) -> Result<Option<String>, String>`, `parse_versioned<T: DeserializeOwned>(&[u8]) -> PhoneResult<T>`
  - Test harness (`tests/reins_server/harness.rs`): `Server::start().await`, `Server::start_with(Options { relay_wait_secs, offline_secs }).await`, `Server::url(&self, path) -> String`, `Server::phone(&self, email) -> Phone` (register + Android login), `Server::login(&self, email, device_id) -> String`; `Phone { base, token }` with `get/put/post/delete(path, Option<&Value>) -> (StatusCode, Value)` for paths under `/reins/api`; `client() -> reqwest::Client` (no redirects).

- [ ] **Step 1: Write the failing unit tests**

`src/api/reins/device_api.rs` — tests first:

```rust
#[cfg(test)]
mod tests {
    use reins_proto::relay::{RelayOutcome, RelayResponse};

    use super::*;

    #[test]
    fn wait_is_parsed_leniently_and_clamped() {
        assert_eq!(clamp_wait(None), 0);
        assert_eq!(clamp_wait(Some("10")), 10);
        assert_eq!(clamp_wait(Some("25")), 25);
        assert_eq!(clamp_wait(Some("999")), 25);
        assert_eq!(clamp_wait(Some("-1")), 0);
        assert_eq!(clamp_wait(Some("abc")), 0);
    }

    #[test]
    fn fcm_tokens_are_trimmed_and_bounded() {
        assert_eq!(normalize_fcm_token(None), Ok(None));
        assert_eq!(normalize_fcm_token(Some("   ".to_owned())), Ok(None));
        assert_eq!(normalize_fcm_token(Some(" abc:DEF_1 ".to_owned())), Ok(Some("abc:DEF_1".to_owned())));
        assert!(normalize_fcm_token(Some("x".repeat(MAX_FCM_TOKEN_BYTES + 1))).is_err());
        assert!(normalize_fcm_token(Some("a\nb".to_owned())).is_err());
    }

    #[test]
    fn versioned_bodies_check_v_before_shape() {
        let ok: RelayResponse = parse_versioned(br#"{"v":1,"outcome":"denied","reason":null}"#).unwrap();
        assert_eq!(
            ok.outcome,
            RelayOutcome::Denied {
                reason: None
            }
        );
        let e = parse_versioned::<RelayResponse>(br#"{"v":2,"outcome":"something-new"}"#).unwrap_err();
        assert_eq!((e.0, e.1.0.error.as_str()), (Status::BadRequest, codes::BAD_VERSION));
        let e = parse_versioned::<RelayResponse>(br#"{"outcome":"denied"}"#).unwrap_err();
        assert_eq!(e.1.0.error, codes::BAD_VERSION);
        let e = parse_versioned::<RelayResponse>(br#"{"v":1,"outcome":"bogus"}"#).unwrap_err();
        assert_eq!((e.0, e.1.0.error.as_str()), (Status::BadRequest, codes::BAD_REQUEST));
        let e = parse_versioned::<RelayResponse>(b"not json").unwrap_err();
        assert_eq!(e.1.0.error, codes::BAD_REQUEST);
    }
}
```

Register the module in `src/api/reins/mod.rs` (list: `device_api`, `fcm`, `pairing`, `relay`, `ttl`).

- [ ] **Step 2: Run the unit tests to verify they fail**

Run: `cargo test --features sqlite api::reins::device_api`
Expected: FAIL to compile — `cannot find function clamp_wait`.

- [ ] **Step 3: Implement the phone API**

Top of `src/api/reins/device_api.rs`:

```rust
//! Phone-facing API (contracts §A) under `{domain_path}/reins/api`.
//!
//! Auth is the normal Vaultwarden login (`Headers`); every endpoint but A1 also requires the
//! caller to be the user's registered approval device.

use std::time::Duration;

use reins_proto::{
    check_version,
    device::{
        ApiError, Connections, DeviceRegistered, DeviceRegistration, MAX_PENDING_WAIT_SECS, PairingResult, Pending,
        codes,
    },
    ids::{ConnectionId, PairingId, RequestId},
    pairing::{PairingRequest, PairingResponse, PushKind, PushMessage},
    relay::{RelayRequest, RelayResponse},
};
use rocket::{
    Catcher, Data, Route, State,
    data::ToByteUnit,
    http::Status,
    response::status::Custom,
    serde::json::Json,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    HUB, fcm, now_unix,
    pairing::{PairingAnswer, PairingAnswerError},
    relay::AnswerError,
};
use crate::{
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{ReinsConnection, ReinsDevice},
    },
};

pub type PhoneResult<T> = Result<T, Custom<Json<ApiError>>>;

pub const MAX_FCM_TOKEN_BYTES: usize = 4096;
const MAX_BODY_MIB: u64 = 20;

pub fn routes() -> Vec<Route> {
    routes![
        put_device,
        get_pending,
        get_request,
        post_request_response,
        get_pairing,
        post_pairing_response,
        get_connections,
        delete_connection
    ]
}

pub fn catchers() -> Vec<Catcher> {
    catchers![unauthorized, not_found_catcher]
}

pub fn api_err(status: Status, code: &str, message: impl Into<String>) -> Custom<Json<ApiError>> {
    Custom(status, Json(ApiError::new(code, message)))
}

fn bad_request(message: impl Into<String>) -> Custom<Json<ApiError>> {
    api_err(Status::BadRequest, codes::BAD_REQUEST, message)
}

fn not_found() -> Custom<Json<ApiError>> {
    api_err(Status::NotFound, codes::NOT_FOUND, "Unknown or expired id")
}

fn already_answered() -> Custom<Json<ApiError>> {
    api_err(Status::Conflict, codes::ALREADY_ANSWERED, "This item was already answered")
}

fn internal(e: &crate::Error) -> Custom<Json<ApiError>> {
    error!("Reins phone API: {e:?}");
    api_err(Status::InternalServerError, codes::INTERNAL, "Server error, please retry")
}

/// `wait` query value: invalid → 0, larger than 25 → 25 (contracts A2).
pub fn clamp_wait(raw: Option<&str>) -> u32 {
    raw.and_then(|w| w.trim().parse::<u32>().ok()).unwrap_or(0).min(MAX_PENDING_WAIT_SECS)
}

/// Decision 32: trimmed, empty → `None`, bounded, no control characters.
pub fn normalize_fcm_token(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(token) = raw.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if token.len() > MAX_FCM_TOKEN_BYTES || token.chars().any(char::is_control) {
        return Err(format!("fcm_token must be at most {MAX_FCM_TOKEN_BYTES} bytes without control characters"));
    }
    Ok(Some(token))
}

/// Parses a relay/pairing message, checking `v` first so a newer phone gets `bad_version`
/// rather than a confusing shape error (proto invariant: receivers call `check_version`).
pub fn parse_versioned<T: DeserializeOwned>(body: &[u8]) -> PhoneResult<T> {
    let value: Value = serde_json::from_slice(body).map_err(|e| bad_request(format!("invalid JSON: {e}")))?;
    let v = value.get("v").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    check_version(v).map_err(|e| api_err(Status::BadRequest, codes::BAD_VERSION, e.to_string()))?;
    serde_json::from_value(value).map_err(|e| bad_request(format!("invalid body: {e}")))
}

async fn read_body(data: Data<'_>) -> PhoneResult<Vec<u8>> {
    let bytes =
        data.open(MAX_BODY_MIB.mebibytes()).into_bytes().await.map_err(|e| bad_request(format!("unreadable body: {e}")))?;
    if !bytes.is_complete() {
        return Err(api_err(Status::PayloadTooLarge, codes::BAD_REQUEST, "body too large"));
    }
    Ok(bytes.into_inner())
}

fn user_key(headers: &Headers) -> String {
    headers.user.uuid.to_string()
}

async fn require_approval_device(headers: &Headers, conn: &DbConn) -> PhoneResult<()> {
    match ReinsDevice::find_by_user(&headers.user.uuid, conn).await {
        Some(device) if device.device_uuid == headers.device.uuid => Ok(()),
        _ => Err(api_err(
            Status::Forbidden,
            codes::NOT_APPROVAL_DEVICE,
            "This device is not the Reins approval device; register it with PUT /reins/api/device",
        )),
    }
}

fn answer_error(e: &AnswerError) -> Custom<Json<ApiError>> {
    match e {
        AnswerError::NotFound => not_found(),
        AnswerError::AlreadyAnswered => already_answered(),
    }
}

/// A1: makes the calling device the approval device; tells the replaced one via push.
#[put("/reins/api/device", data = "<data>")]
async fn put_device(
    data: Data<'_>,
    headers: Headers,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<DeviceRegistered>> {
    let body = read_body(data).await?;
    let registration: DeviceRegistration = if body.iter().all(u8::is_ascii_whitespace) {
        DeviceRegistration::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| bad_request(format!("invalid body: {e}")))?
    };
    let fcm_token = normalize_fcm_token(registration.fcm_token).map_err(bad_request)?;
    let row = ReinsDevice {
        user_uuid: headers.user.uuid.clone(),
        device_uuid: headers.device.uuid.clone(),
        fcm_token,
        updated_at: now_unix(),
    };
    let previous = row.replace(&conn).await.map_err(|e| internal(&e))?;
    let replaced = previous.filter(|p| p.device_uuid != row.device_uuid);
    if let Some(old) = &replaced {
        fcm::spawn_push(
            pool.inner().clone(),
            headers.user.uuid.clone(),
            old.fcm_token.clone(),
            PushMessage {
                t: PushKind::Replaced,
                id: String::new(),
            },
        );
    }
    Ok(Json(DeviceRegistered {
        replaced_previous: replaced.is_some(),
    }))
}

/// A2: undelivered requests and pairings; long-polls up to `wait` seconds when empty.
#[get("/reins/api/pending?<wait>")]
async fn get_pending(wait: Option<String>, headers: Headers, conn: DbConn) -> PhoneResult<Json<Pending>> {
    require_approval_device(&headers, &conn).await?;
    // Never hold a pooled DB connection during a long-poll.
    drop(conn);
    let wait = Duration::from_secs(u64::from(clamp_wait(wait.as_deref())));
    Ok(Json(HUB.pending(&user_key(&headers), wait).await))
}

/// A3
#[get("/reins/api/requests/<id>")]
async fn get_request(id: &str, headers: Headers, conn: DbConn) -> PhoneResult<Json<RelayRequest>> {
    require_approval_device(&headers, &conn).await?;
    HUB.relay.fetch(&user_key(&headers), &RequestId::from(id)).map(Json).ok_or_else(not_found)
}

/// A4
#[post("/reins/api/requests/<id>/response", data = "<data>")]
async fn post_request_response(id: &str, data: Data<'_>, headers: Headers, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &conn).await?;
    let response: RelayResponse = parse_versioned(&read_body(data).await?)?;
    HUB.relay.answer(&user_key(&headers), &RequestId::from(id), response.outcome).map_err(|e| answer_error(&e))?;
    Ok(Status::NoContent)
}

/// A5
#[get("/reins/api/pairings/<id>")]
async fn get_pairing(id: &str, headers: Headers, conn: DbConn) -> PhoneResult<Json<PairingRequest>> {
    require_approval_device(&headers, &conn).await?;
    HUB.pairings.fetch(&user_key(&headers), &PairingId::from(id)).map(Json).ok_or_else(not_found)
}

/// A6: creates the connection when the right code was chosen.
#[post("/reins/api/pairings/<id>/response", data = "<data>")]
async fn post_pairing_response(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    conn: DbConn,
) -> PhoneResult<Json<PairingResult>> {
    require_approval_device(&headers, &conn).await?;
    let response: PairingResponse = parse_versioned(&read_body(data).await?)?;
    let pairing_id = PairingId::from(id);
    match HUB.pairings.answer(&user_key(&headers), &pairing_id, &response) {
        Ok(PairingAnswer::Denied) => Ok(Json(PairingResult {
            connection_id: None,
        })),
        Ok(PairingAnswer::WrongCode) => Err(api_err(
            Status::Conflict,
            codes::WRONG_CODE,
            "The chosen code does not match the browser; the connection request was cancelled",
        )),
        Ok(PairingAnswer::Approved {
            client,
            label,
        }) => {
            let connection = ReinsConnection::new(
                headers.user.uuid.clone(),
                client.client_id,
                client.client_name,
                client.client_host,
                label,
                now_unix(),
            );
            if let Err(e) = connection.save(&conn).await {
                HUB.pairings.fail(&pairing_id);
                return Err(internal(&e));
            }
            let connection_id = ConnectionId(connection.uuid);
            HUB.pairings.complete(&pairing_id, connection_id.clone());
            Ok(Json(PairingResult {
                connection_id: Some(connection_id),
            }))
        }
        Err(PairingAnswerError::NotFound) => Err(not_found()),
        Err(PairingAnswerError::AlreadyAnswered) => Err(already_answered()),
        Err(PairingAnswerError::Invalid(message)) => Err(bad_request(message)),
    }
}

/// A7
#[get("/reins/api/connections")]
async fn get_connections(headers: Headers, conn: DbConn) -> PhoneResult<Json<Connections>> {
    require_approval_device(&headers, &conn).await?;
    let connections = ReinsConnection::find_by_user(&headers.user.uuid, &conn).await;
    Ok(Json(Connections {
        connections: connections.iter().map(ReinsConnection::to_info).collect(),
    }))
}

/// A8: access tokens die at once because every MCP call re-checks the connection.
#[delete("/reins/api/connections/<id>")]
async fn delete_connection(id: &str, headers: Headers, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &conn).await?;
    let Some(connection) = ReinsConnection::find_by_uuid_and_user(id, &headers.user.uuid, &conn).await else {
        return Err(not_found());
    };
    connection.delete(&conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

#[catch(401)]
fn unauthorized() -> Json<ApiError> {
    Json(ApiError::new(codes::UNAUTHORIZED, "Missing or invalid Vaultwarden access token"))
}

#[catch(404)]
fn not_found_catcher() -> Json<ApiError> {
    Json(ApiError::new(codes::NOT_FOUND, "No such Reins API endpoint"))
}
```

In `src/api/reins/mod.rs` add, after `enabled()`:

```rust
/// Current time as unix seconds (the unit of every Reins timestamp).
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}
```

and replace the bodies of `routes()` and `catchers()`:

```rust
pub fn routes() -> Vec<Route> {
    if !enabled() {
        return Vec::new();
    }
    device_api::routes()
}
```

```rust
pub fn catchers() -> Vec<Catcher> {
    if !enabled() {
        return Vec::new();
    }
    device_api::catchers()
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo test --features sqlite api::reins::device_api`
Expected: 3 passed.

- [ ] **Step 5: Write the integration harness and failing HTTP tests**

`tests/reins_server/main.rs`:

```rust
//! HTTP tests against the real `vaultwarden` binary with Reins enabled
//! (plan Decision 27). Each test starts its own server on a free port with a
//! temporary SQLite database.

mod harness;
mod phone_api;
```

`tests/reins_server/harness.rs`:

```rust
use std::{
    fs::File,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

use reqwest::{Client, StatusCode, redirect::Policy};
use serde_json::{Value, json};

/// Any string works: the server stores a salted PBKDF2 of whatever the client sends.
pub const PASSWORD_HASH: &str = "reins-test-master-password-hash";

pub struct Options {
    pub relay_wait_secs: u64,
    pub offline_secs: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            relay_wait_secs: 4,
            offline_secs: 2,
        }
    }
}

pub struct Server {
    pub base: String,
    child: Child,
    dir: PathBuf,
}

pub fn client() -> Client {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Another test thread may win the race; either way a provider is installed.
        rustls::crypto::ring::default_provider().install_default().ok();
    }
    Client::builder().redirect(Policy::none()).timeout(Duration::from_secs(90)).build().expect("reqwest client")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

impl Server {
    pub async fn start() -> Self {
        Self::start_with(Options::default()).await
    }

    pub async fn start_with(options: Options) -> Self {
        let port = free_port();
        let dir = std::env::temp_dir().join(format!("reins-it-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let log = File::create(dir.join("server.log")).expect("log file");
        let base = format!("http://127.0.0.1:{port}");
        let child = Command::new(env!("CARGO_BIN_EXE_vaultwarden"))
            .current_dir(&dir)
            .env_clear()
            .env("DATA_FOLDER", &dir)
            .env("DOMAIN", &base)
            .env("ROCKET_ADDRESS", "127.0.0.1")
            .env("ROCKET_PORT", port.to_string())
            .env("WEB_VAULT_ENABLED", "false")
            .env("LOG_LEVEL", "info")
            .env("REINS_ENABLED", "true")
            .env("REINS_RELAY_WAIT_SECS", options.relay_wait_secs.to_string())
            .env("REINS_OFFLINE_SECS", options.offline_secs.to_string())
            .stdout(log.try_clone().expect("log clone"))
            .stderr(log)
            .spawn()
            .expect("spawn vaultwarden");
        let server = Self {
            base,
            child,
            dir,
        };
        server.wait_alive().await;
        server
    }

    async fn wait_alive(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Ok(r) = client().get(self.url("/alive")).send().await
                && r.status() == StatusCode::OK
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let log = std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default();
        panic!("server did not start:\n{log}");
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub async fn register(&self, email: &str) {
        let body = json!({
            "email": email,
            "name": "Reins Test",
            "masterPasswordHash": PASSWORD_HASH,
            "masterPasswordHint": null,
            "key": "2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "kdf": 0,
            "kdfIterations": 600_000
        });
        let r = client().post(self.url("/identity/accounts/register")).json(&body).send().await.expect("register");
        let status = r.status();
        assert!(status.is_success(), "register {email}: {status} {}", r.text().await.unwrap_or_default());
    }

    /// Password login as an Android device (contracts §B); returns the access token.
    pub async fn login(&self, email: &str, device_id: &str) -> String {
        let form = [
            ("grant_type", "password"),
            ("username", email),
            ("password", PASSWORD_HASH),
            ("scope", "api offline_access"),
            ("client_id", "mobile"),
            ("deviceType", "0"),
            ("deviceIdentifier", device_id),
            ("deviceName", "Reins"),
        ];
        let r = client().post(self.url("/identity/connect/token")).form(&form).send().await.expect("login");
        let status = r.status();
        let body: Value = r.json().await.expect("login json");
        assert!(status.is_success(), "login {email}: {status} {body}");
        body["access_token"].as_str().expect("access_token").to_owned()
    }

    /// Registers `email` and logs in a fresh "phone" device.
    pub async fn phone(&self, email: &str) -> Phone {
        self.register(email).await;
        self.second_device(email).await
    }

    /// Another logged-in device of an already registered user.
    pub async fn second_device(&self, email: &str) -> Phone {
        let token = self.login(email, &uuid::Uuid::new_v4().to_string()).await;
        Phone {
            base: self.url("/reins/api"),
            token,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// A logged-in device talking to the phone API.
pub struct Phone {
    pub base: String,
    pub token: String,
}

impl Phone {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> (StatusCode, Value) {
        let mut request = client().request(method, format!("{}{path}", self.base)).bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("phone request");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call(reqwest::Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    pub async fn put(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.call(reqwest::Method::PUT, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> (StatusCode, Value) {
        self.call(reqwest::Method::DELETE, path, None).await
    }

    /// Registers this device as the approval device.
    pub async fn register_device(&self) {
        let (status, body) = self.put("/device", &json!({"fcm_token": null})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}
```

`tests/reins_server/phone_api.rs`:

```rust
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::json;

use crate::harness::{Server, client};

#[tokio::test]
async fn unauthenticated_calls_get_json_401() {
    let server = Server::start().await;
    let r = client().get(server.url("/reins/api/pending")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn only_the_registered_device_may_use_the_api() {
    let server = Server::start().await;
    let a = server.phone("alice@example.com").await;
    let (status, body) = a.get("/pending").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));

    let (status, body) = a.put("/device", &json!({"fcm_token": "tok-a"})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})));
    let (status, body) = a.put("/device", &json!({"fcm_token": "tok-a2"})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})), "same device re-registers");
    let (status, body) = a.get("/pending?wait=0").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"requests": [], "pairings": []})));

    let b = server.second_device("alice@example.com").await;
    assert_eq!(b.get("/connections").await.0, StatusCode::FORBIDDEN);
    let (status, body) = b.put("/device", &json!({})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": true})));
    assert_eq!(b.get("/connections").await.0, StatusCode::OK);
    let (status, body) = a.get("/pending").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));
}

#[tokio::test]
async fn pending_long_polls_for_the_requested_time() {
    let server = Server::start().await;
    let phone = server.phone("bob@example.com").await;
    phone.register_device().await;
    let start = Instant::now();
    let (status, body) = phone.get("/pending?wait=1").await;
    let elapsed = start.elapsed();
    assert_eq!((status, body), (StatusCode::OK, json!({"requests": [], "pairings": []})));
    assert!(elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(5), "{elapsed:?}");
    let start = Instant::now();
    assert_eq!(phone.get("/pending?wait=abc").await.0, StatusCode::OK);
    assert!(start.elapsed() < Duration::from_millis(900), "invalid wait means no wait");
}

#[tokio::test]
async fn unknown_items_and_malformed_answers() {
    let server = Server::start().await;
    let phone = server.phone("carol@example.com").await;
    phone.register_device().await;
    let (status, body) = phone.get("/requests/nope").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    let denied = json!({"v": 1, "outcome": "denied", "reason": null});
    assert_eq!(phone.post("/requests/nope/response", &denied).await.0, StatusCode::NOT_FOUND);
    let (status, body) = phone.post("/requests/nope/response", &json!({"v": 2, "outcome": "denied"})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_version")));
    let (status, body) = phone.post("/requests/nope/response", &json!({"v": 1, "outcome": "maybe"})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));
    assert_eq!(phone.get("/pairings/nope").await.0, StatusCode::NOT_FOUND);
    let approve = json!({"v": 1, "approved": true, "chosen_code": 12, "label": null});
    assert_eq!(phone.post("/pairings/nope/response", &approve).await.0, StatusCode::NOT_FOUND);
    let (status, body) = phone.get("/no-such-endpoint").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
}

#[tokio::test]
async fn connections_start_empty_and_fcm_tokens_are_validated() {
    let server = Server::start().await;
    let phone = server.phone("dave@example.com").await;
    let (status, body) = phone.put("/device", &json!({"fcm_token": "x".repeat(5000)})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));
    phone.register_device().await;
    let (status, body) = phone.get("/connections").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"connections": []})));
    let (status, body) = phone.delete("/connections/nope").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
}
```

- [ ] **Step 6: Run the integration tests**

Run: `cargo test --features sqlite --test reins_server`
Expected: 5 passed (`phone_api::*`). If a test fails at startup, the panic message contains the server log. If registration fails with a KDF/`key` validation error, adjust only the harness `register` body to what `src/api/core/accounts.rs` `RegisterData` requires (no server change).

- [ ] **Step 7: Lints and commit**

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings && cargo clippy --features sqlite,mysql,postgresql -- -D warnings`
Expected: clean.

```bash
git add src/api/reins/mod.rs src/api/reins/device_api.rs tests/reins_server
git commit -m "feat(reins): phone API A1-A8 with approval-device check and HTTP test harness"
```

---

### Task 8: MCP token primitives (`src/auth/reins.rs`)

**Files:**
- Create: `src/auth/reins.rs`
- Modify: `src/auth.rs:1-4` (declare the module next to `send`)

**Interfaces:**
- Consumes: private items of `crate::auth` visible to its child module: `JWT_ALGORITHM`, `JWT_HEADER`, `PRIVATE_RSA_KEY`, `PUBLIC_RSA_KEY` (auth.rs lines 40-68); `CONFIG.domain_origin()`; `crate::crypto::{encode_random_bytes, sha256_hex}`; `crate::api::reins::ACCESS_TOKEN_SECS`.
- Produces (`crate::auth::reins`):
  - `static JWT_MCP_ISSUER: LazyLock<String>` = `"{domain_origin}|mcp"`
  - `struct McpClaims { nbf: i64, exp: i64, iss: String, aud: String, sub: String /* user uuid */, cid: String /* connection uuid */, client_id: String, scope: String /* "mcp" */ }`
  - `fn mcp_claims(issuer: &str, audience: &str, user_uuid: &str, connection_uuid: &str, client_id: &str, now: i64) -> McpClaims` (lifetime `ACCESS_TOKEN_SECS`)
  - `fn encode_with(key: &EncodingKey, claims: &McpClaims) -> Result<String, String>`, `fn decode_with(key: &DecodingKey, token: &str, issuer: &str, audience: &str) -> Result<McpClaims, String>`
  - `fn issue_access_token(user_uuid: &str, connection_uuid: &str, client_id: &str, mcp_url: &str) -> String`, `fn decode_access_token(token: &str, mcp_url: &str) -> Result<McpClaims, String>` (global keys)
  - `fn random_token() -> String` (32 random bytes, base64url, no padding), `fn hash_token(token: &str) -> String` (SHA-256 hex)

- [ ] **Step 1: Write the failing tests**

`src/auth/reins.rs` — tests first:

```rust
#[cfg(test)]
mod tests {
    use openssl::rsa::Rsa;

    use super::*;

    const ISS: &str = "https://rw.example.com|mcp";
    const AUD: &str = "https://rw.example.com/mcp";

    fn keys() -> (EncodingKey, DecodingKey) {
        let rsa = Rsa::generate(2048).unwrap();
        (
            EncodingKey::from_rsa_pem(&rsa.private_key_to_pem().unwrap()).unwrap(),
            DecodingKey::from_rsa_pem(&rsa.public_key_to_pem().unwrap()).unwrap(),
        )
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    #[test]
    fn round_trips_with_issuer_and_audience() {
        let (enc, dec) = keys();
        let claims = mcp_claims(ISS, AUD, "user-1", "conn-1", "client-1", now());
        assert_eq!(claims.exp - claims.nbf, 3600);
        assert_eq!(claims.scope, "mcp");
        let token = encode_with(&enc, &claims).unwrap();
        assert_eq!(decode_with(&dec, &token, ISS, AUD).unwrap(), claims);
    }

    #[test]
    fn rejects_wrong_audience_issuer_key_and_tampering() {
        let (enc, dec) = keys();
        let token = encode_with(&enc, &mcp_claims(ISS, AUD, "u", "c", "k", now())).unwrap();
        assert!(decode_with(&dec, &token, ISS, "https://other.example.com/mcp").is_err());
        assert!(decode_with(&dec, &token, "https://rw.example.com|login", AUD).is_err());
        let (_, other_dec) = keys();
        assert!(decode_with(&other_dec, &token, ISS, AUD).is_err());
        let mut tampered = token.clone();
        tampered.insert(tampered.len() - 5, 'A');
        assert!(decode_with(&dec, &tampered, ISS, AUD).is_err());
        assert!(decode_with(&dec, "not-a-jwt", ISS, AUD).is_err());
    }

    #[test]
    fn rejects_expired_and_not_yet_valid_tokens() {
        let (enc, dec) = keys();
        let expired = encode_with(&enc, &mcp_claims(ISS, AUD, "u", "c", "k", now() - 3600 - 31)).unwrap();
        assert!(decode_with(&dec, &expired, ISS, AUD).is_err());
        let future = encode_with(&enc, &mcp_claims(ISS, AUD, "u", "c", "k", now() + 120)).unwrap();
        assert!(decode_with(&dec, &future, ISS, AUD).is_err());
    }

    #[test]
    fn opaque_tokens_are_random_urlsafe_and_hashed() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
```

Declare the module at the top of `src/auth.rs` (lines 1-4 become):

```rust
#[path = "auth/send.rs"]
pub mod send;
pub type SendTokens = send::SendTokens;
pub type SendHeaders = send::SendHeaders;
#[path = "auth/reins.rs"]
#[allow(dead_code, reason = "used by the Reins API as Plan 2 wires it; removed in Task 14")]
pub mod reins;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --features sqlite auth::reins`
Expected: FAIL to compile — `cannot find function mcp_claims`.

- [ ] **Step 3: Implement**

Top of `src/auth/reins.rs`:

```rust
//! Reins MCP tokens (spec §4.5): RS256 access JWTs signed with Vaultwarden's RSA key,
//! issuer `{domain_origin}|mcp`, audience = canonical MCP URL; opaque refresh tokens that
//! are stored only as SHA-256 hex.

use std::sync::LazyLock;

use data_encoding::BASE64URL_NOPAD;
use jsonwebtoken::{DecodingKey, EncodingKey, Validation};

use super::{JWT_ALGORITHM, JWT_HEADER, PRIVATE_RSA_KEY, PUBLIC_RSA_KEY};
use crate::{
    CONFIG,
    api::reins::ACCESS_TOKEN_SECS,
    crypto::{encode_random_bytes, sha256_hex},
};

pub static JWT_MCP_ISSUER: LazyLock<String> = LazyLock::new(|| format!("{}|mcp", CONFIG.domain_origin()));

/// Clock skew tolerated on `exp`/`nbf` (same as `auth::decode_jwt`).
const LEEWAY_SECS: u64 = 30;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpClaims {
    pub nbf: i64,
    pub exp: i64,
    pub iss: String,
    /// The canonical MCP URL this token is for (RFC 8707 audience).
    pub aud: String,
    /// User uuid.
    pub sub: String,
    /// Connection uuid; every MCP request checks it still exists.
    pub cid: String,
    pub client_id: String,
    pub scope: String,
}

pub fn mcp_claims(
    issuer: &str,
    audience: &str,
    user_uuid: &str,
    connection_uuid: &str,
    client_id: &str,
    now: i64,
) -> McpClaims {
    McpClaims {
        nbf: now,
        exp: now + ACCESS_TOKEN_SECS,
        iss: issuer.to_owned(),
        aud: audience.to_owned(),
        sub: user_uuid.to_owned(),
        cid: connection_uuid.to_owned(),
        client_id: client_id.to_owned(),
        scope: "mcp".to_owned(),
    }
}

pub fn encode_with(key: &EncodingKey, claims: &McpClaims) -> Result<String, String> {
    jsonwebtoken::encode(&JWT_HEADER, claims, key).map_err(|e| e.to_string())
}

pub fn decode_with(key: &DecodingKey, token: &str, issuer: &str, audience: &str) -> Result<McpClaims, String> {
    let mut validation = Validation::new(JWT_ALGORITHM);
    validation.leeway = LEEWAY_SECS;
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);
    validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
    jsonwebtoken::decode::<McpClaims>(token.trim(), key, &validation).map(|d| d.claims).map_err(|e| e.to_string())
}

/// A 1 h access token for `connection_uuid`, audience `mcp_url`.
pub fn issue_access_token(user_uuid: &str, connection_uuid: &str, client_id: &str, mcp_url: &str) -> String {
    let claims =
        mcp_claims(&JWT_MCP_ISSUER, mcp_url, user_uuid, connection_uuid, client_id, chrono::Utc::now().timestamp());
    encode_with(PRIVATE_RSA_KEY.wait(), &claims).expect("signing with the server key cannot fail")
}

pub fn decode_access_token(token: &str, mcp_url: &str) -> Result<McpClaims, String> {
    decode_with(PUBLIC_RSA_KEY.wait(), token, &JWT_MCP_ISSUER, mcp_url)
}

/// 256-bit random opaque token (authorization codes, refresh tokens, session ids).
pub fn random_token() -> String {
    encode_random_bytes::<32>(&BASE64URL_NOPAD)
}

/// How opaque tokens are stored and looked up.
pub fn hash_token(token: &str) -> String {
    sha256_hex(token.as_bytes())
}
```

- [ ] **Step 4: Run the tests and lints**

Run: `cargo test --features sqlite auth::reins`
Expected: 4 passed.

Run: `cargo fmt --check && cargo clippy --features sqlite --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/auth.rs src/auth/reins.rs
git commit -m "feat(reins): MCP access-token JWTs with audience check and opaque token helpers"
```

---

