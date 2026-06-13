//! Bundle-registry domain: builds/artifacts, ZIP ingest, manifest generation, and
//! the on-disk layout under data/bundle-registry. The public manifest router is
//! mounted by the server crate at `/api/bundle-registry`; static file serving lives
//! at `/bundle-registry/builds/{slug}/files/...`. Real handlers (and the byte-exact
//! manifest shape) land in phase 5; routes are wired now so the graph is complete.

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

use loontail_core::AppState;

/// Build the public bundle-registry router. The server crate nests this under
/// `/api/bundle-registry`.
pub fn routes() -> Router<AppState> {
    Router::new().route("/builds/{slug}/manifest", get(not_implemented))
}

// TODO(phase 5): emit the byte-exact manifest (Record<category, ManifestEntry[]>)
// the launcher hashes, with `Cache-Control: no-cache`.
async fn not_implemented() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
