//! On-disk storage for skin/cape PNGs. Files live under
//! `config.textures.storage_root/{skins|capes}/{profile_uuid}-{6byte-hex}.png`.
//! The 6-byte hex suffix is a fresh revision on every upload so a new texture
//! busts any client/CDN cache keyed on the URL; the previous file is unlinked.

use std::path::{Path, PathBuf};

use rand::RngCore;

/// The two texture kinds, used to pick the storage subdirectory and the URL slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Skin,
    Cape,
}

impl Kind {
    /// Storage subdirectory (`skins` / `capes`).
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Skin => "skins",
            Kind::Cape => "capes",
        }
    }

    /// URL slug used by the PNG read endpoint (`/textures/{uuid}/{slug}`).
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Skin => "skin",
            Kind::Cape => "cape",
        }
    }

    /// Parse the trailing path segment (`skin` / `cape`) into a kind.
    pub fn parse(segment: &str) -> Option<Kind> {
        match segment {
            "skin" => Some(Kind::Skin),
            "cape" => Some(Kind::Cape),
            _ => None,
        }
    }
}

/// A fresh 6-byte revision, hex-encoded (12 chars). New on every upload.
pub fn revision_hex() -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Build the absolute on-disk path for a texture file under the storage root.
/// File name is `{profile_uuid}-{revision}.png` (profile_uuid is undashed lc).
pub fn disk_path(storage_root: &str, kind: Kind, profile_uuid: &str, revision: &str) -> PathBuf {
    Path::new(storage_root)
        .join(kind.dir())
        .join(format!("{profile_uuid}-{revision}.png"))
}

/// Create the `{skins,capes}` subdirectories under the storage root. Called once
/// at startup so uploads never race directory creation.
pub async fn ensure_dirs(storage_root: &str) -> std::io::Result<()> {
    for kind in [Kind::Skin, Kind::Cape] {
        let dir = Path::new(storage_root).join(kind.dir());
        tokio::fs::create_dir_all(&dir).await?;
    }
    Ok(())
}

/// Write the texture bytes to disk, returning the path written. The parent dir is
/// created on demand (idempotent) so a stale/missing storage tree self-heals.
pub async fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await
}

/// Best-effort unlink of a previous revision. A missing file is not an error
/// (the row's `file_path` may point at an already-gone file after a crash).
pub async fn unlink_quiet(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "failed to unlink old texture");
        }
    }
}
