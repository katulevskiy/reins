-- Organizations, O4 (Decisions v2): policy as code, simulation and staged rollout.

-- Each decision's body-free input (`reins_policy::org::SimInput`: effects, zones, levels; decision metadata, never the
-- request body) for simulation, and the rollout step it was decided under (effective-version pinning, 05 §7.3).
ALTER TABLE reins_org_requests ADD COLUMN analysis TEXT;
ALTER TABLE reins_org_requests ADD COLUMN rollout_id TEXT;
ALTER TABLE reins_org_requests ADD COLUMN rollout_seq INTEGER;

-- Where a draft came from (`pr#12@<sha>`, `main@<sha>`, `console`, `template:<name>`) and its rollout plan.
ALTER TABLE reins_org_policies ADD COLUMN source_ref TEXT;
ALTER TABLE reins_org_policies ADD COLUMN rollout_plan TEXT;

-- The org's policy repo (`acme/reins-policy`, lowercase), bound by a two-person admin change.
CREATE TABLE reins_org_policy_repo (
    org_uuid         TEXT NOT NULL PRIMARY KEY REFERENCES organizations (uuid) ON DELETE CASCADE,
    repo             TEXT NOT NULL,
    default_branch   TEXT NOT NULL,
    bound_by         TEXT NOT NULL,
    bound_at         BIGINT NOT NULL
);
CREATE UNIQUE INDEX reins_org_policy_repo_repo ON reins_org_policy_repo (repo);

-- Simulation reports, by their hash: the evidence an activation names.
CREATE TABLE reins_org_simulations (
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    report_hash      TEXT NOT NULL,
    revision         BIGINT NOT NULL,
    draft_hash       TEXT NOT NULL,
    base_hash        TEXT NOT NULL,
    source_ref       TEXT,
    report           TEXT NOT NULL,
    ran_at           BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, report_hash)
);
CREATE INDEX reins_org_simulations_draft ON reins_org_simulations (org_uuid, draft_hash);

-- Staged rollouts: every step pre-signed by the issuer at activation (`envelopes`, seq 1..n+1), published in order.
CREATE TABLE reins_org_rollouts (
    rollout_id       TEXT NOT NULL PRIMARY KEY,
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    revision         BIGINT NOT NULL,
    epoch            BIGINT NOT NULL,
    state            TEXT NOT NULL,
    stable           TEXT NOT NULL,
    candidate        TEXT NOT NULL,
    plan             TEXT NOT NULL,
    envelopes        TEXT NOT NULL,
    published_seq    INTEGER NOT NULL,
    stage_started_at BIGINT NOT NULL,
    reason           TEXT,
    created_at       BIGINT NOT NULL,
    updated_at       BIGINT NOT NULL
);
CREATE INDEX reins_org_rollouts_org ON reins_org_rollouts (org_uuid, state);

-- GitHub webhook deliveries seen in the last day (each is acted on once).
CREATE TABLE reins_org_webhook_deliveries (
    delivery_id      TEXT NOT NULL PRIMARY KEY,
    received_at      BIGINT NOT NULL
);
