CREATE TABLE rewarden_devices (
    user_uuid   CHAR(36) NOT NULL PRIMARY KEY REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid CHAR(36) NOT NULL,
    fcm_token   TEXT,
    updated_at  BIGINT   NOT NULL
);

CREATE TABLE rewarden_clients (
    client_id     VARCHAR(64) NOT NULL PRIMARY KEY,
    client_name   TEXT        NOT NULL,
    redirect_uris TEXT        NOT NULL,
    created_at    BIGINT      NOT NULL
);

CREATE TABLE rewarden_connections (
    uuid         CHAR(36) NOT NULL PRIMARY KEY,
    user_uuid    CHAR(36) NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    client_id    TEXT     NOT NULL,
    client_name  TEXT     NOT NULL,
    client_host  TEXT     NOT NULL,
    label        TEXT     NOT NULL,
    created_at   BIGINT   NOT NULL,
    last_used_at BIGINT
);

CREATE INDEX idx_rewarden_connections_user ON rewarden_connections (user_uuid);

CREATE TABLE rewarden_refresh_tokens (
    token_hash      CHAR(64) NOT NULL PRIMARY KEY,
    connection_uuid CHAR(36) NOT NULL REFERENCES rewarden_connections (uuid) ON DELETE CASCADE,
    expires_at      BIGINT   NOT NULL
);

CREATE INDEX idx_rewarden_refresh_tokens_connection ON rewarden_refresh_tokens (connection_uuid);
