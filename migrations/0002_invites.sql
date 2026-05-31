-- World invites: a host (or a friend, under the friend-of-friend policy) invites
-- a user into a hosted world. Adds the per-world invite policy and the invites
-- table with its lifecycle states.

-- ---------------------------------------------------------------------------
-- Per-world invite policy.
--   host_only          — only the host may invite (their own friends).
--   friends_of_friends — friends already trusted by the host may also invite
--                        their own friends (subject to host approval when the
--                        invitee is not already the host's friend).
-- ---------------------------------------------------------------------------
ALTER TABLE world_sessions
    ADD COLUMN invite_policy TEXT NOT NULL DEFAULT 'host_only';

-- ---------------------------------------------------------------------------
-- World invites.
--   pending          — delivered to the invitee, awaiting accept/decline.
--   pending_approval — friend-of-friend invite awaiting the host's approval.
--   accepted | declined | revoked — terminal states.
-- ---------------------------------------------------------------------------
CREATE TABLE world_invites (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    world_session_id UUID NOT NULL REFERENCES world_sessions (id) ON DELETE CASCADE,
    host_user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    inviter_user_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    invitee_user_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status           TEXT NOT NULL DEFAULT 'pending',
    expires_at       TIMESTAMPTZ NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT world_invites_not_self CHECK (inviter_user_id <> invitee_user_id)
);

CREATE INDEX world_invites_invitee_idx ON world_invites (invitee_user_id, status);
CREATE INDEX world_invites_host_idx ON world_invites (host_user_id, status);
CREATE INDEX world_invites_inviter_idx ON world_invites (inviter_user_id, status);

-- At most one active invite per (world, invitee).
CREATE UNIQUE INDEX world_invites_active_unique
    ON world_invites (world_session_id, invitee_user_id)
    WHERE status IN ('pending', 'pending_approval');
