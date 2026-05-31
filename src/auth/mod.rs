use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::state::AppState;

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

/// Resolve a network session token to its owning user, enforcing that the
/// session is unexpired and not revoked. Used by both the HTTP extractor and
/// the WebSocket endpoints (which carry the token as a query parameter).
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

fn bearer_token(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = header.strip_prefix("Bearer ").or_else(|| header.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
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
