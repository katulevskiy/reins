-- The Family tenant (Decisions v2 · P7, 08 R7): an org of tier `family`. Its guardians are the org; its members joined
-- by an invitation bound to their exact email, with a new account (08 R6). An invitation keeps the SHA-256 of its secret,
-- never the secret. `reins_family_members.minor`: the member's account is a minor's (set at invitation, or later with
-- the member's acceptance). `reins_family_leaves`: a member's request to leave (an adult's takes effect after the
-- family's notice; a minor's waits for a guardian's consent).
CREATE TABLE reins_families (
    org_uuid    TEXT    NOT NULL PRIMARY KEY REFERENCES organizations (uuid) ON DELETE CASCADE,
    notice_days INTEGER NOT NULL,
    created_at  BIGINT  NOT NULL
);

CREATE TABLE reins_family_invites (
    id         TEXT    NOT NULL PRIMARY KEY,
    token_hash TEXT    NOT NULL UNIQUE,
    org_uuid   TEXT    NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    email      TEXT    NOT NULL,
    minor      BOOLEAN NOT NULL,
    guardian   BOOLEAN NOT NULL,
    approved   BOOLEAN NOT NULL,
    created_by TEXT    NOT NULL,
    created_at BIGINT  NOT NULL,
    expires_at BIGINT  NOT NULL,
    used_at    BIGINT,
    used_by    TEXT
);

CREATE INDEX idx_reins_family_invites_org ON reins_family_invites (org_uuid);

CREATE TABLE reins_family_members (
    org_uuid          TEXT    NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid         TEXT    NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    minor             BOOLEAN NOT NULL,
    minor_proposed_at BIGINT,
    minor_proposed_by TEXT,
    PRIMARY KEY (org_uuid, user_uuid)
);

CREATE TABLE reins_family_leaves (
    org_uuid     TEXT   NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid    TEXT   NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    requested_at BIGINT NOT NULL,
    effective_at BIGINT,
    consented_by TEXT,
    PRIMARY KEY (org_uuid, user_uuid)
);
