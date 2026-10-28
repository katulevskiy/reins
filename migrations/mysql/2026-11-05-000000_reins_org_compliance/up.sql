-- Organizations, O5 (Decisions v2): compliance packaging.

-- Audit streaming to the org's SIEM or object storage: the configuration sealed under the tenant data key, turned on
-- by a two-person change, and how far the chain was delivered (`sent_seq`).
CREATE TABLE reins_org_audit_sinks (
    org_uuid     VARCHAR(40) NOT NULL,
    kind         VARCHAR(16) NOT NULL,
    key_id       VARCHAR(64) NOT NULL,
    sealed       TEXT NOT NULL,
    digest       CHAR(64) NOT NULL,
    enabled      BOOLEAN NOT NULL,
    sent_seq     BIGINT NOT NULL,
    last_ok_at   BIGINT,
    last_error   TEXT,
    failing      BOOLEAN NOT NULL,
    updated_at   BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, kind),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);

-- RFC 3161 timestamps of audit checkpoints (the TSA's whole DER response, base64).
CREATE TABLE reins_org_witness (
    org_uuid     VARCHAR(40) NOT NULL,
    seq          BIGINT NOT NULL,
    tsa          TEXT NOT NULL,
    response     MEDIUMTEXT NOT NULL,
    at           BIGINT NOT NULL,
    PRIMARY KEY (org_uuid, seq),
    FOREIGN KEY (org_uuid) REFERENCES organizations (uuid) ON DELETE CASCADE
);
