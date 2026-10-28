-- The email send hold (Decisions v2, 01 §5, 08 R10): an approved email waits here for its undo window, sealed to the
-- approval device's own key (`sealed`, opaque to the server). Undo deletes the row; the release is one claim that
-- deletes it too. `device_uuid` is the device that sealed it, the only one that may claim it; `connection_uuid` the
-- connection that asked, which may cancel it. `woken`: the release push went out.
CREATE TABLE reins_send_holds (
    id              TEXT    NOT NULL PRIMARY KEY,
    user_uuid       TEXT    NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid     TEXT    NOT NULL,
    connection_uuid TEXT    NOT NULL REFERENCES reins_connections (uuid) ON DELETE CASCADE,
    request_id      TEXT    NOT NULL,
    sealed          TEXT    NOT NULL,
    release_at      BIGINT  NOT NULL,
    created_at      BIGINT  NOT NULL,
    woken           BOOLEAN NOT NULL,
    UNIQUE (user_uuid, request_id)
);

CREATE INDEX idx_reins_send_holds_release ON reins_send_holds (woken, release_at);
