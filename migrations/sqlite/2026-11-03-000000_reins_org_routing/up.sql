-- Organizations, O3 (Decisions v2): approval routing. Who a member's manager is, their linked GitHub, Slack and Teams
-- identities; group backups and windows; on-call sources, approval channels and the org's read-only GitHub App (their
-- secrets sealed under the tenant's data key); each obligation's escalation steps; web passkeys, one-time web approval
-- tokens (SHA-256 only) and the channel messages posted for a review.
CREATE TABLE reins_org_people (
    org_uuid            TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid           TEXT NOT NULL,
    manager_user        TEXT,
    manager_source      TEXT,
    github_login        TEXT,
    github_verified_via TEXT,
    slack_user          TEXT,
    teams_aad           TEXT,
    teams_conversation  TEXT,
    updated_at          BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid)
);

CREATE TABLE reins_org_group_settings (
    org_uuid     TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    group_uuid   TEXT NOT NULL,
    backup_group TEXT,
    window_secs  BIGINT,
    PRIMARY KEY (org_uuid, group_uuid)
);

CREATE TABLE reins_org_oncall (
    org_uuid    TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    alias       TEXT NOT NULL,
    provider    TEXT NOT NULL,
    schedule_id TEXT NOT NULL,
    key_id      TEXT,
    sealed      TEXT,
    PRIMARY KEY (org_uuid, alias)
);

CREATE TABLE reins_org_channels (
    org_uuid     TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    key_id       TEXT,
    sealed       TEXT,
    enabled      BOOLEAN NOT NULL,
    installed_at BIGINT,
    PRIMARY KEY (org_uuid, kind)
);

CREATE TABLE reins_org_github_reader (
    org_uuid        TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    app_id          BIGINT NOT NULL,
    installation_id BIGINT NOT NULL,
    key_id          TEXT,
    sealed          TEXT,
    set_at          BIGINT NOT NULL,
    PRIMARY KEY (org_uuid)
);

CREATE TABLE reins_org_steps (
    request_uuid TEXT NOT NULL REFERENCES reins_org_requests (uuid) ON DELETE CASCADE,
    obligation   TEXT NOT NULL,
    step         INTEGER NOT NULL,
    reviewers    TEXT NOT NULL,
    resolved     TEXT NOT NULL,
    source       TEXT NOT NULL,
    started_at   BIGINT NOT NULL,
    deadline     BIGINT NOT NULL,
    PRIMARY KEY (request_uuid, obligation, step)
);

CREATE TABLE reins_org_web_passkeys (
    credential_id    TEXT NOT NULL,
    org_uuid         TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid        TEXT NOT NULL,
    passkey          TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    confirmed_at     BIGINT,
    confirmed_device TEXT,
    revoked_at       BIGINT,
    PRIMARY KEY (credential_id)
);

CREATE INDEX idx_reins_org_web_passkeys_user ON reins_org_web_passkeys (org_uuid, user_uuid);

CREATE TABLE reins_org_web_tokens (
    token_hash   TEXT NOT NULL,
    org_uuid     TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    request_uuid TEXT NOT NULL,
    user_uuid    TEXT NOT NULL,
    expires_at   BIGINT NOT NULL,
    used_at      BIGINT,
    PRIMARY KEY (token_hash)
);

CREATE TABLE reins_org_posted (
    request_uuid TEXT NOT NULL REFERENCES reins_org_requests (uuid) ON DELETE CASCADE,
    user_uuid    TEXT NOT NULL,
    channel      TEXT NOT NULL,
    external_ref TEXT NOT NULL,
    posted_at    BIGINT NOT NULL,
    PRIMARY KEY (request_uuid, user_uuid, channel)
);

ALTER TABLE reins_org_requests ADD COLUMN resolved TEXT;
