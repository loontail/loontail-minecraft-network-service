//! Shared on-disk helpers for the static-asset stores (catalog media, textures):
//! a cache-busting revision token, a parent-dir-creating write, and a
//! NotFound-tolerant unlink. Bundle storage has its own slug-guarded layout.

use std::path::Path;

use rand::RngCore;

/// A fresh 6-byte revision, hex-encoded. New on every upload so a new asset busts
/// any client/CDN cache keyed on the URL.
pub fn revision_hex() -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Write bytes, creating the parent dir on demand so a missing storage tree
/// self-heals.
pub async fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await
}

/// Best-effort unlink; a missing file is not an error, since a row's path may point
/// at an already-gone file after a crash or manual cleanup.
pub async fn unlink_quiet(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "failed to unlink stored file");
        }
    }
}
