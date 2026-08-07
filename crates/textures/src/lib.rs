//! Textures domain: the skin/cape registry. Reads/writes the `user_textures` table
//! (one row per user per kind, keyed by `(user_id, kind)`), validates uploads as
//! Minecraft PNGs via `yggdrasil-protocol`, stores the bytes under
//! `config.textures.storage_root`, and serves them back from its own GET handlers.
//! Mounted by the server crate at `/textures`.

mod admin;
mod public;
mod storage;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Router;

use loontail_core::{AppState, Config};

pub use storage::TextureKind;

/// Cap an uploaded texture at 256 KiB. A valid 64x64 RGBA skin PNG is a few KiB;
/// this leaves generous headroom while bounding multipart buffering.
pub const MAX_UPLOAD_BYTES: usize = 256 * 1024;

fn relative_texture_url(profile_uuid: &str, kind: &str) -> String {
    format!("/textures/{profile_uuid}/{kind}")
}

/// Absolutize a server-relative texture URL against the configured public origin
/// (when one is set). Path-only configs leave URLs server-relative, preserving the
/// launcher's "server-relative, client absolutizes" contract.
fn absolutize_url(config: &Config, relative: &str) -> String {
    if let Some(origin) = public_origin(&config.yggdrasil.public_url) {
        format!("{origin}{relative}")
    } else {
        relative.to_string()
    }
}

/// The `scheme://host[:port]` origin of a configured public URL, or `None` for
/// path-only values (e.g. the default `/api/yggdrasil`).
fn public_origin(public_url: &str) -> Option<String> {
    loontail_core::config::parse_public_url(public_url).0
}

/// The textures domain router, nested under `/textures`.
///
/// A single dynamic `{segment}` for the top-level path avoids conflicting matchit
/// routes between the write targets (`skin`/`cape`) and the lookup UUID: GET treats
/// it as a UUID, PUT/DELETE as a kind. The body limit is raised so multipart
/// uploads up to [`MAX_UPLOAD_BYTES`] are not rejected by axum's default 2 MiB cap.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{segment}",
            get(public::lookup)
                .put(public::upload)
                .delete(public::delete),
        )
        .route("/{segment}/{kind}", get(public::read_png))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}

/// Admin moderation router for the texture registry. The server crate nests this
/// under `/admin/textures` (behind the admin cookie + CSRF double-submit).
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/skins", get(admin::list_skins))
        .route("/capes", get(admin::list_capes))
        .route("/skins/{user_id}", delete(admin::delete_skin))
        .route("/capes/{user_id}", delete(admin::delete_cape))
        .route("/orphans", get(admin::orphans))
        .route("/purge-missing", post(admin::purge_missing))
}

/// Create the on-disk storage directories (`{storage_root}/{skins,capes}`) so
/// uploads never race directory creation. Call once at server startup.
pub async fn init(config: &Config) -> std::io::Result<()> {
    storage::ensure_dirs(&config.textures.storage_root).await
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
            relative_texture_url(
                "f84c6a790a4e45e0879f0e478de5cb7e",
                TextureKind::Skin.as_str()
            ),
            "/textures/f84c6a790a4e45e0879f0e478de5cb7e/skin"
        );
        assert_eq!(
            relative_texture_url(
                "f84c6a790a4e45e0879f0e478de5cb7e",
                TextureKind::Cape.as_str()
            ),
            "/textures/f84c6a790a4e45e0879f0e478de5cb7e/cape"
        );
    }
}
