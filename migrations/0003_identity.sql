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

-- Yggdrasil/admin-created accounts have no Minecraft session, so minecraft_uuid
-- must be nullable (the mod-bootstrap path still sets it). The UNIQUE constraint
-- from 0001 already ignores NULLs, so multiple credential-only users coexist.
ALTER TABLE users ALTER COLUMN minecraft_uuid DROP NOT NULL;

CREATE UNIQUE INDEX users_email_uniq
    ON users (email) WHERE email IS NOT NULL;
CREATE UNIQUE INDEX users_profile_uuid_uniq
    ON users (profile_uuid) WHERE profile_uuid IS NOT NULL;
CREATE UNIQUE INDEX users_normalized_username_uniq
    ON users (normalized_username);
