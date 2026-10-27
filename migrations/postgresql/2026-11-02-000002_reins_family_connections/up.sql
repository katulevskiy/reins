-- Family connections (Decisions v2 · P7): the accounts a guardian connected for a family member. The refresh tokens
-- live only in the family's executor; the server keeps which provider is connected, and the jobs it queues for the
-- executor. A finishing job holds the provider's one-time code sealed at rest (useless without the executor's PKCE
-- verifier and client secret), and only until the executor claims it.
CREATE TABLE reins_family_connections (
    org_uuid     VARCHAR(40) NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid    CHAR(36) NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    provider     VARCHAR(16) NOT NULL,
    account      VARCHAR(255),
    state        VARCHAR(16) NOT NULL,
    connected_at BIGINT,
    updated_at   BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid, provider)
);

CREATE TABLE reins_family_connect_jobs (
    id           VARCHAR(32) NOT NULL PRIMARY KEY,
    org_uuid     VARCHAR(40) NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid    CHAR(36) NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    provider     VARCHAR(16) NOT NULL,
    phase        VARCHAR(16) NOT NULL,
    oauth_state  VARCHAR(128) UNIQUE,
    code_key_id  VARCHAR(128),
    sealed_code  TEXT,
    auth_url     TEXT,
    state        VARCHAR(16) NOT NULL,
    claimed_by   VARCHAR(128),
    requested_by CHAR(36) NOT NULL,
    created_at   BIGINT NOT NULL,
    expires_at   BIGINT NOT NULL,
    done_at      BIGINT
);

CREATE INDEX idx_reins_family_connect_jobs_org ON reins_family_connect_jobs (org_uuid, state);
