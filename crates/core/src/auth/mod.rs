use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::state::AppState;

pub mod admin;
pub mod csrf;
pub mod yggdrasil;

pub use admin::{
    cleanup_expired_admin_sessions, issue_admin_session, revoke_admin_session,
    revoke_all_admin_sessions_for_user, validate_admin_session, AdminSession, AdminUser,
};
pub use csrf::{generate_csrf_token, verify_csrf, CSRF_COOKIE_NAME, CSRF_HEADER_NAME};
pub use yggdrasil::{
    cleanup_expired_yggdrasil, invalidate_yggdrasil, issue_yggdrasil_tokens, refresh_yggdrasil,
    validate_yggdrasil, YggdrasilTokens, YggdrasilUser,
};

/// Generate a fresh opaque session/ticket token (256 bits, hex-encoded).
/// The raw value is returned to the client exactly once; only its hash is
/// ever persisted.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 of a token, hex-encoded. Tokens carry 256 bits of entropy, so a
/// plain cryptographic hash (no salt/KDF) is appropriate here.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Revoke every active network session for a user (e.g. when an admin disables
/// or resets the account). Returns the number of sessions revoked.
pub async fn revoke_all_network_sessions_for_user(pool: &PgPool, user_id: Uuid) -> AppResult<u64> {
    let affected = sqlx::query(
        "UPDATE network_sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

/// Resolve a network session token to its owning user, enforcing that the
/// session is unexpired and not revoked. Used by both the HTTP extractor and
/// the WebSocket endpoints, which authenticate the upgrade request from the same
/// `Authorization: Bearer` header (with a `?token=` query fallback).
pub async fn user_from_token(pool: &PgPool, token: &str) -> AppResult<User> {
    let token_hash = hash_token(token);
    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT u.*
        FROM network_sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1
          AND s.revoked_at IS NULL
          AND s.expires_at > now()
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    user.ok_or(AppError::Unauthorized)
}

/// Extract a Bearer token from request headers. Shared by the HTTP `AuthUser`
/// extractor and the WebSocket handlers, so a WS upgrade authenticates exactly
/// like a REST call — keeping the token out of the URL (and the access log).
pub fn bearer_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    bearer_token_from_headers(&parts.headers)
}

/// Extractor that authenticates a request from its `Authorization: Bearer`
/// header. Handlers take `auth: AuthUser` to require a valid session.
pub struct AuthUser {
    pub user: User,
}

impl AuthUser {
    pub fn id(&self) -> uuid::Uuid {
        self.user.id
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or(AppError::Unauthorized)?;
        let user = user_from_token(&state.pool, &token).await?;
        Ok(AuthUser { user })
    }
}
