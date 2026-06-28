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
    BulkDelete, Bundle, BundleArtifact, BundleWithArtifacts, CreateBundle, CreateFolder, DiskSpace,
    MissingEntry, MoveFile, MoveFiles, OrphanEntry, RenameFile, ToggleDownloadOnce, UpdateBundle,
    ValidateResult,
};
use crate::storage::{
    delete_build_files, disk_space, ensure_build_dir, files_path, normalize_relative_path,
    split_relative_path, validate_slug,
};
use crate::{repo, storage, MAX_UPLOAD_BYTES};

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

/// All builds, newest first.
pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<Bundle>>> {
    Ok(Json(repo::list_bundles(&state.pool).await?))
}

/// Create a draft build and its on-disk directory via [`repo::provision_bundle`]
/// (the same path the catalog uses to auto-provision a build's owned bundle), then
/// apply any supplied description/version.
pub async fn create(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateBundle>,
) -> AppResult<(axum::http::StatusCode, Json<Bundle>)> {
    if body.name.trim().is_empty() || body.slug.trim().is_empty() {
        return Err(AppError::BadRequest("name and slug are required".into()));
    }

    let id = repo::provision_bundle(
        &state.pool,
        storage_root(&state),
        body.slug.trim(),
        body.name.trim(),
    )
    .await?;

    let bundle = if body.description.is_some() || body.version.is_some() {
        repo::update_bundle_meta(
            &state.pool,
            id,
            None,
            body.description.as_deref(),
            body.version.as_deref(),
        )
        .await?
    } else {
        repo::require_by_slug(&state.pool, body.slug.trim()).await?
    };

    Ok((axum::http::StatusCode::CREATED, Json(bundle)))
}

/// Build metadata plus its ordered artifact rows.
pub async fn get(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<BundleWithArtifacts>> {
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let artifacts = repo::list_artifacts(&state.pool, bundle.id).await?;
    Ok(Json(BundleWithArtifacts { bundle, artifacts }))
}

pub async fn update(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<UpdateBundle>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
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

/// Drop artifact rows, on-disk files, and the bundle.
pub async fn delete(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    // why (SEC-6): reject a traversal slug before any FS join.
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    delete_build_files(storage_root(&state), &slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("delete build files: {e}")))?;
    repo::delete_bundle(&state.pool, bundle.id).await?;
    Ok(Json(serde_json::json!({ "message": "build deleted" })))
}

/// Ingest a multipart ZIP (form field `archive`): stream it to a temp file, extract,
/// scan with streamed SHA-256, upsert artifacts, regenerate the manifest. Status
/// walks draft→processing→ready (or →failed).
pub async fn upload_archive(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;

    let root = storage_root(&state);
    ensure_build_dir(root, &bundle.slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;

    // why: stream to a temp file with a running byte cap so we never buffer the whole
    // (up to 10 GiB) upload in RAM.
    let tmp = tempfile_for(root, &bundle.slug)?;
    let mut have_archive = false;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart: {e}")))?
    {
        if field.name() == Some("archive") {
            if let Err(err) = stream_field_to_file(field, &tmp).await {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(err);
            }
            have_archive = true;
            break;
        }
    }

    if !have_archive {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(AppError::BadRequest(
            "no archive file provided — send the ZIP as form field \"archive\"".into(),
        ));
    }

    repo::set_status(&state.pool, bundle.id, "processing", None).await?;

    match ingest_archive(&state, &bundle, &tmp).await {
        Ok(()) => {
            let refreshed = repo::require_by_slug(&state.pool, &slug).await?;
            Ok(Json(refreshed))
        }
        Err(err) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            let message = err.to_string();
            repo::set_status(&state.pool, bundle.id, "failed", Some(&message)).await?;
            Err(err)
        }
    }
}

async fn ingest_archive(state: &AppState, bundle: &Bundle, tmp: &std::path::Path) -> AppResult<()> {
    let root = storage_root(state);
    let files_root = files_path(root, &bundle.slug);

    let tmp_for_task = tmp.to_path_buf();
    let files_for_task = files_root.clone();
    // Extraction + hashing are blocking; run off the async runtime.
    let scan = tokio::task::spawn_blocking(move || -> AppResult<_> {
        // why: a re-upload fully replaces the build, so wipe files/ first (else a file
        // dropped from the new ZIP lingers). Only files/ is cleared — the temp .zip.tmp
        // is a sibling in the build root, so it survives.
        if files_for_task.exists() {
            std::fs::remove_dir_all(&files_for_task)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("clear files dir: {e}")))?;
        }
        std::fs::create_dir_all(&files_for_task)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("recreate files dir: {e}")))?;
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

/// Stream a multipart `field` to `dest` chunk by chunk, aborting with a 400 once the
/// running byte total exceeds [`MAX_UPLOAD_BYTES`]. Peak memory stays a single chunk.
async fn stream_field_to_file(
    mut field: axum::extract::multipart::Field<'_>,
    dest: &std::path::Path,
) -> AppResult<u64> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("create temp upload: {e}")))?;
    let mut written: u64 = 0;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::BadRequest(format!("read upload field: {e}")))?
    {
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_UPLOAD_BYTES {
            return Err(AppError::BadRequest(format!(
                "upload exceeds {MAX_UPLOAD_BYTES} bytes"
            )));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("write upload: {e}")))?;
    }
    file.flush()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("flush upload: {e}")))?;
    Ok(written)
}

/// Upload a single file to `targetPath` (form field `file`, optional text field
/// `targetPath`).
pub async fn upload_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;

    let root = storage_root(&state);
    ensure_build_dir(root, &slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;

    // why: stream to a temp file with a running cap, then move it into place once
    // `targetPath` is known.
    let tmp = tempfile_for(root, &slug)?;
    let mut size: Option<u64> = None;
    let mut target_path: Option<String> = None;
    let mut original_filename: Option<String> = None;

    let result: AppResult<()> =
        async {
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid multipart: {e}")))?
            {
                match field.name() {
                    Some("file") => {
                        original_filename = field.file_name().map(str::to_string);
                        size = Some(stream_field_to_file(field, &tmp).await?);
                    }
                    Some("targetPath") => {
                        target_path =
                            Some(field.text().await.map_err(|e| {
                                AppError::BadRequest(format!("read targetPath: {e}"))
                            })?);
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        .await;
    if let Err(err) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err);
    }

    let size = match size {
        Some(s) => s,
        None => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(AppError::BadRequest(
                "no file provided — send the file as form field \"file\"".into(),
            ));
        }
    };
    let raw_target = match target_path
        .filter(|p| !p.trim().is_empty())
        .or(original_filename)
    {
        Some(t) => t,
        None => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(AppError::BadRequest(
                "targetPath or a filename is required".into(),
            ));
        }
    };

    let normalized = match normalize_relative_path(&raw_target, "targetPath") {
        Ok(n) => n,
        Err(err) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(err);
        }
    };
    let files_root = files_path(root, &slug);
    let dest = repo::join_files(&files_root, &normalized);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir: {e}")))?;
    }
    tokio::fs::rename(&tmp, &dest)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("move file into place: {e}")))?;

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
        size as i64,
        Some(&sha256),
        false,
        modified,
    )
    .await?;

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// Create a folder (and ancestor folder rows).
pub async fn create_folder(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<CreateFolder>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
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

/// Delete a file or folder (and its descendants) on disk and in the DB.
pub async fn delete_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    validate_slug(&slug)?;
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

/// Toggle the `downloadOnce` flag.
pub async fn toggle_download_once(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
    Json(body): Json<ToggleDownloadOnce>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;
    repo::set_download_once(&state.pool, entry.id, body.download_once).await?;
    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// Move/rename a file or folder (descendant rows follow), sharing the hardened
/// [`repo::move_subtree`] path with `move`: DB-aware conflict (409), self-into-subtree
/// guard, atomic tx.
pub async fn rename_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
    Json(body): Json<RenameFile>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;
    let normalized = normalize_relative_path(body.new_relative_path.trim(), "newRelativePath")?;

    let files_root = files_path(storage_root(&state), &slug);
    apply_move(&state, &bundle, &files_root, &entry, &normalized).await?;

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// Move a single entry into `targetDir` (`""` = build root); new path is
/// `join(targetDir, name)`.
pub async fn move_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
    Json(body): Json<MoveFile>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let entry = artifact_in_bundle(&state, bundle.id, entry_id).await?;

    let new_rel = join_target_dir(&body.target_dir, &entry.name)?;
    let files_root = files_path(storage_root(&state), &slug);
    apply_move(&state, &bundle, &files_root, &entry, &new_rel).await?;

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// Move many entries into `targetDir` (`""` = build root) in ONE transaction
/// (all-or-nothing: a collision aborts the whole batch with a 409), regenerating the
/// manifest once at the end.
pub async fn move_files(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<MoveFiles>,
) -> AppResult<Json<Bundle>> {
    if body.ids.is_empty() {
        return Err(AppError::BadRequest("ids must be a non-empty array".into()));
    }
    validate_slug(&slug)?;
    let bundle = repo::require_by_slug(&state.pool, &slug).await?;
    let files_root = files_path(storage_root(&state), &slug);

    // why: resolve every id up front so a bad id is a clean 404 before any row moves.
    let mut moves: Vec<(BundleArtifact, String)> = Vec::with_capacity(body.ids.len());
    for id in &body.ids {
        let entry = artifact_in_bundle(&state, bundle.id, *id).await?;
        let new_rel = join_target_dir(&body.target_dir, &entry.name)?;
        validate_move(&entry, &new_rel)?;
        moves.push((entry, new_rel));
    }

    let mut tx = state.pool.begin().await?;
    for (entry, new_rel) in &moves {
        check_destination_free(&mut tx, bundle.id, new_rel, entry.is_dir).await?;
        repo::move_subtree(
            &mut tx,
            bundle.id,
            &files_root,
            &entry.relative_path,
            new_rel,
            entry.is_dir,
        )
        .await?;
    }
    tx.commit().await?;

    regenerate(&state, &bundle).await?;
    Ok(Json(repo::require_by_slug(&state.pool, &slug).await?))
}

/// Join a `targetDir` (`""` = root) with an entry's `name` into a normalized,
/// validated relative path.
fn join_target_dir(target_dir: &str, name: &str) -> AppResult<String> {
    let target = target_dir.trim();
    let raw = if target.is_empty() {
        name.to_string()
    } else {
        format!("{target}/{name}")
    };
    normalize_relative_path(&raw, "targetDir")
}

/// Reject an illegal move before touching disk/DB: a no-op or a folder into its own
/// descendant.
fn validate_move(entry: &BundleArtifact, new_rel: &str) -> AppResult<()> {
    if new_rel == entry.relative_path {
        return Err(AppError::Conflict(
            "the entry is already at that path".into(),
        ));
    }
    if entry.is_dir && new_rel.starts_with(&format!("{}/", entry.relative_path)) {
        return Err(AppError::BadRequest(
            "cannot move a folder into its own descendant".into(),
        ));
    }
    Ok(())
}

/// Refuse with a clean 409 if a row already occupies `new_rel` (or, for a folder,
/// anything under `new_rel/`) instead of letting the unique index raise a raw 500.
/// Callers must have already passed [`validate_move`].
async fn check_destination_free(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bundle_id: Uuid,
    new_rel: &str,
    is_dir: bool,
) -> AppResult<()> {
    if repo::artifact_exists_at(&mut **tx, bundle_id, new_rel).await? {
        return Err(AppError::Conflict(
            "a file or folder already exists at that path".into(),
        ));
    }
    if is_dir {
        let new_prefix = format!("{new_rel}/");
        if repo::any_artifact_with_prefix(&mut **tx, bundle_id, &new_prefix).await? {
            return Err(AppError::Conflict(
                "a folder already exists at that path".into(),
            ));
        }
    }
    Ok(())
}

/// Run a single move inside its own transaction: guard, conflict check,
/// [`repo::move_subtree`], commit. Shared by `rename_file` and `move_file`.
async fn apply_move(
    state: &AppState,
    bundle: &Bundle,
    files_root: &std::path::Path,
    entry: &BundleArtifact,
    new_rel: &str,
) -> AppResult<()> {
    validate_move(entry, new_rel)?;

    // Physical-exists defense (kept alongside the DB-aware check below).
    let new_physical = repo::join_files(files_root, new_rel);
    if new_physical.exists() && new_rel != entry.relative_path {
        return Err(AppError::Conflict(
            "a file or folder already exists at that path".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    check_destination_free(&mut tx, bundle.id, new_rel, entry.is_dir).await?;
    repo::move_subtree(
        &mut tx,
        bundle.id,
        files_root,
        &entry.relative_path,
        new_rel,
        entry.is_dir,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Recompute a file's SHA-256.
pub async fn rehash_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((slug, entry_id)): Path<(String, Uuid)>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
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

/// Delete many entries by id.
pub async fn bulk_delete(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
    Json(body): Json<BulkDelete>,
) -> AppResult<Json<serde_json::Value>> {
    if body.ids.is_empty() {
        return Err(AppError::BadRequest("ids must be a non-empty array".into()));
    }
    validate_slug(&slug)?;
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

/// Artifact rows whose file is gone (`missing`) and on-disk files no row tracks
/// (`orphaned`).
pub async fn validate(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<ValidateResult>> {
    validate_slug(&slug)?;
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

/// Rebuild `artifacts.json` from the rows and flip status to ready (or failed).
pub async fn regenerate_manifest(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(slug): Path<String>,
) -> AppResult<Json<Bundle>> {
    validate_slug(&slug)?;
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

/// Free/total bytes on the storage volume.
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
