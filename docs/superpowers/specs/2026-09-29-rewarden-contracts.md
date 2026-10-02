# Rewarden MVP — Component Contracts

Companion to `2026-09-28-rewarden-mvp-design.md`. Plans 2 (server), 3 (phone core) and
4 (Android) are written and implemented in parallel; this document is the **binding
interface between them**. Any change here must be made here first and propagated to every
plan that consumes it.

All JSON is snake_case. All timestamps are unix seconds (`i64`). Wire types marked
`proto::X` live in `crates/rewarden-proto` (Plan 1); types marked **new in Plan 2** are
added by Plan 2 to `crates/rewarden-proto/src/device.rs` (module `device`), so the phone
core can use the same structs.

---

## A. Phone API — server ↔ `rewarden-core`

Base: `{domain}/rewarden/api`. Auth: `Authorization: Bearer <Vaultwarden access token>`
obtained from `/identity/connect/token` (existing Vaultwarden login, validated with the
existing `Headers` guard). Errors: HTTP status + body `{"error": "<code>", "message": "<text>"}`
(`proto::device::ApiError`, new in Plan 2). Missing/invalid token → 401 (Vaultwarden's own
error body is acceptable there).

Every endpoint except `PUT /device` additionally requires that the caller's device
(`claims.device`) is the user's registered approval device, else **403
`not_approval_device`**.

| # | Method, path | Request body | Success | Errors |
|---|---|---|---|---|
| A1 | `PUT /device` | `DeviceRegistration { fcm_token: Option<String> }` | 200 `DeviceRegistered { replaced_previous: bool }` | — |
| A2 | `GET /pending?wait=<0..=25>` | — | 200 `Pending { requests: Vec<proto::relay::RelayRequest>, pairings: Vec<proto::pairing::PairingRequest> }`; when both lists are empty and `wait > 0`, the server holds the response up to `wait` s and returns as soon as an item appears. Values > 25 are clamped. Returning items **marks them delivered**. | 403 |
| A3 | `GET /requests/{id}` | — | 200 `proto::relay::RelayRequest` (marks delivered) | 404 `not_found` (unknown, expired, other user) |
| A4 | `POST /requests/{id}/response` | `proto::relay::RelayResponse` | 204 | 404 `not_found`; 409 `already_answered`; 400 `bad_version` if `v != 1` |
| A5 | `GET /pairings/{id}` | — | 200 `proto::pairing::PairingRequest` (marks delivered) | 404 |
| A6 | `POST /pairings/{id}/response` | `proto::pairing::PairingResponse` | 200 `PairingResult { connection_id: Option<ConnectionId> }` (`None` when denied) | 404; 409 `wrong_code` — the pairing is cancelled and the browser shows failure |
| A7 | `GET /connections` | — | 200 `Connections { connections: Vec<ConnectionInfo> }` | 403 |
| A8 | `DELETE /connections/{id}` | — | 204 (deletes connection + its refresh tokens; its access tokens stop working immediately) | 404 |

`ConnectionInfo { id: ConnectionId, label: String, client_name: String, client_host: String, created_at: i64, last_used_at: Option<i64> }`.

Request lifetime on the server: pending requests and pairings are retrievable for **600 s**;
a request answered after the AI stopped waiting keeps its result for 600 s for
`rewarden_get_result`. Registering a new approval device (A1 from a different device than
the current one) sends the old device a push `{t: "replaced", id: ""}`; the server forgets
nothing else.

### Push (FCM data message, high priority)
`proto::pairing::PushMessage { t: "req" | "pair" | "replaced", id }`. The phone must treat
the push as a hint only: it always fetches via A3/A5 (or A2) over HTTPS. `replaced` tells the
phone it is no longer the approval device (show a notice; A2 will return 403).

The iOS app registers `fcm_token` as `apns:<hex>` (production) or `apns-sandbox:<hex>` (development), 64 to 200 hex digits (APNs tokens vary in length: 32 bytes on a phone, 80 on a simulator); the
server sends those through APNs instead of FCM (`src/api/rewarden/apns.rs`). The APNs payload is
`{"aps": {...fixed alert, category "request" | "pairing" | "blob"...}, "t", "id"}` (alert push, priority 10), and
`{"aps": {"content-available": 1}, "t": "replaced", "id": ""}` (background push, priority 5) for `replaced`.

### Proto invariants consumers must honor (from Plan 1 final review)
- Every `RelayRequest`/`RelayResponse`/`PairingRequest`/`PairingResponse` receiver checks
  `rewarden_proto::check_version(msg.v)?`.
- The server calls `ToolCall::normalized()` before relaying; the phone calls it again on
  every received `RelayRequest.call` before evaluating (defense in depth).
- `MessageSummary { id, thread_id, from, from_name: Option<String>, to, cc, subject, date, snippet }`
  — `to` = To only, `cc` = Cc only, all bare normalized addresses; `date` = Gmail
  `internalDate / 1000` (unix **seconds**). `rewarden_policy::MessageFacts.to` is To **and** Cc.
- `OutgoingEmail` and `Grant` reject unknown fields; the MCP `gmail_send` input is flat
  (`to, cc, subject, body, reply_to_message_id`) and the server maps it into
  `ToolCall::GmailSend { email: Box<OutgoingEmail> }` (wire tag `{"tool":"gmail_send","email":{…}}`).
- `rewarden_policy::Pattern::literal(text)` exists for "contains this text" UI inputs;
  `GrantScopeChoice.subject_pattern` is treated as literal text by the core (use
  `Pattern::literal`), not as a regex.

## B. Vaultwarden login used by the phone (existing upstream API)

1. `POST {domain}/identity/accounts/prelogin` JSON `{"email": "<email>"}` → `{"kdf": 0|1, "kdfIterations": n, "kdfMemory": m|null, "kdfParallelism": p|null}` (0 = PBKDF2-SHA256, 1 = Argon2id; memory in MiB).
2. Master key: PBKDF2-SHA256(password, salt = lower(email), iterations) → 32 bytes; or
   Argon2id(password, salt = SHA-256(lower(email)), t = iterations, m = memory MiB, p = parallelism) → 32 bytes.
3. Master password hash: `base64(PBKDF2-SHA256(key = master_key, salt = password, 1 iteration, 32 bytes))`.
4. `POST {domain}/identity/connect/token` form: `grant_type=password`, `username=<email>`,
   `password=<hash>`, `scope=api offline_access`, `client_id=mobile`, `deviceType=0`,
   `deviceIdentifier=<stable uuid>`, `deviceName=Rewarden`. When the response is 400 with
   `TwoFactorProviders` containing `0`, retry with `twoFactorToken=<totp>`,
   `twoFactorProvider=0`, `twoFactorRemember=0`. Other providers → "not supported".
   Plan 3 must verify header requirements (e.g. `Auth-Email`) against `src/api/identity.rs`.
5. Refresh: `grant_type=refresh_token&client_id=mobile&refresh_token=<rt>`.

## C. MCP and OAuth — AI client ↔ server

Defined by spec §4.2–4.5; Plan 2 owns the details. Fixed points other plans rely on:
- MCP endpoint `{domain}/mcp`; tools `gmail_search`, `gmail_read`, `gmail_send`, `rewarden_get_result` (spec §4.2 table).
- Relay timing (spec §4.3): offline threshold 10 s (request not delivered via A2/A3), wait 45 s.
- Default `max_results` for `gmail_search` is 10, applied by the server before relaying.
- The server calls `ToolCall::normalized()` before relaying; a validation error returns an
  `isError: true` tool result describing the problem.

## D. `rewarden-core` UniFFI surface — core ↔ Kotlin (Plan 3 produces, Plan 4 consumes)

Crate `crates/rewarden-core`, UniFFI 0.32 proc-macros, Kotlin package
`dev.rewarden.core` (via `uniffi.toml`: `[bindings.kotlin] package_name = "dev.rewarden.core"`,
`cdylib_name = "rewarden_core"`). All `async` methods are Kotlin `suspend` functions
and never block the calling thread.

```rust
#[derive(uniffi::Object)] pub struct RewardenCore;

#[uniffi::export(async_runtime = "tokio")]
impl RewardenCore {
    #[uniffi::constructor]
    pub fn new(data_dir: String, keys: Arc<dyn KeyWrapper>, google: Arc<dyn GoogleTokenProvider>,
               notifier: Arc<dyn Notifier>) -> Result<Arc<Self>, CoreError>;

    pub async fn session(&self) -> Option<SessionInfo>;
    pub async fn login(&self, server_url: String, email: String, password: String,
                       totp: Option<String>) -> Result<SessionInfo, CoreError>;
    pub async fn logout(&self) -> Result<(), CoreError>;
    pub async fn register_device(&self, fcm_token: Option<String>) -> Result<(), CoreError>;

    /// Background entry point from FCM. Fetches, evaluates, executes if covered,
    /// otherwise parks the item and calls `Notifier`.
    pub async fn handle_push(&self, kind: String, id: String) -> Result<(), CoreError>;
    /// Foreground long-poll (A2) then processes everything received like `handle_push`.
    pub async fn sync(&self, wait_secs: u32) -> Result<Vec<PendingItem>, CoreError>;
    /// Items parked locally, awaiting the user, newest first.
    pub async fn pending(&self) -> Result<Vec<PendingItem>, CoreError>;

    pub async fn approval_view(&self, request_id: String) -> Result<ApprovalView, CoreError>;
    pub async fn approve(&self, request_id: String, choice: ApprovalChoice) -> Result<(), CoreError>;
    pub async fn deny(&self, request_id: String) -> Result<(), CoreError>;

    pub async fn pairing_view(&self, pairing_id: String) -> Result<PairingView, CoreError>;
    pub async fn answer_pairing(&self, pairing_id: String, approve: bool, chosen_code: Option<u8>,
                                label: Option<String>) -> Result<(), CoreError>;

    pub async fn grants(&self) -> Result<Vec<GrantView>, CoreError>;
    pub async fn revoke_grant(&self, grant_id: String) -> Result<(), CoreError>;
    pub async fn connections(&self) -> Result<Vec<ConnectionView>, CoreError>;
    pub async fn revoke_connection(&self, connection_id: String) -> Result<(), CoreError>;
    pub async fn activity(&self, limit: u32) -> Result<Vec<ActivityEntry>, CoreError>;
    /// Asks GoogleTokenProvider for a token and reports whether Gmail is usable.
    pub async fn gmail_status(&self) -> GmailStatus;
}
```

Records / enums (all `#[derive(uniffi::Record)]` / `uniffi::Enum`):

```rust
pub struct SessionInfo { pub server_url: String, pub email: String }

pub enum PendingKind { Request, Pairing }
pub struct PendingItem { pub kind: PendingKind, pub id: String, pub title: String,
                         pub subtitle: String, pub created_at: i64 }

pub enum ApprovalKind { Search, Read, Send }
pub struct MessageView { pub id: String, pub from: String, pub subject: String, pub date: i64,
                         pub snippet: String, pub covered_by_grant: bool }
pub struct EmailView { pub to: Vec<String>, pub cc: Vec<String>, pub subject: String, pub body: String }
pub struct ApprovalView { pub request_id: String, pub connection_label: String, pub kind: ApprovalKind,
                          pub query: Option<String>, pub messages: Vec<MessageView>,
                          pub email: Option<EmailView>, pub created_at: i64 }

/// What the user approved. For Search/Read: `selected_message_ids` ⊆ the view's messages
/// (covered ones are always included). For Send: ignored.
pub struct ApprovalChoice { pub selected_message_ids: Vec<String>, pub standing: Option<StandingGrant> }
/// Optional standing grant created alongside the approval.
pub struct StandingGrant { pub duration_secs: Option<u64>, pub max_uses: Option<u32>, pub scope: GrantScopeChoice }
/// Read grants: any combination of the read fields (at least one). Send grants: recipients.
pub struct GrantScopeChoice {
    pub selected_messages_only: bool,       // read: message_ids = the selected ids
    pub sender_addresses: Vec<String>,      // read: from Exact
    pub sender_domains: Vec<String>,        // read: from Domain
    pub subject_pattern: Option<String>,    // read/send: subject Pattern::find
    pub recipient_addresses: Vec<String>,   // send: recipients Exact
    pub recipient_domains: Vec<String>,     // send: recipients Domain
}

pub struct PairingView { pub id: String, pub client_name: String, pub client_host: String,
                         pub choices: Vec<u8>, pub created_at: i64 }
pub struct GrantView { pub id: String, pub connection_label: String, pub action: String /* "read"|"send" */,
                       pub summary: String, pub expires_at: Option<i64>, pub max_uses: Option<u32>,
                       pub uses: u32 }
pub struct ConnectionView { pub id: String, pub label: String, pub client_host: String,
                            pub created_at: i64, pub last_used_at: Option<i64> }
pub struct ActivityEntry { pub at: i64, pub connection_label: String, pub action: String,
                           pub outcome: String /* "released"|"sent"|"denied"|"error" */,
                           pub detail: String, pub grant_id: Option<String> }
pub enum GmailStatus { Ready, NeedsConsent, Unavailable { message: String } }

#[derive(uniffi::Error)]
pub enum CoreError {
    NotLoggedIn, TwoFactorRequired, UnsupportedTwoFactor, InvalidCredentials,
    Network { message: String }, Server { status: u16, message: String },
    GmailNeedsConsent, Gmail { message: String }, NotFound, Invalid { message: String },
    Storage { message: String },
}
```

Foreign traits implemented in Kotlin:

```rust
#[uniffi::export(with_foreign)]   // or #[uniffi::export(foreign)] per UniFFI 0.32 syntax
pub trait KeyWrapper: Send + Sync {
    /// Android Keystore AES-256-GCM (non-exportable key). Output includes the IV.
    fn wrap(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, ForeignError>;
    fn unwrap(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, ForeignError>;
}
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait GoogleTokenProvider: Send + Sync {
    /// Fresh Gmail access token (scopes gmail.readonly + gmail.send) or NeedsUserInteraction.
    async fn access_token(&self) -> Result<String, ForeignError>;
}
#[uniffi::export(with_foreign)]
pub trait Notifier: Send + Sync {
    fn item_pending(&self, item: PendingItem);
    fn item_resolved(&self, id: String);
}
#[derive(uniffi::Error)] pub enum ForeignError { NeedsUserInteraction, Failed { message: String } }
```

Kotlin side names follow UniFFI defaults (camelCase methods, e.g. `core.handlePush(kind, id)`,
`ApprovalChoice(selectedMessageIds = …, standing = …)`).

## E. Local store (phone, Plan 3 internal)

SQLite at `<data_dir>/rewarden.db`. Secrets (Vaultwarden refresh token, audit `detail`) are
AES-256-GCM encrypted with a random 32-byte data key stored at `<data_dir>/dek.bin`
wrapped by `KeyWrapper`. Grants stored as `proto`/`policy` JSON.
