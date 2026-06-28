//! Admin domain: cookie-session REST for user management, API tokens, and live
//! analytics, plus the embedded React SPA, mounted at `/admin`.
//!
//! Auth model: login requires `is_admin` and sets an httpOnly session cookie plus
//! a readable CSRF cookie. Mutations need the `AdminUser` extractor and pass the
//! CSRF double-submit check (`x-csrf-token` header == `loontail_csrf` cookie);
//! reads need only the session.

pub mod analytics;
pub mod auth;
pub mod cookies;
pub mod dto;
pub mod requests;
pub mod spa;
pub mod startup;
pub mod users;

use axum::routing::{get, post};
use axum::Router;

use loontail_core::AppState;

pub use startup::ensure_bootstrap_admin;

/// Mount with `nest_service` (not `nest`) so the SPA shell answers the bare
/// `/admin/` request; a plain `nest` 404s that exact trailing-slash path:
///
/// ```ignore
/// router.nest_service("/admin", loontail_admin::routes().with_state(state))
/// ```
pub fn routes() -> Router<AppState> {
    let api = Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/users", get(users::list).post(users::create))
        .route(
            "/users/{id}",
            get(users::get).patch(users::patch).delete(users::delete),
        )
        .route("/users/{id}/block", post(users::block_user))
        .route("/users/{id}/unblock", post(users::unblock_user))
        .route("/users/{id}/reset-password", post(users::reset_password))
        .route("/users/{id}/revoke-tokens", post(users::revoke_tokens))
        .route("/analytics/overview", get(analytics::overview))
        .route("/analytics/timeseries", get(analytics::timeseries))
        .route("/logs/tail", get(requests::tail))
        .route("/analytics/requests", get(requests::list))
        .route("/analytics/requests/summary", get(requests::summary))
        .route("/analytics/requests/timeseries", get(requests::timeseries));

    // An explicit `/` route is needed because in axum 0.8 a fallback alone does
    // not match the bare `/admin/` under a nest. The navigation guard serves the
    // shell for browser page loads even where a REST route shadows the path (e.g.
    // `GET /admin/users`), so client routes deep link and survive a refresh while
    // JSON `fetch` calls pass through.
    api.route("/", get(spa::index))
        .fallback(spa::fallback)
        .layer(axum::middleware::from_fn(spa::navigation_guard))
}
