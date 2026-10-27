-- Organizations, O4 (Decisions v2): policy as code, simulation and staged rollout.

-- Each decision's body-free input (`reins_policy::org::SimInput`: effects, zones, levels; decision metadata, never the
-- request body) for simulation, and the rollout step it was decided under (effective-version pinning, 05 §7.3).
ALTER TABLE reins_org_requests ADD COLUMN analysis MEDIUMTEXT;
ALTER TABLE reins_org_requests ADD COLUMN rollout_id VARCHAR(40) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin;
ALTER TABLE reins_org_requests ADD COLUMN rollout_seq INT;

-- Where a draft came from (`pr#12@<sha>`, `main@<sha>`, `console`, `template:<name>`) and its rollout plan.
ALTER TABLE reins_org_policies ADD COLUMN source_ref TEXT;
ALTER TABLE reins_org_policies ADD COLUMN rollout_plan TEXT;

-- The org's policy repo (`acme/reins-policy`, lowercase), bound by a two-person admin change.
CREATE TABLE reins_org_policy_repo (
    org_uuid         VARCHAR(40) NOT NULL PRIMARY KEY,
    repo             VARCHAR(160) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    default_branch   VARCHAR(255) NOT NULL,
    bound_by         CHAR(36) NOT NULL,
    bound_at         BIGINT NOT NULL,
    UNIQUE KEY reins_org_policy_repo_repo (repo),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Simulation reports, by their hash: the evidence an activation names.
CREATE TABLE reins_org_simulations (
    org_uuid         VARCHAR(40) NOT NULL,
    report_hash      CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    revision         BIGINT NOT NULL,
    draft_hash       CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    base_hash        CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    source_ref       TEXT,
    report           MEDIUMTEXT NOT NULL,
    ran_at           BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, report_hash),
    KEY reins_org_simulations_draft (org_uuid, draft_hash),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Staged rollouts: every step pre-signed by the issuer at activation (`envelopes`, seq 1..n+1), published in order.
CREATE TABLE reins_org_rollouts (
    rollout_id       VARCHAR(40) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL PRIMARY KEY,
    org_uuid         VARCHAR(40) NOT NULL,
    revision         BIGINT NOT NULL,
    epoch            BIGINT NOT NULL,
    state            VARCHAR(32) NOT NULL,
    stable           CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    candidate        CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    plan             MEDIUMTEXT NOT NULL,
    envelopes        MEDIUMTEXT NOT NULL,
    published_seq    INT NOT NULL,
    stage_started_at BIGINT NOT NULL,
    reason           TEXT,
    created_at       BIGINT NOT NULL,
    updated_at       BIGINT NOT NULL,
    KEY reins_org_rollouts_org (org_uuid, state),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- GitHub webhook deliveries seen in the last day (each is acted on once).
CREATE TABLE reins_org_webhook_deliveries (
    delivery_id      VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL PRIMARY KEY,
    received_at      BIGINT NOT NULL
);
