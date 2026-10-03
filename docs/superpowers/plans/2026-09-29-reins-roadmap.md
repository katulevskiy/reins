# Reins MVP — Plan Roadmap

Spec: `docs/superpowers/specs/2026-09-28-reins-mvp-design.md`

The spec spans four subsystems. Each gets its own plan, written when its predecessor has
landed so it can cite real interfaces. Every plan ends in working, tested software.

| # | Plan | Delivers | Depends on |
|---|------|----------|------------|
| 1 | `2026-09-29-reins-1-proto-policy.md` | `reins-proto` wire types + `reins-policy` grant engine, property-tested | — |
| 2 | server (Vaultwarden fork) | DB tables, OAuth AS + phone pairing, dual-era MCP endpoint, relay, phone API, FCM sender | 1 |
| 3 | `reins-core` + `reins-e2e` | Vaultwarden client, encrypted local store, Gmail connector, request handler, UniFFI surface; headless end-to-end test against the real server | 1, 2 |
| 4 | Android app | Kotlin shell + Compose screens, Keystore/Google/FCM/biometric integrations, cargo-ndk build, emulator run | 3 |
| 5 | deployment | Google Cloud + Firebase wiring, the public deployment, connect Claude.ai and ChatGPT for real | 2, 4 |

Status is tracked by the checkboxes inside each plan.
