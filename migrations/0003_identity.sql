-- Extend the network `users` table with launcher/Yggdrasil identity fields.
-- gen_random_uuid() (used by every new table) needs pgcrypto; ensure it early.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

ALTER TABLE users
    ADD COLUMN email                  TEXT,
    ADD COLUMN password_hash          TEXT,
    ADD COLUMN origin                 TEXT NOT NULL DEFAULT 'mod',   -- 'mod' | 'yggdrasil' | 'admin'
    ADD COLUMN profile_uuid           TEXT,                          -- 32-char undashed, lowercase
    ADD COLUMN confirmed              BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN blocked                BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN is_admin               BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN yggdrasil_validated_at TIMESTAMPTZ;

CREATE UNIQUE INDEX users_email_uniq
    ON users (email) WHERE email IS NOT NULL;
CREATE UNIQUE INDEX users_profile_uuid_uniq
    ON users (profile_uuid) WHERE profile_uuid IS NOT NULL;
CREATE UNIQUE INDEX users_normalized_username_uniq
    ON users (normalized_username);
