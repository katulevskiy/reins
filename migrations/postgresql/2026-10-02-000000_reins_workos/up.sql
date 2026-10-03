-- Small server state that outlives a restart: the WorkOS events cursor.
CREATE TABLE rewarden_settings (
    name  VARCHAR(64) NOT NULL PRIMARY KEY,
    value TEXT        NOT NULL
);

-- The SSO provider's session each device signed in with, so that a revoked session signs that device out.
CREATE TABLE rewarden_sso_sessions (
    session_id  VARCHAR(128) NOT NULL PRIMARY KEY,
    user_uuid   CHAR(36)     NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid CHAR(36)     NOT NULL,
    created_at  BIGINT       NOT NULL
);

CREATE INDEX idx_rewarden_sso_sessions_user ON rewarden_sso_sessions (user_uuid);
