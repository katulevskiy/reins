-- Phone roles and live sync (reins_proto::roster, reins_proto::oplog). Everything here is signed by the phones, and
-- everything but routing metadata is ciphertext the server cannot open. An account without a roster row is a legacy
-- account and behaves exactly as before.

-- Device ids, keys and hashes compare byte for byte (`utf8mb4_bin`): MySQL's default collations ignore case, and
-- base64url keys and device ids differ by case alone.

-- The devices a roster lists (role `primary` or `viewer`), and those it listed before (`removed`, kept so that their op
-- counters cannot start over). `key_hash` binds the member to the device key it enrolled with.
CREATE TABLE reins_members (
    user_uuid    CHAR(36)    NOT NULL,
    device_uuid  VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    role         VARCHAR(16) NOT NULL,
    sign_pub     VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    key_hash     CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    fcm_token    TEXT,
    last_counter BIGINT      NOT NULL,
    acked_seq    BIGINT      NOT NULL,
    heartbeat    TEXT,
    heartbeat_at BIGINT,
    updated_at   BIGINT      NOT NULL,
    PRIMARY KEY (user_uuid, device_uuid),
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

-- The roster chain: one signed entry per epoch. The primary key is the epoch compare-and-swap. Only the latest entry
-- keeps the recovery wrap and a handover's sealed bundle.
CREATE TABLE reins_rosters (
    user_uuid     CHAR(36) NOT NULL,
    epoch         BIGINT   NOT NULL,
    entry         TEXT     NOT NULL,
    entry_hash    CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    recovery_wrap TEXT,
    bundle        TEXT,
    created_at    BIGINT   NOT NULL,
    PRIMARY KEY (user_uuid, epoch),
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

-- The op log: signed ops sealed with the view key, numbered per account. Dropped up to the older of the two kept
-- checkpoints. An op is up to 64 KiB before base64, more than TEXT holds.
CREATE TABLE reins_ops (
    user_uuid  CHAR(36)    NOT NULL,
    seq        BIGINT      NOT NULL,
    author     VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    sign_pub   VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    counter    BIGINT      NOT NULL,
    op         MEDIUMTEXT  NOT NULL,
    size       INTEGER     NOT NULL,
    created_at BIGINT      NOT NULL,
    PRIMARY KEY (user_uuid, seq),
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

-- An author's counter is unique per signing key: a phone that enrolls again with a new key starts over, and a lookup
-- by (author, key, counter) never finds another key's op.
CREATE UNIQUE INDEX idx_reins_ops_author ON reins_ops (user_uuid, author, sign_pub, counter);

-- The primary's sealed, signed snapshots (the latest two), up to 8 MiB before base64 (about 11 MiB stored, under the
-- 16 MiB max_allowed_packet MariaDB and older MySQL servers default to).
CREATE TABLE reins_checkpoints (
    user_uuid  CHAR(36) NOT NULL,
    seq        BIGINT   NOT NULL,
    checkpoint LONGTEXT NOT NULL,
    created_at BIGINT   NOT NULL,
    PRIMARY KEY (user_uuid, seq),
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);
