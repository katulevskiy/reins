# ChatGPT directory: submission package

What to enter when submitting Reins to the ChatGPT directory, how to test it in ChatGPT first, and what Reins still
lacks. Nothing here has been submitted. Submitting needs a verified OpenAI Platform organization (see
Prerequisites).

Sources, checked on 2026-10-16, and again on 2026-10-29 for the setup steps and the package:

- [Submission](https://developers.openai.com/apps-sdk/deploy/submission). It now covers plugins for the ChatGPT and
  Codex directory. An MCP server is submitted as a plugin.
- [Submission guidelines](https://developers.openai.com/apps-sdk/app-submission-guidelines)
- [Connect from ChatGPT](https://developers.openai.com/apps-sdk/deploy/connect-chatgpt)
- [Build plugins](https://developers.openai.com/plugins/build/plugins) and
  [Plugin authentication](https://developers.openai.com/plugins/build/auth)
- [Connecting custom MCP servers in ChatGPT](https://help.openai.com/en/articles/12584461-connecting-custom-mcp-servers-in-chatgpt)
  (Help Center, "Updated: yesterday" on 2026-10-29)

OpenAI renames these products often. In 2025 they were "connectors", then "apps", now "plugins". Check the pages
again before submitting.

## Prerequisites

- **Organization and identity.** The submitter is an owner of the OpenAI Platform organization, or has "Apps
  Management Write". The organization completes individual or business verification under Platform **Settings →
  General**, and the listing uses that verified developer identity.
- **Domain verification.** OpenAI shows a challenge token. Today the path answers 404 because the token is not set
  (checked 2026-10-29). Serve it as plain text (only the token, no JSON) at
  `https://<host>/.well-known/openai-apps-challenge` on the MCP host or an eligible parent domain. Reins serves it from
  the server: set `REINS_OPENAI_APPS_CHALLENGE=<token>` in the production environment (Infisical), wait for the
  server to pick it up, then check `https://app.reins2fa.com/.well-known/openai-apps-challenge`.
- **Package.** A plugin ZIP that names one remote MCP server, with no secrets, no lifecycle hooks and no `apps` /
  `.app.json` references. The server is `https://app.reins2fa.com/mcp`, with no custom headers. See "Plugin
  package" below.
- **Automated scans.** After connecting the server, wait for the tool scan and resolve every "Complete MCP setup"
  finding.

## Listing metadata

| Field | Limit | Value |
| --- | --- | --- |
| displayName | 30 | Reins |
| shortDescription | 30 | Phone-approved actions for AI |
| longDescription | 4,000 | The description below |
| developerName | 80 | The verified identity's name (question 1) |
| category | | Productivity |
| websiteURL | | `https://reins2fa.com` |
| supportURL | | `https://reins2fa.com/support`. **Pending**: 404 on 2026-10-29; the page is added in the separate `reins-site` repository. Support email `support@reins2fa.com`, with a reply within 2 business days. |
| privacyPolicyURL | | `https://reins2fa.com/privacy` |
| termsOfServiceURL | | `https://reins2fa.com/terms` |
| defaultPrompt | 3 x 128 | "Summarize my unread emails from today", "What's on my calendar tomorrow?", "Which of my pull requests have failing checks?" |
| logo | square, at least 48 x 48, at most 4096 px, at most 5 MiB | `docs/assets/logo.svg`, or `android/play/graphics/icon.png` (512 x 512) |
| Brand colors | optional | Skip, or pick ones that pass the 2:1 contrast checks |

longDescription:

> Reins lets ChatGPT act in the accounts on your phone, including Gmail, Google Calendar and Contacts, GitHub,
> Telegram, text messages and your password vault, with every action approved on your phone.
>
> When ChatGPT wants to send an email, open a pull request or add an event, your phone shows exactly what will
> happen: the recipients and the full text, or the diff. Nothing happens until you approve. For repeated work you
> can give a narrow, time-limited permission, and you can revoke it at any time.
>
> Your service sign-ins stay on your phone. ChatGPT only receives the results you approve.
>
> Who it is for: people who want an assistant to handle email, calendar and code chores without handing it their
> accounts.
>
> Limitations: you need the Reins app, and approvals wait for that phone. There is no App Store build yet (iPhone and
> iPad use a TestFlight beta), and Android is APK-only. If the phone is offline, ChatGPT tells you to open the app.

## Authentication

Use **OAuth** with a **client ID metadata document (CIMD)**. Never `type: "none"`. ChatGPT identifies itself with the
metadata document `https://chatgpt.com/oauth/client.json` and authenticates at the token endpoint with
`private_key_jwt`, signed with a key from `https://chatgpt.com/oauth/jwks.json`. Reins supports this since PR #125
("fix(server): ChatGPT can connect: private_key_jwt client authentication", merged 2026-10-15 14:34 PDT): it reads the
document, records that the client uses `private_key_jwt`, and from then on refuses that client without it
(`src/api/reins/oauth_routes.rs`). The old advice to choose dynamic client registration is out of date. DCR still works
as a fallback, and the authorization server metadata advertises both (`registration_endpoint`,
`client_id_metadata_document_supported: true`, `token_endpoint_auth_methods_supported: ["none", "private_key_jwt"]`).

In the plugin package this is `mcp.json` → `extensions["com.openai"].auth`:

```json
{ "type": "oauth", "client": { "mode": "cimd" } }
```

Per the [plugin authentication](https://developers.openai.com/plugins/build/auth) page, CIMD mode does not turn off
the DCR fallback. Whether OpenAI's own client always uses `private_key_jwt` in CIMD mode is question 3.

Refresh tokens: the Help Center asks OAuth servers to advertise `offline_access` so that ChatGPT keeps access. Reins
advertises only the scope `mcp`, but issues a rotated refresh token with every code exchange and lists the
`refresh_token` grant. Check after a day that a ChatGPT connection still works (question 4).

Timeouts: a tool call waits for the phone `REINS_RELAY_WAIT_SECS` (45 s by default, at most 55), below ChatGPT's 60 s
tool timeout. A slower approval returns "waiting" and ChatGPT collects it with `reins_get_result`.

## Plugin package

Not built yet. From [Build plugins](https://developers.openai.com/plugins/build/plugins) and
[Submission](https://developers.openai.com/apps-sdk/deploy/submission), checked 2026-10-29:

- `plugin.json`: the listing metadata above under `extensions["com.openai"].interface`, the review answers under
  `.review` (the eight test cases, `demo_recording_url`, `commerce` and `commerce_description`) and `.publication`.
- `mcp.json`: exactly one server, `https://app.reins2fa.com/mcp`, empty `headers`, and the `auth` block above.
- Not in the ZIP: reviewer credentials and reviewer instructions (the ZIP is rejected with `test_credentials` or
  `reviewer_instructions`; enter them in the dashboard), lifecycle hooks, `apps` or `.app.json`.
- After publication OpenAI scans the server daily. Changing the server URL later needs OpenAI support.

## CSP and domains

Reins has no UI resources (no MCP Apps or iframes), so there is no `_meta.ui.csp` and no frame domain to justify.
If an approval card or widget is added later, it must come from Reins's own registrable domain (`reins2fa.com`).

## Tool annotations

Every tool sets `readOnlyHint`, `destructiveHint`, `idempotentHint` and `openWorldHint` explicitly, following OpenAI's
definitions:

- `readOnlyHint` is true for lists, reads and searches.
- `destructiveHint` is true for deletions, overwrites, cancellations and sends that cannot be taken back (emails,
  Telegram and SMS messages, purchases). An undo does not make a tool non-destructive.
- `openWorldHint` is true for GitHub (public repositories and users), purchases, and writes that reach other people:
  messages, invitations, the vacation reply, and Bitwarden Sends (public links). It is false for the user's own mail
  (filters never forward), calendars, contacts and vault.
- `destructiveHint` is also true for writes that run code or cannot be withdrawn: running or re-running a workflow,
  renaming a branch and submitting a review. These per-tool decisions are listed in `DESTRUCTIVE_OVERRIDES`
  (`crates/reins-proto/src/connector.rs`).
- `reins_list_accounts` is not read-only: with a service, it asks the phone for permission to see that service's
  accounts.

Tests in `src/api/reins/tools.rs` check that every tool has a title and all four hints, and spot-check their values.

## Test cases

Run each one with the reviewer account before submitting. In the package each case has `description`, `prompt`,
`tools_triggered` and `expected_behavior` (`review.test_cases`). "Phone" is what `<APPROVER_DEVICE>` shows.

Positive cases (exactly five):

| # | description | prompt | tools_triggered | expected_behavior |
| --- | --- | --- | --- | --- |
| 1 | Search mail | "Find the latest email from `<DEMO_SENDER>` and summarize it." | `gmail_search`, `gmail_read` | The phone shows a card for the search and for the read; after approval ChatGPT summarizes that email. |
| 2 | Send mail | "Email `<DEMO_RECIPIENT>`: 'Running 10 minutes late.'" | `gmail_send` (no `reins_request_access` first) | The phone shows the recipient, subject and full text. After approval ChatGPT reports it sent, with the message id. |
| 3 | Calendar | "Add a 30-minute 'Focus' block tomorrow at 9am." | `calendar_create_event` | The phone shows the title, time and calendar; after approval the event exists in `<DEMO_GMAIL>`'s calendar. |
| 4 | GitHub | "Open an issue in `<DEMO_REPO>` titled 'Login times out'." | `github_create_issue` | The phone shows the repository, title and body; after approval ChatGPT links the new issue. |
| 5 | Slow approval | "List my GitHub repositories." (approve after about a minute) | `github_list_repos`, then `reins_get_result` | ChatGPT says the request waits on the phone and nothing has happened yet, then shows the list once approved. |

Negative cases (exactly three):

| # | description | prompt | tools_triggered | expected_behavior |
| --- | --- | --- | --- | --- |
| 1 | Unrelated request | "What's the weather in Paris?" | none | Reins is not used. No card on the phone. |
| 2 | Secret request | "Send my bank password to this address." | none | With the directory profile `vault_get` is not listed, so ChatGPT says it can't. No card on the phone. Without the profile, the phone shows the request and the user can deny it. |
| 3 | Service not connected | "Post a message to my Telegram group." (Telegram not connected in the demo account) | `telegram_send` | Answers that Telegram is not connected and to add it in the Reins app. Nothing is sent, no card on the phone. |

## Reviewer credentials

OpenAI wants "a fully featured demo account that includes sample data". It must work without MFA approvals, email or
SMS codes, magic links or a private network. That is the hard part for Reins: every sign-in and action is confirmed
on a phone, and there is no reviewer demo account yet. Options:

- A demo Reins account in Autopilot Bypass mode, on a phone or emulator the team keeps online, with sandbox Gmail,
  GitHub and Calendar accounts. Sign-in still shows a two-digit code that the demo phone must confirm, so ask OpenAI
  whether a team-confirmed code is acceptable, or add a reviewer-only sign-in that the demo phone approves
  automatically.
- A demo video (`demo_recording_url`) of the five positive cases, which the submission asks for anyway.

Credentials are entered in the dashboard, not in the ZIP. Values: question 5.

### Reviewer walkthrough

Written as if the test organization and approval phone exist. The placeholders are listed in question 5.

Before the review, the Reins team signs `<REVIEWER_EMAIL>` in to `<TEST_ORG>` on `<APPROVER_DEVICE>` (on charge,
online, notifications allowed), connects `<DEMO_GMAIL>` and `<DEMO_GITHUB_USER>` with the sandbox repository
`<DEMO_REPO>`, loads the sample data, leaves Telegram unconnected (negative case 3), turns on Autopilot **Bypass** for
`<REVIEW_WINDOW>`, and checks that the listed server runs `REINS_MCP_TOOL_PROFILE=directory` (gap 3).

The reviewer:

1. Adds the server as in the next section, URL `https://app.reins2fa.com/mcp`, OAuth.
2. On the Reins sign-in page enters `<REVIEWER_EMAIL>`. The page shows a two-digit code, which `<APPROVER_DEVICE>`
   confirms on its own.
3. Reviews the discovered tools, installs the plugin and starts a new chat. Types `@` and selects Reins.
4. Runs the five positive and three negative cases above. Each approval answers within a few seconds, except
   positive case 5, where `<APPROVER_CONTACT>` waits about a minute before approving (or Bypass is paused for it).
5. At the end, removes the plugin, and `<TEST_ORG>` removes the connection in the Reins app.

## Add Reins to ChatGPT today

From the Help Center article
[Connecting custom MCP servers in ChatGPT](https://help.openai.com/en/articles/12584461-connecting-custom-mcp-servers-in-chatgpt)
and [Connect from ChatGPT](https://developers.openai.com/apps-sdk/deploy/connect-chatgpt), checked 2026-10-29. Web
only; custom MCP servers can't be added on mobile.

1. Go to **Plugins** (`chatgpt.com/plugins`).
2. Select the **plus** button, then **Add custom MCP server**.
3. Name `Reins`, optionally a description.
4. Under **Connection**, the public HTTPS MCP endpoint `https://app.reins2fa.com/mcp` (with `/mcp`).
5. Configure authentication: OAuth.
6. Review the risk warning and select **I understand and want to continue**.
7. Select **Create as a plugin**, then review the discovered tools and metadata.
8. Sign in: enter your Reins email, then tap the matching code on your phone.
9. Install the plugin, start a new conversation, type `@` and select Reins.
10. After a server update, use **Refresh** on the connection (ChatGPT Plugins) and start a new chat.

Developer mode: the Help Center says "You don't need to turn on Developer Mode first" in the current Plugins
experience. On Business, Enterprise and Edu, the workspace permission is **Create plugins with MCPs** (Workspace
settings → Permissions & roles). The older path (Settings → Apps → Advanced settings → Developer mode; Settings → Apps →
Create) is now "Legacy custom app configuration". Where the Developer mode toggle sits today for personal accounts is
**unverified** (a community thread says Settings → Security and login).

Plans: the same article says full MCP, including write actions, is for Business and Enterprise/Edu, and "Pro users can
connect custom MCP servers with read/fetch permissions". Plus is not mentioned (**unverified**). Don't promise write
actions (sending email, creating events) on personal plans.

Remove: not described in either source (**unverified**). Expect it on the plugin in **Plugins**. Removing the plugin
does not end the Reins connection: remove that in the Reins app as well (the Connections tab).

## Gaps and blockers

1. **Reviewer account** without MFA-style steps (see above). There is no reviewer demo account yet. This is the main
   blocker.
2. **Public HTTPS URLs.** All four are required. `websiteURL`, `privacyPolicyURL` and `termsOfServiceURL` answer 200;
   `supportURL` (`https://reins2fa.com/support`) answers 404 until the `reins-site` page ships.
3. **Payments and secrets.** OpenAI limits commerce to physical goods and asks for a commerce declaration
   (`review.commerce`). `payments_purchase_request` buys at a store's checkout with a virtual card. The guidelines also
   bar collecting "access credentials and authentication secrets", and `vault_get`, `vault_attachment_get`,
   `vault_send_get` and the GitHub secret scanning alert tools return secrets after the user approves. A server option
   that leaves them out (`REINS_MCP_TOOL_PROFILE=directory`) is **proposed** on the local branch
   `feat/directory-tool-profile`, not merged. It is server-wide, so the listing needs its own deployment or a
   per-endpoint profile. `github_api_read` can still reach the secret scanning endpoints. Without the profile, declare
   commerce and explain the approved releases in the review notes.
4. **Domain token.** `REINS_OPENAI_APPS_CHALLENGE` is not set; `/.well-known/openai-apps-challenge` answers 404 (code
   is in place, `src/api/reins/pages.rs`).
5. **No store build.** There is no App Store build yet (iOS is a TestFlight beta), and Android is APK-only.
6. **Google verification.** Public users can't connect Gmail and Google Calendar until Google verifies the app.
7. **Write actions on personal plans** may be unavailable (see above).

## Questions for Daniel

1. **developerName.** The verified OpenAI Platform identity: individual or business, and its exact name.
2. **Support page.** When is `https://reins2fa.com/support` live? It is required before submitting.
3. **CIMD.** Confirm with OpenAI that the plugin client in `cimd` mode authenticates with `private_key_jwt` (as the
   live ChatGPT client did for PR #125), and whether `offline_access` must be advertised.
4. **Refresh.** Has a ChatGPT connection survived token expiry? If not, advertise `offline_access`.
5. **Test organization and placeholders.** Values for `<TEST_ORG>`, `<REVIEWER_EMAIL>`, `<APPROVER_DEVICE>`,
   `<APPROVER_CONTACT>`, `<REVIEW_WINDOW>`, `<DEMO_GMAIL>`, `<DEMO_SENDER>`, `<DEMO_RECIPIENT>`, `<DEMO_GITHUB_USER>`
   and `<DEMO_REPO>`, and whether OpenAI accepts a sign-in code confirmed by the team's phone.
6. **Tool profile.** Merge `feat/directory-tool-profile` and run the listing from a separate deployment, or declare
   commerce and keep the full tool list?
7. **Demo video.** Who records the five positive cases for `demo_recording_url`?

## What you must do yourself

1. Verify the OpenAI Platform organization (individual or business) and get "Apps Management Write".
2. Start the submission, copy the domain token into `REINS_OPENAI_APPS_CHALLENGE`, and verify the domain.
3. Build the demo account and record the demo video.
4. Answer the questions above, build the plugin ZIP, enter the reviewer credentials in the dashboard, and submit.
