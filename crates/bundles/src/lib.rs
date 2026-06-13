//! Bundle-registry domain: builds/artifacts, ZIP ingest, manifest generation, and
//! the on-disk layout under `{config.bundles.storage_root}/builds/{slug}`.
//!
//! Public surface (mounted by the server at `/api/bundle-registry`):
//! `GET /builds/{slug}/manifest` serves `artifacts.json` verbatim. Static file
//! bytes are served from `static_routes()` (also merged into `routes()`) at
//! `/bundle-registry/builds/{slug}/files/{*path}`. Admin operations live in
//! `admin_routes()`, mounted at `/admin/bundles` and `AdminUser`-guarded.

mod admin;
mod archive;
mod manifest;
mod models;
mod public;
mod repo;
mod storage;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Router;

use loontail_core::AppState;

/// Hard cap on a single bundle upload (ZIP archive or individual file): 10 GiB,
/// matching the archive's uncompressed ceiling. Enforced both as axum's
/// `DefaultBodyLimit` on the route and as a running byte cap while streaming each
/// multipart field to disk, so a malicious client cannot exhaust memory or disk.
pub const MAX_UPLOAD_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Public bundle-registry router. Mounted by the server at `/api/bundle-registry`.
/// Includes the manifest endpoint and the static file routes so a single mount
/// covers the whole public surface.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/builds/{slug}/manifest", get(public::get_manifest))
        .merge(static_routes())
}

/// Static byte-serving of build files at `/builds/{slug}/files/{*path}`. Exposed
/// separately so the server can also mount it at the configured public prefix
/// (`/bundle-registry`) where the manifest's `url` fields point.
pub fn static_routes() -> Router<AppState> {
    Router::new().route("/builds/{slug}/files/{*path}", get(public::serve_file))
}

/// Admin (AdminUser-guarded) bundle management. Mounted by the server at
/// `/admin/bundles`. The upload route disables axum's body limit so a large ZIP
/// streams to disk instead of being rejected or buffered whole.
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/disk-space", get(admin::disk_space_handler))
        .route("/builds", get(admin::list).post(admin::create))
        .route(
            "/builds/{slug}",
            get(admin::get).put(admin::update).delete(admin::delete),
        )
        .route(
            "/builds/{slug}/upload",
            post(admin::upload_archive).layer(DefaultBodyLimit::max(usize_cap())),
        )
        .route(
            "/builds/{slug}/regenerate",
            post(admin::regenerate_manifest),
        )
        .route("/builds/{slug}/validate", post(admin::validate))
        .route(
            "/builds/{slug}/files",
            post(admin::upload_file).layer(DefaultBodyLimit::max(usize_cap())),
        )
        .route("/builds/{slug}/folders", post(admin::create_folder))
        .route("/builds/{slug}/files/bulk-delete", post(admin::bulk_delete))
        .route(
            "/builds/{slug}/files/{entryId}",
            delete(admin::delete_file).put(admin::toggle_download_once),
        )
        .route(
            "/builds/{slug}/files/{entryId}/rename",
            post(admin::rename_file),
        )
        .route(
            "/builds/{slug}/files/{entryId}/rehash",
            post(admin::rehash_file),
        )
}

/// [`MAX_UPLOAD_BYTES`] as a `usize`, saturating on 32-bit targets where 10 GiB
/// exceeds `usize::MAX` (axum's `DefaultBodyLimit::max` takes a `usize`).
fn usize_cap() -> usize {
    usize::try_from(MAX_UPLOAD_BYTES).unwrap_or(usize::MAX)
}

/// Create the bundle storage root (`{storage_root}/builds`) at startup so the
/// first upload/create doesn't race a missing directory.
pub fn init(storage_root: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(storage::builds_root(storage_root))
}
