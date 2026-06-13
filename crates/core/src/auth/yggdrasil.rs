//! Yggdrasil token namespace: access+client token pairs stored as 64-hex
//! plaintext. The Mojang protocol requires the client to echo these verbatim, so
//! unlike network/admin sessions they cannot be hashed at rest.
//!
//! Lifecycle: issue (capped per user, evicting the oldest), validate, refresh
//! (revoke old + reissue preserving the client token), invalidate, and an hourly
//! cleanup of expired rows. Each pair carries a 15-day TTL (config-driven).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

use super::bearer_token_from_headers;
use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::state::AppState;

/// A freshly issued access/client token pair, returned plaintext to the client.
#[derive(Debug, Clone)]
pub struct YggdrasilTokens {
    pub access_token: String,
    pub client_token: String,
    pub expires_at: DateTime<Utc>,
}

fn random_64_hex() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Issue a new token pair for `user_id`. A caller-supplied `client_token` is
/// preserved (the Mojang refresh contract keeps a stable client token across an
/// access-token rotation); otherwise a fresh one is minted. Enforces the
/// per-user active cap from config by deleting the oldest rows beyond it.
pub async fn issue_yggdrasil_tokens(
    pool: &PgPool,
    user_id: Uuid,
    client_token: Option<String>,
    ttl: std::time::Duration,
    max_per_user: i64,
) -> AppResult<YggdrasilTokens> {
    let access_token = random_64_hex();
    let client_token = client_token.unwrap_or_else(random_64_hex);
    let expires_at =
        Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::days(15));

    sqlx::query(
        r#"
        INSERT INTO yggdrasil_tokens (user_id, access_token, client_token, expires_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(user_id)
    .bind(&access_token)
    .bind(&client_token)
    .bind(expires_at)
    .execute(pool)
    .await?;

    // Evict the oldest pairs beyond the active cap so a single user cannot grow
    // the table unbounded across many logins.
    if max_per_user > 0 {
        sqlx::query(
            r#"
            DELETE FROM yggdrasil_tokens
            WHERE user_id = $1
              AND id NOT IN (
                SELECT id FROM yggdrasil_tokens
                WHERE user_id = $1
                ORDER BY issued_at DESC
                LIMIT $2
              )
            "#,
        )
        .bind(user_id)
        .bind(max_per_user)
        .execute(pool)
        .await?;
    }

    Ok(YggdrasilTokens {
        access_token,
        client_token,
        expires_at,
    })
}

/// Resolve an access token to its owning user, requiring the pair to be
/// unexpired, the user eligible (not blocked, confirmed), and — when a
/// `client_token` is supplied — that it matches the stored one.
pub async fn validate_yggdrasil(
    pool: &PgPool,
    access_token: &str,
    client_token: Option<&str>,
) -> AppResult<User> {
    let row = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT user_id, client_token FROM yggdrasil_tokens
        WHERE access_token = $1 AND expires_at > now()
        "#,
    )
    .bind(access_token)
    .fetch_optional(pool)
    .await?;

    let (user_id, stored_client) = row.ok_or(AppError::Forbidden)?;
    if let Some(supplied) = client_token {
        if supplied != stored_client {
            return Err(AppError::Forbidden);
        }
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::Forbidden)?;

    if user.blocked || !user.confirmed {
        return Err(AppError::Forbidden);
    }
    Ok(user)
}

/// Refresh an access token: validate it (optionally against the supplied client
/// token), revoke the old pair, and issue a new one that preserves the existing
/// client token. Returns the new pair.
pub async fn refresh_yggdrasil(
    pool: &PgPool,
    access_token: &str,
    client_token: Option<&str>,
    ttl: std::time::Duration,
    max_per_user: i64,
) -> AppResult<(User, YggdrasilTokens)> {
    let row = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT user_id, client_token FROM yggdrasil_tokens
        WHERE access_token = $1 AND expires_at > now()
        "#,
    )
    .bind(access_token)
    .fetch_optional(pool)
    .await?;

    let (user_id, stored_client) = row.ok_or(AppError::Forbidden)?;
    if let Some(supplied) = client_token {
        if supplied != stored_client {
            return Err(AppError::Forbidden);
        }
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::Forbidden)?;
    if user.blocked || !user.confirmed {
        return Err(AppError::Forbidden);
    }

    sqlx::query("DELETE FROM yggdrasil_tokens WHERE access_token = $1")
        .bind(access_token)
        .execute(pool)
        .await?;

    let tokens =
        issue_yggdrasil_tokens(pool, user_id, Some(stored_client), ttl, max_per_user).await?;
    Ok((user, tokens))
}

/// Invalidate (delete) a token pair. With a `client_token` supplied the pair is
/// only removed when it matches; returns the number of rows removed.
pub async fn invalidate_yggdrasil(
    pool: &PgPool,
    access_token: &str,
    client_token: Option<&str>,
) -> AppResult<u64> {
    let affected = match client_token {
        Some(ct) => sqlx::query(
            "DELETE FROM yggdrasil_tokens WHERE access_token = $1 AND client_token = $2",
        )
        .bind(access_token)
        .bind(ct)
        .execute(pool)
        .await?
        .rows_affected(),
        None => sqlx::query("DELETE FROM yggdrasil_tokens WHERE access_token = $1")
            .bind(access_token)
            .execute(pool)
            .await?
            .rows_affected(),
    };
    Ok(affected)
}

/// Delete all expired token pairs. Runs hourly off the request path; returns the
/// number of rows removed.
pub async fn cleanup_expired_yggdrasil(pool: &PgPool) -> AppResult<u64> {
    let affected = sqlx::query("DELETE FROM yggdrasil_tokens WHERE expires_at <= now()")
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected)
}

/// Extractor for Yggdrasil-authenticated requests: a `Authorization: Bearer
/// <access_token>` whose pair is unexpired and whose user is eligible.
pub struct YggdrasilUser {
    pub user: User,
    pub access_token: String,
}

impl YggdrasilUser {
    pub fn id(&self) -> Uuid {
        self.user.id
    }
}

impl FromRequestParts<AppState> for YggdrasilUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let access_token = bearer_token_from_headers(&parts.headers).ok_or(AppError::Forbidden)?;
        let user = validate_yggdrasil(&state.pool, &access_token, None).await?;
        Ok(YggdrasilUser { user, access_token })
    }
}
