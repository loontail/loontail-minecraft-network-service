-- Admin sessions (opaque cookie tokens, stored hashed), launcher API tokens
-- (catalog/manifest auth, replaces strapi_api_tokens), and append-only analytics
-- events written off the hot paths.
CREATE TABLE admin_sessions (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX admin_sessions_user ON admin_sessions (user_id);

CREATE TABLE api_tokens (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    scopes       TEXT[] NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);

CREATE TABLE user_events (
    id         BIGSERIAL PRIMARY KEY,
    user_id    UUID REFERENCES users (id),
    event_type TEXT NOT NULL,
    event_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX user_events_type_time ON user_events (event_type, created_at);
CREATE INDEX presence_status_hb ON presence (status, last_heartbeat_at);
