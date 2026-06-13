//! Yggdrasil online-mode domain: `/authserver`, `/sessionserver`, `/api/profiles`
//! and the `/` meta document, plus RSA-SHA1 (PKCS#1 v1.5) texture-property signing.
//! Mounted by the server crate at the configured public URL (e.g.
//! `/api/yggdrasil`). Real handlers land in phase 3; routes are wired now so the
//! workspace graph is complete.

use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;

use loontail_core::AppState;

/// Build the Yggdrasil domain router. The server crate nests this under the
/// configured public URL prefix.
pub fn routes() -> Router<AppState> {
    Router::new()
        // authserver
        .route("/authserver/authenticate", post(not_implemented))
        .route("/authserver/refresh", post(not_implemented))
        .route("/authserver/validate", post(not_implemented))
        .route("/authserver/invalidate", post(not_implemented))
        // sessionserver
        .route(
            "/sessionserver/session/minecraft/join",
            post(not_implemented),
        )
        .route(
            "/sessionserver/session/minecraft/hasJoined",
            get(not_implemented),
        )
        .route(
            "/sessionserver/session/minecraft/profile/{uuid}",
            get(not_implemented),
        )
        // profiles + meta
        .route("/api/profiles/minecraft", post(not_implemented))
        .route("/", get(not_implemented))
}

// TODO(phase 3): replace with real authserver/sessionserver/meta handlers and the
// RSA-SHA1 signing service backed by data/yggdrasil/keys/active.key.pem.
async fn not_implemented() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
