use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::models::{UserDto, UserStatus};
use loontail_core::AppState;
use loontail_core::Metrics;
use loontail_core::ServerEvent;

/// Collapse a stored status to `offline` when the last heartbeat is older than
/// the configured timeout.
pub fn effective_status(
    stored: UserStatus,
    last_heartbeat: DateTime<Utc>,
    timeout: StdDuration,
) -> UserStatus {
    let timeout = Duration::from_std(timeout).unwrap_or_else(|_| Duration::seconds(60));
    if Utc::now().signed_duration_since(last_heartbeat) > timeout {
        UserStatus::Offline
    } else {
        stored
    }
}

#[derive(sqlx::FromRow)]
struct PresenceRow {
    status: String,
    last_heartbeat_at: DateTime<Utc>,
}

/// Resolve the effective status for a single user (used by `/me`).
pub async fn effective_status_for(state: &AppState, user_id: Uuid) -> AppResult<UserStatus> {
    let row = sqlx::query_as::<_, PresenceRow>(
        "SELECT status, last_heartbeat_at FROM presence WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(match row {
        Some(row) => effective_status(
            UserStatus::from_db(&row.status),
            row.last_heartbeat_at,
            state.config.heartbeat_timeout,
        ),
        None => UserStatus::Offline,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub status: UserStatus,
}

/// `POST /presence/heartbeat` — refresh liveness. Returns the effective status.
pub async fn heartbeat(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<StatusResponse>> {
    sqlx::query(
        r#"
        INSERT INTO presence (user_id, status, last_heartbeat_at)
        VALUES ($1, 'online', now())
        ON CONFLICT (user_id) DO UPDATE SET
            last_heartbeat_at = now(),
            status = CASE WHEN presence.status = 'offline' THEN 'online' ELSE presence.status END,
            updated_at = now()
        "#,
    )
    .bind(auth.id())
    .execute(&state.pool)
    .await?;

    Metrics::incr(&state.metrics.heartbeats);
    let status = effective_status_for(&state, auth.id()).await?;
    Ok(Json(StatusResponse { status }))
}

/// Mark a user online (used when a signaling connection opens). Mirrors the
/// heartbeat upsert: only flips an offline row to online, preserving in-world.
pub async fn mark_online(state: &AppState, user_id: Uuid) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO presence (user_id, status, last_heartbeat_at)
        VALUES ($1, 'online', now())
        ON CONFLICT (user_id) DO UPDATE SET
            last_heartbeat_at = now(),
            status = CASE WHEN presence.status = 'offline' THEN 'online' ELSE presence.status END,
            updated_at = now()
        "#,
    )
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Mark a user offline (used when their last signaling connection closes).
pub async fn mark_offline(state: &AppState, user_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "UPDATE presence SET status = 'offline', current_world_session_id = NULL, updated_at = now() WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// The user ids of a user's friends.
async fn friend_ids(state: &AppState, user_id: Uuid) -> AppResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT CASE WHEN user_a_id = $1 THEN user_b_id ELSE user_a_id END
        FROM friendships
        WHERE user_a_id = $1 OR user_b_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(ids)
}

/// Notify a user's friends that their presence changed, so each reloads their
/// friends list and sees the new status in real time.
pub async fn broadcast_presence(state: &AppState, user_id: Uuid) -> AppResult<()> {
    for friend_id in friend_ids(state, user_id).await? {
        state
            .realtime
            .signaling
            .send_to(friend_id, ServerEvent::PresenceUpdate { user_id });
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetStatusRequest {
    pub status: String,
    #[serde(default)]
    pub current_world_session_id: Option<Uuid>,
}

/// `POST /presence/status` — explicitly set status (online/inWorld/joinable).
pub async fn set_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetStatusRequest>,
) -> AppResult<Json<StatusResponse>> {
    let status = UserStatus::from_client(&body.status)
        .ok_or_else(|| AppError::BadRequest("status must be online, inWorld or joinable".into()))?;

    // When entering a world, the referenced session must be an open world
    // owned by this user.
    if status.is_in_world() {
        if let Some(world_id) = body.current_world_session_id {
            let owns: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1 FROM world_sessions
                    WHERE id = $1 AND host_user_id = $2 AND status = 'open'
                )
                "#,
            )
            .bind(world_id)
            .bind(auth.id())
            .fetch_one(&state.pool)
            .await?;
            if !owns {
                return Err(AppError::BadRequest(
                    "currentWorldSessionId must be your own open world session".into(),
                ));
            }
        }
    }

    let world_id = if status.is_in_world() {
        body.current_world_session_id
    } else {
        None
    };

    sqlx::query(
        r#"
        INSERT INTO presence (user_id, status, last_heartbeat_at, current_world_session_id)
        VALUES ($1, $2, now(), $3)
        ON CONFLICT (user_id) DO UPDATE SET
            status = EXCLUDED.status,
            last_heartbeat_at = now(),
            current_world_session_id = EXCLUDED.current_world_session_id,
            updated_at = now()
        "#,
    )
    .bind(auth.id())
    .bind(status.as_str())
    .bind(world_id)
    .execute(&state.pool)
    .await?;

    // Push the new status to friends in real time.
    let _ = broadcast_presence(&state, auth.id()).await;

    Ok(Json(StatusResponse { status }))
}

#[derive(sqlx::FromRow)]
struct FriendPresenceRow {
    id: Uuid,
    minecraft_uuid: String,
    username: String,
    avatar_url: Option<String>,
    skin_hash: Option<String>,
    status: Option<String>,
    last_heartbeat_at: Option<DateTime<Utc>>,
    current_world_session_id: Option<Uuid>,
    /// The friend's last-reported Minecraft version + mod loader, used by the client to gate
    /// joins/invites (both must match to play together).
    minecraft_version: Option<String>,
    loader: Option<String>,
    /// Set when this friend is an active guest in an open friend-of-friends
    /// world — the world the viewer (their friend) may ask to join.
    guest_world_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendPresence {
    #[serde(flatten)]
    pub user: UserDto,
    pub status: UserStatus,
    pub current_world_session_id: Option<Uuid>,
    /// The friend's reported Minecraft version + mod loader, so a client can gate joins/invites
    /// against its own (both must match). Null until the friend bootstraps with the field.
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
}

/// Shared query: a user's friends with their effective presence.
pub async fn friends_with_presence(
    state: &AppState,
    user_id: Uuid,
) -> AppResult<Vec<FriendPresence>> {
    let rows = sqlx::query_as::<_, FriendPresenceRow>(
        r#"
        SELECT
            u.id,
            u.minecraft_uuid,
            u.username,
            u.avatar_url,
            u.skin_hash,
            p.status,
            p.last_heartbeat_at,
            p.current_world_session_id,
            p.minecraft_version,
            p.loader,
            g.guest_world_id
        FROM friendships f
        JOIN users u
            ON u.id = CASE WHEN f.user_a_id = $1 THEN f.user_b_id ELSE f.user_a_id END
        LEFT JOIN presence p ON p.user_id = u.id
        LEFT JOIN LATERAL (
            SELECT ws.id AS guest_world_id
            FROM relay_sessions rs
            JOIN world_sessions ws ON ws.id = rs.world_session_id
            WHERE rs.guest_user_id = u.id
              AND rs.status = 'active'
              AND ws.status = 'open'
              AND ws.invite_policy = 'friends_of_friends'
            ORDER BY rs.created_at DESC
            LIMIT 1
        ) g ON true
        WHERE f.user_a_id = $1 OR f.user_b_id = $1
        ORDER BY u.normalized_username
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;

    let timeout = state.config.heartbeat_timeout;
    let friends = rows
        .into_iter()
        .map(|row| {
            let base = match (row.status, row.last_heartbeat_at) {
                (Some(status), Some(heartbeat)) => {
                    effective_status(UserStatus::from_db(&status), heartbeat, timeout)
                }
                _ => UserStatus::Offline,
            };
            // A friend connected as a guest to a friend-of-friends world is shown
            // as in-world (pointing at that world) so the viewer can ask to join
            // it. host-only worlds never surface here, so the guest stays plain
            // "online" and no join/invite affordance appears.
            let (status, current_world_session_id) = match row.guest_world_id {
                Some(world) if base != UserStatus::Offline => (UserStatus::InWorld, Some(world)),
                _ if base.is_in_world() => (base, row.current_world_session_id),
                _ => (base, None),
            };
            FriendPresence {
                user: UserDto {
                    id: row.id,
                    minecraft_uuid: row.minecraft_uuid,
                    username: row.username,
                    avatar_url: row.avatar_url,
                    skin_hash: row.skin_hash,
                },
                status,
                current_world_session_id,
                minecraft_version: row.minecraft_version,
                loader: row.loader,
            }
        })
        .collect();

    Ok(friends)
}

/// `GET /presence/friends` — friends with their current statuses.
pub async fn friends_presence(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<FriendPresence>>> {
    let friends = friends_with_presence(&state, auth.id()).await?;
    Ok(Json(friends))
}
