//! Textures domain: skin/cape registry, PNG validation (via yggdrasil-protocol),
//! and static serving. Mounted by the server crate at `/textures`. Real handlers
//! land in phase 3; routes are wired now so the workspace graph is complete.

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

use loontail_core::AppState;

/// Build the textures domain router. The server crate nests this under `/textures`.
pub fn routes() -> Router<AppState> {
    // `{segment}` is `uuid` for reads and `skin|cape` for writes; the same param
    // name on the shared single-segment path keeps axum's router happy (GET vs
    // PUT/DELETE disambiguate). Real handlers branch on method + value.
    Router::new()
        .route(
            "/{segment}",
            get(not_implemented)
                .put(not_implemented)
                .delete(not_implemented),
        )
        .route("/{segment}/{kind}", get(not_implemented))
}

// TODO(phase 3): implement texture lookup, multipart upload (PNG-validated), delete,
// and on-disk static serving under data/textures/{skins|capes}.
async fn not_implemented() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
