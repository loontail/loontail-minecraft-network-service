//! Admin domain: cookie-session REST for user management (Yggdrasil-bound), API
//! tokens, and live analytics, plus serving the embedded React SPA. Mounted by
//! the server crate at `/admin`.
//!
//! Auth model: `POST /auth/login` verifies credentials, requires `is_admin`, and
//! sets an httpOnly session cookie plus a readable CSRF cookie. Mutations require
//! the `AdminUser` extractor and pass the CSRF double-submit check
//! (`x-csrf-token` header == `loontail_csrf` cookie). Reads require only the
//! session.

pub mod analytics;
pub mod auth;
pub mod cookies;
pub mod dto;
pub mod spa;
pub mod startup;
pub mod tokens;
pub mod users;

use axum::routing::{get, patch, post};
use axum::Router;

use loontail_core::AppState;

pub use startup::ensure_bootstrap_admin;
pub use tokens::verify_api_token;

/// Build the admin domain router. The server crate mounts this under `/admin`.
///
/// Mount with `nest_service` (not `nest`) so the SPA shell answers the bare
/// `/admin/` request; a plain `nest` 404s that exact trailing-slash path:
///
/// ```ignore
/// router.nest_service("/admin", loontail_admin::routes().with_state(state))
/// ```
///
/// REST routes take precedence over the SPA; any remaining `GET` (the shell at
/// `/`, embedded assets, and unknown client routes) falls back to the SPA.
pub fn routes() -> Router<AppState> {
    let api = Router::new()
        // auth
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        // users
        .route("/users", get(users::list).post(users::create))
        .route(
            "/users/{id}",
            get(users::get).patch(users::patch).delete(users::delete),
        )
        .route("/users/{id}/block", post(users::block_user))
        .route("/users/{id}/unblock", post(users::unblock_user))
        .route("/users/{id}/reset-password", post(users::reset_password))
        .route("/users/{id}/revoke-tokens", post(users::revoke_tokens))
        // api tokens
        .route("/api-tokens", get(tokens::list).post(tokens::create))
        .route(
            "/api-tokens/{id}",
            patch(tokens::update).delete(tokens::delete),
        )
        // analytics
        .route("/analytics/overview", get(analytics::overview))
        .route("/analytics/timeseries", get(analytics::timeseries));

    // The SPA shell + embedded assets. An explicit `/` route serves the nest-root
    // (a router fallback alone does not match the bare `/admin/` under a nest in
    // axum 0.8), and the fallback covers assets and unknown client routes. The
    // navigation guard serves the shell for browser page loads even when a REST
    // route shadows the same path (e.g. `GET /admin/users`), so client routes deep
    // link and survive a refresh while the SPA's JSON `fetch` calls pass through.
    api.route("/", get(spa::index))
        .fallback(spa::fallback)
        .layer(axum::middleware::from_fn(spa::navigation_guard))
}
