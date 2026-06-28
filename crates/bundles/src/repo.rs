//! Database access for bundles and their artifacts (runtime sqlx only) and the
//! manifest regeneration routine that ties on-disk scanning to the artifacts.json.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use loontail_core::error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::archive::ScanEntry;
use crate::manifest::{build_manifest, to_json};
use crate::models::{Bundle, BundleArtifact};
use crate::storage::{files_path, split_relative_path, write_manifest_atomic};

pub async fn list_bundles(pool: &PgPool) -> AppResult<Vec<Bundle>> {
    Ok(
        sqlx::query_as::<_, Bundle>("SELECT * FROM bundles ORDER BY created_at DESC")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn find_by_slug(pool: &PgPool, slug: &str) -> AppResult<Option<Bundle>> {
    Ok(
        sqlx::query_as::<_, Bundle>("SELECT * FROM bundles WHERE slug = $1")
            .bind(slug)
            .fetch_optional(pool)
            .await?,
    )
}

/// Look up an existing bundle's id by slug, or `None`. The slug is the cross-crate
/// link key (a catalog client owns a bundle whose slug matches it), so this is the
/// find half of catalog's find-or-create provisioning. Runs on any sqlx executor so
/// callers can use it inside a transaction.
pub async fn find_bundle_id_by_slug<'e, E>(executor: E, slug: &str) -> AppResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    Ok(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM bundles WHERE slug = $1")
            .bind(slug)
            .fetch_optional(executor)
            .await?,
    )
}

/// Insert a new draft `bundles` row (description/version NULL) and return its id —
/// the DB half of provisioning, with no filesystem side effect, so it runs inside a
/// caller's transaction. Validates the slug and rejects a pre-existing slug as
/// [`AppError::Conflict`]. Pair with [`ensure_bundle_dir`] after the tx commits.
pub async fn provision_bundle_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    slug: &str,
    name: &str,
) -> AppResult<Uuid> {
    crate::storage::validate_slug(slug)?;
    if find_bundle_id_by_slug(&mut **tx, slug).await?.is_some() {
        return Err(AppError::Conflict(
            "a build with this slug already exists".into(),
        ));
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO bundles (name, slug, status) VALUES ($1, $2, 'draft') RETURNING id",
    )
    .bind(name)
    .bind(slug)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

/// Create a bundle's on-disk `{storage_root}/builds/{slug}/files` directory
/// (idempotent). Run after the bundle row is committed so a rolled-back provision
/// leaves no stray directory.
pub fn ensure_bundle_dir(storage_root: &str, slug: &str) -> AppResult<()> {
    crate::storage::ensure_build_dir(storage_root, slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir build dir: {e}")))
}

/// Provision a new draft bundle: validate the slug, insert the `bundles` row
/// (description/version NULL), create its on-disk `{storage_root}/builds/{slug}/files`
/// directory, and return the new bundle id. The slug must be free — callers should
/// find-first, but a pre-existing slug is reported as [`AppError::Conflict`] so this
/// never silently collides.
pub async fn provision_bundle(
    pool: &PgPool,
    storage_root: &str,
    slug: &str,
    name: &str,
) -> AppResult<Uuid> {
    let mut tx = pool.begin().await?;
    let id = provision_bundle_row(&mut tx, slug, name).await?;
    tx.commit().await?;
    ensure_bundle_dir(storage_root, slug)?;
    Ok(id)
}

pub async fn require_by_slug(pool: &PgPool, slug: &str) -> AppResult<Bundle> {
    find_by_slug(pool, slug)
        .await?
        .ok_or_else(|| AppError::NotFound("build not found".into()))
}

pub async fn update_bundle_meta(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
    version: Option<&str>,
) -> AppResult<Bundle> {
    // COALESCE keeps the existing column when the field is omitted, except for
    // description/version which are nullable and may be intentionally cleared.
    Ok(sqlx::query_as::<_, Bundle>(
        r#"
        UPDATE bundles
        SET name = COALESCE($2, name),
            description = $3,
            version = $4,
            updated_at = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(version)
    .fetch_one(pool)
    .await?)
}

pub async fn delete_bundle(pool: &PgPool, id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM bundles WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Read a bundle's slug by id (any executor, so callers can run it in a tx). `None`
/// when no such bundle exists.
pub async fn bundle_slug_by_id<'e, E>(executor: E, id: Uuid) -> AppResult<Option<String>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    Ok(
        sqlx::query_scalar::<_, String>("SELECT slug FROM bundles WHERE id = $1")
            .bind(id)
            .fetch_optional(executor)
            .await?,
    )
}

/// Delete a `bundles` row (any executor). CASCADE drops its `bundle_artifacts`.
pub async fn delete_bundle_row<'e, E>(executor: E, id: Uuid) -> AppResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("DELETE FROM bundles WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Remove a build's on-disk `{storage_root}/builds/{slug}` tree (best-effort: a
/// failed unlink is logged, never propagated, so an orphaned directory can't block
/// the authoritative row deletion).
pub fn remove_bundle_dir(storage_root: &str, slug: &str) {
    if let Err(e) = crate::storage::delete_build_files(storage_root, slug) {
        tracing::warn!(slug, error = %e, "failed to delete on-disk build files");
    }
}

/// Fully delete a build's owned bundle: look up its slug, drop the `bundles` row
/// (CASCADE removes `bundle_artifacts`), then best-effort remove the on-disk
/// `{storage_root}/builds/{slug}` tree. Reusable across crates — the catalog calls
/// this when a build (catalog client) that owns a bundle is deleted, since the
/// `ON DELETE SET NULL` FK would otherwise orphan the bundle, its artifacts, and its
/// files forever. The directory is removed after the row delete and independently of
/// it (a stale dir never blocks the DB delete).
pub async fn delete_owned_bundle(
    pool: &PgPool,
    storage_root: &str,
    bundle_id: Uuid,
) -> AppResult<()> {
    let slug = bundle_slug_by_id(pool, bundle_id).await?;
    delete_bundle_row(pool, bundle_id).await?;
    if let Some(slug) = slug {
        remove_bundle_dir(storage_root, &slug);
    }
    Ok(())
}

pub async fn set_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    processing_error: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE bundles SET status = $2, processing_error = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(processing_error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_artifacts(pool: &PgPool, bundle_id: Uuid) -> AppResult<Vec<BundleArtifact>> {
    Ok(sqlx::query_as::<_, BundleArtifact>(
        "SELECT * FROM bundle_artifacts WHERE bundle_id = $1 ORDER BY relative_path ASC",
    )
    .bind(bundle_id)
    .fetch_all(pool)
    .await?)
}

pub async fn find_artifact(pool: &PgPool, id: Uuid) -> AppResult<Option<BundleArtifact>> {
    Ok(
        sqlx::query_as::<_, BundleArtifact>("SELECT * FROM bundle_artifacts WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Upsert one file/dir artifact by `(bundle_id, relative_path)`. A new row inserts
/// `download_once = false`; an existing row's `download_once` is left untouched (the
/// `DO UPDATE` deliberately omits it, preserving an operator's toggle across rescans).
/// Backed by the `(bundle_id, relative_path)` unique index (migration 0013).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_artifact(
    pool: &PgPool,
    bundle_id: Uuid,
    relative_path: &str,
    name: &str,
    category: &str,
    size: i64,
    sha256: Option<&str>,
    is_dir: bool,
    file_modified_at: Option<DateTime<Utc>>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO bundle_artifacts
            (bundle_id, relative_path, name, category, size, sha256, is_dir, download_once, file_modified_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, false, $8)
        ON CONFLICT (bundle_id, relative_path) DO UPDATE
        SET name = EXCLUDED.name,
            category = EXCLUDED.category,
            size = EXCLUDED.size,
            sha256 = EXCLUDED.sha256,
            is_dir = EXCLUDED.is_dir,
            file_modified_at = EXCLUDED.file_modified_at
        "#,
    )
    .bind(bundle_id)
    .bind(relative_path)
    .bind(name)
    .bind(category)
    .bind(size)
    .bind(sha256)
    .bind(is_dir)
    .bind(file_modified_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace all artifact rows for a bundle with a freshly scanned set.
pub async fn upsert_scan(pool: &PgPool, bundle_id: Uuid, scan: &[ScanEntry]) -> AppResult<()> {
    for entry in scan {
        upsert_artifact(
            pool,
            bundle_id,
            &entry.relative_path,
            &entry.name,
            &entry.category,
            entry.size,
            entry.sha256.as_deref(),
            entry.is_dir,
            entry.file_modified_at,
        )
        .await?;
    }
    Ok(())
}

pub async fn update_artifact_sha(pool: &PgPool, id: Uuid, sha256: &str) -> AppResult<()> {
    sqlx::query("UPDATE bundle_artifacts SET sha256 = $2 WHERE id = $1")
        .bind(id)
        .bind(sha256)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_download_once(pool: &PgPool, id: Uuid, value: bool) -> AppResult<()> {
    sqlx::query("UPDATE bundle_artifacts SET download_once = $2 WHERE id = $1")
        .bind(id)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_artifact(pool: &PgPool, id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM bundle_artifacts WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_artifacts_with_prefix(
    pool: &PgPool,
    bundle_id: Uuid,
    prefix: &str,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path LIKE $2 || '%'",
    )
    .bind(bundle_id)
    .bind(prefix)
    .execute(pool)
    .await?;
    Ok(())
}

/// `true` when an artifact row already occupies `relative_path` in this bundle.
/// Used to return a clean 409 before a move/rename instead of letting the
/// `(bundle_id, relative_path)` unique index raise a raw 500. Runs on any executor
/// so callers can check inside a transaction.
pub async fn artifact_exists_at<'e, E>(
    executor: E,
    bundle_id: Uuid,
    relative_path: &str,
) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path = $2",
    )
    .bind(bundle_id)
    .bind(relative_path)
    .fetch_one(executor)
    .await?;
    Ok(count > 0)
}

/// `true` when any artifact row in this bundle lives under `prefix` (pass it with a
/// trailing `/`). Detects a destination folder that already has contents so a folder
/// move can be refused with a clean 409.
pub async fn any_artifact_with_prefix<'e, E>(
    executor: E,
    bundle_id: Uuid,
    prefix: &str,
) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path LIKE $2 || '%'",
    )
    .bind(bundle_id)
    .bind(prefix)
    .fetch_one(executor)
    .await?;
    Ok(count > 0)
}

/// Update one artifact row's path on a transaction executor, so a whole subtree can
/// move atomically inside the caller's transaction.
async fn update_artifact_path_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    relative_path: &str,
    name: &str,
    category: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE bundle_artifacts SET relative_path = $2, name = $3, category = $4 WHERE id = $1",
    )
    .bind(id)
    .bind(relative_path)
    .bind(name)
    .bind(category)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Move (rename) a file or folder subtree within one bundle: rewrite the moved row's
/// `relative_path`/`name`/`category` plus every descendant row's path (prefix
/// `old_rel` -> `new_rel`, re-deriving `category`), then perform the single on-disk
/// rename of the subtree. All DB writes run inside the passed transaction.
///
/// Ordering (recoverability): the DB rewrites happen FIRST, the OS rename LAST. The
/// tx is NOT yet committed when the OS rename runs, so if `std::fs::rename` fails we
/// return early and the caller's tx rolls back every row change — disk and DB stay
/// consistent. The OS rename is the only non-transactional step and is deferred to
/// the very end where its failure is still recoverable by dropping the tx.
///
/// `old_rel`/`new_rel` are normalized forward-slash relative paths. Caller is
/// responsible for the conflict + self-into-subtree guards (this fn assumes the move
/// is legal). A no-op (`old_rel == new_rel`) is rejected by the caller's guards.
pub(crate) async fn move_subtree(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bundle_id: Uuid,
    files_root: &Path,
    old_rel: &str,
    new_rel: &str,
    is_dir: bool,
) -> AppResult<()> {
    let entry_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path = $2",
    )
    .bind(bundle_id)
    .bind(old_rel)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::NotFound("file entry not found in this build".into()))?;

    let (name, category) = split_relative_path(new_rel);
    update_artifact_path_tx(tx, entry_id, new_rel, &name, &category).await?;

    if is_dir {
        let old_prefix = format!("{old_rel}/");
        let children = sqlx::query_as::<_, BundleArtifact>(
            "SELECT * FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path LIKE $2 || '%'",
        )
        .bind(bundle_id)
        .bind(&old_prefix)
        .fetch_all(&mut **tx)
        .await?;
        for child in children {
            if let Some(suffix) = child.relative_path.strip_prefix(&old_prefix) {
                let child_new = format!("{new_rel}/{suffix}");
                let (child_name, child_category) = split_relative_path(&child_new);
                update_artifact_path_tx(tx, child.id, &child_new, &child_name, &child_category)
                    .await?;
            }
        }
    }

    // why: DB rows are rewritten above (still inside the uncommitted tx); the OS
    // rename is the last, only non-transactional step. If it fails we bail and the
    // caller's tx rolls back, leaving disk + DB consistent.
    let old_physical = join_files(files_root, old_rel);
    let new_physical = join_files(files_root, new_rel);
    if old_physical.exists() {
        if let Some(parent) = new_physical.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir: {e}")))?;
        }
        tokio::fs::rename(&old_physical, &new_physical)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("rename: {e}")))?;
    }

    Ok(())
}

/// Regenerate `artifacts.json` from the bundle's artifact rows (filtered to files
/// still present on disk), write it atomically, and update the bundle's
/// `files_count`, `total_size`, `status='ready'`, and `last_generated_at`.
pub async fn regenerate_manifest(
    pool: &PgPool,
    bundle: &Bundle,
    storage_root: &str,
    public_prefix: &str,
) -> AppResult<()> {
    let artifacts = list_artifacts(pool, bundle.id).await?;
    let files_root = files_path(storage_root, &bundle.slug);

    // why: cache existence per path so the predicate touches the FS once per entry.
    let mut present: HashMap<String, bool> = HashMap::new();
    for artifact in &artifacts {
        let exists = files_root
            .join(rel_to_native(&artifact.relative_path))
            .exists();
        present.insert(artifact.relative_path.clone(), exists);
    }

    let built = build_manifest(&artifacts, &bundle.slug, public_prefix, |a| {
        present.get(&a.relative_path).copied().unwrap_or(false)
    });

    let json = to_json(&built.manifest);
    write_manifest_atomic(storage_root, &bundle.slug, &json)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write manifest: {e}")))?;

    sqlx::query(
        r#"
        UPDATE bundles
        SET status = 'ready', files_count = $2, total_size = $3,
            last_generated_at = now(), processing_error = NULL, updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(bundle.id)
    .bind(built.files_count)
    .bind(built.total_size)
    .execute(pool)
    .await?;

    Ok(())
}

fn rel_to_native(relative_path: &str) -> std::path::PathBuf {
    let mut path = std::path::PathBuf::new();
    for segment in relative_path.split('/') {
        path.push(segment);
    }
    path
}

/// Native-path join of a build's files root and a forward-slash relative path.
pub fn join_files(files_root: &Path, relative_path: &str) -> std::path::PathBuf {
    files_root.join(rel_to_native(relative_path))
}
