# Policy support at phone handover

An account with retained authorization protections cannot transfer its primary phone role to an app that might ignore
them. Active, paused, revoked and unresolved policy history, connection revocations, execution receipts and usage
floors all require a policy-capable recipient, even if no policy document was installed. This
applies before constructing or sending the bundle containing the owner seed and provider secrets.

The viewer sends the existing signed handover request plus an optional `policy_enforcement` proof. The additional
Ed25519 signature has its own canonical context and binds the account, roster epoch, viewer device, request nonce,
signed time and sorted supported authorization versions. Both the server and primary verify it against the viewer's
key in the roster. A signature for another request, account, device or capability set cannot be substituted.

| Account state | Recipient evidence | Result |
| --- | --- | --- |
| No retained authorization protections | Valid legacy request | Existing handover behavior |
| Retained authorization protections | Missing or unsupported policy proof | Transfer held with phone/server update guidance |
| Retained authorization protections | Valid current-version proof | Existing verified history and handover checks continue |
| Any state | Present but invalid policy proof | Request refused |

The ordinary handover signature and roster encoding do not change. Older servers can still handle transfers of
unmanaged accounts; a server that drops the new optional proof cannot transfer a managed account. No roster member,
native view or generated-binding field is added.

Policy administration and positive Gmail execution use a shared account-specific process lease. Handover holds an
exclusive lease and the local roster-change guard while checking retained protection and transferring keys. Writers refresh primary
authority after acquiring their lease. This prevents a second process from installing policy between the transfer's
capability check and key release. Shared administration leases also let a new Deny or revocation become effective
while an execution is awaiting provider authentication or a response.

This is authenticated software capability advertisement, not hardware attestation or protection against a malicious
paired phone that lies about its implementation. Owner-key recovery and accounts operated through older deciding
apps remain outside this handover guarantee. Enterprise deployment still needs a server-enforced supported-client
baseline and organization authority before claiming enforcement across arbitrary old clients or owner recovery.
