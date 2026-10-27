-- Organizations, O3 (Decisions v2): approval routing. Who a member's manager is, their linked GitHub, Slack and Teams
-- identities; group backups and windows; on-call sources and approval channels (their secrets sealed under the
-- tenant's data key); each obligation's escalation steps; web passkeys, one-time web approval tokens (SHA-256 only)
-- and the channel messages posted for a review. The org's GitHub reader is `2026-11-01-000000_reins_org_github_reader`,
-- shared with O4.
CREATE TABLE reins_org_people (
    org_uuid            VARCHAR(40) NOT NULL,
    user_uuid           CHAR(36) NOT NULL,
    manager_user        CHAR(36),
    manager_source      VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    github_login        VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    github_verified_via VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    slack_user          VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    teams_aad           VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    teams_conversation  TEXT,
    updated_at          BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_org_group_settings (
    org_uuid     VARCHAR(40) NOT NULL,
    group_uuid   CHAR(36) NOT NULL,
    backup_group CHAR(36),
    window_secs  BIGINT,
    PRIMARY KEY (org_uuid, group_uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_org_oncall (
    org_uuid    VARCHAR(40) NOT NULL,
    alias       VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    provider    VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    schedule_id VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    key_id      VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    sealed      TEXT,
    PRIMARY KEY (org_uuid, alias),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_org_channels (
    org_uuid     VARCHAR(40) NOT NULL,
    kind         VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    key_id       VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    sealed       TEXT,
    enabled      BOOLEAN NOT NULL,
    installed_at BIGINT,
    PRIMARY KEY (org_uuid, kind),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_org_steps (
    request_uuid VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    obligation   VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    step         INTEGER NOT NULL,
    reviewers    TEXT NOT NULL,
    resolved     TEXT NOT NULL,
    source       VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    started_at   BIGINT NOT NULL,
    deadline     BIGINT NOT NULL,
    PRIMARY KEY (request_uuid, obligation, step),
    FOREIGN KEY (request_uuid) REFERENCES reins_org_requests (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_org_web_passkeys (
    credential_id    VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    org_uuid         VARCHAR(40) NOT NULL,
    user_uuid        CHAR(36) NOT NULL,
    passkey          TEXT NOT NULL,
    created_at       BIGINT NOT NULL,
    confirmed_at     BIGINT,
    confirmed_device VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    revoked_at       BIGINT,
    PRIMARY KEY (credential_id),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_org_web_passkeys_user ON reins_org_web_passkeys (org_uuid, user_uuid);

CREATE TABLE reins_org_web_tokens (
    token_hash   CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    org_uuid     VARCHAR(40) NOT NULL,
    request_uuid VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    user_uuid    CHAR(36) NOT NULL,
    expires_at   BIGINT NOT NULL,
    used_at      BIGINT,
    PRIMARY KEY (token_hash),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_org_posted (
    request_uuid VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    user_uuid    CHAR(36) NOT NULL,
    channel      VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    external_ref TEXT NOT NULL,
    posted_at    BIGINT NOT NULL,
    PRIMARY KEY (request_uuid, user_uuid, channel),
    FOREIGN KEY (request_uuid) REFERENCES reins_org_requests (uuid) ON DELETE CASCADE
);

ALTER TABLE reins_org_requests ADD COLUMN resolved MEDIUMTEXT;
