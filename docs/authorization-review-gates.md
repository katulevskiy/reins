# Mandatory connector and MCP review gates

This delivery extends the [mandatory runtime foundation](authorization-runtime.md). It adds owner review for
known connector operations outside Payments and the immediate session-end shortcut, and for `mcp_<server> / tool / call`. It does not activate positive
permissions, delegations, organization membership or remote reviewer proofs. Those remain separate adapters.

## Authorization and disclosure

A request is checked before provider dispatch. Supported review policies may permit the phone to collect a
minimal preview, but do not permit the agent to receive its content. Existing grants and retry passes are not
reserved or preselected when mandatory review applies. Auto, Bypass and Overnight cannot supply an owner review.
A local approval or notification approval carries its explicit owner source through the final adapter.

A connector write checks again immediately before its integration is invoked. Read results check before file
handoff, after awaited header discovery and before answering the agent. A policy installed while a fetch is
waiting also suppresses provider errors, which can contain content. MCP tools check after session initialization
and token renewal, before every direct or proxy attempt, before large output handoff and before releasing the
result. A new deny or unsupported reviewer requirement always stops those later boundaries.

The final checks do not roll back a provider action already in flight. A connector that returns success after a
new deny is recorded as successful, its result is withheld, and the original request is settled with a policy
error. This avoids pretending that withholding a result means the action did not happen.

| Operation | Mandatory deny | Owner review | External roles / multiple reviewers |
| --- | --- | --- | --- |
| Gmail send | Enforced | Enforced by the foundation | Held |
| Other registered connector reads and writes, except Payments and session-end | Enforced | Enforced | Held |
| MCP direct/proxy calls and retries | Enforced | Enforced | Held |
| Specialized Payments operations | Enforced | Activation rejected; imported review held | Held |
| Account / integration enumeration | Enforced | Activation rejected; imported review held | Held |
| Standalone upload release | Enforced | Activation rejected; imported review held | Held |
| Permission creation / immediate session-end shortcut | Enforced | Activation rejected; imported review held | Held |

The supported owner proof is exactly role `owner`, quorum one, with no requester exclusion or distinct-person
group. No ordinary approval impersonates Legal or Security. Organization-issued policies still require the
separate authority implementation before activation. A resource or recipient condition without authenticated
facts holds; parsing an agent's display label or command summary never supplies those facts.

## Captured execution evidence

When policy state exists, connector and MCP invocation/disclosure adapters record an encrypted `started` event
before invocation. Their final activity record carries the same attempt identifier and captured policy evidence:

- Exact request digest, request identifier and unique attempt identifier.
- Complete policy-state revision and SHA-256 commitment, and all policy identity/revision pairs.
- Determining policy identities, required reviews and explicit automatic/owner source.
- Verified fact keys and provenance/timestamps, without attribute values or provider content.
- The check time and `started`, `succeeded` or `unknown` phase.

The provider result never reconstructs a decision using newer policies. The trace is audit data, deliberately
separate from execution permission: deserializing it cannot authorize an action. `Engine::activity_authorization`
is a local Rust query for the captured evidence. Native permission screens can expose it through their own
stable facade once those types are ready.

A failed provider response does not prove the action failed. Once a connector write or MCP tool invocation
begins, its legacy grant use stays consumed. A final connector gate rejection before invocation can refund its
reservation. Policy-enabled attempts that fail after invocation record `unknown` without provider error content.
A device-local atomic barrier in the existing handled-request store is written before invocation. It prevents
two local app/extension processes from starting the same request, and blocks a resumed approval after a crash.
Safe MCP authentication/session retries share an explicit in-memory invocation object and still recheck policy
before every actual tool call. Post-invocation errors settle the original approval as unknown; a successful but
withheld response explicitly tells the agent that the provider reported success and the action must not be repeated.
This barrier uses the existing one-day handled retention, exceeding the server's ten-minute request lifetime; it
is not a synchronized receipt across primary-device handover or arbitrary device backup rollback. A deliberately
new request ID remains a new approval. Positive-permission adapters need their own account-synchronized immutable
request receipts and atomic budget accounting before they can authorize new automatic executions.

These records use the existing encrypted local audit and its retention limits. They are not an externally
witnessed transparency log, indefinite corporate archive or cryptographic proof of provider side effects. Signed
policy synchronization and conflict behavior remain those of the foundation. Old clients still need supported
client-version enforcement before any organization-wide guarantee is claimed.

## Remaining coverage

Gmail read/search review and positive permissions are being implemented independently against the same normalized
action/state contract. Captured trace coverage for their adapters must be integrated with their final execution
proofs rather than taking this narrowing baseline as new authority. Payments, standalone upload release,
account enumeration and permission creation need their own review-source and disclosure adapters before review
activation is enabled. Organization reviewers, quorum, separation of duties, provider idempotency and complete
archival retention remain explicit later work.
