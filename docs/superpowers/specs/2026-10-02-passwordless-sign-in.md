# Passwordless sign-in, the keyless vault, and another phone

Date: 2026-10-02. Status: implemented (server, core, e2e, iOS and Android apps).

A new user never sets a password: "Continue" in the phone apps signs in through WorkOS AuthKit (Google, Apple,
GitHub, Microsoft, an email code; WorkOS verifies emails and keeps bots out), the phone makes the vault's keys itself,
and WorkOS's user lifecycle follows into Reins.

## Sign-in (server: `src/sso.rs`, `src/sso_workos.rs`; core: `crates/rewarden-core/src/sso.rs`, `account.rs`)

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

`SSO_ONLY=true` turns password sign-in off: the hosted server's setting. Self-hosted servers keep it `false` (default)
and the apps show the master-password forms under "Use another server".

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

## Another phone (proto: `rewarden-proto/src/join.rs`; server: `src/api/rewarden/join.rs`; core: `join.rs`)

In memory on the server for 10 minutes (`ITEM_TTL`), at most 3 open per account, one per device.

| Call | Who | Body / answer |
|---|---|---|
| `POST /rewarden/api/joins` | a signed-in device of the account that is not its approval device (409 when the account has none) | `NewJoin {v, device_name, public_key}` (X25519, base64url) → `JoinCreated {id, expires_at}`; push `{t: "join", id}` to the approval device |
| `GET /rewarden/api/pending` | the approval device | `Pending.joins: [JoinRequest {v, id, device_name, public_key, created_at}]` |
| `GET /rewarden/api/joins/<id>` | the asking device | `JoinState {status: waiting \| approved \| denied \| expired, sealed?}` |
| `GET /rewarden/api/joins/<id>` | the approval device | `JoinRequest` while waiting |
| `POST /rewarden/api/joins/<id>/response` | the approval device | `JoinAnswer {v, approve, sealed?}` → 204; 404 gone, 409 answered |

- Both phones show `join_code(public_key)`: six digits of `SHA-256("reins-join-key/1" || key)`, "482 193". The approval
  device computes it from the key the server relayed, so a swapped key shows a different code.
- `sealed` = base64url(crypto_box sealed box to the new phone's key of `SealedSecret {v, join_id, secret}`); the new
  phone checks `join_id` and that the secret opens the account's keys before keeping it.
- The core parks a join as `PendingKind::Join` (`action: "join"`, store migration 9 adds the kind); Autopilot never
  judges it. `answer_join(id, true)` needs the account secret on this phone (accounts made with a master password
  have none: they sign in with it on the new phone).
- After `Joined`, the new phone registers as the approval device like any sign-in: the old phone gets `replaced`.

## WorkOS lifecycle (server: `src/api/rewarden/workos_sync.rs`)

A task polls `GET https://api.workos.com/events?events=user.updated,user.deleted,session.revoked&limit=100&after=<id>`
every `REWARDEN_WORKOS_SYNC_SECS` (30; 0 disables) and on a signed webhook delivery (`POST /rewarden/workos/webhook`,
`WorkOS-Signature: t=<ms>, v1=<hex HMAC-SHA256("<t>.<body>")>`, 5 minutes tolerance, `REWARDEN_WORKOS_WEBHOOK_SECRET`;
the body is ignored). The cursor (`rewarden_settings` `workos.events.after`) moves after each applied event.

- `user.updated` with `email_verified: true` and a new email: the account's email changes (refused and logged when
  another account has it); the name follows.
- `user.deleted`: the Reins data (`rewarden_connections` and their refresh tokens, `rewarden_devices`,
  `rewarden_sso_sessions`, `sso_users`) and the Vaultwarden account are deleted.
- `session.revoked`: the device that signed in with that session (`rewarden_sso_sessions`) loses its Vaultwarden
  device row, so its refresh token and access tokens stop working at once.

## Checks

- `cargo test -p rewarden-e2e --test sso_workos`: the real server and phone cores against a fake WorkOS (sign-in,
  keys, recovery code, email change, revoked session, deleted user, another phone approved and denied).
- `scripts/workos-live.sh`: the same against a WorkOS staging environment, headless (Chrome on AuthKit's hosted page).
