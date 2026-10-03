-- SHA-256 (hex) of the approval device's device key (the `Reins-Device-Key` header); NULL for rows from before.
ALTER TABLE reins_devices ADD COLUMN key_hash TEXT;
