//! Serve the embedded admin SPA. The Vite build output (`admin-ui/dist`) is
//! embedded at compile time via rust-embed; a `build.rs` guarantees the folder
//! exists (placeholder index.html) so the crate always builds even without a UI
//! build. Unknown paths fall back to `index.html` for client-side routing.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::Next;
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

/// Serve the SPA shell for browser page navigations so client-side routes deep
/// link and survive a refresh — even when a REST route shadows the same path
/// (e.g. `GET /admin/users`, which is both a JSON list endpoint and an SPA
/// route). A request is a page navigation when it is a `GET` whose `Accept`
/// prefers `text/html`; the SPA's own `fetch` calls send `Accept:
/// application/json` and so fall through to the JSON handlers untouched. Embedded
/// asset requests (scripts/styles) never send `text/html`, so they are likewise
/// unaffected.
pub async fn navigation_guard(request: Request, next: Next) -> Response {
    if request.method() == Method::GET && wants_html(&request) {
        return serve_index();
    }
    next.run(request).await
}

fn wants_html(request: &Request) -> bool {
    request
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
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
