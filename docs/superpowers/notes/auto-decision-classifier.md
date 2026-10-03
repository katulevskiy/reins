# Where a trained auto-decision classifier plugs in

Goal (from the product owner): a small, trainable model, in the spirit of a JEPA-style representation, learns from the user's own
past choices which requests to **auto-allow**, which to **escalate** to the user, and which to **auto-deny**.

## The seam already exists
Every request already passes one decision point on the phone, `Engine::process_request_unguarded` in
`crates/reins-core/src/handler.rs`, right after grants are evaluated:

1. `reins_policy::evaluate_read` / `evaluate_send` say *Allowed by grant*, or *needs the user*.
2. Not covered → the request is parked and the user is notified.

The classifier sits **between 1 and 2** and may only return `Allow | Escalate | Deny`:

* **Deny** answers the AI immediately (audit entry `denied`, detail `auto: <reason>`).
* **Escalate** is today's behaviour: park and notify.
* **Allow** is treated like a matching grant with `source = classifier` in the audit log, never like a stored grant, so it
  cannot be revoked by accident or accumulate silently.

## Constraints that should hold whatever the model is
* It only ever runs on the phone. Nothing about the user's mail or decisions goes to the server or to a model API.
* Fail closed: any error, low confidence or unseen feature → **Escalate**. It can never widen what a *send* is allowed
  to do without a matching explicit user setting (sending stays opt-in per recipient class).
* Hard rules stay above it: grants that were revoked, expired or exhausted are not resurrected, and the all-mail grant
  limits (time-boxed, at most 30 days, reads only) are enforced in `reins-policy` regardless of the model.
* Explainable: each automatic decision stores which features drove it, shown in **Activity**, with one tap to
  "always ask for this kind of request" (a negative training example).

## Training data
The audit log (`crates/reins-core/src/store/audit.rs`) already records every decision with connection, action,
outcome and grant. Add per-decision features at write time (sender domain class, label set, hour of day, query kind,
count of messages, recipient class, connection, whether the user ticked everything or a subset) so the model can be
trained on-device from real choices. Approve/deny/"select all"/"allow all mail for N" are all labelled examples.

## Suggested shape
* `crates/reins-classifier`: pure Rust, `no_std`-friendly feature extraction, versioned model file, deterministic
  inference (unit-testable with golden vectors), called from the engine behind a trait `Advisor`.
* Model storage: encrypted like the rest of the store; exported/imported only by the user.
* UI: Settings → "Learning" with an on/off switch, a "what has it learned" list, and a reset.
