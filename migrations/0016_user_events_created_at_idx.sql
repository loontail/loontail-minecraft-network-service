-- `user_events` is the only table in the hourly retention sweep that grows without
-- bound (it is the analytics event log). 0008 indexes it as (event_type, created_at)
-- only, which a `WHERE created_at < cutoff` predicate cannot use as a leading column,
-- so every sweep seq-scanned the whole log while holding a pooled connection.
CREATE INDEX user_events_created_at_idx ON user_events (created_at);
