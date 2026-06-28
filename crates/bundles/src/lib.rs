//! Bundle-registry domain: builds/artifacts, ZIP ingest, manifest generation, and
//! the on-disk layout under `{config.bundles.storage_root}/builds/{slug}`.
//! `routes()` mount at `/api/bundle-registry` (manifest + static files);
//! `admin_routes()` at `/admin/bundles` (`AdminUser`-guarded).

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

/// Provisioning + teardown of a build's owned bundle, reusable by the catalog (1:1
/// bundle per build). [`provision_bundle`] does the whole thing; the tx-capable parts
/// ([`provision_bundle_row`]/[`ensure_bundle_dir`], [`find_bundle_id_by_slug`],
/// [`delete_bundle_row`]/[`bundle_slug_by_id`]/[`remove_bundle_dir`]) let the catalog
/// provision and tear down a bundle inside its own transaction so deleting a build
/// never orphans its bundle.
pub use repo::{
    bundle_slug_by_id, delete_bundle_row, ensure_bundle_dir, find_bundle_id_by_slug,
    provision_bundle, provision_bundle_row, remove_bundle_dir,
};

/// Hard cap on a single bundle upload (10 GiB). Enforced both as axum's
/// `DefaultBodyLimit` and as a running byte cap while streaming each multipart field
/// to disk, so a malicious client cannot exhaust memory or disk.
pub const MAX_UPLOAD_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Public router: the manifest endpoint plus the static file routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/builds/{slug}/manifest", get(public::get_manifest))
        .merge(static_routes())
}

/// Static byte-serving of build files. Exposed separately so the server can also
/// mount it at the configured public prefix (`/bundle-registry`) where the manifest's
/// `url` fields point.
pub fn static_routes() -> Router<AppState> {
    Router::new().route("/builds/{slug}/files/{*path}", get(public::serve_file))
}

/// Admin bundle management. The upload route raises axum's body limit so a large ZIP
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
        .route("/builds/{slug}/files/move", post(admin::move_files))
        .route(
            "/builds/{slug}/files/{entryId}",
            delete(admin::delete_file).put(admin::toggle_download_once),
        )
        .route(
            "/builds/{slug}/files/{entryId}/rename",
            post(admin::rename_file),
        )
        .route(
            "/builds/{slug}/files/{entryId}/move",
            post(admin::move_file),
        )
        .route(
            "/builds/{slug}/files/{entryId}/rehash",
            post(admin::rehash_file),
        )
}

/// [`MAX_UPLOAD_BYTES`] as a `usize`, saturating on 32-bit targets where 10 GiB
/// exceeds `usize::MAX`.
fn usize_cap() -> usize {
    usize::try_from(MAX_UPLOAD_BYTES).unwrap_or(usize::MAX)
}

/// Create the bundle storage root at startup so the first upload/create doesn't race
/// a missing directory.
pub fn init(storage_root: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(storage::builds_root(storage_root))
}
