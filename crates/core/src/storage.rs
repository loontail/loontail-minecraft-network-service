//! Shared on-disk helpers for the static-asset stores (catalog media, textures):
//! a cache-busting revision token, a parent-dir-creating write, a
//! NotFound-tolerant unlink, and the capped multipart reader both upload
//! endpoints use. Bundle storage has its own slug-guarded layout.

use std::path::Path;

use axum::body::Bytes;
use axum::extract::Multipart;
use rand::Rng;

use crate::error::{AppError, AppResult};

/// A fresh 6-byte revision, hex-encoded. New on every upload so a new asset busts
/// any client/CDN cache keyed on the URL.
pub fn revision_hex() -> String {
    let mut bytes = [0u8; 6];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Write bytes, creating the parent dir on demand so a missing storage tree
/// self-heals.
pub async fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await
}

/// Best-effort unlink; a missing file is not an error, since a row's path may point
/// at an already-gone file after a crash or manual cleanup.
pub async fn unlink_quiet(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "failed to unlink stored file");
        }
    }
}

/// Drain a `multipart/form-data` body into the `file` bytes plus one optional text
/// field, enforcing `max_bytes` as the bytes are read. The `file` field is read
/// chunk-by-chunk and aborted the instant the accumulated size crosses the cap, so
/// an oversized upload is never fully buffered — callers should also raise axum's
/// default body limit to the same cap as a second line of defense. Unknown fields
/// are drained and ignored so the stream stays well-formed.
pub async fn read_capped_upload(
    mut multipart: Multipart,
    max_bytes: usize,
    text_field: &str,
) -> AppResult<(Option<Bytes>, Option<String>)> {
    let mut file: Option<Bytes> = None;
    let mut text: Option<String> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|err| AppError::BadRequest(format!("malformed multipart: {err}")))?
    {
        match field.name() {
            Some("file") => {
                let mut buf: Vec<u8> = Vec::with_capacity(max_bytes.min(64 * 1024));
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|err| AppError::BadRequest(format!("reading file: {err}")))?
                {
                    if buf.len() + chunk.len() > max_bytes {
                        return Err(AppError::BadRequest(format!(
                            "file is too large (max {max_bytes} bytes)"
                        )));
                    }
                    buf.extend_from_slice(&chunk);
                }
                file = Some(Bytes::from(buf));
            }
            Some(name) if name == text_field => {
                let value = field
                    .text()
                    .await
                    .map_err(|err| AppError::BadRequest(format!("reading {text_field}: {err}")))?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    text = Some(trimmed.to_string());
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    Ok((file, text))
}
