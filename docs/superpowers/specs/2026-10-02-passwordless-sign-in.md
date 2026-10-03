# Passwordless sign-in, the keyless vault, and another phone

Date: 2026-10-02. Status: implemented (server, core, e2e, iOS and Android apps).

A new user never sets a password: "Continue" in the phone apps signs in through WorkOS AuthKit (Google, Apple,
GitHub, Microsoft, an email code for legacy deployments; hosted Reins requires a passkey), the phone makes the vault's keys itself,
and WorkOS's user lifecycle follows into Reins.

## Sign-in (server: `src/sso.rs`, `src/sso_workos.rs`; core: `crates/reins-core/src/sso.rs`, `account.rs`)

The fork's Vaultwarden SSO flow, with WorkOS spoken to through its User Management API (`SSO_PROVIDER=workos`, picked
automatically for an `SSO_AUTHORITY` on `api.workos.com`). AuthKit's OpenID discovery document is unusable by a generic
OpenID Connect client: its `issuer` is another client id than the one in its URL, it has no `userinfo_endpoint`, no
`id_token_signing_alg_values_supported`, and its token endpoint answers with the user instead of an id token.

1. Core `sso_begin(server)`: `state` (16 random bytes), PKCE `verifier` (32 random bytes, base64url), and
   `GET {server}/identity/connect/authorize?client_id=mobile&redirect_uri=com.reins2fa.app://sso-callback&
   response_type=code&response_mode=query&scope=api offline_access&state&code_challenge&code_challenge_method=S256`.
   The app keeps `state` and `verifier` (Android may restart it while the Custom Tab is open).
2. Server `authorize`: client `mobile` may use `com.reins2fa.app://sso-callback` (`sso::REINS_APP_REDIRECTS`), else the
   Bitwarden redirects as before. It stores the `SsoAuth` row, sets the browser-binding cookie, and redirects to
   `https://api.workos.com/user_management/authorize?client_id&redirect_uri={DOMAIN}/identity/connect/oidc-signin&
   response_type=code&state=<base64 state>&provider=authkit&code_challenge=<the app's>&code_challenge_method=S256`
   (plus `SSO_AUTHORIZE_EXTRA_PARAMS`; `provider` is left out when they pick a provider, connection or organization).
3. AuthKit signs the person in and redirects to `oidc-signin?code&state`; the server checks the cookie and redirects to
   `com.reins2fa.app://sso-callback?code&state&scope&iss`.
4. Core `sso_finish(server, callback, state, verifier)`: checks scheme, host and `state`, then
   `POST {server}/identity/connect/token` (`grant_type=authorization_code`, `code`, `code_verifier`, `redirect_uri`,
   `client_id=mobile`, `scope`, device fields).
5. Server: `POST https://api.workos.com/user_management/authenticate {grant_type: authorization_code, client_id,
   client_secret: <API key>, code, code_verifier}` (PKCE end to end: WorkOS checks the app's verifier). The answer's
   `user {id, email, email_verified, first_name, last_name}` is the identity (no id token to verify: it came from the
   token endpoint over TLS, authenticated by the API key); the access token's `sid` is kept for the lifecycle sync.
   An answer with `impersonator` is refused. The identifier is `{SSO_AUTHORITY}/{user id}` in `sso_users`. Then the
   fork's account logic: a new user is created (verified email required), an existing password account with the same
   verified email is linked when `SSO_SIGNUPS_MATCH_EMAIL=true`. A verified email that differs from the account's
   becomes the account's (unless another account has it).
6. With `SSO_AUTH_ONLY_NOT_SESSION=true` (recommended) the server issues its own tokens; otherwise refreshes go to
   `authenticate` with `grant_type=refresh_token` (WorkOS access tokens live five minutes).

With `REINS_ENABLED=true` and WorkOS, password sign-in, password-token refresh and local account registration are
always disabled, even if `SSO_ONLY` is omitted. Other self-hosted identity providers retain their existing settings.
`REINS_WORKOS_REQUIRE_PASSKEY=true` (default) also checks the server-to-server WorkOS authentication response:
only `authentication_method: "Passkey"` may produce a Reins session. A missing method, password, social OAuth or email
code is refused with an actionable message. `false` is an explicit legacy/staging exception, used by the headless
Magic Auth harness only.

Enable passkeys on the production AuthKit custom domain **before** deploying this policy. WorkOS supports passkeys
through hosted AuthKit only; progressive enrollment can be skipped and currently applies to password users. It has
no public native passkey enrollment API or passkey management screen. Consequently, social/email sign-in alone
cannot satisfy the policy: the user must enroll and authenticate with a passkey through AuthKit. Configure and
verify the production signup journey before rollout; do not describe optional enrollment as mandatory.
See [WorkOS passkeys](https://workos.com/docs/authkit/passkeys) and the
[authentication response](https://workos.com/docs/reference/authkit/authentication).

## The keyless vault (core: `sso.rs`, `account.rs`)

- **Account secret:** 32 random bytes. Its "password" is its base32 (RFC 4648, no padding, 52 characters); the master
  key is `PBKDF2-SHA256(password, salt = "reins-account-secret-v1", 100 000)` (`SECRET_SALT`, `SECRET_KDF`), the
  master password hash as Bitwarden's (`PBKDF2(master key, password, 1)`). The salt does not follow the email, so a
  WorkOS email change leaves the vault openable.
- **New account** (`Key` absent from the token answer): `crypto::new_account_keys(password, SECRET_SALT, SECRET_KDF)`
  (random 64-byte user key wrapped with the stretched master key; RSA-2048, private key wrapped with the user key),
  then `POST /api/accounts/set-password {masterPasswordHash, key, keys {publicKey, encryptedPrivateKey}, kdf 0,
  kdfIterations 100000}`. The secret is stored before the call (store secrets: `reins.account-secret` / server user
  id), the vault key after it (`vault` / email, and the vault is registered as a connected integration).
- **Existing keys:** opened with the stored secret (`Unlocked`), else `Locked`: the app shows the Unlock screen and
  does not register as approval device.
- **Recovery code:** the base32 in 13 groups of four, `ABCD-EFGH-...`. `unlock_account` takes it (case, spaces and
  dashes ignored) or, when the text is not a code, a master password (prelogin KDF, email salt), for accounts made
  with one.

## Another phone (proto: `reins-proto/src/join.rs`; server: `src/api/reins/join.rs`; core: `join.rs`)

In memory on the server for 10 minutes (`ITEM_TTL`), at most 3 open per account, one per device.

| Call | Who | Body / answer |
|---|---|---|
| `POST /reins/api/joins` | a signed-in device of the account that is not its approval device (409 when the account has none) | `NewJoin {v, device_name, public_key}` (X25519, base64url) → `JoinCreated {id, expires_at}`; push `{t: "join", id}` to the approval device |
| `GET /reins/api/pending` | the approval device | `Pending.joins: [JoinRequest {v, id, device_name, public_key, created_at}]` |
| `GET /reins/api/joins/<id>` | the asking device | `JoinState {status: waiting \| approved \| denied \| expired, sealed?}` |
| `GET /reins/api/joins/<id>` | the approval device | `JoinRequest` while waiting |
| `POST /reins/api/joins/<id>/response` | the approval device | `JoinAnswer {v, approve, sealed?}` → 204; 404 gone, 409 answered |

- Both phones show `join_code(public_key)`: six digits of `SHA-256("reins-join-key/1" || key)`, "482 193". The approval
  device computes it from the key the server relayed, so a swapped key shows a different code.
- `sealed` = base64url(crypto_box sealed box to the new phone's key of `SealedSecret {v, join_id, secret}`); the new
  phone checks `join_id` and that the secret opens the account's keys before keeping it.
- The core parks a join as `PendingKind::Join` (`action: "join"`, store migration 9 adds the kind); Autopilot never
  judges it. `answer_join(id, true)` needs the account secret on this phone (accounts made with a master password
  have none: they sign in with it on the new phone).
- After `Joined`, the new phone registers as the approval device; the approval is its proof (below), and the old phone
  gets `replaced`.

## Taking the approval role (server: `src/api/reins/device_api.rs`; core: `engine.rs` `register_device`)

Signing in proves only that someone controls the identity (WorkOS) or knows the password; it no longer makes a device
the approval device by itself.

- **Device key.** The core makes 32 random bytes once (store secret `reins.device-key`) and sends them, base64url,
  as `Reins-Device-Key` with every phone-API call (`PhoneApi`, `mcp/server_api.rs`; never to another host). The server
  keeps `SHA-256` hex in `reins_devices.key_hash` (migration `2026-10-02-100000_reins_device_key`). The caller is
  the approval device when the Vaultwarden device id and the key both match (`is_caller`); a row without a key (from
  before) matches on the id and takes the key of its device's next registration.
- **`PUT /reins/api/device`** (`DeviceRegistration {fcm_token, master_password_hash?}`): no proof when the account
  has no approval device or the caller is it. Otherwise one of:
  1. an approval of the caller's join request (`JoinHub::take_takeover`): status approved, within `TAKEOVER_TTL`
     (5 minutes) of the answer, asked with the caller's device key, spent by the first use;
  2. `master_password_hash` that `User::check_valid_password` accepts: the hash of the account secret's "password"
     (`AccountSecret::master_password_hash`) or of the master password (the login hash).

  Missing: `403 proof_required`; wrong: `403 wrong_proof`, counted per account (`limits::DEVICE_PROOFS`,
  `REINS_DEVICE_PROOF_MAX_FAILURES` = 5 in `REINS_DEVICE_PROOF_WINDOW_SECONDS` = 900); at the limit every proof
  gets `429 rate_limited` until the oldest failure ages out. Both 403s carry the message
  `reins_proto::device::TAKEOVER_REFUSED`.
- **Core.** `register_device` first sends no proof; on a 403 above it computes one (the hash of the password this
  session signed in or unlocked with, kept in memory until the registration succeeds, else of the account secret in
  the store; PBKDF2 off the async threads) and tries once more. Without a proof, or refused again:
  `CoreError::OtherApprovalDevice`, whose text is that message.
- **Apps.** On `OtherApprovalDevice` the phone shows the Unlock screen ("Another phone approves for this account") with
  that message and its two ways: **Ask my other phone** (the join flow: the approval is the proof) and **Enter recovery
  code** (`unlock_account`, which keeps the secret, or the master password's hash); then `register_device` again and on
  as after a sign-in. iOS: `SessionState.otherApprovalDevice`, not kept across launches (the next launch is refused
  again); a quietly refused phone that thought it approved shows the "replaced" banner instead, and **Use this phone**
  leads to the screen. Android: a flag kept in `DeviceStatusStore`, a **Not now** link back to the app, and
  `RegisterDeviceWorker` gives up on this error instead of retrying.

## WorkOS lifecycle (server: `src/api/reins/workos_sync.rs`)

A task polls `GET https://api.workos.com/events?events=user.updated,user.deleted,session.revoked&limit=100&after=<id>`
every `REINS_WORKOS_SYNC_SECS` (30; 0 disables) and on a signed webhook delivery (`POST /reins/workos/webhook`,
`WorkOS-Signature: t=<ms>, v1=<hex HMAC-SHA256("<t>.<body>")>`, 5 minutes tolerance, `REINS_WORKOS_WEBHOOK_SECRET`;
the body is ignored). The cursor (`reins_settings` `workos.events.after`) moves after each applied event.

- `user.updated` with `email_verified: true` and a new email: the account's email changes (refused and logged when
  another account has it); the name follows. The normal approval-phone poll includes the email after its wait, so
  existing phones migrate their vault alias, grants and saved session atomically before token expiry.
- `user.deleted`: the account is disabled and its device tokens invalidated before deletion. The Reins data (`reins_connections` and their refresh tokens, `reins_devices`,
  `reins_sso_sessions`, `sso_users`) and the Vaultwarden account are deleted. The account id is queued durably before removing its identity; failed vault-account
  cleanup retries without blocking later lifecycle events. A last organization owner needs administrator intervention while the account remains disabled.
- `session.revoked`: the device that signed in with that session (`reins_sso_sessions`) loses its Vaultwarden
  device row, so its refresh token and access tokens stop working at once. Deleting the device and session mapping is
  transactional, so database errors leave both intact for a retry.

## Checks

- `cargo test -p reins-e2e --test sso_workos`: the real server and phone cores against a fake WorkOS (sign-in,
  keys, recovery code, email change, revoked session, deleted user, another phone approved and denied, a phone that
  signed in to the identity refused the approval role until it has the recovery code or the approval).
- `cargo test --features sqlite --test reins_server takeover`: the takeover rule against the real server (first
  device, same device, another device without, with a right and with wrong proofs, the rate limit, a join approval
  spent once, a device id claimed without its key).
- `scripts/workos-live.sh`: the same against a WorkOS staging environment, headless (Chrome on AuthKit's hosted page).
