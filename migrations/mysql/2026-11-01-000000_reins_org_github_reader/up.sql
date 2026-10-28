-- The org's GitHub reader (Decisions v2 · O3 and O4, whichever lands first): a second, read-mostly GitHub App the org
-- installs (contents, members, pull requests: read; checks: write). Its private key and webhook secret are sealed under
-- the org's data key; app and installation ids are set by a two-person admin change.
CREATE TABLE reins_org_github_reader (
    org_uuid         VARCHAR(40) NOT NULL PRIMARY KEY,
    app_id           BIGINT NOT NULL,
    installation_id  BIGINT NOT NULL,
    key_id           VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    sealed           TEXT,
    webhook_key_id   VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    webhook_sealed   TEXT,
    set_at           BIGINT NOT NULL,
    UNIQUE KEY reins_org_github_reader_installation (installation_id),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);
