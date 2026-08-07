//! The two membership predicates the admission paths share. They live together so the
//! executor-choice warning below is discoverable from either, and so the
//! `LEAST/GREATEST` canonical ordering of a friendship pair is stated once.

use uuid::Uuid;

use loontail_core::error::AppResult;

/// Runs on any executor. why: an admission path that already holds a transaction MUST
/// pass `&mut *tx` here — reaching back for `&state.pool` while holding one pooled
/// connection makes the request need two, which self-deadlocks the pool under load.
pub(crate) async fn are_friends<'e, E>(executor: E, a: Uuid, b: Uuid) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    // `friendships` stores one row per pair with (user_a_id, user_b_id) canonically
    // ordered, so both directions must be probed through LEAST/GREATEST.
    Ok(sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM friendships
            WHERE user_a_id = LEAST($1, $2) AND user_b_id = GREATEST($1, $2)
        )
        "#,
    )
    .bind(a)
    .bind(b)
    .fetch_one(executor)
    .await?)
}

/// Runs on any executor — same pool-deadlock rule as [`are_friends`].
pub(crate) async fn user_exists<'e, E>(executor: E, id: Uuid) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(id)
            .fetch_one(executor)
            .await?,
    )
}
