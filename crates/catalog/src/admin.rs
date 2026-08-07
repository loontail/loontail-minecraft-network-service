//! Admin catalog CRUD (`AdminUser`-guarded): create/update/delete clients,
//! keywords, and servers; publish/unpublish; and media management.

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use loontail_core::auth::AdminUser;
use loontail_core::error::{found, is_foreign_key_violation, slug_conflict, AppError, AppResult};
use loontail_core::storage::read_capped_upload;
use loontail_core::AppState;

use crate::dto::{CatalogQuery, ClientAdminList};
use crate::storage;
use crate::{repo, MAX_MEDIA_UPLOAD_BYTES};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertClient {
    pub slug: String,
    #[serde(default)]
    pub available: bool,
    pub minecraft_version: Option<String>,
    pub forge_version: Option<String>,
    pub fabric_version: Option<String>,
    pub runtime_version: Option<String>,
    /// The bundle to own. ABSENT means "keep whatever this build is already linked
    /// to" (a pure slug rename must not retarget); an explicit value (including
    /// `null`) re-resolves the link. See [`update_client`].
    #[serde(default, deserialize_with = "double_option")]
    pub bundle_slug: Option<Option<String>>,
    /// Absent leaves the existing display order untouched — the column is only ever
    /// set deliberately, so a save that omits it must not silently reset it to 0.
    #[serde(default)]
    pub sort_order: Option<i32>,
    /// When `Some(true)`, publish on create; absent/`false` leaves a draft. Only
    /// consulted by [`create_client`].
    #[serde(default)]
    pub publish: Option<bool>,
    #[serde(default)]
    pub locales: Vec<ClientLocaleInput>,
    /// Opt in to making `locales` the complete set: locales absent from the payload
    /// are deleted. Default `false` upserts only what was sent, so a single-locale
    /// save cannot destroy the other translations.
    #[serde(default)]
    pub replace_locales: bool,
    /// Opt in to destroying a bundle that this update strands. Default `false` leaves
    /// an unreferenced bundle (and its uploaded files) in place, reported back as
    /// `orphanedBundleSlug`.
    #[serde(default)]
    pub delete_orphaned_bundle: bool,
}

/// Distinguish "field absent" from "field present and null" — `Option<Option<T>>`
/// where the outer layer is presence. serde's `#[serde(default)]` alone collapses
/// both to `None`.
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientLocaleInput {
    pub locale: String,
    pub title: String,
    pub description: Option<String>,
    pub short_description: Option<String>,
}

/// List every client including drafts, each with its `published` state (the public
/// `/api/clients` hides drafts).
pub async fn list_clients(
    State(state): State<AppState>,
    _admin: AdminUser,
    query: CatalogQuery,
) -> AppResult<Json<ClientAdminList>> {
    let clients = repo::list_clients_admin(&state.pool, &query).await?;
    Ok(Json(ClientAdminList { clients }))
}

/// Create a build: insert the client row (+ locales), then find-or-create its owned
/// bundle (1:1) and link it via `bundle_id`/`bundle_slug` so the build immediately
/// has a resolvable manifest URL. Returns `{ id, bundleSlug }`.
pub async fn create_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<UpsertClient>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let published_setter = if body.publish == Some(true) {
        "now()"
    } else {
        "NULL"
    };

    let mut tx = state.pool.begin().await?;
    // `bundle_slug` is deliberately absent here: `link_owned_bundle` below is its only
    // writer, so binding it twice would just be a dead write.
    let id = sqlx::query_scalar::<_, Uuid>(AssertSqlSafe(format!(
        "INSERT INTO catalog_clients \
         (slug, available, minecraft_version, forge_version, fabric_version, \
          runtime_version, sort_order, published_at) \
         VALUES ($1,$2,$3,$4,$5,$6,COALESCE($7, 0),{published_setter}) RETURNING id"
    )))
    .bind(&body.slug)
    .bind(body.available)
    .bind(&body.minecraft_version)
    .bind(&body.forge_version)
    .bind(&body.fabric_version)
    .bind(&body.runtime_version)
    .bind(body.sort_order)
    .fetch_one(&mut *tx)
    .await
    .map_err(slug_conflict("client"))?;

    write_client_locales(&mut tx, id, &body.locales).await?;

    let owned_slug = requested_bundle_slug(&body)
        .unwrap_or_else(|| body.slug.trim())
        .to_string();
    let LinkedBundle {
        bundle_id: _,
        effective_slug,
        provisioned,
    } = link_owned_bundle(&mut tx, id, &owned_slug, &body.locales).await?;

    tx.commit().await?;

    // why: create the on-disk dir only after commit, so a rolled-back create leaves
    // no stray directory.
    if provisioned {
        loontail_bundles::ensure_bundle_dir(&state.config.bundles.storage_root, &effective_slug)?;
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "bundleSlug": effective_slug })),
    ))
}

/// Result of [`link_owned_bundle`]: the linked bundle id/slug and whether a new
/// bundle row was provisioned (so the caller creates the on-disk dir after commit).
/// `bundle_id` lets [`update_client`] detect when the link moved to a different
/// bundle and tear down the now-unreferenced old one.
struct LinkedBundle {
    bundle_id: Uuid,
    effective_slug: String,
    provisioned: bool,
}

/// The bundle slug this payload explicitly asks for: `None` when the field is absent
/// OR present-but-null/blank. Callers distinguish those two cases via
/// `body.bundle_slug.is_some()`.
fn requested_bundle_slug(body: &UpsertClient) -> Option<&str> {
    body.bundle_slug
        .as_ref()
        .and_then(|inner| inner.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Find-or-create the bundle at `effective_slug` (1:1 with the client) and link it,
/// inside the caller's tx so a provision failure rolls the whole upsert back.
/// `locales` only supplies a display name for a freshly provisioned bundle. This is
/// the single writer of `catalog_clients.bundle_id`/`bundle_slug`. The caller creates
/// the on-disk dir after commit when `provisioned` is true.
async fn link_owned_bundle(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    client_id: Uuid,
    effective_slug: &str,
    locales: &[ClientLocaleInput],
) -> AppResult<LinkedBundle> {
    let effective_slug = effective_slug.to_string();

    let (bundle_id, provisioned) =
        match loontail_bundles::find_bundle_id_by_slug(&mut **tx, &effective_slug).await? {
            Some(existing) => (existing, false),
            None => {
                let name = locales
                    .first()
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| effective_slug.clone());
                let id = loontail_bundles::provision_bundle_row(tx, &effective_slug, &name).await?;
                (id, true)
            }
        };

    sqlx::query("UPDATE catalog_clients SET bundle_id = $1, bundle_slug = $2 WHERE id = $3")
        .bind(bundle_id)
        .bind(&effective_slug)
        .bind(client_id)
        .execute(&mut **tx)
        .await?;

    Ok(LinkedBundle {
        bundle_id,
        effective_slug,
        provisioned,
    })
}

/// Full-column replace of a build's own fields, with three deliberately
/// non-destructive defaults:
/// - `bundleSlug` ABSENT keeps the current bundle link (only an explicit value
///   retargets), so renaming a build never strands its uploaded files;
/// - `sortOrder` absent keeps the stored order;
/// - `locales` upserts what was sent and keeps the rest unless `replaceLocales` is
///   `true`.
///
/// A retarget that strands the previous bundle deletes it only when nothing references
/// it AND (it holds no artifacts OR `deleteOrphanedBundle` is `true`); otherwise the
/// bundle survives and its slug comes back as `orphanedBundleSlug`.
pub async fn update_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertClient>,
) -> AppResult<Json<Value>> {
    let mut tx = state.pool.begin().await?;

    // Capture the currently-linked bundle before re-pointing, so a slug change that
    // moves the link can tear the old bundle down (below) rather than orphaning it.
    // Outer Option = 404 guard, inner = the nullable `bundle_id`.
    let old_bundle_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT bundle_id FROM catalog_clients WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("client not found".into()))?;

    sqlx::query(
        "UPDATE catalog_clients SET slug=$2, available=$3, minecraft_version=$4, \
         forge_version=$5, fabric_version=$6, runtime_version=$7, \
         sort_order=COALESCE($8, sort_order), updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .bind(&body.slug)
    .bind(body.available)
    .bind(&body.minecraft_version)
    .bind(&body.forge_version)
    .bind(&body.fabric_version)
    .bind(&body.runtime_version)
    .bind(body.sort_order)
    .execute(&mut *tx)
    .await
    .map_err(slug_conflict("client"))?;

    write_client_locales(&mut tx, id, &body.locales).await?;
    if body.replace_locales {
        let submitted: Vec<&str> = body.locales.iter().map(|l| l.locale.as_str()).collect();
        sqlx::query(
            "DELETE FROM catalog_client_locales WHERE client_id = $1 AND locale <> ALL($2::text[])",
        )
        .bind(id)
        .bind(&submitted)
        .execute(&mut *tx)
        .await?;
    }

    let owned_slug = resolve_owned_bundle_slug(&mut tx, &body, old_bundle_id).await?;
    let LinkedBundle {
        bundle_id: new_bundle_id,
        effective_slug,
        provisioned,
    } = link_owned_bundle(&mut tx, id, &owned_slug, &body.locales).await?;

    let stranded = match old_bundle_id {
        Some(old) if old != new_bundle_id => {
            strand_old_bundle(&mut tx, old, body.delete_orphaned_bundle).await?
        }
        _ => Stranded::None,
    };

    tx.commit().await?;

    if provisioned {
        loontail_bundles::ensure_bundle_dir(&state.config.bundles.storage_root, &effective_slug)?;
    }

    let orphaned = match stranded {
        Stranded::Deleted(slug) => {
            loontail_bundles::remove_bundle_dir(&state.config.bundles.storage_root, &slug);
            None
        }
        Stranded::Kept(slug) => Some(slug),
        Stranded::None => None,
    };

    Ok(Json(json!({ "id": id, "orphanedBundleSlug": orphaned })))
}

/// Which bundle slug this update should own. An explicit `bundleSlug` wins; an
/// explicit null (or blank) re-provisions from the client slug, matching create; an
/// ABSENT field keeps whatever bundle the row is already linked to, so a pure slug
/// rename cannot retarget and strand the build's uploaded files.
async fn resolve_owned_bundle_slug(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    body: &UpsertClient,
    old_bundle_id: Option<Uuid>,
) -> AppResult<String> {
    if let Some(explicit) = requested_bundle_slug(body) {
        return Ok(explicit.to_string());
    }
    let client_slug = body.slug.trim().to_string();
    if body.bundle_slug.is_some() {
        return Ok(client_slug);
    }
    match old_bundle_id {
        Some(old) => Ok(loontail_bundles::bundle_slug_by_id(&mut **tx, old)
            .await?
            .unwrap_or(client_slug)),
        None => Ok(client_slug),
    }
}

/// Fate of a bundle a retarget left behind.
enum Stranded {
    /// Row deleted in the tx (CASCADE dropped its artifacts); caller removes the dir.
    Deleted(String),
    /// Left in place because it still holds artifacts and the caller did not opt in.
    Kept(String),
    /// Nothing was stranded (still referenced, or no link moved).
    None,
}

/// Decide and apply what happens to the bundle a retarget just unlinked. A bundle
/// another build still references is never touched. An unreferenced one is deleted
/// when it is empty or when the caller explicitly opted in — otherwise it survives so
/// gigabytes of uploads are never destroyed by an implicit slug rename.
async fn strand_old_bundle(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    old: Uuid,
    delete_orphaned: bool,
) -> AppResult<Stranded> {
    let still_referenced: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog_clients WHERE bundle_id = $1")
            .bind(old)
            .fetch_one(&mut **tx)
            .await?;
    if still_referenced > 0 {
        return Ok(Stranded::None);
    }

    let Some(slug) = loontail_bundles::bundle_slug_by_id(&mut **tx, old).await? else {
        return Ok(Stranded::None);
    };
    let artifacts = loontail_bundles::count_artifacts(&mut **tx, old).await?;
    if delete_orphaned || artifacts == 0 {
        loontail_bundles::delete_bundle_row(&mut **tx, old).await?;
        Ok(Stranded::Deleted(slug))
    } else {
        Ok(Stranded::Kept(slug))
    }
}

async fn write_client_locales(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    client_id: Uuid,
    locales: &[ClientLocaleInput],
) -> AppResult<()> {
    for l in locales {
        sqlx::query(
            "INSERT INTO catalog_client_locales \
             (client_id, locale, title, description, short_description) \
             VALUES ($1,$2,$3,$4,$5) \
             ON CONFLICT (client_id, locale) DO UPDATE SET \
             title=EXCLUDED.title, description=EXCLUDED.description, \
             short_description=EXCLUDED.short_description",
        )
        .bind(client_id)
        .bind(&l.locale)
        .bind(&l.title)
        .bind(&l.description)
        .bind(&l.short_description)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Delete a build and its owned bundle. The FK
/// `catalog_clients.bundle_id REFERENCES bundles(id) ON DELETE SET NULL` would
/// otherwise orphan the bundle row, its artifacts, and the on-disk tree. So in ONE
/// transaction we read the owned `bundle_id`/slug, delete the client (dropping the FK
/// ref), then delete the bundle (CASCADE drops artifacts); on-disk files are removed
/// best-effort after commit.
pub async fn delete_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let mut tx = state.pool.begin().await?;

    // Outer Option = row presence (404 guard); inner = the nullable `bundle_id`.
    let bundle_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT bundle_id FROM catalog_clients WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("client not found".into()))?;

    // why: delete the client first to drop the FK reference, then the owned bundle in
    // the same tx so both rows go or neither does.
    sqlx::query("DELETE FROM catalog_clients WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let bundle_slug = match bundle_id {
        Some(bundle_id) => {
            let slug = loontail_bundles::bundle_slug_by_id(&mut *tx, bundle_id).await?;
            loontail_bundles::delete_bundle_row(&mut *tx, bundle_id).await?;
            slug
        }
        None => None,
    };

    tx.commit().await?;

    if let Some(slug) = bundle_slug {
        loontail_bundles::remove_bundle_dir(&state.config.bundles.storage_root, &slug);
    }

    // why: the cascade dropped the `catalog_media` rows but not the bytes on disk, so
    // also remove the client's catalog-media dir (undashed hex, matching the upload
    // path) or the images leak forever.
    let media_dir =
        std::path::Path::new(&state.config.catalog.storage_root).join(id.simple().to_string());
    tokio::fs::remove_dir_all(media_dir).await.ok();

    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, PublishableTable::Clients, id, true).await
}

pub async fn unpublish_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, PublishableTable::Clients, id, false).await
}

/// Tables whose `published_at` flag the admin can toggle. An enum (not a `&str`) so
/// `set_published` only ever interpolates a fixed, in-code table name into the SQL.
#[derive(Clone, Copy)]
enum PublishableTable {
    Clients,
    Keywords,
    Servers,
}

impl PublishableTable {
    fn name(self) -> &'static str {
        match self {
            PublishableTable::Clients => "catalog_clients",
            PublishableTable::Keywords => "catalog_keywords",
            PublishableTable::Servers => "catalog_servers",
        }
    }

    /// The noun the 404 names, so a missing id says which kind of row was missing.
    fn entity(self) -> &'static str {
        match self {
            PublishableTable::Clients => "client",
            PublishableTable::Keywords => "keyword",
            PublishableTable::Servers => "server",
        }
    }
}

async fn set_published(
    state: &AppState,
    table: PublishableTable,
    id: Uuid,
    published: bool,
) -> AppResult<Json<Value>> {
    let entity = table.entity();
    let table = table.name();
    let setter = if published {
        "published_at = now()"
    } else {
        "published_at = NULL"
    };
    let affected = sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET {setter}, updated_at = now() WHERE id = $1"
    )))
    .bind(id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    found(affected, entity)?;
    Ok(Json(json!({ "id": id, "published": published })))
}

const MEDIA_ROLES: [&str; 4] = ["poster", "background", "titleImage", "screenshot"];

/// Roles that occupy a single slot per client: a new upload replaces the existing
/// row (and unlinks its file). `screenshot` is the only multi-row role.
const SINGULAR_ROLES: [&str; 3] = ["poster", "background", "titleImage"];

async fn require_client(state: &AppState, client_id: Uuid) -> AppResult<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_clients WHERE id=$1)")
            .bind(client_id)
            .fetch_one(&state.pool)
            .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::NotFound("client not found".into()))
    }
}

struct ImageKind {
    ext: &'static str,
    mime: &'static str,
}

/// Sniff the image format from magic bytes; accepts PNG, JPEG, WebP, and GIF,
/// rejecting anything else with `None`.
fn sniff_image(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some(ImageKind {
            ext: "png",
            mime: "image/png",
        })
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageKind {
            ext: "jpg",
            mime: "image/jpeg",
        })
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageKind {
            ext: "webp",
            mime: "image/webp",
        })
    } else if bytes.starts_with(b"GIF8") {
        Some(ImageKind {
            ext: "gif",
            mime: "image/gif",
        })
    } else {
        None
    }
}

/// Pixel dimensions of a PNG, `None` for every other accepted format (JPEG, WebP and
/// GIF are stored with NULL width/height). Delegates to the workspace's tested PNG
/// header parser, which also validates the 8-byte signature and the IHDR length — the
/// inlined copy this replaces checked neither, so a hostile non-PNG whose 13th byte
/// happened to spell IHDR had its "dimensions" recorded.
fn png_dimensions(bytes: &[u8]) -> (Option<i32>, Option<i32>) {
    match loontail_yggdrasil_protocol::png::parse_png_header(bytes) {
        Ok((width, height)) => (Some(width as i32), Some(height as i32)),
        Err(_) => (None, None),
    }
}

/// Upload real image bytes (multipart): sniff the format, write a fresh revision to
/// disk, and insert a `catalog_media` row with a server-relative `url`. For singular
/// roles the prior row is deleted (and its file unlinked) first; screenshots append.
pub async fn upload_media(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(client_id): Path<Uuid>,
    multipart: Multipart,
) -> AppResult<(StatusCode, Json<Value>)> {
    require_client(&state, client_id).await?;

    let (file, role) = read_capped_upload(multipart, MAX_MEDIA_UPLOAD_BYTES, "role").await?;
    let role = role.ok_or_else(|| AppError::BadRequest("missing role field".into()))?;
    if !MEDIA_ROLES.contains(&role.as_str()) {
        return Err(AppError::BadRequest(format!("invalid media role '{role}'")));
    }
    let file = file.ok_or_else(|| AppError::BadRequest("missing file field".into()))?;

    let kind = sniff_image(&file).ok_or_else(|| {
        AppError::BadRequest("unsupported image type — use PNG, JPG, WebP, or GIF".into())
    })?;
    let (width, height) = png_dimensions(&file);

    let client_hex = client_id.simple().to_string();
    let revision = storage::revision_hex();
    let disk_path = storage::disk_path(
        &state.config.catalog.storage_root,
        &client_hex,
        &role,
        &revision,
        kind.ext,
    );
    let url = storage::public_url(&client_hex, &role, &revision, kind.ext);
    let size = file.len() as i32;

    storage::write_file(&disk_path, &file)
        .await
        .map_err(|err| AppError::Internal(anyhow::anyhow!("write catalog media: {err}")))?;

    let mut tx = state.pool.begin().await?;
    if SINGULAR_ROLES.contains(&role.as_str()) {
        let old_urls: Vec<String> = sqlx::query_scalar(
            "DELETE FROM catalog_media WHERE client_id = $1 AND role = $2 RETURNING url",
        )
        .bind(client_id)
        .bind(&role)
        .fetch_all(&mut *tx)
        .await?;
        for old in old_urls {
            if let Some(path) = url_to_disk_path(&state.config.catalog.storage_root, &old) {
                storage::unlink_quiet(&path).await;
            }
        }
    }

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_media \
         (client_id, role, url, ext, mime, width, height, size) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
    )
    .bind(client_id)
    .bind(&role)
    .bind(&url)
    .bind(kind.ext)
    .bind(kind.mime)
    .bind(width)
    .bind(height)
    .bind(size)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id, "url": url }))))
}

pub async fn list_media(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(client_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    require_client(&state, client_id).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<i32>, Option<i32>, i32)>(
        "SELECT id, role, url, width, height, sort_order FROM catalog_media \
         WHERE client_id = $1 ORDER BY sort_order, created_at",
    )
    .bind(client_id)
    .fetch_all(&state.pool)
    .await?;
    let media: Vec<Value> = rows
        .into_iter()
        .map(|(id, role, url, width, height, sort_order)| {
            json!({
                "id": id,
                "role": role,
                "url": url,
                "width": width,
                "height": height,
                "sortOrder": sort_order,
            })
        })
        .collect();
    Ok(Json(json!({ "media": media })))
}

pub async fn delete_media(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, media_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    let url = sqlx::query_scalar::<_, String>(
        "DELETE FROM catalog_media WHERE id = $1 AND client_id = $2 RETURNING url",
    )
    .bind(media_id)
    .bind(client_id)
    .fetch_optional(&state.pool)
    .await?;
    let url = url.ok_or_else(|| AppError::NotFound("media not found".into()))?;
    if let Some(path) = url_to_disk_path(&state.config.catalog.storage_root, &url) {
        storage::unlink_quiet(&path).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Map a server-relative `/catalog-media/...` url back to its on-disk path. `None`
/// for externally-hosted urls, so deleting those never touches the disk.
fn url_to_disk_path(storage_root: &str, url: &str) -> Option<std::path::PathBuf> {
    let rel = url.strip_prefix("/catalog-media/")?;
    if rel.is_empty() || rel.contains("..") {
        return None;
    }
    Some(std::path::Path::new(storage_root).join(rel))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertKeyword {
    pub slug: String,
    #[serde(default)]
    pub locales: Vec<KeywordLocaleInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordLocaleInput {
    pub locale: String,
    pub title: String,
}

pub async fn create_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<UpsertKeyword>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = state.pool.begin().await?;
    // why: published on insert — the catalog read inlines a keyword only when
    // `published_at IS NOT NULL`, so an unpublished one would never reach the UI.
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_keywords (slug, published_at) VALUES ($1, now()) RETURNING id",
    )
    .bind(&body.slug)
    .fetch_one(&mut *tx)
    .await
    .map_err(slug_conflict("keyword"))?;
    for l in &body.locales {
        sqlx::query(
            "INSERT INTO catalog_keyword_locales (keyword_id, locale, title) \
             VALUES ($1,$2,$3) ON CONFLICT (keyword_id, locale) \
             DO UPDATE SET title = EXCLUDED.title",
        )
        .bind(id)
        .bind(&l.locale)
        .bind(&l.title)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn delete_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let affected = sqlx::query("DELETE FROM catalog_keywords WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    found(affected, "keyword")?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, PublishableTable::Keywords, id, true).await
}

pub async fn unpublish_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, PublishableTable::Keywords, id, false).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertServer {
    pub slug: String,
    pub name: Option<String>,
    pub address: String,
}

pub async fn create_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<UpsertServer>,
) -> AppResult<(StatusCode, Json<Value>)> {
    // why: published on insert (see `create_keyword`).
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_servers (slug, name, address, published_at) \
         VALUES ($1,$2,$3, now()) RETURNING id",
    )
    .bind(&body.slug)
    .bind(&body.name)
    .bind(&body.address)
    .fetch_one(&state.pool)
    .await
    .map_err(slug_conflict("server"))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn update_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertServer>,
) -> AppResult<Json<Value>> {
    let affected = sqlx::query(
        "UPDATE catalog_servers SET slug=$2, name=$3, address=$4, updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .bind(&body.slug)
    .bind(&body.name)
    .bind(&body.address)
    .execute(&state.pool)
    .await
    .map_err(slug_conflict("server"))?
    .rows_affected();
    found(affected, "server")?;
    Ok(Json(json!({ "id": id })))
}

pub async fn delete_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let affected = sqlx::query("DELETE FROM catalog_servers WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    found(affected, "server")?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, PublishableTable::Servers, id, true).await
}

pub async fn unpublish_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, PublishableTable::Servers, id, false).await
}

pub async fn attach_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, keyword_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    sqlx::query(
        "INSERT INTO catalog_client_keywords (client_id, keyword_id) VALUES ($1,$2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(client_id)
    .bind(keyword_id)
    .execute(&state.pool)
    .await
    .map_err(map_fk)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn attach_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, server_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    sqlx::query(
        "INSERT INTO catalog_client_servers (client_id, server_id) VALUES ($1,$2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(client_id)
    .bind(server_id)
    .execute(&state.pool)
    .await
    .map_err(map_fk)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Detach a keyword from a build. Idempotent: `204` even when no row was present.
pub async fn detach_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, keyword_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    sqlx::query("DELETE FROM catalog_client_keywords WHERE client_id = $1 AND keyword_id = $2")
        .bind(client_id)
        .bind(keyword_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Detach a server from a build. Idempotent (see [`detach_keyword`]).
pub async fn detach_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, server_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    sqlx::query("DELETE FROM catalog_client_servers WHERE client_id = $1 AND server_id = $2")
        .bind(client_id)
        .bind(server_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn map_fk(err: sqlx::Error) -> AppError {
    if is_foreign_key_violation(&err) {
        AppError::NotFound("client, keyword, or server not found".into())
    } else {
        err.into()
    }
}
