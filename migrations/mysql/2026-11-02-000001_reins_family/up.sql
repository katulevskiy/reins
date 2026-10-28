-- The Family tenant (Decisions v2 · P7, 08 R7): an org of tier `family`. Its guardians are the org; its members joined
-- by an invitation bound to their exact email, with a new account (08 R6). An invitation keeps the SHA-256 of its secret,
-- never the secret. `reins_family_members.minor`: the member's account is a minor's (set at invitation, or later with
-- the member's acceptance). `reins_family_leaves`: a member's request to leave (an adult's takes effect after the
-- family's notice; a minor's waits for a guardian's consent).
CREATE TABLE reins_families (
    org_uuid    VARCHAR(40) NOT NULL PRIMARY KEY,
    notice_days INTEGER     NOT NULL,
    created_at  BIGINT      NOT NULL,
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_family_invites (
    id         VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL PRIMARY KEY,
    token_hash CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL UNIQUE,
    org_uuid   VARCHAR(40)  NOT NULL,
    email      VARCHAR(255) NOT NULL,
    minor      BOOLEAN      NOT NULL,
    guardian   BOOLEAN      NOT NULL,
    approved   BOOLEAN      NOT NULL,
    created_by CHAR(36)     NOT NULL,
    created_at BIGINT       NOT NULL,
    expires_at BIGINT       NOT NULL,
    used_at    BIGINT,
    used_by    CHAR(36),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_family_invites_org ON reins_family_invites (org_uuid);

CREATE TABLE reins_family_members (
    org_uuid          VARCHAR(40) NOT NULL,
    user_uuid         CHAR(36)    NOT NULL,
    minor             BOOLEAN     NOT NULL,
    minor_proposed_at BIGINT,
    minor_proposed_by CHAR(36),
    PRIMARY KEY (org_uuid, user_uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE TABLE reins_family_leaves (
    org_uuid     VARCHAR(40) NOT NULL,
    user_uuid    CHAR(36)    NOT NULL,
    requested_at BIGINT      NOT NULL,
    effective_at BIGINT,
    consented_by CHAR(36),
    PRIMARY KEY (org_uuid, user_uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);
