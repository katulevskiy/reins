-- Family connections (Decisions v2 · P7): the accounts a guardian connected for a family member. The refresh tokens
-- live only in the family's executor; the server keeps which provider is connected, and the jobs it queues for the
-- executor. A finishing job holds the provider's one-time code sealed at rest (useless without the executor's PKCE
-- verifier and client secret), and only until the executor claims it.
CREATE TABLE reins_family_connections (
    org_uuid     TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid    TEXT NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    provider     TEXT NOT NULL,
    account      TEXT,
    state        TEXT NOT NULL,
    connected_at BIGINT,
    updated_at   BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, user_uuid, provider)
);

CREATE TABLE reins_family_connect_jobs (
    id           TEXT NOT NULL PRIMARY KEY,
    org_uuid     TEXT NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid    TEXT NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    provider     TEXT NOT NULL,
    phase        TEXT NOT NULL,
    oauth_state  TEXT UNIQUE,
    code_key_id  TEXT,
    sealed_code  TEXT,
    auth_url     TEXT,
    state        TEXT NOT NULL,
    claimed_by   TEXT,
    requested_by TEXT NOT NULL,
    created_at   BIGINT NOT NULL,
    expires_at   BIGINT NOT NULL,
    done_at      BIGINT
);

CREATE INDEX idx_reins_family_connect_jobs_org ON reins_family_connect_jobs (org_uuid, state);
