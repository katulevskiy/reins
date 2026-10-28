-- Family connections (Decisions v2 · P7): the accounts a guardian connected for a family member. The refresh tokens
-- live only in the family's executor; the server keeps which provider is connected, and the jobs it queues for the
-- executor. A finishing job holds the provider's one-time code sealed at rest (useless without the executor's PKCE
-- verifier and client secret), and only until the executor claims it.
CREATE TABLE reins_family_connections (
    org_uuid     VARCHAR(40) NOT NULL,
    user_uuid    CHAR(36) NOT NULL,
    provider     VARCHAR(16) NOT NULL,
    account      VARCHAR(255),
    state        VARCHAR(16) NOT NULL,
    connected_at BIGINT,
    updated_at   BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid, provider),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_family_connect_jobs (
    id           VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL PRIMARY KEY,
    org_uuid     VARCHAR(40) NOT NULL,
    user_uuid    CHAR(36) NOT NULL,
    provider     VARCHAR(16) NOT NULL,
    phase        VARCHAR(16) NOT NULL,
    oauth_state  VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin UNIQUE,
    code_key_id  VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    sealed_code  MEDIUMTEXT,
    auth_url     MEDIUMTEXT,
    state        VARCHAR(16) NOT NULL,
    claimed_by   VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    requested_by CHAR(36) NOT NULL,
    created_at   BIGINT NOT NULL,
    expires_at   BIGINT NOT NULL,
    done_at      BIGINT,
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_family_connect_jobs_org ON reins_family_connect_jobs (org_uuid, state);
