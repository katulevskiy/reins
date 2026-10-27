# Reins documentation

**Using Reins**

- [Quick start](quick-start.md): account, phone app, desktop app, first agent. Also `ask`, `run`, the API proxy,
  SSH, and local mode.
- [Linux packages](linux-packages.md): the APT and RPM repositories, for Debian, Ubuntu and Fedora. On Arch Linux,
  use the install script.
- [Harnesses](harnesses.md): Claude Code, Codex, Gemini CLI, Cursor and cloud AIs. What `reins harness add`
  changes, and the guard rules for hooks.
- [Troubleshooting](troubleshooting.md): a request that never arrives, a failing git push, a new or lost phone.
- [Autopilot](autopilot.md): automatic approvals on the phone. Modes, the hard floor, learning, privacy, and the
  model's limits.
- [Payments](payments.md): an agent buys something you approve on the phone. Payment methods, spend limits, budgets,
  the ledger and the signed mandate.

**Understanding it**

- [Security model](security-model.md): what an agent can and cannot do, where secrets live, what the server sees.
- [Reins Secrets: design](secrets.md): projects, environments, scan and import, agents, CI.
- [Architecture](architecture.md): components, endpoints and the main flows.
- [Autopilot model card](../tools/laya/MODEL_CARD.md): `laya-approvals-ml-v1`, its data, evaluation and limits.
- [Autopilot risk: the Overnight mode](autopilot-risk.md): severity in context, the risk head, thresholds and rollout.

**Running it**

- [Self-hosting](self-hosting.md): building and configuring the server, reverse proxy, push, accounts.
- [Deployment notes](deployment.md): the setup of the hosted instance, with the Google Cloud and Firebase steps.
- [CI](reference/ci.md): which suites run, reuse of passes, macOS admission, labels, and the measurements behind them.
- AI client directories: submission packages for [Claude](directories/anthropic-connectors.md) and
  [ChatGPT](directories/openai-apps.md), and how to add Reins to [Perplexity and Grok](directories/perplexity-grok.md).

**Legal (drafts)**

- [Privacy policy](legal/privacy-policy.md) and [terms of service](legal/terms.md) for the hosted service.

**Design records**

The specs in [`superpowers/specs/`](superpowers/specs/) record the architecture and security decisions.
The completed implementation recipes have been removed; current behavior is documented above and covered by tests.
