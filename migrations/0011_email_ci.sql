-- Email uniqueness must match the case-insensitive login lookup
-- (authenticate_password resolves `lower(email) = $1`). The original
-- case-sensitive index let `Victim@x.com` and `victim@x.com` coexist — email
-- squatting plus a non-deterministic `LIMIT 1` at login. Replace it with a
-- functional unique index on `lower(email)`. (Pre-launch: no rows to dedupe.)
DROP INDEX IF EXISTS users_email_uniq;
CREATE UNIQUE INDEX users_email_uniq ON users (lower(email)) WHERE email IS NOT NULL;
