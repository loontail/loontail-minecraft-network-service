use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Migrations are embedded from the workspace root at compile time.
pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await?;

    sqlx::migrate!("../../migrations").run(&pool).await?;

    Ok(pool)
}

/// A restart drops every live relay/WebSocket connection, but their DB rows linger
/// as active. Close them so stale sessions don't keep guests falsely "in world"
/// (presence + friend-of-friend membership) or inflate current_players.
pub async fn reconcile_after_restart(pool: &PgPool) -> anyhow::Result<()> {
    let closed = sqlx::query(
        "UPDATE relay_sessions SET status = 'closed', closed_at = now() WHERE status <> 'closed'",
    )
    .execute(pool)
    .await?
    .rows_affected();
    sqlx::query("UPDATE world_sessions SET current_players = 0 WHERE status = 'open'")
        .execute(pool)
        .await?;
    if closed > 0 {
        tracing::info!(
            closed_relay_sessions = closed,
            "reconciled orphaned relay sessions"
        );
    }
    Ok(())
}
