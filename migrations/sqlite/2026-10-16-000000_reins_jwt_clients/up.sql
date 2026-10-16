-- Clients (metadata-document URLs, stored as SHA-256 hex) ever seen with private_key_jwt in their metadata document
-- or in a grant. Such a client is never accepted without a valid assertion again, whatever an older grant or a later
-- document says. (Stored refresh tokens that say private_key_jwt count too; they are checked directly.)
CREATE TABLE reins_jwt_clients (
    client_hash   CHAR(64) NOT NULL PRIMARY KEY,
    first_seen_at BIGINT   NOT NULL
);
