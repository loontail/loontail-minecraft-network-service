//! On-disk storage for skin/cape PNGs. Files live under
//! `config.textures.storage_root/{skins|capes}/{profile_uuid}-{6byte-hex}.png`.
//! The 6-byte hex suffix is a fresh revision on every upload so a new texture
//! busts any client/CDN cache keyed on the URL; the previous file is unlinked.

use std::path::{Path, PathBuf};

pub use loontail_core::storage::{revision_hex, unlink_quiet, write_file};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    Skin,
    Cape,
}

impl TextureKind {
    /// The wire and database spelling: the `{skin|cape}` URL segment of the PNG read
    /// endpoint and the `user_textures.kind` value.
    pub fn as_str(self) -> &'static str {
        match self {
            TextureKind::Skin => "skin",
            TextureKind::Cape => "cape",
        }
    }

    /// Storage subdirectory under the textures root.
    pub fn storage_dir(self) -> &'static str {
        match self {
            TextureKind::Skin => "skins",
            TextureKind::Cape => "capes",
        }
    }

    pub fn parse(segment: &str) -> Option<TextureKind> {
        match segment {
            "skin" => Some(TextureKind::Skin),
            "cape" => Some(TextureKind::Cape),
            _ => None,
        }
    }
}

/// The absolute on-disk path for a texture file: `{root}/{dir}/{profile_uuid}-{revision}.png`.
pub fn disk_path(
    storage_root: &str,
    kind: TextureKind,
    profile_uuid: &str,
    revision: &str,
) -> PathBuf {
    Path::new(storage_root)
        .join(kind.storage_dir())
        .join(format!("{profile_uuid}-{revision}.png"))
}

/// Create the `{skins,capes}` subdirectories under the storage root. Called once
/// at startup so uploads never race directory creation.
pub async fn ensure_dirs(storage_root: &str) -> std::io::Result<()> {
    for kind in [TextureKind::Skin, TextureKind::Cape] {
        let dir = Path::new(storage_root).join(kind.storage_dir());
        tokio::fs::create_dir_all(&dir).await?;
    }
    Ok(())
}

/// An admin-facing texture registry row. `variant` is `None` for capes.
/// `updated_at` is the `timestamptz` rendered as text so this crate needs no date
/// dependency. Serialized camelCase to match the admin SPA.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AdminTextureRow {
    pub user_id: String,
    pub profile_uuid: String,
    pub username: String,
    pub file_url: String,
    pub file_path: String,
    pub file_size: i32,
    pub variant: Option<String>,
    pub updated_at: String,
}

const ADMIN_LIST_SQL: &str = "SELECT user_id::text AS user_id, profile_uuid, username, file_url, \
     file_path, file_size, variant, updated_at::text AS updated_at \
     FROM user_textures \
     WHERE kind = $1 AND ($2 = '' OR username ILIKE $3 OR profile_uuid ILIKE $3) \
     ORDER BY updated_at DESC LIMIT $4 OFFSET $5";

/// Paginated, case-insensitive listing of a kind's registry rows, newest first.
/// An empty `search` returns everything; otherwise it matches username or profile uuid.
pub async fn admin_list(
    pool: &sqlx::PgPool,
    kind: TextureKind,
    search: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<AdminTextureRow>> {
    let pattern = format!("%{search}%");
    sqlx::query_as::<_, AdminTextureRow>(ADMIN_LIST_SQL)
        .bind(kind.as_str())
        .bind(search)
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

/// Total rows matching the same search filter as [`admin_list`].
pub async fn admin_count(
    pool: &sqlx::PgPool,
    kind: TextureKind,
    search: &str,
) -> sqlx::Result<i64> {
    let pattern = format!("%{search}%");
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM user_textures \
         WHERE kind = $1 AND ($2 = '' OR username ILIKE $3 OR profile_uuid ILIKE $3)",
    )
    .bind(kind.as_str())
    .bind(search)
    .bind(&pattern)
    .fetch_one(pool)
    .await
}

/// Every row of a kind (no paging) — used by the orphan scan, which stats each
/// row's `file_path` on disk.
pub async fn admin_fetch_all(
    pool: &sqlx::PgPool,
    kind: TextureKind,
) -> sqlx::Result<Vec<AdminTextureRow>> {
    admin_list(pool, kind, "", i64::MAX, 0).await
}

/// Delete one kind's row for a user. Returns the removed row's `file_path` so the
/// caller can unlink it, or `None` if absent.
pub async fn delete_by_user(
    pool: &sqlx::PgPool,
    kind: TextureKind,
    user_id: uuid::Uuid,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "DELETE FROM user_textures WHERE user_id = $1 AND kind = $2 RETURNING file_path",
    )
    .bind(user_id)
    .bind(kind.as_str())
    .fetch_optional(pool)
    .await
}
