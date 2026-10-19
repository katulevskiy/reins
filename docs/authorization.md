# Authorization v1 foundation

This document describes the additive, IO-free foundation in `reins-proto::authorization` and
`reins-policy::authorization`. It does not claim that storage, mobile settings, signed receipts, enterprise
membership, atomic budget reservations or execution enforcement are implemented by these crates. Existing
`Grant` and `Scope` APIs and exact shell-command identities are unchanged.

## Trust and integration boundary

`ActionContext` and `PolicyRecord` are strict, versioned wire claims. Parsing them establishes no authority. The
runtime must authenticate identities and policies, normalize the exact original request, collect evidence, obtain
the complete applicable policy set, and construct the deliberately non-serializable `AuthorityBindings` for the
same immutable values. Never derive organization membership from an email suffix, account ownership from a display
name, or repository identity from an agent's description. Cloud agents have an authenticated connection without a
paired computer; their requesting `device_id` is absent. Credential-owning executor and human beneficiary are
separate identities, even when a personal installation uses the same principal for both.

`AuthorityBindings` must bind the principal/tenant, requesting connection/device, executor principal/device, account,
optional resource, and exact request digest. `account_access` is a separate authorization prerequisite and cannot
be established by a new account default. `verified_attribute_keys` attest the exact values and provenance in this
action, not merely an attribute name or an evidence ID that can be copied from another request.
`authorized_policy_ids` attest the exact supplied record contents/revisions and the issuer's authority, not an ID
allowlist usable with arbitrarily changed records. These Rust bindings must never be accepted from an agent's JSON.

The evaluator cannot discover omitted mandatory policies, verify a signature, or determine that a membership lease
is fresh. Those checks belong to the trusted runtime. Failure to obtain a complete authenticated policy snapshot
must stop automatic authorization before invoking the evaluator.

## Fixed v1 schema

An action contains its exact SHA-256 request digest and a separate `OperationId {service, capability, action}`.
Operation selectors use exact tuples only. They cannot prefix-match future capabilities. Actor/account/resource
selectors use stable authenticated identities. Optional facts carry a typed key/value, evidence source and a
bounded validity interval; absence, untrusted assertions, unresolved values and stale evidence remain unknown.

Policies have a version, stable ID, positive CAS revision, authenticated issuer/tenant, lifecycle, validity interval,
scope, conjunctive conditions, effect, optional delegation parent IDs, and usage limits. The lifecycle is `active`,
`paused` or `revoked`; revoked records remain authenticated tombstones. Personal policies belong to the credential
owner; organization policies match their exact tenant. Permitting effects enumerate accounts, agent connections and
operation tuples explicitly. Account defaults cannot silently give every future agent access.

Conditions are fixed string equality/membership, set containment, boolean equality, and unsigned upper bounds.
There is no freeform expression, regex, shell execution or policy script. Recipient containment checks every
recipient and requires a nonempty actual recipient set. All conditions and inherited bounds must hold.

Effects are `deny`, `require_review`, `permit`, `eligible_for_auto`, and `delegation_authority`.
`require_review` carries role/quorum/distinct-person/exclude-requester requirements but does not verify their
satisfaction. `eligible_for_auto` permits calling the learned evaluator, not executing the request. A
`delegation_authority` record is an issuance upper bound and never directly permits execution. An owner creating a
temporary delegation can atomically create an equally narrow and equally expiring authority parent; it need not
create a broader lasting permission. Delegations require expiry and an acyclic chain of authenticated parents.

V1 bounds: 64 KiB per encoded action/policy, 256 policies per evaluation, 128 items per collection, 2048 bytes per
text value, and 128 ASCII bytes per operation component. `from_json` checks the byte bound before parsing and
rejects unknown fields, duplicate JSON fields, unknown enum variants, unsupported versions and type coercion.
`validate` also checks values constructed directly in Rust, using a bounded serialization writer without allocating
the potentially oversized encoding. Callers must apply a byte limit to outer policy collections before decoding.

## Combination and unknowns

The runtime passes a fixed clock value and the existing baseline decision. With no new policy, existing valid
baseline results remain unchanged. Invalid actions, authority mismatches, missing account access, invalid policy
records, unauthenticated records, duplicate IDs or invalid delegation graphs cannot yield a permit.

The combination order is deterministic and independent of policy ordering:

1. Applicable prohibitions and baseline denial win.
2. An unresolved mandatory condition or selector holds execution.
3. Applicable review requirements accumulate without being removed by another permission or Auto.
4. Active explicit permissions are OR sources. Every delegation and all its parents remain AND bounds.
5. Learned Auto runs only for a matching eligible permission, subject to all preceding restrictions.
6. Otherwise retain the legacy baseline or require review.

A matching explicit authorizing scope supersedes a legacy baseline permit, including when its policy is paused,
revoked, expired, missing required facts, outside a condition, or out of budget. This prevents falling back to an
older grant after a scoped permission was narrowed or revoked. A separate active matching permission remains an
independent OR source. Unrelated accounts, connections and operations do not suppress their legacy baseline.
Removing a deny/review restriction requires an authenticated policy update; the evaluator does not accept a
revocation request merely because a wire record says `revoked`.

Before activating explicit account permissions, the runtime must migrate their old grants or mark those
accounts/operations as policy-managed with a review baseline. A removed agent/account/operation no longer matches
an edited scope, so an evaluator cannot infer that an unrelated legacy grant should have been retired. Scope edits
must never reactivate old authority through that fallback.

Mandatory conditions use three-valued conjunction: true AND unknown stays unknown, while a known false conjunct
proves that particular rule does not apply. An unknown repo label cannot make a corporate rule disappear.
Known facts count only when the runtime attests them, their source is not the agent, and their evidence is current
(`observed_at <= now < valid_until`). Device/resource-scoped rules hold if the needed optional identity is absent.

## Reservations and execution

`UsageSnapshot` includes committed and reserved usage at the matching policy revision. Missing usage or arithmetic
overflow cannot authorize an action. Monetary budgets additionally require explicit matching `Currency` on the
policy, verified action cost evidence, and usage snapshot; the evaluator performs no currency conversion.

The result supplies the exact request digest, determining policies, structured reasons, review requirements and
one deterministic `authorization_path` containing every parent bound and revision. Before execution the runtime
must reserve every affected limit atomically, recheck policy/role/membership leases and account access, verify any
required reviewer receipts, and bind a short-lived/replay-protected receipt to the original payload. A preflight
permit is not an execution receipt. Unknown execution outcomes retain reservations until reconciled.

Execution adapters must enforce the limits they advertise. Recognizing a Gradle build is not evidence that it
cannot execute project scripts, access inherited credentials or reach the network. Unsupported restrictions cannot
be enabled merely because command presentation has a reassuring summary.
