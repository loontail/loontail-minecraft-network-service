-- loontail-launcher-api — initial network schema.
-- Applied automatically at service startup via sqlx::migrate!.

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ---------------------------------------------------------------------------
-- Users: the network's own user records, keyed by Minecraft UUID.
-- No Minecraft access token is ever stored.
-- ---------------------------------------------------------------------------
CREATE TABLE users (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    minecraft_uuid      TEXT NOT NULL UNIQUE,
    username            TEXT NOT NULL,
    normalized_username TEXT NOT NULL,
    account_type        TEXT,
    xuid                TEXT,
    client_id           TEXT,
    avatar_url          TEXT,
    skin_hash           TEXT,
    first_seen_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX users_normalized_username_idx ON users (normalized_username);

-- ---------------------------------------------------------------------------
-- Network sessions: opaque bearer tokens, stored only as a SHA-256 hash.
-- ---------------------------------------------------------------------------
CREATE TABLE network_sessions (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX network_sessions_user_id_idx ON network_sessions (user_id);

-- ---------------------------------------------------------------------------
-- Friend requests.
-- ---------------------------------------------------------------------------
CREATE TABLE friend_requests (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    from_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    to_user_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status       TEXT NOT NULL DEFAULT 'pending',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT friend_requests_not_self CHECK (from_user_id <> to_user_id)
);

-- At most one pending request per ordered pair.
CREATE UNIQUE INDEX friend_requests_pending_unique
    ON friend_requests (from_user_id, to_user_id)
    WHERE status = 'pending';
CREATE INDEX friend_requests_to_user_idx ON friend_requests (to_user_id, status);

-- ---------------------------------------------------------------------------
-- Friendships: stored once per pair with user_a_id < user_b_id (text order).
-- ---------------------------------------------------------------------------
CREATE TABLE friendships (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_a_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_b_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT friendships_ordered CHECK (user_a_id < user_b_id)
);

CREATE UNIQUE INDEX friendships_pair_unique ON friendships (user_a_id, user_b_id);
CREATE INDEX friendships_user_b_idx ON friendships (user_b_id);

-- ---------------------------------------------------------------------------
-- Presence: one row per user, status computed effective at read time using
-- last_heartbeat_at vs HEARTBEAT_TIMEOUT_SECONDS.
-- ---------------------------------------------------------------------------
CREATE TABLE presence (
    user_id                  UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    status                   TEXT NOT NULL DEFAULT 'online',
    last_heartbeat_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    minecraft_version        TEXT,
    mod_version              TEXT,
    loader                   TEXT,
    current_world_session_id UUID,
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- World sessions: a host's open local world.
-- ---------------------------------------------------------------------------
CREATE TABLE world_sessions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    host_user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'open',
    max_players     INT NOT NULL DEFAULT 5,
    current_players INT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at       TIMESTAMPTZ
);

CREATE INDEX world_sessions_host_idx ON world_sessions (host_user_id, status);

-- One open world session per host at a time.
CREATE UNIQUE INDEX world_sessions_one_open_per_host
    ON world_sessions (host_user_id)
    WHERE status = 'open';

ALTER TABLE presence
    ADD CONSTRAINT presence_world_fk
    FOREIGN KEY (current_world_session_id)
    REFERENCES world_sessions (id) ON DELETE SET NULL;

-- ---------------------------------------------------------------------------
-- Join requests (inWorld flow: requires host approval).
-- ---------------------------------------------------------------------------
CREATE TABLE join_requests (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    requester_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    host_user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    world_session_id  UUID NOT NULL REFERENCES world_sessions (id) ON DELETE CASCADE,
    status            TEXT NOT NULL DEFAULT 'pending',
    expires_at        TIMESTAMPTZ NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX join_requests_host_idx ON join_requests (host_user_id, status);
CREATE INDEX join_requests_requester_idx ON join_requests (requester_user_id, status);

-- ---------------------------------------------------------------------------
-- Join tickets: short-lived proof allowing a guest to open a relay session.
-- ---------------------------------------------------------------------------
CREATE TABLE join_tickets (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id          UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    world_session_id UUID NOT NULL REFERENCES world_sessions (id) ON DELETE CASCADE,
    token_hash       TEXT NOT NULL UNIQUE,
    expires_at       TIMESTAMPTZ NOT NULL,
    consumed_at      TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX join_tickets_world_idx ON join_tickets (world_session_id);

-- ---------------------------------------------------------------------------
-- Relay sessions: one rendezvous pairing (host <-> one guest) per row.
-- ---------------------------------------------------------------------------
CREATE TABLE relay_sessions (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    world_session_id UUID NOT NULL REFERENCES world_sessions (id) ON DELETE CASCADE,
    host_user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    guest_user_id    UUID REFERENCES users (id) ON DELETE SET NULL,
    join_ticket_id   UUID REFERENCES join_tickets (id) ON DELETE SET NULL,
    status           TEXT NOT NULL DEFAULT 'pending',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at        TIMESTAMPTZ
);

CREATE INDEX relay_sessions_world_idx ON relay_sessions (world_session_id, status);
