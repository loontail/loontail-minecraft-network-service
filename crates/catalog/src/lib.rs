//! Catalog domain: launcher clients/keywords/servers with i18n and media,
//! 1:1 compatible with the launcher's Strapi contract (`{data,meta:{pagination}}`
//! envelope, `populate[field]=true` parsing, `publishedAt` draft filter, locale
//! fallback, server-relative media URLs). Public `routes()` mount under `/api`;
//! `admin_routes()` mount under `/admin/catalog` (AdminUser-guarded).

mod admin;
mod apitoken;
mod dto;
mod public;
mod query;
mod repo;

pub use apitoken::{authorize_public_read, hash_api_token, verify_api_token};

use axum::routing::{get, post};
use axum::Router;

use loontail_core::AppState;

/// Public catalog router. The server crate nests this under `/api`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/clients", get(public::list_clients))
        .route("/clients/{id}", get(public::get_client))
        .route("/keywords", get(public::list_keywords))
        .route("/keywords/{id}", get(public::get_keyword))
        .route("/servers", get(public::list_servers))
        .route("/servers/{id}", get(public::get_server))
}

/// Admin catalog router (AdminUser-guarded). The server crate nests this under
/// `/admin/catalog`.
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        // Clients
        .route("/clients", post(admin::create_client))
        .route(
            "/clients/{id}",
            axum::routing::patch(admin::update_client).delete(admin::delete_client),
        )
        .route("/clients/{id}/publish", post(admin::publish_client))
        .route("/clients/{id}/unpublish", post(admin::unpublish_client))
        .route("/clients/{id}/media", post(admin::attach_media))
        .route(
            "/clients/{client_id}/keywords/{keyword_id}",
            post(admin::attach_keyword),
        )
        .route(
            "/clients/{client_id}/servers/{server_id}",
            post(admin::attach_server),
        )
        // Keywords
        .route("/keywords", post(admin::create_keyword))
        .route(
            "/keywords/{id}",
            axum::routing::delete(admin::delete_keyword),
        )
        .route("/keywords/{id}/publish", post(admin::publish_keyword))
        .route("/keywords/{id}/unpublish", post(admin::unpublish_keyword))
        // Servers
        .route("/servers", post(admin::create_server))
        .route(
            "/servers/{id}",
            axum::routing::patch(admin::update_server).delete(admin::delete_server),
        )
        .route("/servers/{id}/publish", post(admin::publish_server))
        .route("/servers/{id}/unpublish", post(admin::unpublish_server))
}
