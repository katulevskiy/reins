# Licensing

This repository holds several programs under two licenses. Each Rust crate states its license in its `Cargo.toml`
(`license = ...`); this page is the map and the reasons. Third-party code, data and assets keep their own licenses and
are listed in [NOTICE](NOTICE).

| Part | Path | License |
| --- | --- | --- |
| Server (a Vaultwarden fork with the AI permission relay) | root crate `vaultwarden` and `macros/`: `src/`, `macros/`, `migrations/`, `tests/`, `docker/`, `playwright/`, `resources/`, `build.rs` | AGPL-3.0-only |
| Phone core (vault client, connectors, approvals, Autopilot) | `crates/rewarden-core` | Apache-2.0 |
| Android app | `android/` | Apache-2.0 |
| Desktop app: daemon, git/SSH/API proxies, harness hooks | `crates/rewarden-desktop` | Apache-2.0 |
| Desktop app: window and tray (GPUI, Apache-2.0; Geist fonts, OFL-1.1) | `crates/rewarden-desktop-app` | Apache-2.0 |
| Wire protocol types | `crates/rewarden-proto` | Apache-2.0 |
| Permission policy engine | `crates/rewarden-policy` | Apache-2.0 |
| Autopilot model runtime for the desktop and `laya-try` | `crates/rewarden-laya` | Apache-2.0 |
| End-to-end test harness | `crates/rewarden-e2e` | Apache-2.0 |
| Autopilot model tooling (data, fine-tuning, export, evaluation) | `tools/laya/` | Apache-2.0 |
| Everything else (scripts, docs, configuration) | | Apache-2.0 |

License texts: [LICENSE](LICENSE) (Apache License 2.0, the repository default) and [LICENSE-AGPL](LICENSE-AGPL)
(GNU Affero General Public License v3.0, for the server). "AGPL-3.0-only" means version 3 of the AGPL and no later
version, as in upstream Vaultwarden.

## Why the split

* **The server is AGPL because Vaultwarden is.** It is a fork of [Vaultwarden](https://github.com/dani-garcia/vaultwarden)
  (AGPL-3.0-only); the fork and everything compiled into it stays under that license. If you run a modified server for
  others, the AGPL asks you to offer them its source.
* **Everything else is Apache-2.0.** The phone app and its core, the desktop daemon, the protocol, the policy engine and
  the model tooling are new code. Apache-2.0 lets AI harnesses, agent frameworks, other clients and alternative servers
  build on them without copyleft obligations, and gives an explicit patent license.

The direction of use matters: the AGPL server may use Apache-2.0 code (it uses `rewarden-proto` and
`rewarden-policy`), but no Apache-2.0 crate may depend on the server's code. `rewarden-e2e` starts the server as a
separate program and talks to it over HTTP; it does not link it. `deny.toml` checks the licenses of third-party crates
(`cargo deny --workspace check licenses bans`).

### Model weights

The Autopilot model files are not in this repository; the app downloads them on demand. The published package is a
fine-tune of Laya (Apache-2.0) on mmBERT-base (MIT); see [NOTICE](NOTICE).

## Contributions

Contributions are accepted under the [Contributor License Agreement](CLA.md) (CLA), which every contributor signs once,
by comment on their first pull request (see [CONTRIBUTING.md](CONTRIBUTING.md)). The CLA gives the project owner the
right to license contributions under the license of the part they touch **and** under other terms, including
relicensing (for example offering commercial licenses). The
Developer Certificate of Origin (DCO) is not used; no `Signed-off-by` line is needed.

Code that comes from upstream Vaultwarden is not covered by the CLA: it stays under AGPL-3.0-only, copyright its
authors. Changes to it that are meant for Vaultwarden itself are best sent upstream.
