//! Serve the embedded admin SPA. The Vite build output (`admin-ui/dist`) is
//! embedded at compile time via rust-embed; a `build.rs` guarantees the folder
//! exists (placeholder index.html) so the crate always builds even without a UI
//! build. Unknown paths fall back to `index.html` for client-side routing.

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../admin-ui/dist"]
struct Spa;

fn serve(path: &str) -> Response {
    match Spa::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut res = Response::new(Body::from(file.data.into_owned()));
            if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
                res.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            res
        }
        None => serve_index(),
    }
}

fn serve_index() -> Response {
    match Spa::get("index.html") {
        Some(file) => {
            let mut res = Response::new(Body::from(file.data.into_owned()));
            res.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            res
        }
        // why: build.rs always writes a placeholder index.html, so this branch is
        // effectively unreachable; keep it explicit rather than unwrap.
        None => (StatusCode::NOT_FOUND, "admin UI not built").into_response(),
    }
}

/// Serve the SPA shell at the nest root (`/admin/`).
pub async fn index() -> Response {
    serve_index()
}

/// Router fallback: serve an embedded asset for the request path, or the SPA
/// shell for any unknown client route. The URI here is relative to the `/admin`
/// mount (axum strips the nest prefix), so e.g. `/admin/assets/x.js` arrives as
/// `/assets/x.js`.
pub async fn fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        return serve_index();
    }
    serve(path)
}
