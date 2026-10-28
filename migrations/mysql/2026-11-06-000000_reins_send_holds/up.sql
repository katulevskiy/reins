-- The email send hold (Decisions v2, 01 §5, 08 R10): an approved email waits here for its undo window, sealed to the
-- approval device's own key (`sealed`, opaque to the server). Undo deletes the row; the release is one claim that
-- deletes it too. `device_uuid` is the device that sealed it, the only one that may claim it; `connection_uuid` the
-- connection that asked, which may cancel it. `woken`: the release push went out. Ids compare byte for byte
-- (`utf8mb4_bin`).
CREATE TABLE reins_send_holds (
    id              CHAR(36)    NOT NULL,
    user_uuid       CHAR(36)    NOT NULL,
    device_uuid     CHAR(36)    NOT NULL,
    connection_uuid CHAR(36)    NOT NULL,
    request_id      VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    sealed          MEDIUMTEXT  NOT NULL,
    release_at      BIGINT      NOT NULL,
    created_at      BIGINT      NOT NULL,
    woken           BOOLEAN     NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (user_uuid, request_id),
    FOREIGN KEY (user_uuid) REFERENCES users (uuid) ON DELETE CASCADE,
    FOREIGN KEY (connection_uuid) REFERENCES reins_connections (uuid) ON DELETE CASCADE
);

CREATE INDEX idx_reins_send_holds_release ON reins_send_holds (woken, release_at);
