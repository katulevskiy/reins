-- Organizations (Decisions v2 · O1). A Reins org tenant is a Vaultwarden organization: `organizations` is the tenant,
-- `users_organizations` its members (`external_id`: the WorkOS directory user) and `groups`/`groups_users` its SCIM
-- groups. These tables add what Reins needs on top, keyed by the organization's uuid.

-- Tier and sign-in of the tenant. A WorkOS organization and a WorkOS directory each belong to one tenant at most.
-- `owner_recovery_at`: when the directory removed the last Org owner (the role is kept, the account cut off).
CREATE TABLE reins_orgs (
    org_uuid      TEXT    NOT NULL PRIMARY KEY REFERENCES organizations (uuid) ON DELETE CASCADE,
    tier          TEXT    NOT NULL,
    identity      TEXT    NOT NULL,
    workos_org_id TEXT    UNIQUE,
    directory_id  TEXT    UNIQUE,
    github_org    TEXT,
    two_person    BOOLEAN NOT NULL,
    owner_recovery_at BIGINT,
    created_by    TEXT    NOT NULL,
    created_at    BIGINT  NOT NULL,
    updated_at    BIGINT  NOT NULL
);

-- Domains the tenant claims; one counts once `verified_at` is set (a DNS TXT record), and one org at most verifies a
-- domain (`reins_org_verified_domains`).
CREATE TABLE reins_org_domains (
    org_uuid    TEXT   NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    domain      TEXT   NOT NULL,
    token       TEXT   NOT NULL,
    verified_at BIGINT,
    checked_at  BIGINT,
    created_at  BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, domain)
);

CREATE INDEX idx_reins_org_domains_domain ON reins_org_domains (domain);

-- The verified domains, one row each: the primary key is what makes one org at most verify a domain, even when two
-- verify it at the same moment (a check-then-write in the transaction would not).
CREATE TABLE reins_org_verified_domains (
    domain      TEXT         NOT NULL PRIMARY KEY,
    org_uuid    TEXT         NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    verified_at BIGINT       NOT NULL
);

-- Each member's state in this org, the authority for everything org-side (sign-in, roles, owners, recovery): one
-- org's directory or admins change only their own rows here, never `users.enabled` or another tenant's. An account is
-- a member of one org at most (`user_uuid` unique): a Managed account is its org's alone, and a Personal account is a
-- different account.
CREATE TABLE reins_org_members (
    org_uuid        TEXT   NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid       TEXT   NOT NULL UNIQUE REFERENCES users (uuid) ON DELETE CASCADE,
    membership_uuid TEXT   NOT NULL,
    state           TEXT   NOT NULL,
    changed_at      BIGINT NOT NULL,
    changed_by      TEXT,
    PRIMARY KEY (org_uuid, user_uuid)
);

-- Roles (05 §3) of a member (`subject_id`: the membership uuid) or of a group (the group uuid).
CREATE TABLE reins_org_roles (
    org_uuid     TEXT   NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    subject_kind TEXT   NOT NULL,
    subject_id   TEXT   NOT NULL,
    role         TEXT   NOT NULL,
    granted_at   BIGINT NOT NULL,
    granted_by   TEXT   NOT NULL,
    PRIMARY KEY (org_uuid, subject_kind, subject_id, role)
);

-- Admin actions waiting for a second person (`action`: reins_proto::org::AdminAction as JSON).
CREATE TABLE reins_org_changes (
    uuid        TEXT   NOT NULL PRIMARY KEY,
    org_uuid    TEXT   NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    action      TEXT   NOT NULL,
    proposed_by TEXT   NOT NULL,
    proposed_at BIGINT NOT NULL,
    expires_at  BIGINT NOT NULL,
    state       TEXT   NOT NULL,
    decided_by  TEXT,
    decided_at  BIGINT
);

CREATE INDEX idx_reins_org_changes_org ON reins_org_changes (org_uuid, state);

-- Devices enrolled to the tenant: the key made in secure hardware, what vouched for it, and its latest posture.
CREATE TABLE reins_org_devices (
    org_uuid      TEXT    NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    user_uuid     TEXT    NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid   TEXT    NOT NULL,
    key_algorithm TEXT    NOT NULL,
    public_key    TEXT    NOT NULL,
    hardware      BOOLEAN NOT NULL,
    attestations  TEXT    NOT NULL,
    posture       TEXT    NOT NULL,
    posture_at    BIGINT  NOT NULL,
    enrolled_at   BIGINT  NOT NULL,
    revoked_at    BIGINT,
    PRIMARY KEY (org_uuid, user_uuid, device_uuid)
);

-- The tenant's admin events, append-only and hash-linked per org (`hash` = SHA-256 of `prev_hash` and the event), so
-- O2's audit log can take them over and check that none was changed.
CREATE TABLE reins_org_events (
    org_uuid  TEXT     NOT NULL REFERENCES organizations (uuid) ON DELETE CASCADE,
    seq       BIGINT   NOT NULL,
    at        BIGINT   NOT NULL,
    kind      TEXT     NOT NULL,
    actor     TEXT,
    subject   TEXT,
    detail    TEXT     NOT NULL,
    prev_hash CHAR(64) NOT NULL,
    hash      CHAR(64) NOT NULL,
    PRIMARY KEY (org_uuid, seq)
);
