-- Request observability: one row per served HTTP request, written off the hot
-- path by a batching background writer (the request thread only pushes to a ring
-- buffer + a bounded channel, never touching Postgres synchronously). Retention
-- is enforced by an hourly cleanup tick (REQUEST_LOG_RETENTION_DAYS, default 7).
-- `user_id` is best-effort principal attribution and nulls out if the user is
-- deleted, so an old log never blocks an account removal.
CREATE TABLE request_logs (
    id         BIGSERIAL PRIMARY KEY,
    ts         TIMESTAMPTZ NOT NULL DEFAULT now(),
    method     TEXT NOT NULL,
    path       TEXT NOT NULL,
    status     SMALLINT NOT NULL,
    latency_ms INTEGER NOT NULL,
    user_id    UUID REFERENCES users (id) ON DELETE SET NULL,
    auth_kind  TEXT NOT NULL DEFAULT 'anon',
    ip         TEXT,
    user_agent TEXT,
    bytes_out  BIGINT
);

CREATE INDEX request_logs_ts_idx ON request_logs (ts DESC);
CREATE INDEX request_logs_path_ts_idx ON request_logs (path, ts DESC);
CREATE INDEX request_logs_status_ts_idx ON request_logs (status, ts DESC);
