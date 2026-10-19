# Mandatory authorization runtime: first delivery slice

This is the first enforcement integration of [Authorization v1](authorization.md), not the completed permissions
product. There is no mobile policy editor or new account default yet. Existing personal installations with no
authorization state keep their existing behavior. No learning history is converted into permission, no account
is shown to a new agent, and existing exact-command identities are unchanged.

## Active capabilities

- A mandatory `deny` applies before every relayed tool dispatch: Gmail, connector reads/writes, MCP, account and
  integration enumeration, desktop/SSH/secrets calls, payment-specialized operations, and upload-slot creation.
- `require_review` can be activated for the exact operation `gmail / write / send`. A requirement for the personal
  `owner`, quorum one, without a distinct-person group or requester exclusion is satisfied by an explicit local
  approval. Existing grants, retry passes, Auto, Bypass, and Overnight cannot satisfy that requirement.
- External review requirements for Gmail sends remain held. A local owner tap never impersonates Legal, Security,
  a second reviewer, or the members of a distinct-person group. Organization issuers cannot be activated without
  the organization authority implementation.
- Owner/primary Rust APIs query and save complete validated policy records using state CAS. Lifecycle changes
  use record revision CAS. The device owner confirms edits; the engine refreshes the primary role before writing.
  Conflict resolution is a separate owner-confirmed API. Policy IDs, owners, kinds, and revoked IDs are immutable.
- `permit`, `eligible_for_auto`, delegation authority and review policies for other operations are visibly rejected
  at activation. An imported matching unsupported outcome holds execution; it cannot become a legacy grant fallback.

`account_resource_id(service, account)` is the canonical JSON tuple of the integration service and the exact account
identity registered locally by that integration. Provider-canonicalized addresses are used after resolution. An
agent's display label, repository path, mail domain, or command summary is never organization identity evidence.
The requesting paired-device identity is currently absent; policies that need it hold rather than trust a label.
Recipients are verified from the normalized complete outgoing email, including every recipient.

MCP tools use `mcp_<server> / tool / call`, with the exact original tool name as the bound `resource_id`. This
preserves valid Unicode, spaces, slashes and other raw names without lossy sanitization. Future positive MCP rules
must enumerate exact tool resources; a generic operation-family selector alone must not inherit new tools.

This first gate uses the evaluator's legacy baseline solely to *narrow* existing authorization paths. It does not
prove account access and cannot be reused to activate new permits. The later permit adapter must supply independent
account-access evidence and replace the managed scope's legacy baseline with `review` or migrate those old grants.
Removing an agent/account/operation from an explicit policy must not revive its earlier legacy grants.

## Enforcement points

`process_request_unguarded` checks before dispatch; canonical account resolution checks again before contacting the
provider. Connector and MCP execution and disclosures check again after previews. `approve_claimed` checks existing
parked requests with an explicit `OwnerReview` or `Automatic` source. Notification and batch approval use the same
owner entry point. Specialized purchase approval and uploaded-file release have independent final checks.

The Gmail send fast path first checks whether automatic execution is permitted, then consults the legacy grant
store. Mandatory review parks the exact request without spending an existing grant. The send checks policy again
at the effect boundary. A policy installed while an item was pending therefore applies to that item too.

Denial and unresolved policy stop before provider calls or on-device model prompts. A denied read cannot return
provider content through a result or provider error. The policy error describes the restriction without account
names, credential values or provider content. Denying a request remains available; policy does not prevent users
from taking authority away.

These checks establish a pre-execution boundary. They do not promise instantaneous revocation of a provider action
already in flight, rollback of sent messages, exactly-once external effects, or control over independent credentials
held outside Reins. Pairing, owner recovery, vault editing and policy administration remain separate control-plane
flows; a tool policy is not a replacement for their owner/role authentication.

## Persistence and synchronization

Authorization state is a bounded, versioned `authorization.state` Meta record in the existing account-owned,
encrypted in-memory SQLite store and authenticated sealed snapshots. The existing signed primary op log carries
it; ordinary viewer proposals cannot write policy. Local edits use the store's process-safe single-writer lock.
The aggregate has a 48 KiB encoded limit, leaving room inside the 64 KiB op-body limit, a separate 40 KiB policy
content budget reserving hold metadata space, and 64 policy/revocation identities. No schema migration creates authority.

Concurrent updates do not use ordinary JSON field merging or wall-clock last-write-wins. They retain a conflict
and the conflicting policy versions for explicit owner resolution. Revocation IDs never disappear. State removal,
unverified checkpoint/recovery data, an older snapshot that lacks the installed state, unsupported versions,
corrupt records and ownership mismatch all hold execution. They never mean "no policies". The same conservative
rules are used by snapshot merges and signed-op application. A clean later authenticated CAS is needed to resolve
a conflict; its update binds the canonical digest of the exact conflicting versions. Neither a same-revision nor a
higher stale branch backup can silently resolve it. Held-state updates retain all known competing versions.

If a conflict union exceeds these bounds or belongs to another owner, the last readable state stays held with a
bounded source commitment instead of writing an unreadable policy union. Restrictive owner recovery remains
available, and the permanent `overflow_digest` marks incomplete history even after that recovery. Positive-permission
adapters MUST reject that marker until the complete policy/revocation history is reconciled; clearing the visible
conflict does not establish that omitted authority was safely recovered.

The current aggregate intentionally has a small configuration bound. Larger enterprise policy sets and archival
revocation retention need a separately versioned storage scheme before those capabilities are enabled. Old clients
do not enforce these new policies. Managed deployment must require a supported enforcement version before claiming
organization guarantees; server/enforcer capability negotiation is still pending.

## Next implementation slices

1. Carry a captured, structured authorization trace (policy versions, evidence provenance, review source, exact
   action digest) through each execution and activity result, rather than recomputing it after an effect.
2. Extend supported review gates to connector writes and MCP calls, then metadata/body reads and account discovery;
   model preview collection separately from agent disclosure. Expose capability coverage to native settings.
3. Implement independently verified account-access prerequisites and operation/resource facts for each adapter.
4. Activate explicit permits only with exact execution receipts, policy-version revalidation, expiry/revocation,
   replay protection, and one shared atomic reservation across every parent budget. Unknown outcomes keep reservations.
5. Add owner-created scoped delegations (including equally scoped/expiring issuance authority), pause/revoke controls,
   usage limits, and deterministic conflict handling. No lifetime parent permission is created implicitly.
6. Migrate managed account/operation scopes away from legacy grant fallback before explicit permits are enabled.
7. Publish stable native APIs and build matched Android/iOS permission editors, account onboarding success/defaults,
   session controls, rule preview and clear effective-decision explanations.
8. Authenticate organization policy bundles, membership/role leases and reviewer receipts; enforce quorum, separation
   of duty, changed-content invalidation, and supported-client capabilities on every execution point.
9. Add accurate approved/started/succeeded/failed/unknown execution states and provider idempotency where available.

Tests cover legacy equivalence through the existing suite, corporate review versus all automatic modes and standing
grants, quick approval, late deny installation, account/connection isolation, canonical account resolution, all
dispatch families, unsupported activation, owner refusal, CAS concurrency, permanent revocations, snapshot rollback,
unverified restore, and signed-op conflict/removal semantics.
