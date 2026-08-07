//! Lifecycle hygiene for the join/invite tables and the analytics event log, run off
//! the request path by the server's hourly tick — the same shape as
//! `cleanup_expired_sessions` and `delete_request_logs_older_than`.
//!
//! Without it, lapsed join requests and invites stay `pending` forever (invisible to
//! every list yet occupying their unique slots), consumed join tickets are never
//! reclaimed, and `user_events` grows for the life of the deployment.

use loontail_core::error::AppResult;
use sqlx::AssertSqlSafe;
use sqlx::PgPool;

/// What one [`cleanup_stale_join_state`] pass touched. Reported per field so an
/// operator can see which lever is actually reclaiming rows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CleanupCounts {
    pub expired_join_requests: u64,
    pub expired_invites: u64,
    pub deleted_join_tickets: u64,
    pub deleted_terminal_join_requests: u64,
    pub deleted_terminal_invites: u64,
    pub deleted_user_events: u64,
}

impl CleanupCounts {
    pub fn total(&self) -> u64 {
        self.expired_join_requests
            + self.expired_invites
            + self.deleted_join_tickets
            + self.deleted_terminal_join_requests
            + self.deleted_terminal_invites
            + self.deleted_user_events
    }
}

/// Terminal statuses — a row in one of these can never transition again, so once it is
/// past the retention window it is pure history and safe to delete.
const TERMINAL_JOIN_REQUEST: &str = "('accepted', 'declined', 'expired')";
const TERMINAL_INVITE: &str = "('accepted', 'declined', 'revoked', 'expired')";

/// Advance lapsed join requests/invites to `expired`, reclaim spent join tickets, and
/// drop terminal rows plus `user_events` older than `retention_days`.
///
/// Deletion is confined to terminal rows, so a live relay can never lose state it still
/// needs: `relay_sessions.join_ticket_id` is `ON DELETE SET NULL` (0001) and a ticket is
/// only removed once consumed or expired, by which point the relay session holds its own
/// `world_session_id`.
pub async fn cleanup_stale_join_state(
    pool: &PgPool,
    retention_days: i64,
) -> AppResult<CleanupCounts> {
    let expired_join_requests = sqlx::query(
        "UPDATE join_requests SET status = 'expired', updated_at = now() \
         WHERE status = 'pending' AND expires_at <= now()",
    )
    .execute(pool)
    .await?
    .rows_affected();

    let expired_invites = sqlx::query(
        "UPDATE world_invites SET status = 'expired', updated_at = now() \
         WHERE status IN ('pending', 'pending_approval') AND expires_at <= now()",
    )
    .execute(pool)
    .await?
    .rows_affected();

    let deleted_join_tickets = sqlx::query(
        "DELETE FROM join_tickets \
         WHERE created_at < now() - make_interval(days => $1::int) \
           AND (consumed_at IS NOT NULL OR expires_at <= now())",
    )
    .bind(retention_days)
    .execute(pool)
    .await?
    .rows_affected();

    let deleted_terminal_join_requests = sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM join_requests \
         WHERE status IN {TERMINAL_JOIN_REQUEST} \
           AND updated_at < now() - make_interval(days => $1::int)"
    )))
    .bind(retention_days)
    .execute(pool)
    .await?
    .rows_affected();

    let deleted_terminal_invites = sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM world_invites \
         WHERE status IN {TERMINAL_INVITE} \
           AND updated_at < now() - make_interval(days => $1::int)"
    )))
    .bind(retention_days)
    .execute(pool)
    .await?
    .rows_affected();

    let deleted_user_events = delete_aged_user_events(pool, retention_days).await?;

    Ok(CleanupCounts {
        expired_join_requests,
        expired_invites,
        deleted_join_tickets,
        deleted_terminal_join_requests,
        deleted_terminal_invites,
        deleted_user_events,
    })
}

/// Rows per retention DELETE against `user_events`.
///
/// why: it is the one unbounded table here. A single unchunked DELETE on a multi-million
/// row log holds a pooled connection for minutes, emits one huge WAL/bloat burst, and a
/// `statement_timeout` kill rolls ALL of it back — so the hourly tick would retry forever
/// making no progress. Chunking commits progress per statement instead.
const USER_EVENT_DELETE_CHUNK: i64 = 50_000;

/// Delete `user_events` older than `retention_days` in [`USER_EVENT_DELETE_CHUNK`]-sized
/// statements, returning the total. The `created_at` index added in 0016 is what keeps
/// each chunk's subquery off a full scan.
async fn delete_aged_user_events(pool: &PgPool, retention_days: i64) -> AppResult<u64> {
    delete_aged_user_events_in_chunks(pool, retention_days, USER_EVENT_DELETE_CHUNK).await
}

/// [`delete_aged_user_events`] with an explicit chunk size, so the loop that must drain
/// EVERY aged row (not just the first chunk) is testable without seeding 50k rows.
async fn delete_aged_user_events_in_chunks(
    pool: &PgPool,
    retention_days: i64,
    chunk: i64,
) -> AppResult<u64> {
    let mut deleted = 0u64;
    loop {
        let affected = sqlx::query(
            "DELETE FROM user_events WHERE id IN ( \
                 SELECT id FROM user_events \
                 WHERE created_at < now() - make_interval(days => $1::int) \
                 ORDER BY created_at \
                 LIMIT $2 \
             )",
        )
        .bind(retention_days)
        .bind(chunk)
        .execute(pool)
        .await?
        .rows_affected();

        deleted += affected;
        if affected < chunk as u64 {
            return Ok(deleted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chunking must not turn the retention sweep into a partial one: with a chunk of 2
    /// and 5 aged rows the loop has to run until the table is drained, and it must not
    /// touch anything inside the window.
    #[sqlx::test(migrations = "../../migrations")]
    async fn chunked_retention_delete_drains_every_aged_row(pool: PgPool) {
        sqlx::query(
            "INSERT INTO user_events (event_type, created_at) \
             SELECT 'aged', now() - interval '40 days' FROM generate_series(1, 5)",
        )
        .execute(&pool)
        .await
        .expect("seed aged");
        sqlx::query("INSERT INTO user_events (event_type, created_at) VALUES ('fresh', now())")
            .execute(&pool)
            .await
            .expect("seed fresh");

        let deleted = delete_aged_user_events_in_chunks(&pool, 30, 2)
            .await
            .expect("delete");
        assert_eq!(deleted, 5, "the loop drains past the first chunk");

        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 1, "only the in-window row survives");
    }

    /// 0016: the sweep's `created_at` predicate needs its own index. 0008 indexes
    /// `(event_type, created_at)`, whose leading column this predicate cannot use, so
    /// without 0016 every hourly pass seq-scans the whole event log.
    #[sqlx::test(migrations = "../../migrations")]
    async fn user_events_created_at_is_indexed(pool: PgPool) {
        let indexed: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_indexes \
                 WHERE tablename = 'user_events' AND indexdef LIKE '%(created_at)%' \
             )",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            indexed,
            "user_events needs a created_at-leading index for the retention sweep"
        );
    }
}
