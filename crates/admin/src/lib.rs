//! Admin domain: cookie-session REST for user management (Yggdrasil-bound),
//! catalog/bundle admin, API tokens, and analytics, plus serving the React SPA.
//! Mounted by the server crate at `/admin`. Real handlers and the SPA static
//! serving land in phase 6; routes are wired now so the graph is complete.

use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;

use loontail_core::AppState;

/// Build the admin domain router. The server crate nests this under `/admin`.
pub fn routes() -> Router<AppState> {
    Router::new()
        // auth
        .route("/auth/login", post(not_implemented))
        .route("/auth/logout", post(not_implemented))
        // users
        .route("/users", get(not_implemented).post(not_implemented))
        .route(
            "/users/{id}",
            get(not_implemented)
                .patch(not_implemented)
                .delete(not_implemented),
        )
        .route("/users/{id}/block", post(not_implemented))
        .route("/users/{id}/unblock", post(not_implemented))
        .route("/users/{id}/reset-password", post(not_implemented))
        .route("/users/{id}/revoke-tokens", post(not_implemented))
        // api tokens
        .route("/api-tokens", get(not_implemented).post(not_implemented))
        .route("/api-tokens/{id}", axum::routing::delete(not_implemented))
        // analytics
        .route("/analytics/overview", get(not_implemented))
        .route("/analytics/timeseries", get(not_implemented))
}

// TODO(phase 6): implement admin auth (cookie + CSRF), user CRUD bound to
// Yggdrasil, catalog/bundle admin, analytics, and SPA static serving under /admin.
async fn not_implemented() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
