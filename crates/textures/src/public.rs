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
use loontail_core::AuthUser;
use loontail_yggdrasil_protocol::payload::{
    LookupCape, LookupSkin, SkinVariant, TexturesLookupResponse,
};
use loontail_yggdrasil_protocol::png::{validate_cape, validate_skin};
use loontail_yggdrasil_protocol::uuid::undash_uuid;

use crate::storage::{self, TextureKind};
use crate::{absolutize_url, relative_texture_url, MAX_UPLOAD_BYTES};

/// `GET /textures/{uuid}` — the texture lookup. Resolves the user by the requested
/// `profile_uuid`, then reads its `user_textures` rows by `users.id`, so a row's
/// denormalized `profile_uuid` going stale after identity reconciliation can never
/// orphan the texture; the served URL is re-derived from the requested
/// `profile_uuid` rather than the (possibly stale) stored `file_url`. An unknown
/// profile resolves to `{skin:null,cape:null}` rather than a 404.
pub async fn lookup(
    State(state): State<AppState>,
    Path(uuid): Path<String>,
) -> AppResult<Json<TexturesLookupResponse>> {
    let profile_uuid =
        undash_uuid(&uuid).map_err(|_| AppError::BadRequest("invalid profile uuid".into()))?;

    let Some(user_id) = resolve_user_id(&state, &profile_uuid).await? else {
        return Ok(Json(TexturesLookupResponse {
            skin: None,
            cape: None,
        }));
    };

    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT kind, variant FROM user_textures WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;

    let mut response = TexturesLookupResponse {
        skin: None,
        cape: None,
    };
    for (kind, variant) in rows {
        let url = absolutize_url(&state.config, &relative_texture_url(&profile_uuid, &kind));
        match TextureKind::parse(&kind) {
            Some(TextureKind::Skin) => {
                response.skin = Some(LookupSkin {
                    url,
                    variant: parse_variant(variant.as_deref().unwrap_or_default()),
                })
            }
            Some(TextureKind::Cape) => response.cape = Some(LookupCape { url }),
            None => {}
        }
    }

    Ok(Json(response))
}

/// Resolve the authoritative `users.id` for a `profile_uuid`. Textures are joined
/// off this id so a stale denormalized `user_textures.profile_uuid` cannot hide a
/// live texture.
async fn resolve_user_id(state: &AppState, profile_uuid: &str) -> AppResult<Option<uuid::Uuid>> {
    Ok(
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE profile_uuid = $1")
            .bind(profile_uuid)
            .fetch_optional(&state.pool)
            .await?,
    )
}

/// `GET /textures/{uuid}/{skin|cape}` — the raw PNG bytes for the profile. Resolves
/// the user by `profile_uuid`, reads the row's on-disk `file_path` by `users.id`,
/// and streams it back as `image/png`. 404 when the profile has no texture of that
/// kind (or the file went missing).
pub async fn read_png(
    State(state): State<AppState>,
    Path((uuid, kind)): Path<(String, String)>,
) -> AppResult<Response> {
    let kind =
        TextureKind::parse(&kind).ok_or_else(|| AppError::NotFound("unknown texture".into()))?;
    let profile_uuid =
        undash_uuid(&uuid).map_err(|_| AppError::BadRequest("invalid profile uuid".into()))?;

    let user_id = resolve_user_id(&state, &profile_uuid)
        .await?
        .ok_or_else(|| AppError::NotFound("texture not found".into()))?;

    let file_path = sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM user_textures WHERE user_id = $1 AND kind = $2",
    )
    .bind(user_id)
    .bind(kind.as_str())
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("texture not found".into()))?;

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
    user: AuthUser,
    multipart: Multipart,
) -> AppResult<StatusCode> {
    let kind =
        TextureKind::parse(&kind).ok_or_else(|| AppError::NotFound("unknown texture".into()))?;

    let UploadFields { file, variant } = read_upload(multipart).await?;
    let file = file.ok_or_else(|| AppError::BadRequest("missing file field".into()))?;

    match kind {
        TextureKind::Skin => validate_skin(&file),
        TextureKind::Cape => validate_cape(&file),
    }
    .map_err(|err| AppError::BadRequest(format!("invalid {} png: {err}", kind.as_str())))?;

    // The `user_textures_variant_kind` CHECK requires a variant for skins and none
    // for capes.
    let variant = match kind {
        TextureKind::Skin => Some(variant_str(
            variant
                .map(|v| parse_variant(&v))
                .unwrap_or(SkinVariant::Classic),
        )),
        TextureKind::Cape => None,
    };

    let profile_uuid = user
        .user
        .profile_uuid
        .clone()
        .ok_or_else(|| AppError::Forbidden)?;
    let username = user.user.username.clone();

    let revision = storage::revision_hex();
    let disk_path = storage::disk_path(
        &state.config.textures.storage_root,
        kind,
        &profile_uuid,
        &revision,
    );
    let file_path = disk_path.to_string_lossy().to_string();
    let file_url = relative_texture_url(&profile_uuid, kind.as_str());
    let file_size = file.len() as i32;

    storage::write_file(&disk_path, &file)
        .await
        .map_err(|err| AppError::Internal(anyhow::anyhow!("write texture: {err}")))?;

    // Capture the previous on-disk path before the upsert overwrites it so the old
    // revision can be unlinked after the new file is durably written.
    let old_path = sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM user_textures WHERE user_id = $1 AND kind = $2",
    )
    .bind(user.user.id)
    .bind(kind.as_str())
    .fetch_optional(&state.pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO user_textures
            (user_id, kind, profile_uuid, username, file_path, file_url, file_size, variant, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
        ON CONFLICT (user_id, kind) DO UPDATE SET
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
    .bind(kind.as_str())
    .bind(&profile_uuid)
    .bind(&username)
    .bind(&file_path)
    .bind(&file_url)
    .bind(file_size)
    .bind(variant)
    .execute(&state.pool)
    .await?;

    if let Some(old) = old_path {
        if old != file_path {
            storage::unlink_quiet(std::path::Path::new(&old)).await;
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
    user: AuthUser,
) -> AppResult<StatusCode> {
    let kind =
        TextureKind::parse(&kind).ok_or_else(|| AppError::NotFound("unknown texture".into()))?;

    if let Some(path) = storage::delete_by_user(&state.pool, kind, user.user.id).await? {
        storage::unlink_quiet(std::path::Path::new(&path)).await;
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

/// The DB text form of a variant (matches the `user_textures.variant` CHECK constraint).
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
/// enforcing the per-upload size cap as bytes are read. The `file` field is read
/// chunk-by-chunk and aborted the instant the accumulated size crosses
/// [`MAX_UPLOAD_BYTES`], so an oversized upload is never fully buffered. The route
/// also raises axum's default body limit to the same cap as a second line of
/// defense. Unknown fields are ignored.
async fn read_upload(multipart: Multipart) -> AppResult<UploadFields> {
    let (file, variant) =
        loontail_core::storage::read_capped_upload(multipart, MAX_UPLOAD_BYTES, "variant").await?;
    Ok(UploadFields { file, variant })
}
