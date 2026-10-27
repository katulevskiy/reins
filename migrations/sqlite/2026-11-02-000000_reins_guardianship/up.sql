-- Guardianship (Decisions v2 P7, 04 §4, 08 R7/R8). In clear, as routing needs: each signed change of a link (its
-- body names no scope or terms) and when the server received it; when a ward's end request takes effect; and the
-- notices the server writes to a ward's guardians for what it observes (a reset, a deletion, a roster change, a change
-- of the guardianship). A notice keeps its link id as text: it outlives the link's row.
CREATE TABLE reins_circle_changes (
    link_id     TEXT NOT NULL,
    seq         BIGINT NOT NULL,
    change_body TEXT NOT NULL,
    at          BIGINT NOT NULL,
    PRIMARY KEY (link_id, seq),
    FOREIGN KEY (link_id) REFERENCES reins_circle_links (id) ON DELETE CASCADE
);

ALTER TABLE reins_circle_links ADD COLUMN effective_at BIGINT;

CREATE TABLE reins_guardian_notices (
    id            TEXT NOT NULL,
    guardian_uuid TEXT NOT NULL,
    link_id       TEXT NOT NULL,
    kind          TEXT NOT NULL,
    detail        TEXT NOT NULL,
    at            BIGINT NOT NULL,
    pushed        BOOLEAN NOT NULL,
    fetched       BOOLEAN NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (guardian_uuid) REFERENCES users (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_guardian_notices_guardian ON reins_guardian_notices (guardian_uuid, fetched);
CREATE INDEX idx_reins_guardian_notices_pushed ON reins_guardian_notices (pushed);
