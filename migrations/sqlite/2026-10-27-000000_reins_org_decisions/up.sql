-- Organizations, O2 (Decisions v2): the decision service, its keys, the org policy, reviews, executors, credential
-- leases and audit checkpoints. The audit chain itself is O1's `reins_org_events`, unchanged.

-- Per-tenant keys (08 R12): the org issuer key, the decision service key and the data key, sealed by the KMS.
CREATE TABLE reins_org_keys (
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    key_id           TEXT NOT NULL,
    purpose          TEXT NOT NULL,
    public_key       TEXT NOT NULL,
    sealed           TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    retired_at       BIGINT,
    PRIMARY KEY (org_uuid, key_id)
);

-- Org policy snapshots: drafts, the active one (signed by the issuer key) and retired ones.
CREATE TABLE reins_org_policies (
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    revision         BIGINT NOT NULL,
    hash             TEXT NOT NULL,
    body             TEXT NOT NULL,
    envelope         TEXT,
    state            TEXT NOT NULL,
    created_by       TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    activated_at     BIGINT,
    PRIMARY KEY (org_uuid, revision)
);

-- A member's own stricter policy, signed by their enrolled device.
CREATE TABLE reins_org_member_policies (
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid        TEXT NOT NULL,
    body             TEXT NOT NULL,
    hash             TEXT NOT NULL,
    device_uuid      TEXT NOT NULL,
    sig              TEXT NOT NULL,
    updated_at       BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid)
);

-- Managed requests and their decisions (`decision`: the countersigned record; `receipt`: the executor's).
CREATE TABLE reins_org_requests (
    uuid             TEXT NOT NULL,
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid        TEXT NOT NULL,
    connection_uuid  TEXT NOT NULL,
    tool             TEXT NOT NULL,
    digest           TEXT NOT NULL,
    kind             TEXT NOT NULL,
    state            TEXT NOT NULL,
    outcome          TEXT NOT NULL,
    snapshot         TEXT NOT NULL,
    member_policy    TEXT,
    trace            TEXT NOT NULL,
    obligations      TEXT NOT NULL,
    decision         TEXT,
    created_at       BIGINT NOT NULL,
    expires_at       BIGINT NOT NULL,
    decided_at       BIGINT,
    valid_until      BIGINT,
    claimed_by       TEXT,
    claimed_at       BIGINT,
    finished_at      BIGINT,
    receipt          TEXT,
    PRIMARY KEY (uuid)
);

-- Request bodies and results, sealed under the tenant's data key, kept per the retention setting.
CREATE TABLE reins_org_bodies (
    request_uuid     TEXT NOT NULL,
    part             TEXT NOT NULL,
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    key_id           TEXT NOT NULL,
    sealed           TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    purge_at         BIGINT,
    PRIMARY KEY (request_uuid, part)
);

-- Reviewers' device-signed answers, to requests and to admin changes (`subject_uuid`).
CREATE TABLE reins_org_approvals (
    subject_uuid     TEXT NOT NULL,
    user_uuid        TEXT NOT NULL,
    decision         TEXT NOT NULL,
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    device_uuid      TEXT NOT NULL,
    at               BIGINT NOT NULL,
    sig              TEXT NOT NULL,
    obligation       TEXT,
    PRIMARY KEY (subject_uuid, user_uuid, decision)
);

-- Registered executors: one tenant each, with their own key.
CREATE TABLE reins_org_executors (
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    executor_id      TEXT NOT NULL,
    mode             TEXT NOT NULL,
    public_key       TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    last_seen        BIGINT,
    revoked_at       BIGINT,
    PRIMARY KEY (org_uuid, executor_id)
);

-- Short-lived managed credentials: what was asked, its state, and the credential sealed to the requester.
CREATE TABLE reins_org_leases (
    uuid             TEXT NOT NULL,
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid        TEXT NOT NULL,
    connection_uuid  TEXT NOT NULL,
    request_uuid     TEXT NOT NULL,
    repo             TEXT NOT NULL,
    access           TEXT NOT NULL,
    box_key          TEXT NOT NULL,
    state            TEXT NOT NULL,
    sealed           TEXT,
    created_at       BIGINT NOT NULL,
    expires_at       BIGINT NOT NULL,
    issued_at        BIGINT,
    revoked_at       BIGINT,
    revoke_done_at   BIGINT,
    PRIMARY KEY (uuid)
);

-- Signed audit checkpoints (06 §5.2), every 10 minutes when the chain grew.
CREATE TABLE reins_org_checkpoints (
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    seq              BIGINT NOT NULL,
    hash             TEXT NOT NULL,
    at               BIGINT NOT NULL,
    envelope         TEXT NOT NULL,
    PRIMARY KEY (org_uuid, seq)
);

CREATE INDEX idx_reins_org_requests_state ON reins_org_requests (org_uuid, state);
CREATE INDEX idx_reins_org_requests_conn ON reins_org_requests (connection_uuid);
CREATE INDEX idx_reins_org_leases_user ON reins_org_leases (org_uuid, user_uuid);
CREATE INDEX idx_reins_org_bodies_purge ON reins_org_bodies (purge_at);

-- Request bodies are kept 0, 30 or 365 days (06 §4); 30 by default.
ALTER TABLE reins_orgs ADD COLUMN body_retention_days INTEGER NOT NULL DEFAULT 30;
-- What verifying the platform attestation found (O2): unverified, verified, failed or none.
ALTER TABLE reins_org_devices ADD COLUMN attestation_state TEXT NOT NULL DEFAULT 'unverified';
ALTER TABLE reins_org_devices ADD COLUMN attestation_detail TEXT;
