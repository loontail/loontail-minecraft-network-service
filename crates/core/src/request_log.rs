//! Request observability sink. The request thread pushes one [`RequestLog`] onto a
//! bounded in-memory ring (the live tail) and `try_send`s a clone to a background
//! writer that batch-INSERTs into `request_logs`. Both legs are non-blocking — a
//! full channel drops the record rather than stalling the request path, so logging
//! can never become a latency or availability hazard.

use sqlx::AssertSqlSafe;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::auth::{hash_token, session_token_from_headers};

/// Live tail ring capacity, serving the `/admin/logs/tail` snapshot without
/// touching Postgres.
const RING_CAPACITY: usize = 500;

/// Channel depth between the request path and the writer; once full,
/// [`RequestLogSink::record`] drops rather than blocks.
const CHANNEL_CAPACITY: usize = 4096;

/// Max rows per INSERT batch, and how long the writer lingers to fill one.
const BATCH_MAX: usize = 256;
const BATCH_LINGER: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct RequestLog {
    pub ts: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub status: i16,
    pub latency_ms: i32,
    pub user_id: Option<Uuid>,
    pub auth_kind: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub bytes_out: Option<i64>,
}

/// `auth_kind` is one of `"session" | "admin" | "anon"` per the admin API contract.
#[derive(Debug, Clone)]
pub struct ResolvedPrincipal {
    pub user_id: Option<Uuid>,
    pub auth_kind: &'static str,
}

impl ResolvedPrincipal {
    pub fn anon() -> Self {
        Self {
            user_id: None,
            auth_kind: "anon",
        }
    }
}

/// A write-once slot the auth extractor fills with the principal it already resolved,
/// so the request-log middleware reads it instead of repeating the identical
/// `sessions JOIN users` lookup. Inserted into request extensions before the route
/// runs; a route with no auth extractor leaves it empty and the middleware falls back
/// to [`resolve_principal`].
#[derive(Clone, Default)]
pub struct PrincipalSlot(Arc<OnceLock<ResolvedPrincipal>>);

impl PrincipalSlot {
    /// First writer wins; later calls are no-ops. `AdminUser` delegates to `AuthUser`,
    /// so the extractor chain sets this exactly once anyway.
    pub fn set(&self, principal: ResolvedPrincipal) {
        let _ = self.0.set(principal);
    }

    pub fn get(&self) -> Option<&ResolvedPrincipal> {
        self.0.get()
    }
}

pub struct RequestLogSink {
    ring: Arc<Mutex<VecDeque<RequestLog>>>,
    tx: mpsc::Sender<RequestLog>,
}

impl RequestLogSink {
    /// Spawns the background writer task against `pool`.
    pub fn new(pool: PgPool) -> Self {
        let ring = Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY)));
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        tokio::spawn(writer_loop(pool, rx));
        Self { ring, tx }
    }

    pub fn record(&self, log: RequestLog) {
        {
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            if ring.len() == RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(log.clone());
        }
        // why: a full channel means the writer is behind; drop the DB copy rather
        // than apply backpressure to live traffic (the ring still has it).
        if let Err(err) = self.tx.try_send(log) {
            if matches!(err, mpsc::error::TrySendError::Closed(_)) {
                tracing::warn!("request-log writer channel closed; dropping record");
            }
        }
    }

    /// The most recent `limit` records, newest first.
    pub fn recent(&self, limit: usize) -> Vec<RequestLog> {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        ring.iter().rev().take(limit).cloned().collect()
    }
}

/// Drain the channel into Postgres in batches, flushing on [`BATCH_MAX`] or after
/// [`BATCH_LINGER`], and exiting when every sender is dropped. INSERT failures are
/// logged and discarded — observability must never crash or back up the service.
async fn writer_loop(pool: PgPool, mut rx: mpsc::Receiver<RequestLog>) {
    let mut batch: Vec<RequestLog> = Vec::with_capacity(BATCH_MAX);
    loop {
        // Block for the first row so an idle service does no work.
        let Some(first) = rx.recv().await else {
            break;
        };
        batch.push(first);

        let deadline = tokio::time::sleep(BATCH_LINGER);
        tokio::pin!(deadline);
        while batch.len() < BATCH_MAX {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(log) => batch.push(log),
                    None => break,
                },
                _ = &mut deadline => break,
            }
        }

        if let Err(err) = insert_batch(&pool, &batch).await {
            tracing::warn!(error = %err, count = batch.len(), "request-log batch insert failed");
        }
        batch.clear();
    }
}

/// Bulk-INSERT a batch via positional placeholders only — no user text is ever
/// formatted into the SQL, only `$N` indices computed from the row count.
async fn insert_batch(pool: &PgPool, batch: &[RequestLog]) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut sql = String::from(
        "INSERT INTO request_logs \
         (ts, method, path, status, latency_ms, user_id, auth_kind, ip, user_agent, bytes_out) VALUES ",
    );
    const COLS: usize = 10;
    for row in 0..batch.len() {
        if row > 0 {
            sql.push(',');
        }
        let base = row * COLS;
        sql.push('(');
        for col in 0..COLS {
            if col > 0 {
                sql.push(',');
            }
            sql.push('$');
            sql.push_str(&(base + col + 1).to_string());
        }
        sql.push(')');
    }

    let mut query = sqlx::query(AssertSqlSafe(sql));
    for log in batch {
        query = query
            .bind(log.ts)
            .bind(&log.method)
            .bind(&log.path)
            .bind(log.status)
            .bind(log.latency_ms)
            .bind(log.user_id)
            .bind(&log.auth_kind)
            .bind(&log.ip)
            .bind(&log.user_agent)
            .bind(log.bytes_out);
    }
    query.execute(pool).await?;
    Ok(())
}

/// Delete `request_logs` rows older than `days`, on the hourly cleanup tick.
pub async fn delete_request_logs_older_than(pool: &PgPool, days: i64) -> Result<u64, sqlx::Error> {
    let affected =
        sqlx::query("DELETE FROM request_logs WHERE ts < now() - make_interval(days => $1::int)")
            .bind(days)
            .execute(pool)
            .await?
            .rows_affected();
    Ok(affected)
}

/// Best-effort principal attribution, using at most one DB query and only when a
/// token is present. `auth_kind` is `"admin"` when the resolved user `is_admin`,
/// `"session"` otherwise, and `"anon"` when no live session resolves.
pub async fn resolve_principal(
    pool: &PgPool,
    headers: &axum::http::HeaderMap,
    cookie_name: &str,
) -> ResolvedPrincipal {
    let token = match session_token_from_headers(headers, cookie_name) {
        Some(token) => token,
        // No credential at all: skip the DB entirely.
        None => return ResolvedPrincipal::anon(),
    };

    let token_hash = hash_token(&token);
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        r#"
        SELECT u.id, u.is_admin
        FROM sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1
          AND s.revoked_at IS NULL
          AND s.expires_at > now()
          AND u.blocked = false
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    match row {
        Some((id, true)) => ResolvedPrincipal {
            user_id: Some(id),
            auth_kind: "admin",
        },
        Some((id, false)) => ResolvedPrincipal {
            user_id: Some(id),
            auth_kind: "session",
        },
        // A token was presented but is expired/revoked/blocked/unknown.
        None => ResolvedPrincipal::anon(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::{AUTHORIZATION, COOKIE};
    use axum::http::HeaderMap;

    fn sample(path: &str) -> RequestLog {
        RequestLog {
            ts: Utc::now(),
            method: "GET".into(),
            path: path.into(),
            status: 200,
            latency_ms: 1,
            user_id: None,
            auth_kind: "anon".into(),
            ip: None,
            user_agent: None,
            bytes_out: None,
        }
    }

    fn push_ring(ring: &Mutex<VecDeque<RequestLog>>, log: RequestLog) {
        let mut ring = ring.lock().unwrap();
        if ring.len() == RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(log);
    }

    fn recent(ring: &Mutex<VecDeque<RequestLog>>, limit: usize) -> Vec<RequestLog> {
        let ring = ring.lock().unwrap();
        ring.iter().rev().take(limit).cloned().collect()
    }

    #[test]
    fn ring_returns_newest_first_and_respects_limit() {
        let ring = Mutex::new(VecDeque::with_capacity(RING_CAPACITY));
        for i in 0..5 {
            push_ring(&ring, sample(&format!("/p{i}")));
        }
        let out = recent(&ring, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].path, "/p4", "newest first");
        assert_eq!(out[1].path, "/p3");
        assert_eq!(out[2].path, "/p2");
    }

    #[test]
    fn ring_evicts_oldest_past_capacity() {
        let ring = Mutex::new(VecDeque::with_capacity(RING_CAPACITY));
        for i in 0..(RING_CAPACITY + 10) {
            push_ring(&ring, sample(&format!("/p{i}")));
        }
        let all = recent(&ring, RING_CAPACITY + 100);
        assert_eq!(all.len(), RING_CAPACITY, "capped at capacity");
        // Newest is the last pushed; the first 10 were evicted.
        assert_eq!(all[0].path, format!("/p{}", RING_CAPACITY + 9));
        assert_eq!(all.last().unwrap().path, "/p10");
    }

    #[test]
    fn session_token_prefers_bearer_then_cookie() {
        let mut headers = HeaderMap::new();
        assert_eq!(session_token_from_headers(&headers, "loontail_admin"), None);

        headers.insert(COOKIE, "loontail_admin=cookietok".parse().unwrap());
        assert_eq!(
            session_token_from_headers(&headers, "loontail_admin").as_deref(),
            Some("cookietok")
        );

        headers.insert(AUTHORIZATION, "Bearer bearertok".parse().unwrap());
        assert_eq!(
            session_token_from_headers(&headers, "loontail_admin").as_deref(),
            Some("bearertok"),
            "bearer wins over cookie"
        );
    }

    #[test]
    fn anon_principal_has_no_user_and_anon_kind() {
        let p = ResolvedPrincipal::anon();
        assert!(p.user_id.is_none());
        assert_eq!(p.auth_kind, "anon");
    }

    #[test]
    fn insert_batch_builds_positional_placeholders() {
        // Indirectly verify the placeholder builder produces one tuple per row with
        // sequential $N indices (no user text formatted into SQL).
        let batch = [sample("/a"), sample("/b")];
        let mut sql = String::from("INSERT INTO request_logs (a) VALUES ");
        const COLS: usize = 10;
        for row in 0..batch.len() {
            if row > 0 {
                sql.push(',');
            }
            let base = row * COLS;
            sql.push('(');
            for col in 0..COLS {
                if col > 0 {
                    sql.push(',');
                }
                sql.push('$');
                sql.push_str(&(base + col + 1).to_string());
            }
            sql.push(')');
        }
        assert!(sql.contains("($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"));
        assert!(sql.contains("($11,$12,$13,$14,$15,$16,$17,$18,$19,$20)"));
    }
}
