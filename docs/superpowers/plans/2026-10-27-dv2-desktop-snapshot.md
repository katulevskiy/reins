# Decisions v2: the phone-signed policy snapshot on the desktop (08 R1). Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the desktop daemon decides hook-level shell commands and file tools with the owner's own Auto rules, signed
by the phone (08 R1): the same matrix, rules, facts, mode and tasks the phone decides with, so Auto runs without a
round trip and works with the phone offline. A stale, missing or invalid snapshot falls back to asking the phone.
Local `[auto]` configuration only tightens. Lockdown and other tightenings reach the computer in seconds.

**Architecture:**
- **Pure (`reins-policy::desktop_policy`).** The signed messages and everything decided from them:
  - `DesktopPolicy` (the snapshot): version, issued-at, short validity, the desktop's key and connection, the profile,
    whether the phone's matrix decides, the mode with its end, the matrix, the owner's rules, the connection's tasks,
    the facts the computer needs (owners, repositories, hosts; never people, domains or chats), the GitHub ids those
    facts are kept by, and the workspace roots.
  - `Tighten`: a small "tighten now" (Lockdown, Deny/Review rules, ended tasks, stop deciding locally), applied on
    top of whatever the computer holds, a snapshot or none.
  - `Signed { body, key, sig }` over `reins/desktop-policy/v1\n` + body; `open` checks the pinned key and the
    signature (the Ed25519 check is passed in: `ring` lives in the apps), `check` the binding (desktop key,
    Personal profile) and the times; `effective` merges a snapshot, its tightenings and the local configuration
    (stricter only); `tighten_of(old, new)` is what the phone sends first.
- **Wire (`reins-proto::desktop`).** `PolicyPost { items: [SealedPolicy { connection_id, version, kind, sealed }] }`
  from the phone; `PolicyMail { items: [MailedPolicy], signing_key }` to the computer. `sealed` is the `Signed` JSON in
  a sealed box to the computer's pinned X25519 key: the server relays and keeps opaque blobs.
- **Server.** `POST /reins/api/desktop/policy` (approval device; only the account's connections) keeps, in memory,
  per connection the newest snapshot and the tightenings newer than it (at most 16), and wakes waiters.
  `GET /reins/desktop/policy?after=<version>&wait=<secs>` (the desktop's token) answers what is newer than `after`,
  long-polling up to 25 s, with the approval device's signing key (as `/reins/desktop/tickets` names it).
- **Phone (`reins-core::autopilot::desktop_policy`).** For each paired computer with a pinned key: build the body,
  compare with what was last sent, and when it changed (or is an hour old) sign it with the device key, seal it to
  the computer, and post it, a `Tighten` first when `tighten_of` finds one. Runs with every mode report (each sync and
  pass, and right after a mode change) and right after each policy, fact and task change. Versions only grow
  (`max(last + 1, now in ms)`). No new UniFFI surface: the apps don't change.
- **Desktop (`reins-desktop::policy`).** `state_dir/policy.json` holds the signed blobs (re-verified on every load),
  the highest version applied, and the uses spent of budgeted tasks. The daemon pulls on start and long-polls; a hook
  without a running daemon and a stale store looks once (1.5 s). The hook:
  - **Lockdown** (snapshot or tighten): every tool use is refused.
  - **Fresh snapshot that decides, Auto or Overnight:** the matrix decides on the effective snapshot (Auto allows
    locally; with a net, on `NetPlan.after` when the net can be applied; Ask asks; Park parks the ticket at once;
    Deny only by a rule). **Bypass:** allowed, except by a Deny rule.
  - **Always asked, whatever the matrix says:** the user's own guard rules and the built-in groups about Reins itself
    (`reins`, `secrets`, `payments`); every other built-in group is a label (01 §2.2).
  - **Otherwise** (no snapshot, stale, invalid, Manual, a mode whose end passed, `stop_auto`): today's flow, the guard
    asks the phone; the local unsigned matrix may only add asks (its netted local allow is removed).
- **Replay.** A `desktop` engine: hook cases (commands, files) through the desktop path (the phone's facts as the
  computer gets them, nets, the always-asked groups), the rest as `effects`; the report adds the desktop ask rate.

**Tech stack:** Rust 1.98.1 (edition 2024), serde, `ring` (Ed25519), `crypto_box` (sealed boxes), Rocket, tokio.

**Spec:** `docs/design/decisions-v2/` 01 (§2.2, §4, §10–§12), 02 (§3, §5), 06 (§2, §8), 08 R1, R3, R6 (08 wins);
plans `2026-10-26-dv2-p2.md`, `-p3.md`, `-p5.md` (their "not in" notes on the signed snapshot), `-p4.md` (the pinned
signing key, `TicketStore::see_signing_key`).

## Global Constraints

- **Only the phone loosens.** Local `[auto]` (facts, matrix, `matrix_decides`), `[workspace]` and `[guard]` can only make
  a decision stricter while a signed snapshot decides:
  - a local fact applies only where the signed one exists and the local zone is stricter;
  - a local matrix that isn't the default tightens cell by cell (stricter of the two);
  - local workspace roots count only inside a signed root; scratch folders only strictly inside one;
  - `disabled_groups` and the allow lists never switch off the Reins groups;
  - local `matrix_decides`/`setup_done` play no part (the snapshot's `decides` does).
  Without a snapshot the local matrix only adds asks.
- **Same outcomes as the phone:** `matrix::evaluate` with the snapshot's matrix, rules and tasks, `connection` = the
  computer's connection id (tasks and rules are per connection), `overnight` from the mode, `netted` from the plan.
  Auto never denies; Deny only by a rule.
- **Freshness:** valid only while `now < not_after` (6 h after issue; the phone refreshes hourly), `issued_at` at most
  5 min ahead, a validity of at most 24 h, a version above the highest applied for that signing key (rollback), signed
  by the pinned key (P4), for this computer's key and the Personal profile (08 R6). Anything else: fall back to asking.
  A mode whose `until` passed: fall back.
- **Tightenings apply at once,** with or without a snapshot, until a snapshot of the same or a later version arrives
  or they expire (24 h). Lockdown refuses every tool use.
- **Budgets:** a budgeted task's uses spent here are counted here and subtracted from the signed `uses_left`.
- **Agent tampering:** `reins/policy.json` and `reins/tickets.json` (the pinned key) join the `reins` guard group;
  the blobs are re-verified on every load. For shell, Reins stays a guardrail (08 R1); the boundary is credentials.
- **Notifications:** none. Local decisions show in Activity only.
- **Private data:** people, domains, chats and the owner's addresses never leave the phone; the server sees only
  sealed blobs, versions and kinds.
- **Builds:** `source ~/Documents/dev/tmp/desktop-snapshot/env.sh` (`CARGO_TARGET_DIR` in the worktree, `TMPDIR` and
  `XDG_CONFIG_HOME` under `~/Documents/dev/tmp/desktop-snapshot/`, no swiftly on `PATH`), `-j 4`, narrow `-p`;
  pinned clippy (`PATH=$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin:$PATH`, `CARGO_TARGET_DIR=target-198`
  in the worktree); never `/tmp`. Commits signed, authored by Daniel Katulevskiy, never mentioning AI.

## Review Focus

1. **Replay or rollback:** an older snapshot (a lower version, or one superseded) from the server or a restored file is
   refused — `older_versions_are_refused`.
2. **Key substitution:** a snapshot signed by another key, or sealed by anyone to the computer's key but signed by a
   key not pinned, is refused; a changed key is held until `reins tickets --trust-key` — `another_key_is_refused`.
3. **A stale snapshot loosening:** past `not_after`, a mode past its `until`, a far-future `issued_at` — fall back to
   asking — `stale_falls_back_to_asking`.
4. **Cross-profile:** a snapshot for another desktop key or a Managed profile is refused — `other_profile_is_refused`.
5. **Agent tampering:** a config that sets Auto everywhere, Mine facts, `/` as a root, `disabled_groups = ["reins"]`
   never loosens; an edited `policy.json` fails verification — `agent_config_cannot_loosen`,
   `edited_store_is_refused`.

## File structure

| File | Responsibility |
|---|---|
| `crates/reins-policy/src/desktop_policy.rs` (new) | messages, signing bytes, open/check, effective policy, `tighten_of`, desktop facts |
| `crates/reins-policy/src/guard.rs` | `REINS_STATE` gains `reins/policy.json`, `reins/tickets.json`; `always_asked` groups |
| `crates/reins-proto/src/desktop.rs` | `PolicyKind`, `SealedPolicy`, `PolicyPost`, `MailedPolicy`, `PolicyMail`, limits |
| `src/api/reins/{mod,device_api,desktop_routes}.rs` | the hub's policy mailbox, the two routes, tests |
| `crates/reins-core/src/autopilot/desktop_policy.rs` (new), `engine.rs`, `phone_api.rs`, `api.rs` call sites | build, sign, seal, post |
| `crates/reins-desktop/src/policy.rs` (new), `hooks.rs`, `consequences.rs`, `daemon.rs`, `server/client.rs` | store, verification, pulling, deciding |
| `crates/reins-replay/src/{engine,report,main}.rs`, `tests/gate.rs` | the desktop path and its ask rate |
| `crates/reins-e2e/tests/desktop_policy.rs` (new) | phone → server → computer, end to end |

---

### Task 1: The signed messages and the effective policy (`reins_policy::desktop_policy`)

**Files:** create `crates/reins-policy/src/desktop_policy.rs` (inline tests); modify `lib.rs`, `guard.rs`.

- [ ] Step 1: failing tests: `signed_bytes_are_domain_separated`, `open_checks_key_and_signature` (a fake verifier),
  `check_binds_key_profile_and_time` (other desktop key, `org:acme`, expired, issued 10 min ahead, validity 48 h),
  `desktop_facts_keep_owners_repos_hosts_only`, `local_config_only_tightens` (Auto-everywhere matrix, a Mine fact for
  an unknown owner, `/` as a root, a scratch equal to a root), `tightenings_apply_until_superseded`,
  `tighten_of_finds_lockdown_rules_ended_tasks_and_stricter_cells`, `budgeted_tasks_spend_locally`,
  `always_asked_groups`.
- [ ] Step 2: `cargo test -p reins-policy -j 4 desktop_policy` fails; Step 3: implement; Step 4: pass, pinned clippy.
- [ ] Step 5: commit `feat(policy): the phone-signed desktop policy and how the computer applies it`.

### Task 2: Wire types (`reins-proto::desktop`)

- [ ] Types and limits above, with a round-trip test; commit with Task 1 or alone.

### Task 3: Server

- [ ] Tests: `policy_mailbox_keeps_the_newest_and_newer_tightenings`, `a_waiter_wakes_on_a_post`, `older_versions_never_replace`.
- [ ] Routes; `cargo test -j 4 --features sqlite policy`; commit `feat(server): relay the phone-signed desktop policy`.

### Task 4: Phone

- [ ] Tests (`reins-core`, `--features testing`): `body_has_no_private_facts`, `published_on_change_and_hourly`,
  `lockdown_sends_a_tighten_first`, `sealed_to_the_pinned_key_and_signed_by_the_device`, `versions_only_grow`.
- [ ] Implement; `cargo test -p reins-core -j 4 --features testing desktop_policy`; commit
  `feat(core): publish the signed policy snapshot to paired computers`.

### Task 5: Desktop

- [ ] Tests: the Review Focus tests, `fresh_snapshot_decides_like_the_phone`, `lockdown_refuses_every_tool_use`,
  `overnight_parks`, `bypass_allows_but_deny_rules_hold`, `reins_groups_are_always_asked`,
  `no_snapshot_keeps_todays_flow`, `local_matrix_never_allows_without_a_snapshot`.
- [ ] Implement the store, the daemon's pull and long poll, the hook; `cargo test -p reins-desktop -j 4`; commit
  `feat(desktop): decide hook requests with the phone-signed policy`.

### Task 6: Replay

- [ ] `--engine desktop`; gate: desktop ask rate ≤ the effects developer ask rate with nets + what the Reins groups
  add, recall 100 %; commit `feat(replay): the desktop path and its ask rate`.

### Task 7: End to end

- [ ] `crates/reins-e2e/tests/desktop_policy.rs`: the phone sets cells, the computer decides the same way as the
  phone's matrix; a stale or tampered snapshot asks; an agent-edited config can't loosen; Lockdown reaches the
  computer in under 5 s.

### Task 8: Docs and gate

- [ ] `docs/security-model.md` (the snapshot, the trust statement), `docs/design/decisions-v2/07-roadmap.md` only if
  it names the gap; this plan's "What changed while building".
- [ ] fmt; pinned clippy on `reins-policy reins-proto reins-core reins-desktop reins-replay` and the server; tests;
  replay `--check`; the e2e; self security pass over Review Focus; push; PR (not merged).

## Not in this plan

- Credentials the phone holds (git push credentials, secret releases, SSH signatures, connector calls) stay the
  phone's to decide (08 R1).
- A desktop app screen for the snapshot: `reins status` names it; Activity shows each local decision.
- A Managed profile's command policy on the computer (O1/R6: org command policy applies to Managed agents through the
  Managed daemon).
