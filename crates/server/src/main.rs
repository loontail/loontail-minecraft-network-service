mod infra;
mod ip;
mod ratelimit;
mod request_log_capture;

use std::time::Duration;

use axum::http::header::{
    HeaderName, CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY,
    X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderValue, Method};
use axum::routing::get;
use axum::{middleware, Router, ServiceExt};
use tower::Layer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use loontail_core::auth::{cleanup_expired_sessions, cleanup_expired_yggdrasil};
use loontail_core::db;
use loontail_core::request_log::delete_request_logs_older_than;
use loontail_core::{AppState, Config};
use loontail_network::cleanup::cleanup_stale_join_state;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Local `.env` is optional; in deployment the env comes from compose.
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(
        http = %format!("{}:{}", config.http_host, config.http_port),
        "starting loontail-launcher-api"
    );
    warn_on_relative_yggdrasil_public_url(&config);

    // Load/generate the signing key before serving so signed-texture handlers never 500.
    loontail_yggdrasil::init_crypto(&config)?;
    // Create storage roots up front so the first upload/create never races a missing dir.
    loontail_textures::init(&config).await?;
    loontail_catalog::init(&config).await?;
    loontail_bundles::init(&config.bundles.storage_root)?;

    let pool = db::connect_and_migrate(&config.database_url).await?;
    tracing::info!("database connected and migrations applied");

    // A restart drops every live relay/WebSocket connection, but their DB rows
    // linger as 'active'/'pending'. Reconcile so stale sessions don't keep guests
    // falsely "in world" (presence + friend-of-friend membership) or inflate
    // current_players.
    db::reconcile_after_restart(&pool).await?;

    loontail_admin::ensure_bootstrap_admin(&pool, &config.admin).await?;

    let addr = format!("{}:{}", config.http_host, config.http_port);
    let state = AppState::new(pool, config);

    spawn_cleanup_tasks(state.clone());

    let app = build_router(state);

    // Trim a trailing slash before routing so client variants like
    // `/api/yggdrasil/` resolve to the nested router's root meta route the same as
    // `/api/yggdrasil`. Must be applied outside the router (matchit runs after the trim).
    let app = NormalizePathLayer::trim_trailing_slash().layer(app);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    // `into_make_service_with_connect_info` injects the peer `SocketAddr` so the
    // rate-limit middleware can resolve the client IP via `ConnectInfo`.
    axum::serve(
        listener,
        ServiceExt::<axum::extract::Request>::into_make_service_with_connect_info::<
            std::net::SocketAddr,
        >(app),
    )
    .await?;

    Ok(())
}

fn build_router(state: AppState) -> Router {
    let cors = build_cors(&state.config.cors_allowed_origins);

    // Grouping the `/api` subtree avoids overlapping top-level `/api` nests, which
    // axum rejects. Yggdrasil nests under the suffix after `/api` so its meta and
    // sub-paths resolve under that prefix.
    let ygg_suffix = yggdrasil_api_suffix(&state.config.yggdrasil.public_url);
    let limiter = ratelimit::RateLimiter::from_config(
        &state.config.rate_limit,
        state.config.trusted_proxy,
        &yggdrasil_mount_prefix(&ygg_suffix),
    );
    let api = Router::new()
        .merge(loontail_catalog::routes())
        .nest("/auth", loontail_yggdrasil::account_routes())
        // Textures are canonically mounted top-level at `/textures` (below), but the
        // launcher's vendored Yggdrasil client derives its texture base from the
        // Yggdrasil `apiRoot` and calls `<ygg_suffix>/textures/*`. A second mount
        // inside the yggdrasil subtree keeps already-shipped launchers working;
        // lookup responses stay server-relative `/textures/...` so the PNG bytes
        // still resolve top-level.
        .nest(
            &ygg_suffix,
            loontail_yggdrasil::routes().nest("/textures", loontail_textures::routes()),
        )
        .nest("/bundle-registry", loontail_bundles::routes());

    // One admin router: admin REST + SPA, with catalog-admin and bundle-admin
    // nested beneath it. Mounted via `nest_service` so the bare `/admin/` path
    // (the SPA shell) resolves; a plain `nest` would 404 that exact path.
    // Security headers are scoped to this subtree so they never alter
    // Yggdrasil/bundle/texture file responses served elsewhere.
    let admin = with_security_headers(
        loontail_admin::routes()
            .nest("/catalog", loontail_catalog::admin_routes())
            .nest("/bundles", loontail_bundles::admin_routes())
            .nest("/textures", loontail_textures::admin_routes()),
    );

    Router::new()
        .route("/health", get(infra::health))
        .route("/metrics", get(infra::metrics_handler))
        .merge(loontail_network::routes())
        .nest("/api", api)
        .nest("/textures", loontail_textures::routes())
        // Uploaded catalog-media bytes; stored `url` fields point here. AuthUser-guarded.
        .nest("/catalog-media", loontail_catalog::media_routes())
        // The manifest's `url` fields point at `/bundle-registry/builds/...`.
        .nest("/bundle-registry", loontail_bundles::static_routes())
        .nest_service("/admin", admin.with_state(state.clone()))
        // Capture every served request (minus probes + WS upgrades) into the
        // observability ring + request_logs. Applied before `.with_state` so it
        // reads the matched-route template from extensions and shares `AppState`.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            request_log_capture::capture,
        ))
        .with_state(state)
        // The limiter self-filters to unauthenticated credential paths by URI; it
        // sits outside the routers so it sees the original request path.
        .layer(middleware::from_fn_with_state(
            limiter,
            ratelimit::rate_limit_middleware,
        ))
        .layer(cors)
        // SEC-8: `nosniff` everywhere (not just /admin) so an uploaded `.html`/`.svg`
        // served from textures/catalog-media/bundle routes can't be MIME-sniffed and
        // rendered in-origin. The stricter CSP/frame/HSTS headers stay admin-scoped
        // (see `with_security_headers`). `overriding` is a no-op on the admin subtree
        // (it already sets the same value).
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        // SEC-10: redact the query string before it reaches the tracing span so a
        // WS `?token=` (or any `?token=` fallback) never lands in logs/proxy access
        // logs.
        .layer(TraceLayer::new_for_http().make_span_with(make_redacted_span))
}

/// Build the per-request tracing span WITHOUT the query string, so a `?token=`
/// fallback on the WS upgrades (`/signaling`, `/relay/:id`) cannot leak into logs.
fn make_redacted_span(request: &axum::http::Request<axum::body::Body>) -> tracing::Span {
    tracing::info_span!(
        "request",
        method = %request.method(),
        // Path only — never `request.uri()`, which includes the query.
        path = %request.uri().path(),
        version = ?request.version(),
    )
}

/// Apply baseline security response headers to `router`. Scoped to the admin
/// subtree by the caller, since admin serves the only browser-facing HTML; the
/// CSP `frame-ancestors 'none'` + `X-Frame-Options: DENY` block clickjacking and
/// `nosniff` blocks MIME confusion without touching binary file responses
/// (Yggdrasil/bundle/texture) served elsewhere.
fn with_security_headers(router: Router<AppState>) -> Router<AppState> {
    let csp: HeaderName = CONTENT_SECURITY_POLICY;
    router
        .layer(SetResponseHeaderLayer::overriding(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            csp,
            // `style-src 'unsafe-inline'`: the admin SPA's UI primitives (react-aria
            // file-tree drag previews/drop indicators, theme/animation helpers) apply
            // inline styles; scripts stay locked to 'self' (no script unsafe-inline).
            HeaderValue::from_static(
                "default-src 'self'; style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; frame-ancestors 'none'; base-uri 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
}

/// Derive the `/api`-relative mount suffix for yggdrasil from its configured
/// public URL. The default `/api/yggdrasil` mounts at `/yggdrasil` inside the
/// `/api` subtree; an absolute external base (e.g. `https://auth.x/api/yggdrasil`)
/// is reduced to the same suffix so the internal routes resolve identically.
fn yggdrasil_api_suffix(public_url: &str) -> String {
    let (_, path) = loontail_core::config::parse_public_url(public_url);
    let trimmed = path.trim_end_matches('/');
    match trimmed.strip_prefix("/api") {
        Some(suffix) if !suffix.is_empty() => suffix.to_string(),
        // why: a bare `/api` (or empty) leaves yggdrasil at the `/api` root,
        // which would collide with catalog; fall back to the conventional slug.
        _ => "/yggdrasil".to_string(),
    }
}

/// `true` when signed profiles built against `public_url` will carry absolute (thus
/// fetchable) texture URLs — i.e. the configured value has a scheme and host.
fn texture_urls_will_be_absolute(public_url: &str) -> bool {
    loontail_core::config::parse_public_url(public_url)
        .0
        .is_some()
}

/// CON-01: a `YGGDRASIL_PUBLIC_URL` with no scheme+host makes every signed profile
/// carry a server-relative texture URL that no game client can fetch. This is not
/// fatal — the test suite and local `docker compose up` run on the path-only default
/// on purpose — but it must never fail silently in a real deployment.
fn warn_on_relative_yggdrasil_public_url(config: &Config) {
    let public_url = &config.yggdrasil.public_url;
    if !texture_urls_will_be_absolute(public_url) {
        tracing::error!(
            %public_url,
            "YGGDRASIL_PUBLIC_URL is not an absolute URL — signed profiles will advertise \
             server-relative texture URLs and IN-GAME SKINS AND CAPES WILL NOT LOAD. Set it \
             to https://<public-host>/api/yggdrasil (and list that host in \
             YGGDRASIL_SKIN_DOMAINS)."
        );
    }
}

/// The absolute path the Yggdrasil router is served at, from the very `api_suffix`
/// passed to `.nest` inside the `/api` subtree. Everything that must match the mount
/// (the rate limiter's credential paths) derives it here so the two cannot diverge.
fn yggdrasil_mount_prefix(api_suffix: &str) -> String {
    format!("/api{api_suffix}")
}

/// Spawn hourly background cleanup of expired Yggdrasil token pairs, admin
/// sessions, aged request logs, and stale join/invite/event rows. Failures are logged
/// and retried on the next tick — they never abort the loop.
fn spawn_cleanup_tasks(state: AppState) {
    let pool = state.pool.clone();
    let retention_days = state.config.request_log.retention_days;
    let event_retention_days = state.config.event_retention.retention_days;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        loop {
            ticker.tick().await;
            match cleanup_expired_yggdrasil(&pool).await {
                Ok(n) if n > 0 => tracing::debug!(removed = n, "cleaned expired yggdrasil tokens"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "yggdrasil token cleanup failed"),
            }
            match cleanup_expired_sessions(&pool).await {
                Ok(n) if n > 0 => tracing::debug!(removed = n, "cleaned expired sessions"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "session cleanup failed"),
            }
            match delete_request_logs_older_than(&pool, retention_days).await {
                Ok(n) if n > 0 => tracing::debug!(removed = n, "cleaned aged request logs"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "request-log cleanup failed"),
            }
            match cleanup_stale_join_state(&pool, event_retention_days).await {
                Ok(counts) if counts.total() > 0 => {
                    tracing::debug!(?counts, "cleaned stale join/invite/event rows");
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "join-state cleanup failed"),
            }
        }
    });
}

/// Build the CORS layer from the configured origin allowlist.
///
/// Hardened against fail-open: methods and headers are explicit allowlists (no
/// `Any`), credentials are enabled ONLY for an explicit origin allowlist, and
/// `allow_origin(Any)` is used only when `*` is explicitly configured (in which
/// case credentials are never combined with it, per the Fetch spec). An empty
/// list blocks every browser cross-origin caller, which we log at startup.
fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let methods = [
        Method::GET,
        Method::POST,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let headers: [HeaderName; 3] = [
        axum::http::header::AUTHORIZATION,
        axum::http::header::CONTENT_TYPE,
        HeaderName::from_static("x-csrf-token"),
    ];
    let base = CorsLayer::new()
        .allow_methods(methods)
        .allow_headers(headers);

    if allowed_origins.iter().any(|origin| origin == "*") {
        // Wildcard is opt-in only; credentials MUST NOT be combined with `Any`.
        base.allow_origin(Any)
    } else {
        if allowed_origins.is_empty() {
            tracing::warn!(
                "CORS_ALLOWED_ORIGINS is empty; all browser cross-origin requests will be blocked. \
                 Set it to a comma-separated origin allowlist (or `*` to allow any, credentials disabled)."
            );
        }
        let origins: Vec<HeaderValue> = allowed_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        base.allow_origin(origins).allow_credentials(true)
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,loontail_launcher_api=debug,loontail_network=debug,loontail_core=debug",
        )
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

#[cfg(test)]
mod tests {
    use super::{texture_urls_will_be_absolute, yggdrasil_api_suffix, yggdrasil_mount_prefix};

    /// CON-01: the committed default is exactly the shape that breaks in-game skins,
    /// so startup must be able to detect it.
    #[test]
    fn relative_public_url_is_detected_as_unfetchable() {
        assert!(!texture_urls_will_be_absolute("/api/yggdrasil"));
        assert!(!texture_urls_will_be_absolute(""));
        assert!(texture_urls_will_be_absolute(
            "https://cms.loontail.dev/api/yggdrasil"
        ));
        assert!(texture_urls_will_be_absolute("http://localhost:8080"));
    }

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

    /// BE-03: the limiter's prefix is the mount, for every supported public-URL shape.
    #[test]
    fn mount_prefix_tracks_the_configured_public_url() {
        for public_url in [
            "/api/yggdrasil",
            "https://auth.loontail.dev/api/yggdrasil",
            "/api/mojang",
            "/api/auth/ygg",
            "https://x",
        ] {
            let suffix = yggdrasil_api_suffix(public_url);
            assert_eq!(
                yggdrasil_mount_prefix(&suffix),
                format!("/api{suffix}"),
                "{public_url}"
            );
        }
        assert_eq!(
            yggdrasil_mount_prefix(&yggdrasil_api_suffix("/api/mojang")),
            "/api/mojang"
        );
    }
}
