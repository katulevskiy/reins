-- Phone roles and live sync (reins_proto::roster, reins_proto::oplog). Everything here is signed by the phones, and
-- everything but routing metadata is ciphertext the server cannot open. An account without a roster row is a legacy
-- account and behaves exactly as before.

-- The devices a roster lists (role `primary` or `viewer`), and those it listed before (`removed`, kept so that their op
-- counters cannot start over). `key_hash` binds the member to the device key it enrolled with.
CREATE TABLE reins_members (
    user_uuid    TEXT    NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid  TEXT    NOT NULL,
    role         TEXT    NOT NULL,
    sign_pub     TEXT    NOT NULL,
    key_hash     TEXT,
    fcm_token    TEXT,
    last_counter BIGINT  NOT NULL,
    acked_seq    BIGINT  NOT NULL,
    heartbeat    TEXT,
    heartbeat_at BIGINT,
    updated_at   BIGINT  NOT NULL,
    PRIMARY KEY (user_uuid, device_uuid)
);

-- The roster chain: one signed entry per epoch. The primary key is the epoch compare-and-swap. Only the latest entry
-- keeps the recovery wrap and a handover's sealed bundle.
CREATE TABLE reins_rosters (
    user_uuid     TEXT   NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    epoch         BIGINT NOT NULL,
    entry         TEXT   NOT NULL,
    entry_hash    TEXT   NOT NULL,
    recovery_wrap TEXT,
    bundle        TEXT,
    created_at    BIGINT NOT NULL,
    PRIMARY KEY (user_uuid, epoch)
);

-- The op log: signed ops sealed with the view key, numbered per account. Dropped up to the older of the two kept
-- checkpoints.
CREATE TABLE reins_ops (
    user_uuid  TEXT    NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    seq        BIGINT  NOT NULL,
    author     TEXT    NOT NULL,
    sign_pub   TEXT    NOT NULL,
    counter    BIGINT  NOT NULL,
    op         TEXT    NOT NULL,
    size       INTEGER NOT NULL,
    created_at BIGINT  NOT NULL,
    PRIMARY KEY (user_uuid, seq)
);

-- An author's counter is unique per signing key: a phone that enrolls again with a new key starts over, and a lookup
-- by (author, key, counter) never finds another key's op.
CREATE UNIQUE INDEX idx_reins_ops_author ON reins_ops (user_uuid, author, sign_pub, counter);

-- The primary's sealed, signed snapshots (the latest two).
CREATE TABLE reins_checkpoints (
    user_uuid  TEXT   NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    seq        BIGINT NOT NULL,
    checkpoint TEXT   NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_uuid, seq)
);
