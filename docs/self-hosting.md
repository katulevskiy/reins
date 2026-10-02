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

**Docker:** `docker build -t reins-server .` builds an image with the server and the web vault (`Dockerfile` is
`docker/Dockerfile.debian`; `docker/Dockerfile.alpine` and `docker buildx bake` are described in
[docker/README.md](../docker/README.md)). The image keeps its data in `/data`; set the variables below with `-e` or an
env file.

## Configure

The server reads its settings from the environment, or from a `.env` file in its working directory. `.env.template`
documents every setting. The minimum for Reins:

```ini
DOMAIN=https://rewarden.example.com
ROCKET_ADDRESS=127.0.0.1
ROCKET_PORT=8000
DATA_FOLDER=/var/lib/rewarden
IP_HEADER=X-Real-IP              # whichever header your proxy sets with the client's address

REWARDEN_ENABLED=true
REWARDEN_FCM_SERVICE_ACCOUNT=/etc/rewarden/fcm-service-account.json   # empty: no push
# REWARDEN_RELAY_WAIT_SECS=45    # how long a tool call waits for the phone, 1..=55
# REWARDEN_OFFLINE_SECS=10       # a request the phone has not fetched by then is reported as "device offline"
# REWARDEN_PURGE_SCHEDULE="0 25 * * * *"   # cron: purge expired refresh tokens, files and memory entries

SIGNUPS_ALLOWED=true             # turn off once your accounts exist, or use invitations
```

| Setting | Notes |
|---|---|
| `DOMAIN` | Required. Must be `https://` (plain `http://` only for `localhost`, `127.0.0.1`, `[::1]`). It becomes the OAuth issuer and the MCP resource `{DOMAIN}/mcp`. Serve Reins at the root of a host name: the `/.well-known/oauth-*` documents must be at the root. |
| `REWARDEN_ENABLED` | Mounts `/mcp`, `/rewarden/oauth/*`, `/.well-known/oauth-*`, `/rewarden/api/*` (phone), `/rewarden/desktop/*` and `/rewarden/blob/*`. |
| `REWARDEN_FCM_SERVICE_ACCOUNT` | Path to a Firebase service-account JSON key. Keep it out of the repository, mode 0600 or 0640. |
| `REWARDEN_RELAY_WAIT_SECS` | ChatGPT aborts tool calls after 60 s; Claude allows longer. After this time the AI is told to call `rewarden_get_result` later. |
| `REWARDEN_OFFLINE_SECS` | Must be at most `REWARDEN_RELAY_WAIT_SECS`. |
| `REWARDEN_TEST_ALLOW_LOOPBACK` | For the test suite only. Never set it in production. |

Everything else (database URL, SMTP, `ADMIN_TOKEN`, two-factor options) works as in Vaultwarden.

## Accounts and the web vault

People need an account before the phone can sign in. There are three ways to create one:

- **The web vault.** Download a Vaultwarden web-vault build (the `docker/` files pin the version this fork is tested
  with, `vaultwarden/web-vault` v2026.7.0), unpack it, and set `WEB_VAULT_FOLDER` to it (`WEB_VAULT_ENABLED=true` is
  the default). Visitors to `https://<domain>/` can then **Create account** while `SIGNUPS_ALLOWED=true`.
- **Any Bitwarden client** set to your server's URL can register an account.
- **From a script:** `REWARDEN_PASSWORD='...' cargo run -p rewarden-e2e --example register_account -- https://<domain>
  you@example.com`. The account it makes signs in to Reins but has a placeholder vault key, so it cannot be used as
  a real password vault.

When the accounts exist, set `SIGNUPS_ALLOWED=false`, or use Vaultwarden's invitations and admin page.

## Reverse proxy

Requirements:

- Forward the client address (`X-Real-IP` or `X-Forwarded-For`) and set `IP_HEADER` to match. Rate limits key on the
  client address. If every request seems to come from the proxy, all clients share one limit.
- **Read timeout of at least 75 s** on `/mcp`, `/rewarden/desktop/calls` and `/rewarden/api/pending`. These are long
  polls: up to 45 s of relay wait, and 25 s of phone polling.
- Pass `/.well-known/oauth-authorization-server` and `/.well-known/oauth-protected-resource`, including the
  path-suffixed forms such as `/.well-known/oauth-protected-resource/mcp`.
- Do not cache, buffer or rewrite `/mcp`, `/rewarden/*` or `/.well-known/*`.

Caddy:

```caddyfile
rewarden.example.com {
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
downloads, big MCP results. They are written to `$DATA_FOLDER/rewarden-blobs/` (mode 0700, random names). Each file is
deleted when its operation is done, and after at most an hour in any case. The directory is emptied at every start.
Limits: 1 GiB per file, 20 files and 2 GiB per user, 8 GiB in total.

Let bodies up to 1 GiB through on `/rewarden/blob/`, and stream them instead of buffering. nginx:

```nginx
location /rewarden/blob/ {
    client_max_body_size 1g;
    proxy_request_buffering off;
    proxy_buffering off;
    proxy_read_timeout 300s;
    proxy_send_timeout 300s;
    proxy_pass http://127.0.0.1:8000;
}
```

Caddy streams by default. If you configured a body limit, raise it with `request_body { max_size 1GB }`.

The path after `/rewarden/blob/` is a capability: whoever has the URL may upload or download that one file. Reins
never logs it. Turn your proxy's access log off for this location, or strip the path from it.

The server also makes outgoing HTTPS requests to URLs the phone names: GitHub uploads, release assets, MCP servers.
Outgoing HTTPS to the internet must work. Private, loopback, link-local and cloud metadata addresses are always
refused.

## Push notifications (Firebase)

Without push, the app receives requests only while it is open (it long-polls `/rewarden/api/pending`). With push, a
data-only Firebase message carrying just a request id wakes the phone, and the phone then fetches the request from
your server. No request content goes through Google.

1. Create a Firebase project and add an Android app with the package `dev.rewarden.android`.
2. Create a service account with the role *Firebase Cloud Messaging Admin* only, download its JSON key, and point
   `REWARDEN_FCM_SERVICE_ACCOUNT` at it.
3. Build the Android app with that project's `google-services.json` (see `android/README.md`):
   ```sh
   cd android
   ./gradlew assembleFullRelease -Prewarden.googleServicesJson=/path/to/google-services.json \
       -Prewarden.defaultServer=https://rewarden.example.com
   ```

**The published APK belongs to the hosted service's Firebase project.** With your own server it signs in and works,
but it gets no push. Requests then reach the phone only while the app is open. Build your own APK as above for push.

Gmail, Google Calendar and Google Contacts are reached by the phone through Google Play services. They depend on the
Google Cloud OAuth client registered for the APK's package and signing certificate, not on the server. If you build
your own APK, register an Android OAuth client for your signing key, and enable the Gmail, Calendar and People APIs
with the scopes `gmail.readonly`, `gmail.send`, `calendar.events`, `calendar.readonly` and `contacts.readonly`. The
Gmail scopes are *restricted*: beyond 100 users Google requires verification and a security assessment. Telegram needs
an `api_id`/`api_hash` from <https://my.telegram.org>, compiled into the app (`rewarden.telegramApiId`,
`rewarden.telegramApiHash` in `~/.gradle/gradle.properties`).

## Desktop app against your server

```sh
rewarden login https://rewarden.example.com
```

Nothing else is needed on the server. Updates come from `releases` in `~/.config/rewarden/config.toml`, by default the
hosted release site. The phone downloads the Autopilot model from the hosted site as well. Its files are checked
against SHA-256 hashes built into the app, so where they come from does not change what is installed.

## Smoke test

```sh
curl -s https://rewarden.example.com/.well-known/oauth-protected-resource/mcp
# JSON whose "resource" is https://rewarden.example.com/mcp
curl -si -X POST https://rewarden.example.com/mcp -d '{}'
# 401 with a WWW-Authenticate header
```

Then sign in on the phone, connect an AI, and ask it to search your mail or list your repositories. A notification
should appear, and the AI gets the result only after you approve. With the phone offline, the AI is told to open the
app within `REWARDEN_OFFLINE_SECS`.

## Operating

- Back up `DATA_FOLDER`. It holds accounts, vaults, AI connections and the server's signing key. Losing it signs
  everyone out. Grants, the activity log and Autopilot's memory live on the phones and are not affected.
- Rotating the Firebase key: create the new key, replace the file, restart, then delete the old key.
- Logs never contain tokens, message contents or tool arguments. `LOG_LEVEL=info` is safe.
- The AI sees tools only for integrations the phone has reported. After a server restart, every tool is listed until
  the phone next syncs. AI clients may need to reconnect to refresh their tool list.
