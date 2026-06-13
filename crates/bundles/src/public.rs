//! Public, unauthenticated bundle-registry endpoints: the manifest (served
//! verbatim from `artifacts.json` with `Cache-Control: no-cache`) and static
//! serving of the build's files.

use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use tower::util::ServiceExt;
use tower_http::services::ServeFile;

use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::repo;
use crate::storage::{files_path, manifest_path, normalize_relative_path};

/// `GET /builds/{slug}/manifest` — return `artifacts.json` byte-for-byte (the
/// launcher hashes the raw bytes) with `Cache-Control: no-cache`.
pub async fn get_manifest(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<Response> {
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

/// `GET /builds/{slug}/files/{*path}` — stream a single file from the build's
/// `files/` directory, with a guarded relative path. Delegates byte streaming and
/// range handling to `tower-http`'s `ServeFile`.
pub async fn serve_file(
    State(state): State<AppState>,
    Path((slug, rel)): Path<(String, String)>,
    request: Request,
) -> AppResult<Response> {
    let normalized = normalize_relative_path(&rel, "path")?;
    let root = files_path(&state.config.bundles.storage_root, &slug);
    let target = repo::join_files(&root, &normalized);

    // why: re-verify the resolved path is inside the files root even though
    // normalize_relative_path already rejected `..`/absolute — defense in depth.
    if !target.starts_with(&root) {
        return Err(AppError::NotFound("file not found".into()));
    }
    if !tokio::fs::try_exists(&target).await.unwrap_or(false) {
        return Err(AppError::NotFound("file not found".into()));
    }

    let mut response = ServeFile::new(&target)
        .oneshot(request)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serve file: {e}")))?
        .into_response();

    // ServeFile guesses by extension; force a known type for the launcher's files.
    if let Some(ct) = guess_content_type(&normalized) {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    }
    Ok(response)
}

/// Override `ServeFile`'s guess for the file types the launcher serves; `None`
/// keeps whatever `ServeFile` inferred.
fn guess_content_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => Some("application/json"),
        "jar" => Some("application/java-archive"),
        _ => None,
    }
}
