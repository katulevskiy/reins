# Claude Connectors Directory: submission package

What to paste into Anthropic's developer portal to list Reins in the Claude Connectors Directory, and what Reins still
lacks. Nothing here has been submitted. The submission needs an Anthropic account on a paid Claude plan that belongs
to the publishing organization.

Sources, checked on 2026-10-16 (the custom connector steps on 2026-10-29):

- [Anthropic MCP Directory FAQ](https://support.claude.com/en/articles/11596036-anthropic-mcp-directory-faq). It now
  points to the pages below.
- [Submit a connector to the directory](https://claude.com/docs/connectors/building/submission)
- [Connector pre-submission checklist](https://claude.com/docs/connectors/building/review-criteria)
- [Authentication for connectors](https://claude.com/docs/connectors/building/authentication)
- [Get started with custom connectors using remote MCP](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)
  and [Use connectors to extend Claude's capabilities](https://support.claude.com/en/articles/11176164-use-connectors-to-extend-claude-s-capabilities)
- [Anthropic Software Directory Policy](https://support.claude.com/en/articles/13145358-anthropic-software-directory-policy)
  and [Terms](https://support.claude.com/en/articles/13145338-anthropic-software-directory-terms)

Portal: [claude.ai/directory/manage](https://claude.ai/directory/manage), **Submit new**, **MCP connector**.

## Connection

| Field | Value |
| --- | --- |
| Server URL | `https://app.reins2fa.com/mcp` (universal URL: every user connects to the same one) |
| Transport | Streamable HTTP. Requests are POSTs; `GET /mcp` answers 405 unless it asks for the ticket update stream with a token. |
| Self-hosted servers | Not part of this listing. Self-hosters add their own `https://<host>/mcp` as a custom connector. |

## Listing

| Field | Limit | Value |
| --- | --- | --- |
| Server name | 100 | Reins |
| One-liner | 200 | Let Claude use your Gmail, GitHub, calendar and more, with every action approved on your phone. |
| Categories | 1 to 5 | Productivity; Communication; Developer tools; Security |
| Documentation URL | | `https://reins2fa.com/developers` (public). The quick start is also public: <https://github.com/katulevskiy/reins/blob/main/docs/quick-start.md> |
| Privacy policy URL | | `https://reins2fa.com/privacy` |
| Support URL | | `https://reins2fa.com/support`. **Pending**: answers 404 on 2026-10-29. A draft is on the local branch `docs/support-page` (`docs/support.md`), not published. |
| Support contact | | `support@reins2fa.com`, with a reply within 2 business days |
| Icon | | [`docs/assets/logo.svg`](../assets/logo.svg) (vector). PNG fallbacks: [`docs/assets/logo.png`](../assets/logo.png) (256 x 256) and `android/play/graphics/icon.png` (512 x 512). |
| Slug | permanent | `reins` |

Description (at most 2,000 characters):

> Reins connects Claude to the accounts on your phone (Gmail, Google Calendar and Contacts, GitHub, Telegram, text
> messages and your password vault) and to MCP servers you add in the Reins app. Claude asks, and your phone decides.
>
> Every action Claude takes, such as sending an email, opening a pull request or adding a calendar event, appears on
> your phone with exactly what will happen: the recipients and the full text, or the diff. Nothing happens until you
> approve it. For repeated work you can give a narrow, time-limited permission, for example "read emails from my bank
> for an hour". You can revoke it at any time.
>
> Your service sign-ins stay on your phone. Claude only gets the results you approve, and the Reins server only
> relays them. A request waits in memory for your answer for two minutes by default. If you don't answer in time, it
> waits as a pending request you can still review, for four hours by default and seven days at most. Its content is
> sealed to your phone's key, so the server can't read it.
>
> To use it, install the Reins app (a TestFlight beta on iPhone and iPad; on Android an APK, or Google Play closed testing for invited testers), connect the services
> you want, then add this connector and sign in with your Reins account.

Use cases (portal step **Use cases**):

- Primary use cases: read, search, draft and send email; manage calendar events; work on GitHub repositories, issues,
  pull requests and Actions; read and send Telegram and SMS messages; look up items in the password vault; call tools
  of other MCP servers through one approval point.
- What users need first: a Reins account and the Reins app with at least one service connected. There is no App
  Store build yet (iOS is a TestFlight beta), and Android is an APK or Google Play closed testing only (no public
  Google Play listing).
- Reads and writes: both. Every write and most reads are approved on the phone.

## Company

| Field | Value |
| --- | --- |
| Company name | Question 1 below |
| Website | `https://reins2fa.com` |
| Primary contact | Question 2 below |

## Authentication

Reins serves both registration styles the portal offers. Choose **OAuth with a client ID metadata document**
(`oauth_cimd`): Anthropic recommends it for directory servers, and the custom connector dialog calls it "Use Claude's
published identity (Recommended)". Dynamic client registration (`oauth_dcr`, "Register automatically" in the dialog)
works too (question 3). What Reins serves today (live at
`https://app.reins2fa.com/.well-known/oauth-authorization-server`, checked on 2026-10-29):

- a 401 with `WWW-Authenticate: Bearer resource_metadata=".../.well-known/oauth-protected-resource/mcp"`;
- protected resource metadata whose `resource` is exactly `https://app.reins2fa.com/mcp`;
- `registration_endpoint`, `client_id_metadata_document_supported: true`, `token_endpoint_auth_methods_supported`
  with `none` and `private_key_jwt`, PKCE `S256`, and rotated refresh tokens with `invalid_grant` errors;
- redirect URIs: any the client registers or declares, including `https://claude.ai/api/mcp/auth_callback` and
  Claude Code's loopback.

The consent step is Reins's own: the browser page asks for the account email and shows a two-digit code, and the
phone confirms the matching code and names the connection.

## Data handling

- API ownership: Reins's own server and API. Calls to Google, GitHub and Telegram are made by the user's phone with
  the user's own tokens, never by the Reins server.
- Personal health data: no. Sponsored content: no.

Where the retention text in the description comes from:

| Claim | Source |
| --- | --- |
| A request waits in memory for two minutes by default (60 to 600 s) | `REINS_INSTANT_WINDOW_SECS`, default 120, in [`src/config.rs`](../../src/config.rs); `INSTANT_DEFAULT_SECS` and `INSTANT_MAX_SECS` in [`crates/reins-proto/src/ticket.rs`](../../crates/reins-proto/src/ticket.rs) |
| Then it becomes a pending request: a row in `reins_tickets`, metadata in clear, body sealed to the approval device's key, and the server forgets the clear body | Module comment of [`src/api/reins/tickets.rs`](../../src/api/reins/tickets.rs) |
| A pending request waits four hours by default, seven days at most | `REINS_DEFERRED_WINDOW_SECS`, default 14,400, in `src/config.rs`; `DEFERRED_DEFAULT_SECS = 4 * 3600` and `DEFERRED_MAX_SECS = 7 * 86_400` in `crates/reins-proto/src/ticket.rs` |
| Answered and expired requests are kept seven more days for the history, then deleted | `CLOSED_KEEP_SECS` in `src/api/reins/tickets.rs` |
| A result waits in memory up to five minutes for the AI to collect it | `TRANSIT_SECS` in `src/api/reins/tickets.rs` |

The old text ("in memory for at most ten minutes") was wrong. The ten minutes are `ITEM_TTL` in
`src/api/reins/mod.rs`, the time an offline phone has to pick a request up. Check that the privacy policy matches
the table above before submitting.

## Add and remove Reins as a custom connector

Steps quoted from the Claude Help Center on 2026-10-29. Remote connectors are set up in the Claude account, so the
steps are the same on the web and in Claude Desktop, and the connection is made from Anthropic's cloud (article
[11175166](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp),
"Why this applies to Cowork and Claude Desktop").

Add, on a Free, Pro or Max plan (Free allows one custom connector):

1. **Customize > Connectors**.
2. Article 11175166 says: click "**+ Add**" then "**Add custom connector**". Article
   [11176164](https://support.claude.com/en/articles/11176164-use-connectors-to-extend-claude-s-capabilities) says:
   click "**Add**", hover over "**Custom**", then select "**Web**". The two pages differ; which one the current UI shows
   is **unverified** (not checked in a live account).
3. Name: `Reins`. Remote MCP server URL: `https://app.reins2fa.com/mcp`. **Continue**.
4. Review the detected authentication settings. **Continue**.
5. **Authentication**: "Sign in now". **OAuth client**: "Use Claude's published identity (Recommended)". No
   **Request headers**.
6. **Add**, then sign in on the Reins page and confirm the two-digit code on the phone.

On Team and Enterprise plans an Owner adds it under **Organization settings > Connectors** with the same dialog, and
each member then opens **Customize > Connectors**, finds the connector (labelled "Custom") and clicks **Connect**.

A prefilled install link opens the same dialog (from
[Directory connectors vs custom connectors](https://claude.com/docs/connectors/building/directory-vs-custom)):
`https://claude.ai/customize/connectors?modal=add-custom-connector&connectorName=Reins&connectorUrl=https%3A%2F%2Fapp.reins2fa.com%2Fmcp`.

Use in a chat: the **+** button at the lower left (or type `/`), hover over **Connectors**, and turn Reins on for
the conversation. Per-tool permissions can be set to **Always allow**, **Needs approval** or **Blocked**. With
**Needs approval**, each call is approved twice: once in Claude, once on the phone.

Remove: **Customize > Connectors** (Owners: **Organization settings > Connectors**), then "**Remove**" or the three
dots next to the connector, and follow the prompts. To change the URL, remove the connector and add it again.
Removing it in Claude does not end the Reins connection. Remove that in the Reins app as well (the Connections tab).

**Unverified** (help articles only, not tried in a live account): every menu label above, and the exact labels in
Claude Desktop (the articles don't show the Desktop UI separately) and on mobile,
where adding a connector is in beta.

## Test and launch

The portal needs credentials for a fully populated account. Reins approvals happen on a phone, so a reviewer needs
one of these options:

- **Option A (main option): a directory reviewer account plus a test phone approver.** The pattern the store reviews
  already use: [android/PLAY_STORE.md, "App access"](../../android/PLAY_STORE.md#app-access) documents dedicated WorkOS
  users with email and password sign-in and no MFA, one per store, used only by that store's reviewers, with the
  password kept in the team's password manager. For Claude: a separate WorkOS user `<CLAUDE_REVIEWER_EMAIL>` in
  `<TEST_ORG>`, with demo Gmail, GitHub and Calendar accounts connected. The phone signed in to it,
  `<APPROVER_DEVICE>`, stays online in Autopilot **Bypass** and approves every call. It also has to confirm the
  two-digit code a new connection shows, which Bypass does not do: either automatically (a connection auto-confirm
  limited to that account, off by default, time-boxed and logged; new code) or by `<APPROVER_CONTACT>` in a set
  window. Whether to create the account is Daniel's call. The password goes only into the portal, never into this
  repository.
- **Option B:** a review session where the team approves on the phone while the reviewer tests.

Confirm before submitting that every listed tool was run once, either through
[MCP Inspector](https://modelcontextprotocol.io/docs/tools/inspector) or as a custom connector in Claude.

### Reviewer walkthrough

Written as if the test organization and approval phone exist. The placeholders are listed in question 4.

Before the review, the Reins team:

1. Signs `<CLAUDE_REVIEWER_EMAIL>` in to the Reins account of `<TEST_ORG>` on `<APPROVER_DEVICE>`, a phone that stays on
   charge and online, with notifications allowed.
2. Connects `<DEMO_GMAIL>` (Gmail, Google Calendar), `<DEMO_GITHUB_USER>` (GitHub, with the sandbox repository
   `<DEMO_REPO>`) on `<APPROVER_DEVICE>`, and fills them with the sample data in question 4.
3. Turns on Autopilot **Bypass** on `<APPROVER_DEVICE>` for the review window `<REVIEW_WINDOW>` (Option A), or puts
   `<APPROVER_CONTACT>` on call (Option B).
4. Checks which tool profile the server runs (`REINS_MCP_TOOL_PROFILE=claude` if Daniel chooses it, gap 1).

Lockdown: `<APPROVER_DEVICE>` must not be in Lockdown during the review: Lockdown denies every request, including
what already waits, and a global Lockdown overrides every connection's own mode. Where it is turned off, from the repository (the exact in-app labels are **unverified**):

- In the app, the **Trust** tab or the mode pill ([docs/autopilot.md](../autopilot.md), lines 6 and 18).
- Android: while Lockdown is on, the Quick Settings tile opens Trust to end it
  (`android/app/src/main/java/dev/reins/android/platform/tiles/ReinsTiles.kt`, line 112).
- iPhone: the Control Center Lockdown toggle, which needs an unlocked phone (`ios/ReinsWidgets/Controls.swift`, line 8).
- Leaving Lockdown from the desktop needs an approval on the phone ([docs/security-model.md](../security-model.md),
  lines 536 to 541), and Lockdown on one phone wins across the account's phones
  ([docs/ACCOUNT_STORAGE.md](../ACCOUNT_STORAGE.md), line 20), so check every phone signed in to the test account.

The reviewer:

1. Adds the connector as in "Add and remove Reins as a custom connector" (`https://app.reins2fa.com/mcp`), with
   **Use Claude's published identity**.
2. On the sign-in page enters `<CLAUDE_REVIEWER_EMAIL>` and the password from the portal. Reins then shows a
   two-digit code, which `<APPROVER_DEVICE>` confirms (Option A), or the reviewer sends it to `<APPROVER_CONTACT>`
   (Option B).
3. In a new chat, turns Reins on under **+ > Connectors**, and sets the Reins tools to **Always allow** so that the
   phone is the only approval (optional).
4. Runs the test cases below in order. With Option A each approval answers within a few seconds. With Option B,
   `<APPROVER_CONTACT>` approves each card and reads what it showed back to the reviewer.
5. Denies one request (Option B only, test 8) to see Claude report the denial.
6. Removes the connector (see above) at the end. `<TEST_ORG>` then removes the connection in the Reins app.

### Test cases

"Phone" is what `<APPROVER_DEVICE>` shows. With Bypass on, the card is approved at once and listed in the Activity
tab.

| # | Prompt | Expected tool | Expected phone approval |
| --- | --- | --- | --- |
| 1 | "Which services do I have connected in Reins?" | `reins_list_accounts` (without `service`) | None: this call needs no approval. |
| 2 | "Find the latest email from `<DEMO_SENDER>` and summarize it." | `gmail_search`, then `gmail_read` | A card for the search (the query) and one for reading the message. |
| 3 | "Send an email to `<DEMO_RECIPIENT>` saying I'll be 10 minutes late." | `gmail_send` | A card with the recipients, subject and the full text. Nothing is sent before approval. |
| 4 | "What's on my calendar tomorrow? Add a 30-minute focus block at 9am." | `calendar_list_events`, then `calendar_create_event` | One card for the read, one with the event's title, time and calendar. |
| 5 | "Open an issue in `<DEMO_REPO>` titled 'Fix login timeout', with the steps from this chat." | `github_create_issue` | A card with the repository, title and body. |
| 6 | "Which of my pull requests in `<DEMO_REPO>` have failing checks?" | `github_pr_list`, then `github_checks_get` | Cards for the reads (or one, if a time-limited permission covers them). |
| 7 | "For the next hour, let yourself read emails from `<DEMO_SENDER>` without asking." | `reins_request_access` | A card for the permission: sender, read only, one hour. Later reads from that sender need no card. |
| 8 | "Send `<DEMO_RECIPIENT>` the text 'test'." (Option B: deny it) | `gmail_send` | A card. After **Deny**, Claude says the user denied it on their Reins device. |
| 9 | "Buy me a phone case" or "Get my GitHub password from the vault" | none | No card: with the `claude` profile, no payments or vault tool is listed, so Claude says it can't. |

## Example prompts

At least three are required. Use tests 2 to 6 above, with the placeholders replaced by the demo data.

## Compliance acknowledgments

The portal asks for seven: directory guidelines, first-party API use, financial transactions, AI media generation,
prompt injection, conversation data collection, and public documentation. Check the financial transactions item
first (see the gaps below).

## Requirement checklist

| Requirement | Status |
| --- | --- |
| Remote server over HTTPS, Streamable HTTP | Done |
| OAuth 2.0 (DCR or CIMD), 401 with `resource_metadata`, PKCE S256, `claude.ai` callback | Done |
| Every tool has a `title` | Done; a test checks it |
| Every tool has `readOnlyHint` / `destructiveHint` (Reins sets all four hints) | Done; a test checks it |
| Tool names at most 64 characters | Done; a test checks it |
| Read and write split into separate tools (no catch-all with a `method`) | Done: `github_api_read` / `github_api_write` are separate, and the write tool takes only unsafe methods. The duplicate `github_request_*` tools are not listed. |
| Custom query tools name the target API | Done: the GitHub API tools link `https://docs.github.com/rest` |
| Descriptions say what the tool does, no prompt injection | Done. Descriptions start with the verb. The server instructions describe only how to use Reins's own tools. |
| Actionable errors | Done: denied, offline, waiting, not connected, and invalid arguments each say what to do |
| Privacy policy (collection, use, retention, contact) | Done at `https://reins2fa.com/privacy`. Check that the site's copy matches [privacy-policy.md](../legal/privacy-policy.md) and the retention table above. |
| Public documentation | Done (`/developers`, GitHub docs). A help-center style "Use Reins with Claude" page would be better. |
| Support channel | `support@reins2fa.com` (reply within 2 business days). The support page `https://reins2fa.com/support` is **pending** (404). |
| Test account with sample data | **Missing**: see the reviewer walkthrough |
| No money transfers or financial transactions | **Gap**: `payments_purchase_request` buys things with a virtual card. The optional `claude` tool profile (proposed) removes it. |
| Tested as a custom connector in Claude | To do before submitting |

## Gaps and blockers

1. **Payments and secrets.** The policy excludes software that "executes financial transactions for users", and the
   review criteria reject connectors that "transfer money". `payments_purchase_request` makes a purchase with a card,
   and the vault tools, the GitHub secret scanning alert tools and SMS reads can return passwords, one-time codes and
   card details. An optional server profile, `REINS_MCP_TOOL_PROFILE=claude`, is **proposed** on the local branch
   `feat/directory-tool-profile`, not merged. It is the same set as the `chatgpt` profile: no vault, Reins Pay,
   secret scanning, `github_secret_set` or SMS read tools, neither listed nor callable, and the GitHub API tools
   refuse `secret-scanning` paths. The per-group counts are in
   [ChatGPT: optional tool profile](openai-apps.md#optional-chatgpt-tool-profile). The default is unchanged. It is
   server-wide, so the listing needs its own deployment or a per-endpoint profile (question 5). Daniel decides
   whether to drop the tools. Without it, expect a rejection with the payments tools listed, or ask
   `mcp-review@anthropic.com` whether an approved, phone-confirmed purchase is acceptable.
2. **Reviewer access.** Approvals need a phone, and there is no Claude reviewer account yet. Main option (Option A):
   a dedicated WorkOS reviewer user with email and password and no MFA, as in
   [android/PLAY_STORE.md, "App access"](../../android/PLAY_STORE.md#app-access), plus a test phone approver in
   Autopilot Bypass. It needs a dedicated Google Workspace or Gmail account, a GitHub user, a phone or emulator that
   stays online, and a way to confirm the new-connection code (automatic, new code; or the team). Daniel decides
   whether to create the account (question 4).
3. **No store build.** There is no App Store build yet (iOS is a TestFlight beta), and Android is an APK or
   Google Play closed testing only (no public listing; tracks in
   [android/PLAY_STORE.md](../../android/PLAY_STORE.md)). A reviewer
   who wants their own phone must join TestFlight, or sideload the APK or be invited to the closed test. Option A avoids this.
4. **Google verification.** Gmail and Google Calendar use scopes that need Google's app verification before the
   general public can use them (see the README's status section). Reviewers use the team's demo account, but public
   users can't connect Gmail until it passes.
5. **Support page.** `https://reins2fa.com/support` answers 404 (pending). A draft is on the local branch `docs/support-page` (`docs/support.md`), not published.
6. **Tool count.** 285 tools are listed with the full profile, 208 of them for GitHub (the duplicate
   `github_request_*` are no longer listed but stay callable); the proposed `claude` profile lists 245. Claude
   loads tools on demand when there are many, but reviewers weigh "token use proportionate to the task". Merging rarely used GitHub tools would help further.
7. **Icon.** A 512 x 512 PNG and the SVG exist. The portal does not state a size; use the SVG, or the 512 PNG.

## Questions for Daniel

1. **Company name.** The legal name of the publishing organization, as the portal and the terms should show it.
2. **Primary contact.** The name and email that get review updates (`support@reins2fa.com`, or a person?).
3. **OAuth client.** Submit as `oauth_cimd` (recommended) or `oauth_dcr`? Has a Claude connection with
   "Use Claude's published identity" been tested end to end?
4. **Test organization and placeholders.** Values for `<TEST_ORG>`, `<CLAUDE_REVIEWER_EMAIL>`, `<APPROVER_DEVICE>` (phone or
   emulator, who keeps it online), `<APPROVER_CONTACT>` (Option B), `<REVIEW_WINDOW>`, `<DEMO_GMAIL>`,
   `<DEMO_SENDER>`, `<DEMO_RECIPIENT>`, `<DEMO_GITHUB_USER>` and `<DEMO_REPO>`, and the sample data: a few emails from
   `<DEMO_SENDER>`, two calendar events tomorrow, and a pull request with a failing check in `<DEMO_REPO>`.
5. **Tool profile.** Drop the tools for the listing (merge `feat/directory-tool-profile` and serve it from a
   deployment that runs `REINS_MCP_TOOL_PROFILE=claude`, a second URL), make the profile per endpoint first, or keep
   the full set? The `claude` profile now matches `chatgpt`, which also leaves out the vault password generators;
   Anthropic's policy alone would allow a looser set (only Reins Pay out).
6. **Support page.** When will `https://reins2fa.com/support` be live? Submit only after it is.
7. **Bypass for review.** Is Autopilot Bypass on the demo phone acceptable, given that the review then never sees a
   real decision? Or Option B with a scheduled session?

## What you must do yourself

1. Sign in to claude.ai with an account on a paid plan in the organization that publishes Reins. Open
   [claude.ai/directory/manage](https://claude.ai/directory/manage).
2. Decide what to do about payments and secrets (gap 1, question 5).
3. Set up the demo account and phone (gap 2, question 4) and publish the support page (gap 5).
4. Add Reins as a custom connector in Claude and run every test case.
5. Paste the fields above, accept the seven acknowledgments and submit. Escalations: `mcp-review@anthropic.com`.
