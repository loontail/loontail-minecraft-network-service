//! Textures domain: the skin/cape registry. Reads/writes the `skins` and `capes`
//! tables (one row per user, keyed by `user_id`), validates uploads as Minecraft
//! PNGs via `yggdrasil-protocol`, stores the bytes under
//! `config.textures.storage_root`, and serves them back from its own GET handlers.
//! Mounted by the server crate at `/textures`.

mod handlers;
mod store;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::Router;

use loontail_core::{AppState, Config};

pub use store::Kind;

/// Cap an uploaded texture at 256 KiB. A valid 64x64 RGBA skin PNG is a few KiB;
/// this leaves generous headroom while bounding multipart buffering.
pub const MAX_UPLOAD_BYTES: usize = 256 * 1024;

/// The URL slug each kind is served under (`/textures/{uuid}/skin|cape`). The
/// stored `file_url` is server-relative; the lookup handler absolutizes it.
fn relative_texture_url(profile_uuid: &str, kind: Kind) -> String {
    format!("/textures/{profile_uuid}/{}", kind.slug())
}

/// Absolutize a server-relative texture URL against the configured public base
/// (`config.yggdrasil.public_url`'s origin, when it carries one). Absolute URLs
/// and bases without a scheme are passed through so a path-only deployment keeps
/// the launcher's "server-relative, client absolutizes" contract intact.
fn absolutize_url(config: &Config, relative: &str) -> String {
    if let Some(origin) = public_origin(&config.yggdrasil.public_url) {
        format!("{origin}{relative}")
    } else {
        relative.to_string()
    }
}

/// Extract the `scheme://host[:port]` origin from a configured public URL. Returns
/// `None` for path-only values (e.g. the default `/api/yggdrasil`), in which case
/// texture URLs stay server-relative.
fn public_origin(public_url: &str) -> Option<String> {
    let scheme_end = public_url.find("://")?;
    let after_scheme = scheme_end + 3;
    let rest = &public_url[after_scheme..];
    let authority_len = rest.find('/').unwrap_or(rest.len());
    if authority_len == 0 {
        return None;
    }
    Some(public_url[..after_scheme + authority_len].to_string())
}

/// Build the textures domain router. The server crate nests this under `/textures`.
///
/// A single dynamic `{segment}` is used for the top-level path so the literal
/// write targets (`skin`/`cape`) and the lookup UUID never declare conflicting
/// routes in matchit: GET treats `{segment}` as a UUID, PUT/DELETE treat it as a
/// kind. The PNG read path is `{segment}/{kind}`. PUT routes raise the body limit
/// so multipart uploads up to [`MAX_UPLOAD_BYTES`] are not rejected by axum's
/// default 2 MiB cap (we enforce our own cap while reading the field).
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{segment}",
            get(handlers::lookup)
                .put(handlers::upload)
                .delete(handlers::delete),
        )
        .route("/{segment}/{kind}", get(handlers::read_png))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}

/// Create the on-disk storage directories (`{storage_root}/{skins,capes}`) so
/// uploads never race directory creation. Call once at server startup.
pub async fn init(config: &Config) -> std::io::Result<()> {
    store::ensure_dirs(&config.textures.storage_root).await
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn origin_parsed_from_full_url() {
        assert_eq!(
            public_origin("https://auth.loontail.com/api/yggdrasil"),
            Some("https://auth.loontail.com".to_string())
        );
        assert_eq!(
            public_origin("http://localhost:8080/api/yggdrasil"),
            Some("http://localhost:8080".to_string())
        );
        // No trailing path still yields the bare origin.
        assert_eq!(
            public_origin("https://auth.loontail.com"),
            Some("https://auth.loontail.com".to_string())
        );
    }

    #[test]
    fn path_only_public_url_has_no_origin() {
        assert_eq!(public_origin("/api/yggdrasil"), None);
        assert_eq!(public_origin(""), None);
    }

    #[test]
    fn relative_url_shape() {
        assert_eq!(
            relative_texture_url("f84c6a790a4e45e0879f0e478de5cb7e", Kind::Skin),
            "/textures/f84c6a790a4e45e0879f0e478de5cb7e/skin"
        );
        assert_eq!(
            relative_texture_url("f84c6a790a4e45e0879f0e478de5cb7e", Kind::Cape),
            "/textures/f84c6a790a4e45e0879f0e478de5cb7e/cape"
        );
    }
}
