# Vaultwarden 1.37.3 — extension points for Rewarden

Line numbers are for tag `1.37.3` (commit eb212e23). Verify before editing; upstream files may shift.

## Route mounting — `src/main.rs` `launch_rocket()` (563-630)

```rust
582  let instance = rocket::custom(config)
583      .mount([basepath, "/"].concat(), api::web_routes())
584      .mount([basepath, "/api"].concat(), api::core_routes())
...
589      .mount([basepath, "/notifications"].concat(), api::notifications_routes())
590      .register([basepath, "/"].concat(), api::web_catchers())
591      .register([basepath, "/api"].concat(), api::core_catchers())
593      .manage(pool)
594      .manage(Arc::clone(&WS_USERS))
596      .attach(util::AppHeaders()) .attach(util::Cors()) .attach(util::BetterLogging(extra_debug))
```
- `basepath = CONFIG.domain_path()`. Body limits json 20 MB.
- New base paths should be added to `LOGGED_ROUTES` in `src/util.rs:300`.
- Crate-level `#[macro_use]` for rocket, serde, serde_json, log, diesel; `err!`/`db_run!` usable everywhere.
- `src/api/mod.rs`: submodules 1-7, re-exports like `notifications::routes as notifications_routes` (25). Result aliases 44-46: `ApiResult<T> = Result<T, crate::error::Error>`, `JsonResult`, `EmptyResult`.
- `core::catchers()` is a JSON 404; no 401 catcher exists. `.well-known` only has apple-app-site-association (web.rs:207, only when web vault enabled) → Rewarden needs its own `/.well-known` mount.
- `util::AppHeaders` (28-148) sets CSP, X-Frame-Options, `Cache-Control: no-cache, no-store` unless set. `util::Cors` (150-208) allows only domain origin etc. and answers every OPTIONS with 200.
- Rocket 0.5.1 has `rocket::response::stream::EventStream`; `rocket_ws` available.

## Auth — `src/auth.rs`
- RS256, single RSA keypair (`initialize_keys()` 69-103). `encode_jwt<T: Serialize>(claims) -> String` (105-110). `decode_jwt<T>(token, issuer: String)` (112-129) validates exp/nbf, leeway 30 s, issuer. Token kinds differ only by `iss` = `format!("{}|<kind>", CONFIG.domain_origin())` (50-64).
- To add `|mcp`: static issuer LazyLock, claims struct (nbf, exp, iss, sub, …), `generate_*`, `decode_*` wrapper, `FromRequest` guard. Worked example: `src/auth/send.rs` (wired as `#[path = "auth/send.rs"] pub mod send;` at auth.rs:1-4).
- `LoginJwtClaims` (179-219): nbf, exp, iss, sub: UserId, premium, name, email, email_verified, sstamp, device: DeviceId, devicetype, client_id, scope, amr. Access 2 h; refresh 30 d (90 d mobile).
- Guards: `Headers { host, device: Device, user: User, ip: ClientIp }` (619-706) — decodes `Authorization: Bearer`, loads Device by (claims.device, claims.sub), User, checks security stamp; failures → 401 via `err_handler!`. `ClientHeaders { device_type, ip }` (592-617, unauthenticated). `ClientIp` (1059-1126). `Host` (551-590).
- `src/api/identity.rs`: `POST /identity/connect/token` (59-150) grant types refresh_token, password, client_credentials, authorization_code(SSO), send_access; form struct `ConnectData` (1150-1211). **`GET /identity/connect/authorize` (1335-1369) is the SSO endpoint — do not reuse; Rewarden OAuth lives under `/rewarden/oauth/*`.** PKCE helper used for SSO: `openidconnect::PkceCodeChallenge::from_code_verifier_sha256` (sso_client.rs:241).
- Device model `src/db/models/device.rs`: PK (uuid, user_uuid); fields name, atype, push_uuid, push_token, refresh_token, twofactor_remember. `DeviceType` Android = 0 (290-346). Finders `find_by_uuid_and_user` (183) etc. `DeviceId(String)` newtype (384).

## Database
- Model template `src/db/models/auth_request.rs` (200 lines): `#[derive(Identifiable, Queryable, Insertable, AsChangeset, Deserialize, Serialize)]`, `#[diesel(table_name = …)]`, `#[diesel(treat_none_as_null = true)]`, `#[diesel(primary_key(uuid))]`; `save` upsert with per-backend branches via `db_run! { conn: mysql {…} postgresql, sqlite {…} }`; finders `conn.run(move |conn| table.filter(..).first::<Self>(conn).ok()).await`; delete via `diesel::delete`.
- Smaller example: `src/db/models/archive.rs` (98 lines).
- `DbConn::run` (db/mod.rs 322-335) uses `block_in_place`. Handlers take `conn: DbConn`; background jobs `pool.get().await`.
- Register models in `src/db/models/mod.rs` (`mod x; pub use self::x::{…};`).
- `src/db/schema.rs` is hand-maintained for all backends: `table! { name (pk) { col -> Text, … } }`, then `joinable!` and `allow_tables_to_appear_in_same_query!` lists — add new tables to both.
- Migrations: `migrations/{sqlite,mysql,postgresql}/YYYY-MM-DD-HHMMSS_name/{up,down}.sql`, same folder name in all three. Dialects (from `2026-03-09-005927_add_archives`): sqlite `CHAR(36) … REFERENCES users (uuid) ON DELETE CASCADE`, `DATETIME`; mysql separate `FOREIGN KEY` clauses, `TIMESTAMP`; postgresql inline REFERENCES, `TIMESTAMP`.
- Build needs a DB feature: `cargo build --features sqlite`. CI tests `--features sqlite,mysql,postgresql`.

## Notifications / push (not reusable for Rewarden)
- `WS_USERS` keyed by user id only; cannot target a device; client→server frames ignored. Bitwarden push relay (`src/api/push.rs`) only reaches official Bitwarden apps; `send_to_push_relay` is private. → Rewarden uses its own FCM sender and HTTPS long-poll.
- Closest analog flow: login-with-device `auth_requests` (accounts.rs 1593-1828): DB row + notify + polling; purge job every 30 s deletes rows > 15 min (`purge_auth_requests`, main.rs 716-720). No in-memory waiters exist anywhere (no oneshot/Notify).

## Config — `src/config.rs`
- `make_config!` (58-487); format doc 489-501: `/// Name |> Description` then `name: type, is_editable, action, <default>;` action ∈ def|auto|option|generated. Group example `push { push_enabled: bool, false, def, false; … }` (527-538). Getter `CONFIG.push_enabled()`; env var = upper-case name. Cross-field checks in `validate_config` (941+). Document options in `.env.template`. Jobs in `schedule_jobs` (main.rs 661-761).

## Errors / rate limiting
- `src/error.rs`: `Error { message, kind, code: u16 (default 400), … }` with `From` for diesel, reqwest, serde_json, jsonwebtoken, io. Macros `err!`, `err_code!(msg, code)`, `err_json!(value, log)`, `err_handler!` (guards → 401). The Responder always emits Bitwarden-shaped JSON → MCP (JSON-RPC) and OAuth (RFC 6749) endpoints need their own responders.
- `src/ratelimit.rs`: governor limiters keyed by IpAddr; `check_limit_unauthenticated(&ip)` etc. → 429.

## Tests & lints
- Very few Rust tests upstream; no `tests/` dir; Playwright e2e in `playwright/`.
- Workspace lints: `warnings = deny`, `unsafe_code = forbid`, clippy pedantic (CI denies). A crate needing `unsafe` (UniFFI scaffolding) must not inherit `[lints] workspace = true` wholesale.
- Toolchain pinned 1.98.1 (`rust-toolchain.toml`), edition 2024.

## Reusable deps (Cargo.lock)
rocket 0.5.1, rocket_ws 0.1.1, tokio 1.53.1 (rt-multi-thread, time, sync available), futures 0.3.34, serde 1.0.229, serde_json 1.0.151, dashmap 6.2.1, reqwest 0.13.5 (use `crate::http_client::make_http_request`, SSRF-guarded), jsonwebtoken 11.0.0, diesel 2.3.13, derive_more 2.1.1, uuid 1.26.0 (`util::get_uuid()`), chrono 0.4.45, governor 0.10.4, moka 0.12.16 (future cache), data-encoding 2.11.1, ring 0.17.14 (`crypto::sha256_hex`), subtle 2.6.1 (`crypto::ct_eq`), rand 0.10.2 (`crypto::encode_random_bytes::<N>(&BASE64URL)`), openidconnect 4.0.1, url 2.5.8, job_scheduler_ng 2.5.0.
