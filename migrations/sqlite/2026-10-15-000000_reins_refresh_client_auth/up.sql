-- How the client authenticated when it got this refresh token ('none' or 'private_key_jwt'): a refresh must
-- authenticate the same way, so a stolen refresh token of a private_key_jwt client is useless without its key.
ALTER TABLE reins_refresh_tokens ADD COLUMN client_auth TEXT NOT NULL DEFAULT 'none';
