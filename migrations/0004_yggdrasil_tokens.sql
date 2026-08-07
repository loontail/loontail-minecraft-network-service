-- Yggdrasil access/client token pairs. Both are 64-hex plaintext: the Mojang
-- protocol requires the client token be echoed verbatim, so it cannot be hashed.
CREATE TABLE yggdrasil_tokens (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    access_token TEXT NOT NULL UNIQUE,          -- 64-hex
    client_token TEXT NOT NULL,                 -- 64-hex
    issued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);

CREATE INDEX yggdrasil_tokens_user ON yggdrasil_tokens (user_id);
