-- Join-request hygiene: at most one pending request per (world, requester), plus the
-- indexes the retention sweep needs for the join/invite tables (`user_events` gets its
-- own in 0016). `friend_requests` (0001) and `world_invites` (0002)
-- already carry the equivalent partial unique index; `join_requests` was the outlier,
-- so a requester could loop POST /world-sessions/{id}/join-requests and flood the
-- host's prompt while the table grew without bound.

-- Collapse any pre-existing duplicates before the unique index is built, keeping the
-- newest pending row per pair and expiring the rest. A fresh DB has none of these.
UPDATE join_requests
SET status = 'expired', updated_at = now()
WHERE status = 'pending'
  AND id NOT IN (
      SELECT DISTINCT ON (world_session_id, requester_user_id) id
      FROM join_requests
      WHERE status = 'pending'
      ORDER BY world_session_id, requester_user_id, created_at DESC
  );

CREATE UNIQUE INDEX join_requests_pending_unique
    ON join_requests (world_session_id, requester_user_id)
    WHERE status = 'pending';

-- Drives the hourly sweep that flips lapsed pending rows to 'expired'.
CREATE INDEX join_requests_expires_idx
    ON join_requests (expires_at)
    WHERE status = 'pending';

-- The sweep also deletes consumed/expired tickets and aged terminal rows.
CREATE INDEX join_tickets_expires_idx ON join_tickets (expires_at);
CREATE INDEX world_invites_expires_idx
    ON world_invites (expires_at)
    WHERE status IN ('pending', 'pending_approval');
