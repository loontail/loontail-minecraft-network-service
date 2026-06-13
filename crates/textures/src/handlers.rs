//! HTTP handlers for the textures domain.
//!
//! The top-level path uses one dynamic `{segment}` (see [`crate::routes`]): GET
//! reads it as a profile UUID for a lookup, PUT/DELETE read it as a kind
//! (`skin`/`cape`) for an authenticated write. The PNG bytes live at
//! `{segment}/{kind}`.

use axum::body::Bytes;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;
use loontail_core::YggdrasilUser;
use loontail_yggdrasil_protocol::payload::{
    LookupCape, LookupSkin, SkinVariant, TexturesLookupResponse,
};
use loontail_yggdrasil_protocol::png::{validate_cape, validate_skin};
use loontail_yggdrasil_protocol::uuid::undash_uuid;

use crate::store::{self, Kind};
use crate::{absolutize_url, relative_texture_url, MAX_UPLOAD_BYTES};

/// `GET /textures/{uuid}` — the texture lookup. Reads the `skins`/`capes` rows by
/// `profile_uuid` (undashed) and returns `{skin?:{url,variant},cape?:{url}}` with
/// URLs absolutized. Absent entries are `null` (omitted) rather than a 404, so a
/// profile with no textures still resolves with an empty body `{}`.
pub async fn lookup(
    State(state): State<AppState>,
    Path(uuid): Path<String>,
) -> AppResult<Json<TexturesLookupResponse>> {
    let profile_uuid =
        undash_uuid(&uuid).map_err(|_| AppError::BadRequest("invalid profile uuid".into()))?;

    let skin = sqlx::query_as::<_, (String, String)>(
        "SELECT file_url, variant FROM skins WHERE profile_uuid = $1",
    )
    .bind(&profile_uuid)
    .fetch_optional(&state.pool)
    .await?
    .map(|(file_url, variant)| LookupSkin {
        url: absolutize_url(&state.config, &file_url),
        variant: parse_variant(&variant),
    });

    let cape = sqlx::query_as::<_, (String,)>("SELECT file_url FROM capes WHERE profile_uuid = $1")
        .bind(&profile_uuid)
        .fetch_optional(&state.pool)
        .await?
        .map(|(file_url,)| LookupCape {
            url: absolutize_url(&state.config, &file_url),
        });

    Ok(Json(TexturesLookupResponse { skin, cape }))
}

/// `GET /textures/{uuid}/{skin|cape}` — the raw PNG bytes for the profile. Reads
/// the row's on-disk `file_path` and streams it back as `image/png`. 404 when the
/// profile has no texture of that kind (or the file went missing).
pub async fn read_png(
    State(state): State<AppState>,
    Path((uuid, kind)): Path<(String, String)>,
) -> AppResult<Response> {
    let kind = Kind::parse(&kind).ok_or_else(|| AppError::NotFound("unknown texture".into()))?;
    let profile_uuid =
        undash_uuid(&uuid).map_err(|_| AppError::BadRequest("invalid profile uuid".into()))?;

    let table = match kind {
        Kind::Skin => "skins",
        Kind::Cape => "capes",
    };
    let row = sqlx::query_as::<_, (String,)>(&format!(
        "SELECT file_path FROM {table} WHERE profile_uuid = $1"
    ))
    .bind(&profile_uuid)
    .fetch_optional(&state.pool)
    .await?;

    let (file_path,) = row.ok_or_else(|| AppError::NotFound("texture not found".into()))?;

    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|_| AppError::NotFound("texture file missing".into()))?;

    Ok((StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}

/// `PUT /textures/{skin|cape}` — authenticated multipart upload. Validates the
/// `file` field as a Minecraft PNG of the right kind, writes a fresh revision to
/// disk (busting the cache), upserts the registry row, and unlinks the previous
/// file. `variant` (skins only) is client-provided and defaults to `CLASSIC`.
/// Returns `204 No Content`.
pub async fn upload(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    user: YggdrasilUser,
    multipart: Multipart,
) -> AppResult<StatusCode> {
    let kind = Kind::parse(&kind).ok_or_else(|| AppError::NotFound("unknown texture".into()))?;

    let UploadFields { file, variant } = read_upload(multipart).await?;
    let file = file.ok_or_else(|| AppError::BadRequest("missing file field".into()))?;

    match kind {
        Kind::Skin => validate_skin(&file),
        Kind::Cape => validate_cape(&file),
    }
    .map_err(|err| AppError::BadRequest(format!("invalid {} png: {err}", kind.slug())))?;

    let variant = match kind {
        Kind::Skin => variant
            .map(|v| parse_variant(&v))
            .unwrap_or(SkinVariant::Classic),
        Kind::Cape => SkinVariant::Classic,
    };

    let profile_uuid = user
        .user
        .profile_uuid
        .clone()
        .ok_or_else(|| AppError::Forbidden)?;
    let username = user.user.username.clone();

    let revision = store::revision_hex();
    let disk_path = store::disk_path(
        &state.config.textures.storage_root,
        kind,
        &profile_uuid,
        &revision,
    );
    let file_path = disk_path.to_string_lossy().to_string();
    let file_url = relative_texture_url(&profile_uuid, kind);
    let file_size = file.len() as i32;

    store::write_file(&disk_path, &file)
        .await
        .map_err(|err| AppError::Internal(anyhow::anyhow!("write texture: {err}")))?;

    let table = match kind {
        Kind::Skin => "skins",
        Kind::Cape => "capes",
    };
    // Capture the previous on-disk path before the upsert overwrites it so the old
    // revision can be unlinked after the new file is durably written.
    let old_path = sqlx::query_scalar::<_, String>(&format!(
        "SELECT file_path FROM {table} WHERE user_id = $1"
    ))
    .bind(user.user.id)
    .fetch_optional(&state.pool)
    .await?;

    match kind {
        Kind::Skin => {
            sqlx::query(
                r#"
                INSERT INTO skins
                    (user_id, profile_uuid, username, file_path, file_url, file_size, variant, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, now())
                ON CONFLICT (user_id) DO UPDATE SET
                    profile_uuid = EXCLUDED.profile_uuid,
                    username     = EXCLUDED.username,
                    file_path    = EXCLUDED.file_path,
                    file_url     = EXCLUDED.file_url,
                    file_size    = EXCLUDED.file_size,
                    variant      = EXCLUDED.variant,
                    updated_at   = now()
                "#,
            )
            .bind(user.user.id)
            .bind(&profile_uuid)
            .bind(&username)
            .bind(&file_path)
            .bind(&file_url)
            .bind(file_size)
            .bind(variant_str(variant))
            .execute(&state.pool)
            .await?;
        }
        Kind::Cape => {
            sqlx::query(
                r#"
                INSERT INTO capes
                    (user_id, profile_uuid, username, file_path, file_url, file_size, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, now())
                ON CONFLICT (user_id) DO UPDATE SET
                    profile_uuid = EXCLUDED.profile_uuid,
                    username     = EXCLUDED.username,
                    file_path    = EXCLUDED.file_path,
                    file_url     = EXCLUDED.file_url,
                    file_size    = EXCLUDED.file_size,
                    updated_at   = now()
                "#,
            )
            .bind(user.user.id)
            .bind(&profile_uuid)
            .bind(&username)
            .bind(&file_path)
            .bind(&file_url)
            .bind(file_size)
            .execute(&state.pool)
            .await?;
        }
    }

    if let Some(old) = old_path {
        if old != file_path {
            store::unlink_quiet(std::path::Path::new(&old)).await;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /textures/{skin|cape}` — authenticated. Removes the registry row for
/// the caller's profile and unlinks its file. Returns `204` whether or not a row
/// existed (idempotent delete).
pub async fn delete(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    user: YggdrasilUser,
) -> AppResult<StatusCode> {
    let kind = Kind::parse(&kind).ok_or_else(|| AppError::NotFound("unknown texture".into()))?;

    let table = match kind {
        Kind::Skin => "skins",
        Kind::Cape => "capes",
    };
    let old_path = sqlx::query_scalar::<_, String>(&format!(
        "DELETE FROM {table} WHERE user_id = $1 RETURNING file_path"
    ))
    .bind(user.user.id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some(path) = old_path {
        store::unlink_quiet(std::path::Path::new(&path)).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Map the stored variant text to the enum. Unknown values fall back to `CLASSIC`
/// (the DB CHECK constraint already restricts writes to `CLASSIC`/`SLIM`).
fn parse_variant(value: &str) -> SkinVariant {
    match value {
        "SLIM" | "slim" => SkinVariant::Slim,
        _ => SkinVariant::Classic,
    }
}

/// The DB text form of a variant (matches the `skins.variant` CHECK constraint).
fn variant_str(variant: SkinVariant) -> &'static str {
    match variant {
        SkinVariant::Slim => "SLIM",
        SkinVariant::Classic => "CLASSIC",
    }
}

struct UploadFields {
    file: Option<Bytes>,
    variant: Option<String>,
}

/// Drain the multipart body into the `file` bytes and an optional `variant` text,
/// enforcing the per-upload size cap as bytes are read. Unknown fields are
/// ignored; the cap guards against an oversized `file` even though the route also
/// raises axum's default body limit.
async fn read_upload(mut multipart: Multipart) -> AppResult<UploadFields> {
    let mut file: Option<Bytes> = None;
    let mut variant: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| AppError::BadRequest(format!("malformed multipart: {err}")))?
    {
        match field.name() {
            Some("file") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|err| AppError::BadRequest(format!("reading file: {err}")))?;
                if bytes.len() > MAX_UPLOAD_BYTES {
                    return Err(AppError::BadRequest(format!(
                        "file exceeds {MAX_UPLOAD_BYTES} bytes"
                    )));
                }
                file = Some(bytes);
            }
            Some("variant") => {
                let text = field
                    .text()
                    .await
                    .map_err(|err| AppError::BadRequest(format!("reading variant: {err}")))?;
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    variant = Some(trimmed.to_string());
                }
            }
            _ => {
                // Drain and ignore unknown fields so the stream stays well-formed.
                let _ = field.bytes().await;
            }
        }
    }

    Ok(UploadFields { file, variant })
}
