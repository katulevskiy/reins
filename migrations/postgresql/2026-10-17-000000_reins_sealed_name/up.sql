-- The name the user gave a computer, sealed on the phone to that computer's desktop app key (the server cannot read
-- it): the app picks it up to show the name the user's phones show. NULL: none.
ALTER TABLE reins_connections ADD COLUMN sealed_name TEXT;
