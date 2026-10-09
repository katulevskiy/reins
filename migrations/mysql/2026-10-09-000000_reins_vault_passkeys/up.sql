-- Passkeys that open an account's vault: per passkey, the account secret sealed with a key only that passkey's PRF
-- output derives (useless to the server). Keyed by SHA-256 (hex) of the credential id, which may be long.
CREATE TABLE reins_vault_passkeys (
    user_uuid       CHAR(36) NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    credential_hash CHAR(64) NOT NULL,
    credential_id   TEXT NOT NULL,
    wrapped         VARCHAR(512) NOT NULL,
    name            VARCHAR(255) NOT NULL,
    created_at      BIGINT NOT NULL,
    PRIMARY KEY (user_uuid, credential_hash)
);
