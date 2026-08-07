//! On-disk storage for catalog media. Files live under
//! `config.catalog.storage_root/{client_id}/{role}-{revision}.{ext}`. The revision
//! is fresh on every upload so a new image busts any URL-keyed cache.

use std::path::{Path, PathBuf};

pub use loontail_core::storage::{revision_hex, unlink_quiet, write_file};

/// Absolute on-disk path for a media file: `{root}/{client_id}/{role}-{revision}.{ext}`.
pub fn disk_path(
    storage_root: &str,
    client_id: &str,
    role: &str,
    revision: &str,
    ext: &str,
) -> PathBuf {
    Path::new(storage_root)
        .join(client_id)
        .join(format!("{role}-{revision}.{ext}"))
}

/// The server-relative URL the media is served under (the launcher absolutizes it
/// against its configured API origin; the admin SPA is same-origin).
pub fn public_url(client_id: &str, role: &str, revision: &str, ext: &str) -> String {
    format!("/catalog-media/{client_id}/{role}-{revision}.{ext}")
}

/// Create the catalog media storage root at startup; per-client subdirs are created
/// on demand at write.
pub async fn ensure_dir(storage_root: &str) -> std::io::Result<()> {
    tokio::fs::create_dir_all(storage_root).await
}
