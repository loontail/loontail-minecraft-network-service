//! Launcher/agent account surface (`/api/auth/*`). Unlike the Mojang `/authserver`
//! endpoints (which speak the game's wire protocol), these issue the unified API
//! session token. A single login returns BOTH: the opaque session token the
//! launcher and agent send as `Authorization: Bearer` on every request, and a
//! Yggdrasil access/client pair the launcher feeds to authlib-injector for the
//! in-game online-mode handshake. The session token is hashed at rest; the
//! Yggdrasil pair is plaintext-at-rest only because the Mojang protocol echoes it.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loontail_core::auth::{
    issue_session, issue_yggdrasil_tokens, revoke_session, session_token_from_headers, AuthUser,
};
use loontail_core::error::AppResult;
use loontail_core::identity::{authenticate_password, register_user};
use loontail_core::models::User;
use loontail_core::AppState;

/// The launcher/agent account router. The server crate nests it at `/api/auth`.
pub fn account_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/register", post(register))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    /// Username or email.
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterRequest {
    username: String,
    email: String,
    password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDto {
    token: String,
    expires_at: DateTime<Utc>,
}

/// The Yggdrasil pair for the in-game handshake (authlib-injector consumes it).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MinecraftTokens {
    access_token: String,
    client_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountDto {
    id: Uuid,
    username: String,
    email: Option<String>,
    /// The undashed profile UUID (the Minecraft identity for this account).
    uuid: Option<String>,
    is_admin: bool,
}

impl From<&User> for AccountDto {
    fn from(user: &User) -> Self {
        AccountDto {
            id: user.id,
            username: user.username.clone(),
            email: user.email.clone(),
            uuid: user.profile_uuid.clone(),
            is_admin: user.is_admin,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    session: SessionDto,
    minecraft: MinecraftTokens,
    profile: AccountDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Ack {
    ok: bool,
}

/// Issue the API session token plus a fresh Yggdrasil pair for `user`.
async fn auth_response(state: &AppState, user: User) -> AppResult<Json<AuthResponse>> {
    let session = issue_session(&state.pool, user.id, state.config.session_ttl).await?;
    let ygg = issue_yggdrasil_tokens(
        &state.pool,
        user.id,
        None,
        state.config.yggdrasil.token_ttl,
        state.config.yggdrasil.max_tokens_per_user,
    )
    .await?;
    Ok(Json(AuthResponse {
        session: SessionDto {
            token: session.token,
            expires_at: session.expires_at,
        },
        minecraft: MinecraftTokens {
            access_token: ygg.access_token,
            client_token: ygg.client_token,
        },
        profile: AccountDto::from(&user),
    }))
}

/// `POST /api/auth/login` — verify credentials, issue a session + game tokens.
async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    let user = authenticate_password(&state.pool, &body.username, &body.password).await?;
    auth_response(&state, user).await
}

/// `POST /api/auth/register` — create a credentialed Yggdrasil account and log in.
async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    let user = register_user(&state.pool, &body.username, &body.email, &body.password).await?;
    loontail_core::analytics::spawn_event(&state, "new_user", Some(user.id), None);
    auth_response(&state, user).await
}

/// `POST /api/auth/refresh` — rotate the session (revoke the presenting token,
/// issue a fresh session + game tokens). Avoids the launcher re-prompting for a
/// password when the 7-day session nears expiry.
async fn refresh(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> AppResult<Json<AuthResponse>> {
    if let Some(token) = session_token_from_headers(&headers, &state.config.admin.cookie_name) {
        revoke_session(&state.pool, &token).await?;
    }
    auth_response(&state, auth.user).await
}

/// `POST /api/auth/logout` — revoke the presenting session token.
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Ack>> {
    if let Some(token) = session_token_from_headers(&headers, &state.config.admin.cookie_name) {
        revoke_session(&state.pool, &token).await?;
    }
    Ok(Json(Ack { ok: true }))
}

/// `GET /api/auth/me` — the authenticated account.
async fn me(auth: AuthUser) -> AppResult<Json<AccountDto>> {
    Ok(Json(AccountDto::from(&auth.user)))
}
