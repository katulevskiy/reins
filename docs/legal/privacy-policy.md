# Privacy policy (hosted service)

> **DRAFT. Not legal advice. Needs review by a qualified lawyer before publication.** Placeholders are in
> `[BRACKETS]`. This policy covers the hosted Reins service at `https://app.reins2fa.com` and its website,
> `https://reins2fa.com`. It does not cover servers that other people run with Reins's open-source code.

Last updated: [DATE]

## Who we are

The hosted Reins service is operated by [OPERATOR NAME, ADDRESS] ("we"). Contact for privacy questions:
[PRIVACY CONTACT EMAIL].

## The short version

- Your phone holds the credentials for your connected services (Gmail, GitHub, Telegram, and others) and makes the
  calls to those services. We never receive or store those credentials, with one narrow exception for large files,
  described below.
- Our server relays requests from your AI tools to your phone and the answers back. Relayed content is held in
  memory, for at most 10 minutes, and is not written to logs.
- Autopilot runs entirely on your phone. Nothing it learns is sent to us.
- We do not sell your data, show ads, or use analytics or tracking in the app or on the server.

## What we store

**Your account.** Reins accounts are Bitwarden-compatible accounts. We store your email address, the name you give
(optional), a hash derived from your master password (we never receive the password itself), your key-derivation
settings, two-factor settings, and your password vault. The vault is encrypted on your devices before it reaches us,
and we cannot read it. We also store the devices you signed in with (device name, type, identifier, and when they
were last used).

**Your approval phone.** Which phone is your approval device, and its Firebase Cloud Messaging token, so that we can
wake it.

**Your AI connections.** For each AI tool or computer you connect: the client's name and host (for example
"Claude", `claude.ai`), the name you gave the connection, when it was created and last used, and a hashed refresh
token. We also store the registration details AI clients send us (client name, redirect addresses).

## What passes through without being stored

**Requests and results.** When an AI tool asks to do something, our server receives the request, including its
arguments (for example the email it wants to send), and holds it in memory until your phone answers, or at most 10
minutes. Your phone's answer (for example the emails you allowed it to read) passes through to the AI tool the same
way. We do not write requests or results to disk or to logs.

**Sealed answers to your computer.** Credentials and answers your phone sends to the Reins desktop app are encrypted
to that computer's key. We relay them but cannot read them.

**What your phone reports.** Which kinds of integrations you have connected (for example "gmail", "github", without
account names), and the names and tool lists of MCP servers you added on your phone. This is kept in memory, so that
AI tools see the right tools. It is replaced when your phone reports again, and lost when the server restarts.

**Large files.** Files too large to pass through a tool call (for example a release asset you upload to GitHub, or a
large download) are stored on our server's disk for that one operation. Each is deleted when the operation is done,
and in any case within one hour. For these operations, and for MCP tools that return very large results, your phone
asks our server to make one request on its behalf. The request includes the authorization header it needs (for
example a GitHub token). We use that header for that request only, and do not store or log it.

## What we never have

- the credentials for your connected services (except as described under "Large files");
- the content of your mailbox, calendar, contacts, messages or repositories, beyond what you approve for an AI tool and
  what passes through as described above;
- your standing permissions, your activity log and your Autopilot data. These live only on your phone, encrypted.

## Autopilot

Autopilot's model runs on your phone. Its memory of your decisions, the profiles and the per-profile adapter are
stored encrypted on your phone and never sent to us. The model files are downloaded from our site, which sees the
download like any other web request (see "Logs").

## Third parties

- **Google Firebase Cloud Messaging** delivers wake-up messages to your phone. A message contains only a request id
  and a type, never request content. Google's privacy policy applies to that delivery.
- **Your connected services** (Google, GitHub, Telegram, GitLab, Codeberg, Bitbucket, MCP servers you add) are
  contacted by your phone directly, under their own terms and privacy policies. The large-file exception above is
  the only time our server contacts them for you.
- **AI providers** (for example Anthropic, OpenAI) receive the results you approve, under their own policies.
- **Hosting:** our server runs at [HOSTING PROVIDER, COUNTRY].

[If applicable: "The Reins Android app's use and transfer of information received from Google APIs adheres to the
Google API Services User Data Policy, including the Limited Use requirements." Review against Google's current
requirements before publishing.]

## Logs

Our server software writes operational logs without tokens, request contents or message contents. Failed sign-in
attempts are logged with the IP address they came from. Our web server keeps access logs (IP address, time, requested
path, user agent) for [N] days. Links for large files are capabilities, and we do not log them. Downloads of the app,
the desktop app and the Autopilot model are logged like any other request.

## How long we keep data

| Data | Kept |
|---|---|
| Account, vault, devices | until you delete your account |
| AI connections | until you remove them on your phone, or delete your account |
| Refresh tokens | 30 days, or until the connection is removed; expired ones are purged hourly |
| Requests and results | in memory, at most 10 minutes |
| Sign-in and pairing sessions | in memory, a few minutes |
| Large files | until the operation is done, at most 1 hour |
| Web server logs | [N] days |
| Backups | [N] days |

Deleting your account (in the Reins app: Settings → Delete account; or in the web vault: Settings → My account →
Delete account) deletes your account, vault, approval-phone registration, AI and computer connections and tokens, and
the encrypted app state stored for it. The app deletes it at once, together with your sign-in at our identity
provider (WorkOS), and removes the account's data from the phone. Copies in backups are deleted within [N] days.

## Your rights

You can see and export your vault with any Bitwarden client, see your AI connections on your phone, and delete your
account at any time. [Depending on where you live (for example the EU/EEA, UK or California), you may have rights to
access, correct, delete, restrict or port your personal data, and to complain to a supervisory authority. Contact us
at [PRIVACY CONTACT EMAIL]. Legal basis for processing: performance of the contract (providing the service);
legitimate interests (security, abuse prevention).]

## Children

The service is not intended for children under [16].

## Security

Approvals require your phone's screen lock or biometrics. Credentials on your phone are encrypted with a key protected
by Android's Keystore. Our server uses TLS, stores refresh tokens hashed, and never stores the credentials for your
connected services. No system is perfectly secure. See the [security model](../security-model.md) for what our server
can and cannot see, and what a compromised server could do.

## Changes

We will post changes here and update the date above. For material changes, we will notify account holders by
[email / in-app notice].

## Contact

[OPERATOR NAME], [ADDRESS], [PRIVACY CONTACT EMAIL]
