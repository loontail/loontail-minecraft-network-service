//! Catalog domain: launcher clients/keywords/servers with i18n and media. The
//! public wire contract is native flat camelCase JSON (no envelopes, no
//! `populate[...]`): relations are always inlined, `id` is the entity's UUID as an
//! undashed 32-char hex string, media `url`s stay server-relative (the launcher
//! absolutizes; the admin SPA is same-origin), and the `published_at` draft filter
//! plus locale fallback are applied in `repo`. Public `routes()` mount under
//! `/api`; `admin_routes()` mount under `/admin/catalog` (AdminUser-guarded).
//! Uploaded image bytes are served from `media_routes()` (AuthUser-guarded)
//! mounted at `/catalog-media`.

mod admin;
mod dto;
mod public;
mod query;
mod repo;
mod store;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::{AppState, Config};

/// Cap a single catalog-media upload at 32 MiB. Posters/backgrounds/screenshots are
/// typically well under this; the cap bounds multipart buffering.
pub const MAX_MEDIA_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

/// The axum `DefaultBodyLimit` for the upload route, set strictly above the
/// in-handler running cap so an oversized body reaches the handler — which rejects
/// it with a friendly JSON 400 — instead of being killed by the body-limit layer as
/// an opaque non-JSON 413. The headroom covers the multipart boundary and the
/// `role` text field on top of the file bytes.
pub const MEDIA_UPLOAD_BODY_LIMIT: usize = MAX_MEDIA_UPLOAD_BYTES + 1024 * 1024;

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
        .route(
            "/clients",
            post(admin::create_client).get(admin::list_clients),
        )
        .route(
            "/clients/{id}",
            axum::routing::patch(admin::update_client).delete(admin::delete_client),
        )
        .route("/clients/{id}/publish", post(admin::publish_client))
        .route("/clients/{id}/unpublish", post(admin::unpublish_client))
        // Media: real byte upload + list + delete; the JSON attach is an escape
        // hatch for externally-hosted URLs.
        .route(
            "/clients/{id}/media",
            post(admin::attach_media).get(admin::list_media),
        )
        .route(
            "/clients/{id}/media/upload",
            post(admin::upload_media).layer(DefaultBodyLimit::max(MEDIA_UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/clients/{id}/media/{media_id}",
            axum::routing::delete(admin::delete_media),
        )
        .route(
            "/clients/{client_id}/keywords/{keyword_id}",
            post(admin::attach_keyword).delete(admin::detach_keyword),
        )
        .route(
            "/clients/{client_id}/servers/{server_id}",
            post(admin::attach_server).delete(admin::detach_server),
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

/// Media-serving router (AuthUser-guarded). The server crate mounts this top-level
/// at `/catalog-media`. The launcher's media cache attaches its Bearer token,
/// matching textures/bundles; serves uploaded image bytes from the storage root.
pub fn media_routes() -> Router<AppState> {
    Router::new().route("/{client_id}/{*path}", get(serve_media))
}

/// `GET /catalog-media/{client_id}/{*path}` — read an uploaded media file from
/// `{storage_root}/{client_id}/{path}` and return its bytes with a Content-Type
/// derived from the extension. AuthUser-guarded.
async fn serve_media(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((client_id, rel)): Path<(String, String)>,
) -> AppResult<Response> {
    // why: `client_id`/`rel` are decoded after routing, so guard against traversal
    // before any FS join.
    if client_id.is_empty() || client_id.contains('/') || client_id.contains("..") {
        return Err(AppError::NotFound("media not found".into()));
    }
    if rel.is_empty() || rel.contains("..") {
        return Err(AppError::NotFound("media not found".into()));
    }
    let root = std::path::Path::new(&state.config.catalog.storage_root).join(&client_id);
    let path = root.join(&rel);

    // why: canonicalize the per-client root and the resolved target and re-verify
    // containment. The `..`/`/` checks above already reject the obvious escapes, but
    // canonicalizing defeats symlinks and any residual escape before we read bytes.
    // Any failure (missing file, symlink escape, root absent) stays a plain 404 so we
    // never leak why.
    let canonical_root = tokio::fs::canonicalize(&root)
        .await
        .map_err(|_| AppError::NotFound("media not found".into()))?;
    let canonical_target = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| AppError::NotFound("media not found".into()))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(AppError::NotFound("media not found".into()));
    }

    let bytes: Bytes = tokio::fs::read(&canonical_target)
        .await
        .map_err(|_| AppError::NotFound("media not found".into()))?
        .into();
    let content_type = content_type_for(&rel);
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        bytes,
    )
        .into_response())
}

/// Map a file extension to a Content-Type for the image kinds we store.
fn content_type_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// Create the on-disk catalog-media storage root so uploads never race directory
/// creation. Call once at server startup.
pub async fn init(config: &Config) -> std::io::Result<()> {
    store::ensure_dir(&config.catalog.storage_root).await
}
