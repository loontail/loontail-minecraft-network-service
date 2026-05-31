mod auth;
mod config;
mod db;
mod error;
mod friends;
mod health;
mod invites;
mod join_requests;
mod metrics;
mod models;
mod presence;
mod relay;
mod routes;
mod sessions;
mod signaling;
mod state;
mod users;
mod worlds;

use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Local `.env` is optional; on Hetzner the env comes from compose.
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(
        http = %format!("{}:{}", config.http_host, config.http_port),
        "starting loontail-minecraft-network-service"
    );

    let pool = db::connect_and_migrate(&config.database_url).await?;
    tracing::info!("database connected and migrations applied");

    // A restart drops every live relay/WebSocket connection, but their DB rows
    // linger as 'active'/'pending'. Reconcile so stale sessions don't keep guests
    // falsely "in world" (presence + friend-of-friend membership) or inflate
    // current_players.
    reconcile_after_restart(&pool).await?;

    let addr = format!("{}:{}", config.http_host, config.http_port);
    let state = AppState::new(pool, config);
    let app = routes::build_router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Close relay sessions and zero player counts left over from a previous run,
/// since none of those connections survive a process restart.
async fn reconcile_after_restart(pool: &sqlx::PgPool) -> anyhow::Result<()> {
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
        tracing::info!(closed_relay_sessions = closed, "reconciled orphaned relay sessions");
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,loontail_minecraft_network_service=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
