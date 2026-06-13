//! API token management: list, create (raw token returned exactly once; only its
//! SHA-256 hash is stored), and delete. These tokens authenticate launcher
//! catalog/manifest reads, replacing the former Strapi API tokens.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use loontail_core::auth::{generate_token, hash_token, verify_csrf, AdminUser};
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::dto::{Ack, ApiTokenDto, CreateApiTokenRequest, CreatedApiTokenDto};

#[derive(sqlx::FromRow)]
struct ApiTokenRow {
    id: Uuid,
    name: String,
    scopes: Vec<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

impl From<ApiTokenRow> for ApiTokenDto {
    fn from(row: ApiTokenRow) -> Self {
        ApiTokenDto {
            id: row.id,
            name: row.name,
            scopes: row.scopes,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

/// `GET /admin/api-tokens` — list tokens (metadata only; raw values are never
/// recoverable).
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<ApiTokenDto>>> {
    let rows = sqlx::query_as::<_, ApiTokenRow>(
        "SELECT id, name, scopes, created_at, last_used_at FROM api_tokens ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(ApiTokenDto::from).collect()))
}

/// `POST /admin/api-tokens` — generate a random token, store its hash, and return
/// the raw value once.
pub async fn create(
    _admin: AdminUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiTokenRequest>,
) -> AppResult<Json<CreatedApiTokenDto>> {
    verify_csrf(&headers)?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }

    let raw = generate_token();
    let token_hash = hash_token(&raw);

    let row = sqlx::query_as::<_, ApiTokenRow>(
        r#"
        INSERT INTO api_tokens (name, token_hash, scopes)
        VALUES ($1, $2, $3)
        RETURNING id, name, scopes, created_at, last_used_at
        "#,
    )
    .bind(name)
    .bind(&token_hash)
    .bind(&body.scopes)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(CreatedApiTokenDto {
        id: row.id,
        name: row.name,
        scopes: row.scopes,
        created_at: row.created_at,
        token: raw,
    }))
}

/// `DELETE /admin/api-tokens/{id}`.
pub async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Ack>> {
    verify_csrf(&headers)?;
    let affected = sqlx::query("DELETE FROM api_tokens WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("api token not found".into()));
    }
    Ok(Json(Ack::ok()))
}

/// Resolve a raw API token to its row id, if it is valid. Provided for other
/// crates (catalog/bundles) to authenticate launcher reads. Updates
/// `last_used_at` so the admin can see token activity.
pub async fn verify_api_token(state: &AppState, raw: &str) -> AppResult<Uuid> {
    let token_hash = hash_token(raw);
    let id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1 RETURNING id",
    )
    .bind(token_hash)
    .fetch_optional(&state.pool)
    .await?;
    id.ok_or(AppError::Unauthorized)
}
