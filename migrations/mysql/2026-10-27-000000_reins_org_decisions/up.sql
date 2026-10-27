-- Organizations, O2 (Decisions v2): the decision service, its keys, the org policy, reviews, executors, credential
-- leases and audit checkpoints. The audit chain itself is O1's `reins_org_events`, unchanged.

-- Per-tenant keys (08 R12): the org issuer key, the decision service key and the data key, sealed by the KMS.
CREATE TABLE reins_org_keys (
    org_uuid         VARCHAR(40) NOT NULL,
    key_id           VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    purpose          VARCHAR(255) NOT NULL,
    public_key       TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    sealed           TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    retired_at       BIGINT,
    PRIMARY KEY (org_uuid, key_id),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Org policy snapshots: drafts, the active one (signed by the issuer key) and retired ones.
CREATE TABLE reins_org_policies (
    org_uuid         VARCHAR(40) NOT NULL,
    revision         BIGINT NOT NULL,
    hash             CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    body             MEDIUMTEXT NOT NULL,
    envelope         MEDIUMTEXT,
    state            VARCHAR(255) NOT NULL,
    created_by       CHAR(36) NOT NULL,
    created_at       BIGINT NOT NULL,
    activated_at     BIGINT,
    PRIMARY KEY (org_uuid, revision),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- A member's own stricter policy, signed by their enrolled device.
CREATE TABLE reins_org_member_policies (
    org_uuid         VARCHAR(40) NOT NULL,
    user_uuid        CHAR(36) NOT NULL,
    body             MEDIUMTEXT NOT NULL,
    hash             CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    device_uuid      VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    sig              TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    updated_at       BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Managed requests and their decisions (`decision`: the countersigned record; `receipt`: the executor's).
CREATE TABLE reins_org_requests (
    uuid             VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    org_uuid         VARCHAR(40) NOT NULL,
    user_uuid        CHAR(36) NOT NULL,
    connection_uuid  VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    tool             VARCHAR(255) NOT NULL,
    digest           CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    kind             VARCHAR(255) NOT NULL,
    state            VARCHAR(255) NOT NULL,
    outcome          VARCHAR(255) NOT NULL,
    snapshot         CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    member_policy    CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    trace            MEDIUMTEXT NOT NULL,
    obligations      TEXT NOT NULL,
    decision         MEDIUMTEXT,
    created_at       BIGINT NOT NULL,
    expires_at       BIGINT NOT NULL,
    decided_at       BIGINT,
    valid_until      BIGINT,
    claimed_by       VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    claimed_at       BIGINT,
    finished_at      BIGINT,
    receipt          MEDIUMTEXT,
    PRIMARY KEY (uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Request bodies and results, sealed under the tenant's data key, kept per the retention setting.
CREATE TABLE reins_org_bodies (
    request_uuid     VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    part             VARCHAR(255) NOT NULL,
    org_uuid         VARCHAR(40) NOT NULL,
    key_id           VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    sealed           MEDIUMTEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    purge_at         BIGINT,
    PRIMARY KEY (request_uuid, part),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Reviewers' device-signed answers, to requests and to admin changes (`subject_uuid`).
CREATE TABLE reins_org_approvals (
    subject_uuid     VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    user_uuid        CHAR(36) NOT NULL,
    decision         VARCHAR(255) NOT NULL,
    org_uuid         VARCHAR(40) NOT NULL,
    device_uuid      VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    at               BIGINT NOT NULL,
    sig              TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    obligation       VARCHAR(255),
    PRIMARY KEY (subject_uuid, user_uuid, decision),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Registered executors: one tenant each, with their own key.
CREATE TABLE reins_org_executors (
    org_uuid         VARCHAR(40) NOT NULL,
    executor_id      VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    mode             VARCHAR(255) NOT NULL,
    public_key       TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    created_at       BIGINT NOT NULL,
    last_seen        BIGINT,
    revoked_at       BIGINT,
    PRIMARY KEY (org_uuid, executor_id),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Short-lived managed credentials: what was asked, its state, and the credential sealed to the requester.
CREATE TABLE reins_org_leases (
    uuid             VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    org_uuid         VARCHAR(40) NOT NULL,
    user_uuid        CHAR(36) NOT NULL,
    connection_uuid  VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    request_uuid     VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    repo             VARCHAR(255) NOT NULL,
    access           VARCHAR(255) NOT NULL,
    box_key          TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    state            VARCHAR(255) NOT NULL,
    sealed           TEXT,
    created_at       BIGINT NOT NULL,
    expires_at       BIGINT NOT NULL,
    issued_at        BIGINT,
    revoked_at       BIGINT,
    revoke_done_at   BIGINT,
    PRIMARY KEY (uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Signed audit checkpoints (06 §5.2), every 10 minutes when the chain grew.
CREATE TABLE reins_org_checkpoints (
    org_uuid         VARCHAR(40) NOT NULL,
    seq              BIGINT NOT NULL,
    hash             CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    at               BIGINT NOT NULL,
    envelope         TEXT NOT NULL,
    PRIMARY KEY (org_uuid, seq),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_org_requests_state ON reins_org_requests (org_uuid, state);
CREATE INDEX idx_reins_org_requests_conn ON reins_org_requests (connection_uuid);
CREATE INDEX idx_reins_org_leases_user ON reins_org_leases (org_uuid, user_uuid);
CREATE INDEX idx_reins_org_bodies_purge ON reins_org_bodies (purge_at);

-- Request bodies are kept 0, 30 or 365 days (06 §4); 30 by default.
ALTER TABLE reins_orgs ADD COLUMN body_retention_days INTEGER NOT NULL DEFAULT 30;
-- What verifying the platform attestation found (O2): unverified, verified, failed or none.
ALTER TABLE reins_org_devices ADD COLUMN attestation_state VARCHAR(16) NOT NULL DEFAULT 'unverified';
ALTER TABLE reins_org_devices ADD COLUMN attestation_detail TEXT;
