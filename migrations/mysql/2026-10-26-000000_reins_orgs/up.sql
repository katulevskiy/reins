-- Organizations (Decisions v2 · O1). A Reins org tenant is a Vaultwarden organization: `organizations` is the tenant,
-- `users_organizations` its members (`external_id`: the WorkOS directory user) and `groups`/`groups_users` its SCIM
-- groups. These tables add what Reins needs on top, keyed by the organization's uuid.

-- WorkOS ids, device ids, keys and hashes compare byte for byte (`utf8mb4_bin`): MySQL's default collations ignore
-- case, and such ids can differ by case alone. Domains are kept lowercase ASCII.

-- Tier and sign-in of the tenant.
CREATE TABLE reins_orgs (
    org_uuid      CHAR(36)     NOT NULL PRIMARY KEY,
    tier          VARCHAR(16)  NOT NULL,
    identity      VARCHAR(32)  NOT NULL,
    workos_org_id VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin UNIQUE,
    directory_id  VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    github_org    VARCHAR(64),
    two_person    BOOLEAN      NOT NULL,
    created_by    CHAR(36)     NOT NULL,
    created_at    BIGINT       NOT NULL,
    updated_at    BIGINT       NOT NULL,
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Domains the tenant claims; one counts once `verified_at` is set (a DNS TXT record), and one org at most verifies a
-- domain (`reins_org_verified_domains`).
CREATE TABLE reins_org_domains (
    org_uuid    CHAR(36)     NOT NULL,
    domain      VARCHAR(253) NOT NULL,
    token       VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    verified_at BIGINT,
    checked_at  BIGINT,
    created_at  BIGINT       NOT NULL,
    PRIMARY KEY (org_uuid, domain),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_org_domains_domain ON reins_org_domains (domain);

-- The verified domains, one row each: the primary key is what makes one org at most verify a domain, even when two
-- verify it at the same moment (a check-then-write in the transaction would not).
CREATE TABLE reins_org_verified_domains (
    domain      VARCHAR(253) NOT NULL PRIMARY KEY,
    org_uuid    CHAR(36)     NOT NULL,
    verified_at BIGINT       NOT NULL,
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Roles (05 §3) of a member (`subject_id`: the membership uuid) or of a group (the group uuid).
CREATE TABLE reins_org_roles (
    org_uuid     CHAR(36)    NOT NULL,
    subject_kind VARCHAR(16) NOT NULL,
    subject_id   CHAR(36)    NOT NULL,
    role         VARCHAR(32) NOT NULL,
    granted_at   BIGINT      NOT NULL,
    granted_by   CHAR(36)    NOT NULL,
    PRIMARY KEY (org_uuid, subject_kind, subject_id, role),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- Admin actions waiting for a second person (`action`: reins_proto::org::AdminAction as JSON).
CREATE TABLE reins_org_changes (
    uuid        CHAR(36)    NOT NULL PRIMARY KEY,
    org_uuid    CHAR(36)    NOT NULL,
    action      TEXT        NOT NULL,
    proposed_by CHAR(36)    NOT NULL,
    proposed_at BIGINT      NOT NULL,
    expires_at  BIGINT      NOT NULL,
    state       VARCHAR(16) NOT NULL,
    decided_by  CHAR(36),
    decided_at  BIGINT,
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_org_changes_org ON reins_org_changes (org_uuid, state);

-- Devices enrolled to the tenant: the key made in secure hardware, what vouched for it, and its latest posture.
CREATE TABLE reins_org_devices (
    org_uuid      CHAR(36)    NOT NULL,
    user_uuid     CHAR(36)    NOT NULL,
    device_uuid   VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    key_algorithm VARCHAR(16) NOT NULL,
    public_key    TEXT        NOT NULL,
    hardware      BOOLEAN     NOT NULL,
    attestations  MEDIUMTEXT  NOT NULL,
    posture       TEXT        NOT NULL,
    posture_at    BIGINT      NOT NULL,
    enrolled_at   BIGINT      NOT NULL,
    revoked_at    BIGINT,
    PRIMARY KEY (org_uuid, user_uuid, device_uuid),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE,
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

-- The tenant's admin events, append-only and hash-linked per org (`hash` = SHA-256 of `prev_hash` and the event), so
-- O2's audit log can take them over and check that none was changed.
CREATE TABLE reins_org_events (
    org_uuid  CHAR(36)    NOT NULL,
    seq       BIGINT      NOT NULL,
    at        BIGINT      NOT NULL,
    kind      VARCHAR(64) NOT NULL,
    actor     CHAR(36),
    subject   VARCHAR(255),
    detail    TEXT        NOT NULL,
    prev_hash CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    hash      CHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    PRIMARY KEY (org_uuid, seq),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);
