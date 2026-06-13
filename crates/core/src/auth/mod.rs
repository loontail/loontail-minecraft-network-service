//! Unified authentication kernel.
//!
//! There is ONE API authorization token: an opaque 256-bit value, persisted only
//! as its SHA-256 hash in the `sessions` table. The launcher and the in-game
//! agent send it as `Authorization: Bearer`; the admin browser SPA carries the
//! same value in an httpOnly cookie. `is_admin` (resolved from the session's
//! user) is the only role. A separate Yggdrasil token namespace (see
//! [`yggdrasil`]) survives only for the Minecraft game handshake.
//!
//! Two extractors gate handlers: [`AuthUser`] (any live, non-blocked session) and
//! [`AdminUser`] (the same, plus `is_admin`). When a request authenticates via the
//! cookie AND mutates (non-GET/HEAD/OPTIONS), the extractor enforces the CSRF
//! double-submit centrally — so every cookie-authorized write is covered, not just
//! the handlers that remember to call [`verify_csrf`]. Bearer requests are immune
//! to CSRF and skip the check.

use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use chrono::{DateTime, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::state::AppState;

pub mod csrf;
pub mod yggdrasil;

pub use csrf::{generate_csrf_token, verify_csrf, CSRF_COOKIE_NAME, CSRF_HEADER_NAME};
pub use yggdrasil::{
    cleanup_expired_yggdrasil, invalidate_all_yggdrasil_for_user, invalidate_yggdrasil,
    issue_yggdrasil_tokens, refresh_yggdrasil, validate_yggdrasil, YggdrasilTokens, YggdrasilUser,
};

/// Generate a fresh opaque session/ticket token (256 bits, hex-encoded). The raw
/// value is returned to the client exactly once; only its hash is ever persisted.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 of a token, hex-encoded. Tokens carry 256 bits of entropy, so a plain
/// cryptographic hash (no salt/KDF) is appropriate here.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// A freshly issued session: the raw token (returned once — set as a Bearer value
/// or session cookie) and its expiry.
#[derive(Debug, Clone)]
pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Issue a session for `user_id`, storing only the SHA-256 hash. The raw token is
/// the universal API bearer (launcher/agent) and the admin cookie value.
pub async fn issue_session(
    pool: &PgPool,
    user_id: Uuid,
    ttl: Duration,
) -> AppResult<IssuedSession> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let expires_at =
        Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::days(7));

    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&token_hash)
        .bind(expires_at)
        .execute(pool)
        .await?;

    Ok(IssuedSession { token, expires_at })
}

/// Resolve a session token to its owning user, requiring the session to be
/// unexpired and unrevoked AND the user to be unblocked. The `blocked` predicate
/// means disabling an account immediately invalidates every one of its live
/// sessions (no waiting for TTL).
pub async fn user_from_token(pool: &PgPool, token: &str) -> AppResult<User> {
    let token_hash = hash_token(token);
    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT u.*
        FROM sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1
          AND s.revoked_at IS NULL
          AND s.expires_at > now()
          AND u.blocked = false
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    user.ok_or(AppError::Unauthorized)
}

/// Revoke a single session by its raw token. Returns the number of rows affected
/// (0 if already revoked or unknown).
pub async fn revoke_session(pool: &PgPool, token: &str) -> AppResult<u64> {
    let token_hash = hash_token(token);
    let affected = sqlx::query(
        "UPDATE sessions SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

/// Revoke every live session for a user (e.g. on password reset, block, or an
/// `is_admin` change). Returns the number of sessions revoked.
pub async fn revoke_all_sessions_for_user(pool: &PgPool, user_id: Uuid) -> AppResult<u64> {
    let affected = sqlx::query(
        "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

/// Delete sessions that have expired or been revoked. Runs hourly off the request
/// path so the table does not grow unbounded; returns the number of rows removed.
pub async fn cleanup_expired_sessions(pool: &PgPool) -> AppResult<u64> {
    let affected =
        sqlx::query("DELETE FROM sessions WHERE expires_at <= now() OR revoked_at IS NOT NULL")
            .execute(pool)
            .await?
            .rows_affected();
    Ok(affected)
}

/// Extract a Bearer token from request headers. Shared by the extractors and the
/// WebSocket handlers, so a WS upgrade authenticates exactly like a REST call.
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

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

/// Where a session token was presented. Cookie-sourced writes require CSRF; Bearer
/// requests (launcher/agent/tooling) do not.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenSource {
    Bearer,
    Cookie,
}

/// Resolve the raw session token from the `Authorization: Bearer` header (preferred
/// — launcher/agent/tooling) or the admin session cookie (browser SPA).
fn session_token(parts: &Parts, cookie_name: &str) -> Option<(String, TokenSource)> {
    if let Some(token) = bearer_token_from_headers(&parts.headers) {
        return Some((token, TokenSource::Bearer));
    }
    cookie_value(&parts.headers, cookie_name).map(|t| (t.to_string(), TokenSource::Cookie))
}

fn is_mutating(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Extractor that authenticates any request bearing a live, non-blocked session —
/// via `Authorization: Bearer` or the admin session cookie. Cookie-authenticated
/// mutations must additionally pass the CSRF double-submit check.
pub struct AuthUser {
    pub user: User,
}

impl AuthUser {
    pub fn id(&self) -> Uuid {
        self.user.id
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let (token, source) =
            session_token(parts, &state.config.admin.cookie_name).ok_or(AppError::Unauthorized)?;
        // why: a cross-site form post can ride the session cookie but cannot read
        // it to forge the double-submit header, so CSRF is required exactly for
        // cookie-authenticated state changes; Bearer callers are immune.
        if source == TokenSource::Cookie && is_mutating(&parts.method) {
            verify_csrf(&parts.headers)?;
        }
        let user = user_from_token(&state.pool, &token).await?;
        Ok(AuthUser { user })
    }
}

/// Extractor for admin-only handlers: a valid [`AuthUser`] whose account carries
/// `is_admin`. Shares the session table and CSRF enforcement with `AuthUser`.
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
        let AuthUser { user } = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(AppError::Forbidden);
        }
        Ok(AdminUser { user })
    }
}
