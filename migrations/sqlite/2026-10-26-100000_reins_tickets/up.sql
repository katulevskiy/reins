-- Deferred decisions (Decisions v2, 03): requests that became tickets. Only metadata is in clear: the ticket, its
-- state, deadlines, connection and the request digest. The request itself is sealed to the approval device's box key
-- (`sealed_body`); the agent's answer to a "Why?" likewise (`sealed_reply`). `question` and `note` are what the
-- reviewer wrote for the agent, and `result` is a write's minimal result (ids, a status), which the agent must be able
-- to read.
CREATE TABLE reins_tickets (
    id               TEXT    NOT NULL PRIMARY KEY,
    user_uuid        TEXT    NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    connection_uuid  TEXT    NOT NULL REFERENCES reins_connections (uuid) ON DELETE CASCADE,
    connection_label TEXT    NOT NULL,
    state            TEXT    NOT NULL,
    digest           TEXT    NOT NULL,
    execution        TEXT    NOT NULL,
    on_expiry        TEXT    NOT NULL,
    finish           TEXT    NOT NULL,
    created_at       BIGINT  NOT NULL,
    deferred_at      BIGINT  NOT NULL,
    deadline         BIGINT  NOT NULL,
    window_secs      BIGINT  NOT NULL,
    escalated        BOOLEAN NOT NULL,
    approved_at      BIGINT,
    valid_secs       BIGINT  NOT NULL,
    sealed_body      TEXT    NOT NULL,
    question         TEXT,
    sealed_reply     TEXT,
    note             TEXT,
    result           TEXT,
    stale            TEXT,
    delivered        BOOLEAN NOT NULL,
    updated_at       BIGINT  NOT NULL
);

CREATE INDEX idx_reins_tickets_connection ON reins_tickets (connection_uuid, state);
CREATE INDEX idx_reins_tickets_user ON reins_tickets (user_uuid, state);
CREATE INDEX idx_reins_tickets_deadline ON reins_tickets (state, deadline);

-- Each transition of a ticket. `signed`: the event is a `SignedTicketEvent` the approval device signed; otherwise the
-- server (or the desktop app) recorded it, and nothing but this log vouches for it.
CREATE TABLE reins_ticket_events (
    ticket_id  TEXT    NOT NULL REFERENCES reins_tickets (id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    state      TEXT    NOT NULL,
    signed     BOOLEAN NOT NULL,
    event      TEXT    NOT NULL,
    created_at BIGINT  NOT NULL,
    PRIMARY KEY (ticket_id, seq)
);

-- The approval device's public keys: Ed25519 (`sign_pub`, checks the ticket events it signs; agents verify webhooks
-- with it) and X25519 (`box_pub`, what parked requests are sealed to). Bound to the device that published them: a
-- new approval device publishes its own.
CREATE TABLE reins_device_keys (
    user_uuid   TEXT   NOT NULL PRIMARY KEY REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid TEXT   NOT NULL,
    sign_pub    TEXT   NOT NULL,
    box_pub     TEXT   NOT NULL,
    updated_at  BIGINT NOT NULL
);

-- A connection's webhook (03 §4.4): an https URL that echoed a challenge.
CREATE TABLE reins_callbacks (
    connection_uuid TEXT   NOT NULL PRIMARY KEY REFERENCES reins_connections (uuid) ON DELETE CASCADE,
    url             TEXT   NOT NULL,
    verified_at     BIGINT NOT NULL
);
