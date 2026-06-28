//! On-disk storage for catalog media (client poster/background/titleImage/
//! screenshots). Files live under
//! `config.catalog.storage_root/{client_id}/{role}-{revision}.{ext}`. The 6-byte
//! hex revision is fresh on every upload so a new image busts any client/CDN cache
//! keyed on the URL; the previous file is unlinked when a singular slot is replaced.

use std::path::{Path, PathBuf};

use rand::RngCore;

/// A fresh 6-byte revision, hex-encoded (12 chars). New on every upload.
pub fn revision_hex() -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Build the absolute on-disk path for a media file under the storage root.
/// File name is `{role}-{revision}.{ext}`, nested under the client's UUID dir.
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

/// The server-relative URL the media is served under. The launcher absolutizes
/// it against its configured API origin; the admin SPA is same-origin.
pub fn public_url(client_id: &str, role: &str, revision: &str, ext: &str) -> String {
    format!("/catalog-media/{client_id}/{role}-{revision}.{ext}")
}

/// Create the catalog media storage root. Called once at startup so uploads never
/// race directory creation; per-client subdirs are created on demand at write.
pub async fn ensure_dir(storage_root: &str) -> std::io::Result<()> {
    tokio::fs::create_dir_all(storage_root).await
}

/// Write the media bytes to disk, returning the path written. The parent dir is
/// created on demand (idempotent) so a stale/missing storage tree self-heals.
pub async fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await
}

/// Best-effort unlink of a media file. A missing file is not an error (the row's
/// file may already be gone after a crash or manual cleanup).
pub async fn unlink_quiet(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "failed to unlink catalog media");
        }
    }
}
