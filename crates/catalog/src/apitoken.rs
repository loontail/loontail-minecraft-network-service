//! Launcher API-token verification. The launcher sends `Authorization: Bearer
//! <API_TOKEN>` for catalog reads; tokens live in `api_tokens` stored as a
//! SHA-256 hash (the `Strapi`-replacement table from migration 0008). Public
//! catalog reads accept a valid Bearer token OR no token at all (public read),
//! per the contract — an INVALID token is the only thing that is rejected.

use axum::http::HeaderMap;
use loontail_core::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

fn bearer(headers: &HeaderMap) -> Option<String> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
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

/// SHA-256 of an API token, hex-encoded — matches how `api_tokens.token_hash` is
/// stored.
pub fn hash_api_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Verify (and touch) an API token, returning true when it exists. The token's
/// `last_used_at` is bumped on a hit.
pub async fn verify_api_token(pool: &PgPool, token: &str) -> AppResult<bool> {
    let token_hash = hash_api_token(token);
    let updated = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(updated > 0)
}

/// Gate a public catalog read: no Bearer header ⇒ allowed (public read); a
/// Bearer header ⇒ must match a known API token, else `Unauthorized`.
pub async fn authorize_public_read(pool: &PgPool, headers: &HeaderMap) -> AppResult<()> {
    match bearer(headers) {
        None => Ok(()),
        Some(token) => {
            if verify_api_token(pool, &token).await? {
                Ok(())
            } else {
                Err(AppError::Unauthorized)
            }
        }
    }
}
