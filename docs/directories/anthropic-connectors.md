# Claude Connectors Directory: submission package

What to paste into Anthropic's developer portal to list Reins in the Claude Connectors Directory, and what Reins still
lacks. Nothing here has been submitted. The submission needs an Anthropic account on a paid Claude plan that belongs
to the publishing organization.

Sources, checked on 2026-10-16:

- [Anthropic MCP Directory FAQ](https://support.claude.com/en/articles/11596036-anthropic-mcp-directory-faq). It now
  points to the pages below.
- [Submit a connector to the directory](https://claude.com/docs/connectors/building/submission)
- [Connector pre-submission checklist](https://claude.com/docs/connectors/building/review-criteria)
- [Authentication for connectors](https://claude.com/docs/connectors/building/authentication)
- [Anthropic Software Directory Policy](https://support.claude.com/en/articles/13145358-anthropic-software-directory-policy)
  and [Terms](https://support.claude.com/en/articles/13145338-anthropic-software-directory-terms)

Portal: [claude.ai/directory/manage](https://claude.ai/directory/manage), **Submit new**, **MCP connector**.

## Connection

| Field | Value |
| --- | --- |
| Server URL | `https://app.reins2fa.com/mcp` (universal URL: every user connects to the same one) |
| Transport | Streamable HTTP (POST only; `GET /mcp` answers 405) |
| Self-hosted servers | Not part of this listing. Self-hosters add their own `https://<host>/mcp` as a custom connector. |

## Listing

| Field | Limit | Value |
| --- | --- | --- |
| Server name | 100 | Reins |
| One-liner | 200 | Let Claude use your Gmail, GitHub, calendar and more, with every action approved on your phone. |
| Categories | 1 to 5 | Productivity; Communication; Developer tools; Security |
| Documentation URL | | `https://reins2fa.com/developers` (public). The quick start is also public: <https://github.com/katulevskiy/reins/blob/main/docs/quick-start.md> |
| Privacy policy URL | | `https://reins2fa.com/privacy` |
| Support contact | | `support@reins2fa.com` |
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
> Your passwords and tokens never leave your phone. Claude only gets the results you approve, and the Reins server
> only relays them: it keeps requests in memory for at most ten minutes and logs no content.
>
> To use it, install the Reins app (iOS or Android), connect the services you want, then add this connector and sign
> in with your Reins account.

Use cases (portal step **Use cases**):

- Primary use cases: read, search, draft and send email; manage calendar events; work on GitHub repositories, issues,
  pull requests and Actions; read and send Telegram and SMS messages; look up items in the password vault; call tools
  of other MCP servers through one approval point.
- What users need first: a Reins account and the Reins app on a phone (iOS or Android) with at least one service
  connected.
- Reads and writes: both. Every write and most reads are approved on the phone.

## Company

| Field | Value |
| --- | --- |
| Company name | TODO: the legal name the publishing organization uses |
| Website | `https://reins2fa.com` |
| Primary contact | TODO: name and email for review updates |

## Authentication

Choose **OAuth with dynamic client registration** (`oauth_dcr`). Reins also advertises a client ID metadata document
(`oauth_cimd`), which Anthropic recommends for high-traffic directory servers because it avoids one registration per
connection. What Reins serves today (live at `https://app.reins2fa.com/.well-known/oauth-authorization-server`):

- a 401 with `WWW-Authenticate: Bearer resource_metadata=".../.well-known/oauth-protected-resource/mcp"`;
- protected resource metadata whose `resource` is exactly `https://app.reins2fa.com/mcp`;
- `registration_endpoint`, `client_id_metadata_document_supported: true`, `token_endpoint_auth_methods_supported`
  with `none`, PKCE `S256`, and rotated refresh tokens with `invalid_grant` errors;
- redirect URIs: any the client registers or declares, including `https://claude.ai/api/mcp/auth_callback` and
  Claude Code's loopback.

The consent step is Reins's own: the browser page asks for the account email and shows a two-digit code, and the
phone confirms the matching code and names the connection.

## Data handling

- API ownership: Reins's own server and API. Calls to Google, GitHub and Telegram are made by the user's phone with
  the user's own tokens, never by the Reins server.
- Personal health data: no. Sponsored content: no.

## Test and launch

The portal needs credentials for a fully populated account. Reins approvals happen on a phone, so a reviewer needs
one of these options:

- **Option A (preferred, needs work):** a demo account whose approvals are answered automatically. An account in
  Autopilot Bypass mode on a phone the team keeps online, with demo Gmail, GitHub and Calendar accounts connected,
  approves every call. The reviewer then needs only the Reins email and the sign-in code flow.
- **Option B:** a review session where the team approves on the phone while the reviewer tests.

Instructions for the reviewer (fill in the placeholders):

1. Add the connector: `https://app.reins2fa.com/mcp`, then **Connect**.
2. On the sign-in page enter `TODO-demo@reins2fa.com`. The page shows a two-digit code. The demo phone confirms it
   (Option A), or tell `TODO contact` the code (Option B).
3. Try the example prompts below. Each action is approved on the demo phone within a few seconds.

Test account: email `TODO`, sign-in steps above, sample data: TODO (demo Gmail inbox, GitHub user with a sandbox
repository, a calendar).

Confirm before submitting that every tool was run once, either through
[MCP Inspector](https://modelcontextprotocol.io/docs/tools/inspector) or as a custom connector in Claude.

## Example prompts

At least three are required.

1. "Find the latest email from my bank and summarize it."
2. "Send an email to alex@example.com saying I'll be 10 minutes late." (The phone shows the recipients and text.)
3. "What's on my calendar tomorrow? Add a 30-minute focus block at 9am."
4. "Open an issue in my repo octo/sandbox titled 'Fix login timeout', with the steps from this chat."
5. "Which of my pull requests have failing checks?"

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
| Read and write split into separate tools (no catch-all with a `method`) | Done: `github_api_read` / `github_api_write` and `github_request_read` / `github_request_write` are separate. The write tools take a method, but only unsafe ones. |
| Custom query tools name the target API | Done: the GitHub API tools link `https://docs.github.com/rest` |
| Descriptions say what the tool does, no prompt injection | Done. Descriptions start with the verb. The server instructions describe only how to use Reins's own tools. |
| Actionable errors | Done: denied, offline, waiting, not connected, and invalid arguments each say what to do |
| Privacy policy (collection, use, retention, contact) | Done at `https://reins2fa.com/privacy`. Check that the site's copy matches [privacy-policy.md](../legal/privacy-policy.md). |
| Public documentation | Done (`/developers`, GitHub docs). A help-center style "Use Reins with Claude" page would be better. |
| Support channel | `support@reins2fa.com` |
| Test account with sample data | **Missing**: see Test and launch |
| No money transfers or financial transactions | **Gap**: `payments_purchase_request` buys things with a virtual card |
| Tested as a custom connector in Claude | To do before submitting |

## Gaps

1. **Payments.** The policy excludes software that "executes financial transactions for users", and the review
   criteria reject connectors that "transfer money". `payments_purchase_request` makes a purchase with a card. Either
   hide the payments tools from the directory listing (a server option that leaves `payments_*` off `tools/list`),
   or ask `mcp-review@anthropic.com` whether an approved, phone-confirmed purchase of physical goods is acceptable.
   Until then, expect a rejection with the payments tools listed.
2. **Reviewer access.** Approvals need a phone. A demo account kept in Autopilot Bypass with sandbox accounts
   (Option A) is the practical way. It needs a dedicated Google Workspace or Gmail account, a GitHub user and a
   phone or emulator that stays online.
3. **Tool count.** 281 tools are listed (210 for GitHub). Claude loads tools on demand when there are many, so this
   works, but reviewers weigh "token use proportionate to the task". Merging rarely used GitHub tools, or leaving
   `github_request_*` (which overlaps `github_api_*`) off the list, would help.
4. **DCR volume.** Each new Claude connection registers a client. Reins supports CIMD, which Claude prefers when both
   values are advertised, as they are. No action is needed, but check after launch.
5. **Icon.** A 512 x 512 PNG and the SVG exist. The portal does not state a size; use the SVG, or the 512 PNG.

## What you must do yourself

1. Sign in to claude.ai with an account on a paid plan in the organization that publishes Reins. Open
   [claude.ai/directory/manage](https://claude.ai/directory/manage).
2. Decide what to do about payments (gap 1).
3. Set up the demo account and phone (gap 2) and fill in the TODOs above.
4. Add Reins as a custom connector in Claude and run every tool once.
5. Paste the fields above, accept the seven acknowledgments and submit. Escalations: `mcp-review@anthropic.com`.
