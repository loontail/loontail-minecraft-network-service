use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::models::{escape_like_pattern, normalize_username, User, UserDto, UserStatus};
use loontail_core::AppState;
use loontail_core::Metrics;

use crate::presence;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapRequest {
    pub minecraft_uuid: String,
    pub username: String,
    #[serde(default)]
    pub account_type: Option<String>,
    #[serde(default)]
    pub xuid: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub minecraft_version: Option<String>,
    #[serde(default)]
    pub mod_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponse {
    pub user: UserDto,
}

/// `POST /users/bootstrap` — the in-game agent starts its network session
/// (authenticated by `AuthUser`; issues NO token). `minecraft_uuid` is filled
/// only when unset, so an account keeps one stable identity.
pub async fn bootstrap(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<BootstrapRequest>,
) -> AppResult<Json<BootstrapResponse>> {
    let minecraft_uuid = body.minecraft_uuid.trim();

    let user = sqlx::query_as::<_, User>(
        r#"
        UPDATE users SET
            minecraft_uuid = COALESCE(minecraft_uuid, NULLIF($2, '')),
            account_type   = COALESCE($3, account_type),
            xuid           = COALESCE($4, xuid),
            client_id      = COALESCE($5, client_id),
            last_seen_at   = now(),
            updated_at     = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(auth.id())
    .bind(minecraft_uuid)
    .bind(&body.account_type)
    .bind(&body.xuid)
    .bind(&body.client_id)
    .fetch_one(&state.pool)
    .await
    .map_err(map_bootstrap_conflict)?;

    sqlx::query(
        r#"
        INSERT INTO presence
            (user_id, status, last_heartbeat_at, minecraft_version, mod_version, loader)
        VALUES ($1, 'online', now(), $2, $3, $4)
        ON CONFLICT (user_id) DO UPDATE SET
            status            = 'online',
            last_heartbeat_at = now(),
            minecraft_version = COALESCE(EXCLUDED.minecraft_version, presence.minecraft_version),
            mod_version       = COALESCE(EXCLUDED.mod_version, presence.mod_version),
            loader            = COALESCE(EXCLUDED.loader, presence.loader),
            updated_at        = now()
        "#,
    )
    .bind(user.id)
    .bind(&body.minecraft_version)
    .bind(&body.mod_version)
    .bind(&body.loader)
    .execute(&state.pool)
    .await?;

    Metrics::incr(&state.metrics.bootstraps);
    loontail_core::analytics::spawn_event(&state, "bootstrap", Some(user.id), None);

    Ok(Json(BootstrapResponse {
        user: UserDto::from(user),
    }))
}

/// Surface a Minecraft UUID already bound to another account as a 409, not a 500.
fn map_bootstrap_conflict(err: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db) = &err {
        if db.constraint() == Some("users_minecraft_uuid_key") {
            return AppError::Conflict(
                "this Minecraft account is already linked to another user".into(),
            );
        }
    }
    AppError::Database(err)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: UserDto,
    pub status: UserStatus,
}

/// `GET /me` — the authenticated user plus their effective status.
pub async fn me(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<MeResponse>> {
    let status = presence::effective_status_for(&state, auth.id()).await?;
    Ok(Json(MeResponse {
        user: UserDto::from(&auth.user),
        status,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub user: UserDto,
    pub is_friend: bool,
    pub has_incoming_request: bool,
    pub has_outgoing_request: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct SearchRow {
    id: uuid::Uuid,
    minecraft_uuid: Option<String>,
    username: String,
    avatar_url: Option<String>,
    skin_hash: Option<String>,
    is_friend: bool,
    has_incoming_request: bool,
    has_outgoing_request: bool,
}

/// `GET /users/search?q=` — case-insensitive substring search by username.
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<Vec<SearchResult>>> {
    let needle = query.q.trim();
    if needle.len() < state.config.search_min_query_length {
        return Err(AppError::BadRequest(format!(
            "query must be at least {} characters",
            state.config.search_min_query_length
        )));
    }

    // why: escape LIKE metacharacters so a literal % or _ matches literally
    // instead of acting as a wildcard (and can't force a full scan).
    let pattern = format!("%{}%", escape_like_pattern(&normalize_username(needle)));
    let me_id = auth.id();

    let rows = sqlx::query_as::<_, SearchRow>(
        r#"
        SELECT
            u.id,
            u.minecraft_uuid,
            u.username,
            u.avatar_url,
            u.skin_hash,
            EXISTS (
                SELECT 1 FROM friendships f
                WHERE (f.user_a_id = LEAST($1, u.id) AND f.user_b_id = GREATEST($1, u.id))
            ) AS is_friend,
            EXISTS (
                SELECT 1 FROM friend_requests r
                WHERE r.from_user_id = u.id AND r.to_user_id = $1 AND r.status = 'pending'
            ) AS has_incoming_request,
            EXISTS (
                SELECT 1 FROM friend_requests r
                WHERE r.from_user_id = $1 AND r.to_user_id = u.id AND r.status = 'pending'
            ) AS has_outgoing_request
        FROM users u
        WHERE u.normalized_username LIKE $2 ESCAPE '\'
          AND u.id <> $1
        ORDER BY u.normalized_username
        LIMIT $3
        "#,
    )
    .bind(me_id)
    .bind(pattern)
    .bind(state.config.search_max_results)
    .fetch_all(&state.pool)
    .await?;

    let results = rows
        .into_iter()
        .map(|row| SearchResult {
            user: UserDto {
                id: row.id,
                minecraft_uuid: row.minecraft_uuid,
                username: row.username,
                avatar_url: row.avatar_url,
                skin_hash: row.skin_hash,
            },
            is_friend: row.is_friend,
            has_incoming_request: row.has_incoming_request,
            has_outgoing_request: row.has_outgoing_request,
        })
        .collect();

    Ok(Json(results))
}
