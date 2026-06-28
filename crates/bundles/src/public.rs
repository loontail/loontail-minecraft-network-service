//! Public bundle-registry read endpoints: the manifest (served verbatim from
//! `artifacts.json`) and static serving of a build's files.

use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use tower::util::ServiceExt;
use tower_http::services::ServeFile;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::repo;
use crate::storage::{files_path, manifest_path, normalize_relative_path, validate_slug};

/// Return `artifacts.json` byte-for-byte (the launcher hashes the raw bytes) with
/// `Cache-Control: no-cache`.
pub async fn get_manifest(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(slug): Path<String>,
) -> AppResult<Response> {
    // why: reject a traversal slug and confirm the build exists *before* any FS
    // join — axum decodes `Path` after routing, so `slug` may be `../../x`.
    validate_slug(&slug)?;
    repo::require_by_slug(&state.pool, &slug).await?;

    let path = manifest_path(&state.config.bundles.storage_root, &slug);
    let bytes: Vec<u8> = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::NotFound(
                "manifest not found — build may not be ready yet".into(),
            ));
        }
        Err(e) => return Err(AppError::Internal(anyhow::anyhow!("read manifest: {e}"))),
    };

    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    Ok((headers, bytes).into_response())
}

/// Stream a single file from the build's `files/` directory with a guarded relative
/// path. Byte streaming and range handling are delegated to `ServeFile`.
pub async fn serve_file(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((slug, rel)): Path<(String, String)>,
    request: Request,
) -> AppResult<Response> {
    // why: reject a traversal slug and confirm the build exists *before* any FS
    // join — axum decodes `Path` after routing, so `slug` may be `../../x`.
    validate_slug(&slug)?;
    repo::require_by_slug(&state.pool, &slug).await?;

    let normalized = normalize_relative_path(&rel, "path")?;
    let root = files_path(&state.config.bundles.storage_root, &slug);
    let target = repo::join_files(&root, &normalized);

    // why: canonicalize both paths and re-verify containment to defeat symlinks and
    // any residual escape before handing the path to ServeFile.
    let canonical_root = tokio::fs::canonicalize(&root)
        .await
        .map_err(|_| AppError::NotFound("file not found".into()))?;
    let canonical_target = match tokio::fs::canonicalize(&target).await {
        Ok(p) => p,
        Err(_) => return Err(AppError::NotFound("file not found".into())),
    };
    if !canonical_target.starts_with(&canonical_root) {
        return Err(AppError::NotFound("file not found".into()));
    }

    let mut response = ServeFile::new(&canonical_target)
        .oneshot(request)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serve file: {e}")))?
        .into_response();

    // why (SEC-8): force the Content-Type rather than let ServeFile guess from the
    // extension — an uploaded `.html`/`.svg` served as text/html could render
    // same-origin. Unknown types fall back to octet-stream (a download, never rendered).
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_for(&normalized)),
    );
    Ok(response)
}

/// Map a build file's extension to a fixed Content-Type; unknown types fall back to
/// `application/octet-stream` so they download rather than render in-origin.
fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => "application/json",
        "jar" => "application/java-archive",
        _ => "application/octet-stream",
    }
}
