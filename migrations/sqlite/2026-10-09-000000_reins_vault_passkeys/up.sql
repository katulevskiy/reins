-- Passkeys that open an account's vault: per passkey, the account secret sealed with a key only that passkey's PRF
-- output derives (useless to the server). Keyed by SHA-256 (hex) of the credential id, which may be long.
CREATE TABLE reins_vault_passkeys (
    user_uuid       TEXT     NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    credential_hash TEXT NOT NULL,
    credential_id   TEXT NOT NULL,
    wrapped         TEXT NOT NULL,
    name            TEXT NOT NULL,
    created_at      BIGINT NOT NULL,
    PRIMARY KEY (user_uuid, credential_hash)
);
