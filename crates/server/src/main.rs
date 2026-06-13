mod infra;

use std::time::Duration;

use axum::http::HeaderValue;
use axum::routing::get;
use axum::{Router, ServiceExt};
use tower::Layer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use loontail_core::auth::{cleanup_expired_admin_sessions, cleanup_expired_yggdrasil};
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

    // Yggdrasil signing key: load the existing PEM or generate+persist a fresh
    // RSA-4096 key before serving, so signed-texture handlers never 500.
    loontail_yggdrasil::init_crypto(&config)?;
    // On-disk storage roots for textures and bundles, created up front so the
    // first upload/create never races a missing directory.
    loontail_textures::init(&config).await?;
    loontail_bundles::init(&config.bundles.storage_root)?;

    let pool = db::connect_and_migrate(&config.database_url).await?;
    tracing::info!("database connected and migrations applied");

    // A restart drops every live relay/WebSocket connection, but their DB rows
    // linger as 'active'/'pending'. Reconcile so stale sessions don't keep guests
    // falsely "in world" (presence + friend-of-friend membership) or inflate
    // current_players.
    db::reconcile_after_restart(&pool).await?;

    // Seed the bootstrap admin (idempotent; no-op once any admin exists or when
    // ADMIN_BOOTSTRAP_PASSWORD is unset) so a fresh deployment is manageable.
    loontail_admin::ensure_bootstrap_admin(&pool, &config.admin).await?;

    let addr = format!("{}:{}", config.http_host, config.http_port);
    let state = AppState::new(pool, config);

    spawn_cleanup_tasks(state.clone());

    let app = build_router(state);

    // Trim a trailing slash before routing so client variants like
    // `/api/yggdrasil/` resolve to the nested router's root meta route the same as
    // `/api/yggdrasil`. Applied outside the router (matchit runs after the trim);
    // `ServiceExt::into_make_service` adapts the layered service for `axum::serve`.
    let app = NormalizePathLayer::trim_trailing_slash().layer(app);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(
        listener,
        ServiceExt::<axum::extract::Request>::into_make_service(app),
    )
    .await?;

    Ok(())
}

/// Compose the full application router: infrastructure endpoints plus every
/// domain, with CORS and HTTP tracing.
fn build_router(state: AppState) -> Router {
    let cors = build_cors(&state.config.cors_allowed_origins);

    // Group the `/api` subtree: catalog at its root (/clients, /keywords,
    // /servers), yggdrasil nested at its configured prefix, and the bundle
    // manifest nested at /bundle-registry. Grouping avoids overlapping top-level
    // `/api` nests, which axum rejects. The yggdrasil public_url default is
    // `/api/yggdrasil`; we nest under the suffix after `/api` so the meta and
    // sub-paths resolve under that prefix.
    let ygg_suffix = yggdrasil_api_suffix(&state.config.yggdrasil.public_url);
    let api = Router::new()
        .merge(loontail_catalog::routes())
        .nest(&ygg_suffix, loontail_yggdrasil::routes())
        .nest("/bundle-registry", loontail_bundles::routes());

    // One admin router: admin REST + SPA, with catalog-admin and bundle-admin
    // nested beneath it. Mounted via `nest_service` so the bare `/admin/` path
    // (the SPA shell) resolves; a plain `nest` would 404 that exact path.
    let admin = loontail_admin::routes()
        .nest("/catalog", loontail_catalog::admin_routes())
        .nest("/bundles", loontail_bundles::admin_routes());

    Router::new()
        .route("/health", get(infra::health))
        .route("/metrics", get(infra::metrics_handler))
        .merge(loontail_network::routes())
        .nest("/api", api)
        .nest("/textures", loontail_textures::routes())
        // The manifest's `url` fields point at `/bundle-registry/builds/...`;
        // serve those file bytes from the configured public prefix.
        .nest("/bundle-registry", loontail_bundles::static_routes())
        .nest_service("/admin", admin.with_state(state.clone()))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

/// Derive the `/api`-relative mount suffix for yggdrasil from its configured
/// public URL. The default `/api/yggdrasil` mounts at `/yggdrasil` inside the
/// `/api` subtree; an absolute external base (e.g. `https://auth.x/api/yggdrasil`)
/// is reduced to the same suffix so the internal routes resolve identically.
fn yggdrasil_api_suffix(public_url: &str) -> String {
    let path = match public_url.find("://") {
        Some(idx) => {
            let rest = &public_url[idx + 3..];
            rest.find('/').map(|p| &rest[p..]).unwrap_or("")
        }
        None => public_url,
    };
    let trimmed = path.trim_end_matches('/');
    match trimmed.strip_prefix("/api") {
        Some(suffix) if !suffix.is_empty() => suffix.to_string(),
        // why: a bare `/api` (or empty) leaves yggdrasil at the `/api` root,
        // which would collide with catalog; fall back to the conventional slug.
        _ => "/yggdrasil".to_string(),
    }
}

/// Spawn hourly background cleanup of expired Yggdrasil token pairs and admin
/// sessions. Failures are logged and retried on the next tick — they never abort
/// the loop.
fn spawn_cleanup_tasks(state: AppState) {
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        loop {
            ticker.tick().await;
            match cleanup_expired_yggdrasil(&pool).await {
                Ok(n) if n > 0 => tracing::debug!(removed = n, "cleaned expired yggdrasil tokens"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "yggdrasil token cleanup failed"),
            }
            match cleanup_expired_admin_sessions(&pool).await {
                Ok(n) if n > 0 => tracing::debug!(removed = n, "cleaned expired admin sessions"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "admin session cleanup failed"),
            }
        }
    });
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

#[cfg(test)]
mod tests {
    use super::yggdrasil_api_suffix;

    #[test]
    fn yggdrasil_suffix_from_default_path() {
        assert_eq!(yggdrasil_api_suffix("/api/yggdrasil"), "/yggdrasil");
    }

    #[test]
    fn yggdrasil_suffix_from_absolute_url() {
        assert_eq!(
            yggdrasil_api_suffix("https://auth.loontail.com/api/yggdrasil"),
            "/yggdrasil"
        );
    }

    #[test]
    fn yggdrasil_suffix_falls_back_on_bare_api() {
        assert_eq!(yggdrasil_api_suffix("/api"), "/yggdrasil");
        assert_eq!(yggdrasil_api_suffix("/api/"), "/yggdrasil");
    }

    #[test]
    fn yggdrasil_suffix_preserves_nested_path() {
        assert_eq!(yggdrasil_api_suffix("/api/auth/ygg"), "/auth/ygg");
    }
}
