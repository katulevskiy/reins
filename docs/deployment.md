# Deploying Reins

Reins is Vaultwarden with the AI permission relay switched on. One server, one Android app per user. This guide is
for a public deployment such as `https://rewarden.example.com`. Nothing here is automated on purpose: DNS, TLS and
the Google consent screen are decisions for whoever owns the domain and the Google Cloud project.

## What runs where

| Piece | Where | Holds |
| --- | --- | --- |
| Reins server (this repo's `vaultwarden` binary) | your host | user accounts, hashed OAuth tokens, connection labels, device registration. **No Gmail credentials, ever.** |
| Reins Android app + `rewarden-core` | the user's phone | Gmail access (Google Play services), grants, encrypted local store, audit log |
| Claude / ChatGPT | their clouds | an OAuth token for `https://<domain>/mcp` |

The server is a blind relay: it forwards a request to the phone, waits for the phone's answer, hands it back to the AI.
If the phone is offline the AI is told to ask the user to open the Reins app.

## 1. Build the server

```bash
# binary (SQLite; use mysql / postgresql features for those databases)
cargo build --release --features sqlite --bin vaultwarden

# or a container, using the upstream Docker build files under docker/
```

The relay keeps waiting requests in memory, so **run exactly one instance** per deployment.

## 2. Configure it

```ini
DOMAIN=https://rewarden.example.com
ROCKET_ADDRESS=127.0.0.1
ROCKET_PORT=8000
DATA_FOLDER=/var/lib/rewarden
SIGNUPS_ALLOWED=true            # until the first accounts exist, then false (or use invitations)
WEB_VAULT_ENABLED=true          # only needed to create accounts; Bitwarden clients also work

REWARDEN_ENABLED=true
REWARDEN_FCM_SERVICE_ACCOUNT=/etc/rewarden/fcm-service-account.json
REWARDEN_APNS_KEY_FILE=/etc/rewarden/AuthKey_ABC123DEFG.p8
REWARDEN_APNS_KEY_ID=ABC123DEFG
REWARDEN_APNS_TEAM_ID=DEF123GHIJ
# REWARDEN_APNS_TOPIC=com.reins2fa.app   # the iOS app's bundle id
# REWARDEN_RELAY_WAIT_SECS=45   # ChatGPT aborts tool calls after 60 s; Claude allows 240 s
# REWARDEN_OFFLINE_SECS=10
```

`DOMAIN` must be the public `https://` origin: it becomes the OAuth issuer and the MCP resource URL.

## 3. Reverse proxy

Terminate TLS in front of Rocket (Caddy, nginx, Cloudflare Tunnel). Requirements:

* Forward `X-Forwarded-For` / `X-Real-IP` and set `IP_HEADER` accordingly (rate limits key on the client IP).
* **Read timeout of at least 75 s** on `/mcp`, `/rewarden/desktop/calls` and `/rewarden/api/pending` (long polls: 45 s
  relay wait, 25 s phone poll).
* Pass everything under `/.well-known/oauth-authorization-server` and `/.well-known/oauth-protected-resource`
  (the path-inserted variants too, e.g. `/.well-known/oauth-protected-resource/mcp`).
* Do not cache, buffer or rewrite `/mcp`, `/rewarden/*` or `/.well-known/*`.

Example (Caddy):

```caddyfile
rewarden.example.com {
    reverse_proxy 127.0.0.1:8000 {
        transport http { read_timeout 90s }
    }
}
```

### Large files (`/rewarden/blob/`)

Files too large for a tool call (uploads for GitHub or the vault, large downloads, big MCP results) pass through the
server for one operation. They are kept in `$DATA_FOLDER/rewarden-blobs/` (mode 0700, emptied at every start, each file
deleted when its operation is done or after at most an hour), at most 20 files / 2 GiB per user and 8 GiB in total, so
give `DATA_FOLDER` that much free space. Only metadata lives in memory: a restart drops every pending file.

* Allow bodies up to 1 GiB on `/rewarden/blob/` and do not buffer them (stream to Rocket). nginx:

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

  Caddy streams by default; set `request_body { max_size 1GB }` if a body limit is configured.
* The path after `/rewarden/blob/` is a capability (whoever has the URL may upload or download). Reins never logs
  it; keep the proxy's access log off for this location, or strip the path.
* The server fetches from and sends to URLs the phone names (GitHub uploads, release assets, MCP servers): outgoing
  https to the internet must work. Private, loopback, link-local and metadata addresses are always refused.
  `REWARDEN_TEST_ALLOW_LOOPBACK` exists only for the test suite (it permits `http://127.0.0.1`); never set it.

## 4. DNS

Point `rewarden.example.com` at the host (A/AAAA, or a Cloudflare Tunnel CNAME). If the zone is proxied by Cloudflare,
check that its proxy timeout (100 s) is above the long-poll times and that WebSockets/HTTP2 stay enabled.

## 5. Firebase (push)

Create a Firebase project (or use your Google Cloud project) and add an Android app with package `com.reins2fa.app`.

1. `REWARDEN_FCM_SERVICE_ACCOUNT` = the key of a dedicated service account (role *Firebase Cloud Messaging Admin*
   only). Keep it out of the repo; mode `0600`.
2. Build the Android app with `google-services.json` in place (`android/README.md`). Without it the app still works but
   only receives requests while it is open.

### Apple (push to the iOS app)

1. In the Apple Developer account of the team that signs the iOS app: *Keys* → create a key with *Apple Push
   Notifications service (APNs)*. Download the `.p8` (only once possible) to `REWARDEN_APNS_KEY_FILE`, mode `0600`;
   set `REWARDEN_APNS_KEY_ID` to its key id and `REWARDEN_APNS_TEAM_ID` to the team id.
2. All three are set together or not at all; the server does not start with a partial set or an unreadable key.
3. Phones that registered an `apns:` or `apns-sandbox:` token are woken through APNs (production or sandbox), all
   others through FCM. The payload is a fixed alert plus the request id, nothing else.

## 6. Google Cloud (Gmail on the phone)

The phone asks Google Play services for Gmail tokens. That needs, in the same project:

1. **Google Auth Platform → Branding**: app name, support and developer emails.
2. **Audience**: External. *Testing* mode limits sign-in to listed test users and expires grants after 7 days; publish to
   *In production* for anyone else.
3. **Data access**: add `gmail.readonly` and `gmail.send`. Both are *restricted* scopes: more than 100 users requires
   Google's verification and a third-party security assessment.
4. **Clients → Create client → Android**: package `com.reins2fa.app`, SHA-1 of the signing certificate. Register
   the SHA-1 of every key you sign with, debug and release
   (`keytool -list -v -keystore release.jks`).
5. Re-download `google-services.json` (`firebase apps:sdkconfig android <app-id> --project <project-id>`).

Until step 4 is done the app shows "Gmail needs setup" and nothing can touch Gmail.

Google Calendar and Google Contacts use the same Android client. Additionally enable the **Google Calendar API** and the
**People API** in the project and add the scopes `calendar.events`, `calendar.readonly` and `contacts.readonly` under
*Data access* (sensitive scopes; in *Testing* mode listed test users can use them without verification).

### Other integrations (no server setup)

* **Telegram** (the user's own account through MTProto): needs an `api_id` / `api_hash` from <https://my.telegram.org>. Put
  them in `~/.gradle/gradle.properties` as `rewarden.telegramApiId` and `rewarden.telegramApiHash`; they are compiled into
  the app and identify the app, not the user. The login (phone number, code, optional 2-step password) happens on the phone
  and the session is stored sealed there; the server never sees it. Telegram may limit accounts driven by automation.
* **GitHub**: the user creates a token on GitHub (the app opens the page prefilled: fine-grained for chosen repositories, or
  classic for everything including gists and notifications) and the app picks it up from the clipboard; it is kept sealed
  on the phone. About 200 tools: repositories and branches, files and commits (including pushing to a branch), tags and
  releases with assets, issues and pull requests (reviews, merging), workflow runs and logs, variables and secrets,
  settings, collaborators, webhooks, security alerts, gists and notifications. Standing permissions can name the
  repository (or all of an owner's), one branch, the kinds of change (issues, pull requests, code, releases, workflow
  runs, settings, account) and a time; destructive or far-reaching changes (deleting or transferring a repository,
  changing visibility, collaborators, webhooks, deploy keys, secrets, branch protection) are asked for every time.
* **Calendar, contacts and text messages on the phone**: Android permissions (`READ/WRITE_CALENDAR`, `READ_CONTACTS`,
  `READ_SMS`, `SEND_SMS`) requested when the user connects each one. Messages that look like login codes are never
  shared unless ticked one by one.
* **Password vault** (items of every kind, folders, trash, archive, attachments, Sends and the generator): the vault of the Vaultwarden account the phone is signed in to. The master password is entered once
  to unlock the vault key, which is then kept sealed on the phone (the password is not kept). Every secret (a password, a
  one-time code, a note, a card number, an SSH private key) is asked for at each request and can never be covered by a standing grant; passwords are not written to
  the activity log.

## 7. Connect an AI

* **Claude** (claude.ai → Settings → Connectors → Add custom connector): URL `https://rewarden.example.com/mcp`.
* **ChatGPT** (Settings → Connectors → developer mode): the same URL.

The AI gets four Gmail-related tools: `gmail_search`, `gmail_read`, `gmail_send` and `rewarden_get_result`, plus
`rewarden_request_access`, with which it can ask for a narrow, time-limited permission in advance (named senders or
recipients, the shortest duration it needs). The phone shows that as a highlighted permission request; the user can
allow it, allow it for less time, or refuse. If the AI's wait for an approval runs out (about 45 seconds), the user can
still approve later: the answer is kept for `rewarden_get_result`, and an approved read leaves a one-time pass so that
simply asking again works.

The user can connect several Google accounts (Integrations → Gmail → Add account). Which accounts exist is private:
`rewarden_list_accounts` without arguments lists only the connected integrations (`gmail`, no addresses, no approval).
With `service: "gmail"` it asks the user, on the phone, to let the AI see that integration's addresses; the user may
allow it once or for a period (a month by default, which leaves a grant for this AI and this integration). Every
listing is logged with the exact addresses shown. The AI then passes one address as `account` to `gmail_search`,
`gmail_read`, `gmail_send` and `rewarden_request_access`. With a single account the argument is optional; with several,
leaving it out is an error, and errors never reveal addresses. Grants belong to one account.

In the activity, the emails an AI was given are listed by sender and subject; the phone keeps only their ids, and opening
one fetches it from Gmail again (so nothing of the mail is stored on the phone or the server).

Each connection opens a browser page with a two-digit code; the user confirms the matching code on the phone (with
biometrics) and names the connection. Nothing else is stored on the server about the AI beyond that label.

### Desktop app (git proxy)

The `rewarden` desktop app lets git on a computer clone and push through GitHub with the phone approving each read and
push; it needs nothing extra on the server. `rewarden login https://rewarden.example.com` connects it like an AI (same
OAuth flow, same browser code), adding its public key as `rewarden_client_key`. The browser page and the phone show the
key's fingerprint ("Desktop app key: 4821 9930"), the terminal shows the same, and on approval the phone pins the key to
the new connection. A malformed key is refused before any pairing starts.

The app then calls `POST /rewarden/desktop/calls` with its access token, body
`{"tool": "github_git_fetch" | "github_git_push" | "github_git_tag_push", "arguments": {…}, "account": null}` (at most
512 KiB). The answer is `{"request_id", "status": "answered", "outcome": …}`, or `"pending"` / `"offline"` without an
outcome, in which case it asks again with `GET /rewarden/desktop/calls/<request_id>` (404 once expired or for another
connection). These tools are never offered to an AI over `/mcp`, and AI tools are refused here (400 `unknown_tool`).
The phone's answer is a GitHub credential sealed to the app's key: the server relays it in memory and cannot open it.
Arguments (a push lists commits and files) are never logged.

## 8. Smoke test after deploying

1. `curl -s https://rewarden.example.com/.well-known/oauth-protected-resource/mcp` returns JSON whose `resource` is
   `https://rewarden.example.com/mcp`.
2. `curl -si -X POST https://rewarden.example.com/mcp -d '{}'` answers `401` with a `WWW-Authenticate` header.
3. Sign in on the phone, tap **Use this phone for approvals**, connect an AI, ask it to search mail: an approval
   notification appears and the result only reaches the AI after approval.
4. Kill the phone's network: the AI gets "open the Reins app and retry" within `REWARDEN_OFFLINE_SECS`.

## Operating notes

* Back up `DATA_FOLDER` (accounts, tokens). Losing it signs everyone out; grants and audit logs live on the phones.
* Rotating the FCM key: create the new key, update the file, restart; delete the old key in IAM.
* Rotating the APNs key: create a new key, update the file and `REWARDEN_APNS_KEY_ID`, restart; then revoke the old
  key in the Apple Developer account.
* Revoking a phone: sign in from another phone and tap **Use this phone for approvals** (it needs the recovery code or
  master password, or the old phone's approval); the old phone is told it was replaced and stops receiving requests.
* Logs never contain tokens, message contents or Gmail data; `LOG_LEVEL=info` is safe.

### Which tools the AI sees

The phone tells the server which integrations have an account (ids only), and `tools/list` shows only their tools plus
`rewarden_*`. Until the phone has reported (after a server restart, until its next sync) every tool is listed. An AI client
may need to reconnect the connector to refresh its tool list after a new integration is added.
