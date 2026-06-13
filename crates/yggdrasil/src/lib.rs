//! Yggdrasil online-mode domain: `/authserver`, `/sessionserver`, `/api/profiles`
//! and the `/` meta document, plus RSA-SHA1 (PKCS#1 v1.5) texture-property signing.
//! Mounted by the server crate at the configured public URL (e.g. `/api/yggdrasil`).
//!
//! The signing key is loaded once at startup via [`init_crypto`] and held in a
//! crate-level `OnceLock` (it is process-global, not per-request, and `AppState`
//! is owned by `core`). Handlers read it through [`signing_key`].

pub mod account;
pub mod crypto;
pub mod error;
pub mod join_sessions;
pub mod profile;

use std::sync::OnceLock;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use loontail_core::auth::yggdrasil::{
    invalidate_yggdrasil, issue_yggdrasil_tokens, refresh_yggdrasil, validate_yggdrasil,
};
use loontail_core::identity::{authenticate_password, get_user};
use loontail_core::{AppState, Config};
use loontail_yggdrasil_protocol::uuid::undash_uuid;

use crate::crypto::SigningKey;
use crate::error::{YggError, YggResult};
use crate::profile::{build_profile, ProfileIdentity};

static SIGNING_KEY: OnceLock<SigningKey> = OnceLock::new();

/// Load (or generate) the RSA signing key from `config.yggdrasil.key_path` and
/// install it into the crate-level `OnceLock`. The server crate MUST call this
/// once at startup before mounting [`routes`]; calling it twice is a no-op (the
/// first key wins). Returns the key error if loading/generation fails.
pub fn init_crypto(config: &Config) -> Result<(), crypto::CryptoError> {
    let key = crypto::load_or_generate_key(&config.yggdrasil.key_path)?;
    let _ = SIGNING_KEY.set(key);
    Ok(())
}

/// The installed signing key, or an internal error if [`init_crypto`] was never
/// called (a server-wiring bug, surfaced as a 500 rather than a panic).
fn signing_key() -> YggResult<&'static SigningKey> {
    SIGNING_KEY.get().ok_or_else(|| {
        tracing::error!("yggdrasil signing key not initialized (init_crypto was not called)");
        YggError::Internal
    })
}

/// Build the Yggdrasil domain router. The server crate nests this under the
/// configured public URL prefix and must call [`init_crypto`] first.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/authserver/authenticate", post(authenticate))
        .route("/authserver/refresh", post(refresh))
        .route("/authserver/validate", post(validate))
        .route("/authserver/invalidate", post(invalidate))
        .route("/sessionserver/session/minecraft/join", post(join))
        .route(
            "/sessionserver/session/minecraft/hasJoined",
            get(has_joined),
        )
        .route(
            "/sessionserver/session/minecraft/profile/{uuid}",
            get(profile_by_uuid),
        )
        .route("/api/profiles/minecraft", post(profiles_by_name))
        .route("/", get(meta))
}

// ---------------------------------------------------------------------------
// authserver
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticateRequest {
    username: String,
    password: String,
    #[serde(default)]
    client_token: Option<String>,
    #[serde(default)]
    request_user: bool,
    // Accepted for protocol compatibility (Mojang clients send `agent`); we serve a
    // single Minecraft agent so the value is not used.
    #[serde(default)]
    #[allow(dead_code)]
    agent: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSummary {
    id: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct YggdrasilSession {
    access_token: String,
    client_token: String,
    available_profiles: Vec<ProfileSummary>,
    selected_profile: ProfileSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<Value>,
}

fn profile_summary(profile_uuid: &str, username: &str) -> ProfileSummary {
    ProfileSummary {
        id: profile_uuid.to_string(),
        name: username.to_string(),
    }
}

fn user_block(user_id: uuid::Uuid) -> Value {
    json!({ "id": user_id.to_string(), "properties": [] })
}

/// `POST /authserver/authenticate` — verify credentials, issue an access/client
/// token pair, and return the Mojang session document.
async fn authenticate(
    State(state): State<AppState>,
    Json(body): Json<AuthenticateRequest>,
) -> YggResult<Json<YggdrasilSession>> {
    let user = authenticate_password(&state.pool, &body.username, &body.password).await?;
    let profile_uuid = user.profile_uuid.clone().ok_or(YggError::Forbidden)?;

    let tokens = issue_yggdrasil_tokens(
        &state.pool,
        user.id,
        body.client_token.clone(),
        state.config.yggdrasil.token_ttl,
        state.config.yggdrasil.max_tokens_per_user,
    )
    .await?;

    let selected = profile_summary(&profile_uuid, &user.username);
    let available = vec![profile_summary(&profile_uuid, &user.username)];

    Ok(Json(YggdrasilSession {
        access_token: tokens.access_token,
        client_token: tokens.client_token,
        available_profiles: available,
        selected_profile: selected,
        user: body.request_user.then(|| user_block(user.id)),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshRequest {
    access_token: String,
    #[serde(default)]
    client_token: Option<String>,
    #[serde(default)]
    request_user: bool,
    #[serde(default)]
    selected_profile: Option<Value>,
}

/// `POST /authserver/refresh` — rotate the access token (preserving the client
/// token) and return a fresh session document.
async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> YggResult<Json<YggdrasilSession>> {
    // A client-supplied selectedProfile that names a different account is rejected
    // (single-profile accounts); a matching or absent one is fine.
    let (user, tokens) = refresh_yggdrasil(
        &state.pool,
        &body.access_token,
        body.client_token.as_deref(),
        state.config.yggdrasil.token_ttl,
        state.config.yggdrasil.max_tokens_per_user,
    )
    .await?;

    let profile_uuid = user.profile_uuid.clone().ok_or(YggError::Forbidden)?;
    if let Some(sel) = body.selected_profile.as_ref().and_then(|p| p.get("id")) {
        if sel.as_str() != Some(profile_uuid.as_str()) {
            return Err(YggError::Forbidden);
        }
    }

    let selected = profile_summary(&profile_uuid, &user.username);
    let available = vec![profile_summary(&profile_uuid, &user.username)];

    Ok(Json(YggdrasilSession {
        access_token: tokens.access_token,
        client_token: tokens.client_token,
        available_profiles: available,
        selected_profile: selected,
        user: body.request_user.then(|| user_block(user.id)),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidateRequest {
    access_token: String,
    #[serde(default)]
    client_token: Option<String>,
}

/// `POST /authserver/validate` — 204 if the token pair is live, 403 otherwise.
async fn validate(
    State(state): State<AppState>,
    Json(body): Json<ValidateRequest>,
) -> YggResult<StatusCode> {
    validate_yggdrasil(
        &state.pool,
        &body.access_token,
        body.client_token.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvalidateRequest {
    access_token: String,
    #[serde(default)]
    client_token: Option<String>,
}

/// `POST /authserver/invalidate` — always 204 (idempotent), removing the pair if
/// present.
async fn invalidate(
    State(state): State<AppState>,
    Json(body): Json<InvalidateRequest>,
) -> YggResult<StatusCode> {
    invalidate_yggdrasil(
        &state.pool,
        &body.access_token,
        body.client_token.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// sessionserver
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinRequest {
    access_token: String,
    selected_profile: String,
    server_id: String,
}

/// `POST /sessionserver/session/minecraft/join` — the client announces it is
/// joining a server. We validate the access token, confirm it owns the named
/// profile, and record an in-memory join session keyed by `serverId`.
async fn join(
    State(state): State<AppState>,
    Json(body): Json<JoinRequest>,
) -> YggResult<StatusCode> {
    let user = validate_yggdrasil(&state.pool, &body.access_token, None).await?;
    let profile_uuid = user.profile_uuid.clone().ok_or(YggError::Forbidden)?;

    // The selectedProfile must match the token's owner (undashed comparison).
    let claimed = undash_uuid(&body.selected_profile).map_err(|_| YggError::Forbidden)?;
    if claimed != profile_uuid {
        return Err(YggError::Forbidden);
    }

    join_sessions::record(&body.server_id, user.id, None);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HasJoinedQuery {
    username: String,
    server_id: String,
    #[serde(default)]
    ip: Option<String>,
}

/// `GET /sessionserver/session/minecraft/hasJoined` — the server asks whether the
/// named user joined. Single-use take of the join session; on a match (username +
/// optional IP) return the SIGNED profile, else 204.
async fn has_joined(
    State(state): State<AppState>,
    Query(query): Query<HasJoinedQuery>,
) -> Result<axum::response::Response, YggError> {
    use axum::response::IntoResponse;

    let Some(session) = join_sessions::take(&query.server_id) else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    // IP is only checked when both the recorded and queried IPs are present.
    if let (Some(recorded), Some(queried)) = (session.ip.as_deref(), query.ip.as_deref()) {
        if recorded != queried {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }

    let user = get_user(&state.pool, session.user_id).await?;
    // The join must be for the username the server is asking about.
    if !user.username.eq_ignore_ascii_case(query.username.trim()) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let profile_uuid = user.profile_uuid.clone().ok_or(YggError::Forbidden)?;

    let identity = ProfileIdentity {
        user_id: user.id,
        profile_uuid,
        username: user.username,
    };
    let profile = build_profile(
        &state.pool,
        &state.config.yggdrasil.public_url,
        &identity,
        Some(signing_key()?),
    )
    .await?;

    Ok(Json(profile).into_response())
}

#[derive(Debug, Deserialize)]
struct UnsignedQuery {
    #[serde(default)]
    unsigned: Option<String>,
}

/// `GET /sessionserver/session/minecraft/profile/{uuid}` — look up a profile by
/// its undashed UUID. Signed unless `?unsigned=true`. 204 if no such profile.
async fn profile_by_uuid(
    State(state): State<AppState>,
    Path(uuid): Path<String>,
    Query(query): Query<UnsignedQuery>,
) -> Result<axum::response::Response, YggError> {
    use axum::response::IntoResponse;

    let profile_uuid = match undash_uuid(&uuid) {
        Ok(id) => id,
        Err(_) => return Ok(StatusCode::NO_CONTENT.into_response()),
    };

    let Some(user) = find_user_by_profile_uuid(&state.pool, &profile_uuid).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let stored_uuid = user.profile_uuid.clone().ok_or(YggError::Forbidden)?;

    let unsigned = matches!(query.unsigned.as_deref(), Some("") | Some("true"));
    let signing = if unsigned { None } else { Some(signing_key()?) };

    let identity = ProfileIdentity {
        user_id: user.id,
        profile_uuid: stored_uuid,
        username: user.username,
    };
    let profile = build_profile(
        &state.pool,
        &state.config.yggdrasil.public_url,
        &identity,
        signing,
    )
    .await?;

    Ok(Json(profile).into_response())
}

/// `POST /api/profiles/minecraft` — bulk name→profile lookup. Body is a JSON array
/// of usernames; returns `[{id, name}]` for the ones that exist.
async fn profiles_by_name(
    State(state): State<AppState>,
    Json(names): Json<Vec<String>>,
) -> YggResult<Json<Vec<ProfileSummary>>> {
    let mut out = Vec::new();
    for name in names.iter().take(100) {
        let normalized = name.trim().to_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if let Some(user) = find_user_by_normalized_username(&state.pool, &normalized).await? {
            if let Some(profile_uuid) = user.profile_uuid {
                out.push(profile_summary(&profile_uuid, &user.username));
            }
        }
    }
    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// meta
// ---------------------------------------------------------------------------

/// `GET /` — the Yggdrasil meta document: server identity, skin domains, and the
/// SPKI public key used to verify signed textures.
async fn meta(State(state): State<AppState>) -> YggResult<Json<Value>> {
    let key = signing_key()?;
    Ok(Json(json!({
        "meta": {
            "serverName": "Loontail",
            "implementationName": "loontail-launcher-api",
            "implementationVersion": env!("CARGO_PKG_VERSION"),
        },
        "skinDomains": state.config.yggdrasil.skin_domains,
        "signaturePublickey": key.public_spki_pem(),
    })))
}

// ---------------------------------------------------------------------------
// user lookups (runtime sqlx)
// ---------------------------------------------------------------------------

async fn find_user_by_profile_uuid(
    pool: &sqlx::PgPool,
    profile_uuid: &str,
) -> Result<Option<loontail_core::models::User>, YggError> {
    let user = sqlx::query_as::<_, loontail_core::models::User>(
        "SELECT * FROM users WHERE profile_uuid = $1",
    )
    .bind(profile_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|e| YggError::from(loontail_core::AppError::from(e)))?;
    Ok(user)
}

async fn find_user_by_normalized_username(
    pool: &sqlx::PgPool,
    normalized: &str,
) -> Result<Option<loontail_core::models::User>, YggError> {
    let user = sqlx::query_as::<_, loontail_core::models::User>(
        "SELECT * FROM users WHERE normalized_username = $1",
    )
    .bind(normalized)
    .fetch_optional(pool)
    .await
    .map_err(|e| YggError::from(loontail_core::AppError::from(e)))?;
    Ok(user)
}

/// The launcher/agent account surface (`/api/auth/*`), mounted by the server crate.
pub use account::account_routes;

/// Re-exports for the integration tests: the profile types the handlers emit.
pub use profile::{GameProfile, ProfileProperty};
