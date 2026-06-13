//! Catalog domain: launcher clients/keywords/servers with i18n and media,
//! 1:1 compatible with the launcher's Strapi contract (`{data,meta:{pagination}}`
//! envelope, `populate[field]=true` parsing, `publishedAt` draft filter, locale
//! fallback, server-relative media URLs). Mounted by the server crate at `/api`.
//! Real handlers land in phase 4; routes are wired now so the graph is complete.

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

use loontail_core::AppState;

/// Build the catalog domain router. The server crate nests this under `/api`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/clients", get(not_implemented))
        .route("/clients/{id}", get(not_implemented))
        .route("/keywords", get(not_implemented))
        .route("/servers", get(not_implemented))
}

// TODO(phase 4): implement Strapi-compatible list/get with populate parsing, locale
// fallback, draft filter, and the {data,meta:{pagination}} envelope.
async fn not_implemented() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
