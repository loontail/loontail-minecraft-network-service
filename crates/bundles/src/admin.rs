//! Admin (AdminUser-guarded) bundle management: CRUD over builds, ZIP/file/folder
//! ingest, per-file operations, validation, manifest regeneration, and disk space.

use axum::extract::{FromRequestParts, Multipart, State};
use axum::http::request::Parts;
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
    disk_space, ensure_build_dir, files_path, normalize_relative_path, split_relative_path,
    validate_slug,
};
use crate::{repo, storage, MAX_UPLOAD_BYTES};

/// The build addressed by the `{slug}` path parameter.
///
/// why (SEC-6): this is the ONLY way a handler in this module obtains a [`Bundle`],
/// and resolving one runs [`validate_slug`] first. `Path`'s percent-decoded `slug`
/// can be `../../x`, so a handler that joined it onto the storage root without the
/// guard would escape `{storage_root}/builds` — making the guard an extractor means a
/// new endpoint cannot forget it.
pub struct ResolvedBundle(pub Bundle);

impl FromRequestParts<AppState> for ResolvedBundle {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let slug = path_param(parts, "slug").await?;
        validate_slug(&slug)?;
        Ok(ResolvedBundle(
            repo::require_by_slug(&state.pool, &slug).await?,
        ))
    }
}

/// The build addressed by `{slug}` plus the one of its file entries addressed by
/// `{entryId}`. Carries [`ResolvedBundle`]'s guarantee and additionally proves the
/// entry belongs to that build, so a cross-build id is a 404 before any handler code
/// runs.
pub struct ResolvedEntry {
    pub bundle: Bundle,
    pub entry: BundleArtifact,
}

impl FromRequestParts<AppState> for ResolvedEntry {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let ResolvedBundle(bundle) = ResolvedBundle::from_request_parts(parts, state).await?;
        let entry_id: Uuid = path_param(parts, "entryId")
            .await?
            .parse()
            .map_err(|_| AppError::BadRequest("invalid entry id".into()))?;
        let entry = artifact_in_bundle(state, bundle.id, entry_id).await?;
        Ok(ResolvedEntry { bundle, entry })
    }
}

/// One artifact row, refused with a 404 unless it belongs to `bundle_id`.
async fn artifact_in_bundle(
    state: &AppState,
    bundle_id: Uuid,
    entry_id: Uuid,
) -> AppResult<BundleArtifact> {
    repo::find_artifact(&state.pool, entry_id)
        .await?
        .filter(|a| a.bundle_id == bundle_id)
        .ok_or_else(|| AppError::NotFound("file entry not found in this build".into()))
}

/// A named route parameter, percent-decoded exactly as `axum::extract::Path` decodes it.
async fn path_param(parts: &mut Parts, name: &str) -> AppResult<String> {
    let params = axum::extract::RawPathParams::from_request_parts(parts, &())
        .await
        .map_err(|err| AppError::BadRequest(err.body_text()))?;
    params
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("route has no {{{name}}} parameter")))
}

fn storage_root(state: &AppState) -> &str {
    &state.config.bundles.storage_root
}

fn public_prefix(state: &AppState) -> &str {
    &state.config.bundles.public_url
}

/// Regenerate the manifest and hand back the refreshed row, so callers return the
/// post-regeneration state without a second `SELECT`.
async fn regenerate(state: &AppState, bundle: &Bundle) -> AppResult<Bundle> {
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
    ResolvedBundle(bundle): ResolvedBundle,
) -> AppResult<Json<BundleWithArtifacts>> {
    let artifacts = repo::list_artifacts(&state.pool, bundle.id).await?;
    Ok(Json(BundleWithArtifacts { bundle, artifacts }))
}

pub async fn update(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
    Json(body): Json<UpdateBundle>,
) -> AppResult<Json<Bundle>> {
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
///
/// Ordering is load-bearing: the authoritative row goes first (CASCADE drops its
/// artifacts) and the on-disk tree follows best-effort, matching catalog's
/// `delete_client`. The inverse order can leave a `bundles` row whose
/// `files_count`/`total_size` point at an already-erased tree.
pub async fn delete(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
) -> AppResult<Json<serde_json::Value>> {
    let mut tx = state.pool.begin().await?;
    repo::delete_bundle_row(&mut *tx, bundle.id).await?;
    tx.commit().await?;

    remove_build_dir_off_thread(storage_root(&state), &bundle.slug).await;
    Ok(Json(serde_json::json!({ "message": "build deleted" })))
}

/// `repo::remove_bundle_dir` (best-effort `rm -rf` of the build tree) on a blocking
/// thread — unlinking up to 100k files must never park a runtime worker.
async fn remove_build_dir_off_thread(storage_root: &str, slug: &str) {
    let root = storage_root.to_string();
    let slug = slug.to_string();
    if let Err(err) =
        tokio::task::spawn_blocking(move || repo::remove_bundle_dir(&root, &slug)).await
    {
        tracing::warn!(error = %err, "build-file removal task failed");
    }
}

/// `tokio::fs::try_exists` with an I/O error read as "absent", so an existence probe
/// on a request path never blocks the runtime.
async fn path_exists(path: &std::path::Path) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
}

/// Ingest a multipart ZIP (form field `archive`): stream it to a temp file, extract,
/// scan with streamed SHA-256, upsert artifacts, regenerate the manifest. Status
/// walks draft→processing→ready (or →failed).
pub async fn upload_archive(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
    mut multipart: Multipart,
) -> AppResult<Json<Bundle>> {
    let root = storage_root(&state);
    ensure_build_dir(root, &bundle.slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;

    // why: stream to a temp file with a running byte cap so we never buffer the whole
    // (up to 10 GiB) upload in RAM.
    let tmp = TempUpload::new(root, &bundle.slug)?;
    let mut have_archive = false;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart: {e}")))?
    {
        if field.name() == Some("archive") {
            stream_field_to_file(field, tmp.path()).await?;
            have_archive = true;
            break;
        }
    }

    if !have_archive {
        return Err(AppError::BadRequest(
            "no archive file provided — send the ZIP as form field \"archive\"".into(),
        ));
    }

    repo::set_status(&state.pool, bundle.id, "processing", None).await?;

    match ingest_archive(&state, &bundle, tmp.path()).await {
        Ok(refreshed) => Ok(Json(refreshed)),
        Err(err) => {
            let message = err.to_string();
            repo::set_status(&state.pool, bundle.id, "failed", Some(&message)).await?;
            Err(err)
        }
    }
}

async fn ingest_archive(
    state: &AppState,
    bundle: &Bundle,
    tmp: &std::path::Path,
) -> AppResult<Bundle> {
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
    regenerate(state, bundle).await
}

/// A staged upload file that unlinks itself on drop.
///
/// The staged file holds up to [`MAX_UPLOAD_BYTES`] (10 GiB) of the storage volume, and
/// the two upload handlers have ~20 early returns between them. Making the cleanup a
/// destructor means a new early return cannot leak it — which seven hand-written
/// `remove_file` arms could not guarantee. [`keep`](Self::keep) defuses the guard on the
/// one path that moves the file into its final place.
struct TempUpload {
    path: std::path::PathBuf,
    keep: bool,
}

impl TempUpload {
    fn new(storage_root: &str, slug: &str) -> AppResult<Self> {
        let dir = storage::build_path(storage_root, slug);
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir for temp: {e}")))?;
        Ok(TempUpload {
            path: dir.join(format!("upload-{}.zip.tmp", Uuid::new_v4())),
            keep: false,
        })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Give up ownership of the file: the caller is now responsible for it (it renames
    /// it into place or hands it to the ingest task).
    fn keep(mut self) -> std::path::PathBuf {
        self.keep = true;
        std::mem::take(&mut self.path)
    }
}

impl Drop for TempUpload {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        // why: `Drop` cannot await, and an unlink is one metadata syscall regardless of
        // the file's size, so doing it synchronously here is cheaper than the detour
        // through the blocking pool `tokio::fs::remove_file` would take. A missing file
        // is not an error — the success paths already moved or consumed it.
        let _ = std::fs::remove_file(&self.path);
    }
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
    ResolvedBundle(bundle): ResolvedBundle,
    multipart: Multipart,
) -> AppResult<Json<Bundle>> {
    let root = storage_root(&state);
    ensure_build_dir(root, &bundle.slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;

    // why: stream to a temp file with a running cap, then move it into place once
    // `targetPath` is known. `TempUpload` unlinks it on every path that does not reach
    // the rename below.
    let tmp = TempUpload::new(root, &bundle.slug)?;
    let (size, target_path, original_filename) = read_upload_parts(multipart, tmp.path()).await?;

    let size = size.ok_or_else(|| {
        AppError::BadRequest("no file provided — send the file as form field \"file\"".into())
    })?;
    let raw_target = target_path
        .filter(|p| !p.trim().is_empty())
        .or(original_filename)
        .ok_or_else(|| AppError::BadRequest("targetPath or a filename is required".into()))?;

    let normalized = normalize_relative_path(&raw_target, "targetPath")?;
    let files_root = files_path(root, &bundle.slug);
    let dest = repo::join_files(&files_root, &normalized);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir: {e}")))?;
    }
    tokio::fs::rename(tmp.keep(), &dest)
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

    Ok(Json(regenerate(&state, &bundle).await?))
}

/// Drain `upload_file`'s multipart body, streaming the `file` field to `staged`.
/// Returns `(bytes written, targetPath, original filename)`.
async fn read_upload_parts(
    mut multipart: Multipart,
    staged: &std::path::Path,
) -> AppResult<(Option<u64>, Option<String>, Option<String>)> {
    let mut size = None;
    let mut target_path = None;
    let mut original_filename = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart: {e}")))?
    {
        match field.name() {
            Some("file") => {
                original_filename = field.file_name().map(str::to_string);
                size = Some(stream_field_to_file(field, staged).await?);
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

    Ok((size, target_path, original_filename))
}

/// Create a folder (and ancestor folder rows).
pub async fn create_folder(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
    Json(body): Json<CreateFolder>,
) -> AppResult<Json<Bundle>> {
    let normalized = normalize_relative_path(body.relative_path.trim(), "relativePath")?;

    let root = storage_root(&state);
    ensure_build_dir(root, &bundle.slug)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ensure dir: {e}")))?;
    let files_root = files_path(root, &bundle.slug);
    let full = repo::join_files(&files_root, &normalized);

    if tokio::fs::metadata(&full).await.is_ok_and(|m| m.is_file()) {
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

    Ok(Json(regenerate(&state, &bundle).await?))
}

/// Delete a file or folder (and its descendants) in the DB, then unlink it on disk.
///
/// Rows first, inside one transaction, then the filesystem best-effort after commit —
/// the inverse order can erase the bytes and then fail to drop the rows, leaving the
/// manifest advertising files that 404.
pub async fn delete_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedEntry { bundle, entry }: ResolvedEntry,
) -> AppResult<Json<serde_json::Value>> {
    let mut tx = state.pool.begin().await?;
    delete_entry_rows(&mut tx, bundle.id, &entry).await?;
    tx.commit().await?;

    let files_root = files_path(storage_root(&state), &bundle.slug);
    unlink_entry(&files_root, &entry).await;
    regenerate(&state, &bundle).await?;

    let message = if entry.is_dir {
        "folder deleted"
    } else {
        "file deleted"
    };
    Ok(Json(
        serde_json::json!({ "message": message, "slug": bundle.slug }),
    ))
}

/// Drop one entry's row plus, for a folder, every descendant row — on the caller's
/// transaction so a batch is all-or-nothing.
async fn delete_entry_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bundle_id: Uuid,
    entry: &BundleArtifact,
) -> AppResult<()> {
    if entry.is_dir {
        let prefix = format!("{}/", entry.relative_path);
        repo::delete_artifacts_with_prefix(&mut **tx, bundle_id, &prefix).await?;
    }
    repo::delete_artifact(&mut **tx, entry.id).await
}

/// Unlink an entry's bytes best-effort. Runs only after its rows are committed, so a
/// failed unlink leaks a file rather than stranding a row that points at nothing.
async fn unlink_entry(files_root: &std::path::Path, entry: &BundleArtifact) {
    let path = repo::join_files(files_root, &entry.relative_path);
    if !path_exists(&path).await {
        return;
    }
    let result = if entry.is_dir {
        tokio::fs::remove_dir_all(&path).await
    } else {
        tokio::fs::remove_file(&path).await
    };
    if let Err(err) = result {
        tracing::warn!(path = %path.display(), error = %err, "failed to unlink deleted entry");
    }
}

/// Toggle the `downloadOnce` flag.
pub async fn toggle_download_once(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedEntry { bundle, entry }: ResolvedEntry,
    Json(body): Json<ToggleDownloadOnce>,
) -> AppResult<Json<Bundle>> {
    repo::set_download_once(&state.pool, entry.id, body.download_once).await?;
    Ok(Json(regenerate(&state, &bundle).await?))
}

/// Move/rename a file or folder (descendant rows follow), sharing the hardened
/// [`repo::move_subtree`] path with `move`: DB-aware conflict (409), self-into-subtree
/// guard, atomic tx.
pub async fn rename_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedEntry { bundle, entry }: ResolvedEntry,
    Json(body): Json<RenameFile>,
) -> AppResult<Json<Bundle>> {
    let normalized = normalize_relative_path(body.new_relative_path.trim(), "newRelativePath")?;

    let files_root = files_path(storage_root(&state), &bundle.slug);
    apply_move(&state, &bundle, &files_root, &entry, &normalized).await?;

    Ok(Json(regenerate(&state, &bundle).await?))
}

/// Move a single entry into `targetDir` (`""` = build root); new path is
/// `join(targetDir, name)`.
pub async fn move_file(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedEntry { bundle, entry }: ResolvedEntry,
    Json(body): Json<MoveFile>,
) -> AppResult<Json<Bundle>> {
    let new_rel = join_target_dir(&body.target_dir, &entry.name)?;
    let files_root = files_path(storage_root(&state), &bundle.slug);
    apply_move(&state, &bundle, &files_root, &entry, &new_rel).await?;

    Ok(Json(regenerate(&state, &bundle).await?))
}

/// Move many entries into `targetDir` (`""` = build root) in ONE transaction
/// (all-or-nothing: a collision aborts the whole batch with a 409), regenerating the
/// manifest once at the end.
pub async fn move_files(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
    Json(body): Json<MoveFiles>,
) -> AppResult<Json<Bundle>> {
    if body.ids.is_empty() {
        return Err(AppError::BadRequest("ids must be a non-empty array".into()));
    }
    let files_root = files_path(storage_root(&state), &bundle.slug);

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

    Ok(Json(regenerate(&state, &bundle).await?))
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
    if path_exists(&new_physical).await && new_rel != entry.relative_path {
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
    ResolvedEntry { bundle, entry }: ResolvedEntry,
) -> AppResult<Json<Bundle>> {
    if entry.is_dir {
        return Err(AppError::BadRequest("cannot rehash a directory".into()));
    }

    let files_root = files_path(storage_root(&state), &bundle.slug);
    let path = repo::join_files(&files_root, &entry.relative_path);
    if !path_exists(&path).await {
        return Err(AppError::BadRequest(
            "physical file not found on disk".into(),
        ));
    }

    let path_for_hash = path.clone();
    let sha256 = tokio::task::spawn_blocking(move || hash_file(&path_for_hash))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash task: {e}")))??;
    repo::update_artifact_sha(&state.pool, entry.id, &sha256).await?;
    Ok(Json(regenerate(&state, &bundle).await?))
}

/// Delete many entries by id, all-or-nothing: every id is resolved up front, the rows go
/// in ONE transaction, and the bytes are unlinked only after commit. `deleted` counts the
/// rows actually removed.
///
/// An id that no longer exists is treated as already-deleted and skipped; an id belonging
/// to a DIFFERENT build is a clean 404 before anything is touched (as in [`move_files`]).
pub async fn bulk_delete(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
    Json(body): Json<BulkDelete>,
) -> AppResult<Json<serde_json::Value>> {
    if body.ids.is_empty() {
        return Err(AppError::BadRequest("ids must be a non-empty array".into()));
    }
    let files_root = files_path(storage_root(&state), &bundle.slug);

    // why: a vanished id is already-deleted, so skipping it keeps a stale selection from
    // costing the whole batch — two admins on the same Files tab would otherwise see one
    // stale entry block the deletion of every valid one. A foreign id is still a 404.
    let mut entries: Vec<BundleArtifact> = Vec::with_capacity(body.ids.len());
    for id in &body.ids {
        match repo::find_artifact(&state.pool, *id).await? {
            Some(entry) if entry.bundle_id == bundle.id => entries.push(entry),
            Some(_) => {
                return Err(AppError::NotFound(
                    "file entry not found in this build".into(),
                ))
            }
            None => continue,
        }
    }

    let mut tx = state.pool.begin().await?;
    for entry in &entries {
        delete_entry_rows(&mut tx, bundle.id, entry).await?;
    }
    tx.commit().await?;

    for entry in &entries {
        unlink_entry(&files_root, entry).await;
    }

    regenerate(&state, &bundle).await?;
    Ok(Json(serde_json::json!({ "deleted": entries.len() })))
}

/// Artifact rows whose file is gone (`missing`) and on-disk files no row tracks
/// (`orphaned`).
pub async fn validate(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
) -> AppResult<Json<ValidateResult>> {
    let artifacts = repo::list_artifacts(&state.pool, bundle.id).await?;
    let files_root = files_path(storage_root(&state), &bundle.slug);

    // why: `scan_directory` streams and SHA-256s every file (up to 10 GiB) and the
    // per-artifact probe is one `stat` each — parking a runtime worker for that stalls
    // unrelated traffic, so the whole filesystem half runs off the executor.
    let files: Vec<(Uuid, String, String)> = artifacts
        .into_iter()
        .filter(|a| !a.is_dir)
        .map(|a| (a.id, a.relative_path, a.name))
        .collect();

    let (missing, orphaned) = tokio::task::spawn_blocking(move || diff_tree(&files_root, files))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("validate task: {e}")))??;

    Ok(Json(ValidateResult { missing, orphaned }))
}

/// Blocking half of [`validate`]: which tracked files are gone from disk, and which
/// on-disk files no row tracks. An absent `files/` dir scans as empty, so a build with
/// no upload yet reports nothing rather than erroring.
#[allow(clippy::type_complexity)]
fn diff_tree(
    files_root: &std::path::Path,
    files: Vec<(Uuid, String, String)>,
) -> AppResult<(Vec<MissingEntry>, Vec<OrphanEntry>)> {
    let mut missing = Vec::new();
    let mut tracked = std::collections::HashSet::with_capacity(files.len());
    for (id, relative_path, name) in files {
        if !repo::join_files(files_root, &relative_path).exists() {
            missing.push(MissingEntry {
                id,
                relative_path: relative_path.clone(),
                name,
            });
        }
        tracked.insert(relative_path);
    }

    let orphaned = scan_directory(files_root)?
        .into_iter()
        .filter(|entry| !entry.is_dir && !tracked.contains(&entry.relative_path))
        .map(|entry| OrphanEntry {
            relative_path: entry.relative_path,
        })
        .collect();

    Ok((missing, orphaned))
}

/// Rebuild `artifacts.json` from the rows and flip status to ready (or failed).
pub async fn regenerate_manifest(
    State(state): State<AppState>,
    _admin: AdminUser,
    ResolvedBundle(bundle): ResolvedBundle,
) -> AppResult<Json<Bundle>> {
    repo::set_status(&state.pool, bundle.id, "processing", None).await?;
    match regenerate(&state, &bundle).await {
        Ok(refreshed) => Ok(Json(refreshed)),
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
