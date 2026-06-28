//! On-disk storage for skin/cape PNGs. Files live under
//! `config.textures.storage_root/{skins|capes}/{profile_uuid}-{6byte-hex}.png`.
//! The 6-byte hex suffix is a fresh revision on every upload so a new texture
//! busts any client/CDN cache keyed on the URL; the previous file is unlinked.

use std::path::{Path, PathBuf};

pub use loontail_core::storage::{revision_hex, unlink_quiet, write_file};

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

/// An admin-facing texture registry row (skins or capes). `variant` is `None`
/// for capes. `updated_at` is the `timestamptz` rendered as text so this crate
/// needs no date dependency. Serialized camelCase to match the admin SPA.
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

/// Projected columns for an [`AdminTextureRow`]. Capes have no `variant` column,
/// so it is projected as NULL to keep the row shape uniform across kinds.
fn admin_select_columns(kind: Kind) -> &'static str {
    match kind {
        Kind::Skin => "user_id::text AS user_id, profile_uuid, username, file_url, file_path, file_size, variant, updated_at::text AS updated_at",
        Kind::Cape => "user_id::text AS user_id, profile_uuid, username, file_url, file_path, file_size, NULL::text AS variant, updated_at::text AS updated_at",
    }
}

/// Paginated, case-insensitive listing of a kind's registry rows, newest first.
/// An empty `search` returns everything; otherwise it matches username or profile
/// uuid. `kind.dir()` doubles as the table name (`skins`/`capes`).
pub async fn admin_list(
    pool: &sqlx::PgPool,
    kind: Kind,
    search: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<AdminTextureRow>> {
    let pattern = format!("%{search}%");
    let sql = format!(
        "SELECT {cols} FROM {table} \
         WHERE ($1 = '' OR username ILIKE $2 OR profile_uuid ILIKE $2) \
         ORDER BY updated_at DESC LIMIT $3 OFFSET $4",
        cols = admin_select_columns(kind),
        table = kind.dir(),
    );
    sqlx::query_as::<_, AdminTextureRow>(&sql)
        .bind(search)
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

/// Total rows matching the same search filter as [`admin_list`].
pub async fn admin_count(pool: &sqlx::PgPool, kind: Kind, search: &str) -> sqlx::Result<i64> {
    let pattern = format!("%{search}%");
    let sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE ($1 = '' OR username ILIKE $2 OR profile_uuid ILIKE $2)",
        table = kind.dir(),
    );
    sqlx::query_scalar::<_, i64>(&sql)
        .bind(search)
        .bind(&pattern)
        .fetch_one(pool)
        .await
}

/// Every row of a kind (no paging) — used by the orphan scan, which stats each
/// row's `file_path` on disk.
pub async fn admin_fetch_all(
    pool: &sqlx::PgPool,
    kind: Kind,
) -> sqlx::Result<Vec<AdminTextureRow>> {
    let sql = format!(
        "SELECT {cols} FROM {table} ORDER BY updated_at DESC",
        cols = admin_select_columns(kind),
        table = kind.dir(),
    );
    sqlx::query_as::<_, AdminTextureRow>(&sql)
        .fetch_all(pool)
        .await
}

/// Delete one kind's row for a user (admin moderation). Returns the removed
/// row's `file_path` so the caller can unlink it, or `None` if absent.
pub async fn admin_delete_by_user(
    pool: &sqlx::PgPool,
    kind: Kind,
    user_id: uuid::Uuid,
) -> sqlx::Result<Option<String>> {
    let sql = format!(
        "DELETE FROM {table} WHERE user_id = $1 RETURNING file_path",
        table = kind.dir(),
    );
    sqlx::query_scalar::<_, String>(&sql)
        .bind(user_id)
        .fetch_optional(pool)
        .await
}
