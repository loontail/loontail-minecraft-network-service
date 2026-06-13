mod infra;

use axum::http::HeaderValue;
use axum::routing::get;
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use loontail_core::db;
use loontail_core::{AppState, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Local `.env` is optional; on Hetzner the env comes from compose.
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(
        http = %format!("{}:{}", config.http_host, config.http_port),
        "starting loontail-launcher-api"
    );

    let pool = db::connect_and_migrate(&config.database_url).await?;
    tracing::info!("database connected and migrations applied");

    // A restart drops every live relay/WebSocket connection, but their DB rows
    // linger as 'active'/'pending'. Reconcile so stale sessions don't keep guests
    // falsely "in world" (presence + friend-of-friend membership) or inflate
    // current_players.
    db::reconcile_after_restart(&pool).await?;

    let addr = format!("{}:{}", config.http_host, config.http_port);
    let state = AppState::new(pool, config);
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Compose the full application router: infrastructure endpoints plus every
/// domain, with CORS and HTTP tracing.
fn build_router(state: AppState) -> Router {
    let cors = build_cors(&state.config.cors_allowed_origins);

    // The `/api` subtree groups catalog (at root: /clients, /keywords, /servers),
    // yggdrasil (nested at /yggdrasil), and the bundle-registry public manifest
    // (nested at /bundle-registry). Grouping avoids overlapping top-level `/api`
    // nests, which axum rejects.
    // TODO: read the yggdrasil prefix from config (publicUrl) once it is added.
    let api = Router::new()
        .merge(loontail_catalog::routes())
        .nest("/yggdrasil", loontail_yggdrasil::routes())
        .nest("/bundle-registry", loontail_bundles::routes());

    Router::new()
        .route("/health", get(infra::health))
        .route("/metrics", get(infra::metrics_handler))
        .merge(loontail_network::routes())
        .nest("/api", api)
        .nest("/textures", loontail_textures::routes())
        .nest("/admin", loontail_admin::routes())
        // TODO(phase 5): serve bundle files statically from
        // data/bundle-registry under `/bundle-registry/builds/{slug}/files`.
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let base = CorsLayer::new().allow_methods(Any).allow_headers(Any);
    if allowed_origins.iter().any(|origin| origin == "*") {
        base.allow_origin(Any)
    } else {
        let origins: Vec<HeaderValue> = allowed_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        base.allow_origin(origins)
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,loontail_server=debug,loontail_network=debug,loontail_core=debug")
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
