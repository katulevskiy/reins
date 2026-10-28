# The org audit log: verify, report, stream

Every org (Managed mode, `REINS_ORGS_ENABLED`) has an append-only, hash-chained audit log: decisions, reviews,
executions, credential leases, policy and executor changes, admin actions, enrollments and member changes
(`docs/design/decisions-v2/06-trust-and-compliance.md` §5). This page is for auditors and security teams: how to
check it without trusting Reins, turn it into evidence, and get it into your own systems.

## Verify an export

In the console, **Audit**: **Export** downloads the log as JSON Lines, and **keys.json** the org's public keys. Then,
anywhere:

```sh
reins-audit verify export.jsonl --keys keys.json
# OK: 1820 events (1..1820) of 4022ebb8-…, 182 checkpoints, head 9f…
```

`reins-audit` is open source (Apache-2.0, `crates/reins-audit`), works offline and needs only the org's decision
service key. It proves:

- **nothing was edited:** every event's hash is recomputed;
- **nothing was removed, inserted or reordered:** sequence numbers run without a gap and each event names its
  predecessor's hash;
- **the tail wasn't cut:** the signed head covers the last event;
- **every checkpoint is on this chain** and signed by the decision service key;
- with `--anchor <file>`: **the chain wasn't rewritten or rolled back** since that checkpoint or head (kept from an
  earlier export, or a timestamped checkpoint below). Someone holding the decision service key could re-sign a
  rewritten chain; they can't make it agree with a checkpoint you kept.

Options: `--key <id>=<base64>` instead of `--keys` (repeatable), `-` to read the export from stdin, `--json` for a
machine-readable result. Exit codes: 0 verified, 1 not verified (stdout empty, the reason on stderr), 2 usage.

Get it from a release (`reins-audit-<version>-<target>.tar.gz`, Linux x86_64 and aarch64, listed in `SHA256SUMS`),
from the executor image (`docker run --rm -v "$PWD:/w" -w /w --entrypoint /reins-audit
ghcr.io/katulevskiy/reins-executor:latest verify export.jsonl --keys keys.json`), or build it on any platform:
`cargo install --locked --git https://github.com/katulevskiy/reins reins-audit`.

Keep each export's last line (its signed head) or the `keys.json` and a checkpoint: the next export is checked against
it with `--anchor`.

## Evidence reports (SOC 2, ISO 27001)

```sh
reins-audit report --framework soc2 export.jsonl --keys keys.json > soc2.md
reins-audit report --framework iso27001 export.jsonl --keys keys.json --from 2026-07-01 --to 2026-09-30 --format json
```

A report is made only from an export that verifies, and is deterministic: the same export and version give the same
bytes, so an auditor can regenerate what the org hands over. It names the export it rests on (range, head hash), then,
per control, the events that evidence it (counts by kind and sample sequence numbers to pull) and the exceptions
(an admin change applied by a sole holder, a last owner kept, request bodies read).

- **SOC 2** (Trust Services Criteria): CC6.1, CC6.2, CC6.3, CC6.6, CC6.7, CC7.2, CC7.3, CC8.1, A1.2, C1.2.
- **ISO/IEC 27001:2022** (Annex A): A.5.15, A.5.16, A.5.18, A.5.26, A.8.2, A.8.5, A.8.10, A.8.15, A.8.16, A.8.32.

The console offers the same reports (Audit, Evidence); it verifies the export with the org's key before building one.
The reports support the org's own controls; Reins' own SOC 2 report is separate.

## Timestamps (RFC 3161)

With `REINS_ORG_AUDIT_TSA_URL` set on the server (an RFC 3161 time-stamping authority, e.g.
`http://timestamp.digicert.com`), every checkpoint is timestamped soon after it is signed. The console's Audit page
links the last checkpoint and its timestamp; any checkpoint is at `…/audit/checkpoints/<seq>.json` and `<seq>.tsr`.
Check one with OpenSSL and the authority's certificate chain:

```sh
openssl ts -verify -data 1820.json -in 1820.tsr -CAfile tsa-chain.pem
```

That proves the checkpoint existed, unchanged, at the authority's time: a later rewrite of the whole chain can't agree
with it. Use the checkpoint as `--anchor` for exports made after it.

## Streaming

Audit, **Streaming**: set up a destination (its secrets are sealed and never shown again), then **Turn on**; two
Security admins confirm on their phones. A destination that is on can't be changed: turning it off takes two admins
as well, so nobody alone can redirect or stop the stream.

| Destination | What is sent |
|---|---|
| Splunk | HTTP Event Collector, `sourcetype` `reins:audit`, one event per audit event |
| Datadog | Logs intake for the chosen site, `ddsource` `reins`, the event under `reins` |
| Elastic | the bulk API, `create` with `_id` `<org>-<seq>` (a resend is a duplicate Elastic drops) |
| Microsoft Sentinel | the Logs Ingestion API (an Entra app with *Monitoring Metrics Publisher* on the rule); stream columns `TimeGenerated`, `Org`, `Seq`, `Kind`, `Actor`, `Subject`, `Detail`, `PrevHash`, `Hash` |
| S3 or GCS | one object per segment between two checkpoints, `<prefix><org>/<after>-<to>.jsonl`, with S3 Object Lock in COMPLIANCE mode for `lock_days` (GCS: HMAC keys, endpoint `https://storage.googleapis.com`, a locked bucket retention policy and `lock_days` 0) |

Delivery is at least once and in order; each record carries its `seq` and `hash`, so a SIEM can drop repeats and
check the chain itself. Each object-storage segment is a whole export: `reins-audit verify <object> --keys keys.json
--anchor <the previous segment's head>`. A failing destination shows on the page and in the log
(`audit.sink_failing`, `audit.sink_recovered`); deliveries resume where they stopped. Destinations must be HTTPS on
public addresses.

## Retention and pruning

The log is kept by plan: Team 30 days, Business and Family 1 year, Enterprise 7 years. Once a day, events older than that are
deleted down to the newest checkpoint signed before the cut-off, which stays: exports then start from it and still
verify (`audit.pruned` records the anchor). A chain that doesn't verify as stored is never pruned. Stream to object
storage with Object Lock to keep more.

## Bring your own KMS

Every org's issuer, decision service and data keys are sealed by a master key. By default it is a file on the server
(`REINS_ORG_KMS_KEY_FILE`). To hold it under your own KMS, set `REINS_ORG_KMS_PROVIDER` (`aws`, `gcp`, `azure`) and
`REINS_ORG_KMS_KEY_ID`:

| Provider | `REINS_ORG_KMS_KEY_ID` | Credentials |
|---|---|---|
| `aws` | a KMS key or alias ARN | `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`, else the instance role (IMDSv2) |
| `gcp` | `projects/…/locations/…/keyRings/…/cryptoKeys/…` | `GOOGLE_APPLICATION_CREDENTIALS`, else the metadata server |
| `azure` | `https://<vault>.vault.azure.net/keys/<name>/<version>` | `AZURE_TENANT_ID`/`AZURE_CLIENT_ID`/`AZURE_CLIENT_SECRET`, else managed identity |

On first launch the server makes a master key, wraps it with your key (checking it can unwrap it) and writes only the
wrapped form to the key file; every launch unwraps it once. If your KMS can't unwrap it, the server doesn't start:
there is no fallback to a local key. Revoking the key stops the next launch from opening any org key. The
unwrapped master and the signing keys live in the server's memory while it runs; HSM-held signing keys are later.
