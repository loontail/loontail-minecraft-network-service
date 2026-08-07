//! Database access for bundles and their artifacts, plus the manifest regeneration
//! routine that ties on-disk scanning to `artifacts.json`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use loontail_core::error::{AppError, AppResult};
use loontail_core::models::escape_like_pattern;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use crate::archive::ScanEntry;
use crate::manifest::{build_manifest, to_json};
use crate::models::{Bundle, BundleArtifact, BUNDLE_ARTIFACT_COLS, BUNDLE_COLS};
use crate::storage::{files_path, split_relative_path, write_manifest_atomic};

pub async fn list_bundles(pool: &PgPool) -> AppResult<Vec<Bundle>> {
    Ok(sqlx::query_as::<_, Bundle>(AssertSqlSafe(format!(
        "SELECT {BUNDLE_COLS} FROM bundles ORDER BY created_at DESC"
    )))
    .fetch_all(pool)
    .await?)
}

pub async fn find_by_slug(pool: &PgPool, slug: &str) -> AppResult<Option<Bundle>> {
    Ok(sqlx::query_as::<_, Bundle>(AssertSqlSafe(format!(
        "SELECT {BUNDLE_COLS} FROM bundles WHERE slug = $1"
    )))
    .bind(slug)
    .fetch_optional(pool)
    .await?)
}

/// An existing bundle's id by slug, or `None`. The slug is the cross-crate link key
/// (a catalog client owns the bundle whose slug matches it). Runs on any executor.
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

/// Insert a draft `bundles` row and return its id — the DB half of provisioning,
/// with no filesystem side effect, so it runs inside the caller's transaction. A
/// pre-existing slug is [`AppError::Conflict`]. Pair with [`ensure_bundle_dir`]
/// after commit.
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

/// Create a bundle's on-disk `files` directory (idempotent). Run after the row is
/// committed so a rolled-back provision leaves no stray directory.
pub fn ensure_bundle_dir(storage_root: &str, slug: &str) -> AppResult<()> {
    crate::storage::ensure_build_dir(storage_root, slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir build dir: {e}")))
}

/// Provision a draft bundle: insert the row and create its on-disk directory,
/// returning the new id. A pre-existing slug is [`AppError::Conflict`] (never a
/// silent collision).
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
    // why: COALESCE keeps the existing `name` when omitted, but description/version
    // are nullable and bound directly so they can be intentionally cleared.
    Ok(sqlx::query_as::<_, Bundle>(AssertSqlSafe(format!(
        r#"
        UPDATE bundles
        SET name = COALESCE($2, name),
            description = $3,
            version = $4,
            updated_at = now()
        WHERE id = $1
        RETURNING {BUNDLE_COLS}
        "#
    )))
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(version)
    .fetch_one(pool)
    .await?)
}

/// A bundle's slug by id (any executor). `None` when no such bundle exists.
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

/// How many artifact rows a bundle holds (any executor). Lets a caller tell an empty
/// bundle from one holding uploaded files before deciding to destroy it.
pub async fn count_artifacts<'e, E>(executor: E, bundle_id: Uuid) -> AppResult<i64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM bundle_artifacts WHERE bundle_id = $1")
            .bind(bundle_id)
            .fetch_one(executor)
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

/// Remove a build's on-disk tree (best-effort: a failed unlink is logged, never
/// propagated, so an orphaned directory can't block the authoritative row deletion).
pub fn remove_bundle_dir(storage_root: &str, slug: &str) {
    if let Err(e) = crate::storage::delete_build_files(storage_root, slug) {
        tracing::warn!(slug, error = %e, "failed to delete on-disk build files");
    }
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
    Ok(sqlx::query_as::<_, BundleArtifact>(AssertSqlSafe(format!(
        "SELECT {BUNDLE_ARTIFACT_COLS} FROM bundle_artifacts WHERE bundle_id = $1 ORDER BY relative_path ASC"
    )))
    .bind(bundle_id)
    .fetch_all(pool)
    .await?)
}

pub async fn find_artifact(pool: &PgPool, id: Uuid) -> AppResult<Option<BundleArtifact>> {
    Ok(sqlx::query_as::<_, BundleArtifact>(AssertSqlSafe(format!(
        "SELECT {BUNDLE_ARTIFACT_COLS} FROM bundle_artifacts WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// Upsert one file/dir artifact by `(bundle_id, relative_path)`. A new row inserts
/// `download_once = false`; an existing row's `download_once` is deliberately left
/// untouched by the `DO UPDATE`, preserving an operator's toggle across rescans.
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

/// Rows per bulk-upsert statement. Bounds the per-statement parameter payload while
/// keeping the round-trip count at `ceil(entries / CHUNK)` instead of one per entry.
const UPSERT_CHUNK: usize = 10_000;

/// Replace a bundle's artifact rows with a freshly scanned set in one transaction:
/// upsert every scanned entry (preserving each existing row's `download_once`), then
/// delete any row absent from the new scan (an empty scan clears all rows).
pub async fn upsert_scan(pool: &PgPool, bundle_id: Uuid, scan: &[ScanEntry]) -> AppResult<()> {
    let mut tx = pool.begin().await?;

    for chunk in scan.chunks(UPSERT_CHUNK) {
        upsert_artifact_chunk(&mut tx, bundle_id, chunk).await?;
    }

    let paths: Vec<&str> = scan.iter().map(|e| e.relative_path.as_str()).collect();
    sqlx::query(
        "DELETE FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path <> ALL($2::text[])",
    )
    .bind(bundle_id)
    .bind(&paths)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Upsert a whole chunk of scanned entries in ONE statement via column arrays, with
/// the same `ON CONFLICT` semantics as [`upsert_artifact`] — `download_once` is
/// deliberately absent from the `DO UPDATE`, so an operator's toggle survives a rescan.
///
/// why: `ON CONFLICT DO UPDATE` cannot touch the same row twice in one statement, so
/// `chunk` must not repeat a `relative_path`. A filesystem walk (the only producer,
/// `archive::scan_directory`) cannot yield a duplicate path.
async fn upsert_artifact_chunk(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bundle_id: Uuid,
    chunk: &[ScanEntry],
) -> AppResult<()> {
    if chunk.is_empty() {
        return Ok(());
    }

    let mut relative_paths: Vec<&str> = Vec::with_capacity(chunk.len());
    let mut names: Vec<&str> = Vec::with_capacity(chunk.len());
    let mut categories: Vec<&str> = Vec::with_capacity(chunk.len());
    let mut sizes: Vec<i64> = Vec::with_capacity(chunk.len());
    let mut hashes: Vec<Option<&str>> = Vec::with_capacity(chunk.len());
    let mut dirs: Vec<bool> = Vec::with_capacity(chunk.len());
    let mut modified: Vec<Option<DateTime<Utc>>> = Vec::with_capacity(chunk.len());

    for entry in chunk {
        relative_paths.push(&entry.relative_path);
        names.push(&entry.name);
        categories.push(&entry.category);
        sizes.push(entry.size);
        hashes.push(entry.sha256.as_deref());
        dirs.push(entry.is_dir);
        modified.push(entry.file_modified_at);
    }

    sqlx::query(
        r#"
        INSERT INTO bundle_artifacts
            (bundle_id, relative_path, name, category, size, sha256, is_dir, download_once, file_modified_at)
        SELECT $1, t.relative_path, t.name, t.category, t.size, t.sha256, t.is_dir, false,
               t.file_modified_at
        FROM UNNEST($2::text[], $3::text[], $4::text[], $5::int8[], $6::text[], $7::bool[],
                    $8::timestamptz[])
             AS t(relative_path, name, category, size, sha256, is_dir, file_modified_at)
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
    .bind(&relative_paths)
    .bind(&names)
    .bind(&categories)
    .bind(&sizes)
    .bind(&hashes)
    .bind(&dirs)
    .bind(&modified)
    .execute(&mut **tx)
    .await?;
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

/// Delete one artifact row (any executor, so a batch delete runs in one transaction).
pub async fn delete_artifact<'e, E>(executor: E, id: Uuid) -> AppResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("DELETE FROM bundle_artifacts WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Delete every artifact row under `prefix` (pass it with a trailing `/`). The
/// prefix is LIKE-escaped, so a folder named `mod_config` or `%` matches only
/// itself and never its siblings. Runs on any executor.
pub async fn delete_artifacts_with_prefix<'e, E>(
    executor: E,
    bundle_id: Uuid,
    prefix: &str,
) -> AppResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        r#"DELETE FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path LIKE $2 || '%' ESCAPE '\'"#,
    )
    .bind(bundle_id)
    .bind(escape_like_pattern(prefix))
    .execute(executor)
    .await?;
    Ok(())
}

/// `true` when an artifact row already occupies `relative_path` in this bundle, so a
/// move/rename can return a clean 409 instead of hitting the unique index. Runs on
/// any executor.
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
/// trailing `/`), so a folder move into an already-populated destination is a 409.
/// The prefix is LIKE-escaped, so `_`/`%` in a folder name cannot phantom-match a
/// sibling directory.
pub async fn any_artifact_with_prefix<'e, E>(
    executor: E,
    bundle_id: Uuid,
    prefix: &str,
) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let count = sqlx::query_scalar::<_, i64>(
        r#"SELECT count(*) FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path LIKE $2 || '%' ESCAPE '\'"#,
    )
    .bind(bundle_id)
    .bind(escape_like_pattern(prefix))
    .fetch_one(executor)
    .await?;
    Ok(count > 0)
}

/// Update one artifact row's path on a transaction executor.
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
/// path plus every descendant's (prefix `old_rel` -> `new_rel`), then perform the
/// single on-disk rename. All DB writes run inside the passed transaction.
///
/// Ordering is load-bearing: DB rewrites happen FIRST, the OS rename LAST and while
/// the tx is still uncommitted, so if the rename fails we return early and the
/// caller's tx rolls back every row change — disk and DB stay consistent.
///
/// Caller owns the conflict + self-into-subtree guards (this fn assumes a legal move).
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
        let children = sqlx::query_as::<_, BundleArtifact>(AssertSqlSafe(format!(
            r#"SELECT {BUNDLE_ARTIFACT_COLS} FROM bundle_artifacts
               WHERE bundle_id = $1 AND relative_path LIKE $2 || '%' ESCAPE '\'"#
        )))
        .bind(bundle_id)
        .bind(escape_like_pattern(&old_prefix))
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

    // why: OS rename is the last, only non-transactional step — on failure we bail and
    // the caller's uncommitted tx rolls back the row changes above.
    let old_physical = join_files(files_root, old_rel);
    let new_physical = join_files(files_root, new_rel);
    if tokio::fs::try_exists(&old_physical).await.unwrap_or(false) {
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
/// `files_count`, `total_size`, `status='ready'`, and `last_generated_at`. Returns the
/// updated row so callers need no follow-up read.
///
/// The whole filesystem half (one `stat` per artifact plus the atomic manifest write)
/// runs on a blocking thread: this fires after EVERY per-file admin op, so doing it
/// inline would park a runtime worker for N syscalls on each admin click.
pub async fn regenerate_manifest(
    pool: &PgPool,
    bundle: &Bundle,
    storage_root: &str,
    public_prefix: &str,
) -> AppResult<Bundle> {
    let artifacts = list_artifacts(pool, bundle.id).await?;
    let files_root = files_path(storage_root, &bundle.slug);

    let probe_paths: Vec<String> = artifacts.iter().map(|a| a.relative_path.clone()).collect();
    let present = tokio::task::spawn_blocking(move || present_paths(&files_root, &probe_paths))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("manifest probe task: {e}")))?;

    let built = build_manifest(&artifacts, &bundle.slug, public_prefix, |a| {
        present.contains(&a.relative_path)
    });

    let json = to_json(&built.manifest);
    let root = storage_root.to_string();
    let slug = bundle.slug.clone();
    tokio::task::spawn_blocking(move || write_manifest_atomic(&root, &slug, &json))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("manifest write task: {e}")))?
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write manifest: {e}")))?;

    Ok(sqlx::query_as::<_, Bundle>(AssertSqlSafe(format!(
        r#"
        UPDATE bundles
        SET status = 'ready', files_count = $2, total_size = $3,
            last_generated_at = now(), processing_error = NULL, updated_at = now()
        WHERE id = $1
        RETURNING {BUNDLE_COLS}
        "#
    )))
    .bind(bundle.id)
    .bind(built.files_count)
    .bind(built.total_size)
    .fetch_one(pool)
    .await?)
}

/// The subset of `relative_paths` that exist under `files_root`. Blocking (one `stat`
/// per path) — call from `spawn_blocking`.
fn present_paths(files_root: &Path, relative_paths: &[String]) -> HashSet<String> {
    relative_paths
        .iter()
        .filter(|rel| join_files(files_root, rel).exists())
        .cloned()
        .collect()
}

fn rel_to_native(relative_path: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in relative_path.split('/') {
        path.push(segment);
    }
    path
}

/// Native-path join of a build's files root and a forward-slash relative path.
pub fn join_files(files_root: &Path, relative_path: &str) -> PathBuf {
    files_root.join(rel_to_native(relative_path))
}
