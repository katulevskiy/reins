# 06 · Trust modes, security and compliance

Reins serves two kinds of accountability:

- **Personal mode is sovereign.** You hold the keys. The server is a blind relay that can't read your credentials or
  your history, and can't decide for you. Security comes from cryptography and your phone.
- **Managed mode is accountable.** The organization holds the keys. A decision service it controls evaluates
  policy, and every decision is recorded where auditors can see it. Compliance comes from central control, provable
  logs and fast provisioning.

They're different on purpose. A company can't run on "only the employee's phone knows", and a person shouldn't have to
trust a company with their personal life.

## 1. The modes compared

| | **Personal** | **Managed** |
|---|---|---|
| Who's accountable | you | the organization |
| Credentials | on your approval device (encrypted, hardware-wrapped) | org executors and vaults, short-lived leases, BYOK/HSM |
| Policy evaluated | on your approval device | the org's decision service (Reins Cloud tenant, dedicated, or self-hosted) |
| Decisions signed by | your approval device's key | the deciding reviewer's enrolled device key, countersigned by the decision service |
| What the server sees | metadata only: connection ids, request digests, ticket states; bodies only in transit, then sealed | everything policy and retention allow: request metadata, effects, decisions and traces; bodies per retention setting |
| History | on your devices (synced sealed between your phones) | the org's audit log, append-only, hash-chained, exportable |
| Recovery | recovery code / master password, which only you have | SSO; IT can reissue devices; nothing personal to recover |
| Shared control | Circles (04), end-to-end | groups and roles (05 §6), centrally |
| Bypass | yours to use | only if org policy allows; always audited; time-boxed by policy |
| Model | on the phone | org choice: on-device, or a hosted model in the org's tenant |
| Compliance target | privacy (GDPR as a processor of minimal metadata) | SOC 2 Type II, ISO 27001-ready; HIPAA/BAA for Enterprise later |

## 2. Two profiles on one device

A person can have a Personal account and a Managed work profile at the same time.

- **Separate everything:**
  - separate keystores (distinct Keychain access groups on iOS; on Android, a separate Keystore alias set, or the
    Android work profile when it's MDM-managed);
  - separate databases, notification channels and Activity.
- **No data crosses between profiles.** The org can't see the Personal profile, its existence, or anything in it. The
  Personal profile can't use org connections.
- **Agents belong to one profile.** Pairing an agent asks which one. The desktop app holds one identity per profile,
  and git routing follows the remote: org-owned remotes go through the Managed identity.
- **The UI** shows a profile switcher in Settings. Cards and Activity rows carry a profile badge (Personal / Acme).
- **Leaving the company** removes the Managed profile remotely (SCIM, or MDM selective wipe). The Personal profile is
  untouched.

## 3. Keys and signing in Managed mode

| Key | Holder | Used for |
|---|---|---|
| **Org issuer key** | org KMS/HSM (Reins Cloud's KMS, or the customer's, through BYOK) | signing policy records and roster/role changes |
| **Decision service key** | the decision service, in KMS | countersigning decisions and audit checkpoints |
| **Device enrollment keys** | employee phones' secure hardware (Secure Enclave / StrongBox), with attestation (App Attest / Play Integrity) | signing a reviewer's decision |
| **Executor credentials** | executors; just-in-time from cloud IAM, GitHub App, vault | performing approved actions |
| **Audit chain key** | decision service; checkpoints optionally witnessed by an external timestamping or transparency service | tamper evidence |

- A decision is valid only if it carries:
  - the reviewer's device signature over (request digest, decision, policy snapshot hash, time);
  - the decision service's countersignature.

  Executors check both before acting.
- Admin actions (policy activation, retention, executor config) require two Security admins' device signatures.

## 4. Data handling (Managed)

- **Retention:** per data class, set by Security (defaults in parentheses):

  | Data class | Kept for |
  |---|---|
  | Decision metadata and traces | 1 year (Business), 7 years max (Enterprise) |
  | Request bodies (email text, commands, diffs) | none, 30 days or 1 year (default: 30 days) |
  | Safety-net artefacts (backups, trash) | as 01 §5 |

- **Encryption:** envelope encryption at rest, with data keys wrapped by the org's KMS key; TLS 1.3 in transit.
  Bodies can be held in an org-owned bucket instead (bring-your-own storage).
- **Residency:** EU, US and other regions for Reins Cloud; any region for self-hosted.
- **Employee privacy:**
  - members are told, in the app, what the org can see;
  - auditor access to bodies is logged and can require a second approver;
  - Personal profiles are out of scope by construction.
- **Legal:** a DPA, a subprocessor list, and SCCs for EU customers. Reins acts as a processor for Managed data.

## 5. Audit log

### 5.1 Events

Every state transition (03 §2), policy change, admin action, enrollment, connection change, credential lease and
safety-net undo is an event:

```json
{
  "seq": 1820331, "prev_hash": "b3…", "at": "2026-10-27T14:03:22.104Z",
  "tenant": "acme", "kind": "decision.approved",
  "request": {"digest": "sha256:…", "ticket": "rt_…", "agent": "claude-code@dana-mbp", "member": "dana@acme.com",
              "effects": [{"kind":"GitPush","repo":"acme/api","ref":"refs/heads/main","rewrites":false}],
              "level": "L2", "zones": ["Work","Production"]},
  "policy": {"snapshot": "sha256:…", "matched": ["prod-change-control@14"], "obligations": ["oncall-payments", "second-engineer"]},
  "decided_by": [{"member":"lee@acme.com","device":"…","sig":"…"},{"member":"kim@acme.com","device":"…","sig":"…"}],
  "countersig": "…"
}
```

### 5.2 Integrity

- Events form a hash chain per tenant.
- Every 10 minutes, a signed checkpoint (the chain head) is published to an external witness (e.g. a transparency
  log, or an RFC 3161 timestamp).
- Auditors can verify an export end to end with the open-source `reins-audit verify`.

### 5.3 Export

- Streaming to SIEMs (Splunk, Datadog, Elastic, Sentinel) and to S3/GCS with object lock.
- Scheduled evidence reports for auditors.

### 5.4 Personal mode

- Your history stays on your devices.
- **Export** produces a signed bundle you can verify. Useful if you ever need to prove what your agents did.

## 6. Devices in Managed mode

- **Enrollment:** SSO sign-in, then a device key generated in secure hardware, attested to the decision service.
- **MDM** (optional, recommended): Intune, Jamf and Workspace ONE can push managed app config (org id, SSO domain,
  required posture), enforce the work profile on Android, and selectively wipe.
- **Posture checks before a device can decide:**
  - a screen lock;
  - the minimum OS version;
  - no jailbreak or root (attestation);
  - the app version.

  A failing device can't approve, and its reviews route to others.

## 7. Mapping to SOC 2 (Trust Services Criteria)

| Criteria | How Reins supports the customer's controls |
|---|---|
| CC6.1–6.3 Logical access | SSO, SCIM, role-based admin, least-privilege managed connections, just-in-time credentials |
| CC6.6 Boundary protection | executors in the org's boundary (dedicated or self-hosted); agents never hold credentials |
| CC6.7 Data transmission | TLS 1.3; signed decisions; sealed packets |
| CC7.2 Monitoring | the audit stream to SIEM; dashboards; anomaly alerts (bursts, new external domains) |
| CC7.3–7.4 Incident response | org-wide kill switch, Lockdown scoped by team, credential revocation, evidence export |
| CC8.1 Change management | policy as code, two-person approval, simulation, staged rollout, signed versions |
| CC9.2 Vendor management | Reins' own SOC 2 report (roadmap), DPA, subprocessor list |
| A1 Availability | multi-region decision service; Instant reviews degrade to Deferred instead of failing |
| C1 Confidentiality | retention controls, BYOK, body minimization |

Reins (the company) also needs its own SOC 2 Type II for the Cloud offering; see 07 for timing.

## 8. Threat models

### 8.1 Both modes

| Threat | Mitigation |
|---|---|
| Prompt-injected agent tries something harmful | consequence engine judges effects, not text; agent text can only lower model confidence; L3 never Auto |
| Agent launders via many small steps | burst limits (01 §8); tasks' ceilings |
| Agent forges a ticket or replays an approval | approvals bound to request digest, nonce and expiry; agents see only their own tickets |
| Malicious repo rules file | stricter-only by default; looser needs acceptance; default branch only |
| Lost or stolen phone | biometric to decide; remote removal (roster in Personal, MDM/SCIM in Managed) |

### 8.2 Personal

| Threat | Mitigation |
|---|---|
| Compromised Reins server | blind: can't read credentials or history, can't forge answers (signed), can only drop or delay; a delay becomes Deferred or Expired, never approved |
| Malicious Circle member | scoped; owner notified; instant pause; signatures attribute every decision |
| Coercive guardianship | ward visibility; adults can leave (≤ 7 days); no content beyond scoped cards |

### 8.3 Managed

| Threat | Mitigation |
|---|---|
| Malicious or compromised admin | two-person admin actions; can't approve own changes; every admin action in the hash-chained audit; witness checkpoints |
| Compromised decision service | executors require reviewer device signatures plus the countersignature; HSM-held keys; self-hosting option; witness log detects rewrites |
| Insider approves own request | `exclude_requester` default; separation of duties |
| Credential exfiltration from executor | just-in-time short-lived credentials; executor in org boundary; per-action scopes |
| Employee privacy abuse by org | Managed sees only Managed; body access logged and optionally dual-approved |

## 9. Break-glass

- **Personal:** Bypass (for you), and recovery with the recovery code.
- **Managed:** an Org owner plus a Security admin can activate break-glass for a named scope and up to 4 h. It
  notifies all Security admins, records loudly in audit, and can't disable audit or retention.
