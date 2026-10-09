# Self-hosting

The Reins server is Vaultwarden with the Reins relay switched on. It is one Rust binary. It serves the
Bitwarden-compatible account and vault API, the MCP endpoint for AIs, an OAuth 2.1 authorization server, the phone
API and the desktop app API.

What the server holds, and what it never holds, is described in [security-model.md](security-model.md#what-the-server-sees).

## What you need

- A host with a public `https://` name. AI clients and phones must reach it, and Claude's connectors need an IPv4
  address.
- A reverse proxy that terminates TLS (Caddy, nginx, a Cloudflare Tunnel, ...).
- Optional: a Firebase project for push notifications ([below](#push-notifications-firebase)).
- Disk space for large file transfers: up to 8 GiB under `DATA_FOLDER` ([below](#large-files)).

Run **exactly one instance** per deployment. Waiting requests, pairings and file metadata live in that process's
memory. A restart drops calls in flight; AI clients retry them.

## Build the server

The toolchain is pinned in `rust-toolchain.toml`.

```sh
git clone https://github.com/katulevskiy/reins && cd reins
# SQLite (bundled). Use --features mysql or --features postgresql for those databases.
cargo build --release --features sqlite --bin vaultwarden
# A portable Linux binary: add vendored_openssl
cargo build --release --features sqlite,vendored_openssl --bin vaultwarden
```

The binary is `target/release/vaultwarden`. Run it as its own user, with `DATA_FOLDER` owned by that user and mode
0700.

**Prebuilt:** every [release](https://github.com/katulevskiy/reins/releases) has the server image on GitHub's
container registry, for linux/amd64 and linux/arm64, with the web vault and all three databases:

```sh
docker pull ghcr.io/katulevskiy/reins-server:latest     # or a version: ghcr.io/katulevskiy/reins-server:0.1.0
docker run -d --name reins -v /srv/reins:/data -p 127.0.0.1:8000:80 --env-file reins.env \
  ghcr.io/katulevskiy/reins-server:latest
```

The image listens on port 80 inside the container and keeps its data in `/data`, so leave `ROCKET_ADDRESS`,
`ROCKET_PORT` and `DATA_FOLDER` out of the env file. Releases also have a Linux x86_64
binary with SQLite (`reins-server-<version>-x86_64-unknown-linux-gnu.tar.gz`, glibc 2.35 or newer), without the web
vault ([below](#accounts-and-the-web-vault)).

**Docker from source:** `docker build -t reins-server .` builds an image with the server and the web vault (`Dockerfile` is
`docker/Dockerfile.debian`; `docker/Dockerfile.alpine` and `docker buildx bake` are described in
[docker/README.md](../docker/README.md)). The image keeps its data in `/data`; set the variables below with `-e` or an
env file.

## Configure

The server reads its settings from the environment, or from a `.env` file in its working directory. `.env.template`
documents every setting. The minimum for Reins:

```ini
DOMAIN=https://reins.example.com
ROCKET_ADDRESS=127.0.0.1
ROCKET_PORT=8000
DATA_FOLDER=/var/lib/reins
IP_HEADER=X-Real-IP              # whichever header your proxy sets with the client's address

REINS_ENABLED=true
REINS_FCM_SERVICE_ACCOUNT=/etc/reins/fcm-service-account.json   # empty: no push to Android
# REINS_APNS_KEY_FILE=/etc/reins/AuthKey_ABC123DEFG.p8           # empty: no push to iPhone
# REINS_APNS_KEY_ID=ABC123DEFG
# REINS_APNS_TEAM_ID=DEF123GHIJ
# REINS_RELAY_WAIT_SECS=45    # how long a tool call waits for the phone, 1..=55
# REINS_OFFLINE_SECS=10       # a request the phone has not fetched by then is reported as "device offline"
# REINS_PURGE_SCHEDULE="0 25 * * * *"   # cron: purge expired refresh tokens, files and memory entries

SIGNUPS_ALLOWED=true             # turn off once your accounts exist, or use invitations
```

| Setting | Notes |
|---|---|
| `DOMAIN` | Required. Must be `https://` (plain `http://` only for `localhost`, `127.0.0.1`, `[::1]`). It becomes the OAuth issuer and the MCP resource `{DOMAIN}/mcp`. Serve Reins at the root of a host name: the `/.well-known/oauth-*` documents must be at the root. |
| `REINS_ENABLED` | Mounts `/mcp`, `/reins/oauth/*`, `/.well-known/oauth-*`, `/reins/api/*` (phone), `/reins/desktop/*` and `/reins/blob/*`. |
| `REINS_FCM_SERVICE_ACCOUNT` | Path to a Firebase service-account JSON key. Keep it out of the repository, mode 0600 or 0640. |
| `REINS_APNS_KEY_FILE`, `REINS_APNS_KEY_ID`, `REINS_APNS_TEAM_ID` | The APNs key (`.p8` file), its key id and your Apple team id, for push to the iOS app. Set all three or none; the server refuses to start with only some, or with a key it cannot read. Same file permissions as the Firebase key. |
| `REINS_APNS_TOPIC` | Bundle id of the iOS app. Default `com.reins2fa.app`; change it only for an app built under another bundle id. |
| `REINS_APPLE_TEAM_ID` | Apple team id of the iOS app, served in `/.well-known/apple-app-site-association` so that a computer's pairing QR code (`{DOMAIN}/pair?code=...`) opens in the app. Empty: `REINS_APNS_TEAM_ID`; neither: no app links (the link opens a page that offers the app). |
| `REINS_ANDROID_CERT_SHA256` | SHA-256 fingerprints of the Android app's signing certificates (comma-separated, `AB:CD:...`), served in `/.well-known/assetlinks.json` for the same links. Empty: none. |
| `REINS_RELAY_WAIT_SECS` | ChatGPT aborts tool calls after 60 s; Claude allows longer. After this time the AI is told to call `reins_get_result` later. |
| `REINS_OFFLINE_SECS` | Must be at most `REINS_RELAY_WAIT_SECS`. |
| `REINS_TEST_ALLOW_LOOPBACK` | For the test suite only. Never set it in production. |

Everything else (database URL, SMTP, `ADMIN_TOKEN`, two-factor options) works as in Vaultwarden.

## Accounts and the web vault

For WorkOS deployments (`REINS_ENABLED=true`, `SSO_ENABLED=true`, `SSO_PROVIDER=workos`), the phone uses hosted AuthKit
and the server accepts the authentication methods enabled in WorkOS by default (`REINS_WORKOS_REQUIRE_PASSKEY=false`).
Enable email and social providers in the correct WorkOS environment; passkey-only deployments can explicitly set
this option to true after configuring and testing AuthKit passkeys. Local password signup and sign-in are disabled automatically.
See [deployment.md](deployment.md#workos-passkeys-and-recovery-rollout).

For self-hosted password deployments, people need an account before the phone can sign in. There are four ways to create one:

- **The phone app.** **Create account** (with **Use another server** set to yours) registers the account with real
  vault keys, like a Bitwarden client, while `SIGNUPS_ALLOWED=true`. With `SIGNUPS_VERIFY=true` the app can sign in once the
  link in the server's welcome email was opened.
- **The web vault.** Download a Vaultwarden web-vault build (the `docker/` files pin the version this fork is tested
  with, `vaultwarden/web-vault` v2026.7.0), unpack it, and set `WEB_VAULT_FOLDER` to it (`WEB_VAULT_ENABLED=true` is
  the default). Visitors to `https://<domain>/` can then **Create account** while `SIGNUPS_ALLOWED=true`.
- **Any Bitwarden client** set to your server's URL can register an account.
- **From a script:** `REINS_PASSWORD='...' cargo run -p reins-e2e --example register_account -- https://<domain>
  you@example.com`. The account it makes signs in to Reins but has a placeholder vault key, so it cannot be used as
  a real password vault.

When the accounts exist, set `SIGNUPS_ALLOWED=false`, or use Vaultwarden's invitations and admin page.

## Sign-in without passwords (WorkOS AuthKit)

The hosted server signs people in through [WorkOS AuthKit](https://workos.com/docs/authkit): the phone apps'
**Continue** opens AuthKit's page (Google, Apple, GitHub, Microsoft or a code by email, with WorkOS checking the
email and keeping bots out), and nobody sets a password. Your server can do the same with your own WorkOS
environment:

```ini
SSO_ENABLED=true
SSO_AUTHORITY=https://api.workos.com/user_management/client_01ABC...   # your WorkOS client id at the end
SSO_CLIENT_ID=client_01ABC...
SSO_CLIENT_SECRET=sk_live_...       # the WorkOS API key
SSO_AUTH_ONLY_NOT_SESSION=true      # WorkOS signs people in; the server keeps its own sessions (see below)
SSO_SIGNUPS_MATCH_EMAIL=true        # an existing account with the same (WorkOS-verified) email is linked
# SSO_ONLY=true                     # no password sign-in at all (the hosted server's setting)
# SSO_AUTHORIZE_EXTRA_PARAMS="screen_hint=sign-up"   # extra AuthKit parameters
# REINS_WORKOS_SYNC_SECS=30
# REINS_WORKOS_WEBHOOK_SECRET=...
```

In the WorkOS dashboard (or through its API), add `https://<domain>/identity/connect/oidc-signin` to the redirect
URIs and turn on the sign-in methods you want under **Authentication**; Google, Apple and GitHub need their own
OAuth credentials for production (WorkOS's shared test credentials work in a staging environment).
Add `https://<domain>/reins/signed-out` to the allowed **Sign-out URIs** too. Mobile sign-out revokes only the
current phone's Reins session and visits WorkOS logout in the sign-in browser, then offers **Return to Reins**.
Offline sign-out still locks the local encrypted account. Mobile sign-in requests fresh authentication even if
the logout browser was closed before it finished.

How it fits together:

- The server speaks to WorkOS's User Management API directly (`SSO_PROVIDER=workos`, picked automatically for an
  authority on `api.workos.com`): AuthKit's discovery document names another issuer than its URL and has no userinfo
  endpoint, so a generic OpenID Connect client cannot use it. AuthKit verifies emails, so
  `SSO_ALLOW_UNKNOWN_EMAIL_VERIFICATION` stays `false`. A sign-in WorkOS marks as an impersonation is refused.
- The phone apps open `/identity/connect/authorize` with `client_id=mobile` and their own redirect,
  `com.reins2fa.app://sso-callback`, which the server allows in addition to the Bitwarden clients' ones.
- A new account gets its vault keys from the phone, protected by a random account secret instead of a master
  password ([security model](security-model.md#accounts-without-a-master-password)). Bitwarden clients can still sign
  in with SSO, but cannot open such a vault.
- `SSO_AUTH_ONLY_NOT_SESSION=true` keeps the server's own 30-day sessions, so approvals keep working when WorkOS is
  unreachable; WorkOS's revocations still reach the server through the sync. With `false`, every token refresh asks
  WorkOS (its access tokens live five minutes).
- **The WorkOS sync.** Every `REINS_WORKOS_SYNC_SECS` the server reads the WorkOS events after the last one it
  applied (the cursor survives restarts): a verified new email becomes the account's email, a deleted WorkOS user's
  account is deleted with its AI connections and devices, and a revoked WorkOS session signs out the device that
  signed in with it. Point a WorkOS webhook at `https://<domain>/reins/workos/webhook` with
  `REINS_WORKOS_WEBHOOK_SECRET` set to make this immediate; the webhook only wakes the sync, which still reads the
  events API.
- **Deleting an account in the app** (Settings → Delete account) deletes the WorkOS user too
  (`DELETE /user_management/users/<id>` with `REINS_WORKOS_API_KEY`, else `SSO_CLIENT_SECRET`), before the account
  itself; while WorkOS cannot, nothing is deleted and the app says to try again. Without WorkOS (password accounts, or
  another SSO provider) the account is deleted on the server alone. See the
  [security model](security-model.md#accounts-without-a-master-password) for what the request needs.
- `SSO_ONLY=true` turns password sign-in off for everyone, including accounts made with a password before; they get
  in through AuthKit with the same email (`SSO_SIGNUPS_MATCH_EMAIL=true`) and open their vault with their master
  password once. Without it, password sign-in stays available (the apps show it under **Use another server**).

`scripts/workos-live.sh` checks all of this against a WorkOS staging environment, headless: it makes a test user,
signs two phone cores in through AuthKit's page in headless Chrome, and has WorkOS change, revoke and delete.

## Reverse proxy

Requirements:

- Forward the client address (`X-Real-IP` or `X-Forwarded-For`) and set `IP_HEADER` to match. Rate limits key on the
  client address. If every request seems to come from the proxy, all clients share one limit.
- **Read timeout of at least 75 s** on `/mcp`, `/reins/desktop/calls` and `/reins/api/pending`. These are long
  polls: up to 45 s of relay wait, and 25 s of phone polling.
- Pass `/.well-known/oauth-authorization-server` and `/.well-known/oauth-protected-resource`, including the
  path-suffixed forms such as `/.well-known/oauth-protected-resource/mcp`.
- Do not cache, buffer or rewrite `/mcp`, `/reins/*` or `/.well-known/*`.

Caddy:

```caddyfile
reins.example.com {
    reverse_proxy 127.0.0.1:8000 {
        transport http {
            read_timeout 90s
        }
    }
}
```

nginx, the parts that matter:

```nginx
location / {
    proxy_pass http://127.0.0.1:8000;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_read_timeout 90s;
    proxy_buffering off;
}
```

If Cloudflare proxies the zone, its 100 s proxy timeout is above the long polls. That is fine.

### Large files

Files too large for a tool call pass through the server for one operation: uploads to GitHub or the vault, large
downloads, big MCP results. They are written to `$DATA_FOLDER/reins-blobs/` (mode 0700, random names). Each file is
deleted when its operation is done, and after at most an hour in any case. The directory is emptied at every start.
Limits: 1 GiB per file, 20 files and 2 GiB per user, 8 GiB in total.

Let bodies up to 1 GiB through on `/reins/blob/`, and stream them instead of buffering. nginx:

```nginx
location /reins/blob/ {
    client_max_body_size 1g;
    proxy_request_buffering off;
    proxy_buffering off;
    proxy_read_timeout 300s;
    proxy_send_timeout 300s;
    proxy_pass http://127.0.0.1:8000;
}
```

Caddy streams by default. If you configured a body limit, raise it with `request_body { max_size 1GB }`.

The path after `/reins/blob/` is a capability: whoever has the URL may upload or download that one file. Reins
never logs it. Turn your proxy's access log off for this location, or strip the path from it.

The server also makes outgoing HTTPS requests to URLs the phone names: GitHub uploads, release assets, MCP servers.
Outgoing HTTPS to the internet must work. Private, loopback, link-local and cloud metadata addresses are always
refused.

## Push notifications (Firebase)

Without push, the app receives requests only while it is open (it long-polls `/reins/api/pending`). With push, a
data-only Firebase message carrying just a request id wakes the phone, and the phone then fetches the request from
your server. No request content goes through Google.

1. Create a Firebase project and add an Android app with the package `com.reins2fa.app`.
2. Create a service account with the role *Firebase Cloud Messaging Admin* only, download its JSON key, and point
   `REINS_FCM_SERVICE_ACCOUNT` at it.
3. Build the Android app with that project's `google-services.json` (see `android/README.md`):
   ```sh
   cd android
   ./gradlew assembleFullRelease -Preins.googleServicesJson=/path/to/google-services.json \
       -Preins.defaultServer=https://reins.example.com
   ```

**The published APK belongs to the hosted service's Firebase project.** With your own server it signs in and works,
but it gets no push. Requests then reach the phone only while the app is open. Build your own APK as above for push.

Gmail, Google Calendar and Google Contacts are reached by the phone through Google Play services. They depend on the
Google Cloud OAuth client registered for the APK's package and signing certificate, not on the server. If you build
your own APK, register an Android OAuth client for your signing key, and enable the Gmail, Calendar and People APIs
with the scopes `gmail.modify`, `gmail.settings.basic`, `calendar.events`, `calendar.readonly` and `contacts.readonly`.
The Gmail scopes are *restricted*: beyond 100 users Google requires verification and a security assessment. Telegram needs
an `api_id`/`api_hash` from <https://my.telegram.org>, compiled into the app (`reins.telegramApiId`,
`reins.telegramApiHash` in `~/.gradle/gradle.properties`).

## Push notifications on iPhone (APNs)

The iOS app is woken by Apple's push service directly, without Firebase. Like the Android push, the notification
carries only an id and a fixed text ("Something is waiting for you"); the app fetches the request from your server and
shows it. No request content goes through Apple.

1. In the Apple Developer account, under *Certificates, Identifiers & Profiles* → *Keys*, create a key with *Apple Push
   Notifications service (APNs)* enabled. Download the `.p8` file (Apple lets you download it once) and note its key
   id. Your team id is shown under *Membership*.
2. Point `REINS_APNS_KEY_FILE` at the `.p8` file and set `REINS_APNS_KEY_ID` and `REINS_APNS_TEAM_ID`.
3. The key has to belong to the team that signs the iOS app, and `REINS_APNS_TOPIC` has to be the app's bundle id.

One key serves both of Apple's environments. The app registers its token as `apns:<token>` (App Store and TestFlight
builds) or `apns-sandbox:<token>` (development builds), and the server sends each to the matching Apple endpoint.
Tokens Apple reports as gone are forgotten, as with Firebase. The server talks to `api.push.apple.com` and
`api.sandbox.push.apple.com` over HTTPS (HTTP/2, port 443).

**The published iOS app belongs to the hosted service's Apple team.** With your own server it signs in and works, but
it gets no push unless your server has that team's key. Requests then reach the phone only while the app is open.

## Desktop app against your server

```sh
reins login https://reins.example.com
```

It shows a QR code for the phone to scan. The published apps open pairing links of the hosted domain only, so with
your own server scan the code in the app (**Settings → Connect a computer**) rather than with the camera, or type the
code. Nothing else is needed on the server. Updates come from `releases` in `~/.config/reins/config.toml`, by default the
hosted release site. The phone downloads the Autopilot model from the hosted site as well. Its files are checked
against SHA-256 hashes built into the app, so where they come from does not change what is installed.

## Smoke test

```sh
curl -s https://reins.example.com/.well-known/oauth-protected-resource/mcp
# JSON whose "resource" is https://reins.example.com/mcp
curl -si -X POST https://reins.example.com/mcp -d '{}'
# 401 with a WWW-Authenticate header
```

Then sign in on the phone, connect an AI, and ask it to search your mail or list your repositories. The apps come
filled in with the hosted server: replace it with yours (on iPhone under **Use another server**), or build the Android
app with `-Preins.defaultServer` as above. A notification should appear, and the AI gets the result only after you
approve. With the phone offline, the AI is told to open the
app within `REINS_OFFLINE_SECS`.

## Operating

- Back up `DATA_FOLDER`. It holds accounts, vaults, AI connections and the server's signing key. Losing it signs
  everyone out. Grants, the activity log and Autopilot's memory live on the phones and are not affected.
- Rotating the Firebase key: create the new key, replace the file, restart, then delete the old key.
- Logs never contain tokens, message contents or tool arguments. `LOG_LEVEL=info` is safe.
- The AI sees tools only for integrations the phone has reported. After a server restart, every tool is listed until
  the phone next syncs. AI clients may need to reconnect to refresh their tool list.
