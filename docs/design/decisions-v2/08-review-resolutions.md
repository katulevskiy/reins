# 08 · Review resolutions

An independent builder review of 00–07 asked twelve questions. They're answered here. **Where this file and an
earlier document disagree, this file wins**, until the earlier documents are folded into one revision. Each answer
says whether it's a design call (made by the author, open to change) or still open for the product owner.

## R1. Where the engine runs for coding-agent requests, and how far desktop facts are trusted

*Design call.*

- **The desktop daemon runs the consequence engine and the context graph** for requests that start on the computer
  (hooks, `reins ask`, git through the proxy). It has the facts: cwd, env, globs, worktrees, generated areas.
- **The phone signs the policy snapshot** (the matrix, rules, zones and fixed overrides) with the owner key. The
  daemon keeps it and evaluates **shell-command** requests against it locally:
  - Auto runs without a round trip, and works with the phone offline.
  - Asks go to the phone as today.
- **What the daemon can't decide locally:** anything that needs a credential the phone holds (git push credentials,
  secret releases, SSH signatures, connector actions). The phone still releases those. For them the daemon sends its
  effect facts, and the phone evaluates the same signed policy.
- **Trust statement, made explicit in `security-model.md`:**
  - For shell commands, Reins is a **guardrail, not a boundary**. An agent running as the same OS user can run
    commands without hooks, or edit the daemon's state.
  - Hook-level Auto decisions trust the daemon's facts for exactly that reason: an agent that can forge them didn't
    need Reins' permission anyway.
  - The boundary is around **credentials**, which stay on the phone and are released per action under phone-evaluated
    policy.
- **Hardening option (P1+):** run the daemon as a separate OS user (macOS launchd agent with its own user, Linux
  system user), so a same-user agent can't read its key. Recommended for Managed devices (MDM-installed).

## R2. Safety nets for actions the agent's own process runs

*Design call.*

- **Hook rewrite where the harness supports it.** Claude Code's `PreToolUse` can return `updatedInput`, and other
  harnesses are added as they support it. Reins rewrites the command to apply the net:
  - `rm -r X` becomes `reins trash X`;
  - `git push -f` becomes `reins git backup-then-push …`;
  - `git branch -D b` becomes `reins git backup-ref b && git branch -D b`.

  Approval is bound to the rewritten command's digest.
- **Where a rewrite isn't possible, the net is advisory**, and the request is levelled **as if there were no net**.
  `rm -r` in workspace source is then L1 only if the daemon can confirm the paths are tracked by git (recoverable from
  history); otherwise L3.
- **Through Reins' own proxies**, nets are always enforced: git push through the proxy, connector actions, and
  secrets.

## R3. Matrix cells are soft; obligations are hard

*Design call. It corrects 02 §3.*

| Kind | Compiles to | Can a permit satisfy it? |
|---|---|---|
| Matrix cell "Ask" | `review_by_default` (the existing soft fallback) | **yes**: promotions, accepted repo permits, tasks and Overnight's looser L1 cells are permits that satisfy it |
| Fixed overrides (01 §4), Circle, guardian and org reviews, repo `review` rules | `require_review` (hard) | **no** |
| Deny (Lockdown, "never" rules, org deny, repo deny) | `deny` | no |
| Overnight parking | a separate `park` effect | — |

- **Permits combine as OR** (any one qualifying permit suffices), per the authorization foundation.
- **Delegations and tasks are AND bounds**: a task's permit is valid only inside its envelope and ceiling, and only
  within the issuer's own authority.
- The "precedence" list in 02 §3 is replaced by these two rules. Issuer order matters only for which *explanation* is
  shown.

## R4. The always-ask list (security floor), restated

*Design call. It tightens 01 §4.*

These are hard obligations (owner review, Instant by default) in Auto and Overnight. Only Bypass covers them, and only
for the owner's own obligations:

- **Secrets:**
  - `FileRead{kind: Secret}`: private keys, `.ssh/*` keys (not `config` or `known_hosts`), cloud credentials,
    keystores, `.git-credentials`, token files. Reading a secret puts it in the agent's context, which is a secret
    leaving the device.
  - `SecretOut` to any destination not seen before.
  - Every vault secret release to an AI (as today: always asked).
- **Desktop secret releases** (`reins run`, the API proxy) and **SSH signatures**: asked, unless an explicit per-item
  (or per key-and-server) permit exists, as today.
- **Connections and phones:**
  - new AI connections and new phones (even in Bypass);
  - the **10-minute quarantine** for new connections, unchanged;
  - account listing (`reins_list_accounts` with addresses).
- **Other effects:**
  - uploads that may execute (scripts, binaries, archives);
  - money over budget;
  - publishing;
  - permissions;
  - mass messaging;
  - certainty `None`.

`.env` reads stay judged by zone and level, as today: L1 in the workspace, never sent.

Effect on the replay estimate: about +1 ask per day on the sample, since 1 of the 45 "key" hits was real. The target
holds.

## R5. Managed: a compromised decision service can't execute Auto decisions alone

*Design call. It tightens 05 §5 and 06 §8.3.*

- Executors **re-evaluate every request independently** with the same engine library, against the **policy snapshot
  signed by the org issuer key**. That key lives in an HSM, and activating a policy takes two Security admins. An
  executor refuses if:
  - the decision service's verdict and its own differ;
  - the snapshot isn't signed by the issuer;
  - a review-required request lacks valid reviewer device signatures.
- Executors run in a separate trust domain from the decision service:
  - a separate process, identity and keys;
  - on Dedicated and Self-hosted, a separate network zone.
- So executing a non-compliant action needs both the decision service *and* the issuer key or an executor
  compromised.

## R6. Which profile a request belongs to

*Design call. It clarifies 00 and 06 §2.*

- **An agent connection belongs to exactly one profile,** and so does a desktop daemon identity. A Managed daemon
  identity is installed by MDM or by SSO sign-in.
- **Requests carry their connection's profile.** No request ever crosses profiles.
- **Org guarantees come from credentials,** not from watching personal agents:
  - org repos, clouds and data are reachable only through Managed connections and leases;
  - a Personal-paired agent in an org worktree simply has no credential, so its push fails (an ordinary auth failure,
    not a routed request).
- **Hook-level commands** in a worktree are evaluated by the daemon of the agent's own profile. The org's command
  policy applies to Managed agents only. Orgs that need more use MDM to keep org repos on managed devices with the
  Managed daemon.
- The org never learns a Personal profile exists, because nothing from it ever reaches the org.

## R7. Guardianship in Personal mode is cooperative

*Open for the product owner; a proposed default follows.*

In Personal mode the ward's own phone holds the credentials and evaluates policy, and the server is blind. A ward can
reset the account, start another one, or use credentials directly, so guardianship there **can't be enforced against a
non-cooperating ward**.

Proposal:
- **Personal guardianship is cooperative and tamper-evident:** the guardian is notified if the arrangement ends, the
  account is reset, or the roster changes.
- **Enforceable guardianship** (for minors) is a **Family** tenant, a small Managed mode where the guardian is the org.
  The guardian provisions the connections and holds the credentials, so the child's agents act through the family's
  managed connections.

## R8. Circle keys, rotation and metadata

*Design call. It completes 04 §5.*

- **Helpers decide from their approval device only.** Viewers (second phones) still can't approve. Packets are sealed
  only to the helper's approval device.
- An answer is signed by the helper's **approval-device signing key**, accompanied by the helper's current **roster
  chain proof**. The owner's phone verifies three things:
  - the signing key is the helper roster's current primary;
  - the roster chains to the **owner key pinned by the safety code at setup**;
  - the arrangement scope and request digest match.
- **Handover** (a new approval phone) changes the primary but not the owner key: it's verified through the chain and
  needs no re-pairing. **Recovery** changes the owner key, so the arrangement suspends until the owner and helper
  compare a new safety code.
- **Metadata:** the server learns who reviews for whom (packet routing). This is accepted for P6 and stated in
  `security-model.md`. Sealed-sender routing is a later option.

## R9. Deferred results and webhooks in Personal mode

*Design call. It completes 03 §4 and §9.*

- **Reads** (search mail, read a calendar) **execute on collect**. Approval stores only the decision. The phone runs
  the read when the agent calls `reins_get_result` or `reins_wait`, with the phone online, and the result goes straight
  to the agent. Results aren't kept on the server.
- **Writes** (send, create, push) execute on approval, since the agent may never come back. Their result is minimal
  (a message id, an HTTP status, a commit SHA) and is kept on the server for the ticket's lifetime.
- **Webhooks** are signed by the **approval device's** signing key, never the server's. Agents verify the signature
  against the device key shown at pairing. So a compromised server can't fake `ticket.approved`, though it can delay
  or drop. The server delivers events the phone signed.

## R10. The email send hold

*Design call. It fixes 01 §5.*

- The Gmail API has no scheduled send, so the hold works like this:
  1. The phone seals the outgoing email to itself and gives the server the ciphertext plus a send-at time.
  2. At send-at, the server wakes the phone with a push, and the phone sends.
  3. **Undo** before then deletes the ciphertext.
- The server never sees the body in clear.
- If the phone can't be reached at send-at, it sends when it next wakes, unless more than 15 minutes have passed. Then
  it asks the owner ("Still send?").
- iOS background limits make the wake best-effort, so the send may be later than 60 s, never earlier.

## R11. Team tier is Managed

*Design call. It corrects 05 §11.*

- **Team** is Managed mode with a lightweight identity provider: Google Workspace or GitHub org sign-in through
  WorkOS, plus domain verification. There's no SAML or SCIM; member lifecycle follows the provider's membership.
- **Business** adds SAML SSO and SCIM.
- A team that doesn't want any of this uses Personal accounts with Circles (04).
- The roadmap adds Team in O1.

## R12. Two smaller points

- **The owner's own L3 reviews are Instant by default,** with **Decide later** on the card. They're Deferred when:
  - a policy says so (Circle, guardian, org, repo);
  - Overnight is on;
  - an Instant window ran out.

  03 §1's table is read that way.
- **Tenant isolation on Reins Cloud:**
  - **Team and Business:** a shared multi-tenant service with logical isolation and per-tenant data keys (envelope
    encryption under each tenant's KMS key).
  - **Enterprise:** a dedicated deployment (its own VMs, database and keys) in the chosen region.
  - **Executors:** per tenant in all tiers (an isolated process or container per tenant with its own identity), given
    R5.
