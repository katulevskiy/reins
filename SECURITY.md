# Security policy

Reins decides what AI agents may do with a person's accounts, and its phone app holds the secrets that make that
possible. We take reports seriously and are grateful to everyone who helps keep users safe.

## Scope

In scope: the current release and the default branch of everything in this repository.

* **Server and relay** (`src/`, in particular `src/api/rewarden/`): the MCP endpoint and OAuth for AI connections,
  the relay between AIs, phones and desktops, pairing, blob storage, push notifications, and the vault server.
* **Phone app** (`android/`) and its **core** (`crates/rewarden-core`): sealed storage of connector tokens and vault
  keys, approval and standing-permission logic, the connectors (Gmail, GitHub and other git hosts, Telegram, calendar,
  contacts, SMS, the vault, MCP servers), and the in-app updater.
* **Desktop app and daemon** (`crates/rewarden-desktop`): login and key pinning, the git, SSH-agent and API proxies,
  `rewarden ask`/`run`, harness hooks, the local MCP server, release signing and `rewarden update`, `scripts/install.sh`.
* **Protocol and policy** (`crates/rewarden-proto`, `crates/rewarden-policy`): anything that lets a request do more
  than the user approved.
* **Autopilot** (`crates/rewarden-core/src/autopilot`, `crates/rewarden-laya`, `tools/laya`): ways to make the
  model approve what it should not (prompt injection through AI-written text, crafted targets or content), to bypass
  the hard floor, the modes or the rate limits, to tamper with the downloaded model (its pinned hashes), or to poison
  what it learns from the user's decisions.

Examples of what we most want to hear about: an AI or a desktop acting without the phone's approval or beyond it;
secrets or message contents leaving the phone, reaching the server or showing up in logs; one user or connection
reaching another's data; bypassing key pinning or update signatures.

## How to report

Please report privately, not in a public issue or pull request:

* **GitHub private vulnerability reporting**: the repository's *Security* tab → *Report a vulnerability*.
* **Email**: `security@<domain>` <!-- TODO: set the security contact address (and optionally a PGP key) once the
  project's domain is final. -->

Include what is affected (component, version or commit), how to reproduce it, the impact you see, and any proof of
concept. If you are not sure whether something is a security issue, report it privately anyway.

## What to expect

* We acknowledge your report within 3 business days and keep you updated as we investigate.
* We aim to confirm or rule out the issue within 10 business days and to ship a fix for confirmed high-severity issues
  as quickly as we can, usually within 90 days.
* We coordinate the disclosure date with you, publish a GitHub security advisory for confirmed issues and, unless you
  prefer otherwise, credit you in it.
* Please give us a reasonable time to fix the issue before disclosing it publicly.

## Safe harbor

We consider security research carried out in good faith under this policy to be authorized. We will not pursue or
support legal action against you for it, as long as you:

* use only accounts, devices, servers and data that are yours or that you have explicit permission to test (run your
  own server: it is open source, and a local instance is the best place to test);
* avoid privacy violations, data destruction and service disruption, and stop and tell us if you come across other
  people's data;
* do not exploit an issue beyond what is needed to demonstrate it, and keep it confidential until it is fixed.

If in doubt about whether something is allowed, ask us first through the channels above.

## Out of scope

* Issues in the Vaultwarden vault server that are not specific to Reins: report them to Vaultwarden (below).
* Bitwarden's clients and web vault, and other third-party software and services (Google, GitHub, Telegram, the AI
  providers): report to their maintainers. Issues in a dependency are welcome here when they affect Reins.
* Attacks that need an already compromised or rooted phone or computer, or physical access to an unlocked device.
* Social engineering of users, maintainers or contributors; phishing; denial of service and volumetric attacks;
  spamming.
* An AI doing something harmful that the user approved, or that a standing permission the user granted covers.
* Missing best practices without a demonstrated security impact (still welcome as normal issues), outdated releases,
  and reports from automated scanners without a working proof.

## Vaultwarden

The vault server in this repository derives from [Vaultwarden](https://github.com/dani-garcia/vaultwarden). If an issue
affects Vaultwarden's own code (not only Reins's additions), please report it to Vaultwarden as well, following
[their security policy](https://github.com/dani-garcia/vaultwarden/blob/main/SECURITY.md), so that its users are
protected too. We are happy to coordinate with them. Reins is not affiliated with Vaultwarden or Bitwarden, Inc.
