-- Devices signed out of an account from its approval phone (Settings > Devices): a sign-in with the same device id is
-- refused, so a lost phone cannot simply sign back in with the keys it kept.
CREATE TABLE reins_device_signouts (
    user_uuid      CHAR(36)     NOT NULL REFERENCES users (uuid) ON DELETE CASCADE,
    device_uuid    VARCHAR(64)  NOT NULL,
    signed_out_at  BIGINT       NOT NULL,
    by_device_name VARCHAR(255) NOT NULL,
    PRIMARY KEY (user_uuid, device_uuid)
);
