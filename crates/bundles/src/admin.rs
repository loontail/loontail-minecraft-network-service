//! Admin (AdminUser-guarded) bundle management: CRUD over builds, ZIP/file/folder
//! ingest, per-file operations, validation, manifest regeneration, and disk space.

use axum::extract::{Multipart, Path, State};
use axum::Json;
use uuid::Uuid;

use loontail_core::auth::AdminUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::archive::{extract_zip, hash_file, scan_directory};
use crate::models::{
    BulkDelete, Bundle, BundleWithArtifacts, CreateBundle, CreateFolder, DiskSpace, MissingEntry,
    OrphanEntry, RenameFile, ToggleDownloadOnce, UpdateBundle, ValidateResult,
};
use crate::storage::{
    delete_build_files, disk_space, ensure_build_dir, files_path, normalize_relative_path,
    split_relative_path,
};
use crate::{repo, storage};

fn storage_root(state: &AppState) -> &str {
    &state.config.bundles.storage_root
}

fn public_prefix(state: &AppState) -> &str {
    &state.config.bundles.public_url
}

async fn regenerate(state: &AppState, bundle: &Bundle) -> AppResult<()> {
    repo::regenerate_manifest(
        &state.pool,
        bundle,
        storage_root(state),
        public_prefix(state),
    )
    .await
}

/// `GET /builds` — all builds, newest first.
pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<Bundle>>> {
    Ok(Json(repo::list_bundles(&state.pool).await?))
}

/// `POST /builds` — create a draft build and its on-disk directory.
pub async fn create(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateBundle>,
) -> AppResult<(axum::http::StatusCode, Json<Bundle>)> {
    if body.name.trim().is_empty() || body.slug.trim().is_empty() {
        return Err(AppError::BadRequest("name and slug are required".into()));
    }
    if repo::find_by_slug(&state.pool, &body.slug).await?.is_some() {
        return Err(AppError::Conflict(
            "a build with this slug already exists".into(),
        ));
    }

    let bundle = repo::create_bundle(
        &state.pool,
        body.name.trim(),
        body.slug.trim(),
        body.description.as_deref(),
        body.version.as_deref(),
    )
    .await?;

    ensure_build_dir(storage_root(&state), &bundle.slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir build dir: {e}")))?;

    Ok((axum::http::StatusCode::CREATED, Json(bundle)))
}

/// `GET /builds/{slug}` — build metadata plus its ordered artifact rows.
pub async fn get(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<BundleWithArtifacts>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let artifacts = repo::list_artifacts(&state.pool, bundle.id).await?;
    Ok(Json(BundleWithArtifacts { bundle, artifacts }))
}

/// `PUT /builds/{slug}` — update name/description/version.
pub async fn update(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<UpdateBundle>,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let updated = repo::update_bundle_meta(
        &state.pool,
        bundle.id,
        body.name.as_deref(),
        body.description.as_deref(),
        body.version.as_deref(),
    )
    .await?;
    Ok(Json(updated))
}

/// `DELETE /builds/{slug}` — drop artifact rows, on-disk files, and the bundle.
pub async fn delete(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    delete_build_files(storage_root(&state), &slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("delete build files: {e}")))?;
    repo::delete_bundle(&state.pool, bundle.id).await?;
    Ok(Json(serde_json::json!({ "message": "build deleted" })))
}

/// `POST /builds/{slug}/upload` — multipart ZIP under form field `archive`. The
/// stream is written to a temp file then extracted (so we never buffer 10 GB),
/// the tree is scanned with streamed SHA-256, artifacts upserted, manifest
/// regenerated. Status walks draft→processing→ready (or →failed).
pub async fn upload_archive(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;

    let mut archive_field: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart: {e}")))?
    {
        if field.name() == Some("archive") {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("read archive field: {e}")))?;
            archive_field = Some(bytes.to_vec());
            break;
        }
    }

    let bytes = archive_field.ok_or_else(|| {
        AppError::BadRequest(
            "no archive file provided — send the ZIP as form field \"archive\"".into(),
        )
    })?;

    repo::set_status(&state.pool, bundle.id, "processing", None).await?;

    match ingest_archive(&state, &bundle, &bytes).await {
        Ok(()) => {
            let refreshed = repo::require_by_slug(&state.pool, &slug).await?;
            Ok(Json(refreshed))
        }
        Err(err) => {
            let message = err.to_string();
            repo::set_status(&state.pool, bundle.id, "failed", Some(&message)).await?;
            Err(err)
        }
    }
}

async fn ingest_archive(state: &AppState, bundle: &Bundle, bytes: &[u8]) -> AppResult<()> {
    let root = storage_root(state);
    ensure_build_dir(root, &bundle.slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;
    let files_root = files_path(root, &bundle.slug);

    // why: persist the upload to a temp file so extraction streams from disk and
    // never holds the whole (up to 10 GB) archive plus extracted tree in memory.
    let tmp = tempfile_for(root, &bundle.slug)?;
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write temp archive: {e}")))?;

    let tmp_for_task = tmp.clone();
    let files_for_task = files_root.clone();
    // Extraction + hashing are blocking; run off the async runtime.
    let scan = tokio::task::spawn_blocking(move || -> AppResult<_> {
        extract_zip(&tmp_for_task, &files_for_task)?;
        let _ = std::fs::remove_file(&tmp_for_task);
        scan_directory(&files_for_task)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("extract task: {e}")))??;

    repo::upsert_scan(&state.pool, bundle.id, &scan).await?;
    regenerate(state, bundle).await?;
    Ok(())
}

fn tempfile_for(storage_root: &str, slug: &str) -> AppResult<std::path::PathBuf> {
    let dir = storage::build_path(storage_root, slug);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir for temp: {e}")))?;
    Ok(dir.join(format!("upload-{}.zip.tmp", Uuid::new_v4())))
}

/// `POST /builds/{slug}/files` — upload a single file to `targetPath` (form field
/// `file`, optional text field `targetPath`).
pub async fn upload_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;

    let mut data: Option<Vec<u8>> = None;
    let mut target_path: Option<String> = None;
    let mut original_filename: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart: {e}")))?
    {
        match field.name() {
            Some("file") => {
                original_filename = field.file_name().map(str::to_string);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("read file field: {e}")))?;
                data = Some(bytes.to_vec());
            }
            Some("targetPath") => {
                target_path = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("read targetPath: {e}")))?,
                );
            }
            _ => {}
        }
    }

    let bytes = data.ok_or_else(|| {
        AppError::BadRequest("no file provided — send the file as form field \"file\"".into())
    })?;
    let raw_target = target_path
        .filter(|p| !p.trim().is_empty())
        .or(original_filename)
        .ok_or_else(|| AppError::BadRequest("targetPath or a filename is required".into()))?;

    let normalized = normalize_relative_path(&raw_target, "targetPath")?;
    let root = storage_root(&state);
    let files_root = files_path(root, &slug);
    let dest = repo::join_files(&files_root, &normalized);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir: {e}")))?;
    }
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write file: {e}")))?;

    let dest_for_hash = dest.clone();
    let sha256 = tokio::task::spawn_blocking(move || hash_file(&dest_for_hash))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash task: {e}")))??;
    let modified = tokio::fs::metadata(&dest)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .map(chrono::DateTime::<chrono::Utc>::from);
    let (name, category) = split_relative_path(&normalized);

    repo::upsert_artifact(
        &state.pool,
        bundle.id,
        &normalized,
        &name,
        &category,
        bytes.len() as i64,
        Some(&sha256),
        false,
        modified,
    )
    .await?;

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// `POST /builds/{slug}/folders` — create a folder (and ancestor folder rows).
pub async fn create_folder(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<CreateFolder>,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let normalized = normalize_relative_path(body.relative_path.trim(), "relativePath")?;

    let root = storage_root(&state);
    ensure_build_dir(root, &slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;
    let files_root = files_path(root, &slug);
    let full = repo::join_files(&files_root, &normalized);

    if full.is_file() {
        return Err(AppError::BadRequest(
            "a file already exists at that path".into(),
        ));
    }
    tokio::fs::create_dir_all(&full)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir: {e}")))?;

    // Create a dir artifact row for the folder and each ancestor segment.
    let segments: Vec<&str> = normalized.split('/').collect();
    for depth in 1..=segments.len() {
        let ancestor = segments[..depth].join("/");
        let (name, category) = split_relative_path(&ancestor);
        repo::upsert_artifact(
            &state.pool,
            bundle.id,
            &ancestor,
            &name,
            &category,
            0,
            None,
            true,
            None,
        )
        .await?;
    }

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// `DELETE /builds/{slug}/files/{entryId}` — delete a file or folder (and its
/// descendants) on disk and in the DB.
pub async fn delete_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;

    let files_root = files_path(storage_root(&state), &slug);
    let path = repo::join_files(&files_root, &entry.relative_path);

    if entry.is_dir {
        if path.exists() {
            tokio::fs::remove_dir_all(&path)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("rmdir: {e}")))?;
        }
        let prefix = format!("{}/", entry.relative_path);
        repo::delete_artifacts_with_prefix(&state.pool, bundle.id, &prefix).await?;
    } else if path.exists() {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("rm: {e}")))?;
    }

    repo::delete_artifact(&state.pool, entry.id).await?;
    regenerate(&state, &bundle).await?;

    let message = if entry.is_dir {
        "folder deleted"
    } else {
        "file deleted"
    };
    Ok(Json(
        serde_json::json!({ "message": message, "slug": slug }),
    ))
}

/// `PUT /builds/{slug}/files/{entryId}` — toggle the `downloadOnce` flag.
pub async fn toggle_download_once(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
    Json(body): Json<ToggleDownloadOnce>,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;
    repo::set_download_once(&state.pool, entry.id, body.download_once).await?;
    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// `POST /builds/{slug}/files/{entryId}/rename` — move/rename a file or folder
/// (descendant rows follow).
pub async fn rename_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
    Json(body): Json<RenameFile>,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;
    let normalized = normalize_relative_path(body.new_relative_path.trim(), "newRelativePath")?;

    let files_root = files_path(storage_root(&state), &slug);
    let old_physical = repo::join_files(&files_root, &entry.relative_path);
    let new_physical = repo::join_files(&files_root, &normalized);

    if new_physical.exists() && normalized != entry.relative_path {
        return Err(AppError::Conflict(
            "a file or folder already exists at that path".into(),
        ));
    }

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

    let (name, category) = split_relative_path(&normalized);
    repo::update_artifact_path(&state.pool, entry.id, &normalized, &name, &category).await?;

    if entry.is_dir {
        let old_prefix = format!("{}/", entry.relative_path);
        let children = repo::list_artifacts(&state.pool, bundle.id).await?;
        for child in children {
            if let Some(suffix) = child.relative_path.strip_prefix(&old_prefix) {
                let child_new = format!("{normalized}/{suffix}");
                let (child_name, child_category) = split_relative_path(&child_new);
                repo::update_artifact_path(
                    &state.pool,
                    child.id,
                    &child_new,
                    &child_name,
                    &child_category,
                )
                .await?;
            }
        }
    }

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// `POST /builds/{slug}/files/{entryId}/rehash` — recompute a file's SHA-256.
pub async fn rehash_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;
    if entry.is_dir {
        return Err(AppError::BadRequest("cannot rehash a directory".into()));
    }

    let files_root = files_path(storage_root(&state), &slug);
    let path = repo::join_files(&files_root, &entry.relative_path);
    if !path.exists() {
        return Err(AppError::BadRequest(
            "physical file not found on disk".into(),
        ));
    }

    let path_for_hash = path.clone();
    let sha256 = tokio::task::spawn_blocking(move || hash_file(&path_for_hash))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash task: {e}")))??;
    repo::update_artifact_sha(&state.pool, entry.id, &sha256).await?;
    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// `POST /builds/{slug}/files/bulk-delete` — delete many entries by id.
pub async fn bulk_delete(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<BulkDelete>,
) -> AppResult<Json<serde_json::Value>> {
    if body.ids.is_empty() {
        return Err(AppError::BadRequest("ids must be a non-empty array".into()));
    }
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let files_root = files_path(storage_root(&state), &slug);

    let mut deleted = 0u64;
    for id in body.ids {
        let Some(entry) = repo::find_artifact(&state.pool, id).await? else {
            continue;
        };
        if entry.bundle_id != bundle.id {
            continue;
        }
        let path = repo::join_files(&files_root, &entry.relative_path);
        if entry.is_dir {
            if path.exists() {
                let _ = tokio::fs::remove_dir_all(&path).await;
            }
            let prefix = format!("{}/", entry.relative_path);
            repo::delete_artifacts_with_prefix(&state.pool, bundle.id, &prefix).await?;
        } else if path.exists() {
            let _ = tokio::fs::remove_file(&path).await;
        }
        repo::delete_artifact(&state.pool, entry.id).await?;
        deleted += 1;
    }

    regenerate(&state, &bundle).await?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

/// `POST /builds/{slug}/validate` — artifact rows whose file is gone (`missing`)
/// and on-disk files no row tracks (`orphaned`).
pub async fn validate(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<ValidateResult>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let artifacts = repo::list_artifacts(&state.pool, bundle.id).await?;
    let files_root = files_path(storage_root(&state), &slug);

    let mut missing = Vec::new();
    let mut tracked = std::collections::HashSet::new();
    for artifact in &artifacts {
        if artifact.is_dir {
            continue;
        }
        tracked.insert(artifact.relative_path.clone());
        if !repo::join_files(&files_root, &artifact.relative_path).exists() {
            missing.push(MissingEntry {
                id: artifact.id,
                relative_path: artifact.relative_path.clone(),
                name: artifact.name.clone(),
            });
        }
    }

    let mut orphaned = Vec::new();
    let scan = scan_directory(&files_root)?;
    for entry in scan {
        if entry.is_dir {
            continue;
        }
        if !tracked.contains(&entry.relative_path) {
            orphaned.push(OrphanEntry {
                relative_path: entry.relative_path,
            });
        }
    }

    Ok(Json(ValidateResult { missing, orphaned }))
}

/// `POST /builds/{slug}/regenerate` — rebuild `artifacts.json` from the rows and
/// flip status to ready (or failed).
pub async fn regenerate_manifest(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<Bundle>> {
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    repo::set_status(&state.pool, bundle.id, "processing", None).await?;
    match regenerate(&state, &bundle).await {
        Ok(()) => Ok(Json(repo::require_by_slug(&state.pool, &slug).await?)),
        Err(err) => {
            let message = err.to_string();
            repo::set_status(&state.pool, bundle.id, "failed", Some(&message)).await?;
            Err(err)
        }
    }
}

/// `GET /disk-space` — free/total bytes on the storage volume.
pub async fn disk_space_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<DiskSpace>> {
    let (free, total) = disk_space(storage_root(&state));
    Ok(Json(DiskSpace { free, total }))
}

async fn artifact_in_bundle(
    state: &AppState,
    bundle_id: Uuid,
    entry_id: Uuid,
) -> AppResult<crate::models::BundleArtifact> {
    let entry = repo::find_artifact(&state.pool, entry_id)
        .await?
        .filter(|a| a.bundle_id == bundle_id)
        .ok_or_else(|| AppError::NotFound("file entry not found in this build".into()))?;
    Ok(entry)
}
