# Security policy

Reins decides what AI agents may do with a person's accounts, and its phone app holds the secrets that make that
possible. We take reports seriously and are grateful to everyone who helps keep users safe.

## Scope

In scope: the current release and the default branch of everything in this repository.

* **Server and relay** (`src/`, in particular `src/api/reins/`): the MCP endpoint and OAuth for AI connections,
  the relay between AIs, phones and desktops, pairing, blob storage, push notifications, and the vault server.
* **Phone apps** (`android/`, `ios/`) and their **core** (`crates/reins-core`): the Android app; the iOS app
  (`ios/Reins`), its widgets and Live Activities (`ios/ReinsWidgets`), its notification service extension
  (`ios/ReinsNotifications`) and the code they share (`ios/Shared`); sealed storage of connector tokens and vault
  keys, approval and standing-permission logic, the connectors (Gmail, GitHub and other git hosts, Telegram, calendar,
  contacts, SMS, the vault, MCP servers), and the in-app updater.
* **Desktop app and daemon** (`crates/reins-desktop`, and `crates/reins-desktop-app` for the pairing window and tray):
  login and key pinning, the git, SSH-agent and API proxies, `reins ask`/`run`, harness hooks, the local MCP server,
  release signing and `reins update`, `scripts/install.sh`, `scripts/install.ps1`.
* **Relay and cryptography**: the relay or server seeing plaintext or secrets it should only carry sealed; replaying,
  reordering or injecting messages; bypassing pairing or key pinning; nonce or key reuse; weak or predictable randomness.
* **Protocol and policy** (`crates/reins-proto`, `crates/reins-policy`): anything that lets a request do more
  than the user approved.
* **Autopilot** (`crates/reins-core/src/autopilot`, `crates/reins-laya`, `tools/laya`): ways to make the
  model approve what it should not (prompt injection through AI-written text, crafted targets or content), to bypass
  the hard floor, the modes or the rate limits, to tamper with the downloaded model (its pinned hashes), or to poison
  what it learns from the user's decisions. In particular: injection that gets an approval past the hard floor,
  tampering with the model's pinned hashes, and poisoning what it learns.
* **Organizations / Managed mode** (`src/api/reins/org/`, behind `REINS_ORGS_ENABLED`): SSO and SCIM provisioning,
  roles, routing, multi-approver quorum, and policy as code. Most wanted: one org reaching another org's members or
  decisions; approving without the required quorum or without the required number of distinct people; and
  self-approval when the policy excludes the requester.
* **Audit log** (`crates/reins-audit`, [docs/audit.md](docs/audit.md)): breaking or forging the append-only hash
  chain, or silently dropping entries.
* **Executor** (`crates/reins-executor`, [docs/executor.md](docs/executor.md)): running an action different from the
  one approved, or one whose approval has expired or is replayed; using an executor's credentials outside its org.
* **Replay tool** (`crates/reins-replay`): in scope only for code execution from a crafted recording.

Org mode is off by default. To test it, run your own server with `REINS_ORGS_ENABLED=true` (see
[docs/self-hosting.md](docs/self-hosting.md)); never test against the hosted service.

Examples of what we most want to hear about: an AI or a desktop acting without the phone's approval or beyond it;
secrets or message contents leaving the phone, reaching the server or showing up in logs; one user or connection
reaching another's data; bypassing key pinning or update signatures.

## How to report

Please report privately, not in a public issue, pull request or chat channel:

* **GitHub private vulnerability reporting** (preferred): the repository's *Security* tab → *Report a vulnerability*,
  or directly at <https://github.com/katulevskiy/reins/security/advisories/new>. Your report stays private between you
  and the maintainers, and we can work on the fix and the advisory with you there.
  <!-- MAINTAINER: this only works once private vulnerability reporting is enabled at
  Settings → Code security → "Private vulnerability reporting" → Enable. As of 2026-10-29 it is disabled. -->
* **Email**: [security@reins2fa.com](mailto:security@reins2fa.com), if you cannot use GitHub or would rather not.
  <!-- PROPOSED: security@reins2fa.com is pending Daniel's confirmation that the mailbox exists and is monitored.
  Optionally add a PGP key here (fingerprint + URL) once one exists. -->

The same contact details are published at <https://reins2fa.com/.well-known/security.txt>
([RFC 9116](https://www.rfc-editor.org/rfc/rfc9116)).

Include what is affected (component, version or commit), how to reproduce it, the impact you see, and any proof of
concept. If you are not sure whether something is a security issue, report it privately anyway.

## What to expect

* We acknowledge your report within 3 business days and keep you updated as we investigate.
* We aim to confirm or rule out the issue within 10 business days.
* We ship a fix for confirmed issues within **14 days for critical, 30 days for high and 90 days for everything
  else**, counted from confirmation. If we need longer, we'll tell you why and when to expect it.
* We coordinate the disclosure date with you, publish a GitHub security advisory for confirmed issues and, unless you
  prefer otherwise, credit you in it. We don't promise to hold disclosure beyond 90 days from your report.
* Please give us a reasonable time to fix the issue before disclosing it publicly.

## Safe harbor

We consider security research carried out in good faith under this policy to be authorized. We will not pursue or
support legal action against you for it, as long as you:

* test only against your own self-hosted server and your own accounts and devices. **Do not test against hosted
  production** (`app.reins2fa.com`, the hosted relay), push notification infrastructure (Apple, Google and our push
  gateway), or anyone else's account. The server is open source, so a local instance is the place to test;
* do not run automated scanners or fuzzers against hosted services;
* do not extract, keep or publish other users' data, even as a proof of concept;
* avoid privacy violations, data destruction and service disruption, and stop and tell us if you come across other
  people's data;
* do not exploit an issue beyond what is needed to demonstrate it, and keep it confidential until it is fixed.

If in doubt about whether something is allowed, ask us first through the channels above.

## Rewards

Reins is a small open-source project without a formal bug bounty program. For confirmed, previously unknown issues
reported under this policy, the maintainer may, at their sole discretion, pay a small thank-you reward:

| Severity (our assessment) | Reward |
| --- | --- |
| Critical: an AI or a desktop acts without or beyond the phone's approval, secrets leave the phone, or one account reaches another's data | TODO(Daniel): amount |
| High: authorization bypass that needs user interaction or one precondition; bypassing an org's quorum or roles; forging the audit chain; reading another user's metadata | TODO(Daniel): amount |
| Medium: limited information leaks; a local privilege issue in the desktop app; denial of service against one user's own pairing | TODO(Daniel): amount |
| Low: hardening with a demonstrated but minor impact | TODO(Daniel): amount, or Hall of Fame credit only |

Rewards depend on impact, report quality and whether the issue was already known; duplicates go to the first report.
Out-of-scope findings (below) are not rewarded. We cannot pay where the law or sanctions forbid it, and you are
responsible for any taxes. Payment method: TODO(Daniel) (for example GitHub Sponsors, PayPal or a donation to a charity
of your choice). Whether to reward, and how much, is the maintainer's decision; this section is not a contract.

## Hall of Fame

We thank everyone who reported a security issue responsibly. Unless you ask us not to, we credit you here and in the
GitHub security advisory.

| Date | Researcher | Issue |
| --- | --- | --- |
| | *Be the first.* | |

<!-- MAINTAINER: add a row when an advisory is published: date (YYYY-MM-DD), name or handle (with link if they want),
and the advisory link, e.g. [GHSA-xxxx-xxxx-xxxx](https://github.com/katulevskiy/reins/security/advisories/GHSA-xxxx-xxxx-xxxx). -->

## Out of scope

* Issues in the Vaultwarden vault server that are not specific to Reins: report them to Vaultwarden (below).
* Bitwarden's clients and web vault, and other third-party software and services (Google, GitHub, Telegram, the AI
  providers): report to their maintainers. Issues in a dependency are welcome here when they affect Reins.
* Attacks that need an already compromised or rooted phone or computer, or physical access to an unlocked device.
* Social engineering of users, maintainers or contributors; phishing; volumetric denial of service; spamming.
  (A logic flaw that lets someone break one user's own pairing is in scope: see Rewards, Medium.)
* An AI doing something harmful that the user approved, or that a standing permission the user granted covers.
* Relay traffic metadata that is already documented (timing and message sizes), unless it reveals content.
* Autopilot: a poor suggestion that a human still approves; jailbreaks with no effect on approvals or security;
  complaints about model accuracy.
* `crates/reins-replay` issues other than code execution from a crafted recording.
* Missing best practices without a demonstrated security impact (still welcome as normal issues), outdated releases,
  and reports from automated scanners without a working proof.

## Vaultwarden

The vault server in this repository derives from [Vaultwarden](https://github.com/dani-garcia/vaultwarden). If an issue
affects Vaultwarden's own code (not only Reins's additions), please report it to Vaultwarden as well, following
[their security policy](https://github.com/dani-garcia/vaultwarden/blob/main/SECURITY.md), so that its users are
protected too. We are happy to coordinate with them. Reins is not affiliated with Vaultwarden or Bitwarden, Inc.
