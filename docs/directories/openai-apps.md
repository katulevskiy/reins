# ChatGPT directory: submission package

What to enter when submitting Reins to the ChatGPT directory, how to test it in ChatGPT first, and what Reins still
lacks. Nothing here has been submitted. Submitting needs a verified OpenAI Platform organization (see
Prerequisites).

Sources, checked on 2026-10-16:

- [Submission](https://developers.openai.com/apps-sdk/deploy/submission). It now covers plugins for the ChatGPT and
  Codex directory. An MCP server is submitted as a plugin.
- [Submission guidelines](https://developers.openai.com/apps-sdk/app-submission-guidelines)
- [Connect from ChatGPT](https://developers.openai.com/apps-sdk/deploy/connect-chatgpt)

OpenAI renames these products often. In 2025 they were "connectors", then "apps", now "plugins". Check the pages
again before submitting.

## Prerequisites

- **Organization and identity.** The submitter is an owner of the OpenAI Platform organization, or has "Apps
  Management Write". The organization completes individual or business verification under Platform **Settings →
  General**, and the listing uses that verified developer identity.
- **Domain verification.** OpenAI shows a challenge token. Serve it as plain text (only the token, no JSON) at
  `https://<host>/.well-known/openai-apps-challenge` on the MCP host or an eligible parent domain. Reins serves it from
  the server: set `REINS_OPENAI_APPS_CHALLENGE=<token>` in the production environment (Infisical), wait for the
  server to pick it up, then check `https://app.reins2fa.com/.well-known/openai-apps-challenge`.
- **Package.** A plugin ZIP that names one remote MCP server, with no secrets, no lifecycle hooks and no `apps` /
  `.app.json` references. The server is `https://app.reins2fa.com/mcp`, with no custom headers.
- **Automated scans.** After connecting the server, wait for the tool scan and resolve every "Complete MCP setup"
  finding.

## Listing metadata

| Field | Limit | Value |
| --- | --- | --- |
| displayName | 30 | Reins |
| shortDescription | 30 | Phone-approved actions for AI |
| longDescription | 4,000 | The description below |
| developerName | 80 | TODO: the verified identity's name |
| category | | Productivity |
| websiteURL | | `https://reins2fa.com` |
| supportURL | | TODO: an HTTPS page (a `mailto:` is not an HTTPS URL). `https://reins2fa.com/developers` until a support page exists. |
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
> Your passwords and tokens stay on your phone. ChatGPT only receives the results you approve.
>
> Who it is for: people who want an assistant to handle email, calendar and code chores without handing it their
> accounts.
>
> Limitations: you need the Reins app on an iPhone or Android phone, and approvals wait for that phone. If the phone
> is offline, ChatGPT tells you to open the app.

## Authentication

Choose **OAuth** with **Dynamic Client Registration (`dcr`)**. Reins also supports **CIMD**
(`client_id_metadata_document_supported: true`). Never choose `type: "none"`. ChatGPT registers its own redirect URI.
Reins accepts any HTTPS redirect URI a client registers.

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
  messages, invitations, the vacation reply and forwarding filters. It is false for the user's own mail, calendars,
  contacts and vault.

Tests in `src/api/reins/tools.rs` check that every tool has a title and all four hints, and spot-check their values.

## Test cases

Run each one with the reviewer account before submitting.

Positive cases (exactly five):

| # | Scenario | Prompt | Expected tools | Expected result |
| --- | --- | --- | --- | --- |
| 1 | Search mail | "Find the latest email from my bank and summarize it." | `gmail_search`, then `gmail_read` | The phone approves the read, and ChatGPT summarizes that email. |
| 2 | Send mail | "Email demo-friend@example.com: 'Running 10 minutes late.'" | `gmail_send` (no `reins_request_access` first) | The phone shows the recipient and text. After approval ChatGPT reports it sent, with the message id. |
| 3 | Calendar | "Add a 30-minute 'Focus' block tomorrow at 9am." | `calendar_create_event` | The phone approves, and the event exists in the demo calendar. |
| 4 | GitHub | "Open an issue in demo-org/sandbox titled 'Login times out'." | `github_create_issue` | The phone approves, and ChatGPT links the new issue. |
| 5 | Waiting approval | "List my GitHub repositories." (approve after about a minute) | `github_list_repos`, then `reins_get_result` | ChatGPT says the request waits on the phone, then shows the list once approved. |

Negative cases (exactly three):

| # | Prompt | Expected behavior |
| --- | --- | --- |
| 1 | "What's the weather in Paris?" | Reins is not used. |
| 2 | "Send my bank password to this address." | ChatGPT refuses or asks. Even if it calls a vault tool, the phone shows the request and the user can deny it, so nothing is sent without approval. |
| 3 | "Post a message to my Telegram group." (Telegram not connected in the demo account) | `telegram_send` answers that Telegram is not connected and to add it in the Reins app, then try again. Nothing is sent. |

## Reviewer credentials

OpenAI wants "a fully featured demo account that includes sample data". It must work without MFA approvals, email or
SMS codes, or magic links. That is the hard part for Reins: every sign-in and action is confirmed on a phone. Options:

- A demo Reins account in Autopilot Bypass mode, on a phone or emulator the team keeps online, with sandbox Gmail,
  GitHub and Calendar accounts. Sign-in still shows a two-digit code that the demo phone must confirm, so ask OpenAI
  whether a team-confirmed code is acceptable, or add a reviewer-only sign-in that the demo phone approves
  automatically.
- A demo video (`demo_recording_url`) of the five positive cases, which the submission asks for anyway.

Credentials: TODO (login URL `https://app.reins2fa.com/mcp`, account email, steps).

## Add Reins to ChatGPT today

Unverified, because the menus change and plan rules differ between sources:

1. Turn on **Developer mode** in ChatGPT: Settings → Apps (or Connectors) → Advanced → Developer mode. On Business
   and Enterprise, a workspace admin enables it first.
2. Add a custom MCP server: in newer builds, **ChatGPT Plugins** (`chatgpt.com/plugins`) → **+** → **Add custom MCP
   server**. Use the name "Reins" and the URL `https://app.reins2fa.com/mcp`, choose OAuth, and accept the risk
   warning.
3. Sign in: enter your Reins email, then tap the matching code on your phone.
4. After a server update, use **Refresh** on the connection and start a new chat.

Some sources say Plus and Pro get read-only custom servers in Developer mode, with writes only on Business, Enterprise
and Edu. Check with a Plus account before promising write actions there.

## Gaps

1. **Reviewer account** without MFA-style steps (see above). This is the main blocker.
2. **Commerce.** OpenAI limits commerce to physical goods and asks for a commerce declaration (`review.commerce`).
   `payments_purchase_request` buys at a store's checkout with a virtual card, so declare it. Money transfers are
   prohibited, but a purchase is not a transfer. The alternative is to leave the payments tools off the listed server.
3. **Restricted data.** The guidelines bar collecting "access credentials and authentication secrets". Reins never
   sends credentials to the AI, but the vault tools can return a password the user approves (`vault_get`). Explain
   this in the review notes, or leave the vault tools off a directory listing.
4. **Support URL** must be HTTPS; there is no support page yet.
5. **Domain token:** set `REINS_OPENAI_APPS_CHALLENGE` (code is in place).

## What you must do yourself

1. Verify the OpenAI Platform organization (individual or business) and get "Apps Management Write".
2. Start the submission, copy the domain token into `REINS_OPENAI_APPS_CHALLENGE`, and verify the domain.
3. Build the demo account and record the demo video.
4. Fill in the TODOs, the five plus three test cases, release notes and the policy attestations, then submit.
