//! Admin session namespace: opaque 256-bit tokens stored as a SHA-256 hash. The
//! raw token rides in an httpOnly cookie (name from config) or, for tooling, an
//! `Authorization: Bearer` header. The `AdminUser` extractor additionally
//! requires `users.is_admin = true`.

use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{bearer_token_from_headers, generate_token, hash_token};
use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::state::AppState;

/// A freshly issued admin session: the raw token (returned once, set as a
/// cookie) and its expiry.
#[derive(Debug, Clone)]
pub struct AdminSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Issue an admin session for `user_id`, storing only the SHA-256 hash. Returns
/// the raw token to set in the session cookie.
pub async fn issue_admin_session(
    pool: &PgPool,
    user_id: Uuid,
    ttl: std::time::Duration,
) -> AppResult<AdminSession> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let expires_at =
        Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::days(7));

    sqlx::query(
        r#"
        INSERT INTO admin_sessions (user_id, token_hash, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(user_id)
    .bind(&token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(AdminSession { token, expires_at })
}

/// Resolve a raw admin token to its owning admin user, requiring the session to
/// be unexpired and unrevoked and the user to have `is_admin = true`.
pub async fn validate_admin_session(pool: &PgPool, token: &str) -> AppResult<User> {
    let token_hash = hash_token(token);
    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT u.*
        FROM admin_sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1
          AND s.revoked_at IS NULL
          AND s.expires_at > now()
          AND u.is_admin = true
          AND u.blocked = false
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    user.ok_or(AppError::Unauthorized)
}

/// Revoke a single admin session by its raw token. Returns the number of rows
/// affected (0 if already revoked or unknown).
pub async fn revoke_admin_session(pool: &PgPool, token: &str) -> AppResult<u64> {
    let token_hash = hash_token(token);
    let affected = sqlx::query(
        "UPDATE admin_sessions SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

/// Revoke every active admin session for a user (e.g. on password reset or
/// privilege change). Returns the number of sessions revoked.
pub async fn revoke_all_admin_sessions_for_user(pool: &PgPool, user_id: Uuid) -> AppResult<u64> {
    let affected = sqlx::query(
        "UPDATE admin_sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

fn cookie_value<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    let header = parts.headers.get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

/// Extractor for admin-authenticated requests: an admin session cookie (name from
/// config) or, as a fallback for tooling, an `Authorization: Bearer` token,
/// resolving to a user with `is_admin = true`.
pub struct AdminUser {
    pub user: User,
}

impl AdminUser {
    pub fn id(&self) -> Uuid {
        self.user.id
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = cookie_value(parts, &state.config.admin.cookie_name)
            .map(str::to_string)
            .or_else(|| bearer_token_from_headers(&parts.headers))
            .ok_or(AppError::Unauthorized)?;
        let user = validate_admin_session(&state.pool, &token).await?;
        Ok(AdminUser { user })
    }
}
