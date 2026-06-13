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
use crate::storage::{files_path, write_manifest_atomic};

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

pub async fn require_by_slug(pool: &PgPool, slug: &str) -> AppResult<Bundle> {
    find_by_slug(pool, slug)
        .await?
        .ok_or_else(|| AppError::NotFound("build not found".into()))
}

pub async fn create_bundle(
    pool: &PgPool,
    name: &str,
    slug: &str,
    description: Option<&str>,
    version: Option<&str>,
) -> AppResult<Bundle> {
    Ok(sqlx::query_as::<_, Bundle>(
        r#"
        INSERT INTO bundles (name, slug, description, version, status)
        VALUES ($1, $2, $3, $4, 'draft')
        RETURNING *
        "#,
    )
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(version)
    .fetch_one(pool)
    .await?)
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

/// Upsert one file/dir artifact by `(bundle_id, relative_path)`.
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
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM bundle_artifacts WHERE bundle_id = $1 AND relative_path = $2",
    )
    .bind(bundle_id)
    .bind(relative_path)
    .fetch_optional(pool)
    .await?;

    match existing {
        Some(id) => {
            sqlx::query(
                r#"
                UPDATE bundle_artifacts
                SET name = $2, category = $3, size = $4, sha256 = $5,
                    is_dir = $6, file_modified_at = $7
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(name)
            .bind(category)
            .bind(size)
            .bind(sha256)
            .bind(is_dir)
            .bind(file_modified_at)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query(
                r#"
                INSERT INTO bundle_artifacts
                    (bundle_id, relative_path, name, category, size, sha256, is_dir, download_once, file_modified_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, false, $8)
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
        }
    }
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

pub async fn update_artifact_path(
    pool: &PgPool,
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
    .execute(pool)
    .await?;
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
