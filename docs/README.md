# Reins documentation

**Using Reins**

- [Quick start](quick-start.md): account, phone app, desktop app, first agent. Also `ask`, `run`, the API proxy,
  SSH, and local mode.
- [Harnesses](harnesses.md): Claude Code, Codex, Gemini CLI, Cursor and cloud AIs. What `reins harness add`
  changes, and the guard rules for hooks.
- [Autopilot](autopilot.md): automatic approvals on the phone. Modes, the hard floor, learning, privacy, and the
  model's limits.

**Understanding it**

- [Security model](security-model.md): what an agent can and cannot do, where secrets live, what the server sees.
- [Architecture](architecture.md): components, endpoints and the main flows.
- [Autopilot model card](../tools/laya/MODEL_CARD.md): `laya-approvals-ml-v1`, its data, evaluation and limits.

**Running it**

- [Self-hosting](self-hosting.md): building and configuring the server, reverse proxy, push, accounts.
- [Deployment notes](deployment.md): the setup of the hosted instance, with the Google Cloud and Firebase steps.

**Legal (drafts)**

- [Privacy policy](legal/privacy-policy.md) and [terms of service](legal/terms.md) for the hosted service.

**Design records**

The specs in [`superpowers/specs/`](superpowers/specs/) and plans in [`superpowers/plans/`](superpowers/plans/)
record how each part was designed. They are working documents and may describe details that have since changed.
