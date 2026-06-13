-- Unify auth onto a single opaque session token (SHA-256-at-rest). The former
-- `network_sessions` becomes the universal `sessions` table: the one bearer the
-- launcher and agent send on every request, and the value the admin browser
-- carries in its httpOnly cookie. `admin_sessions` collapses into it (an admin is
-- just a session whose user has `is_admin`), and the never-enforced `api_tokens`
-- table is dropped. Yggdrasil tokens (migration 0004) survive only for the
-- Minecraft game handshake (/authenticate -> /join -> /hasJoined).

ALTER TABLE network_sessions RENAME TO sessions;
ALTER INDEX network_sessions_user_id_idx RENAME TO sessions_user_id_idx;

-- Drop the now-redundant admin session table and the unused launcher API tokens.
DROP TABLE IF EXISTS admin_sessions;
DROP TABLE IF EXISTS api_tokens;
