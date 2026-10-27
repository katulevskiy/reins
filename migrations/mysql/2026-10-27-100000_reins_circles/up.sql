-- Circles (Decisions v2, 04): shared control between two Reins accounts. The server routes and stores only what
-- routing needs in clear: who is in an arrangement with whom (08 R8 accepts that it learns this), its kind and state,
-- deadlines and timestamps. The arrangement itself is stored sealed to each side's roster credential key; a review
-- packet sealed to the helper's approval phone, an answer to the owner's. An invitation keeps the SHA-256 of its link's
-- secret, never the secret, and its protocol steps (keys, nonces, sealed terms) as JSON.
CREATE TABLE reins_circle_invites (
    id          VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    token_hash  CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    owner_uuid  CHAR(36) NOT NULL,
    kind        VARCHAR(16) NOT NULL,
    state       VARCHAR(16) NOT NULL,
    owner_pub   VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    owner_epoch BIGINT NOT NULL,
    commitment  CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    helper_uuid CHAR(36),
    data        MEDIUMTEXT NOT NULL,
    link_id     VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    created_at  BIGINT NOT NULL,
    expires_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (token_hash),
    FOREIGN KEY (owner_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_circle_invites_owner ON reins_circle_invites (owner_uuid, state);

-- One arrangement's routing. `owner_copy` and `helper_copy`: the signed arrangement sealed to each side's roster
-- credential key. `last_change`: the last signed state change (pause, resume, end), whose body names no scope.
CREATE TABLE reins_circle_links (
    id          VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    owner_uuid  CHAR(36) NOT NULL,
    helper_uuid CHAR(36) NOT NULL,
    kind        VARCHAR(16) NOT NULL,
    state       VARCHAR(16) NOT NULL,
    owner_copy  MEDIUMTEXT NOT NULL,
    helper_copy MEDIUMTEXT NOT NULL,
    last_change TEXT,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (owner_uuid) REFERENCES users (uuid) ON DELETE CASCADE,
    FOREIGN KEY (helper_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_circle_links_owner ON reins_circle_links (owner_uuid);
CREATE INDEX idx_reins_circle_links_helper ON reins_circle_links (helper_uuid);

-- A review packet (or a notice) for the helper, sealed to its approval phone; the answer sealed to the owner's.
CREATE TABLE reins_circle_packets (
    id            VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    link_id       VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    owner_uuid    CHAR(36) NOT NULL,
    helper_uuid   CHAR(36) NOT NULL,
    kind          VARCHAR(16) NOT NULL,
    state         VARCHAR(16) NOT NULL,
    sealed        MEDIUMTEXT NOT NULL,
    sealed_answer MEDIUMTEXT,
    deadline      BIGINT NOT NULL,
    created_at    BIGINT NOT NULL,
    answered_at   BIGINT,
    delivered     BOOLEAN NOT NULL,
    owner_seen    BOOLEAN NOT NULL,
    updated_at    BIGINT NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (link_id) REFERENCES reins_circle_links (id) ON DELETE CASCADE,
    FOREIGN KEY (owner_uuid) REFERENCES users (uuid) ON DELETE CASCADE,
    FOREIGN KEY (helper_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_circle_packets_helper ON reins_circle_packets (helper_uuid, state);
CREATE INDEX idx_reins_circle_packets_owner ON reins_circle_packets (owner_uuid, state);
CREATE INDEX idx_reins_circle_packets_deadline ON reins_circle_packets (state, deadline);
