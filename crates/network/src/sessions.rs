use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use loontail_core::auth::{generate_token, hash_token};
use loontail_core::config::Config;
use loontail_core::error::AppResult;

/// A freshly issued network session: the raw token (shown once) and its expiry.
pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Create a new network session for a user and persist only its hash.
pub async fn issue_session(
    pool: &PgPool,
    config: &Config,
    user_id: Uuid,
) -> AppResult<IssuedSession> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let ttl = Duration::from_std(config.session_ttl).unwrap_or_else(|_| Duration::seconds(86_400));
    let expires_at = Utc::now() + ttl;

    sqlx::query(
        r#"
        INSERT INTO network_sessions (user_id, token_hash, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(user_id)
    .bind(&token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(IssuedSession { token, expires_at })
}
