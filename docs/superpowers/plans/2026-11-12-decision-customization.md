# Decisions v2: fewer routine asks, and settings the owner can reach

**Goal:** stop asking the owner about the same routine, harmless requests, without widening anything unknown:
a temp folder the owner can set per computer, local branch clean-up, the first command after the phone has been idle,
and folders Reins can learn from what the owner keeps approving, with the owner's consent each time.

**Measured with:** one computer's Activity, 28 October to 12 November 2026 (after Decisions v2 P0–P5 and the signed
desktop policy): 108 hook requests that matched something, 42 decided on the computer, **66 asked on the phone, 64 of
them approved** (none denied, two unanswered), answered in 10 s at the median. An approval rate of 97% at that speed is
the owner approving by habit, which is the failure Auto exists to prevent.

## The 66 asks, by cause

A request can have more than one cause; each is counted under every cause that alone would have asked it.

| Cause | Asks | What decides it |
|---|---|---|
| **The temp folder is read as a project** | 22 | `rm -rf ~/Documents/dev/tmp/<run>`: `~/Documents/dev/tmp` was never set as a scratch folder on that computer (no command sets it; `[workspace]` is undocumented), so it is a project named `tmp` under the root `~/Documents/dev`: a delete in source that git can't bring back, L3. For 16 of them this is the only cause. |
| **The computer's copy of the rules had run out** | 30 | The computer was not deciding (`decided: false`): the signed snapshot is used for 10 minutes and the phone signs again only while it is awake, so the first guarded command after the phone has been idle goes to the phone, where a hook question is not read by the matrix at all and the word list of the old floor (`rm -r`, `git branch -d`) asks. In 7 of them the matrix said Auto (`cat ~/.ssh/config`, a project's `.env`, a delete in build output). |
| **A program the engine has no model of** | 12 | `timedatectl show`, `findmnt`, `losetup -a`, `pdfunite`, `docker`, `udisksctl`, `mise`, an agent CLI's `--version`, and `reins tickets` itself: certainty None, always asked. |
| **Reads one key file** | 8 | The same token file read into a `curl` to a loopback port, each time: the secrets floor (08 R4). |
| **Writes in the home folder, outside the projects** | 9 | An agent's own memory and settings files (`~/.claude/projects/*/memory`, another agent's `~/.<tool>/agent/*.json`), `~/.claude.json`, new folders in `~`. Outside, L3. |
| **Reaches a host on the LAN** | 3 | `ssh`/`nc` to a host with no zone: Outside, L2. |

The older log of another computer (16–18 October, before P1) shows the same shape with the keyword guard: 33 asks
for `rm -r` (most in `/tmp` and build output) and 13 for `git branch -D` right after the branch's pull request was
merged.

## Why the owner's answers taught nothing

Read from the code, each with where it is decided:

1. **What is always asked is never remembered.** `ap_learn` writes a memory row only when the model judged the
   request (`engine.rs`, `if let Some(j) = &s.judged`); the floor, the default-branch rule and a matrix Ask all return
   before the model runs. A thousand approvals of `rm -rf` in a temp folder leave no row, no neighbour, no adapter
   example, and the kind never appears on the profile page.
2. **What does reach the model is always new.** A hook question's target is the exact command in its folder
   (`connector/desktop.rs::exact_resource`); a GitHub write on a branch is `owner/repo@branch`. A new target never
   approves and its shadow verdict is Ask, and rows whose shadow verdict is Ask are left out of the agreement that
   relaxes the cold bars (`gates.rs::trust`). Trust stays 0.
3. **The matrix is static, by design** (01 §7: no silent drift). The designed way to learn, a promotion offer after
   repeated approvals that writes a visible rule, is not built: the phone creates no owner Permit rule.
4. Smaller ones: an approval given while the model was not loaded, under Bypass, or covered by a standing permission
   is not learned; the adapter never trains while every answer is an approval; a model update sets everything aside.

## What this branch does

1. **`reins folders`** (computer, `reins_desktop::folders_cli`): lists the work folders and scratch folders with how
   each stands, adds and removes them (`reins folders temp ~/Documents/dev/tmp`), and says what the phone still has
   to confirm. `/tmp`, `/var/tmp`, `/dev/shm` and `~/.cache` stay scratch everywhere. A scratch folder the phone
   signed now counts on the computer without a local entry (`clamp_scratch`), so confirming a card is enough.
2. **Folders learned from approvals** (`reins_desktop::suggest`; the computer proposes, the owner confirms on the
   phone): when the owner approved at least three requests that deleted at least three different folders directly
   under one folder in 14 days, and denied none there, the computer suggests that folder as scratch, and its own
   `$TMPDIR` when it is not a standard one. Suggestions travel as their own list of the proposal
   (`FolderProposal::suggested_scratch`; an older phone or server ignores it). The phone shows the existing folder
   card, also for a folder inside a signed root; confirming needs the owner check; only then is it signed and
   counts. Nothing loosens by itself: a forged journal or proposal yields at most a card that names the exact
   folder. On the log above it suggests `~/Documents/dev/tmp` and nothing else.
3. **Local branch deletes are read** (hook and engine): the hook reads the repository's default branch for any
   command that names git, not only a push. That alone is not enough where the hook's strict reader of git's
   configuration reports nothing (an `insteadOf`, which `reins git setup` writes, an include), so a local
   `git branch` delete, rename or move of a name that is not a long-lived one is L1 with no default known
   (`effects::git::local_default`): one local ref goes, its commits stay until gc. Long-lived names, the default
   that was read (compared without regard to case), patterns, the previous branch and, with `-D`, a name that is not
   in the command are still taken as the default. Pushes, forge calls and `update-ref` are read as before.
4. **Models for forms that only look** (engine): `timedatectl`, `findmnt`, `losetup` (listing), `blkid`,
   `pdfunite`, `qpdf` (one input, one output), an agent CLI's `--version` and `--help`, and Reins's own state
   (`tickets`, `pending`, `net list`, `harness list`, `folders`, `--help`). Each is closed, and holds for the
   installed program only: a file of that name, or a name found on a `PATH` the line set, is not it.
5. **The phone decides a hook question with the computer's reading** (`reins_policy::desktop_reading`,
   `Engine::ap_hook_item`): a hook's question carries what the computer read of the request, and in Auto and
   Overnight, where the matrix decides, the phone's own matrix decides it, as the computer would have with a fresh
   snapshot, without the model, so also in a pass that has none. The owner is asked only where the matrix asks.
   The 10-minute validity, the rollback protection, is unchanged. What the phone checks itself:
   - the question is a hook's (its layout, and a digest of the command, folder and net that the phone makes again
     from what it shows), never `reins ask`;
   - every folder of the workspace the reading was read with counts under what the phone signs for that computer
     now, else the whole reading is unused (the workspace decides more than paths: what a script or a database file
     is read as); each path is then placed again, and the level and floor are worked out again from the effects;
   - the computer's own `[auto]` only tightens; Reins's own groups, a young connection and Auto's rate limit hold.

   A reading goes only to a phone whose signed snapshot says it takes them (`"reading": 1`): a phone re-checks
   every call against its own tool description, and an older one would refuse the question. A computer the phone
   knows no home folder for has no usable reading.

## What is left, and proposed next (each needs a decision by the owner, and a screen)

| Proposal | What it would do | Why it is not in this branch |
|---|---|---|
| **A strictness setting per AI** ("Careful", today's; "Relaxed") | In Relaxed, L3 inside the owner's own projects and commands that could not be fully read but stay inside the workspace become soft asks Auto may take; the R4 floor (secrets, publishing, permissions, money, mass messages), anything Outside and anything that reaches people still ask. A bounded alternative to Bypass, which the same log shows in use for 38 requests. | Changes which asks are hard (01 §4, 08 R3): the owner's call, with a hold and the owner check, and a screen on both phones. |
| **Programs the owner knows** | "Treat `docker` as a tool that works in my projects": the unread program becomes Partial, L1 in the workspace, instead of None. Offered after a few approvals of the same program, confirmed like a folder. | A new signed list and its cards. |
| **Promotion offers** (01 §7) | After three approvals of the same (zone, effect kind, resource) in 14 days, "Let Auto do these here": one visible, removable Permit rule. This is the learning the design intends for GitHub and messages. | Needs the offer on the approval card, the Rules list, and a record of approvals of asked requests (cause 1 above). |
| **One file, one use** | A standing answer for exactly one secret file read by one AI for a set time, so the eighth identical read in an afternoon is not a new ask. | 08 R4 lists secret reads as covered only by Bypass; widening that is a security decision. |
| **A zone for LAN hosts** | Hosts on the owner's own network named Mine once, on the phone. | Exists as a fact (`FactKind::Host`); needs the question card to be offered for hosts a command names. |

## Checks

- **The log above, replayed** (its 64 hook questions through `reins-replay`, the standard roots, no known worktrees
  or facts, so a floor of what the computer itself reads): all 64 were asked when they ran. With this branch's
  engine and the default matrix, 52 are asked without nets and 44 with them; with `~/Documents/dev/tmp` a scratch
  folder, **33**, with or without nets. What is left is the table of proposals: the key file (11, some also unread),
  unread programs (`docker`, `udisksctl`, `script`, `mise`), writes in the home folder, and LAN hosts.
- **Replay gate** (`reins-replay run --engine effects --check`, and with `--policy crates/reins-replay/policy/nets.json`):
  passed both ways. Real-risk recall 263/263 (23 new risky cases), known false positives asked 0/75, developer asks
  42 of 738 (5.7%) without nets and 27 (3.7%) with them, as before: the corpus computer already had its temp folder
  set and a default branch known, so the 7 new routine cases carry the change (asked before, Auto now).
  `baseline.json` is unchanged.
- **Tests** (aarch64 Linux, `RUST_MIN_STACK=16777216` as in CI): `reins-core --features testing` 1395 passed;
  `reins-desktop`, `reins-policy`, `reins-proto`, `reins-replay` 1275 passed; `reins-e2e` 79 passed (all 24 targets);
  the server's `desktop::` tests 14 passed. Clippy `-D warnings` on those crates and the server; `cargo fmt`.
- **Not run here:** the Android and iOS suites and `reins-desktop-app` (no SDKs or fontconfig on this computer; no
  file of theirs changed, and the UniFFI surface is the same), Windows and macOS.
- **Known flake, not from this branch:** `crates/reins-core/tests/github_repos.rs` and `github_actions.rs` fail now
  and then on a stray `HEAD /` their mocks count (2 of 8 runs on `main` at 6be09fa5 as well; `github_api.rs` and
  `github_code.rs` leave that request out of their counts).
